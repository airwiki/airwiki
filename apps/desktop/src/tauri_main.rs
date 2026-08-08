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

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

use airwiki_inference::ModelProfile;
use airwiki_types::SearchPurpose;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{Manager, ipc::Channel};
use tokio::sync::{broadcast, mpsc, watch};
use uuid::Uuid;

use crate::{
    paths::AppPaths,
    worker::{WorkerCommand, WorkerEvent, run_worker},
};

const COMMAND_CAPACITY: usize = 64;
const PRESENTATION_CAPACITY: usize = 128;
const CONTRACT_VERSION: u16 = 1;
const FOLDER_SELECTION_TTL: Duration = Duration::from_secs(5 * 60);

struct AppRuntime {
    commands: mpsc::Sender<WorkerCommand>,
    snapshot: Mutex<watch::Receiver<AppSnapshot>>,
    folder_selections: Mutex<HashMap<Uuid, PendingFolderSelection>>,
}

struct PendingFolderSelection {
    path: PathBuf,
    expires_at: Instant,
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
    search: Option<SearchSummary>,
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
struct SearchSummary {
    request_id: String,
    status: &'static str,
    hits: Vec<SearchHitSummary>,
    coverage: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchHitSummary {
    title: String,
    snippet: String,
    heading_or_page: String,
    logical_resource_uri: String,
    rank: u32,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderSelection {
    token: String,
    display_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CollectionPolicyInput {
    local_only: bool,
    peer_shareable: bool,
    allow_external_ai: bool,
    internet_public: bool,
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

#[tauri::command]
fn set_model_profile(
    runtime: tauri::State<'_, AppRuntime>,
    profile: ModelProfile,
) -> Result<(), UiError> {
    send_command(&runtime, WorkerCommand::SetModelProfile(profile))
}

#[tauri::command]
async fn pick_collection_folder(
    runtime: tauri::State<'_, AppRuntime>,
) -> Result<Option<FolderSelection>, UiError> {
    let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await else {
        return Ok(None);
    };
    let path = folder.path().to_path_buf();
    let token = Uuid::new_v4();
    let mut selections = runtime
        .folder_selections
        .lock()
        .map_err(|_| UiError::internal())?;
    selections.retain(|_, selection| selection.expires_at > Instant::now());
    selections.insert(
        token,
        PendingFolderSelection {
            path: path.clone(),
            expires_at: Instant::now() + FOLDER_SELECTION_TTL,
        },
    );
    Ok(Some(FolderSelection {
        token: token.to_string(),
        display_path: path.to_string_lossy().into_owned(),
    }))
}

#[tauri::command]
fn add_collection(
    runtime: tauri::State<'_, AppRuntime>,
    name: String,
    folder_token: String,
) -> Result<(), UiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(UiError::invalid("invalidCollectionName"));
    }
    let folder = consume_folder_selection(&runtime, &folder_token)?;
    send_command(
        &runtime,
        WorkerCommand::AddCollection {
            name: name.to_owned(),
            folder,
        },
    )
}

#[tauri::command]
fn relink_collection(
    runtime: tauri::State<'_, AppRuntime>,
    collection_id: String,
    folder_token: String,
) -> Result<(), UiError> {
    let collection_id = parse_uuid(&collection_id)?;
    let folder = consume_folder_selection(&runtime, &folder_token)?;
    send_command(
        &runtime,
        WorkerCommand::RelinkCollection {
            collection_id,
            folder,
        },
    )
}

#[tauri::command]
fn rescan_collection(
    runtime: tauri::State<'_, AppRuntime>,
    collection_id: String,
) -> Result<(), UiError> {
    send_command(
        &runtime,
        WorkerCommand::RescanCollection(parse_uuid(&collection_id)?),
    )
}

#[tauri::command]
fn update_collection_policy(
    runtime: tauri::State<'_, AppRuntime>,
    collection_id: String,
    policy: CollectionPolicyInput,
) -> Result<(), UiError> {
    send_command(
        &runtime,
        WorkerCommand::UpdateCollectionPolicy {
            collection_id: parse_uuid(&collection_id)?,
            local_only: policy.local_only,
            peer_shareable: policy.peer_shareable,
            allow_external_ai: policy.allow_external_ai,
            internet_public: policy.internet_public,
        },
    )
}

#[tauri::command]
fn search(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    question: String,
    top_k: u8,
    public_network: bool,
) -> Result<(), UiError> {
    if question.trim().is_empty() || question.len() > 4_096 || !(1..=10).contains(&top_k) {
        return Err(UiError::invalid("invalidSearchRequest"));
    }
    send_command(
        &runtime,
        WorkerCommand::Search {
            request_id: parse_uuid(&request_id)?,
            question,
            top_k,
            purpose: SearchPurpose::LocalAssistant,
            public_network,
        },
    )
}

fn consume_folder_selection(runtime: &AppRuntime, token: &str) -> Result<PathBuf, UiError> {
    let token = parse_uuid(token)?;
    let selection = runtime
        .folder_selections
        .lock()
        .map_err(|_| UiError::internal())?
        .remove(&token)
        .ok_or_else(|| UiError::invalid("folderSelectionExpired"))?;
    if selection.expires_at <= Instant::now() {
        return Err(UiError::invalid("folderSelectionExpired"));
    }
    Ok(selection.path)
}

fn parse_uuid(value: &str) -> Result<Uuid, UiError> {
    Uuid::parse_str(value).map_err(|_| UiError::invalid("invalidIdentifier"))
}

impl UiError {
    const fn invalid(message_key: &'static str) -> Self {
        Self {
            code: "invalidInput",
            message_key,
            retryable: false,
        }
    }

    const fn internal() -> Self {
        Self {
            code: "internal",
            message_key: "runtimeStateUnavailable",
            retryable: true,
        }
    }
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
            search: None,
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
            WorkerEvent::SearchPartial { request_id, hits } => {
                self.search = Some(SearchSummary {
                    request_id: request_id.to_string(),
                    status: "searching",
                    hits: hits.into_iter().map(SearchHitSummary::from).collect(),
                    coverage: "partial",
                });
            }
            WorkerEvent::SearchFinished { request_id, result } => {
                self.search = Some(match result {
                    Ok((hits, coverage, _route)) => SearchSummary {
                        request_id: request_id.to_string(),
                        status: "complete",
                        hits: hits.into_iter().map(SearchHitSummary::from).collect(),
                        coverage: match coverage {
                            worker::SearchCoverageView::Complete => "complete",
                            worker::SearchCoverageView::FederationDisabled => "federationDisabled",
                            worker::SearchCoverageView::OfflineDevices { .. } => "offlineDevices",
                            worker::SearchCoverageView::PublicNetworkOffline => {
                                "publicNetworkOffline"
                            }
                            worker::SearchCoverageView::Partial => "partial",
                        },
                    },
                    Err(_) => SearchSummary {
                        request_id: request_id.to_string(),
                        status: "failed",
                        hits: Vec::new(),
                        coverage: "partial",
                    },
                });
            }
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

impl From<airwiki_types::SearchHit> for SearchHitSummary {
    fn from(value: airwiki_types::SearchHit) -> Self {
        Self {
            title: value.title,
            snippet: value.snippet,
            heading_or_page: value.heading_or_page,
            logical_resource_uri: value.logical_resource_uri,
            rank: value.rank,
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
            folder_selections: Mutex::new(HashMap::new()),
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
            cancel_model_install,
            set_model_profile,
            pick_collection_folder,
            add_collection,
            relink_collection,
            rescan_collection,
            update_collection_policy,
            search
        ])
        .run(tauri::generate_context!())
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_with_selection(token: Uuid, path: PathBuf, expires_at: Instant) -> AppRuntime {
        let (commands, _receiver) = mpsc::channel(COMMAND_CAPACITY);
        let (_snapshot_sender, snapshot) = watch::channel(AppSnapshot::starting());
        AppRuntime {
            commands,
            snapshot: Mutex::new(snapshot),
            folder_selections: Mutex::new(HashMap::from([(
                token,
                PendingFolderSelection { path, expires_at },
            )])),
        }
    }

    #[test]
    fn folder_selection_is_consumed_exactly_once() {
        let token = Uuid::new_v4();
        let expected = PathBuf::from("/synthetic/knowledge");
        let runtime = runtime_with_selection(
            token,
            expected.clone(),
            Instant::now() + Duration::from_secs(30),
        );

        assert_eq!(
            consume_folder_selection(&runtime, &token.to_string()).ok(),
            Some(expected)
        );
        assert!(consume_folder_selection(&runtime, &token.to_string()).is_err());
    }

    #[test]
    fn expired_folder_selection_fails_closed() {
        let token = Uuid::new_v4();
        let runtime = runtime_with_selection(
            token,
            PathBuf::from("/synthetic/expired"),
            Instant::now() - Duration::from_secs(1),
        );

        let error = consume_folder_selection(&runtime, &token.to_string()).unwrap_err();

        assert_eq!(error.message_key, "folderSelectionExpired");
    }
}
