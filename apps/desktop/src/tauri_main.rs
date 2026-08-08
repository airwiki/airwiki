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

use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::Serialize;
use tauri::{Manager, ipc::Channel};
use tokio::sync::{broadcast, mpsc, watch};

use crate::{
    paths::AppPaths,
    worker::{WorkerCommand, WorkerEvent, run_worker},
};

const COMMAND_CAPACITY: usize = 64;
const PRESENTATION_CAPACITY: usize = 128;
const CONTRACT_VERSION: u16 = 1;

struct AppRuntime {
    commands: mpsc::Sender<WorkerCommand>,
    snapshot: Mutex<watch::Receiver<AppSnapshot>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSnapshot {
    schema_version: u16,
    sequence: u64,
    phase: &'static str,
    collections: Vec<CollectionSummary>,
    reviews: Vec<ReviewSummary>,
    source_issues: Vec<SourceIssueSummary>,
    peers: Vec<PeerSummary>,
    model: Option<ModelSummary>,
    notice: Option<NoticeSummary>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CollectionSummary {
    id: String,
    name: String,
    document_count: usize,
    needs_review_count: usize,
    published_count: usize,
    failed_count: usize,
    local_only: bool,
    peer_shareable: bool,
    allow_external_ai: bool,
    internet_public: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewSummary {
    concept_id: String,
    source_revision: u32,
    source_name: String,
    collection_name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceIssueSummary {
    collection_id: String,
    source_name: String,
    collection_name: String,
    code: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PeerSummary {
    peer_id: String,
    device_name: Option<String>,
    trust: &'static str,
    activity: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelSummary {
    display_name: Option<String>,
    active: bool,
    installed: bool,
    degraded: bool,
    download_bytes: u64,
    required_free_bytes: u64,
    fits_available_disk: bool,
    license_accepted: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NoticeSummary {
    level: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiEventEnvelope {
    schema_version: u16,
    sequence: u64,
    kind: &'static str,
    snapshot: AppSnapshot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UiError {
    code: &'static str,
    message_key: &'static str,
    retryable: bool,
}

#[tauri::command]
fn connect(runtime: tauri::State<'_, AppRuntime>, events: Channel<UiEventEnvelope>) -> AppSnapshot {
    let Ok(snapshot_receiver) = runtime.snapshot.lock() else {
        return AppSnapshot::starting();
    };
    let mut receiver = snapshot_receiver.clone();
    let snapshot = receiver.borrow().clone();
    drop(snapshot_receiver);
    tauri::async_runtime::spawn(async move {
        while receiver.changed().await.is_ok() {
            let snapshot = receiver.borrow_and_update().clone();
            if events
                .send(UiEventEnvelope {
                    schema_version: CONTRACT_VERSION,
                    sequence: snapshot.sequence,
                    kind: "stateChanged",
                    snapshot,
                })
                .is_err()
            {
                break;
            }
        }
    });
    snapshot
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

impl AppSnapshot {
    fn starting() -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            sequence: 0,
            phase: "starting",
            collections: Vec::new(),
            reviews: Vec::new(),
            source_issues: Vec::new(),
            peers: Vec::new(),
            model: None,
            notice: None,
        }
    }

    fn apply(&mut self, event: WorkerEvent) {
        self.sequence = self.sequence.saturating_add(1);
        match event {
            WorkerEvent::Ready {
                collections,
                reviews,
                source_issues,
                ..
            } => {
                self.phase = "ready";
                self.collections = collections
                    .into_iter()
                    .map(CollectionSummary::from)
                    .collect();
                self.reviews = reviews.into_iter().map(ReviewSummary::from).collect();
                self.source_issues = source_issues
                    .into_iter()
                    .map(SourceIssueSummary::from)
                    .collect();
            }
            WorkerEvent::Collections(collections) => {
                self.collections = collections
                    .into_iter()
                    .map(CollectionSummary::from)
                    .collect();
            }
            WorkerEvent::Reviews(reviews) => {
                self.reviews = reviews.into_iter().map(ReviewSummary::from).collect();
            }
            WorkerEvent::SourceIssues(issues) => {
                self.source_issues = issues.into_iter().map(SourceIssueSummary::from).collect();
            }
            WorkerEvent::Peers(peers) => {
                self.peers = peers.into_iter().map(PeerSummary::from).collect();
            }
            WorkerEvent::ModelState(model) => self.model = Some(ModelSummary::from(model)),
            WorkerEvent::Notice(message) => {
                self.notice = Some(NoticeSummary {
                    level: "notice",
                    message,
                });
            }
            WorkerEvent::Error(_) => {
                self.notice = Some(NoticeSummary {
                    level: "error",
                    message: "runtime-operation-failed".to_owned(),
                });
            }
            _ => {}
        }
    }
}

impl From<worker::CollectionView> for CollectionSummary {
    fn from(value: worker::CollectionView) -> Self {
        Self {
            id: value.id.to_string(),
            name: value.name,
            document_count: value.document_count,
            needs_review_count: value.needs_review_count,
            published_count: value.published_count,
            failed_count: value.failed_count,
            local_only: value.local_only,
            peer_shareable: value.peer_shareable,
            allow_external_ai: value.allow_external_ai,
            internet_public: value.internet_public,
        }
    }
}

impl From<worker::ReviewItemView> for ReviewSummary {
    fn from(value: worker::ReviewItemView) -> Self {
        Self {
            concept_id: value.concept_id.to_string(),
            source_revision: value.source_revision,
            source_name: value.source_name,
            collection_name: value.collection_name,
        }
    }
}

impl From<worker::SourceIssueView> for SourceIssueSummary {
    fn from(value: worker::SourceIssueView) -> Self {
        Self {
            collection_id: value.collection_id.to_string(),
            source_name: value.source_name,
            collection_name: value.collection_name,
            code: format!("{:?}", value.code),
        }
    }
}

impl From<worker::PeerView> for PeerSummary {
    fn from(value: worker::PeerView) -> Self {
        Self {
            peer_id: value.peer_id,
            device_name: value.device_name,
            trust: match value.trust {
                worker::PeerTrustState::Unpaired => "unpaired",
                worker::PeerTrustState::Trusted => "trusted",
                worker::PeerTrustState::Blocked => "blocked",
            },
            activity: match value.activity {
                worker::PeerActivityState::NotObserved => "notObserved",
                worker::PeerActivityState::Discovered => "discovered",
                worker::PeerActivityState::Pairing => "pairing",
                worker::PeerActivityState::Connected => "connected",
            },
        }
    }
}

impl From<worker::ModelStateView> for ModelSummary {
    fn from(value: worker::ModelStateView) -> Self {
        Self {
            display_name: value.recommended_display_name,
            active: value.active_model_id.is_some(),
            installed: value.recommended_assets_installed,
            degraded: value.degraded,
            download_bytes: value.download_bytes,
            required_free_bytes: value.required_free_bytes,
            fits_available_disk: value.fits_available_disk,
            license_accepted: value.license_accepted,
        }
    }
}

fn main() -> Result<()> {
    let paths = AppPaths::discover().context("failed to discover application paths")?;
    let (commands, command_receiver) = mpsc::channel(COMMAND_CAPACITY);
    let (events, _) = broadcast::channel(PRESENTATION_CAPACITY);
    let worker_events = events.clone();
    let (snapshot_sender, snapshot_receiver) = watch::channel(AppSnapshot::starting());
    let mut presentation_events = events.subscribe();

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
            snapshot: Mutex::new(snapshot_receiver),
        })
        .setup(move |_app| {
            tauri::async_runtime::spawn(async move {
                let mut snapshot = AppSnapshot::starting();
                loop {
                    match presentation_events.recv().await {
                        Ok(event) => {
                            snapshot.apply(event);
                            if snapshot_sender.send(snapshot.clone()).is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            snapshot.notice = Some(NoticeSummary {
                                level: "warning",
                                message: "runtime-snapshot-required".to_owned(),
                            });
                            snapshot.sequence = snapshot.sequence.saturating_add(1);
                            if snapshot_sender.send(snapshot.clone()).is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
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
