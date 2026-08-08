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
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use airwiki_core::{KnowledgeBundleState, KnowledgePageId, KnowledgePageView, ReviewVersionToken};
use airwiki_inference::ModelProfile;
use airwiki_types::{ConceptType, EnrichmentDraft, SearchPurpose, SuggestedEntity, SuggestedLink};
use anyhow::{Context, Result};
use pulldown_cmark::{CodeBlockKind, Event as MarkdownEvent, HeadingLevel, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, WindowEvent,
    ipc::Channel,
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    model_config::{CloseBehavior, LanPreference, LocalePreference},
    paths::AppPaths,
    worker::{DesktopPreferencesUpdate, WorkerCommand, WorkerEvent, run_worker},
};

const COMMAND_CAPACITY: usize = 64;
const PRESENTATION_CAPACITY: usize = 128;
const CONTRACT_VERSION: u16 = 1;
const FOLDER_SELECTION_TTL: Duration = Duration::from_secs(5 * 60);

struct AppRuntime {
    commands: mpsc::Sender<WorkerCommand>,
    snapshot: Mutex<watch::Receiver<AppSnapshot>>,
    folder_selections: Mutex<HashMap<Uuid, PendingFolderSelection>>,
    review_versions: Arc<Mutex<HashMap<Uuid, CachedReviewVersion>>>,
    knowledge_fingerprints: Arc<Mutex<HashMap<(Uuid, KnowledgePageId), String>>>,
    requests: Arc<Mutex<RequestTracker>>,
    tray_operational: AtomicBool,
    exiting: AtomicBool,
    worker_finished: Mutex<Option<oneshot::Receiver<()>>>,
}

#[derive(Clone)]
struct CachedReviewVersion {
    source_revision: u32,
    token: ReviewVersionToken,
}

#[derive(Default)]
struct RequestTracker {
    search: Option<Uuid>,
    review_evidence: HashMap<Uuid, Uuid>,
    knowledge_bundle: HashMap<Uuid, Uuid>,
    knowledge_page: HashMap<Uuid, Uuid>,
    preferences: Option<Uuid>,
}

struct PendingFolderSelection {
    path: PathBuf,
    expires_at: Instant,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct AppSnapshot {
    schema_version: u16,
    #[ts(type = "number")]
    sequence: u64,
    phase: AppPhase,
    collections: Vec<CollectionSummary>,
    reviews: Vec<ReviewSummary>,
    source_issues: Vec<SourceIssueSummary>,
    peers: Vec<PeerSummary>,
    model: Option<ModelSummary>,
    model_install: Option<ModelInstallSummary>,
    search: Option<SearchSummary>,
    review_evidence: Option<ReviewEvidenceSummary>,
    knowledge: Option<KnowledgeBundleSummary>,
    knowledge_page: Option<KnowledgePageSummary>,
    preferences: Option<PreferencesSummary>,
    notice: Option<NoticeSummary>,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum AppPhase {
    Starting,
    Ready,
}

#[derive(Clone, Debug, Serialize, TS)]
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

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct ReviewSummary {
    concept_id: String,
    source_revision: u32,
    source_name: String,
    collection_name: String,
    draft: EnrichmentDraftDto,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "EnrichmentDraft", rename_all = "camelCase")]
struct EnrichmentDraftDto {
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    concept_type: ConceptTypeDto,
    title: String,
    description: String,
    language: String,
    tags: Vec<String>,
    entities: Vec<SuggestedEntityDto>,
    links: Vec<SuggestedLinkDto>,
    summary: String,
    classification_confidence: f32,
    classification_explanation: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[ts(rename = "ConceptType")]
enum ConceptTypeDto {
    Document,
    Policy,
    Procedure,
    Runbook,
    Reference,
    Report,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "SuggestedEntity")]
struct SuggestedEntityDto {
    name: String,
    kind: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "SuggestedLink")]
struct SuggestedLinkDto {
    label: String,
    target: String,
}

impl From<EnrichmentDraft> for EnrichmentDraftDto {
    fn from(value: EnrichmentDraft) -> Self {
        Self {
            concept_type: value.concept_type.into(),
            title: value.title,
            description: value.description,
            language: value.language,
            tags: value.tags,
            entities: value.entities.into_iter().map(Into::into).collect(),
            links: value.links.into_iter().map(Into::into).collect(),
            summary: value.summary,
            classification_confidence: value.classification_confidence,
            classification_explanation: value.classification_explanation,
        }
    }
}

impl From<EnrichmentDraftDto> for EnrichmentDraft {
    fn from(value: EnrichmentDraftDto) -> Self {
        Self {
            concept_type: value.concept_type.into(),
            title: value.title,
            description: value.description,
            language: value.language,
            tags: value.tags,
            entities: value.entities.into_iter().map(Into::into).collect(),
            links: value.links.into_iter().map(Into::into).collect(),
            summary: value.summary,
            classification_confidence: value.classification_confidence,
            classification_explanation: value.classification_explanation,
        }
    }
}

impl From<ConceptType> for ConceptTypeDto {
    fn from(value: ConceptType) -> Self {
        match value {
            ConceptType::Document => Self::Document,
            ConceptType::Policy => Self::Policy,
            ConceptType::Procedure => Self::Procedure,
            ConceptType::Runbook => Self::Runbook,
            ConceptType::Reference => Self::Reference,
            ConceptType::Report => Self::Report,
        }
    }
}

impl From<ConceptTypeDto> for ConceptType {
    fn from(value: ConceptTypeDto) -> Self {
        match value {
            ConceptTypeDto::Document => Self::Document,
            ConceptTypeDto::Policy => Self::Policy,
            ConceptTypeDto::Procedure => Self::Procedure,
            ConceptTypeDto::Runbook => Self::Runbook,
            ConceptTypeDto::Reference => Self::Reference,
            ConceptTypeDto::Report => Self::Report,
        }
    }
}

impl From<SuggestedEntity> for SuggestedEntityDto {
    fn from(value: SuggestedEntity) -> Self {
        Self {
            name: value.name,
            kind: value.kind,
        }
    }
}

impl From<SuggestedEntityDto> for SuggestedEntity {
    fn from(value: SuggestedEntityDto) -> Self {
        Self {
            name: value.name,
            kind: value.kind,
        }
    }
}

impl From<SuggestedLink> for SuggestedLinkDto {
    fn from(value: SuggestedLink) -> Self {
        Self {
            label: value.label,
            target: value.target,
        }
    }
}

