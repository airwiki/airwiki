#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[cfg(all(feature = "e2e", not(debug_assertions)))]
compile_error!("the desktop e2e secret store must never be compiled into a release build");

mod autostart;
mod connectivity_platform;
mod external_navigation;
mod i18n;
mod integrations;
mod manual_lan_route;
mod model_activation_status;
mod model_config;
mod paths;
mod services;
mod updater;
mod worker;

use std::{
    collections::HashMap,
    ffi::OsStr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use airwiki_core::{
    CollectionMaintenanceStatus, GuidedRepairChange, GuidedRepairPreview, KnowledgeBundleState,
    KnowledgeLinkDisposition, KnowledgeLinkView, KnowledgePageId, KnowledgePageView,
    RepairAuthority, ReviewVersionToken,
};
use airwiki_inference::ModelProfile;
use airwiki_types::{ConceptType, EnrichmentDraft, SearchPurpose, SuggestedEntity, SuggestedLink};
use anyhow::{Context, Result};
use pulldown_cmark::{CodeBlockKind, Event as MarkdownEvent, HeadingLevel, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, WindowEvent,
    image::Image,
    ipc::Channel,
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::{Semaphore, broadcast, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;
use uuid::Uuid;

use crate::{
    connectivity_platform::SystemDestination,
    i18n::{Localization, UiLocale},
    model_config::{CloseBehavior, LanPreference, LocalePreference, ThemePreference},
    paths::AppPaths,
    updater::{
        TauriUpdateBackend, UpdateBackend, UpdateIssueCode, UpdateSummary, UpdaterBuildConfig,
        UpdaterDisabledReason, UpdaterStatus, UpdaterView,
    },
    worker::{DesktopPreferencesUpdate, WorkerCommand, WorkerEvent, WorkerIntent, run_worker},
};

const COMMAND_CAPACITY: usize = 64;
const INTERNAL_EVENT_CAPACITY: usize = 256;
const TRANSIENT_EVENT_CAPACITY: usize = 128;
const CONTRACT_VERSION: u16 = 1;
const FOLDER_SELECTION_TTL: Duration = Duration::from_secs(5 * 60);
const TRAY_ICON_WIDTH: u32 = 24;
const TRAY_ICON_HEIGHT: u32 = 24;
const TRAY_ICON_RGBA: &[u8; 2_304] =
    include_bytes!("../../../resources/branding/airwiki-tray.rgba");

#[derive(Clone, Copy)]
enum NativeConfirmation {
    ModelLicenses,
    ExternalLink,
    GuidedRepair,
    ExternalCollectionPolicy,
    CollectionGrant,
    DeleteWiki,
    InstallUpdate,
}

impl NativeConfirmation {
    const fn message_id(self) -> &'static str {
        match self {
            Self::ModelLicenses => "native-confirm-model-licenses",
            Self::ExternalLink => "native-confirm-external-link",
            Self::GuidedRepair => "native-confirm-guided-repair",
            Self::ExternalCollectionPolicy => "native-confirm-external-policy",
            Self::CollectionGrant => "native-confirm-collection-grant",
            Self::DeleteWiki => "native-confirm-delete-wiki",
            Self::InstallUpdate => "native-confirm-install-update",
        }
    }
}

async fn require_native_confirmation(
    app: &AppHandle,
    confirmation: NativeConfirmation,
    detail: Option<&str>,
) -> Result<(), UiError> {
    let _permit = app
        .state::<AppRuntime>()
        .confirmation_gate
        .clone()
        .try_acquire_owned()
        .map_err(|_| UiError::busy("humanConfirmationAlreadyOpen"))?;
    let (title, mut description) = {
        let localization =
            Localization::new(UiLocale::from_system()).map_err(|_| UiError::internal())?;
        let title = localization
            .text("native-confirm-title")
            .ok_or_else(UiError::internal)?;
        let description = localization
            .text(confirmation.message_id())
            .ok_or_else(UiError::internal)?;
        (title, description)
    };
    if let Some(detail) = detail {
        description.push_str("\n\n");
        description.push_str(detail);
    }
    let mut dialog = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title(title)
        .set_description(description)
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    if matches!(dialog.show().await, rfd::MessageDialogResult::Yes) {
        Ok(())
    } else {
        Err(UiError::invalid("humanConfirmationRequired"))
    }
}

fn launch_in_background<I>(arguments: I) -> bool
where
    I: IntoIterator,
    I::Item: AsRef<OsStr>,
{
    arguments
        .into_iter()
        .any(|argument| argument.as_ref() == OsStr::new("--background"))
}

fn navigation_is_allowed(url: &url::Url) -> bool {
    let packaged_origin = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"));
    let initial_blank = url.scheme() == "about" && url.path() == "blank";
    let development_origin = cfg!(debug_assertions)
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
        && url.port() == Some(1420);
    packaged_origin || initial_blank || development_origin
}

fn navigation_guard() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_navigation(|_, url| navigation_is_allowed(url))
        .build()
}

fn init_logging(paths: &AppPaths) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    std::fs::create_dir_all(&paths.logs).context("failed to create the sanitized log directory")?;
    let file = tracing_appender::rolling::daily(&paths.logs, "airwiki.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "airwiki=info,airwiki_=info,warn".into()),
        )
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .try_init()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(guard)
}

struct AppRuntime {
    commands: mpsc::Sender<WorkerIntent>,
    cancellation: CancellationToken,
    snapshot: Mutex<watch::Receiver<PublishedSnapshot>>,
    folder_selections: Mutex<HashMap<Uuid, PendingFolderSelection>>,
    review_versions: Arc<Mutex<HashMap<Uuid, CachedReviewVersion>>>,
    knowledge_fingerprints: Arc<Mutex<HashMap<(Uuid, KnowledgePageId), String>>>,
    guided_repairs: Arc<Mutex<HashMap<Uuid, GuidedRepairPreview>>>,
    requests: Arc<Mutex<RequestTracker>>,
    confirmation_gate: Arc<Semaphore>,
    tray_operational: AtomicBool,
    exiting: AtomicBool,
    worker_finished: Mutex<Option<oneshot::Receiver<()>>>,
}

#[derive(Clone)]
struct PublishedSnapshot {
    snapshot: AppSnapshot,
    request_id: Option<Uuid>,
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
    guided_repair: HashMap<Uuid, Uuid>,
    public_browse: Option<Uuid>,
    preferences: Option<Uuid>,
    autostart: Option<Uuid>,
    wiki_health: Option<Uuid>,
    connectivity: Option<Uuid>,
    integrations: Option<Uuid>,
    updater: Option<Uuid>,
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
    platform: HostPlatform,
    phase: AppPhase,
    node_id: Option<String>,
    mcp_url: Option<String>,
    blocked_public_publishers: Vec<String>,
    hardware: Option<HardwareSummary>,
    wikis: Vec<WikiSummary>,
    wiki_scans: Vec<WikiScanSummary>,
    reviews: Vec<ReviewSummary>,
    reanalyzing_review_ids: Vec<String>,
    source_issues: Vec<SourceIssueSummary>,
    peers: Vec<PeerSummary>,
    model: Option<ModelSummary>,
    model_install: Option<ModelInstallSummary>,
    search: Option<SearchSummary>,
    public_browse: Option<PublicBrowseSummary>,
    review_evidence: Option<ReviewEvidenceSummary>,
    knowledge: Option<KnowledgeBundleSummary>,
    knowledge_page: Option<KnowledgePageSummary>,
    preferences: Option<PreferencesSummary>,
    autostart: Option<AutostartStatusDto>,
    wiki_health: Option<WikiHealthSummary>,
    guided_repair: Option<GuidedRepairSummary>,
    connectivity: Option<ConnectivitySummary>,
    lan_runtime: Option<LanRuntimeSummary>,
    firewall_operation: Option<FirewallOperationStatus>,
    integrations: Option<IntegrationsSummary>,
    updater: Option<UpdaterSummary>,
    notice: Option<NoticeSummary>,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "both supported platform tags must remain in the shared UI contract"
)]
enum HostPlatform {
    MacOs,
    Windows,
}

impl HostPlatform {
    #[cfg(target_os = "macos")]
    const CURRENT: Self = Self::MacOs;

