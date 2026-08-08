#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![expect(
    dead_code,
    reason = "the internal migration runner reuses backend modules before the egui cutover"
)]

mod autostart;
mod connectivity_platform;
mod integrations;
mod manual_lan_route;
mod model_activation_status;
mod model_config;
mod paths;
mod readiness;
mod services;
mod updater;
mod worker;

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde::Serialize;
use tauri::{Manager, ipc::Channel};
use tokio::sync::{broadcast, mpsc};

use crate::{
    paths::AppPaths,
    worker::{WorkerCommand, WorkerEvent, run_worker},
};

const COMMAND_CAPACITY: usize = 64;
const PRESENTATION_CAPACITY: usize = 128;
const CONTRACT_VERSION: u16 = 1;

struct AppRuntime {
    commands: mpsc::Sender<WorkerCommand>,
    events: broadcast::Sender<WorkerEvent>,
    sequence: AtomicU64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSnapshot {
    schema_version: u16,
    sequence: u64,
    phase: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiEventEnvelope {
    schema_version: u16,
    sequence: u64,
    kind: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiError {
    code: &'static str,
    message_key: &'static str,
    retryable: bool,
}

#[tauri::command]
fn connect(
    runtime: tauri::State<'_, AppRuntime>,
    events: Channel<UiEventEnvelope>,
) -> AppSnapshot {
    let mut receiver = runtime.events.subscribe();
    let sequence = runtime.sequence.fetch_add(1, Ordering::Relaxed) + 1;
    tauri::async_runtime::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let kind = worker_event_kind(&event);
                    if events
                        .send(UiEventEnvelope {
                            schema_version: CONTRACT_VERSION,
                            sequence,
                            kind,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if events
                        .send(UiEventEnvelope {
                            schema_version: CONTRACT_VERSION,
                            sequence,
                            kind: "snapshotRequired",
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    AppSnapshot {
        schema_version: CONTRACT_VERSION,
        sequence,
        phase: "starting",
    }
}

#[tauri::command]
fn install_models(runtime: tauri::State<'_, AppRuntime>) -> Result<(), UiError> {
    send_command(&runtime, WorkerCommand::InstallModels)
}

#[tauri::command]
fn cancel_model_install(runtime: tauri::State<'_, AppRuntime>) -> Result<(), UiError> {
    send_command(&runtime, WorkerCommand::CancelInstall)
}

fn send_command(runtime: &AppRuntime, command: WorkerCommand) -> Result<(), UiError> {
    runtime.commands.try_send(command).map_err(|error| UiError {
        code: match error {
            mpsc::error::TrySendError::Full(_) => "busy",
            mpsc::error::TrySendError::Closed(_) => "unavailable",
        },
        message_key: "runtime-command-unavailable",
        retryable: true,
    })
}

fn worker_event_kind(event: &WorkerEvent) -> &'static str {
    match event {
        WorkerEvent::Ready { .. } => "ready",
        WorkerEvent::Hardware(_) => "hardware",
        WorkerEvent::ModelState(_) => "modelState",
        WorkerEvent::InstallProgress(_) => "installProgress",
        WorkerEvent::ModelsReady => "modelsReady",
        WorkerEvent::ModelsMissing => "modelsMissing",
        WorkerEvent::Collections(_) => "collections",
        WorkerEvent::Reviews(_) => "reviews",
        WorkerEvent::SourceIssues(_) => "sourceIssues",
        WorkerEvent::Peers(_) => "peers",
        WorkerEvent::Notice(_) => "notice",
        WorkerEvent::Error(_) => "error",
        _ => "stateChanged",
    }
}

fn main() -> Result<()> {
    let paths = AppPaths::discover().context("failed to discover application paths")?;
    let (commands, command_receiver) = mpsc::channel(COMMAND_CAPACITY);
    let (events, _) = broadcast::channel(PRESENTATION_CAPACITY);
    let worker_events = events.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .manage(AppRuntime {
            commands,
            events,
            sequence: AtomicU64::new(0),
        })
        .setup(move |_app| {
            tauri::async_runtime::spawn(run_worker(paths, command_receiver, worker_events));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            connect,
            install_models,
            cancel_model_install
        ])
        .run(tauri::generate_context!())
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}