impl From<SuggestedLinkDto> for SuggestedLink {
    fn from(value: SuggestedLinkDto) -> Self {
        Self {
            label: value.label,
            target: value.target,
        }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct ReviewEvidenceSummary {
    request_id: String,
    concept_id: String,
    source_revision: u32,
    status: ReviewEvidenceStatus,
    excerpts: Vec<ReviewExcerptSummary>,
    total_chunks: usize,
    next_ordinal: Option<u32>,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum ReviewEvidenceStatus {
    Ready,
    Stale,
    Missing,
    Failed,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct ReviewExcerptSummary {
    ordinal: u32,
    heading_or_page: String,
    text: String,
    truncated: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct KnowledgeBundleSummary {
    collection_id: String,
    collection_name: String,
    status: KnowledgeBundleStatus,
    concepts: Vec<KnowledgeConceptSummary>,
    error_count: usize,
    warning_count: usize,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum KnowledgeBundleStatus {
    Empty,
    Ready,
    Updating,
    Failed,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct KnowledgeConceptSummary {
    page: KnowledgePageInput,
    title: String,
    description: String,
    concept_type: String,
    tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct KnowledgePageSummary {
    collection_id: String,
    page: KnowledgePageInput,
    title: String,
    status: KnowledgePageStatus,
    blocks: Vec<KnowledgeBlock>,
    metadata: Vec<(String, String)>,
    backlinks: Vec<KnowledgePageInput>,
    truncated: bool,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum KnowledgePageStatus {
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum KnowledgePageInput {
    Index,
    Log,
    Concept { id: Uuid },
}

impl From<KnowledgePageId> for KnowledgePageInput {
    fn from(value: KnowledgePageId) -> Self {
        match value {
            KnowledgePageId::Index => Self::Index,
            KnowledgePageId::Log => Self::Log,
            KnowledgePageId::Concept(id) => Self::Concept { id },
        }
    }
}

impl From<KnowledgePageInput> for KnowledgePageId {
    fn from(value: KnowledgePageInput) -> Self {
        match value {
            KnowledgePageInput::Index => Self::Index,
            KnowledgePageInput::Log => Self::Log,
            KnowledgePageInput::Concept { id } => Self::Concept(id),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum KnowledgeBlock {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
    },
    ListItem {
        ordered: bool,
        text: String,
    },
    Code {
        language: Option<String>,
        text: String,
    },
    Quote {
        text: String,
    },
    Rule,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct SourceIssueSummary {
    collection_id: String,
    source_name: String,
    collection_name: String,
    code: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct PeerSummary {
    peer_id: String,
    device_name: Option<String>,
    trust: PeerTrust,
    activity: PeerActivity,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum PeerTrust {
    Unpaired,
    Trusted,
    Blocked,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum PeerActivity {
    NotObserved,
    Discovered,
    Pairing,
    Connected,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct ModelSummary {
    display_name: Option<String>,
    active: bool,
    installed: bool,
    degraded: bool,
    #[ts(type = "number")]
    download_bytes: u64,
    #[ts(type = "number")]
    required_free_bytes: u64,
    fits_available_disk: bool,
    license_accepted: bool,
    license: Option<String>,
    license_url: Option<String>,
    revision: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct ModelInstallSummary {
    status: ModelInstallStatus,
    #[ts(type = "number")]
    downloaded: u64,
    #[ts(type = "number")]
    total_bytes: u64,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum ModelInstallStatus {
    Downloading,
    Verifying,
    Extracting,
    Activating,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct SearchSummary {
    request_id: String,
    status: SearchStatus,
    hits: Vec<SearchHitSummary>,
    coverage: SearchCoverage,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum SearchStatus {
    Searching,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum SearchCoverage {
    Complete,
    FederationDisabled,
    OfflineDevices,
    PublicNetworkOffline,
    Partial,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct SearchHitSummary {
    title: String,
    snippet: String,
    heading_or_page: String,
    logical_resource_uri: String,
    rank: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct NoticeSummary {
    level: NoticeLevel,
    message: String,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum NoticeLevel {
    Notice,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
struct PreferencesSummary {
    completed_onboarding_version: Option<u32>,
    locale: LocalePreferenceDto,
    lan_preference: LanPreferenceDto,
    close_behavior: CloseBehaviorDto,
    automatic_update_checks: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
struct PreferencesInput {
    locale: LocalePreferenceDto,
    lan_preference: LanPreferenceDto,
    close_behavior: CloseBehaviorDto,
    automatic_update_checks: bool,
    complete_onboarding: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "LocalePreference", rename_all = "snake_case")]
enum LocalePreferenceDto {
    System,
    Es,
    En,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "LanPreference", rename_all = "snake_case")]
enum LanPreferenceDto {
    Undecided,
    Disabled,
    Enabled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "CloseBehavior", rename_all = "snake_case")]
enum CloseBehaviorDto {
    Ask,
    HideToTray,
    Quit,
}

impl From<LocalePreference> for LocalePreferenceDto {
    fn from(value: LocalePreference) -> Self {
        match value {
            LocalePreference::System => Self::System,
            LocalePreference::Es => Self::Es,
            LocalePreference::En => Self::En,
        }
    }
}

impl From<LocalePreferenceDto> for LocalePreference {
    fn from(value: LocalePreferenceDto) -> Self {
        match value {
            LocalePreferenceDto::System => Self::System,
            LocalePreferenceDto::Es => Self::Es,
            LocalePreferenceDto::En => Self::En,
        }
    }
}

impl From<LanPreference> for LanPreferenceDto {
    fn from(value: LanPreference) -> Self {
        match value {
            LanPreference::Undecided => Self::Undecided,
            LanPreference::Disabled => Self::Disabled,
            LanPreference::Enabled => Self::Enabled,
        }
    }
}

impl From<LanPreferenceDto> for LanPreference {
    fn from(value: LanPreferenceDto) -> Self {
        match value {
            LanPreferenceDto::Undecided => Self::Undecided,
            LanPreferenceDto::Disabled => Self::Disabled,
            LanPreferenceDto::Enabled => Self::Enabled,
        }
    }
}

impl From<CloseBehavior> for CloseBehaviorDto {
    fn from(value: CloseBehavior) -> Self {
        match value {
            CloseBehavior::Ask => Self::Ask,
            CloseBehavior::HideToTray => Self::HideToTray,
            CloseBehavior::Quit => Self::Quit,
        }
    }
}

impl From<CloseBehaviorDto> for CloseBehavior {
    fn from(value: CloseBehaviorDto) -> Self {
        match value {
            CloseBehaviorDto::Ask => Self::Ask,
            CloseBehaviorDto::HideToTray => Self::HideToTray,
            CloseBehaviorDto::Quit => Self::Quit,
        }
    }
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct UiEventEnvelope {
    schema_version: u16,
    #[ts(type = "number")]
    sequence: u64,
    kind: UiEventKind,
    snapshot: AppSnapshot,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum UiEventKind {
    StateChanged,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct UiError {
    code: &'static str,
    message_key: &'static str,
    retryable: bool,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct FolderSelection {
    token: String,
    display_path: String,
}

#[derive(Debug, Deserialize, TS)]
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
                    kind: UiEventKind::StateChanged,
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
fn install_models(
    runtime: tauri::State<'_, AppRuntime>,
    licenses_confirmed: bool,
) -> Result<(), UiError> {
    if !licenses_confirmed {
        return Err(UiError::invalid("modelLicensesMustBeConfirmed"));
    }
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
    let request_id = parse_uuid(&request_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .search = Some(request_id);
    send_command(
        &runtime,
        WorkerCommand::Search {
            request_id,
            question,
            top_k,
            purpose: SearchPurpose::LocalAssistant,
            public_network,
        },
    )
}

#[tauri::command]
fn load_review_evidence(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    concept_id: String,
    source_revision: u32,
    after_ordinal: Option<u32>,
) -> Result<(), UiError> {
    let concept_id = parse_uuid(&concept_id)?;
    let request_id = parse_uuid(&request_id)?;
    let expected_review_version = runtime
        .review_versions
        .lock()
        .map_err(|_| UiError::internal())?
        .get(&concept_id)
        .filter(|cached| cached.source_revision == source_revision)
        .map(|cached| cached.token.clone());
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .review_evidence
        .insert(concept_id, request_id);
    send_command(
        &runtime,
        WorkerCommand::LoadReviewEvidence {
            request_id,
            concept_id,
            expected_source_revision: source_revision,
            expected_review_version,
            after_ordinal,
        },
    )
}

#[tauri::command]
fn approve_review(
    runtime: tauri::State<'_, AppRuntime>,
    concept_id: String,
    source_revision: u32,
    draft: EnrichmentDraftDto,
) -> Result<(), UiError> {
    let concept_id = parse_uuid(&concept_id)?;
    let expected_review_version = approval_version(&runtime, concept_id, source_revision)?;
    send_command(
        &runtime,
        WorkerCommand::Approve {
            concept_id,
            expected_review_version,
            draft: draft.into(),
        },
    )
}

fn approval_version(
    runtime: &AppRuntime,
    concept_id: Uuid,
    source_revision: u32,
) -> Result<ReviewVersionToken, UiError> {
    runtime
        .review_versions
        .lock()
        .map_err(|_| UiError::internal())?
        .get(&concept_id)
        .filter(|cached| cached.source_revision == source_revision)
        .map(|cached| cached.token.clone())
        .ok_or_else(|| UiError::invalid("currentEvidenceRequired"))
}

#[tauri::command]
fn reject_review(runtime: tauri::State<'_, AppRuntime>, concept_id: String) -> Result<(), UiError> {
    send_command(
        &runtime,
        WorkerCommand::Reject {
            concept_id: parse_uuid(&concept_id)?,
        },
    )
}

#[tauri::command]
fn reanalyze_review(
    runtime: tauri::State<'_, AppRuntime>,
    concept_id: String,
) -> Result<(), UiError> {
    send_command(
        &runtime,
        WorkerCommand::ReanalyzeReview {
            concept_id: parse_uuid(&concept_id)?,
        },
    )
}

#[tauri::command]
fn load_knowledge_bundle(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    collection_id: String,
) -> Result<(), UiError> {
    let request_id = parse_uuid(&request_id)?;
    let collection_id = parse_uuid(&collection_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .knowledge_bundle
        .insert(collection_id, request_id);
    send_command(
        &runtime,
        WorkerCommand::LoadKnowledgeBundle {
            request_id,
            collection_id,
        },
    )
}

#[tauri::command]
fn load_knowledge_page(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    collection_id: String,
    page: KnowledgePageInput,
) -> Result<(), UiError> {
    let collection_id = parse_uuid(&collection_id)?;
    let request_id = parse_uuid(&request_id)?;
    let page_id = KnowledgePageId::from(page);
    let expected_fingerprint = runtime
        .knowledge_fingerprints
        .lock()
        .map_err(|_| UiError::internal())?
        .get(&(collection_id, page_id))
        .cloned()
        .ok_or_else(|| UiError::invalid("currentKnowledgeSnapshotRequired"))?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .knowledge_page
        .insert(collection_id, request_id);
    send_command(
        &runtime,
        WorkerCommand::LoadKnowledgePage {
            request_id,
            collection_id,
            page_id,
            expected_fingerprint,
        },
    )
}

#[tauri::command]
fn update_preferences(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    preferences: PreferencesInput,
) -> Result<(), UiError> {
    let request_id = parse_uuid(&request_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .preferences = Some(request_id);
    send_command(
        &runtime,
        WorkerCommand::UpdateDesktopPreferences {
            request_id,
            update: DesktopPreferencesUpdate {
                locale: preferences.locale.into(),
                lan_preference: preferences.lan_preference.into(),
                close_behavior: preferences.close_behavior.into(),
                automatic_update_checks: preferences.automatic_update_checks,
                complete_onboarding: preferences.complete_onboarding,
            },
        },
    )
}

#[tauri::command]
fn hide_to_tray(app: AppHandle, runtime: tauri::State<'_, AppRuntime>) -> Result<(), UiError> {
    if !runtime.tray_operational.load(Ordering::Acquire) {
        return Err(UiError::invalid("trayUnavailable"));
    }
    app.get_webview_window("main")
        .ok_or_else(UiError::internal)?
        .hide()
        .map_err(|_| UiError::internal())
}

#[tauri::command]
fn quit_completely(app: AppHandle) {
    begin_shutdown(app);
}

fn begin_shutdown(app: AppHandle) {
    let runtime = app.state::<AppRuntime>();
    if runtime.exiting.swap(true, Ordering::AcqRel) {
        return;
    }
    let commands = runtime.commands.clone();
    let finished = runtime
        .worker_finished
        .lock()
        .ok()
        .and_then(|mut receiver| receiver.take());
    tauri::async_runtime::spawn(async move {
        let shutdown = async {
            let _ = commands.send(WorkerCommand::Shutdown).await;
            if let Some(finished) = finished {
                let _ = finished.await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(2), shutdown)
            .await
            .is_err()
        {
            tracing::warn!(error_kind = "shutdown_timeout", "shutdown deadline elapsed");
        }
        app.exit(0);
    });
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Abrir AirWiki", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Salir completamente", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::new()
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => begin_shutdown(app.clone()),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { .. }) {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseAction {
    Hide,
    Prompt,
    Quit,
}

const fn close_action(preference: CloseBehavior, tray_operational: bool) -> CloseAction {
    match (preference, tray_operational) {
        (CloseBehavior::HideToTray, true) => CloseAction::Hide,
        (CloseBehavior::Ask, true) => CloseAction::Prompt,
        (CloseBehavior::HideToTray | CloseBehavior::Ask | CloseBehavior::Quit, _) => {
            CloseAction::Quit
        }
    }
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
            phase: AppPhase::Starting,
            collections: Vec::new(),
            reviews: Vec::new(),
            source_issues: Vec::new(),
            peers: Vec::new(),
            model: None,
            model_install: None,
            search: None,
            review_evidence: None,
            knowledge: None,
            knowledge_page: None,
            preferences: None,
            notice: None,
        }
    }

    async fn apply(
        &mut self,
        event: WorkerEvent,
        review_versions: &Mutex<HashMap<Uuid, CachedReviewVersion>>,
        knowledge_fingerprints: &Mutex<HashMap<(Uuid, KnowledgePageId), String>>,
        requests: &Mutex<RequestTracker>,
    ) {
        if !request_is_current(&event, requests) {
            return;
        }
        self.sequence = self.sequence.saturating_add(1);
        match event {
            WorkerEvent::Ready {
                collections,
                reviews,
                source_issues,
                ..
            } => {
                self.phase = AppPhase::Ready;
                self.collections = collections
                    .into_iter()
                    .map(CollectionSummary::from)
                    .collect();
                self.reviews = reviews.into_iter().map(ReviewSummary::from).collect();
                retain_pending_review_versions(review_versions, &self.reviews);
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
                retain_pending_review_versions(review_versions, &self.reviews);
            }
            WorkerEvent::SourceIssues(issues) => {
                self.source_issues = issues.into_iter().map(SourceIssueSummary::from).collect();
            }
            WorkerEvent::Peers(peers) => {
                self.peers = peers.into_iter().map(PeerSummary::from).collect();
            }
            WorkerEvent::ModelState(model) => self.model = Some(ModelSummary::from(model)),
            WorkerEvent::InstallProgress(progress) => {
                self.model_install = Some(match progress {
                    airwiki_inference::InstallEvent::Started { total_bytes, .. } => {
                        ModelInstallSummary {
                            status: ModelInstallStatus::Downloading,
                            downloaded: 0,
                            total_bytes,
                        }
                    }
                    airwiki_inference::InstallEvent::Progress {
                        downloaded,
                        total_bytes,
                        ..
                    } => ModelInstallSummary {
                        status: ModelInstallStatus::Downloading,
                        downloaded,
                        total_bytes,
                    },
                    airwiki_inference::InstallEvent::Verifying { .. } => ModelInstallSummary {
                        status: ModelInstallStatus::Verifying,
                        downloaded: 0,
                        total_bytes: 0,
                    },
                    airwiki_inference::InstallEvent::Extracting { .. } => ModelInstallSummary {
                        status: ModelInstallStatus::Extracting,
                        downloaded: 0,
                        total_bytes: 0,
                    },
                    airwiki_inference::InstallEvent::Complete { .. } => ModelInstallSummary {
                        status: ModelInstallStatus::Activating,
                        downloaded: 0,
                        total_bytes: 0,
                    },
                });
            }
            WorkerEvent::InstallStopped | WorkerEvent::ModelsReady => self.model_install = None,
            WorkerEvent::DesktopPreferencesUpdated { result, .. } => match result {
                Ok(preferences) => {
                    self.preferences = Some(PreferencesSummary {
                        completed_onboarding_version: preferences.completed_onboarding_version,
                        locale: preferences.locale.into(),
                        lan_preference: preferences.lan_preference.into(),
                        close_behavior: preferences.close_behavior.into(),
                        automatic_update_checks: preferences.automatic_update_checks,
                    });
                }
                Err(_) => {
                    self.notice = Some(NoticeSummary {
                        level: NoticeLevel::Error,
                        message: "preferences-update-failed".to_owned(),
                    });
                }
            },
            WorkerEvent::ReviewEvidenceLoaded {
                request_id,
                concept_id,
                expected_source_revision,
                result,
            } => {
                self.review_evidence = Some(match result {
                    Ok(page) => {
                        if let Ok(mut versions) = review_versions.lock() {
                            versions.insert(
                                concept_id,
                                CachedReviewVersion {
                                    source_revision: page.source_revision,
                                    token: page.review_version,
                                },
                            );
                        }
                        let mut summary = ReviewEvidenceSummary {
                            request_id: request_id.to_string(),
                            concept_id: concept_id.to_string(),
                            source_revision: page.source_revision,
                            status: ReviewEvidenceStatus::Ready,
                            excerpts: page
                                .excerpts
                                .into_iter()
                                .map(ReviewExcerptSummary::from)
                                .collect(),
                            total_chunks: page.total_chunks,
                            next_ordinal: page.next_ordinal,
                        };
                        if summary
                            .excerpts
                            .first()
                            .is_some_and(|excerpt| excerpt.ordinal > 0)
                            && let Some(previous) = self.review_evidence.as_ref()
                            && previous.concept_id == summary.concept_id
                            && previous.source_revision == summary.source_revision
                        {
                            let mut excerpts = previous.excerpts.clone();
                            excerpts.extend(summary.excerpts);
                            summary.excerpts = excerpts;
                        }
                        summary
                    }
                    Err(error) => {
                        if let Ok(mut versions) = review_versions.lock() {
                            versions.remove(&concept_id);
                        }
                        ReviewEvidenceSummary {
                            request_id: request_id.to_string(),
                            concept_id: concept_id.to_string(),
                            source_revision: expected_source_revision,
                            status: match error {
                                worker::ReviewEvidenceErrorView::NoLongerPending => {
                                    ReviewEvidenceStatus::Stale
                                }
                                worker::ReviewEvidenceErrorView::MissingEvidence => {
                                    ReviewEvidenceStatus::Missing
                                }
                                worker::ReviewEvidenceErrorView::Unavailable => {
                                    ReviewEvidenceStatus::Failed
                                }
                            },
                            excerpts: Vec::new(),
                            total_chunks: 0,
                            next_ordinal: None,
                        }
                    }
                });
            }
            WorkerEvent::KnowledgeBundleLoaded {
                collection_id,
                result,
                ..
            } => match result {
                Ok(bundle) if bundle.collection_id == collection_id => {
                    if let Ok(mut fingerprints) = knowledge_fingerprints.lock() {
                        fingerprints.retain(|(cached_collection, _), _| {
                            *cached_collection != collection_id
                        });
                        if let Some(fingerprint) = bundle.index_fingerprint.as_ref() {
                            fingerprints.insert(
                                (collection_id, KnowledgePageId::Index),
                                fingerprint.clone(),
                            );
                        }
                        if let Some(fingerprint) = bundle.log_fingerprint.as_ref() {
                            fingerprints
                                .insert((collection_id, KnowledgePageId::Log), fingerprint.clone());
                        }
                        for concept in &bundle.concepts {
                            fingerprints.insert(
                                (collection_id, KnowledgePageId::Concept(concept.id)),
                                concept.fingerprint.clone(),
                            );
                        }
                    }
                    self.knowledge = Some(KnowledgeBundleSummary {
                        collection_id: collection_id.to_string(),
                        collection_name: bundle.collection_name,
                        status: match bundle.state {
                            KnowledgeBundleState::Empty => KnowledgeBundleStatus::Empty,
                            KnowledgeBundleState::Ready => KnowledgeBundleStatus::Ready,
                            KnowledgeBundleState::Updating => KnowledgeBundleStatus::Updating,
                        },
                        concepts: bundle
                            .concepts
                            .into_iter()
                            .map(|concept| KnowledgeConceptSummary {
                                page: KnowledgePageInput::Concept { id: concept.id },
                                title: concept.title,
                                description: concept.description,
                                concept_type: concept.concept_type,
                                tags: concept.tags,
                            })
                            .collect(),
                        error_count: bundle.health.error_count,
                        warning_count: bundle.health.warning_count,
                    });
                    self.knowledge_page = None;
                }
                _ => {
                    if let Ok(mut fingerprints) = knowledge_fingerprints.lock() {
                        fingerprints.retain(|(cached_collection, _), _| {
                            *cached_collection != collection_id
                        });
                    }
                    self.knowledge = Some(KnowledgeBundleSummary {
                        collection_id: collection_id.to_string(),
                        collection_name: String::new(),
                        status: KnowledgeBundleStatus::Failed,
                        concepts: Vec::new(),
                        error_count: 1,
                        warning_count: 0,
                    });
                    self.knowledge_page = None;
                }
            },
            WorkerEvent::KnowledgePageLoaded {
                collection_id,
                page_id,
                result,
                ..
            } => {
                self.knowledge_page = Some(match result {
                    Ok(page) if page.collection_id == collection_id && page.page_id == page_id => {
                        match tokio::task::spawn_blocking(move || knowledge_page_summary(page))
                            .await
                        {
                            Ok(summary) => summary,
                            Err(_) => failed_knowledge_page(collection_id, page_id),
                        }
                    }
                    _ => failed_knowledge_page(collection_id, page_id),
                });
            }
            WorkerEvent::SearchPartial { request_id, hits } => {
                self.search = Some(SearchSummary {
                    request_id: request_id.to_string(),
                    status: SearchStatus::Searching,
                    hits: hits.into_iter().map(SearchHitSummary::from).collect(),
                    coverage: SearchCoverage::Partial,
                });
            }
            WorkerEvent::SearchFinished { request_id, result } => {
                self.search = Some(match result {
                    Ok((hits, coverage, _route)) => SearchSummary {
                        request_id: request_id.to_string(),
                        status: SearchStatus::Complete,
                        hits: hits.into_iter().map(SearchHitSummary::from).collect(),
                        coverage: match coverage {
                            worker::SearchCoverageView::Complete => SearchCoverage::Complete,
                            worker::SearchCoverageView::FederationDisabled => {
                                SearchCoverage::FederationDisabled
                            }
                            worker::SearchCoverageView::OfflineDevices { .. } => {
                                SearchCoverage::OfflineDevices
                            }
                            worker::SearchCoverageView::PublicNetworkOffline => {
                                SearchCoverage::PublicNetworkOffline
                            }
                            worker::SearchCoverageView::Partial => SearchCoverage::Partial,
                        },
                    },
                    Err(_) => SearchSummary {
                        request_id: request_id.to_string(),
                        status: SearchStatus::Failed,
                        hits: Vec::new(),
                        coverage: SearchCoverage::Partial,
                    },
                });
            }
            WorkerEvent::Notice(message) => {
                self.notice = Some(NoticeSummary {
                    level: NoticeLevel::Notice,
                    message,
                });
            }
            WorkerEvent::Error(_) => {
                self.notice = Some(NoticeSummary {
                    level: NoticeLevel::Error,
                    message: "runtime-operation-failed".to_owned(),
                });
            }
            _ => {}
        }
    }
}

fn request_is_current(event: &WorkerEvent, requests: &Mutex<RequestTracker>) -> bool {
    let Ok(mut requests) = requests.lock() else {
        return false;
    };
    match event {
        WorkerEvent::SearchPartial { request_id, .. } => requests.search == Some(*request_id),
        WorkerEvent::SearchFinished { request_id, .. } => {
            if requests.search == Some(*request_id) {
                requests.search = None;
                true
            } else {
                false
            }
        }
        WorkerEvent::ReviewEvidenceLoaded {
            request_id,
            concept_id,
            ..
        } => remove_matching_request(&mut requests.review_evidence, concept_id, request_id),
        WorkerEvent::KnowledgeBundleLoaded {
            request_id,
            collection_id,
            ..
        } => remove_matching_request(&mut requests.knowledge_bundle, collection_id, request_id),
        WorkerEvent::KnowledgePageLoaded {
            request_id,
            collection_id,
            ..
        } => remove_matching_request(&mut requests.knowledge_page, collection_id, request_id),
        WorkerEvent::DesktopPreferencesUpdated { request_id, .. } if request_id.is_nil() => true,
        WorkerEvent::DesktopPreferencesUpdated { request_id, .. } => {
            if requests.preferences == Some(*request_id) {
                requests.preferences = None;
                true
            } else {
                false
            }
        }
        _ => true,
    }
}

fn remove_matching_request(
    requests: &mut HashMap<Uuid, Uuid>,
    key: &Uuid,
    request_id: &Uuid,
) -> bool {
    if requests.get(key) == Some(request_id) {
        requests.remove(key);
        true
    } else {
        false
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
            draft: value.draft.into(),
        }
    }
}

impl From<worker::ReviewEvidenceExcerptView> for ReviewExcerptSummary {
    fn from(value: worker::ReviewEvidenceExcerptView) -> Self {
        Self {
            ordinal: value.ordinal,
            heading_or_page: value.heading_or_page,
            text: value.text,
            truncated: value.truncated,
        }
    }
}

fn retain_pending_review_versions(
    review_versions: &Mutex<HashMap<Uuid, CachedReviewVersion>>,
    reviews: &[ReviewSummary],
) {
    if let Ok(mut versions) = review_versions.lock() {
        versions.retain(|concept_id, cached| {
            let concept_id = concept_id.to_string();
            reviews.iter().any(|review| {
                review.concept_id == concept_id && review.source_revision == cached.source_revision
            })
        });
    }
}

fn failed_knowledge_page(collection_id: Uuid, page_id: KnowledgePageId) -> KnowledgePageSummary {
    KnowledgePageSummary {
        collection_id: collection_id.to_string(),
        page: page_id.into(),
        title: String::new(),
        status: KnowledgePageStatus::Failed,
        blocks: Vec::new(),
        metadata: Vec::new(),
        backlinks: Vec::new(),
        truncated: false,
    }
}

fn knowledge_page_summary(page: KnowledgePageView) -> KnowledgePageSummary {
    KnowledgePageSummary {
        collection_id: page.collection_id.to_string(),
        page: page.page_id.into(),
        title: page.title,
        status: KnowledgePageStatus::Ready,
        blocks: parse_knowledge_blocks(&page.body_markdown),
        metadata: page.metadata,
        backlinks: page.backlinks.into_iter().map(Into::into).collect(),
        truncated: page.truncated,
    }
}

enum BlockBuilder {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    ListItem {
        ordered: bool,
        text: String,
    },
    Code {
        language: Option<String>,
        text: String,
    },
    Quote(String),
}

impl BlockBuilder {
    fn text_mut(&mut self) -> &mut String {
        match self {
            Self::Heading { text, .. }
            | Self::Paragraph(text)
            | Self::ListItem { text, .. }
            | Self::Code { text, .. }
            | Self::Quote(text) => text,
        }
    }

    fn finish(self) -> Option<KnowledgeBlock> {
        let block = match self {
            Self::Heading { level, text } => KnowledgeBlock::Heading { level, text },
            Self::Paragraph(text) => KnowledgeBlock::Paragraph { text },
            Self::ListItem { ordered, text } => KnowledgeBlock::ListItem { ordered, text },
            Self::Code { language, text } => KnowledgeBlock::Code { language, text },
            Self::Quote(text) => KnowledgeBlock::Quote { text },
        };
        match &block {
            KnowledgeBlock::Heading { text, .. }
            | KnowledgeBlock::Paragraph { text }
            | KnowledgeBlock::ListItem { text, .. }
            | KnowledgeBlock::Code { text, .. }
            | KnowledgeBlock::Quote { text }
                if text.trim().is_empty() =>
            {
                None
            }
            _ => Some(block),
        }
    }
}

fn parse_knowledge_blocks(markdown: &str) -> Vec<KnowledgeBlock> {
    let mut blocks = Vec::new();
    let mut current = None;
    let mut list_types = Vec::new();
    let mut image_depth = 0_u16;

    for event in Parser::new(markdown) {
        match event {
            MarkdownEvent::Start(Tag::Image { .. }) => {
                image_depth = image_depth.saturating_add(1);
            }
            MarkdownEvent::End(TagEnd::Image) => {
                image_depth = image_depth.saturating_sub(1);
            }
            _ if image_depth > 0 => {}
            MarkdownEvent::Start(Tag::Heading { level, .. }) => {
                flush_block(&mut current, &mut blocks);
                current = Some(BlockBuilder::Heading {
                    level: heading_level(level),
                    text: String::new(),
                });
            }
            MarkdownEvent::End(TagEnd::Heading(_)) => flush_block(&mut current, &mut blocks),
            MarkdownEvent::Start(Tag::Paragraph) => {
                flush_block(&mut current, &mut blocks);
                current = Some(BlockBuilder::Paragraph(String::new()));
            }
            MarkdownEvent::End(TagEnd::Paragraph) => flush_block(&mut current, &mut blocks),
            MarkdownEvent::Start(Tag::List(first)) => list_types.push(first.is_some()),
            MarkdownEvent::End(TagEnd::List(_)) => {
                list_types.pop();
            }
            MarkdownEvent::Start(Tag::Item) => {
                flush_block(&mut current, &mut blocks);
                current = Some(BlockBuilder::ListItem {
                    ordered: list_types.last().copied().unwrap_or(false),
                    text: String::new(),
                });
            }
            MarkdownEvent::End(TagEnd::Item) => flush_block(&mut current, &mut blocks),
            MarkdownEvent::Start(Tag::CodeBlock(kind)) => {
                flush_block(&mut current, &mut blocks);
                let language = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(language) => {
                        let language = language.trim();
                        (!language.is_empty()).then(|| language.to_owned())
                    }
                };
                current = Some(BlockBuilder::Code {
                    language,
                    text: String::new(),
                });
            }
            MarkdownEvent::End(TagEnd::CodeBlock) => flush_block(&mut current, &mut blocks),
            MarkdownEvent::Start(Tag::BlockQuote(_)) => {
                flush_block(&mut current, &mut blocks);
                current = Some(BlockBuilder::Quote(String::new()));
            }
            MarkdownEvent::End(TagEnd::BlockQuote(_)) => flush_block(&mut current, &mut blocks),
            MarkdownEvent::Rule => {
                flush_block(&mut current, &mut blocks);
                blocks.push(KnowledgeBlock::Rule);
            }
            MarkdownEvent::Text(text) | MarkdownEvent::Code(text) => {
                if let Some(builder) = current.as_mut() {
                    builder.text_mut().push_str(&text);
                }
            }
            MarkdownEvent::SoftBreak | MarkdownEvent::HardBreak => {
                if let Some(builder) = current.as_mut() {
                    builder.text_mut().push('\n');
                }
            }
            MarkdownEvent::Html(_) | MarkdownEvent::InlineHtml(_) => {}
            _ => {}
        }
    }
    flush_block(&mut current, &mut blocks);
    blocks
}

fn flush_block(current: &mut Option<BlockBuilder>, blocks: &mut Vec<KnowledgeBlock>) {
    if let Some(block) = current.take().and_then(BlockBuilder::finish) {
        blocks.push(block);
    }
}

const fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn ui_bindings_source() -> String {
    let config = ts_rs::Config::default();
    let declarations = [
        exported_declaration::<ConceptTypeDto>(&config),
        exported_declaration::<SuggestedEntityDto>(&config),
        exported_declaration::<SuggestedLinkDto>(&config),
        exported_declaration::<EnrichmentDraftDto>(&config),
        exported_declaration::<CollectionSummary>(&config),
        exported_declaration::<ReviewSummary>(&config),
        exported_declaration::<ReviewExcerptSummary>(&config),
        exported_declaration::<ReviewEvidenceStatus>(&config),
        exported_declaration::<ReviewEvidenceSummary>(&config),
        exported_declaration::<KnowledgePageInput>(&config),
        exported_declaration::<KnowledgeBlock>(&config),
        exported_declaration::<KnowledgeConceptSummary>(&config),
        exported_declaration::<KnowledgeBundleStatus>(&config),
        exported_declaration::<KnowledgeBundleSummary>(&config),
        exported_declaration::<KnowledgePageStatus>(&config),
        exported_declaration::<KnowledgePageSummary>(&config),
        exported_declaration::<SourceIssueSummary>(&config),
        exported_declaration::<PeerTrust>(&config),
        exported_declaration::<PeerActivity>(&config),
        exported_declaration::<PeerSummary>(&config),
        exported_declaration::<ModelSummary>(&config),
        exported_declaration::<ModelInstallStatus>(&config),
        exported_declaration::<ModelInstallSummary>(&config),
        exported_declaration::<SearchHitSummary>(&config),
        exported_declaration::<SearchStatus>(&config),
        exported_declaration::<SearchCoverage>(&config),
        exported_declaration::<SearchSummary>(&config),
        exported_declaration::<NoticeLevel>(&config),
        exported_declaration::<NoticeSummary>(&config),
        exported_declaration::<LocalePreferenceDto>(&config),
        exported_declaration::<LanPreferenceDto>(&config),
        exported_declaration::<CloseBehaviorDto>(&config),
        exported_declaration::<PreferencesSummary>(&config),
        exported_declaration::<PreferencesInput>(&config),
        exported_declaration::<AppPhase>(&config),
        exported_declaration::<AppSnapshot>(&config),
        exported_declaration::<UiEventKind>(&config),
        exported_declaration::<UiEventEnvelope>(&config),
        exported_declaration::<UiError>(&config),
        exported_declaration::<FolderSelection>(&config),
        exported_declaration::<CollectionPolicyInput>(&config),
    ]
    .join("\n\n");
    format!(
        "// Generated by `cargo run --locked -p xtask -- ui-bindings generate`.\n// Do not edit by hand.\n\n{declarations}\n"
    )
}

fn exported_declaration<T: TS>(config: &ts_rs::Config) -> String {
    format!("export {}", T::decl(config))
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
                worker::PeerTrustState::Unpaired => PeerTrust::Unpaired,
                worker::PeerTrustState::Trusted => PeerTrust::Trusted,
                worker::PeerTrustState::Blocked => PeerTrust::Blocked,
            },
            activity: match value.activity {
                worker::PeerActivityState::NotObserved => PeerActivity::NotObserved,
                worker::PeerActivityState::Discovered => PeerActivity::Discovered,
                worker::PeerActivityState::Pairing => PeerActivity::Pairing,
                worker::PeerActivityState::Connected => PeerActivity::Connected,
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
            license: value.license,
            license_url: value.license_url,
            revision: value.revision,
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
    let review_versions = Arc::new(Mutex::new(HashMap::new()));
    let presentation_review_versions = Arc::clone(&review_versions);
    let knowledge_fingerprints = Arc::new(Mutex::new(HashMap::new()));
    let presentation_knowledge_fingerprints = Arc::clone(&knowledge_fingerprints);
    let requests = Arc::new(Mutex::new(RequestTracker::default()));
    let presentation_requests = Arc::clone(&requests);
    let (worker_finished_sender, worker_finished) = oneshot::channel();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .manage(AppRuntime {
            commands,
            snapshot: Mutex::new(snapshot_receiver),
            folder_selections: Mutex::new(HashMap::new()),
            review_versions,
            knowledge_fingerprints,
            requests,
            tray_operational: AtomicBool::new(false),
            exiting: AtomicBool::new(false),
            worker_finished: Mutex::new(Some(worker_finished)),
        })
        .setup(move |app| {
            if install_tray(app).is_ok() {
                app.state::<AppRuntime>()
                    .tray_operational
                    .store(true, Ordering::Release);
            } else {
                tracing::warn!(
                    error_kind = "tray_unavailable",
                    "tray initialization failed"
                );
            }
            tauri::async_runtime::spawn(async move {
                let mut snapshot = AppSnapshot::starting();
                loop {
                    match presentation_events.recv().await {
                        Ok(event) => {
                            snapshot
                                .apply(
                                    event,
                                    &presentation_review_versions,
                                    &presentation_knowledge_fingerprints,
                                    &presentation_requests,
                                )
                                .await;
                            if snapshot_sender.send(snapshot.clone()).is_err() {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            snapshot.notice = Some(NoticeSummary {
                                level: NoticeLevel::Warning,
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
            tauri::async_runtime::spawn(async move {
                run_worker(paths, command_receiver, worker_events).await;
                let _ = worker_finished_sender.send(());
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let runtime = app.state::<AppRuntime>();
                if runtime.exiting.load(Ordering::Acquire) {
                    return;
                }
                api.prevent_close();
                let close_behavior = runtime
                    .snapshot
                    .lock()
                    .ok()
                    .and_then(|snapshot| snapshot.borrow().preferences)
                    .map_or(CloseBehavior::Ask, |preferences| {
                        preferences.close_behavior.into()
                    });
                match close_action(
                    close_behavior,
                    runtime.tray_operational.load(Ordering::Acquire),
                ) {
                    CloseAction::Hide => {
                        let _ = window.hide();
                    }
                    CloseAction::Prompt => {
                        let _ = window.emit("close-choice-required", ());
                    }
                    CloseAction::Quit => {
                        begin_shutdown(app.clone());
                    }
                }
            }
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
            search,
            load_review_evidence,
            approve_review,
            reject_review,
            reanalyze_review,
            load_knowledge_bundle,
            load_knowledge_page,
            update_preferences,
            hide_to_tray,
            quit_completely
        ])
        .run(tauri::generate_context!())
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI_BINDINGS_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/ui/src/generated/ui-contract.ts"
    );

    fn runtime_with_selection(token: Uuid, path: PathBuf, expires_at: Instant) -> AppRuntime {
        let (commands, _receiver) = mpsc::channel(COMMAND_CAPACITY);
        let (_snapshot_sender, snapshot) = watch::channel(AppSnapshot::starting());
        let (_worker_finished_sender, worker_finished) = oneshot::channel();
        AppRuntime {
            commands,
            snapshot: Mutex::new(snapshot),
            folder_selections: Mutex::new(HashMap::from([(
                token,
                PendingFolderSelection { path, expires_at },
            )])),
            review_versions: Arc::new(Mutex::new(HashMap::new())),
            knowledge_fingerprints: Arc::new(Mutex::new(HashMap::new())),
            requests: Arc::new(Mutex::new(RequestTracker::default())),
            tray_operational: AtomicBool::new(false),
            exiting: AtomicBool::new(false),
            worker_finished: Mutex::new(Some(worker_finished)),
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
    fn enrichment_draft_dto_round_trips_without_contract_drift() -> Result<()> {
        let draft = EnrichmentDraft {
            concept_type: ConceptType::Runbook,
            title: "Synthetic recovery".to_owned(),
            description: "A synthetic review fixture".to_owned(),
            language: "en".to_owned(),
            tags: vec!["recovery".to_owned()],
            entities: vec![SuggestedEntity {
                name: "AirWiki".to_owned(),
                kind: "application".to_owned(),
            }],
            links: vec![SuggestedLink {
                label: "Evidence".to_owned(),
                target: "okf://synthetic/evidence".to_owned(),
            }],
            summary: "Synthetic summary".to_owned(),
            classification_confidence: 0.9,
            classification_explanation: "Synthetic fixture".to_owned(),
        };

        let dto = EnrichmentDraftDto::from(draft.clone());
        let serialized = serde_json::to_value(&dto)?;
        anyhow::ensure!(serialized.get("classificationConfidence").is_some());
        let round_trip = EnrichmentDraft::from(dto);

        assert_eq!(
            serde_json::to_value(round_trip)?,
            serde_json::to_value(draft)?
        );
        Ok(())
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

    #[test]
    fn approval_without_loaded_evidence_fails_closed() {
        let token = Uuid::new_v4();
        let runtime = runtime_with_selection(token, PathBuf::new(), Instant::now());
        let error = approval_version(&runtime, Uuid::new_v4(), 1).unwrap_err();

        assert_eq!(error.message_key, "currentEvidenceRequired");
    }

    #[test]
    fn approval_rejects_evidence_from_an_older_source_revision() -> Result<(), &'static str> {
        let concept_id = Uuid::new_v4();
        let token = Uuid::new_v4();
        let runtime = runtime_with_selection(token, PathBuf::new(), Instant::now());
        {
            let mut versions = runtime
                .review_versions
                .lock()
                .map_err(|_| "poisoned lock")?;
            versions.insert(
                concept_id,
                CachedReviewVersion {
                    source_revision: 4,
                    token: ReviewVersionToken::from_digest([7; 32]),
                },
            );
        }

        let error = approval_version(&runtime, concept_id, 5).unwrap_err();

        assert_eq!(error.message_key, "currentEvidenceRequired");
        Ok(())
    }

    #[test]
    fn markdown_contract_excludes_html_and_images() {
        let blocks = parse_knowledge_blocks(
            "# Visible\n\nHello <script>alert(1)</script> world.\n\n![secret](https://example.test/a.png)",
        );

        assert_eq!(
            blocks,
            vec![
                KnowledgeBlock::Heading {
                    level: 1,
                    text: "Visible".to_owned(),
                },
                KnowledgeBlock::Paragraph {
                    text: "Hello alert(1) world.".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn markdown_contract_keeps_code_as_text() {
        let blocks = parse_knowledge_blocks("```rust\nfn main() {}\n```");

        assert_eq!(
            blocks,
            vec![KnowledgeBlock::Code {
                language: Some("rust".to_owned()),
                text: "fn main() {}\n".to_owned(),
            }]
        );
    }

    #[test]
    fn stale_search_events_are_discarded() {
        let current = Uuid::new_v4();
        let stale = Uuid::new_v4();
        let requests = Mutex::new(RequestTracker {
            search: Some(current),
            ..RequestTracker::default()
        });

        assert!(!request_is_current(
            &WorkerEvent::SearchPartial {
                request_id: stale,
                hits: Vec::new(),
            },
            &requests,
        ));
        assert!(request_is_current(
            &WorkerEvent::SearchFinished {
                request_id: current,
                result: Err("synthetic".to_owned()),
            },
            &requests,
        ));
        assert!(!request_is_current(
            &WorkerEvent::SearchPartial {
                request_id: current,
                hits: Vec::new(),
            },
            &requests,
        ));
    }

    #[test]
    fn tray_failure_never_leaves_an_inaccessible_process() {
        assert_eq!(
            close_action(CloseBehavior::HideToTray, false),
            CloseAction::Quit
        );
        assert_eq!(close_action(CloseBehavior::Ask, false), CloseAction::Quit);
        assert_eq!(
            close_action(CloseBehavior::HideToTray, true),
            CloseAction::Hide
        );
    }

    #[test]
    fn ui_bindings_match_committed_file() -> Result<()> {
        let committed = std::fs::read_to_string(UI_BINDINGS_PATH)
            .context("committed UI bindings are missing")?;
        anyhow::ensure!(
            committed == ui_bindings_source(),
            "UI bindings are stale; run `cargo run --locked -p xtask -- ui-bindings generate`"
        );
        Ok(())
    }

    #[test]
    #[ignore = "writes the committed TypeScript contract"]
    fn generate_ui_bindings() -> Result<()> {
        let path = PathBuf::from(UI_BINDINGS_PATH);
        let parent = path.parent().context("UI bindings path has no parent")?;
        std::fs::create_dir_all(parent).context("failed to create UI bindings directory")?;
        std::fs::write(path, ui_bindings_source()).context("failed to write UI bindings")
    }
}