    #[cfg(target_os = "windows")]
    const CURRENT: Self = Self::Windows;
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
struct WikiSummary {
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
    public_description: String,
    public_languages: String,
    public_announcement: PublicAnnouncementSummary,
    maintenance_required: bool,
    origin: WikiOriginDto,
    indexing_mode: IndexingModeDto,
    okf_version: String,
    trust_summary: TrustSummaryDto,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum TrustSummaryDto {
    Unverified,
    MachineConfirmed,
    HumanReviewed,
    VerificationOutdated,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum WikiOriginDto {
    Folder,
    ImportedOkf,
    AiMemory,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum IndexingModeDto {
    Continuous,
    Manual,
    NotApplicable,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase", tag = "status")]
#[ts(rename_all = "camelCase", tag = "status")]
enum PublicAnnouncementSummary {
    Offline,
    Advertised {
        #[serde(rename = "acceptedIndexes")]
        #[ts(rename = "acceptedIndexes")]
        accepted_indexes: usize,
    },
    Expired,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct HardwareSummary {
    os: String,
    architecture: String,
    #[ts(type = "number")]
    total_memory_bytes: u64,
    #[ts(type = "number")]
    available_memory_bytes: u64,
    #[ts(type = "number")]
    available_disk_bytes: u64,
    avx2: bool,
    metal_available: bool,
    supported_target: bool,
    can_install: bool,
    issues: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
struct WikiScanSummary {
    wiki_id: String,
    state: WikiScanStatus,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum WikiScanStatus {
    Queued,
    Scanning,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct ReviewSummary {
    concept_id: String,
    wiki_id: String,
    source_revision: u32,
    source_name: String,
    wiki_name: String,
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(transparent)]
#[ts(rename = "ConceptType")]
struct ConceptTypeDto(#[ts(type = "string")] ConceptType);

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
        Self(value)
    }
}

impl From<ConceptTypeDto> for ConceptType {
    fn from(value: ConceptTypeDto) -> Self {
        value.0
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
    wiki_id: String,
    wiki_name: String,
    version: String,
    status: KnowledgeBundleStatus,
    concepts: Vec<KnowledgeConceptSummary>,
    links: Vec<KnowledgeGraphLinkSummary>,
    error_count: usize,
    warning_count: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
struct KnowledgeGraphLinkSummary {
    source: KnowledgePageInput,
    target: KnowledgePageInput,
    label: String,
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
    wiki_id: String,
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
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
    wiki_id: String,
    source_name: String,
    wiki_name: String,
    code: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct PeerSummary {
    peer_id: String,
    device_name: Option<String>,
    address: String,
    trust: PeerTrust,
    activity: PeerActivity,
    sas_words: Option<Vec<String>>,
    granted_wiki_ids: Vec<String>,
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
    #[ts(type = "number")]
    state_sequence: u64,
    profile: String,
    recommended_model_id: Option<String>,
    display_name: Option<String>,
    recommendation_reason: Option<String>,
    active: bool,
    installed: bool,
    degraded: bool,
    issues: Vec<String>,
    pending_model_id: Option<String>,
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
    Queued,
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

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct PublicBrowseSummary {
    request_id: String,
    status: PublicBrowseStatus,
    publisher_id: Option<String>,
    wiki_id: Option<String>,
    wiki_name: Option<String>,
    description: Option<String>,
    languages: Vec<String>,
    concepts: Vec<PublicConceptSummaryDto>,
    next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum PublicBrowseStatus {
    Direct,
    Relay,
    Expired,
    Offline,
    Failed,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct PublicConceptSummaryDto {
    concept_id: String,
    concept_type: ConceptTypeDto,
    title: String,
    description: String,
    language: String,
    tags: Vec<String>,
    summary: String,
    source_revision: u32,
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
    concept_id: String,
    wiki_id: String,
    title: String,
    snippet: String,
    heading_or_page: String,
    logical_resource_uri: String,
    source_revision: u32,
    source_sha256: String,
    rank: u32,
    node_id: String,
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
    theme: ThemePreferenceDto,
    lan_preference: LanPreferenceDto,
    close_behavior: CloseBehaviorDto,
    automatic_update_checks: bool,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
struct PreferencesInput {
    locale: LocalePreferenceDto,
    theme: ThemePreferenceDto,
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
#[ts(rename = "ThemePreference", rename_all = "snake_case")]
enum ThemePreferenceDto {
    System,
    Light,
    Dark,
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

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "AutostartStatus", rename_all = "camelCase")]
enum AutostartStatusDto {
    Disabled,
    Enabled,
    RequiresApproval,
    Conflict,
    Unsupported,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct WikiHealthSummary {
    #[ts(type = "number")]
    generation: u64,
    status: WikiHealthStatus,
    error_count: usize,
    warning_count: usize,
    updating_count: usize,
    attention_wiki_id: Option<String>,
    checked: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct GuidedRepairSummary {
    request_id: String,
    wiki_id: String,
    status: GuidedRepairStatus,
    impact_code: Option<String>,
    authorities: Vec<RepairAuthorityDto>,
    files: Vec<GuidedRepairFileSummary>,
    concepts_returned_to_review: usize,
    orphan_concepts_removed: usize,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum GuidedRepairStatus {
    Prepared,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
enum RepairAuthorityDto {
    HumanReview,
    PublishedDatabase,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct GuidedRepairFileSummary {
    page: KnowledgePageInput,
    change: GuidedRepairChangeDto,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
enum GuidedRepairChangeDto {
    WithdrawConcept,
    RemoveOrphan,
    RegenerateIndex,
    AppendDeprecationHistory,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum WikiHealthStatus {
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct ConnectivitySummary {
    system_permission: SystemPermissionStatus,
    network_profile: NetworkProfileStatus,
    firewall: FirewallStatus,
    firewall_helper: FirewallHelperStatus,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum SystemPermissionStatus {
    NotApplicable,
    Unknown,
    Granted,
    Denied,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum NetworkProfileStatus {
    NotApplicable,
    Unknown,
    Private,
    Domain,
    Public,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum FirewallStatus {
    NotApplicable,
    Unknown,
    Ready,
    FirewallDisabled,
    BlockAllInbound,
    RulesMissing,
    Conflict,
    LegacyExposure,
    ManagedPolicy,
    Unsupported,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum FirewallHelperStatus {
    NotApplicable,
    Verified,
    Missing,
    Untrusted,
    PublisherMismatch,
    Unsupported,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct LanRuntimeSummary {
    listener: LanListenerStatus,
    discovery: LanDiscoveryStatus,
    address_count: usize,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum LanListenerStatus {
    Stopped,
    Starting,
    Listening,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum LanDiscoveryStatus {
    Disabled,
    Starting,
    Active,
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
enum FirewallOperationStatus {
    AwaitingWindows,
    TakingLonger,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "SystemDestination", rename_all = "camelCase")]
enum SystemDestinationInput {
    NetworkSettings,
    AdvancedFirewall,
    LocalNetworkPrivacy,
}

impl From<SystemDestinationInput> for SystemDestination {
    fn from(value: SystemDestinationInput) -> Self {
        match value {
            SystemDestinationInput::NetworkSettings => Self::NetworkSettings,
            SystemDestinationInput::AdvancedFirewall => Self::AdvancedFirewall,
            SystemDestinationInput::LocalNetworkPrivacy => Self::LocalNetworkPrivacy,
        }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct IntegrationsSummary {
    integrations: Vec<IntegrationSummary>,
    external_ai_wiki_count: usize,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct UpdaterSummary {
    status: UpdaterStatusDto,
    version: Option<String>,
    release_notes: Option<String>,
    issue: Option<UpdaterIssueDto>,
    retryable: bool,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "UpdaterStatus", rename_all = "camelCase")]
enum UpdaterStatusDto {
    Disabled,
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    ReadyToInstall,
    Installing,
    Installed,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "UpdaterIssue", rename_all = "camelCase")]
enum UpdaterIssueDto {
    NotConfigured,
    InvalidConfiguration,
    Unsupported,
    Offline,
    InvalidManifest,
    InvalidSignature,
    Internal,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct IntegrationSummary {
    client: IntegrationClientDto,
    status: IntegrationStatusDto,
    detected_version: Option<String>,
    activity_recent: bool,
    restart_required: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "IntegrationClient", rename_all = "camelCase")]
enum IntegrationClientDto {
    ChatGptDesktop,
    ClaudeDesktop,
    GeminiCli,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "IntegrationStatus", rename_all = "camelCase")]
enum IntegrationStatusDto {
    NotInstalled,
    Available,
    AwaitingClientApproval,
    Configured,
    UpdateAvailable,
    Conflict,
    Unsupported,
    Error,
}

impl From<IntegrationClientDto> for integrations::ChatClientKind {
    fn from(value: IntegrationClientDto) -> Self {
        match value {
            IntegrationClientDto::ChatGptDesktop => Self::ChatGptDesktop,
            IntegrationClientDto::ClaudeDesktop => Self::ClaudeDesktop,
            IntegrationClientDto::GeminiCli => Self::GeminiCli,
        }
    }
}

impl From<integrations::ChatClientKind> for IntegrationClientDto {
    fn from(value: integrations::ChatClientKind) -> Self {
        match value {
            integrations::ChatClientKind::ChatGptDesktop => Self::ChatGptDesktop,
            integrations::ChatClientKind::ClaudeDesktop => Self::ClaudeDesktop,
            integrations::ChatClientKind::GeminiCli => Self::GeminiCli,
        }
    }
}

impl From<autostart::AutostartStatus> for AutostartStatusDto {
    fn from(value: autostart::AutostartStatus) -> Self {
        match value {
            autostart::AutostartStatus::Disabled => Self::Disabled,
            autostart::AutostartStatus::Enabled => Self::Enabled,
            autostart::AutostartStatus::RequiresApproval => Self::RequiresApproval,
            autostart::AutostartStatus::Conflict => Self::Conflict,
            autostart::AutostartStatus::Unsupported => Self::Unsupported,
        }
    }
}

impl From<connectivity_platform::ConnectivityPlatformSnapshot> for ConnectivitySummary {
    fn from(value: connectivity_platform::ConnectivityPlatformSnapshot) -> Self {
        Self {
            system_permission: match value.system_permission {
                connectivity_platform::SystemPermissionState::NotApplicable => {
                    SystemPermissionStatus::NotApplicable
                }
                connectivity_platform::SystemPermissionState::Unknown => {
                    SystemPermissionStatus::Unknown
                }
                connectivity_platform::SystemPermissionState::Granted => {
                    SystemPermissionStatus::Granted
                }
                connectivity_platform::SystemPermissionState::Denied => {
                    SystemPermissionStatus::Denied
                }
            },
            network_profile: match value.network_profile {
                connectivity_platform::NetworkProfileState::NotApplicable => {
                    NetworkProfileStatus::NotApplicable
                }
                connectivity_platform::NetworkProfileState::Unknown => {
                    NetworkProfileStatus::Unknown
                }
                connectivity_platform::NetworkProfileState::Private => {
                    NetworkProfileStatus::Private
                }
                connectivity_platform::NetworkProfileState::Domain => NetworkProfileStatus::Domain,
                connectivity_platform::NetworkProfileState::Public => NetworkProfileStatus::Public,
            },
            firewall: match value.firewall {
                connectivity_platform::FirewallDiagnosticState::NotApplicable => {
                    FirewallStatus::NotApplicable
                }
                connectivity_platform::FirewallDiagnosticState::Unknown => FirewallStatus::Unknown,
                connectivity_platform::FirewallDiagnosticState::Ready => FirewallStatus::Ready,
                connectivity_platform::FirewallDiagnosticState::FirewallDisabled => {
                    FirewallStatus::FirewallDisabled
                }
                connectivity_platform::FirewallDiagnosticState::BlockAllInbound => {
                    FirewallStatus::BlockAllInbound
                }
                connectivity_platform::FirewallDiagnosticState::RulesMissing => {
                    FirewallStatus::RulesMissing
                }
                connectivity_platform::FirewallDiagnosticState::Conflict => {
                    FirewallStatus::Conflict
                }
                connectivity_platform::FirewallDiagnosticState::LegacyExposure => {
                    FirewallStatus::LegacyExposure
                }
                connectivity_platform::FirewallDiagnosticState::ManagedPolicy => {
                    FirewallStatus::ManagedPolicy
                }
                connectivity_platform::FirewallDiagnosticState::Unsupported => {
                    FirewallStatus::Unsupported
                }
                connectivity_platform::FirewallDiagnosticState::Error => FirewallStatus::Error,
            },
            firewall_helper: match value.firewall_helper {
                connectivity_platform::FirewallHelperState::NotApplicable => {
                    FirewallHelperStatus::NotApplicable
                }
                connectivity_platform::FirewallHelperState::Verified => {
                    FirewallHelperStatus::Verified
                }
                connectivity_platform::FirewallHelperState::Missing => {
                    FirewallHelperStatus::Missing
                }
                connectivity_platform::FirewallHelperState::Untrusted => {
                    FirewallHelperStatus::Untrusted
                }
                connectivity_platform::FirewallHelperState::PublisherMismatch => {
                    FirewallHelperStatus::PublisherMismatch
                }
                connectivity_platform::FirewallHelperState::Unsupported => {
                    FirewallHelperStatus::Unsupported
                }
                connectivity_platform::FirewallHelperState::Error => FirewallHelperStatus::Error,
            },
        }
    }
}

impl From<worker::LanListenerView> for LanListenerStatus {
    fn from(value: worker::LanListenerView) -> Self {
        match value {
            worker::LanListenerView::Stopped => Self::Stopped,
            worker::LanListenerView::Starting => Self::Starting,
            worker::LanListenerView::Listening => Self::Listening,
            worker::LanListenerView::Failed => Self::Failed,
        }
    }
}

impl From<worker::LanDiscoveryView> for LanDiscoveryStatus {
    fn from(value: worker::LanDiscoveryView) -> Self {
        match value {
            worker::LanDiscoveryView::Disabled => Self::Disabled,
            worker::LanDiscoveryView::Starting => Self::Starting,
            worker::LanDiscoveryView::Active => Self::Active,
            worker::LanDiscoveryView::Failed => Self::Failed,
        }
    }
}

impl From<worker::FirewallOperationView> for FirewallOperationStatus {
    fn from(value: worker::FirewallOperationView) -> Self {
        match value {
            worker::FirewallOperationView::AwaitingWindows => Self::AwaitingWindows,
            worker::FirewallOperationView::TakingLonger => Self::TakingLonger,
        }
    }
}

impl From<integrations::ChatIntegrationsSnapshot> for IntegrationsSummary {
    fn from(value: integrations::ChatIntegrationsSnapshot) -> Self {
        Self {
            integrations: value
                .integrations
                .into_iter()
                .map(|integration| IntegrationSummary {
                    client: integration.client.into(),
                    status: match integration.status {
                        integrations::IntegrationStatus::NotInstalled => {
                            IntegrationStatusDto::NotInstalled
                        }
                        integrations::IntegrationStatus::Available => {
                            IntegrationStatusDto::Available
                        }
                        integrations::IntegrationStatus::AwaitingClientApproval => {
                            IntegrationStatusDto::AwaitingClientApproval
                        }
                        integrations::IntegrationStatus::Configured => {
                            IntegrationStatusDto::Configured
                        }
                        integrations::IntegrationStatus::UpdateAvailable => {
                            IntegrationStatusDto::UpdateAvailable
                        }
                        integrations::IntegrationStatus::Conflict => IntegrationStatusDto::Conflict,
                        integrations::IntegrationStatus::Unsupported => {
                            IntegrationStatusDto::Unsupported
                        }
                        integrations::IntegrationStatus::Error => IntegrationStatusDto::Error,
                    },
                    detected_version: integration.detected_version,
                    activity_recent: integration.activity_recent,
                    restart_required: integration.restart_required,
                })
                .collect(),
            external_ai_wiki_count: value.external_ai_collection_count,
        }
    }
}

impl From<worker::UpdaterWorkerView> for UpdaterSummary {
    fn from(value: worker::UpdaterWorkerView) -> Self {
        match value {
            worker::UpdaterWorkerView::Disabled(reason) => Self {
                status: UpdaterStatusDto::Disabled,
                version: None,
                release_notes: None,
                issue: Some(match reason {
                    UpdaterDisabledReason::NotConfigured => UpdaterIssueDto::NotConfigured,
                    UpdaterDisabledReason::UnsupportedPlatform => UpdaterIssueDto::Unsupported,
                    UpdaterDisabledReason::InvalidEndpoint
                    | UpdaterDisabledReason::InvalidPublicKey
                    | UpdaterDisabledReason::InvalidCurrentVersion => {
                        UpdaterIssueDto::InvalidConfiguration
                    }
                }),
                retryable: false,
            },
            worker::UpdaterWorkerView::Ready(view) => Self::from(view),
        }
    }
}

impl From<UpdaterView> for UpdaterSummary {
    fn from(view: UpdaterView) -> Self {
        let (status, update) = match view.status {
            UpdaterStatus::Idle => (UpdaterStatusDto::Idle, None),
            UpdaterStatus::Checking => (UpdaterStatusDto::Checking, None),
            UpdaterStatus::UpToDate => (UpdaterStatusDto::UpToDate, None),
            UpdaterStatus::Available(update) => (UpdaterStatusDto::Available, Some(update)),
            UpdaterStatus::Downloading(update) => (UpdaterStatusDto::Downloading, Some(update)),
            UpdaterStatus::ReadyToInstall(update) => {
                (UpdaterStatusDto::ReadyToInstall, Some(update))
            }
            UpdaterStatus::Installing(update) => (UpdaterStatusDto::Installing, Some(update)),
            UpdaterStatus::Installed(update) => (UpdaterStatusDto::Installed, Some(update)),
        };
        let (version, release_notes) = update.map_or((None, None), |update: UpdateSummary| {
            (Some(update.version), update.release_notes)
        });
        let issue = view.last_issue.map(|issue| match issue.code {
            UpdateIssueCode::Offline => UpdaterIssueDto::Offline,
            UpdateIssueCode::InvalidManifest => UpdaterIssueDto::InvalidManifest,
            UpdateIssueCode::InvalidSignature => UpdaterIssueDto::InvalidSignature,
            UpdateIssueCode::Unsupported => UpdaterIssueDto::Unsupported,
            UpdateIssueCode::Internal => UpdaterIssueDto::Internal,
        });
        Self {
            status,
            version,
            release_notes,
            issue,
            retryable: view.last_issue.is_some_and(|issue| issue.retryable),
        }
    }
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

impl From<ThemePreference> for ThemePreferenceDto {
    fn from(value: ThemePreference) -> Self {
        match value {
            ThemePreference::System => Self::System,
            ThemePreference::Light => Self::Light,
            ThemePreference::Dark => Self::Dark,
        }
    }
}

impl From<ThemePreferenceDto> for ThemePreference {
    fn from(value: ThemePreferenceDto) -> Self {
        match value {
            ThemePreferenceDto::System => Self::System,
            ThemePreferenceDto::Light => Self::Light,
            ThemePreferenceDto::Dark => Self::Dark,
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
    request_id: Option<String>,
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

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
struct OkfImportSummary {
    entry_count: usize,
    concept_count: usize,
    #[ts(type = "number")]
    uncompressed_bytes: u64,
    okf_version: String,
    warning_count: usize,
}

#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WikiPolicyInput {
    local_only: bool,
    peer_shareable: bool,
    allow_external_ai: bool,
    internet_public: bool,
}

fn policy_expands_authority(current: Option<&WikiSummary>, requested: &WikiPolicyInput) -> bool {
    current.map_or(
        requested.peer_shareable || requested.allow_external_ai || requested.internet_public,
        |current| {
            (requested.peer_shareable && !current.peer_shareable)
                || (requested.allow_external_ai && !current.allow_external_ai)
                || (requested.internet_public && !current.internet_public)
        },
    )
}

#[tauri::command]
fn connect(runtime: tauri::State<'_, AppRuntime>, events: Channel<UiEventEnvelope>) -> AppSnapshot {
    let Ok(snapshot_receiver) = runtime.snapshot.lock() else {
        return AppSnapshot::starting();
    };
    let mut receiver = snapshot_receiver.clone();
    let snapshot = receiver.borrow().snapshot.clone();
    drop(snapshot_receiver);
    tauri::async_runtime::spawn(async move {
        while receiver.changed().await.is_ok() {
            let published = receiver.borrow_and_update().clone();
            if events
                .send(UiEventEnvelope {
                    schema_version: CONTRACT_VERSION,
                    sequence: published.snapshot.sequence,
                    request_id: published
                        .request_id
                        .map(|request_id| request_id.to_string()),
                    kind: UiEventKind::StateChanged,
                    snapshot: published.snapshot,
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
async fn install_models(
    app: AppHandle,
    runtime: tauri::State<'_, AppRuntime>,
) -> Result<(), UiError> {
    require_native_confirmation(&app, NativeConfirmation::ModelLicenses, None).await?;
    send_command(&runtime, WorkerCommand::InstallModels).await
}

#[tauri::command]
async fn cancel_model_install(runtime: tauri::State<'_, AppRuntime>) -> Result<(), UiError> {
    send_command(&runtime, WorkerCommand::CancelInstall).await
}

#[tauri::command]
async fn set_model_profile(
    runtime: tauri::State<'_, AppRuntime>,
    profile: ModelProfile,
) -> Result<(), UiError> {
    send_command(&runtime, WorkerCommand::SetModelProfile(profile)).await
}

#[tauri::command]
async fn pick_wiki_folder(
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
async fn pick_okf_import(
    runtime: tauri::State<'_, AppRuntime>,
    zip: bool,
) -> Result<Option<FolderSelection>, UiError> {
    let selection = if zip {
        rfd::AsyncFileDialog::new()
            .add_filter("OKF ZIP", &["zip"])
            .pick_file()
            .await
    } else {
        rfd::AsyncFileDialog::new().pick_folder().await
    };
    let Some(selection) = selection else {
        return Ok(None);
    };
    let path = selection.path().to_path_buf();
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
async fn validate_okf_import(
    runtime: tauri::State<'_, AppRuntime>,
    selection_token: String,
) -> Result<OkfImportSummary, UiError> {
    let token = Uuid::parse_str(&selection_token)
        .map_err(|_| UiError::invalid("invalidFolderSelection"))?;
    let path = {
        let selections = runtime
            .folder_selections
            .lock()
            .map_err(|_| UiError::internal())?;
        let selection = selections
            .get(&token)
            .ok_or_else(|| UiError::invalid("folderSelectionExpired"))?;
        if selection.expires_at <= Instant::now() {
            return Err(UiError::invalid("folderSelectionExpired"));
        }
        selection.path.clone()
    };
    let report = tauri::async_runtime::spawn_blocking(move || {
        airwiki_core::OkfImportValidator::validate_path(&path)
    })
    .await
    .map_err(|_| UiError::internal())?
    .map_err(|_| UiError::invalid("invalidOkfImport"))?;
    Ok(OkfImportSummary {
        entry_count: report.entry_count,
        concept_count: report.concept_count,
        uncompressed_bytes: report.uncompressed_bytes,
        okf_version: report.okf_version,
        warning_count: report.warnings.len(),
    })
}

#[tauri::command]
async fn import_okf(
    runtime: tauri::State<'_, AppRuntime>,
    name: String,
    selection_token: String,
) -> Result<(), UiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(UiError::invalid("invalidWikiName"));
    }
    let source = consume_folder_selection(&runtime, &selection_token)?;
    send_command(
        &runtime,
        WorkerCommand::ImportOkfBundle {
            name: name.to_owned(),
            source,
        },
    )
    .await
}

#[tauri::command]
async fn set_wiki_indexing(
    runtime: tauri::State<'_, AppRuntime>,
    wiki_id: String,
    continuous: bool,
) -> Result<(), UiError> {
    let collection_id = parse_uuid(&wiki_id)?;
    send_command(
        &runtime,
        WorkerCommand::SetCollectionIndexing {
            collection_id,
            indexing_mode: if continuous {
                airwiki_core::IndexingMode::Continuous
            } else {
                airwiki_core::IndexingMode::Manual
            },
        },
    )
    .await
}

#[tauri::command]
async fn add_wiki(
    runtime: tauri::State<'_, AppRuntime>,
    name: String,
    folder_token: String,
    continuous_indexing: bool,
) -> Result<(), UiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(UiError::invalid("invalidWikiName"));
    }
    let folder = consume_folder_selection(&runtime, &folder_token)?;
    send_command(
        &runtime,
        WorkerCommand::AddCollection {
            name: name.to_owned(),
            folder,
            indexing_mode: if continuous_indexing {
                airwiki_core::IndexingMode::Continuous
            } else {
                airwiki_core::IndexingMode::Manual
            },
        },
    )
    .await
}

#[tauri::command]
async fn relink_wiki(
    runtime: tauri::State<'_, AppRuntime>,
    wiki_id: String,
    folder_token: String,
) -> Result<(), UiError> {
    let collection_id = parse_uuid(&wiki_id)?;
    let folder = consume_folder_selection(&runtime, &folder_token)?;
    send_command(
        &runtime,
        WorkerCommand::RelinkCollection {
            collection_id,
            folder,
        },
    )
    .await
}

#[tauri::command]
async fn rescan_wiki(
    runtime: tauri::State<'_, AppRuntime>,
    wiki_id: String,
) -> Result<(), UiError> {
    send_command(
        &runtime,
        WorkerCommand::RescanCollection(parse_uuid(&wiki_id)?),
    )
    .await
}

#[tauri::command]
async fn update_wiki_policy(
    app: AppHandle,
    runtime: tauri::State<'_, AppRuntime>,
    wiki_id: String,
    policy: WikiPolicyInput,
) -> Result<(), UiError> {
    let collection_id = parse_uuid(&wiki_id)?;
    let collection_id_text = collection_id.to_string();
    let expands_authority = {
        let snapshot = runtime.snapshot.lock().map_err(|_| UiError::internal())?;
        let published = snapshot.borrow();
        policy_expands_authority(
            published
                .snapshot
                .wikis
                .iter()
                .find(|collection| collection.id == collection_id_text),
            &policy,
        )
    };
    if expands_authority {
        require_native_confirmation(&app, NativeConfirmation::ExternalCollectionPolicy, None)
            .await?;
    }
    send_command(
        &runtime,
        WorkerCommand::UpdateCollectionPolicy {
            collection_id,
            local_only: policy.local_only,
            peer_shareable: policy.peer_shareable,
            allow_external_ai: policy.allow_external_ai,
            internet_public: policy.internet_public,
        },
    )
    .await
}

#[tauri::command]
async fn delete_wiki(
    app: AppHandle,
    runtime: tauri::State<'_, AppRuntime>,
    wiki_id: String,
) -> Result<(), UiError> {
    let collection_id = parse_uuid(&wiki_id)?;
    require_native_confirmation(&app, NativeConfirmation::DeleteWiki, None).await?;
    send_command(&runtime, WorkerCommand::DeleteWiki { collection_id }).await
}

#[tauri::command]
async fn add_federation_index(
    runtime: tauri::State<'_, AppRuntime>,
    peer_id: String,
    address: String,
) -> Result<(), UiError> {
    let peer_id = validate_peer_id(peer_id)?;
    let address = validate_network_address(address)?;
    send_command(
        &runtime,
        WorkerCommand::AddFederationIndex { peer_id, address },
    )
    .await
}

#[tauri::command]
async fn remove_federation_index(
    runtime: tauri::State<'_, AppRuntime>,
    peer_id: String,
) -> Result<(), UiError> {
    send_command(
        &runtime,
        WorkerCommand::RemoveFederationIndex {
            peer_id: validate_peer_id(peer_id)?,
        },
    )
    .await
}

#[tauri::command]
async fn update_public_wiki_profile(
    runtime: tauri::State<'_, AppRuntime>,
    wiki_id: String,
    description: String,
    languages: Vec<String>,
) -> Result<(), UiError> {
    if description.len() > 2_048
        || description.chars().any(char::is_control)
        || languages.len() > 16
        || languages.iter().any(|language| {
            language.is_empty() || language.len() > 35 || language.chars().any(char::is_control)
        })
    {
        return Err(UiError::invalid("invalidPublicCollectionProfile"));
    }
    send_command(
        &runtime,
        WorkerCommand::UpdatePublicCollectionProfile {
            collection_id: parse_uuid(&wiki_id)?,
            description,
            languages,
        },
    )
    .await
}

#[tauri::command]
async fn browse_public_wiki(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    publisher_id: String,
    wiki_id: String,
    cursor: Option<String>,
) -> Result<(), UiError> {
    if cursor
        .as_ref()
        .is_some_and(|value| value.len() > 128 || value.chars().any(char::is_control))
    {
        return Err(UiError::invalid("invalidPublicBrowseCursor"));
    }
    let request_id = parse_uuid(&request_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .public_browse = Some(request_id);
    if let Err(error) = send_command(
        &runtime,
        WorkerCommand::BrowsePublicCollection {
            request_id,
            publisher_id: validate_peer_id(publisher_id)?,
            collection_id: parse_uuid(&wiki_id)?,
            cursor,
        },
    )
    .await
    {
        if let Ok(mut requests) = runtime.requests.lock()
            && requests.public_browse == Some(request_id)
        {
            requests.public_browse = None;
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn set_public_publisher_blocked(
    runtime: tauri::State<'_, AppRuntime>,
    publisher_id: String,
    blocked: bool,
) -> Result<(), UiError> {
    send_command(
        &runtime,
        WorkerCommand::SetPublicPublisherBlocked {
            publisher_id: validate_peer_id(publisher_id)?,
            blocked,
        },
    )
    .await
}

#[tauri::command]
async fn dial_peer(runtime: tauri::State<'_, AppRuntime>, address: String) -> Result<(), UiError> {
    send_command(
        &runtime,
        WorkerCommand::Dial {
            address: validate_network_address(address)?,
        },
    )
    .await
}

#[tauri::command]
async fn pair_peer(runtime: tauri::State<'_, AppRuntime>, peer_id: String) -> Result<(), UiError> {
    let peer_id = validate_peer_id(peer_id)?;
    send_command(&runtime, WorkerCommand::Pair { peer_id }).await
}

#[tauri::command]
async fn confirm_pairing(
    runtime: tauri::State<'_, AppRuntime>,
    peer_id: String,
    accepted: bool,
) -> Result<(), UiError> {
    let peer_id = validate_peer_id(peer_id)?;
    send_command(
        &runtime,
        WorkerCommand::ConfirmPairing { peer_id, accepted },
    )
    .await
}

#[tauri::command]
async fn revoke_peer(
    runtime: tauri::State<'_, AppRuntime>,
    peer_id: String,
) -> Result<(), UiError> {
    let peer_id = validate_peer_id(peer_id)?;
    send_command(&runtime, WorkerCommand::RevokePeer { peer_id }).await
}

#[tauri::command]
async fn allow_peer_pairing_again(
    runtime: tauri::State<'_, AppRuntime>,
    peer_id: String,
) -> Result<(), UiError> {
    let peer_id = validate_peer_id(peer_id)?;
    send_command(&runtime, WorkerCommand::AllowPeerPairingAgain { peer_id }).await
}

#[tauri::command]
async fn set_wiki_grant(
    app: AppHandle,
    runtime: tauri::State<'_, AppRuntime>,
    peer_id: String,
    wiki_id: String,
    granted: bool,
) -> Result<(), UiError> {
    if granted {
        require_native_confirmation(&app, NativeConfirmation::CollectionGrant, None).await?;
    }
    let peer_id = validate_peer_id(peer_id)?;
    let collection_id = parse_uuid(&wiki_id)?;
    send_command(
        &runtime,
        WorkerCommand::GrantCollection {
            peer_id,
            collection_id,
            granted,
        },
    )
    .await
}

#[derive(Debug, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum IntegrationActionInput {
    Refresh,
    Connect { client: IntegrationClientDto },
    Disconnect { client: IntegrationClientDto },
    ConfirmClaudeInstalled,
    OpenClaudeSettings,
}

#[tauri::command]
async fn manage_integration(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    action: IntegrationActionInput,
) -> Result<(), UiError> {
    let request_id = parse_uuid(&request_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .integrations = Some(request_id);
    let action = match action {
        IntegrationActionInput::Refresh => integrations::IntegrationAction::Refresh,
        IntegrationActionInput::Connect { client } => {
            integrations::IntegrationAction::Connect(client.into())
        }
        IntegrationActionInput::Disconnect { client } => {
            integrations::IntegrationAction::Disconnect(client.into())
        }
        IntegrationActionInput::ConfirmClaudeInstalled => {
            integrations::IntegrationAction::ConfirmClaudeInstalled
        }
        IntegrationActionInput::OpenClaudeSettings => {
            integrations::IntegrationAction::OpenClaudeSettings
        }
    };
    if let Err(error) = send_command(
        &runtime,
        WorkerCommand::ManageChatIntegration { request_id, action },
    )
    .await
    {
        if let Ok(mut requests) = runtime.requests.lock()
            && requests.integrations == Some(request_id)
        {
            requests.integrations = None;
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn search(
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
    .await
}

#[tauri::command]
async fn load_review_evidence(
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
    .await
}

#[tauri::command]
async fn approve_review(
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
    .await
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
async fn reject_review(
    runtime: tauri::State<'_, AppRuntime>,
    concept_id: String,
) -> Result<(), UiError> {
    send_command(
        &runtime,
        WorkerCommand::Reject {
            concept_id: parse_uuid(&concept_id)?,
        },
    )
    .await
}

#[tauri::command]
async fn reanalyze_review(
    runtime: tauri::State<'_, AppRuntime>,
    concept_id: String,
) -> Result<(), UiError> {
    send_command(
        &runtime,
        WorkerCommand::ReanalyzeReview {
            concept_id: parse_uuid(&concept_id)?,
        },
    )
    .await
}

#[tauri::command]
async fn load_wiki_bundle(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    wiki_id: String,
) -> Result<(), UiError> {
    let request_id = parse_uuid(&request_id)?;
    let collection_id = parse_uuid(&wiki_id)?;
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
    .await
}

#[tauri::command]
async fn load_wiki_page(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    wiki_id: String,
    page: KnowledgePageInput,
) -> Result<(), UiError> {
    let collection_id = parse_uuid(&wiki_id)?;
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
    .await
}

#[tauri::command]
async fn update_preferences(
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
                theme: preferences.theme.into(),
                lan_preference: preferences.lan_preference.into(),
                close_behavior: preferences.close_behavior.into(),
                automatic_update_checks: preferences.automatic_update_checks,
                complete_onboarding: preferences.complete_onboarding,
            },
        },
    )
    .await
}

#[tauri::command]
async fn refresh_autostart(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
) -> Result<(), UiError> {
    send_autostart_command(&runtime, request_id, |request_id| {
        WorkerCommand::RefreshAutostart { request_id }
    })
    .await
}

#[tauri::command]
async fn set_autostart(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    enabled: bool,
) -> Result<(), UiError> {
    send_autostart_command(&runtime, request_id, |request_id| {
        WorkerCommand::SetAutostart {
            request_id,
            enabled,
        }
    })
    .await
}

#[tauri::command]
async fn check_updates(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
) -> Result<(), UiError> {
    send_updater_command(&runtime, request_id, |request_id| {
        WorkerCommand::CheckUpdates { request_id }
    })
    .await
}

#[tauri::command]
async fn download_update(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
) -> Result<(), UiError> {
    send_updater_command(&runtime, request_id, |request_id| {
        WorkerCommand::DownloadUpdate { request_id }
    })
    .await
}

#[tauri::command]
async fn install_update(
    app: AppHandle,
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
) -> Result<(), UiError> {
    require_native_confirmation(&app, NativeConfirmation::InstallUpdate, None).await?;
    let result = send_updater_command(&runtime, request_id, |request_id| {
        WorkerCommand::InstallUpdate { request_id }
    })
    .await;
    #[cfg(target_os = "windows")]
    if result.is_ok() {
        begin_shutdown(app);
    }
    result
}

async fn send_updater_command(
    runtime: &AppRuntime,
    request_id: String,
    command: impl FnOnce(Uuid) -> WorkerCommand,
) -> Result<(), UiError> {
    let request_id = parse_uuid(&request_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .updater = Some(request_id);
    if let Err(error) = send_command(runtime, command(request_id)).await {
        if let Ok(mut requests) = runtime.requests.lock()
            && requests.updater == Some(request_id)
        {
            requests.updater = None;
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn refresh_wiki_health(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
) -> Result<(), UiError> {
    let request_id = parse_uuid(&request_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .wiki_health = Some(request_id);
    if let Err(error) =
        send_command(&runtime, WorkerCommand::RefreshWikiHealth { request_id }).await
    {
        if let Ok(mut requests) = runtime.requests.lock()
            && requests.wiki_health == Some(request_id)
        {
            requests.wiki_health = None;
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn prepare_guided_wiki_repair(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    wiki_id: String,
) -> Result<(), UiError> {
    let request_id = parse_uuid(&request_id)?;
    let collection_id = parse_uuid(&wiki_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .guided_repair
        .insert(collection_id, request_id);
    if let Err(error) = send_command(
        &runtime,
        WorkerCommand::PrepareGuidedWikiRepair {
            request_id,
            collection_id,
        },
    )
    .await
    {
        if let Ok(mut requests) = runtime.requests.lock() {
            remove_matching_request(&mut requests.guided_repair, &collection_id, &request_id);
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn execute_guided_wiki_repair(
    app: AppHandle,
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    wiki_id: String,
) -> Result<(), UiError> {
    require_native_confirmation(&app, NativeConfirmation::GuidedRepair, None).await?;
    let collection_id = parse_uuid(&wiki_id)?;
    let preview = runtime
        .guided_repairs
        .lock()
        .map_err(|_| UiError::internal())?
        .get(&collection_id)
        .cloned()
        .ok_or_else(|| UiError::invalid("guidedRepairPreviewRequired"))?;
    let request_id = parse_uuid(&request_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .guided_repair
        .insert(collection_id, request_id);
    if let Err(error) = send_command(
        &runtime,
        WorkerCommand::ExecuteGuidedWikiRepair {
            request_id,
            preview,
        },
    )
    .await
    {
        if let Ok(mut requests) = runtime.requests.lock() {
            remove_matching_request(&mut requests.guided_repair, &collection_id, &request_id);
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn refresh_connectivity(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
) -> Result<(), UiError> {
    send_connectivity_command(&runtime, request_id, |request_id| {
        WorkerCommand::RefreshConnectivity { request_id }
    })
    .await
}

#[tauri::command]
async fn configure_firewall(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    install: bool,
) -> Result<(), UiError> {
    send_connectivity_command(&runtime, request_id, |request_id| {
        WorkerCommand::ConfigureFirewall {
            request_id,
            install,
        }
    })
    .await
}

#[tauri::command]
async fn open_system_destination(
    runtime: tauri::State<'_, AppRuntime>,
    request_id: String,
    destination: SystemDestinationInput,
) -> Result<(), UiError> {
    send_connectivity_command(&runtime, request_id, |request_id| {
        WorkerCommand::OpenSystemDestination {
            request_id,
            destination: destination.into(),
        }
    })
    .await
}

async fn send_connectivity_command(
    runtime: &AppRuntime,
    request_id: String,
    command: impl FnOnce(Uuid) -> WorkerCommand,
) -> Result<(), UiError> {
    let request_id = parse_uuid(&request_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .connectivity = Some(request_id);
    if let Err(error) = send_command(runtime, command(request_id)).await {
        if let Ok(mut requests) = runtime.requests.lock()
            && requests.connectivity == Some(request_id)
        {
            requests.connectivity = None;
        }
        return Err(error);
    }
    Ok(())
}

async fn send_autostart_command(
    runtime: &AppRuntime,
    request_id: String,
    command: impl FnOnce(Uuid) -> WorkerCommand,
) -> Result<(), UiError> {
    let request_id = parse_uuid(&request_id)?;
    runtime
        .requests
        .lock()
        .map_err(|_| UiError::internal())?
        .autostart = Some(request_id);
    if let Err(error) = send_command(runtime, command(request_id)).await {
        if let Ok(mut requests) = runtime.requests.lock()
            && requests.autostart == Some(request_id)
        {
            requests.autostart = None;
        }
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn open_external_link(app: AppHandle, url: String) -> Result<(), UiError> {
    let url = external_navigation::validate_external_url(&url)
        .map_err(|_| UiError::invalid("invalidExternalLink"))?;
    require_native_confirmation(&app, NativeConfirmation::ExternalLink, Some(url.as_str())).await?;
    tokio::task::spawn_blocking(move || external_navigation::open_external_url(&url))
        .await
        .map_err(|_| UiError::internal())?
        .map_err(|error| match error {
            external_navigation::ExternalNavigationError::InvalidUrl => {
                UiError::invalid("invalidExternalLink")
            }
            external_navigation::ExternalNavigationError::OpenFailed => UiError::internal(),
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            external_navigation::ExternalNavigationError::Unsupported => UiError::internal(),
        })
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
    runtime.cancellation.cancel();
    let finished = runtime
        .worker_finished
        .lock()
        .ok()
        .and_then(|mut receiver| receiver.take());
    tauri::async_runtime::spawn(async move {
        let shutdown = async {
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

const fn tray_icon() -> Image<'static> {
    Image::new(TRAY_ICON_RGBA, TRAY_ICON_WIDTH, TRAY_ICON_HEIGHT)
}

const fn tray_click_opens_window(button: MouseButton) -> bool {
    matches!(button, MouseButton::Left)
}

fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let labels = Localization::new(UiLocale::from_system()).ok();
    let open_label = match labels
        .as_ref()
        .and_then(|localization| localization.text("tray-open"))
    {
        Some(label) => label,
        None => "Open AirWiki".to_owned(),
    };
    let quit_label = match labels
        .as_ref()
        .and_then(|localization| localization.text("tray-quit"))
    {
        Some(label) => label,
        None => "Quit completely".to_owned(),
    };
    let open = MenuItem::with_id(app, "open", open_label, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::new()
        .icon(tray_icon())
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("AirWiki")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => begin_shutdown(app.clone()),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click { button, .. } if tray_click_opens_window(button)
            ) {
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

fn validate_peer_id(value: String) -> Result<String, UiError> {
    if value.is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(UiError::invalid("invalidPeerIdentifier"));
    }
    Ok(value)
}

fn validate_network_address(value: String) -> Result<String, UiError> {
    if value.is_empty()
        || value.len() > 512
        || value.trim() != value
        || !value.starts_with('/')
        || value.chars().any(char::is_control)
    {
        return Err(UiError::invalid("invalidNetworkAddress"));
    }
    Ok(value)
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

    const fn busy(message_key: &'static str) -> Self {
        Self {
            code: "busy",
            message_key,
            retryable: true,
        }
    }
}

async fn send_command(runtime: &AppRuntime, command: WorkerCommand) -> Result<(), UiError> {
    let (accepted, response) = oneshot::channel();
    runtime
        .commands
        .try_send(WorkerIntent { command, accepted })
        .map_err(|error| UiError {
            code: match error {
                mpsc::error::TrySendError::Full(_) => "busy",
                mpsc::error::TrySendError::Closed(_) => "unavailable",
            },
            message_key: "runtime-command-unavailable",
            retryable: true,
        })?;
    response.await.map_err(|_| UiError {
        code: "unavailable",
        message_key: "runtime-command-unavailable",
        retryable: true,
    })
}

impl AppSnapshot {
    fn starting() -> Self {
        Self {
            schema_version: CONTRACT_VERSION,
            sequence: 0,
            platform: HostPlatform::CURRENT,
            phase: AppPhase::Starting,
            node_id: None,
            mcp_url: None,
            blocked_public_publishers: Vec::new(),
            hardware: None,
            wikis: Vec::new(),
            wiki_scans: Vec::new(),
            reviews: Vec::new(),
            reanalyzing_review_ids: Vec::new(),
            source_issues: Vec::new(),
            peers: Vec::new(),
            model: None,
            model_install: None,
            search: None,
            public_browse: None,
            review_evidence: None,
            knowledge: None,
            knowledge_page: None,
            preferences: None,
            autostart: None,
            wiki_health: None,
            guided_repair: None,
            connectivity: None,
            lan_runtime: None,
            firewall_operation: None,
            integrations: None,
            updater: None,
            notice: None,
        }
    }

    async fn apply(
        &mut self,
        event: WorkerEvent,
        review_versions: &Mutex<HashMap<Uuid, CachedReviewVersion>>,
        knowledge_fingerprints: &Mutex<HashMap<(Uuid, KnowledgePageId), String>>,
        guided_repairs: &Mutex<HashMap<Uuid, GuidedRepairPreview>>,
        requests: &Mutex<RequestTracker>,
    ) {
        if !request_is_current(&event, requests) {
            return;
        }
        self.sequence = self.sequence.saturating_add(1);
        match event {
            WorkerEvent::Ready {
                node_id,
                mcp_url,
                collections,
                reviews,
                source_issues,
                blocked_public_publishers,
            } => {
                self.phase = AppPhase::Ready;
                self.node_id = Some(node_id);
                self.mcp_url = Some(mcp_url);
                self.blocked_public_publishers = blocked_public_publishers;
                self.wikis = collections.into_iter().map(WikiSummary::from).collect();
                self.reviews = reviews.into_iter().map(ReviewSummary::from).collect();
                retain_pending_review_versions(review_versions, &self.reviews);
                self.source_issues = source_issues
                    .into_iter()
                    .map(SourceIssueSummary::from)
                    .collect();
            }
            WorkerEvent::Hardware(report) => {
                self.hardware = Some(HardwareSummary {
                    os: report.os,
                    architecture: report.architecture,
                    total_memory_bytes: report.total_memory_bytes,
                    available_memory_bytes: report.available_memory_bytes,
                    available_disk_bytes: report.available_disk_bytes,
                    avx2: report.avx2,
                    metal_available: report.metal_available,
                    supported_target: report.supported_target,
                    can_install: report.can_install,
                    issues: report.issues,
                });
            }
            WorkerEvent::Collections(collections) => {
                self.wikis = collections.into_iter().map(WikiSummary::from).collect();
            }
            WorkerEvent::CollectionScan {
                collection_id,
                state,
            } => update_wiki_scan(&mut self.wiki_scans, collection_id, state),
            WorkerEvent::Reviews(reviews) => {
                self.reviews = reviews.into_iter().map(ReviewSummary::from).collect();
                retain_pending_review_versions(review_versions, &self.reviews);
            }
            WorkerEvent::ReviewReanalysis {
                concept_id,
                running,
            } => update_running_review(&mut self.reanalyzing_review_ids, concept_id, running),
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
                        theme: preferences.theme.into(),
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
            WorkerEvent::AutostartUpdated { result, .. } => match result {
                Ok(status) => self.autostart = Some(status.into()),
                Err(_) => {
                    self.notice = Some(NoticeSummary {
                        level: NoticeLevel::Error,
                        message: "autostart-update-failed".to_owned(),
                    });
                }
            },
            WorkerEvent::WikiHealthUpdated {
                generation, result, ..
            } => {
                if !wiki_health_generation_applies(
                    self.wiki_health.as_ref().map(|current| current.generation),
                    generation,
                ) {
                    return;
                }
                self.wiki_health = Some(match result {
                    Ok(summary) => WikiHealthSummary {
                        generation,
                        status: WikiHealthStatus::Ready,
                        error_count: summary.error_count,
                        warning_count: summary.warning_count,
                        updating_count: summary.updating_count,
                        attention_wiki_id: summary
                            .attention_collection_id
                            .map(|collection_id| collection_id.to_string()),
                        checked: summary.checked_at.is_some(),
                    },
                    Err(_) => WikiHealthSummary {
                        generation,
                        status: WikiHealthStatus::Failed,
                        error_count: 0,
                        warning_count: 0,
                        updating_count: 0,
                        attention_wiki_id: None,
                        checked: false,
                    },
                });
            }
            WorkerEvent::ConnectivityPlatformUpdated { result, .. } => match result {
                Ok(connectivity) => self.connectivity = Some(connectivity.into()),
                Err(_) => {
                    self.notice = Some(NoticeSummary {
                        level: NoticeLevel::Error,
                        message: "connectivity-check-failed".to_owned(),
                    });
                }
            },
            WorkerEvent::WikiMaintenanceFinished {
                collection_id,
                repaired,
            } => {
                let collection_id = collection_id.to_string();
                self.notice = Some(NoticeSummary {
                    level: if repaired {
                        NoticeLevel::Notice
                    } else {
                        NoticeLevel::Warning
                    },
                    message: if repaired {
                        "wiki-maintenance-completed"
                    } else {
                        "wiki-maintenance-no-change"
                    }
                    .to_owned(),
                });
                if self
                    .wiki_health
                    .as_ref()
                    .is_some_and(|health| health.attention_wiki_id.as_ref() == Some(&collection_id))
                {
                    self.wiki_health = None;
                }
            }
            WorkerEvent::GuidedWikiRepairPrepared {
                request_id,
                collection_id,
                result,
            } => match result {
                Ok(preview) => {
                    if let Ok(mut repairs) = guided_repairs.lock() {
                        repairs.insert(collection_id, preview.clone());
                    }
                    self.guided_repair = Some(guided_repair_preview_summary(request_id, preview));
                }
                Err(_) => {
                    self.guided_repair = Some(failed_guided_repair(request_id, collection_id));
                }
            },
            WorkerEvent::GuidedWikiRepairFinished {
                request_id,
                collection_id,
                result,
            } => {
                if let Ok(mut repairs) = guided_repairs.lock() {
                    repairs.remove(&collection_id);
                }
                self.guided_repair = Some(match result {
                    Ok(result) => GuidedRepairSummary {
                        request_id: request_id.to_string(),
                        wiki_id: collection_id.to_string(),
                        status: GuidedRepairStatus::Completed,
                        impact_code: None,
                        authorities: Vec::new(),
                        files: Vec::new(),
                        concepts_returned_to_review: result.concepts_returned_to_review.len(),
                        orphan_concepts_removed: result.orphan_concepts_removed.len(),
                    },
                    Err(_) => failed_guided_repair(request_id, collection_id),
                });
            }
            WorkerEvent::FirewallOperationUpdated { state, .. } => {
                self.firewall_operation = state.map(Into::into);
            }
            WorkerEvent::LanRuntimeUpdated {
                listener,
                discovery,
                local_addresses,
                ..
            } => {
                self.lan_runtime = Some(LanRuntimeSummary {
                    listener: listener.into(),
                    discovery: discovery.into(),
                    address_count: local_addresses.len(),
                });
            }
            WorkerEvent::ChatIntegrationsUpdated { result, .. } => match result {
                Ok(integrations) => self.integrations = Some(integrations.into()),
                Err(_) => {
                    self.notice = Some(NoticeSummary {
                        level: NoticeLevel::Error,
                        message: "integration-operation-failed".to_owned(),
                    });
                }
            },
            WorkerEvent::UpdaterUpdated { result, .. } => match result {
                Ok(updater) => self.updater = Some(updater.into()),
                Err(_) => {
                    self.notice = Some(NoticeSummary {
                        level: NoticeLevel::Error,
                        message: "updater-operation-failed".to_owned(),
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
                    let graph_links = knowledge_graph_links(&bundle.links);
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
                        wiki_id: collection_id.to_string(),
                        wiki_name: bundle.collection_name,
                        version: bundle.fingerprint,
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
                        links: graph_links,
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
                        wiki_id: collection_id.to_string(),
                        wiki_name: String::new(),
                        version: String::new(),
                        status: KnowledgeBundleStatus::Failed,
                        concepts: Vec::new(),
                        links: Vec::new(),
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
            WorkerEvent::PublicBrowseFinished {
                request_id,
                append,
                result,
            } => {
                let mut summary = match result {
                    Ok(result) => {
                        let status = match result.availability {
                            airwiki_network::PublicCollectionAvailability::Available(
                                airwiki_network::PublicRouteKind::Direct,
                            ) => PublicBrowseStatus::Direct,
                            airwiki_network::PublicCollectionAvailability::Available(
                                airwiki_network::PublicRouteKind::Relay,
                            ) => PublicBrowseStatus::Relay,
                            airwiki_network::PublicCollectionAvailability::Available(
                                airwiki_network::PublicRouteKind::Offline,
                            )
                            | airwiki_network::PublicCollectionAvailability::Offline => {
                                PublicBrowseStatus::Offline
                            }
                            airwiki_network::PublicCollectionAvailability::Expired => {
                                PublicBrowseStatus::Expired
                            }
                        };
                        let (concepts, next_cursor) = result.page.map_or_else(
                            || (Vec::new(), None),
                            |page| {
                                (
                                    page.concepts
                                        .into_iter()
                                        .map(|concept| PublicConceptSummaryDto {
                                            concept_id: concept.concept_id.to_string(),
                                            concept_type: concept.concept_type.into(),
                                            title: concept.title,
                                            description: concept.description,
                                            language: concept.language,
                                            tags: concept.tags,
                                            summary: concept.summary,
                                            source_revision: concept.source_revision,
                                        })
                                        .collect(),
                                    page.next_cursor,
                                )
                            },
                        );
                        PublicBrowseSummary {
                            request_id: request_id.to_string(),
                            status,
                            publisher_id: Some(result.summary.publisher_id),
                            wiki_id: Some(result.summary.collection_id.to_string()),
                            wiki_name: Some(result.summary.name),
                            description: Some(result.summary.description),
                            languages: result.summary.languages,
                            concepts,
                            next_cursor,
                        }
                    }
                    Err(_) => PublicBrowseSummary {
                        request_id: request_id.to_string(),
                        status: PublicBrowseStatus::Failed,
                        publisher_id: None,
                        wiki_id: None,
                        wiki_name: None,
                        description: None,
                        languages: Vec::new(),
                        concepts: Vec::new(),
                        next_cursor: None,
                    },
                };
                if append
                    && let Some(previous) = self.public_browse.take()
                    && previous.publisher_id == summary.publisher_id
                    && previous.wiki_id == summary.wiki_id
                {
                    let mut concepts = previous.concepts;
                    concepts.extend(summary.concepts);
                    summary.concepts = concepts;
                }
                self.public_browse = Some(summary);
            }
            WorkerEvent::Notice(message) => {
                self.notice = Some(NoticeSummary {
                    level: NoticeLevel::Notice,
                    message,
                });
            }
            WorkerEvent::InstallQueued(_model_id) => {
                self.model_install = Some(ModelInstallSummary {
                    status: ModelInstallStatus::Queued,
                    downloaded: 0,
                    total_bytes: 0,
                });
            }
            WorkerEvent::RestartRequired(_model_id) => {
                self.notice = Some(NoticeSummary {
                    level: NoticeLevel::Warning,
                    message: "model-restart-required".to_owned(),
                });
            }
            WorkerEvent::Error(_sanitized_error) => {
                self.notice = Some(NoticeSummary {
                    level: NoticeLevel::Error,
                    message: "runtime-operation-failed".to_owned(),
                });
            }
            _ => {}
        }
    }
}

fn update_wiki_scan(
    scans: &mut Vec<WikiScanSummary>,
    collection_id: Uuid,
    state: Option<worker::CollectionScanState>,
) {
    let collection_id = collection_id.to_string();
    scans.retain(|scan| scan.wiki_id != collection_id);
    if let Some(state) = state {
        scans.push(WikiScanSummary {
            wiki_id: collection_id,
            state: match state {
                worker::CollectionScanState::Queued => WikiScanStatus::Queued,
                worker::CollectionScanState::Scanning => WikiScanStatus::Scanning,
            },
        });
    }
}

fn guided_repair_preview_summary(
    request_id: Uuid,
    preview: GuidedRepairPreview,
) -> GuidedRepairSummary {
    GuidedRepairSummary {
        request_id: request_id.to_string(),
        wiki_id: preview.collection_id.to_string(),
        status: GuidedRepairStatus::Prepared,
        impact_code: Some(preview.impact_code),
        authorities: preview
            .authorities
            .into_iter()
            .map(|authority| match authority {
                RepairAuthority::HumanReview => RepairAuthorityDto::HumanReview,
                RepairAuthority::PublishedDatabase => RepairAuthorityDto::PublishedDatabase,
            })
            .collect(),
        files: preview
            .files
            .into_iter()
            .map(|file| GuidedRepairFileSummary {
                page: file.page.into(),
                change: match file.change {
                    GuidedRepairChange::WithdrawConcept => GuidedRepairChangeDto::WithdrawConcept,
                    GuidedRepairChange::RemoveOrphan => GuidedRepairChangeDto::RemoveOrphan,
                    GuidedRepairChange::RegenerateIndex => GuidedRepairChangeDto::RegenerateIndex,
                    GuidedRepairChange::AppendDeprecationHistory => {
                        GuidedRepairChangeDto::AppendDeprecationHistory
                    }
                },
            })
            .collect(),
        concepts_returned_to_review: preview.concepts_returned_to_review.len(),
        orphan_concepts_removed: preview.orphan_concepts_removed.len(),
    }
}

fn failed_guided_repair(request_id: Uuid, collection_id: Uuid) -> GuidedRepairSummary {
    GuidedRepairSummary {
        request_id: request_id.to_string(),
        wiki_id: collection_id.to_string(),
        status: GuidedRepairStatus::Failed,
        impact_code: None,
        authorities: Vec::new(),
        files: Vec::new(),
        concepts_returned_to_review: 0,
        orphan_concepts_removed: 0,
    }
}

fn update_running_review(review_ids: &mut Vec<String>, concept_id: Uuid, running: bool) {
    let concept_id = concept_id.to_string();
    review_ids.retain(|current| current != &concept_id);
    if running {
        review_ids.push(concept_id);
    }
}

const fn wiki_health_generation_applies(current: Option<u64>, candidate: u64) -> bool {
    match current {
        Some(current) => candidate >= current,
        None => true,
    }
}

const fn worker_event_request_id(event: &WorkerEvent) -> Option<Uuid> {
    match event {
        WorkerEvent::DesktopPreferencesUpdated { request_id, .. }
        | WorkerEvent::AutostartUpdated { request_id, .. }
        | WorkerEvent::UpdaterUpdated { request_id, .. }
        | WorkerEvent::ConnectivityPlatformUpdated { request_id, .. }
        | WorkerEvent::FirewallOperationUpdated { request_id, .. }
        | WorkerEvent::LanRuntimeUpdated { request_id, .. }
        | WorkerEvent::WikiHealthUpdated { request_id, .. }
        | WorkerEvent::GuidedWikiRepairPrepared { request_id, .. }
        | WorkerEvent::GuidedWikiRepairFinished { request_id, .. }
        | WorkerEvent::ReviewEvidenceLoaded { request_id, .. }
        | WorkerEvent::KnowledgeBundleLoaded { request_id, .. }
        | WorkerEvent::KnowledgePageLoaded { request_id, .. }
        | WorkerEvent::SearchFinished { request_id, .. }
        | WorkerEvent::SearchPartial { request_id, .. }
        | WorkerEvent::PublicBrowseFinished { request_id, .. }
        | WorkerEvent::ChatIntegrationsUpdated { request_id, .. } => Some(*request_id),
        _ => None,
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
        WorkerEvent::GuidedWikiRepairPrepared {
            request_id,
            collection_id,
            ..
        }
        | WorkerEvent::GuidedWikiRepairFinished {
            request_id,
            collection_id,
            ..
        } => remove_matching_request(&mut requests.guided_repair, collection_id, request_id),
        WorkerEvent::PublicBrowseFinished { request_id, .. } => {
            if requests.public_browse == Some(*request_id) {
                requests.public_browse = None;
                true
            } else {
                false
            }
        }
        WorkerEvent::DesktopPreferencesUpdated { request_id, .. } if request_id.is_nil() => true,
        WorkerEvent::DesktopPreferencesUpdated { request_id, .. } => {
            if requests.preferences == Some(*request_id) {
                requests.preferences = None;
                true
            } else {
                false
            }
        }
        WorkerEvent::AutostartUpdated { request_id, .. } if request_id.is_nil() => true,
        WorkerEvent::AutostartUpdated { request_id, .. } => {
            if requests.autostart == Some(*request_id) {
                requests.autostart = None;
                true
            } else {
                false
            }
        }
        WorkerEvent::UpdaterUpdated { request_id, .. } if request_id.is_nil() => true,
        WorkerEvent::UpdaterUpdated { request_id, .. } => match requests.updater {
            Some(current) if current == *request_id => {
                requests.updater = None;
                true
            }
            Some(_) => false,
            None => true,
        },
        WorkerEvent::WikiHealthUpdated { request_id, .. } if request_id.is_nil() => true,
        WorkerEvent::WikiHealthUpdated { request_id, .. } => {
            if requests.wiki_health == Some(*request_id) {
                requests.wiki_health = None;
                true
            } else {
                false
            }
        }
        WorkerEvent::ConnectivityPlatformUpdated { request_id, .. } if request_id.is_nil() => true,
        WorkerEvent::ConnectivityPlatformUpdated { request_id, .. } => {
            match requests.connectivity {
                Some(current) if current == *request_id => {
                    requests.connectivity = None;
                    true
                }
                Some(_) => false,
                None => true,
            }
        }
        WorkerEvent::FirewallOperationUpdated { request_id, .. } if request_id.is_nil() => true,
        WorkerEvent::FirewallOperationUpdated { request_id, .. } => {
            requests.connectivity == Some(*request_id)
        }
        WorkerEvent::ChatIntegrationsUpdated { request_id, .. } => {
            if requests.integrations == Some(*request_id) {
                requests.integrations = None;
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

impl From<worker::CollectionView> for WikiSummary {
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
            public_description: value.public_description,
            public_languages: value.public_languages,
            public_announcement: match value.public_announcement {
                worker::PublicAnnouncementStatusView::Offline => PublicAnnouncementSummary::Offline,
                worker::PublicAnnouncementStatusView::Advertised {
                    accepted_indexes, ..
                } => PublicAnnouncementSummary::Advertised { accepted_indexes },
                worker::PublicAnnouncementStatusView::Expired { .. } => {
                    PublicAnnouncementSummary::Expired
                }
            },
            maintenance_required: maintenance_requires_attention(
                value.maintenance.map(|maintenance| maintenance.status),
            ),
            origin: match value.origin {
                airwiki_core::WikiOrigin::Folder => WikiOriginDto::Folder,
                airwiki_core::WikiOrigin::ImportedOkf => WikiOriginDto::ImportedOkf,
                airwiki_core::WikiOrigin::AiMemory => WikiOriginDto::AiMemory,
            },
            indexing_mode: match value.indexing_mode {
                airwiki_core::IndexingMode::Continuous => IndexingModeDto::Continuous,
                airwiki_core::IndexingMode::Manual => IndexingModeDto::Manual,
                airwiki_core::IndexingMode::NotApplicable => IndexingModeDto::NotApplicable,
            },
            okf_version: value.okf_version,
            trust_summary: match value.trust_summary {
                worker::TrustSummaryView::Unverified => TrustSummaryDto::Unverified,
                worker::TrustSummaryView::MachineConfirmed => TrustSummaryDto::MachineConfirmed,
                worker::TrustSummaryView::HumanReviewed => TrustSummaryDto::HumanReviewed,
                worker::TrustSummaryView::VerificationOutdated => {
                    TrustSummaryDto::VerificationOutdated
                }
            },
        }
    }
}

const fn maintenance_requires_attention(status: Option<CollectionMaintenanceStatus>) -> bool {
    matches!(
        status,
        Some(
            CollectionMaintenanceStatus::Partial
                | CollectionMaintenanceStatus::Failed
                | CollectionMaintenanceStatus::Quarantined
        )
    )
}

impl From<worker::ReviewItemView> for ReviewSummary {
    fn from(value: worker::ReviewItemView) -> Self {
        Self {
            concept_id: value.concept_id.to_string(),
            wiki_id: value.collection_id.to_string(),
            source_revision: value.source_revision,
            source_name: value.source_name,
            wiki_name: value.collection_name,
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
        wiki_id: collection_id.to_string(),
        page: page_id.into(),
        title: String::new(),
        status: KnowledgePageStatus::Failed,
        blocks: Vec::new(),
        metadata: Vec::new(),
        backlinks: Vec::new(),
        truncated: false,
    }
}

fn knowledge_graph_links(links: &[KnowledgeLinkView]) -> Vec<KnowledgeGraphLinkSummary> {
    links
        .iter()
        .filter_map(|link| {
            let KnowledgeLinkDisposition::Internal(target) = link.disposition else {
                return None;
            };
            Some(KnowledgeGraphLinkSummary {
                source: link.source.into(),
                target: target.into(),
                label: link.label.clone(),
            })
        })
        .collect()
}

fn knowledge_page_summary(page: KnowledgePageView) -> KnowledgePageSummary {
    KnowledgePageSummary {
        wiki_id: page.collection_id.to_string(),
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

fn republish_snapshot_after_progress_lag(
    snapshot_sender: &watch::Sender<PublishedSnapshot>,
    snapshot: &mut AppSnapshot,
) -> bool {
    snapshot.sequence = snapshot.sequence.saturating_add(1);
    snapshot_sender
        .send(PublishedSnapshot {
            snapshot: snapshot.clone(),
            request_id: None,
        })
        .is_ok()
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

#[cfg(test)]
fn ui_bindings_source() -> String {
    let config = ts_rs::Config::default();
    let declarations = [
        exported_declaration::<ConceptTypeDto>(&config),
        exported_declaration::<SuggestedEntityDto>(&config),
        exported_declaration::<SuggestedLinkDto>(&config),
        exported_declaration::<EnrichmentDraftDto>(&config),
        exported_declaration::<WikiOriginDto>(&config),
        exported_declaration::<IndexingModeDto>(&config),
        exported_declaration::<TrustSummaryDto>(&config),
        exported_declaration::<WikiSummary>(&config),
        exported_declaration::<PublicAnnouncementSummary>(&config),
        exported_declaration::<HardwareSummary>(&config),
        exported_declaration::<WikiScanStatus>(&config),
        exported_declaration::<WikiScanSummary>(&config),
        exported_declaration::<ReviewSummary>(&config),
        exported_declaration::<ReviewExcerptSummary>(&config),
        exported_declaration::<ReviewEvidenceStatus>(&config),
        exported_declaration::<ReviewEvidenceSummary>(&config),
        exported_declaration::<KnowledgePageInput>(&config),
        exported_declaration::<KnowledgeBlock>(&config),
        exported_declaration::<KnowledgeConceptSummary>(&config),
        exported_declaration::<KnowledgeGraphLinkSummary>(&config),
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
        exported_declaration::<PublicBrowseStatus>(&config),
        exported_declaration::<PublicConceptSummaryDto>(&config),
        exported_declaration::<PublicBrowseSummary>(&config),
        exported_declaration::<NoticeLevel>(&config),
        exported_declaration::<NoticeSummary>(&config),
        exported_declaration::<LocalePreferenceDto>(&config),
        exported_declaration::<ThemePreferenceDto>(&config),
        exported_declaration::<LanPreferenceDto>(&config),
        exported_declaration::<CloseBehaviorDto>(&config),
        exported_declaration::<AutostartStatusDto>(&config),
        exported_declaration::<WikiHealthStatus>(&config),
        exported_declaration::<WikiHealthSummary>(&config),
        exported_declaration::<GuidedRepairStatus>(&config),
        exported_declaration::<RepairAuthorityDto>(&config),
        exported_declaration::<GuidedRepairChangeDto>(&config),
        exported_declaration::<GuidedRepairFileSummary>(&config),
        exported_declaration::<GuidedRepairSummary>(&config),
        exported_declaration::<SystemPermissionStatus>(&config),
        exported_declaration::<NetworkProfileStatus>(&config),
        exported_declaration::<FirewallStatus>(&config),
        exported_declaration::<FirewallHelperStatus>(&config),
        exported_declaration::<ConnectivitySummary>(&config),
        exported_declaration::<LanListenerStatus>(&config),
        exported_declaration::<LanDiscoveryStatus>(&config),
        exported_declaration::<LanRuntimeSummary>(&config),
        exported_declaration::<FirewallOperationStatus>(&config),
        exported_declaration::<SystemDestinationInput>(&config),
        exported_declaration::<IntegrationClientDto>(&config),
        exported_declaration::<IntegrationStatusDto>(&config),
        exported_declaration::<IntegrationSummary>(&config),
        exported_declaration::<IntegrationsSummary>(&config),
        exported_declaration::<IntegrationActionInput>(&config),
        exported_declaration::<UpdaterStatusDto>(&config),
        exported_declaration::<UpdaterIssueDto>(&config),
        exported_declaration::<UpdaterSummary>(&config),
        exported_declaration::<PreferencesSummary>(&config),
        exported_declaration::<PreferencesInput>(&config),
        exported_declaration::<HostPlatform>(&config),
        exported_declaration::<AppPhase>(&config),
        exported_declaration::<AppSnapshot>(&config),
        exported_declaration::<UiEventKind>(&config),
        exported_declaration::<UiEventEnvelope>(&config),
        exported_declaration::<UiError>(&config),
        exported_declaration::<FolderSelection>(&config),
        exported_declaration::<OkfImportSummary>(&config),
        exported_declaration::<WikiPolicyInput>(&config),
    ]
    .join("\n\n");
    format!(
        "// Generated by `cargo run --locked -p xtask -- ui-bindings generate`.\n// Do not edit by hand.\n\n{declarations}\n"
    )
}

#[cfg(test)]
fn exported_declaration<T: TS>(config: &ts_rs::Config) -> String {
    format!("export {}", T::decl(config))
}

impl From<worker::SourceIssueView> for SourceIssueSummary {
    fn from(value: worker::SourceIssueView) -> Self {
        Self {
            wiki_id: value.collection_id.to_string(),
            source_name: value.source_name,
            wiki_name: value.collection_name,
            code: format!("{:?}", value.code),
        }
    }
}

impl From<worker::PeerView> for PeerSummary {
    fn from(value: worker::PeerView) -> Self {
        let mut granted_wiki_ids = value
            .granted_collections
            .into_iter()
            .map(|collection_id| collection_id.to_string())
            .collect::<Vec<_>>();
        granted_wiki_ids.sort();
        Self {
            peer_id: value.peer_id,
            device_name: value.device_name,
            address: value.address,
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
            sas_words: value.sas_words.map(Vec::from),
            granted_wiki_ids,
        }
    }
}

impl From<worker::ModelStateView> for ModelSummary {
    fn from(value: worker::ModelStateView) -> Self {
        Self {
            state_sequence: value.state_sequence,
            profile: match value.profile {
                ModelProfile::Automatic => "automatic",
                ModelProfile::Efficient => "efficient",
                ModelProfile::Quality => "quality",
            }
            .to_owned(),
            recommended_model_id: value.recommended_model_id,
            display_name: value.recommended_display_name,
            recommendation_reason: value.recommendation_reason,
            active: value.active_model_id.is_some(),
            installed: value.recommended_assets_installed,
            degraded: value.degraded,
            issues: value.issues,
            pending_model_id: value.pending_model_id,
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
            concept_id: value.concept_id.to_string(),
            wiki_id: value.collection_id.to_string(),
            title: value.title,
            snippet: value.snippet,
            heading_or_page: value.heading_or_page,
            logical_resource_uri: value.logical_resource_uri,
            source_revision: value.source_revision,
            source_sha256: value.source_sha256,
            rank: value.rank,
            node_id: value.node_id,
        }
    }
}

fn main() -> Result<()> {
    let background_requested = launch_in_background(std::env::args_os().skip(1));
    let paths = AppPaths::discover().context("failed to discover application paths")?;
    let logging_guard = init_logging(&paths)?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting AirWiki");
    let (commands, command_receiver) = mpsc::channel(COMMAND_CAPACITY);
    let (worker_events, mut presentation_events) = mpsc::channel(INTERNAL_EVENT_CAPACITY);
    let (progress_events, _) = broadcast::channel(TRANSIENT_EVENT_CAPACITY);
    let worker_progress_events = progress_events.clone();
    let mut presentation_progress_events = progress_events.subscribe();
    let (snapshot_sender, snapshot_receiver) = watch::channel(PublishedSnapshot {
        snapshot: AppSnapshot::starting(),
        request_id: None,
    });
    let review_versions = Arc::new(Mutex::new(HashMap::new()));
    let presentation_review_versions = Arc::clone(&review_versions);
    let knowledge_fingerprints = Arc::new(Mutex::new(HashMap::new()));
    let presentation_knowledge_fingerprints = Arc::clone(&knowledge_fingerprints);
    let guided_repairs = Arc::new(Mutex::new(HashMap::new()));
    let presentation_guided_repairs = Arc::clone(&guided_repairs);
    let requests = Arc::new(Mutex::new(RequestTracker::default()));
    let presentation_requests = Arc::clone(&requests);
    let (worker_finished_sender, worker_finished) = oneshot::channel();
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();

    let builder = tauri::Builder::default();
    #[cfg(feature = "e2e")]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());

    let result = builder
        .plugin(navigation_guard())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppRuntime {
            commands,
            cancellation,
            snapshot: Mutex::new(snapshot_receiver),
            folder_selections: Mutex::new(HashMap::new()),
            review_versions,
            knowledge_fingerprints,
            guided_repairs,
            requests,
            confirmation_gate: Arc::new(Semaphore::new(1)),
            tray_operational: AtomicBool::new(false),
            exiting: AtomicBool::new(false),
            worker_finished: Mutex::new(Some(worker_finished)),
        })
        .setup(move |app| {
            let updater_backend = UpdaterBuildConfig::from_compile_time()
                .and_then(|config| TauriUpdateBackend::new(app.handle().clone(), config))
                .map(|backend| Box::new(backend) as Box<dyn UpdateBackend>);
            let presentation_app = app.handle().clone();
            let tray_operational = if install_tray(app).is_ok() {
                app.state::<AppRuntime>()
                    .tray_operational
                    .store(true, Ordering::Release);
                true
            } else {
                tracing::warn!(
                    error_kind = "tray_unavailable",
                    "tray initialization failed"
                );
                false
            };
            if !background_requested || !tray_operational {
                show_main_window(app.handle());
            }
            tauri::async_runtime::spawn(async move {
                let mut snapshot = AppSnapshot::starting();
                loop {
                    let event = tokio::select! {
                        event = presentation_events.recv() => {
                            let Some(event) = event else { break; };
                            event
                        }
                        event = presentation_progress_events.recv() => match event {
                            Ok(event) => event,
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                if !republish_snapshot_after_progress_lag(
                                    &snapshot_sender,
                                    &mut snapshot,
                                ) {
                                    break;
                                }
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        },
                    };
                    let request_id = worker_event_request_id(&event);
                    snapshot
                        .apply(
                            event,
                            &presentation_review_versions,
                            &presentation_knowledge_fingerprints,
                            &presentation_guided_repairs,
                            &presentation_requests,
                        )
                        .await;
                    if snapshot_sender
                        .send(PublishedSnapshot {
                            snapshot: snapshot.clone(),
                            request_id,
                        })
                        .is_err()
                    {
                        break;
                    }
                    if snapshot.updater.as_ref().is_some_and(|updater| {
                        matches!(updater.status, UpdaterStatusDto::Installed)
                    }) {
                        begin_shutdown(presentation_app.clone());
                        break;
                    }
                }
            });
            tauri::async_runtime::spawn(async move {
                run_worker(
                    paths,
                    command_receiver,
                    worker_events,
                    worker_progress_events,
                    worker_cancellation,
                    updater_backend,
                )
                .await;
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
                    .and_then(|snapshot| snapshot.borrow().snapshot.preferences)
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
            pick_wiki_folder,
            pick_okf_import,
            validate_okf_import,
            import_okf,
            set_wiki_indexing,
            add_wiki,
            relink_wiki,
            rescan_wiki,
            update_wiki_policy,
            delete_wiki,
            add_federation_index,
            remove_federation_index,
            update_public_wiki_profile,
            browse_public_wiki,
            set_public_publisher_blocked,
            dial_peer,
            pair_peer,
            confirm_pairing,
            revoke_peer,
            allow_peer_pairing_again,
            set_wiki_grant,
            manage_integration,
            search,
            load_review_evidence,
            approve_review,
            reject_review,
            reanalyze_review,
            load_wiki_bundle,
            load_wiki_page,
            update_preferences,
            refresh_autostart,
            set_autostart,
            check_updates,
            download_update,
            install_update,
            refresh_wiki_health,
            prepare_guided_wiki_repair,
            execute_guided_wiki_repair,
            refresh_connectivity,
            configure_firewall,
            open_system_destination,
            open_external_link,
            hide_to_tray,
            quit_completely
        ])
        .run(tauri::generate_context!())
        .map_err(|error| anyhow::anyhow!(error.to_string()));
    drop(logging_guard);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI_BINDINGS_PATH: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/ui/src/generated/ui-contract.ts"
    );

    #[test]
    fn background_mode_requires_the_exact_flag() {
        assert!(launch_in_background(["--background"]));
        assert!(!launch_in_background(["background", "--foreground"]));
    }

    #[test]
    fn successful_maintenance_does_not_require_attention() {
        assert!(!maintenance_requires_attention(Some(
            CollectionMaintenanceStatus::Success
        )));
    }

    #[test]
    fn failed_maintenance_requires_attention() {
        assert!(maintenance_requires_attention(Some(
            CollectionMaintenanceStatus::Failed
        )));
    }

    #[test]
    fn pending_maintenance_does_not_require_human_attention() {
        assert!(!maintenance_requires_attention(Some(
            CollectionMaintenanceStatus::Never
        )));
    }

    #[test]
    fn navigation_guard_allows_only_local_application_origins()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(navigation_is_allowed(&url::Url::parse(
            "tauri://localhost/library"
        )?));
        assert!(navigation_is_allowed(&url::Url::parse(
            "http://tauri.localhost/system"
        )?));
        assert!(navigation_is_allowed(&url::Url::parse("about:blank")?));
        assert!(!navigation_is_allowed(&url::Url::parse(
            "https://airwiki.example.test/phishing"
        )?));
        assert!(!navigation_is_allowed(&url::Url::parse(
            "file:///tmp/untrusted.html"
        )?));
        Ok(())
    }

    #[test]
    fn every_native_confirmation_has_localized_copy() -> Result<(), Box<dyn std::error::Error>> {
        let confirmations = [
            NativeConfirmation::ModelLicenses,
            NativeConfirmation::ExternalLink,
            NativeConfirmation::GuidedRepair,
            NativeConfirmation::ExternalCollectionPolicy,
            NativeConfirmation::CollectionGrant,
            NativeConfirmation::InstallUpdate,
        ];
        for locale in [UiLocale::EnUs, UiLocale::Es] {
            let localization = Localization::new(locale)?;
            assert!(localization.text("native-confirm-title").is_some());
            for confirmation in confirmations {
                assert!(localization.text(confirmation.message_id()).is_some());
            }
        }
        Ok(())
    }

    fn test_runtime(
        commands: mpsc::Sender<WorkerIntent>,
        folder_selections: HashMap<Uuid, PendingFolderSelection>,
    ) -> AppRuntime {
        let (_snapshot_sender, snapshot) = watch::channel(PublishedSnapshot {
            snapshot: AppSnapshot::starting(),
            request_id: None,
        });
        let (_worker_finished_sender, worker_finished) = oneshot::channel();
        AppRuntime {
            commands,
            cancellation: CancellationToken::new(),
            snapshot: Mutex::new(snapshot),
            folder_selections: Mutex::new(folder_selections),
            review_versions: Arc::new(Mutex::new(HashMap::new())),
            knowledge_fingerprints: Arc::new(Mutex::new(HashMap::new())),
            guided_repairs: Arc::new(Mutex::new(HashMap::new())),
            requests: Arc::new(Mutex::new(RequestTracker::default())),
            confirmation_gate: Arc::new(Semaphore::new(1)),
            tray_operational: AtomicBool::new(false),
            exiting: AtomicBool::new(false),
            worker_finished: Mutex::new(Some(worker_finished)),
        }
    }

    fn runtime_with_selection(token: Uuid, path: PathBuf, expires_at: Instant) -> AppRuntime {
        let (commands, _receiver) = mpsc::channel(COMMAND_CAPACITY);
        test_runtime(
            commands,
            HashMap::from([(token, PendingFolderSelection { path, expires_at })]),
        )
    }

    #[tokio::test]
    async fn command_response_requires_worker_acceptance() {
        let (commands, mut receiver) = mpsc::channel(COMMAND_CAPACITY);
        let runtime = test_runtime(commands, HashMap::new());
        let worker = tokio::spawn(async move {
            if let Some(intent) = receiver.recv().await {
                let _ = intent.accepted.send(());
            }
        });

        assert!(
            send_command(&runtime, WorkerCommand::CancelInstall)
                .await
                .is_ok()
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(1), worker)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn command_response_fails_when_worker_is_disconnected() {
        let (commands, receiver) = mpsc::channel(COMMAND_CAPACITY);
        drop(receiver);
        let runtime = test_runtime(commands, HashMap::new());

        let error = send_command(&runtime, WorkerCommand::CancelInstall).await;
        assert!(matches!(
            error,
            Err(UiError {
                code: "unavailable",
                ..
            })
        ));
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
    fn graph_contract_exposes_only_internal_knowledge_links() {
        let concept_id = Uuid::new_v4();
        let links = vec![
            KnowledgeLinkView {
                source: KnowledgePageId::Index,
                label: "Verified concept".to_owned(),
                raw_target: format!("concepts/{concept_id}.md"),
                disposition: KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(
                    concept_id,
                )),
            },
            KnowledgeLinkView {
                source: KnowledgePageId::Index,
                label: "External site".to_owned(),
                raw_target: "https://example.com".to_owned(),
                disposition: KnowledgeLinkDisposition::External,
            },
            KnowledgeLinkView {
                source: KnowledgePageId::Index,
                label: "Unsafe target".to_owned(),
                raw_target: "file:///private/data".to_owned(),
                disposition: KnowledgeLinkDisposition::Unsafe,
            },
        ];

        assert_eq!(
            knowledge_graph_links(&links),
            vec![KnowledgeGraphLinkSummary {
                source: KnowledgePageInput::Index,
                target: KnowledgePageInput::Concept { id: concept_id },
                label: "Verified concept".to_owned(),
            }]
        );
    }

    #[test]
    fn transient_operation_states_replace_and_clear_by_identifier() {
        let collection_id = Uuid::new_v4();
        let concept_id = Uuid::new_v4();
        let mut scans = Vec::new();
        let mut reviews = Vec::new();

        update_wiki_scan(
            &mut scans,
            collection_id,
            Some(worker::CollectionScanState::Queued),
        );
        update_wiki_scan(
            &mut scans,
            collection_id,
            Some(worker::CollectionScanState::Scanning),
        );
        update_running_review(&mut reviews, concept_id, true);

        assert_eq!(
            scans,
            vec![WikiScanSummary {
                wiki_id: collection_id.to_string(),
                state: WikiScanStatus::Scanning,
            }]
        );
        assert_eq!(reviews, vec![concept_id.to_string()]);

        update_wiki_scan(&mut scans, collection_id, None);
        update_running_review(&mut reviews, concept_id, false);

        assert!(scans.is_empty());
        assert!(reviews.is_empty());
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
    fn stale_updater_events_are_discarded() {
        let current = Uuid::new_v4();
        let stale = Uuid::new_v4();
        let requests = Mutex::new(RequestTracker {
            updater: Some(current),
            ..RequestTracker::default()
        });
        let event = |request_id| WorkerEvent::UpdaterUpdated {
            request_id,
            result: Ok(worker::UpdaterWorkerView::Ready(UpdaterView {
                status: UpdaterStatus::UpToDate,
                last_issue: None,
            })),
        };

        assert!(!request_is_current(&event(stale), &requests));
        assert!(request_is_current(&event(current), &requests));
        assert!(request_is_current(&event(Uuid::new_v4()), &requests));
    }

    #[test]
    fn updater_contract_exposes_only_sanitized_issue_codes() {
        let summary = UpdaterSummary::from(UpdaterView {
            status: UpdaterStatus::Idle,
            last_issue: Some(crate::updater::UpdateIssue {
                code: UpdateIssueCode::InvalidSignature,
                retryable: false,
            }),
        });

        assert!(matches!(
            summary.issue,
            Some(UpdaterIssueDto::InvalidSignature)
        ));
        assert!(!summary.retryable);
        assert!(summary.version.is_none());
        assert!(summary.release_notes.is_none());
    }

    #[test]
    fn stale_autostart_response_cannot_replace_current_request() {
        let current = Uuid::new_v4();
        let stale = Uuid::new_v4();
        let requests = Mutex::new(RequestTracker {
            autostart: Some(current),
            ..RequestTracker::default()
        });

        assert!(!request_is_current(
            &WorkerEvent::AutostartUpdated {
                request_id: stale,
                result: Ok(autostart::AutostartStatus::Disabled),
            },
            &requests,
        ));
        assert!(request_is_current(
            &WorkerEvent::AutostartUpdated {
                request_id: current,
                result: Ok(autostart::AutostartStatus::Enabled),
            },
            &requests,
        ));
        assert!(!request_is_current(
            &WorkerEvent::AutostartUpdated {
                request_id: current,
                result: Ok(autostart::AutostartStatus::Enabled),
            },
            &requests,
        ));
    }

    #[test]
    fn wiki_health_generation_never_moves_backwards() {
        assert_eq!(
            (
                wiki_health_generation_applies(Some(8), 8),
                wiki_health_generation_applies(Some(8), 7),
            ),
            (true, false)
        );
    }

    #[test]
    fn connectivity_request_rejects_stale_completion_while_pending() {
        let current = Uuid::new_v4();
        let stale = Uuid::new_v4();
        let requests = Mutex::new(RequestTracker {
            connectivity: Some(current),
            ..RequestTracker::default()
        });
        let stale_applies = request_is_current(
            &WorkerEvent::ConnectivityPlatformUpdated {
                request_id: stale,
                result: Err(worker::ConnectivityIssueCode::Busy),
            },
            &requests,
        );
        let current_applies = request_is_current(
            &WorkerEvent::ConnectivityPlatformUpdated {
                request_id: current,
                result: Err(worker::ConnectivityIssueCode::Busy),
            },
            &requests,
        );

        assert_eq!((stale_applies, current_applies), (false, true));
    }

    #[test]
    fn peer_identifier_validation_rejects_ambiguous_input() {
        assert_eq!(
            (
                validate_peer_id("synthetic-peer".to_owned()).is_ok(),
                validate_peer_id(" synthetic-peer".to_owned()).is_err(),
                validate_peer_id("synthetic\npeer".to_owned()).is_err(),
            ),
            (true, true, true)
        );
    }

    #[test]
    fn integration_contract_excludes_paths_and_diagnostic_detail() -> Result<()> {
        let summary = IntegrationsSummary::from(integrations::ChatIntegrationsSnapshot {
            integrations: vec![integrations::IntegrationView {
                client: integrations::ChatClientKind::ClaudeDesktop,
                status: integrations::IntegrationStatus::Error,
                detected_version: Some("synthetic".to_owned()),
                detail: "sensitive diagnostic".to_owned(),
                planned_path: Some(PathBuf::from("/synthetic/private/config")),
                activity_recent: false,
                restart_required: false,
            }],
            external_ai_collection_count: 0,
        });
        let serialized = serde_json::to_value(summary)?;
        let integration = serialized
            .pointer("/integrations/0")
            .context("integration DTO missing from serialized fixture")?;

        assert_eq!(
            (
                integration.get("detail").is_none(),
                integration.get("plannedPath").is_none(),
            ),
            (true, true)
        );
        Ok(())
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
    fn tray_icon_has_expected_visible_rgba_pixels() {
        let icon = tray_icon();

        assert_eq!(icon.width(), TRAY_ICON_WIDTH);
        assert_eq!(icon.height(), TRAY_ICON_HEIGHT);
        assert_eq!(icon.rgba().len(), TRAY_ICON_RGBA.len());
        assert!(
            icon.rgba()
                .chunks_exact(4)
                .any(|pixel| matches!(pixel, [_, _, _, alpha] if *alpha != 0))
        );
    }

    #[test]
    fn tray_only_restores_the_window_on_left_click() {
        assert!(tray_click_opens_window(MouseButton::Left));
        assert!(!tray_click_opens_window(MouseButton::Right));
        assert!(!tray_click_opens_window(MouseButton::Middle));
    }

    #[test]
    fn lagged_progress_receiver_republishes_a_complete_snapshot() {
        let (progress_sender, _) = broadcast::channel(1);
        let mut progress_receiver = progress_sender.subscribe();
        assert!(progress_sender.send(WorkerEvent::InstallStopped).is_ok());
        assert!(progress_sender.send(WorkerEvent::ModelsReady).is_ok());
        assert!(matches!(
            progress_receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(1))
        ));

        let mut snapshot = AppSnapshot::starting();
        let (snapshot_sender, snapshot_receiver) = watch::channel(PublishedSnapshot {
            snapshot: snapshot.clone(),
            request_id: Some(Uuid::new_v4()),
        });

        assert!(republish_snapshot_after_progress_lag(
            &snapshot_sender,
            &mut snapshot,
        ));
        let published = snapshot_receiver.borrow();
        assert_eq!(published.snapshot.sequence, 1);
        assert!(published.request_id.is_none());
        assert!(matches!(published.snapshot.phase, AppPhase::Starting));
    }

    #[test]
    fn snapshot_reports_the_compile_target_platform() -> Result<()> {
        let serialized = serde_json::to_value(HostPlatform::CURRENT)?;
        #[cfg(target_os = "macos")]
        assert_eq!(serialized, serde_json::Value::String("macOs".to_owned()));
        #[cfg(target_os = "windows")]
        assert_eq!(serialized, serde_json::Value::String("windows".to_owned()));
        Ok(())
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
    fn tauri_updater_plugin_has_an_explicit_disabled_default() -> Result<()> {
        let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        let config: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&config_path).context("failed to read the Tauri configuration")?,
        )
        .context("failed to parse the Tauri configuration")?;
        let updater = config
            .pointer("/plugins/updater")
            .context("the updater plugin requires an explicit configuration object")?;

        anyhow::ensure!(
            updater
                .pointer("/endpoints")
                .and_then(serde_json::Value::as_array)
                .is_some_and(Vec::is_empty),
            "the base updater configuration must not contain network endpoints"
        );
        anyhow::ensure!(
            updater
                .pointer("/pubkey")
                .and_then(serde_json::Value::as_str)
                == Some(""),
            "the base updater key must remain empty; release values are compile-time only"
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
