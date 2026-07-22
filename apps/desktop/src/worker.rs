use std::{
    collections::{HashMap, HashSet, VecDeque},
    panic::AssertUnwindSafe,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use airwiki_core::{
    CollectionMaintenanceRecord, GuidedRepairPreview, GuidedRepairResult, IngestOutcome,
    KnowledgeBundleView, KnowledgePageId, KnowledgePageView, ReviewVersionToken, WikiRepairError,
};
use airwiki_inference::{
    AssetManager, E5_FILES, E5_REVISION, HardwareReport, InstallEvent, InstallOutcome, InstallPlan,
    LLAMA_CPP_BUILD, MMARCO_COMMON_FILES, MMARCO_REVISION, ModelDecision, ModelProfile,
    ModelSelection, diagnose_hardware, install_failure_is_transient, select_model,
    selection_for_model,
};
use airwiki_mcp::{McpClientActivity, McpClientKind};
use airwiki_network::{NetworkEvent, PublicRouteKind};
use airwiki_types::{
    CollectionPolicy, EnrichmentDraft, PublicConceptSummary, SearchHit, SearchPurpose,
    SearchResponse,
};
use futures::FutureExt;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    autostart::{AutostartManager, AutostartStatus},
    connectivity_platform::{
        ConnectivityPlatformSnapshot, FirewallActionError, FirewallDiagnosticState,
        FirewallHelperState, LanRuntimePolicy, NetworkProfileState,
        diagnose as diagnose_connectivity, install_firewall_rules, lan_runtime_policy,
        open_advanced_firewall_rules, remove_firewall_rules,
    },
    integrations::{
        ChatClientKind, ChatIntegrationManager, ChatIntegrationsSnapshot, IntegrationAction,
        IntegrationStatus, IntegrationView,
    },
    model_config::{
        CloseBehavior, DesktopConfig, LanPreference, LocalePreference, ONBOARDING_VERSION,
    },
    paths::AppPaths,
    services::{
        CollectionWatchEvent, CollectionWatcherHandle, DesktopServices, ModelRuntimePaths,
        WikiHealthRollup,
    },
    updater::{
        PackagerUpdateBackend, UpdateSchedule, UpdaterBuildConfig, UpdaterDisabledReason,
        UpdaterService, UpdaterView, schedule_jitter,
    },
};

#[derive(Debug, Clone)]
pub struct CollectionView {
    pub id: Uuid,
    pub name: String,
    pub folder: PathBuf,
    pub document_count: usize,
    pub needs_review_count: usize,
    pub published_count: usize,
    pub failed_count: usize,
    pub local_only: bool,
    pub peer_shareable: bool,
    pub allow_external_ai: bool,
    pub internet_public: bool,
    pub public_description: String,
    pub public_languages: String,
    pub public_accepted_indexes: usize,
    pub public_last_announced_at: Option<chrono::DateTime<chrono::Utc>>,
    pub public_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub maintenance: Option<CollectionMaintenanceRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionScanState {
    Queued,
    Scanning,
}

#[derive(Debug, Clone)]
pub struct ReviewItemView {
    pub concept_id: Uuid,
    pub source_revision: u32,
    pub source_name: String,
    pub collection_name: String,
    pub draft: EnrichmentDraft,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReviewEvidenceExcerptView {
    pub ordinal: u32,
    pub heading_or_page: String,
    pub text: String,
    pub truncated: bool,
}

impl std::fmt::Debug for ReviewEvidenceExcerptView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReviewEvidenceExcerptView")
            .field("ordinal", &self.ordinal)
            .field("heading_present", &!self.heading_or_page.is_empty())
            .field("text_bytes", &self.text.len())
            .field("truncated", &self.truncated)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReviewEvidencePageView {
    pub concept_id: Uuid,
    pub source_revision: u32,
    pub review_version: ReviewVersionToken,
    pub excerpts: Vec<ReviewEvidenceExcerptView>,
    pub total_chunks: usize,
    pub next_ordinal: Option<u32>,
}

impl std::fmt::Debug for ReviewEvidencePageView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReviewEvidencePageView")
            .field("source_revision", &self.source_revision)
            .field("excerpt_count", &self.excerpts.len())
            .field("total_chunks", &self.total_chunks)
            .field("next_ordinal", &self.next_ordinal)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEvidenceErrorView {
    NoLongerPending,
    MissingEvidence,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIssueView {
    pub collection_id: Uuid,
    pub source_name: String,
    pub collection_name: String,
    pub code: airwiki_core::SourceIssueCode,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerTrustState {
    Unpaired,
    Trusted,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerActivityState {
    NotObserved,
    Discovered,
    Pairing,
    Connected,
}

#[derive(Debug, Clone)]
pub struct PeerView {
    pub peer_id: String,
    pub device_name: Option<String>,
    pub address: String,
    pub trust: PeerTrustState,
    pub activity: PeerActivityState,
    pub sas_words: Option<[String; 6]>,
    pub granted_collections: HashSet<Uuid>,
}

#[derive(Debug, Clone)]
pub struct ModelStateView {
    pub state_sequence: u64,
    pub profile: ModelProfile,
    pub recommended_model_id: Option<String>,
    pub recommended_display_name: Option<String>,
    pub recommendation_reason: Option<String>,
    pub degraded: bool,
    pub issues: Vec<String>,
    pub active_model_id: Option<String>,
    pub pending_model_id: Option<String>,
    /// All immutable artifacts required by the recommendation are already present and passed
    /// their integrity checks. Activation can still require a smoke test or an application
    /// restart, so this is deliberately separate from `active_model_id`/`pending_model_id`.
    pub recommended_assets_installed: bool,
    pub download_bytes: u64,
    pub required_free_bytes: u64,
    pub fits_available_disk: bool,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub revision: Option<String>,
    pub license_accepted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopPreferencesView {
    pub completed_onboarding_version: Option<u32>,
    pub locale: LocalePreference,
    pub lan_preference: LanPreference,
    pub close_behavior: CloseBehavior,
    pub automatic_update_checks: bool,
}

impl From<&DesktopConfig> for DesktopPreferencesView {
    fn from(config: &DesktopConfig) -> Self {
        Self {
            completed_onboarding_version: config.completed_onboarding_version,
            locale: config.locale,
            lan_preference: config.lan_preference,
            close_behavior: config.close_behavior,
            automatic_update_checks: config.automatic_update_checks,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopPreferencesUpdate {
    pub locale: LocalePreference,
    pub lan_preference: LanPreference,
    pub close_behavior: CloseBehavior,
    pub automatic_update_checks: bool,
    pub complete_onboarding: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdaterWorkerView {
    Disabled(UpdaterDisabledReason),
    Ready(UpdaterView),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanListenerView {
    Stopped,
    Starting,
    Listening,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanDiscoveryView {
    Disabled,
    Starting,
    Active,
    Failed,
}

/// Sanitized coverage state for the desktop search UI.
///
/// Transport diagnostics and authenticated peer identifiers remain inside the
/// search/network layers. The regular UI only needs enough information to
/// explain whether the result set may be incomplete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchCoverageView {
    Complete,
    FederationDisabled,
    OfflineDevices { count: usize },
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallOperationView {
    AwaitingWindows,
    TakingLonger,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WikiHealthSummaryView {
    pub error_count: usize,
    pub warning_count: usize,
    pub updating_count: usize,
    pub attention_collection_id: Option<Uuid>,
    pub checked_at: Option<SystemTime>,
}

/// Stable, localized-by-the-UI connectivity failures.
///
/// Raw operating-system errors and paths deliberately do not cross the worker
/// boundary. Keeping the cause typed prevents an English UI from receiving a
/// Spanish integration string and lets each recovery path remain explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectivityIssueCode {
    Busy,
    FirewallCancelled,
    FirewallManagedPolicy,
    FirewallInboundBlocked,
    FirewallConflict,
    FirewallInstallationInvalid,
    FirewallUnsupported,
    FirewallStateChanged,
    FirewallInternal,
}

impl From<FirewallActionError> for ConnectivityIssueCode {
    fn from(error: FirewallActionError) -> Self {
        match error {
            FirewallActionError::Cancelled => Self::FirewallCancelled,
            FirewallActionError::ManagedPolicy => Self::FirewallManagedPolicy,
            FirewallActionError::InboundBlocked => Self::FirewallInboundBlocked,
            FirewallActionError::Conflict => Self::FirewallConflict,
            FirewallActionError::InvalidLayoutOrSignature => Self::FirewallInstallationInvalid,
            FirewallActionError::Unsupported => Self::FirewallUnsupported,
            FirewallActionError::StateChanged => Self::FirewallStateChanged,
            FirewallActionError::Internal => Self::FirewallInternal,
        }
    }
}

#[derive(Debug)]
pub enum WorkerCommand {
    InstallModels,
    CancelInstall,
    SetModelProfile(ModelProfile),
    UpdateDesktopPreferences {
        request_id: Uuid,
        update: DesktopPreferencesUpdate,
    },
    SetAutostart {
        request_id: Uuid,
        enabled: bool,
    },
    RefreshAutostart {
        request_id: Uuid,
    },
    CheckUpdates {
        request_id: Uuid,
    },
    DownloadUpdate {
        request_id: Uuid,
    },
    InstallUpdate {
        request_id: Uuid,
    },
    RefreshConnectivity {
        request_id: Uuid,
    },
    ConfigureFirewall {
        request_id: Uuid,
        install: bool,
    },
    OpenAdvancedFirewall {
        request_id: Uuid,
    },
    RefreshWikiHealth {
        request_id: Uuid,
    },
    PrepareGuidedWikiRepair {
        request_id: Uuid,
        collection_id: Uuid,
    },
    ExecuteGuidedWikiRepair {
        request_id: Uuid,
        preview: GuidedRepairPreview,
    },
    AddCollection {
        name: String,
        folder: PathBuf,
    },
    RelinkCollection {
        collection_id: Uuid,
        folder: PathBuf,
    },
    RescanCollection(Uuid),
    UpdateCollectionPolicy {
        collection_id: Uuid,
        local_only: bool,
        peer_shareable: bool,
        allow_external_ai: bool,
        internet_public: bool,
    },
    Approve {
        concept_id: Uuid,
        expected_review_version: ReviewVersionToken,
        draft: EnrichmentDraft,
    },
    Reject {
        concept_id: Uuid,
    },
    ReanalyzeReview {
        concept_id: Uuid,
    },
    LoadReviewEvidence {
        request_id: Uuid,
        concept_id: Uuid,
        expected_source_revision: u32,
        expected_review_version: Option<ReviewVersionToken>,
        after_ordinal: Option<u32>,
    },
    LoadKnowledgeBundle {
        request_id: Uuid,
        collection_id: Uuid,
    },
    LoadKnowledgePage {
        request_id: Uuid,
        collection_id: Uuid,
        page_id: KnowledgePageId,
        expected_fingerprint: String,
    },
    Search {
        request_id: Uuid,
        question: String,
        top_k: u8,
        purpose: SearchPurpose,
        public_network: bool,
    },
    AddFederationIndex {
        peer_id: String,
        address: String,
    },
    RemoveFederationIndex {
        peer_id: String,
    },
    UpdatePublicCollectionProfile {
        collection_id: Uuid,
        description: String,
        languages: Vec<String>,
    },
    BrowsePublicCollection {
        request_id: Uuid,
        publisher_id: String,
        collection_id: Uuid,
        cursor: Option<String>,
    },
    SetPublicPublisherBlocked {
        publisher_id: String,
        blocked: bool,
    },
    ManageChatIntegration {
        request_id: Uuid,
        action: IntegrationAction,
    },
    Pair {
        peer_id: String,
    },
    Dial {
        address: String,
    },
    ConfirmPairing {
        peer_id: String,
        accepted: bool,
    },
    RevokePeer {
        peer_id: String,
    },
    GrantCollection {
        peer_id: String,
        collection_id: Uuid,
        granted: bool,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Ready {
        node_id: String,
        mcp_url: String,
        collections: Vec<CollectionView>,
        reviews: Vec<ReviewItemView>,
        source_issues: Vec<SourceIssueView>,
        blocked_public_publishers: Vec<String>,
    },
    Hardware(HardwareReport),
    ModelState(ModelStateView),
    DesktopPreferencesUpdated {
        request_id: Uuid,
        result: Result<DesktopPreferencesView, String>,
    },
    AutostartUpdated {
        request_id: Uuid,
        result: Result<AutostartStatus, String>,
    },
    UpdaterUpdated {
        request_id: Uuid,
        result: Result<UpdaterWorkerView, String>,
    },
    ConnectivityPlatformUpdated {
        request_id: Uuid,
        result: Result<ConnectivityPlatformSnapshot, ConnectivityIssueCode>,
    },
    FirewallOperationUpdated {
        request_id: Uuid,
        state: Option<FirewallOperationView>,
    },
    LanRuntimeUpdated {
        request_id: Uuid,
        listener: LanListenerView,
        discovery: LanDiscoveryView,
        local_addresses: Vec<String>,
    },
    WikiHealthUpdated {
        request_id: Uuid,
        generation: u64,
        result: Result<WikiHealthSummaryView, String>,
    },
    WikiMaintenanceFinished {
        collection_id: Uuid,
        repaired: bool,
    },
    GuidedWikiRepairPrepared {
        request_id: Uuid,
        collection_id: Uuid,
        result: Result<GuidedRepairPreview, String>,
    },
    GuidedWikiRepairFinished {
        request_id: Uuid,
        collection_id: Uuid,
        result: Result<GuidedRepairResult, String>,
    },
    InstallProgress(InstallEvent),
    InstallQueued(String),
    InstallStopped,
    ModelsReady,
    ModelsMissing,
    RestartRequired(String),
    Collections(Vec<CollectionView>),
    CollectionScan {
        collection_id: Uuid,
        state: Option<CollectionScanState>,
    },
    Reviews(Vec<ReviewItemView>),
    SourceIssues(Vec<SourceIssueView>),
    ReviewReanalysis {
        concept_id: Uuid,
        running: bool,
    },
    ReviewEvidenceLoaded {
        request_id: Uuid,
        concept_id: Uuid,
        expected_source_revision: u32,
        result: Result<ReviewEvidencePageView, ReviewEvidenceErrorView>,
    },
    KnowledgeBundleLoaded {
        request_id: Uuid,
        collection_id: Uuid,
        result: Result<KnowledgeBundleView, String>,
    },
    KnowledgePageLoaded {
        request_id: Uuid,
        collection_id: Uuid,
        page_id: KnowledgePageId,
        result: Result<KnowledgePageView, String>,
    },
    SearchFinished {
        request_id: Uuid,
        result: Result<(Vec<SearchHit>, SearchCoverageView, PublicRouteKind), String>,
    },
    SearchPartial {
        request_id: Uuid,
        hits: Vec<SearchHit>,
    },
    PublicBrowseFinished {
        request_id: Uuid,
        result: Result<(Vec<PublicConceptSummary>, Option<String>), String>,
    },
    ChatIntegrationsUpdated {
        request_id: Uuid,
        result: Result<ChatIntegrationsSnapshot, String>,
    },
    Peers(Vec<PeerView>),
    Notice(String),
    Error(String),
}

pub struct WorkerHandle {
    commands: UnboundedSender<WorkerCommand>,
    events: Receiver<WorkerEvent>,
    thread: Option<thread::JoinHandle<()>>,
    finished: Receiver<()>,
}

impl WorkerHandle {
    pub fn spawn(paths: AppPaths) -> Self {
        let (commands_tx, commands_rx) = tokio::sync::mpsc::unbounded_channel();
        let (events_tx, events_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("airwiki-runtime".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("airwiki-worker")
                    .build()
                    .expect("Tokio runtime must start");
                runtime.block_on(run_worker(paths, commands_rx, events_tx));
                drop(runtime);
                let _ = finished_tx.send(());
            })
            .expect("background runtime thread must start");
        Self {
            commands: commands_tx,
            events: events_rx,
            thread: Some(thread),
            finished: finished_rx,
        }
    }

    pub fn send(&self, command: WorkerCommand) {
        if self.commands.send(command).is_err() {
            tracing::error!("background runtime stopped unexpectedly");
        }
    }

    pub fn try_events(&self) -> impl Iterator<Item = WorkerEvent> + '_ {
        self.events.try_iter()
    }
}

const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerJoinOutcome {
    Joined,
    Panicked,
    TimedOut,
}

fn join_worker_with_timeout(
    thread: thread::JoinHandle<()>,
    finished: &Receiver<()>,
    timeout: Duration,
) -> WorkerJoinOutcome {
    match finished.recv_timeout(timeout) {
        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            if thread.join().is_ok() {
                WorkerJoinOutcome::Joined
            } else {
                WorkerJoinOutcome::Panicked
            }
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Dropping a JoinHandle detaches the OS thread. This keeps application exit bounded
            // when an OS integration such as Keychain does not return control to the worker.
            drop(thread);
            WorkerJoinOutcome::TimedOut
        }
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(WorkerCommand::Shutdown);
        let Some(thread) = self.thread.take() else {
            return;
        };
        match join_worker_with_timeout(thread, &self.finished, WORKER_SHUTDOWN_TIMEOUT) {
            WorkerJoinOutcome::Joined => {}
            WorkerJoinOutcome::Panicked => {
                tracing::error!("airwiki background runtime panicked during shutdown");
            }
            WorkerJoinOutcome::TimedOut => {
                tracing::warn!(
                    timeout_seconds = WORKER_SHUTDOWN_TIMEOUT.as_secs(),
                    "airwiki background runtime did not stop before the shutdown deadline"
                );
            }
        }
    }
}

enum InternalEvent {
    VerificationFinished(Result<InstallOutcome, String>),
    ProfileActivationProbed {
        model_id: String,
        result: Result<InstallOutcome, String>,
    },
    InstallFinished(Result<InstallOutcome, String>),
}

enum BackgroundCompletion {
    Approve {
        concept_id: Uuid,
        result: Result<(), String>,
    },
    Preflight {
        collection_id: Uuid,
        result: Result<(), String>,
    },
    Quarantine {
        collection_id: Uuid,
        result: Result<(), String>,
    },
    Scan {
        collection_id: Uuid,
        result: Result<Vec<IngestOutcome>, String>,
    },
    ReanalyzeReview {
        concept_id: Uuid,
        result: Result<(), String>,
    },
    ReviewEvidence {
        request_id: Uuid,
        concept_id: Uuid,
        expected_source_revision: u32,
        result: Result<ReviewEvidencePageView, ReviewEvidenceErrorView>,
    },
    KnowledgeBundle {
        request_id: Uuid,
        collection_id: Uuid,
        result: Result<KnowledgeBundleView, String>,
    },
    KnowledgePage {
        request_id: Uuid,
        collection_id: Uuid,
        page_id: KnowledgePageId,
        result: Result<KnowledgePageView, String>,
    },
    Search {
        request_id: Uuid,
        result: Result<SearchResponse, String>,
        route_kind: PublicRouteKind,
    },
    PublicBrowse {
        request_id: Uuid,
        result: Result<airwiki_types::PublicBrowsePage, String>,
    },
    ChatIntegrations {
        request_id: Uuid,
        action: IntegrationAction,
        result: Result<Vec<IntegrationView>, String>,
    },
    ModelsEnabled {
        model_id: String,
        result: Result<(), String>,
    },
    Autostart {
        request_id: Uuid,
        result: Result<AutostartStatus, String>,
    },
    Updater {
        request_id: Uuid,
        result: Result<UpdaterView, String>,
    },
    WikiMaintenance {
        collection_id: Uuid,
        result: Result<bool, String>,
    },
    RelinkCollection {
        collection_id: Uuid,
        folder: PathBuf,
        result: Result<(), String>,
    },
    ConnectivityDiagnosed {
        request_id: Uuid,
        result: Result<ConnectivityPlatformSnapshot, ConnectivityIssueCode>,
    },
    FirewallConfigured {
        request_id: Uuid,
        result: Result<ConnectivityPlatformSnapshot, FirewallActionError>,
    },
    LanAddressesResolved {
        generation: u64,
        listener: airwiki_network::Multiaddr,
        result: Result<Vec<String>, String>,
    },
    WikiHealth {
        request_id: Uuid,
        generation: u64,
        result: Result<WikiHealthSummaryView, String>,
    },
    GuidedWikiRepairPrepared {
        request_id: Uuid,
        collection_id: Uuid,
        result: Result<GuidedRepairPreview, String>,
    },
    GuidedWikiRepairFinished {
        request_id: Uuid,
        collection_id: Uuid,
        result: Result<GuidedRepairResult, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdaterOperation {
    Check,
    Download,
    Install,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelLifecycle {
    Verifying,
    Missing,
    Installing,
    Enabling,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LanReconcileRequest {
    Apply(LanRuntimePolicy),
    RefreshDiscovery(LanRuntimePolicy),
}

impl LanReconcileRequest {
    const fn policy(self) -> LanRuntimePolicy {
        match self {
            Self::Apply(policy) | Self::RefreshDiscovery(policy) => policy,
        }
    }

    const fn force_restart(self) -> bool {
        matches!(self, Self::RefreshDiscovery(_))
    }
}

#[derive(Debug, Clone, Copy)]
struct FirewallOperationTracker {
    request_id: Uuid,
    started_at: Instant,
    slow_notice_sent: bool,
}

const MAX_CONCURRENT_PREFLIGHTS: usize = 2;
const MAX_CONCURRENT_SCANS: usize = 1;
const MCP_CLIENT_ACTIVITY_RECENT: Duration = Duration::from_secs(5 * 60);
const CONNECTIVITY_RECONCILE_INTERVAL: Duration = Duration::from_secs(5);
const FIREWALL_SLOW_NOTICE_AFTER: Duration = Duration::from_secs(30);

fn slow_notice_is_due(elapsed: Duration, sent: bool) -> bool {
    !sent && elapsed >= FIREWALL_SLOW_NOTICE_AFTER
}

fn firewall_request_is_busy(
    active_connectivity_request: Option<Uuid>,
    firewall_operation: Option<FirewallOperationTracker>,
) -> bool {
    active_connectivity_request.is_some() || firewall_operation.is_some()
}

fn firewall_update_overlap_is_busy(
    firewall_operation: Option<FirewallOperationTracker>,
    active_updater_request: Option<Uuid>,
) -> bool {
    firewall_operation.is_some() || active_updater_request.is_some()
}

fn firewall_completion_is_authoritative(
    firewall_operation: Option<FirewallOperationTracker>,
    request_id: Uuid,
) -> bool {
    firewall_operation.is_some_and(|operation| operation.request_id == request_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ClaudeApprovalState {
    #[default]
    NotRequested,
    Awaiting,
    Confirmed,
}
pub(crate) const PERIODIC_RECONCILE_INTERVAL: Duration = Duration::from_secs(15 * 60);
const PERIODIC_RECONCILE_MAX_JITTER: Duration = Duration::from_secs(30);
static MODEL_STATE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static MODEL_STATE_PLAN_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn model_state_request_is_current(state_sequence: u64, next_sequence: u64) -> bool {
    next_sequence == state_sequence.wrapping_add(1)
}

fn should_schedule_initial_model_state(config: &DesktopConfig) -> bool {
    config.active_selection.is_none() && config.pending_selection.is_none()
}

#[derive(Debug, Default)]
struct WatcherSetup {
    started: Vec<Uuid>,
    failures: Vec<(Uuid, String)>,
}

/// Keeps healthy nodes from reconciling at exactly the same instant while
/// preserving a fixed interval after the first tick. The stable FNV-1a hash is
/// intentionally local and dependency-free; it is not used for security.
fn periodic_reconcile_jitter(node_id: &str, maximum: Duration) -> Duration {
    let maximum_millis = u64::try_from(maximum.as_millis()).unwrap_or(u64::MAX);
    if maximum_millis == 0 {
        return Duration::ZERO;
    }

    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in node_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Duration::from_millis(hash % maximum_millis.saturating_add(1))
}

fn periodic_reconcile_first_delay(node_id: &str) -> Duration {
    PERIODIC_RECONCILE_INTERVAL + periodic_reconcile_jitter(node_id, PERIODIC_RECONCILE_MAX_JITTER)
}

/// Coalesces filesystem bursts without ever running two scans for one
/// collection. A change observed during an active scan schedules exactly one
/// follow-up pass, ensuring the final state includes the newest filesystem view.
#[derive(Debug)]
struct ScanScheduler {
    max_active: usize,
    active: HashSet<Uuid>,
    queued: VecDeque<Uuid>,
    queued_set: HashSet<Uuid>,
    dirty: HashSet<Uuid>,
}

impl ScanScheduler {
    fn new(max_active: usize) -> Self {
        assert!(max_active > 0, "scan concurrency must be positive");
        Self {
            max_active,
            active: HashSet::new(),
            queued: VecDeque::new(),
            queued_set: HashSet::new(),
            dirty: HashSet::new(),
        }
    }

    /// Returns collection IDs that acquired an execution slot.
    fn request(&mut self, collection_id: Uuid) -> Vec<Uuid> {
        if self.active.contains(&collection_id) {
            self.dirty.insert(collection_id);
        } else if self.queued_set.insert(collection_id) {
            self.queued.push_back(collection_id);
        }
        self.start_ready()
    }

    /// Requests work only when no pass is active or queued. Periodic safety
    /// ticks use this so a coincident watcher event remains the sole pass.
    fn request_if_idle(&mut self, collection_id: Uuid) -> Option<Vec<Uuid>> {
        if self.state(collection_id).is_some() {
            None
        } else {
            Some(self.request(collection_id))
        }
    }

    /// Releases a slot and returns the next IDs to start. A dirty active scan is
    /// requeued behind already waiting collections for fairness.
    fn finish(&mut self, collection_id: Uuid) -> Vec<Uuid> {
        self.active.remove(&collection_id);
        if self.dirty.remove(&collection_id) && self.queued_set.insert(collection_id) {
            self.queued.push_back(collection_id);
        }
        self.start_ready()
    }

    /// Removes queued/dirty work for a collection whose watcher is no longer
    /// trustworthy and releases its slot. An already running future may still
    /// complete, but the completion path quarantines it again and cannot
    /// schedule a follow-up.
    fn cancel(&mut self, collection_id: Uuid) -> Vec<Uuid> {
        self.active.remove(&collection_id);
        self.dirty.remove(&collection_id);
        if self.queued_set.remove(&collection_id) {
            self.queued.retain(|queued| *queued != collection_id);
        }
        self.start_ready()
    }

    fn state(&self, collection_id: Uuid) -> Option<CollectionScanState> {
        if self.active.contains(&collection_id) {
            Some(CollectionScanState::Scanning)
        } else if self.queued_set.contains(&collection_id) {
            Some(CollectionScanState::Queued)
        } else {
            None
        }
    }

    fn start_ready(&mut self) -> Vec<Uuid> {
        let mut ready = Vec::new();
        while self.active.len() < self.max_active {
            let Some(collection_id) = self.queued.pop_front() else {
                break;
            };
            self.queued_set.remove(&collection_id);
            self.active.insert(collection_id);
            ready.push(collection_id);
        }
        ready
    }
}

async fn run_worker(
    paths: AppPaths,
    mut commands: UnboundedReceiver<WorkerCommand>,
    events: Sender<WorkerEvent>,
) {
    send(
        &events,
        WorkerEvent::Ready {
            node_id: "—".to_owned(),
            mcp_url: "http://127.0.0.1:43123/mcp".to_owned(),
            collections: Vec::new(),
            reviews: Vec::new(),
            source_issues: Vec::new(),
            blocked_public_publishers: Vec::new(),
        },
    );

    let hardware = match diagnose_hardware(&paths.data) {
        Ok(report) => {
            send(&events, WorkerEvent::Hardware(report.clone()));
            report
        }
        Err(error) => {
            send(
                &events,
                WorkerEvent::Error(format!("Falló el diagnóstico: {error:#}")),
            );
            return;
        }
    };
    let loaded_config = match DesktopConfig::load_or_default(&paths.config) {
        Ok(loaded) => loaded,
        Err(error) => {
            send(
                &events,
                WorkerEvent::Error(format!(
                    "No se pudo cargar la configuración de modelos: {error:#}"
                )),
            );
            return;
        }
    };
    if let Some(warning) = loaded_config.warning {
        send(&events, WorkerEvent::Error(warning));
    }
    let mut desktop_config = loaded_config.config;
    send(
        &events,
        WorkerEvent::DesktopPreferencesUpdated {
            request_id: Uuid::nil(),
            result: Ok(DesktopPreferencesView::from(&desktop_config)),
        },
    );
    let (updater, updater_disabled_reason) =
        match UpdaterBuildConfig::from_compile_time().and_then(PackagerUpdateBackend::new) {
            Ok(backend) => (
                Some(Arc::new(Mutex::new(UpdaterService::new(backend)))),
                None,
            ),
            Err(reason) => (None, Some(reason)),
        };
    if let Some(reason) = updater_disabled_reason {
        send(
            &events,
            WorkerEvent::UpdaterUpdated {
                request_id: Uuid::nil(),
                result: Ok(UpdaterWorkerView::Disabled(reason)),
            },
        );
    } else if let Some(service) = updater.as_ref()
        && let Ok(service) = service.lock()
    {
        send(
            &events,
            WorkerEvent::UpdaterUpdated {
                request_id: Uuid::nil(),
                result: Ok(UpdaterWorkerView::Ready(service.view().clone())),
            },
        );
    }
    let mut updater_schedule = UpdateSchedule::new(
        Instant::now(),
        desktop_config.automatic_update_checks && updater.is_some(),
    );
    let mut recommendation = select_model(desktop_config.profile, &hardware);
    let mut lifecycle = JoinSet::<()>::new();

    let asset_manager = match AssetManager::new(paths.data.clone()) {
        Ok(manager) => manager.with_bundled_runtime(paths.bundled_llama_server()),
        Err(error) => {
            send(
                &events,
                WorkerEvent::Error(format!(
                    "No se pudo iniciar el gestor de modelos: {error:#}"
                )),
            );
            return;
        }
    };
    // A fresh installation has no trusted assets to activate, so its install plan can be
    // prepared immediately while storage, identity and MCP finish starting. Existing installs
    // still wait for verification and reuse that exact result instead of hashing immutable
    // assets twice.
    let initial_model_state_scheduled = should_schedule_initial_model_state(&desktop_config);
    if initial_model_state_scheduled {
        send_model_state(
            &mut lifecycle,
            &events,
            &asset_manager,
            &desktop_config,
            &recommendation,
        );
    }
    // Storage, MCP and local search must be available even when Windows cannot
    // safely expose a LAN listener. The optional runtime is started only after
    // the platform diagnostic has proved its prerequisites.
    let services = match DesktopServices::start(&paths, false).await {
        Ok(services) => Arc::new(services),
        Err(error) => {
            send(
                &events,
                WorkerEvent::Error(format!(
                    "No se pudieron iniciar los servicios privados: {error:#}"
                )),
            );
            return;
        }
    };
    let (integration_manager, integration_manager_error) =
        match ChatIntegrationManager::new(paths.clone()) {
            Ok(manager) => (Some(manager), None),
            Err(error) => (
                None,
                Some(format!(
                    "No se pudo preparar la administración de integraciones: {error:#}"
                )),
            ),
        };
    let mut network_events = services.subscribe_network_events();
    let mut network_open = true;
    let mut lan_runtime_enabled = false;
    let mut lan_listener = if desktop_config.lan_preference == LanPreference::Enabled {
        LanListenerView::Starting
    } else {
        LanListenerView::Stopped
    };
    let mut lan_discovery = if desktop_config.lan_preference == LanPreference::Enabled {
        LanDiscoveryView::Starting
    } else {
        LanDiscoveryView::Disabled
    };
    let mut lan_local_addresses = Vec::new();
    let mut current_lan_listener = None;
    send_lan_runtime(&events, lan_listener, lan_discovery, &lan_local_addresses);
    send_ready(&services, &events);
    refresh_peers(&services, &events);

    let (watch_tx, mut watch_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut watchers = HashMap::new();
    let mut background = JoinSet::<BackgroundCompletion>::new();
    let mut wiki_health_generation = 0_u64;
    spawn_autostart(&mut background, Uuid::nil(), None);
    spawn_connectivity_diagnostic(&mut background, Uuid::nil());
    spawn_next_wiki_health(
        &services,
        &mut background,
        &mut wiki_health_generation,
        Uuid::nil(),
    );
    if let Ok(collections) = services.collection_views() {
        for collection in collections {
            spawn_wiki_maintenance(&services, &mut background, collection.id);
        }
    }
    let mut watcher_quarantined = HashSet::new();
    let mut approving_reviews = HashSet::new();
    let mut reanalyzing_reviews = HashSet::new();
    let mut manual_rescans = HashSet::new();
    let mut preflight_scheduler = ScanScheduler::new(MAX_CONCURRENT_PREFLIGHTS);
    let mut scan_scheduler = ScanScheduler::new(MAX_CONCURRENT_SCANS);
    let mut active_integration_request = None;
    let mut active_updater_request = None;
    let mut active_connectivity_request = Some(Uuid::nil());
    let mut restart_lan_request = None;
    let mut firewall_operation = None;
    let mut active_guided_repair_request = None;
    let mut claude_approval_state = ClaudeApprovalState::NotRequested;
    match ensure_watchers(&services, &mut watchers, &watch_tx) {
        Ok(WatcherSetup { failures, .. }) if !failures.is_empty() => {
            for (collection_id, _) in &failures {
                request_quarantine(
                    &services,
                    &mut background,
                    &mut watcher_quarantined,
                    *collection_id,
                    "no se pudo crear el watcher de la colección",
                );
                tracing::warn!(
                    error_kind = "collection_watcher_start_failed",
                    "collection watcher could not start"
                );
            }
            send(
                &events,
                WorkerEvent::Error(format!(
                    "No se pudo observar una colección: {}",
                    watcher_failure_summary(&failures)
                )),
            );
        }
        Err(error) => send(
            &events,
            WorkerEvent::Error(format!("No se pudieron preparar los watchers: {error:#}")),
        ),
        Ok(_) => {}
    }
    let mut watcher_retry = tokio::time::interval(Duration::from_secs(5));
    watcher_retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut connectivity_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + CONNECTIVITY_RECONCILE_INTERVAL,
        CONNECTIVITY_RECONCILE_INTERVAL,
    );
    connectivity_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut updater_tick = tokio::time::interval(Duration::from_secs(60));
    updater_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let periodic_first_delay = periodic_reconcile_first_delay(services.node_id());
    let periodic_jitter = periodic_first_delay - PERIODIC_RECONCILE_INTERVAL;
    let periodic_start = tokio::time::Instant::now() + periodic_first_delay;
    let mut periodic_reconcile =
        tokio::time::interval_at(periodic_start, PERIODIC_RECONCILE_INTERVAL);
    periodic_reconcile.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::debug!(
        interval_seconds = PERIODIC_RECONCILE_INTERVAL.as_secs(),
        initial_jitter_millis = periodic_jitter.as_millis(),
        "periodic collection reconciliation scheduled"
    );
    let (internal_tx, mut internal_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut install_cancel: Option<CancellationToken> = None;
    let mut model_lifecycle = ModelLifecycle::Verifying;
    let mut queued_install = false;
    let mut enabling_model_id: Option<String> = None;
    // A successful verification or installation is also sufficient evidence
    // for the model-state view. Keep that one-shot result until activation
    // settles instead of hashing the same immutable assets a second time.
    let mut verified_model_plan: Option<InstallPlan> = None;
    let mut attempted_startup_fallback = false;
    let pending_selection = desktop_config
        .pending_selection
        .as_deref()
        .and_then(|id| {
            selection_for_model(
                desktop_config.profile,
                id,
                "Selección pendiente de activación",
            )
        })
        .filter(|selection| {
            selection.manifest.is_hardware_eligible(&hardware)
                && recommendation
                    .selection
                    .as_ref()
                    .is_some_and(|recommended| recommended.model_id == selection.model_id)
        });
    if desktop_config.pending_selection.is_some() && pending_selection.is_none() {
        send(
            &events,
            WorkerEvent::Error(
                "La selección pendiente ya no existe, no es segura o no coincide con el perfil actual; se descartó"
                    .into(),
            ),
        );
        desktop_config.pending_selection = None;
        let _ = persist_config(&desktop_config, &paths, &events);
    }
    let active_selection = desktop_config
        .active_selection
        .as_deref()
        .and_then(|id| selection_for_model(desktop_config.profile, id, "Modelo activo persistido"))
        .filter(|selection| selection.manifest.is_hardware_eligible(&hardware));
    if desktop_config.active_selection.is_some() && active_selection.is_none() {
        send(
            &events,
            WorkerEvent::Error(
                "El modelo activo guardado ya no existe o no es seguro para este hardware; se intentará el fallback compatible"
                    .into(),
            ),
        );
    }
    let mut verifying_selection = pending_selection
        .or(active_selection)
        .unwrap_or_else(ModelSelection::legacy_qwen);
    if verifying_selection.manifest.is_hardware_eligible(&hardware) {
        send(
            &events,
            WorkerEvent::Notice(format!(
                "Verificando la integridad de {}…",
                verifying_selection.manifest.display_name
            )),
        );
        spawn_verification(
            &mut lifecycle,
            asset_manager.clone(),
            verifying_selection.clone(),
            internal_tx.clone(),
        );
    } else {
        model_lifecycle = ModelLifecycle::Missing;
        send(
            &events,
            WorkerEvent::Error(
                "No hay un modelo instalado que sea seguro para este hardware".into(),
            ),
        );
        send(&events, WorkerEvent::ModelsMissing);
        if !initial_model_state_scheduled {
            send_model_state(
                &mut lifecycle,
                &events,
                &asset_manager,
                &desktop_config,
                &recommendation,
            );
        }
    }

    'running: loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break 'running };
                match command {
                    WorkerCommand::InstallModels => {
                        let Some(selection) = recommendation.selection.clone() else {
                            send(
                                &events,
                                WorkerEvent::Error(format!(
                                    "No hay un modelo elegible: {}",
                                    recommendation.issues.join("; ")
                                )),
                            );
                            continue;
                        };
                        match model_lifecycle {
                            ModelLifecycle::Verifying => {
                                queued_install = true;
                                send(
                                    &events,
                                    WorkerEvent::InstallQueued(
                                        "Esperando que termine la verificación actual…".into(),
                                    ),
                                );
                                send(
                                    &events,
                                    WorkerEvent::Notice(
                                        "La instalación comenzará al terminar la verificación actual"
                                            .into(),
                                    ),
                                );
                            }
                            ModelLifecycle::Installing => send(
                                &events,
                                WorkerEvent::Notice("Ya hay una instalación en curso".into()),
                            ),
                            ModelLifecycle::Enabling => {
                                queued_install = true;
                                send(
                                    &events,
                                    WorkerEvent::InstallQueued(
                                        "Esperando que termine el smoke test actual…".into(),
                                    ),
                                );
                                send(
                                    &events,
                                    WorkerEvent::Notice(
                                        "La instalación comenzará al terminar el smoke test actual"
                                            .into(),
                                    ),
                                );
                            }
                            ModelLifecycle::Missing | ModelLifecycle::Ready => {
                                if desktop_config.active_selection.as_deref()
                                    == Some(selection.model_id)
                                    && model_lifecycle == ModelLifecycle::Ready
                                {
                                    send(
                                        &events,
                                        WorkerEvent::Notice(format!(
                                            "{} ya está activo y verificado",
                                            selection.manifest.display_name
                                        )),
                                    );
                                    continue;
                                }
                                let previous_config = desktop_config.clone();
                                accept_selection_licenses(&mut desktop_config, &selection);
                                if !persist_config(&desktop_config, &paths, &events) {
                                    desktop_config = previous_config;
                                    send_model_state(
                                        &mut lifecycle,
                                        &events,
                                        &asset_manager,
                                        &desktop_config,
                                        &recommendation,
                                    );
                                    continue;
                                }
                                start_install(
                                    &mut lifecycle,
                                    &asset_manager,
                                    selection,
                                    &events,
                                    &internal_tx,
                                    &mut install_cancel,
                                );
                                model_lifecycle = ModelLifecycle::Installing;
                            }
                        }
                    }
                    WorkerCommand::CancelInstall => {
                        match model_lifecycle {
                            ModelLifecycle::Verifying | ModelLifecycle::Enabling
                                if queued_install =>
                            {
                                queued_install = false;
                                send(&events, WorkerEvent::InstallStopped);
                                send(
                                    &events,
                                    WorkerEvent::Notice(
                                        "Instalación pendiente cancelada; la operación actual continúa"
                                            .into(),
                                    ),
                                );
                            }
                            ModelLifecycle::Installing => {
                                if let Some(cancel) = install_cancel.as_ref() {
                                    cancel.cancel();
                                }
                            }
                            _ => send(
                                &events,
                                WorkerEvent::Notice("No hay una instalación cancelable".into()),
                            ),
                        }
                    }
                    WorkerCommand::SetModelProfile(profile) => {
                        if !can_change_model_profile(model_lifecycle) {
                            send(
                                &events,
                                WorkerEvent::Notice(
                                    "Espera a que termine la verificación u operación de modelos para cambiar el perfil"
                                        .into(),
                                ),
                            );
                            send_model_state(
                                &mut lifecycle,
                                &events,
                                &asset_manager,
                                &desktop_config,
                                &recommendation,
                            );
                            continue;
                        }
                        let updated_recommendation = select_model(profile, &hardware);
                        let (updated_config, cleared_pending) = config_with_profile(
                            &desktop_config,
                            profile,
                            &updated_recommendation,
                        );
                        if !persist_config(&updated_config, &paths, &events) {
                            send_model_state(
                                &mut lifecycle,
                                &events,
                                &asset_manager,
                                &desktop_config,
                                &recommendation,
                            );
                            continue;
                        }
                        desktop_config = updated_config;
                        recommendation = updated_recommendation;
                        if cleared_pending {
                            send(
                                &events,
                                WorkerEvent::Notice(
                                    "Se canceló la activación pendiente porque no coincide con el perfil; el modelo descargado se conserva"
                                        .into(),
                                ),
                            );
                        }
                        send_model_state(
                            &mut lifecycle,
                            &events,
                            &asset_manager,
                            &desktop_config,
                            &recommendation,
                        );
                        if should_probe_profile_activation(&desktop_config, &recommendation)
                            && let Some(selection) = recommendation.selection.clone()
                        {
                            spawn_profile_activation_probe(
                                &mut lifecycle,
                                asset_manager.clone(),
                                selection,
                                internal_tx.clone(),
                            );
                        }
                    }
                    WorkerCommand::UpdateDesktopPreferences { request_id, update } => {
                        let previous = desktop_config.clone();
                        desktop_config.locale = update.locale;
                        desktop_config.lan_preference = update.lan_preference;
                        desktop_config.close_behavior = update.close_behavior;
                        desktop_config.automatic_update_checks = update.automatic_update_checks;
                        if update.complete_onboarding {
                            desktop_config.completed_onboarding_version = Some(ONBOARDING_VERSION);
                        }
                        let lan_changed = previous.lan_preference != desktop_config.lan_preference;
                        let result = if persist_config(&desktop_config, &paths, &events) {
                                updater_schedule.set_enabled(
                                    Instant::now(),
                                    desktop_config.automatic_update_checks && updater.is_some(),
                                );
                                if lan_changed {
                                    if desktop_config.lan_preference != LanPreference::Enabled {
                                        let policy = LanRuntimePolicy::DisabledByPreference;
                                        if let Err(message) = reconcile_lan_runtime(
                                            &services,
                                            &events,
                                            LanReconcileRequest::Apply(policy),
                                            &mut lan_runtime_enabled,
                                            &mut lan_listener,
                                            &mut lan_discovery,
                                            &mut lan_local_addresses,
                                        )
                                        .await
                                        {
                                            send(&events, WorkerEvent::Error(message));
                                        }
                                    } else {
                                        lan_listener = LanListenerView::Starting;
                                        lan_discovery = LanDiscoveryView::Starting;
                                        lan_local_addresses.clear();
                                        send_lan_runtime(
                                            &events,
                                            lan_listener,
                                            lan_discovery,
                                            &lan_local_addresses,
                                        );
                                        if active_connectivity_request.is_none() {
                                            let diagnostic_id = Uuid::new_v4();
                                            active_connectivity_request = Some(diagnostic_id);
                                            spawn_connectivity_diagnostic(
                                                &mut background,
                                                diagnostic_id,
                                            );
                                        }
                                    }
                                }
                                refresh_peers(&services, &events);
                                Ok(DesktopPreferencesView::from(&desktop_config))
                        } else {
                            desktop_config = previous;
                            Err("No se pudieron guardar las preferencias".to_owned())
                        };
                        send(
                            &events,
                            WorkerEvent::DesktopPreferencesUpdated { request_id, result },
                        );
                    }
                    WorkerCommand::SetAutostart { request_id, enabled } => {
                        spawn_autostart(&mut background, request_id, Some(enabled));
                    }
                    WorkerCommand::RefreshAutostart { request_id } => {
                        spawn_autostart(&mut background, request_id, None);
                    }
                    WorkerCommand::CheckUpdates { request_id } => {
                        queue_updater_operation(
                            &mut background,
                            &events,
                            updater.as_ref(),
                            updater_disabled_reason,
                            &mut active_updater_request,
                            request_id,
                            UpdaterOperation::Check,
                        );
                    }
                    WorkerCommand::DownloadUpdate { request_id } => {
                        queue_updater_operation(
                            &mut background,
                            &events,
                            updater.as_ref(),
                            updater_disabled_reason,
                            &mut active_updater_request,
                            request_id,
                            UpdaterOperation::Download,
                        );
                    }
                    WorkerCommand::InstallUpdate { request_id } => {
                        if firewall_update_overlap_is_busy(firewall_operation, None) {
                            send(
                                &events,
                                WorkerEvent::UpdaterUpdated {
                                    request_id,
                                    result: Err(
                                        "La configuración del firewall todavía está en curso"
                                            .to_owned(),
                                    ),
                                },
                            );
                        } else {
                            queue_updater_operation(
                                &mut background,
                                &events,
                                updater.as_ref(),
                                updater_disabled_reason,
                                &mut active_updater_request,
                                request_id,
                                UpdaterOperation::Install,
                            );
                        }
                    }
                    WorkerCommand::RefreshConnectivity { request_id } => {
                        if firewall_request_is_busy(
                            active_connectivity_request,
                            firewall_operation,
                        ) {
                            send(
                                &events,
                                WorkerEvent::ConnectivityPlatformUpdated {
                                    request_id,
                                    result: Err(ConnectivityIssueCode::Busy),
                                },
                            );
                        } else {
                            active_connectivity_request = Some(request_id);
                            restart_lan_request = Some(request_id);
                            spawn_connectivity_diagnostic(&mut background, request_id);
                        }
                    }
                    WorkerCommand::ConfigureFirewall { request_id, install } => {
                        if firewall_request_is_busy(
                            active_connectivity_request,
                            firewall_operation,
                        ) || firewall_update_overlap_is_busy(None, active_updater_request)
                        {
                            send(
                                &events,
                                WorkerEvent::ConnectivityPlatformUpdated {
                                    request_id,
                                    result: Err(ConnectivityIssueCode::Busy),
                                },
                            );
                        } else if install
                            && desktop_config.lan_preference != LanPreference::Enabled
                        {
                            send(
                                &events,
                                WorkerEvent::ConnectivityPlatformUpdated {
                                    request_id,
                                    result: Err(ConnectivityIssueCode::FirewallStateChanged),
                                },
                            );
                        } else {
                            active_connectivity_request = Some(request_id);
                            firewall_operation = Some(FirewallOperationTracker {
                                request_id,
                                started_at: Instant::now(),
                                slow_notice_sent: false,
                            });
                            send(
                                &events,
                                WorkerEvent::FirewallOperationUpdated {
                                    request_id,
                                    state: Some(FirewallOperationView::AwaitingWindows),
                                },
                            );
                            spawn_firewall_configuration(&mut background, request_id, install);
                        }
                    }
                    WorkerCommand::OpenAdvancedFirewall { request_id } => {
                        if firewall_request_is_busy(
                            active_connectivity_request,
                            firewall_operation,
                        ) {
                            send(
                                &events,
                                WorkerEvent::ConnectivityPlatformUpdated {
                                    request_id,
                                    result: Err(ConnectivityIssueCode::Busy),
                                },
                            );
                        } else {
                            active_connectivity_request = Some(request_id);
                            spawn_advanced_firewall_rules(&mut background, request_id);
                        }
                    }
                    WorkerCommand::RefreshWikiHealth { request_id } => {
                        spawn_next_wiki_health(
                            &services,
                            &mut background,
                            &mut wiki_health_generation,
                            request_id,
                        );
                    }
                    WorkerCommand::PrepareGuidedWikiRepair {
                        request_id,
                        collection_id,
                    } => {
                        if active_guided_repair_request.is_some() {
                            send(
                                &events,
                                WorkerEvent::GuidedWikiRepairPrepared {
                                    request_id,
                                    collection_id,
                                    result: Err("wiki_repair_operation_in_progress".to_owned()),
                                },
                            );
                        } else {
                            active_guided_repair_request = Some(request_id);
                            spawn_guided_repair_preview(
                                &services,
                                &mut background,
                                request_id,
                                collection_id,
                            );
                        }
                    }
                    WorkerCommand::ExecuteGuidedWikiRepair {
                        request_id,
                        preview,
                    } => {
                        let collection_id = preview.collection_id;
                        if active_guided_repair_request.is_some() {
                            send(
                                &events,
                                WorkerEvent::GuidedWikiRepairFinished {
                                    request_id,
                                    collection_id,
                                    result: Err("wiki_repair_operation_in_progress".to_owned()),
                                },
                            );
                        } else {
                            active_guided_repair_request = Some(request_id);
                            spawn_guided_repair_execution(
                                &services,
                                &mut background,
                                request_id,
                                preview,
                            );
                        }
                    }
                    WorkerCommand::AddCollection { name, folder } => {
                        match services.add_collection(name, &folder) {
                            Ok(collection) => {
                                match CollectionWatcherHandle::spawn(collection.id, folder, watch_tx.clone()) {
                                    Ok(watcher) => {
                                        watchers.insert(collection.id, watcher);
                                        if services.models_ready() {
                                            request_scan(
                                                &services,
                                                &mut scan_scheduler,
                                                &mut background,
                                                &events,
                                                collection.id,
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        request_quarantine(
                                            &services,
                                            &mut background,
                                            &mut watcher_quarantined,
                                            collection.id,
                                            "no se pudo crear el watcher de la colección nueva",
                                        );
                                        send(&events, WorkerEvent::Error(format!("No se pudo observar la carpeta: {error:#}")));
                                    }
                                }
                                refresh_content_views(&services, &events);
                            }
                            Err(error) => send(&events, WorkerEvent::Error(format!("No se pudo crear la colección: {error:#}"))),
                        }
                    }
                    WorkerCommand::RelinkCollection { collection_id, folder } => {
                        watchers.remove(&collection_id);
                        for ready in preflight_scheduler.cancel(collection_id) {
                            spawn_preflight(&services, &mut background, ready);
                        }
                        let ready = scan_scheduler.cancel(collection_id);
                        publish_scan_state(&events, &scan_scheduler, collection_id);
                        spawn_ready_scans(&services, &mut background, &events, ready);
                        let services = Arc::clone(&services);
                        background.spawn_blocking(move || {
                            let result = services
                                .quarantine_collection(
                                    collection_id,
                                    "la carpeta de la colección se está volviendo a vincular",
                                )
                                .and_then(|()| {
                                    services.relink_collection(collection_id, &folder)
                                })
                                .map_err(|error| format!("{error:#}"));
                            BackgroundCompletion::RelinkCollection {
                                collection_id,
                                folder,
                                result,
                            }
                        });
                    }
                    WorkerCommand::RescanCollection(collection_id) => {
                        if !services.models_ready() {
                            send(&events, WorkerEvent::Error("Instala los modelos antes de escanear".into()));
                            send(
                                &events,
                                WorkerEvent::CollectionScan {
                                    collection_id,
                                    state: None,
                                },
                            );
                        } else if !watchers.contains_key(&collection_id) {
                            send(&events, WorkerEvent::Error(
                                "La colección no tiene un watcher activo; corrige la carpeta antes de escanear"
                                    .into(),
                            ));
                            send(
                                &events,
                                WorkerEvent::CollectionScan {
                                    collection_id,
                                    state: None,
                                },
                            );
                        } else {
                            manual_rescans.insert(collection_id);
                            request_scan(
                                &services,
                                &mut scan_scheduler,
                                &mut background,
                                &events,
                                collection_id,
                            );
                        }
                    }
                    WorkerCommand::UpdateCollectionPolicy { collection_id, local_only, peer_shareable, allow_external_ai, internet_public } => {
                        let policy = CollectionPolicy { local_only, peer_shareable, allow_external_ai, internet_public };
                        if let Err(error) = services.update_collection_policy(collection_id, policy) {
                            send(&events, WorkerEvent::Error(format!("No se pudo actualizar la política: {error:#}")));
                        } else if let Err(error) = services.reconcile_public_network().await {
                            send(&events, WorkerEvent::Error(format!("No se pudo actualizar la red pública: {error:#}")));
                        } else if let Err(error) = services.sync_public_collection(collection_id).await {
                            send(&events, WorkerEvent::Error(format!("No se pudo sincronizar el anuncio público: {error:#}")));
                        }
                        refresh_content_views(&services, &events);
                    }
                    WorkerCommand::Approve {
                        concept_id,
                        expected_review_version,
                        draft,
                    } => {
                        if !approving_reviews.insert(concept_id) {
                            send(
                                &events,
                                WorkerEvent::Notice(
                                    "Ese documento ya se está publicando".into(),
                                ),
                            );
                        } else {
                            spawn_review_approval(
                                &services,
                                &mut background,
                                concept_id,
                                expected_review_version,
                                draft,
                            );
                        }
                    }
                    WorkerCommand::Reject { concept_id } => {
                        if let Err(error) = services.reject_review(concept_id) {
                            send(&events, WorkerEvent::Error(format!("No se pudo rechazar el borrador: {error:#}")));
                        } else {
                            send(&events, WorkerEvent::Notice("Borrador rechazado; permanece fuera de publicación".into()));
                        }
                        refresh_content_views(&services, &events);
                    }
                    WorkerCommand::ReanalyzeReview { concept_id } => {
                        if model_lifecycle != ModelLifecycle::Ready || !services.models_ready() {
                            send(
                                &events,
                                WorkerEvent::Error(
                                    "El modelo local todavía no está listo para volver a analizar"
                                        .into(),
                                ),
                            );
                        } else if !reanalyzing_reviews.insert(concept_id) {
                            send(
                                &events,
                                WorkerEvent::Notice(
                                    "Ese documento ya se está volviendo a analizar".into(),
                                ),
                            );
                        } else {
                            send(
                                &events,
                                WorkerEvent::ReviewReanalysis {
                                    concept_id,
                                    running: true,
                                },
                            );
                            spawn_review_reanalysis(&services, &mut background, concept_id);
                        }
                    }
                    WorkerCommand::LoadReviewEvidence {
                        request_id,
                        concept_id,
                        expected_source_revision,
                        expected_review_version,
                        after_ordinal,
                    } => {
                        spawn_review_evidence(
                            &services,
                            &mut background,
                            request_id,
                            concept_id,
                            expected_source_revision,
                            expected_review_version,
                            after_ordinal,
                        );
                    }
                    WorkerCommand::LoadKnowledgeBundle { request_id, collection_id } => {
                        spawn_knowledge_bundle(
                            &services,
                            &mut background,
                            request_id,
                            collection_id,
                        );
                    }
                    WorkerCommand::LoadKnowledgePage {
                        request_id,
                        collection_id,
                        page_id,
                        expected_fingerprint,
                    } => {
                        spawn_knowledge_page(
                            &services,
                            &mut background,
                            request_id,
                            collection_id,
                            page_id,
                            expected_fingerprint,
                        );
                    }
                    WorkerCommand::Search {
                        request_id,
                        question,
                        top_k,
                        purpose,
                        public_network,
                    } => {
                        spawn_search(
                            &services,
                            &mut background,
                            &events,
                            SearchTask {
                                request_id,
                                question,
                                top_k,
                                purpose,
                                public_network,
                            },
                        );
                    }
                    WorkerCommand::AddFederationIndex { peer_id, address } => {
                        match services.add_federation_index(&peer_id, &address) {
                            Ok(()) => match services.restart_public_network().await {
                                Ok(()) => match services.sync_all_public_collections().await {
                                    Ok(()) => send(&events, WorkerEvent::Notice("Índice comunitario agregado".into())),
                                    Err(error) => send(&events, WorkerEvent::Error(format!("El índice se agregó, pero no se pudieron sincronizar los anuncios: {error:#}"))),
                                },
                                Err(error) => send(&events, WorkerEvent::Error(format!("El índice se agregó, pero no se pudo reiniciar la red pública: {error:#}"))),
                            },
                            Err(error) => send(&events, WorkerEvent::Error(format!("No se pudo agregar el índice: {error:#}"))),
                        }
                    }
                    WorkerCommand::RemoveFederationIndex { peer_id } => {
                        match services.remove_federation_index(&peer_id) {
                            Ok(()) => match services.restart_public_network().await {
                                Ok(()) => send(&events, WorkerEvent::Notice("Índice comunitario desactivado".into())),
                                Err(error) => send(&events, WorkerEvent::Error(format!("El índice se desactivó, pero no se pudo reiniciar la red pública: {error:#}"))),
                            },
                            Err(error) => send(&events, WorkerEvent::Error(format!("No se pudo desactivar el índice: {error:#}"))),
                        }
                    }
                    WorkerCommand::UpdatePublicCollectionProfile { collection_id, description, languages } => {
                        match services.update_public_collection_profile(collection_id, &description, &languages) {
                            Ok(()) => match services.sync_public_collection(collection_id).await {
                                Ok(()) => send(&events, WorkerEvent::Notice("Perfil público actualizado".into())),
                                Err(error) => send(&events, WorkerEvent::Error(format!("El perfil se guardó, pero no se pudo anunciar: {error:#}"))),
                            },
                            Err(error) => send(&events, WorkerEvent::Error(format!("No se pudo actualizar el perfil público: {error:#}"))),
                        }
                        refresh_content_views(&services, &events);
                    }
                    WorkerCommand::BrowsePublicCollection { request_id, publisher_id, collection_id, cursor } => {
                        let services = Arc::clone(&services);
                        background.spawn(async move {
                            let result = services
                                .browse_public_collection(&publisher_id, collection_id, cursor)
                                .await
                                .map_err(|error| error.to_string());
                            BackgroundCompletion::PublicBrowse { request_id, result }
                        });
                    }
                    WorkerCommand::SetPublicPublisherBlocked { publisher_id, blocked } => {
                        match services.set_public_publisher_blocked(&publisher_id, blocked).await {
                            Ok(()) => {
                                send(
                                    &events,
                                    WorkerEvent::Notice(if blocked {
                                        "Publicador público bloqueado en este dispositivo".into()
                                    } else {
                                        "Publicador público desbloqueado".into()
                                    }),
                                );
                                send_ready(&services, &events);
                            }
                            Err(error) => send(
                                &events,
                                WorkerEvent::Error(format!(
                                    "No se pudo actualizar el bloqueo del publicador: {error:#}"
                                )),
                            ),
                        }
                    }
                    WorkerCommand::ManageChatIntegration { request_id, action } => {
                        if active_integration_request.is_some() {
                            send(
                                &events,
                                WorkerEvent::ChatIntegrationsUpdated {
                                    request_id,
                                    result: Err(
                                        "Ya hay una operación de integración en curso".into(),
                                    ),
                                },
                            );
                        } else if let Some(error) = integration_manager_error.as_ref() {
                            send(
                                &events,
                                WorkerEvent::ChatIntegrationsUpdated {
                                    request_id,
                                    result: Err(error.clone()),
                                },
                            );
                        } else if let Some(manager) = integration_manager.clone() {
                            active_integration_request = Some(request_id);
                            background.spawn(async move {
                                let result = AssertUnwindSafe(manager.execute(action))
                                    .catch_unwind()
                                    .await
                                    .map_err(|_| {
                                        "La operación de integración terminó inesperadamente"
                                            .to_owned()
                                    })
                                    .and_then(|result| result.map_err(|error| format!("{error:#}")));
                                BackgroundCompletion::ChatIntegrations {
                                    request_id,
                                    action,
                                    result,
                                }
                            });
                        }
                    }
                    WorkerCommand::Pair { peer_id } => {
                        if let Err(error) = services.begin_pairing(&peer_id).await {
                            send(&events, WorkerEvent::Error(format!("No se pudo iniciar el emparejamiento: {error:#}")));
                        }
                    }
                    WorkerCommand::Dial { address } => {
                        if let Err(error) = services.dial(&address).await {
                            send(&events, WorkerEvent::Error(format!("No se pudo conectar: {error:#}")));
                        }
                    }
                    WorkerCommand::ConfirmPairing { peer_id, accepted } => {
                        if let Err(error) = services.confirm_pairing(&peer_id, accepted).await {
                            send(&events, WorkerEvent::Error(format!("No se pudo confirmar el emparejamiento: {error:#}")));
                        }
                    }
                    WorkerCommand::RevokePeer { peer_id } => {
                        if let Err(error) = services.revoke_peer(&peer_id).await {
                            send(&events, WorkerEvent::Error(format!("No se pudo revocar el peer: {error:#}")));
                        }
                        refresh_peers(&services, &events);
                    }
                    WorkerCommand::GrantCollection { peer_id, collection_id, granted } => {
                        if let Err(error) = services.set_collection_grant(&peer_id, collection_id, granted).await {
                            send(&events, WorkerEvent::Error(format!("No se pudo cambiar el grant: {error:#}")));
                        }
                        refresh_peers(&services, &events);
                    }
                    WorkerCommand::Shutdown => break 'running,
                }
            }
            internal = internal_rx.recv() => {
                match internal {
                    Some(InternalEvent::VerificationFinished(result))
                        if model_lifecycle == ModelLifecycle::Verifying =>
                    {
                        match result {
                            Ok(outcome) => {
                                verified_model_plan = Some(outcome.verified_install_plan());
                                enabling_model_id = Some(outcome.selection.model_id.to_owned());
                                model_lifecycle = ModelLifecycle::Enabling;
                                spawn_model_enable(
                                    &services,
                                    &mut background,
                                    ModelRuntimePaths::from_install(&outcome),
                                );
                            }
                            Err(_error) if queued_install => {
                                tracing::info!("installed model verification requested repair");
                                queued_install = false;
                                if let Some(selection) = recommendation.selection.clone() {
                                    let previous_config = desktop_config.clone();
                                    accept_selection_licenses(&mut desktop_config, &selection);
                                    if !persist_config(&desktop_config, &paths, &events) {
                                        desktop_config = previous_config;
                                        model_lifecycle = ModelLifecycle::Missing;
                                        send(&events, WorkerEvent::InstallStopped);
                                        send(&events, WorkerEvent::ModelsMissing);
                                        send_model_state(
                                            &mut lifecycle,
                                            &events,
                                            &asset_manager,
                                            &desktop_config,
                                            &recommendation,
                                        );
                                        continue;
                                    }
                                    start_install(
                                        &mut lifecycle,
                                        &asset_manager,
                                        selection,
                                        &events,
                                        &internal_tx,
                                        &mut install_cancel,
                                    );
                                    model_lifecycle = ModelLifecycle::Installing;
                                } else {
                                    model_lifecycle = ModelLifecycle::Missing;
                                    send(&events, WorkerEvent::ModelsMissing);
                                }
                            }
                            Err(error)
                                if !attempted_startup_fallback
                                    && desktop_config.pending_selection.as_deref()
                                        == Some(verifying_selection.model_id)
                                    && desktop_config.active_selection.as_deref()
                                        != Some(verifying_selection.model_id) =>
                            {
                                attempted_startup_fallback = true;
                                let failed = verifying_selection.model_id;
                                desktop_config.pending_selection = None;
                                let _ = persist_config(&desktop_config, &paths, &events);
                                send(
                                    &events,
                                    WorkerEvent::Error(format!(
                                        "El modelo pendiente {failed} falló la verificación; se conserva el anterior: {error}"
                                    )),
                                );
                                if let Some(active) = desktop_config.active_selection.as_deref()
                                    .and_then(|id| selection_for_model(
                                        desktop_config.profile,
                                        id,
                                        "Fallback activo tras fallar la selección pendiente",
                                    ))
                                {
                                    verifying_selection = active;
                                    spawn_verification(
                                        &mut lifecycle,
                                        asset_manager.clone(),
                                        verifying_selection.clone(),
                                        internal_tx.clone(),
                                    );
                                } else {
                                    model_lifecycle = ModelLifecycle::Missing;
                                    send(&events, WorkerEvent::ModelsMissing);
                                }
                                send_model_state(
                                    &mut lifecycle,
                                    &events,
                                    &asset_manager,
                                    &desktop_config,
                                    &recommendation,
                                );
                            }
                            Err(_) => {
                                tracing::info!(
                                    error_kind = "installed_model_invalid",
                                    "installed model is missing or invalid"
                                );
                                model_lifecycle = ModelLifecycle::Missing;
                                send(
                                    &events,
                                    WorkerEvent::Notice(
                                        format!(
                                            "{} no está listo; instala la recomendación para este equipo",
                                            verifying_selection.manifest.display_name
                                        ),
                                    ),
                                );
                                send(&events, WorkerEvent::ModelsMissing);
                                if !initial_model_state_scheduled {
                                    send_model_state(
                                        &mut lifecycle,
                                        &events,
                                        &asset_manager,
                                        &desktop_config,
                                        &recommendation,
                                    );
                                }
                            }
                        }
                    }
                    Some(InternalEvent::InstallFinished(result))
                        if model_lifecycle == ModelLifecycle::Installing =>
                    {
                        let was_cancelled = install_cancel
                            .as_ref()
                            .is_some_and(CancellationToken::is_cancelled);
                        install_cancel = None;
                        send(&events, WorkerEvent::InstallStopped);
                        match result {
                            Ok(outcome) => {
                                let verified_plan = outcome.verified_install_plan();
                                let has_different_active = should_stage_for_restart(
                                    services.models_ready(),
                                    desktop_config.active_selection.as_deref(),
                                    outcome.selection.model_id,
                                );
                                if has_different_active {
                                    let previous_pending =
                                        desktop_config.pending_selection.clone();
                                    desktop_config.pending_selection =
                                        Some(outcome.selection.model_id.to_owned());
                                    model_lifecycle = ModelLifecycle::Ready;
                                    if persist_config(&desktop_config, &paths, &events) {
                                        send(
                                            &events,
                                            WorkerEvent::RestartRequired(format!(
                                                "{} quedó verificado y se activará al reiniciar",
                                                outcome.selection.manifest.display_name
                                            )),
                                        );
                                    } else {
                                        desktop_config.pending_selection = previous_pending;
                                        send(
                                            &events,
                                            WorkerEvent::Notice(format!(
                                                "{} quedó descargado y verificado, pero no se pudo programar su activación",
                                                outcome.selection.manifest.display_name
                                            )),
                                        );
                                    }
                                    send_model_state_with_known_plan(
                                        &mut lifecycle,
                                        &events,
                                        &asset_manager,
                                        &desktop_config,
                                        &recommendation,
                                        Some(verified_plan),
                                    );
                                } else {
                                    verified_model_plan = Some(verified_plan);
                                    enabling_model_id =
                                        Some(outcome.selection.model_id.to_owned());
                                    model_lifecycle = ModelLifecycle::Enabling;
                                    spawn_model_enable(
                                        &services,
                                        &mut background,
                                        ModelRuntimePaths::from_install(&outcome),
                                    );
                                }
                            }
                            Err(_) if was_cancelled => {
                                model_lifecycle = settled_model_lifecycle(&services);
                                send(
                                    &events,
                                    WorkerEvent::Notice(
                                        "Instalación cancelada; se conservará la descarga parcial para reanudar"
                                            .into(),
                                    ),
                                );
                                if model_lifecycle == ModelLifecycle::Missing {
                                    send(&events, WorkerEvent::ModelsMissing);
                                }
                                send_model_state(
                                    &mut lifecycle,
                                    &events,
                                    &asset_manager,
                                    &desktop_config,
                                    &recommendation,
                                );
                            }
                            Err(error) => {
                                model_lifecycle = settled_model_lifecycle(&services);
                                let message = match error.as_str() {
                                    "model_install_network_unavailable" => {
                                        "La conexión sigue sin estar disponible. La descarga parcial quedó guardada para reintentar."
                                    }
                                    "model_install_action_required" => {
                                        "No se pudo preparar la IA local. Revisa el espacio, la memoria y la compatibilidad antes de reintentar."
                                    }
                                    _ => {
                                        "No se pudo completar la preparación de la IA local. Puedes reintentar sin descargar de nuevo los archivos verificados."
                                    }
                                };
                                send(
                                    &events,
                                    WorkerEvent::Error(message.to_owned()),
                                );
                                if model_lifecycle == ModelLifecycle::Missing {
                                    send(&events, WorkerEvent::ModelsMissing);
                                }
                                send_model_state(
                                    &mut lifecycle,
                                    &events,
                                    &asset_manager,
                                    &desktop_config,
                                    &recommendation,
                                );
                            }
                        }
                    }
                    Some(InternalEvent::ProfileActivationProbed { model_id, result }) => {
                        let recommendation_still_matches = recommendation
                            .selection
                            .as_ref()
                            .is_some_and(|selection| selection.model_id == model_id);
                        let can_stage = model_lifecycle == ModelLifecycle::Ready
                            && services.models_ready()
                            && desktop_config.pending_selection.is_none()
                            && desktop_config.active_selection.as_deref()
                                != Some(model_id.as_str())
                            && recommendation_still_matches;
                        match result {
                            Ok(outcome) if can_stage => {
                                let previous_pending = desktop_config.pending_selection.clone();
                                desktop_config.pending_selection = Some(model_id.clone());
                                if persist_config(&desktop_config, &paths, &events) {
                                    send(
                                        &events,
                                        WorkerEvent::RestartRequired(format!(
                                            "{} ya estaba descargado y verificado; se activará al reiniciar",
                                            outcome.selection.manifest.display_name
                                        )),
                                    );
                                    send_model_state_with_known_plan(
                                        &mut lifecycle,
                                        &events,
                                        &asset_manager,
                                        &desktop_config,
                                        &recommendation,
                                        Some(outcome.verified_install_plan()),
                                    );
                                } else {
                                    desktop_config.pending_selection = previous_pending;
                                }
                            }
                            Ok(_) => tracing::debug!(
                                %model_id,
                                "ignored stale profile activation probe"
                            ),
                            Err(_) => tracing::debug!(
                                error_kind = "recommended_model_incomplete",
                                "recommended model is not fully installed yet"
                            ),
                        }
                    }
                    Some(stale) => tracing::warn!(
                        ?model_lifecycle,
                        event = internal_event_name(&stale),
                        "ignored stale model lifecycle completion"
                    ),
                    None => {}
                }
            }
            watch = watch_rx.recv() => {
                match watch {
                    Some(CollectionWatchEvent::Changed { collection_id, paths })
                        if services.models_ready()
                            && watchers.contains_key(&collection_id)
                            && services
                                .startup_preflight_blocks_automatic_scan(collection_id)
                                .is_ok_and(|blocked| !blocked) =>
                    {
                        let _changed_path_count = paths.len();
                        let scan_started = request_scan(
                            &services,
                            &mut scan_scheduler,
                            &mut background,
                            &events,
                            collection_id,
                        );
                        // A newly started scan begins with the same preflight.
                        // If the collection is already scanning or queued behind
                        // another one, run a separate inference-free preflight so
                        // stale evidence is withdrawn immediately.
                        if !scan_started {
                            request_preflight(
                                &services,
                                &mut preflight_scheduler,
                                &mut background,
                                collection_id,
                            );
                        }
                    }
                    Some(CollectionWatchEvent::Changed { .. }) => {}
                    Some(CollectionWatchEvent::Failed { collection_id, error }) => {
                        watchers.remove(&collection_id);
                        clear_manual_rescan(&mut manual_rescans, collection_id);
                        for ready in preflight_scheduler.cancel(collection_id) {
                            spawn_preflight(&services, &mut background, ready);
                        }
                        let ready = scan_scheduler.cancel(collection_id);
                        publish_scan_state(&events, &scan_scheduler, collection_id);
                        spawn_ready_scans(
                            &services,
                            &mut background,
                            &events,
                            ready,
                        );
                        request_quarantine(
                            &services,
                            &mut background,
                            &mut watcher_quarantined,
                            collection_id,
                            "el watcher de la colección dejó de estar disponible",
                        );
                        send(&events, WorkerEvent::Error(format!("El watcher de {collection_id} se detuvo: {error}")));
                    }
                    None => {}
                }
            }
            _ = watcher_retry.tick() => {
                match ensure_watchers(&services, &mut watchers, &watch_tx) {
                    Ok(WatcherSetup { started, failures }) => {
                        if !failures.is_empty() {
                            tracing::warn!(
                                error_kind = "watcher_restart",
                                failure_count = failures.len(),
                                "collection watcher restart remains pending"
                            );
                            for (collection_id, _) in &failures {
                                request_quarantine(
                                    &services,
                                    &mut background,
                                    &mut watcher_quarantined,
                                    *collection_id,
                                    "no se pudo reiniciar el watcher de la colección",
                                );
                            }
                        }
                        if !services.models_ready() {
                            continue;
                        }
                        for collection_id in started {
                            if !services
                                .startup_preflight_blocks_automatic_scan(collection_id)
                                .is_ok_and(|blocked| !blocked)
                            {
                                continue;
                            }
                            request_scan(
                                &services,
                                &mut scan_scheduler,
                                &mut background,
                                &events,
                                collection_id,
                            );
                        }
                    }
                    Err(_error) => tracing::warn!(
                        error_kind = "watcher_restart",
                        "could not inspect collections while retrying watchers"
                    ),
                }
            }
            _ = connectivity_tick.tick() => {
                if let Some(operation) = firewall_operation.as_mut()
                    && slow_notice_is_due(
                        operation.started_at.elapsed(),
                        operation.slow_notice_sent,
                    )
                {
                    operation.slow_notice_sent = true;
                    send(
                        &events,
                        WorkerEvent::FirewallOperationUpdated {
                            request_id: operation.request_id,
                            state: Some(FirewallOperationView::TakingLonger),
                        },
                    );
                }
                if desktop_config.lan_preference == LanPreference::Enabled
                    && !firewall_request_is_busy(
                    active_connectivity_request,
                    firewall_operation,
                ) {
                    let request_id = Uuid::new_v4();
                    active_connectivity_request = Some(request_id);
                    spawn_connectivity_diagnostic(&mut background, request_id);
                }
            }
            _ = updater_tick.tick() => {
                let now = Instant::now();
                if updater_schedule.is_due(now) && active_updater_request.is_none() {
                    updater_schedule.record_attempt(now, schedule_jitter(SystemTime::now()));
                    queue_updater_operation(
                        &mut background,
                        &events,
                        updater.as_ref(),
                        updater_disabled_reason,
                        &mut active_updater_request,
                        Uuid::new_v4(),
                        UpdaterOperation::Check,
                    );
                }
            }
            _ = periodic_reconcile.tick() => {
                let checked_at = chrono::Utc::now();
                if !services.models_ready() {
                    tracing::info!(
                        reason = "periodic_safety_reconciliation",
                        checked_at = %checked_at.to_rfc3339(),
                        "periodic collection reconciliation skipped because models are not ready"
                    );
                    continue;
                }

                match schedule_idle_scans(
                    &services,
                    &watchers,
                    &mut scan_scheduler,
                    &mut background,
                    &events,
                ) {
                    Ok(summary) => tracing::info!(
                        reason = "periodic_safety_reconciliation",
                        checked_at = %checked_at.to_rfc3339(),
                        watched_collection_count = summary.watched,
                        scheduled_collection_count = summary.scheduled,
                        already_pending_count = summary.already_pending,
                        "periodic collection reconciliation checked"
                    ),
                    Err(_) => tracing::warn!(
                        reason = "periodic_safety_reconciliation",
                        checked_at = %checked_at.to_rfc3339(),
                        error_kind = "collection_listing",
                        "periodic collection reconciliation could not inspect collections"
                    ),
                }
            }
            completion = lifecycle.join_next(), if !lifecycle.is_empty() => {
                if completion.is_some_and(|result| result.is_err()) {
                    tracing::warn!(
                        error_kind = "model_lifecycle_task_join",
                        "a supervised model lifecycle task did not join cleanly"
                    );
                }
            }
            completion = background.join_next(), if !background.is_empty() => {
                match completion {
                    Some(Ok(BackgroundCompletion::Approve { concept_id, result })) => {
                        approving_reviews.remove(&concept_id);
                        match result {
                            Ok(()) => send(
                                &events,
                                WorkerEvent::Notice("Documento revisado y publicado".into()),
                            ),
                            Err(error) => send(
                                &events,
                                WorkerEvent::Error(format!(
                                    "No se pudo publicar el concepto OKF: {error}"
                                )),
                            ),
                        }
                        refresh_content_views(&services, &events);
                        spawn_next_wiki_health(
                            &services,
                            &mut background,
                            &mut wiki_health_generation,
                            Uuid::new_v4(),
                        );
                    }
                    Some(Ok(BackgroundCompletion::Preflight { collection_id, result })) => {
                        if let Err(error) = result {
                            send(&events, WorkerEvent::Error(format!(
                                "No se pudo retirar inmediatamente una revisión cambiada de la colección {collection_id}: {error}"
                            )));
                            request_quarantine(
                                &services,
                                &mut background,
                                &mut watcher_quarantined,
                                collection_id,
                                "falló la prevalidación del filesystem observado",
                            );
                        }
                        if watchers.contains_key(&collection_id) {
                            // The preflight may have raced with the follow-up
                            // scan that originally triggered it. Request one
                            // final pass while its watcher remains active.
                            request_scan(
                                &services,
                                &mut scan_scheduler,
                                &mut background,
                                &events,
                                collection_id,
                            );
                        } else {
                            request_quarantine(
                                &services,
                                &mut background,
                                &mut watcher_quarantined,
                                collection_id,
                                "la prevalidación terminó sin un watcher activo",
                            );
                        }
                        refresh_content_views(&services, &events);
                        for ready in preflight_scheduler.finish(collection_id) {
                            spawn_preflight(&services, &mut background, ready);
                        }
                    }
                    Some(Ok(BackgroundCompletion::Scan { collection_id, result })) => {
                        let mut successful_manual_summary = None;
                        if !watchers.contains_key(&collection_id) {
                            clear_manual_rescan(&mut manual_rescans, collection_id);
                            request_quarantine(
                                &services,
                                &mut background,
                                &mut watcher_quarantined,
                                collection_id,
                                "un escaneo terminó después de perderse el watcher",
                            );
                        } else {
                            match result {
                            Ok(outcomes) => {
                                if services
                                    .clear_startup_preflight_block(collection_id)
                                    .is_err()
                                {
                                    tracing::warn!(
                                        error_kind = "startup_preflight_state",
                                        "a successful scan could not clear its startup block"
                                    );
                                }
                                watcher_quarantined.remove(&collection_id);
                                report_ingest_outcomes(&outcomes, &events);
                                successful_manual_summary =
                                    Some(manual_rescan_summary(&outcomes));
                                spawn_wiki_maintenance(
                                    &services,
                                    &mut background,
                                    collection_id,
                                );
                            }
                            Err(error) => {
                                clear_manual_rescan(&mut manual_rescans, collection_id);
                                send(&events, WorkerEvent::Error(format!(
                                    "Falló el escaneo de la colección {collection_id}: {error}"
                                )));
                                request_quarantine(
                                    &services,
                                    &mut background,
                                    &mut watcher_quarantined,
                                    collection_id,
                                    "falló la reconciliación completa del filesystem observado",
                                );
                            }
                            }
                        }
                        refresh_content_views(&services, &events);
                        let ready = scan_scheduler.finish(collection_id);
                        publish_scan_state(&events, &scan_scheduler, collection_id);
                        if let Some(summary) = take_manual_rescan_summary(
                            &mut manual_rescans,
                            &scan_scheduler,
                            collection_id,
                            successful_manual_summary,
                        ) {
                            send(&events, WorkerEvent::Notice(summary));
                        }
                        spawn_ready_scans(
                            &services,
                            &mut background,
                            &events,
                            ready,
                        );
                    }
                    Some(Ok(BackgroundCompletion::Quarantine { collection_id, result })) => {
                        match result {
                            Ok(()) => send(
                                &events,
                                WorkerEvent::Notice(format!(
                                    "La colección {collection_id} quedó retirada hasta un nuevo escaneo y revisión"
                                )),
                            ),
                            Err(error) => {
                                // The collection was not proven safe. Allow the
                                // watcher retry or a later failed scan to request
                                // the fail-closed transition again.
                                watcher_quarantined.remove(&collection_id);
                                send(
                                    &events,
                                    WorkerEvent::Error(format!(
                                        "No se pudo completar la cuarentena de la colección {collection_id}: {error}"
                                    )),
                                );
                            }
                        }
                        refresh_content_views(&services, &events);
                    }
                    Some(Ok(BackgroundCompletion::ReanalyzeReview { concept_id, result })) => {
                        reanalyzing_reviews.remove(&concept_id);
                        send(
                            &events,
                            WorkerEvent::ReviewReanalysis {
                                concept_id,
                                running: false,
                            },
                        );
                        match result {
                            Ok(()) => send(
                                &events,
                                WorkerEvent::Notice(
                                    "El borrador automático se actualizó y continúa pendiente de aprobación"
                                        .into(),
                                ),
                            ),
                            Err(error) => send(
                                &events,
                                WorkerEvent::Error(format!(
                                    "No se pudo volver a analizar el documento; se conservó el borrador anterior: {error}"
                                )),
                            ),
                        }
                        refresh_content_views(&services, &events);
                    }
                    Some(Ok(BackgroundCompletion::ReviewEvidence {
                        request_id,
                        concept_id,
                        expected_source_revision,
                        result,
                    })) => send(
                        &events,
                        WorkerEvent::ReviewEvidenceLoaded {
                            request_id,
                            concept_id,
                            expected_source_revision,
                            result,
                        },
                    ),
                    Some(Ok(BackgroundCompletion::KnowledgeBundle {
                        request_id,
                        collection_id,
                        result,
                    })) => send(
                        &events,
                        knowledge_bundle_loaded_event(request_id, collection_id, result),
                    ),
                    Some(Ok(BackgroundCompletion::KnowledgePage {
                        request_id,
                        collection_id,
                        page_id,
                        result,
                    })) => send(
                        &events,
                        knowledge_page_loaded_event(
                            request_id,
                            collection_id,
                            page_id,
                            result,
                        ),
                    ),
                    Some(Ok(BackgroundCompletion::Search { request_id, result, route_kind })) => {
                        let result = result
                            .map(|response| {
                                let coverage = search_coverage_view(&response);
                                (response.hits, coverage, route_kind)
                            })
                            .map_err(|error| format!("Falló la búsqueda: {error}"));
                        send(
                            &events,
                            WorkerEvent::SearchFinished { request_id, result },
                        );
                    }
                    Some(Ok(BackgroundCompletion::PublicBrowse { request_id, result })) => {
                        let result = result
                            .map(|page| (page.concepts, page.next_cursor))
                            .map_err(|error| format!("Falló la navegación pública: {error}"));
                        send(&events, WorkerEvent::PublicBrowseFinished { request_id, result });
                    }
                    Some(Ok(BackgroundCompletion::ChatIntegrations {
                        request_id,
                        action,
                        result,
                    })) => {
                        if active_integration_request == Some(request_id) {
                            active_integration_request = None;
                        }
                        if result.is_ok() {
                            update_claude_approval_after_action(
                                &mut claude_approval_state,
                                action,
                            );
                        }
                        let result = result.and_then(|mut integrations| {
                            let now = SystemTime::now();
                            let activities = services.latest_mcp_client_activities();
                            let claude_activity_recent = apply_recent_mcp_activities(
                                &mut integrations,
                                activities.iter(),
                                now,
                            );
                            apply_claude_approval_state(
                                &mut integrations,
                                &mut claude_approval_state,
                                claude_activity_recent,
                            );
                            services
                                .collection_views()
                                .map(|collections| ChatIntegrationsSnapshot {
                                    integrations,
                                    external_ai_collection_count: collections
                                        .iter()
                                        .filter(|collection| collection.allow_external_ai)
                                        .count(),
                                })
                                .map_err(|error| {
                                    format!(
                                        "No se pudo comprobar la política de colecciones: {error:#}"
                                    )
                                })
                        });
                        send(
                            &events,
                            WorkerEvent::ChatIntegrationsUpdated { request_id, result },
                        );
                    }
                    Some(Ok(BackgroundCompletion::Autostart { request_id, result })) => {
                        send(
                            &events,
                            WorkerEvent::AutostartUpdated { request_id, result },
                        );
                    }
                    Some(Ok(BackgroundCompletion::Updater { request_id, result })) => {
                        if active_updater_request == Some(request_id) {
                            active_updater_request = None;
                        }
                        send(
                            &events,
                            WorkerEvent::UpdaterUpdated {
                                request_id,
                                result: result.map(UpdaterWorkerView::Ready),
                            },
                        );
                    }
                    Some(Ok(BackgroundCompletion::WikiMaintenance {
                        collection_id,
                        result,
                    })) => {
                        match result {
                            Ok(true) => {
                                send(
                                    &events,
                                    WorkerEvent::WikiMaintenanceFinished {
                                        collection_id,
                                        repaired: true,
                                    },
                                );
                            }
                            Ok(false) => {}
                            Err(_) => {
                                tracing::warn!(
                                    error_kind = "wiki_derived_maintenance",
                                    "automatic derived Wiki maintenance was not applied"
                                );
                                send(
                                    &events,
                                    WorkerEvent::WikiMaintenanceFinished {
                                        collection_id,
                                        repaired: false,
                                    },
                                );
                            }
                        }
                        spawn_next_wiki_health(
                            &services,
                            &mut background,
                            &mut wiki_health_generation,
                            Uuid::new_v4(),
                        );
                    }
                    Some(Ok(BackgroundCompletion::RelinkCollection {
                        collection_id,
                        folder,
                        result,
                    })) => {
                        match result {
                            Ok(()) => match CollectionWatcherHandle::spawn(
                                collection_id,
                                folder,
                                watch_tx.clone(),
                            ) {
                                Ok(watcher) => {
                                    watchers.insert(collection_id, watcher);
                                    if services.models_ready() {
                                        request_scan(
                                            &services,
                                            &mut scan_scheduler,
                                            &mut background,
                                            &events,
                                            collection_id,
                                        );
                                    }
                                    send(
                                        &events,
                                        WorkerEvent::Notice(
                                            "La carpeta quedó vinculada y será reconciliada"
                                                .to_owned(),
                                        ),
                                    );
                                }
                                Err(error) => send(
                                    &events,
                                    WorkerEvent::Error(format!(
                                        "La carpeta cambió, pero no se pudo iniciar su supervisión: {error}"
                                    )),
                                ),
                            },
                            Err(error) => send(
                                &events,
                                WorkerEvent::Error(format!(
                                    "No se pudo volver a vincular la carpeta: {error}"
                                )),
                            ),
                        }
                        refresh_content_views(&services, &events);
                    }
                    Some(Ok(BackgroundCompletion::LanAddressesResolved {
                        generation,
                        listener,
                        result,
                    })) => {
                        if lan_address_resolution_is_current(
                            services.network_event_is_current(generation),
                            current_lan_listener.as_ref(),
                            &listener,
                            lan_runtime_enabled,
                        ) {
                            match result {
                                Ok(addresses) => lan_local_addresses = addresses,
                                Err(_) => {
                                    lan_local_addresses.clear();
                                    tracing::warn!(
                                        error_kind = "lan_manual_fallback_resolution",
                                        "advanced LAN fallback address is unavailable"
                                    );
                                }
                            }
                            send_lan_runtime(
                                &events,
                                lan_listener,
                                lan_discovery,
                                &lan_local_addresses,
                            );
                        }
                    }
                    Some(Ok(BackgroundCompletion::ConnectivityDiagnosed {
                        request_id,
                        result,
                    })) => {
                        let force_restart = restart_lan_request == Some(request_id);
                        if force_restart {
                            restart_lan_request = None;
                        }
                        if active_connectivity_request == Some(request_id) {
                            active_connectivity_request = None;
                        }
                        let snapshot_for_policy = match result {
                            Ok(snapshot) => {
                                send(
                                    &events,
                                    WorkerEvent::ConnectivityPlatformUpdated {
                                        request_id,
                                        result: Ok(snapshot),
                                    },
                                );
                                Some(snapshot)
                            }
                            Err(error) => {
                                send(
                                    &events,
                                    WorkerEvent::ConnectivityPlatformUpdated {
                                        request_id,
                                        result: Err(error),
                                    },
                                );
                                None
                            }
                        };
                        let policy = lan_runtime_policy(
                            desktop_config.lan_preference == LanPreference::Enabled,
                            snapshot_for_policy,
                        );
                        if let Err(message) = reconcile_lan_runtime(
                            &services,
                            &events,
                            if force_restart {
                                LanReconcileRequest::RefreshDiscovery(policy)
                            } else {
                                LanReconcileRequest::Apply(policy)
                            },
                            &mut lan_runtime_enabled,
                            &mut lan_listener,
                            &mut lan_discovery,
                            &mut lan_local_addresses,
                        )
                        .await
                        {
                            send(&events, WorkerEvent::Error(message));
                        }
                    }
                    Some(Ok(BackgroundCompletion::FirewallConfigured {
                        request_id,
                        result,
                    })) => {
                        if !firewall_completion_is_authoritative(
                            firewall_operation,
                            request_id,
                        ) {
                            tracing::warn!(
                                error_kind = "stale_firewall_completion",
                                "ignored a stale firewall completion"
                            );
                            continue;
                        }
                        firewall_operation = None;
                        send(
                            &events,
                            WorkerEvent::FirewallOperationUpdated {
                                request_id,
                                state: None,
                            },
                        );
                        let force_restart = restart_lan_request == Some(request_id);
                        if force_restart {
                            restart_lan_request = None;
                        }
                        if active_connectivity_request == Some(request_id) {
                            active_connectivity_request = None;
                        }
                        match result {
                            Ok(snapshot) => {
                                send(
                                    &events,
                                    WorkerEvent::ConnectivityPlatformUpdated {
                                        request_id,
                                        result: Ok(snapshot),
                                    },
                                );
                                let policy = lan_runtime_policy(
                                    desktop_config.lan_preference == LanPreference::Enabled,
                                    Some(snapshot),
                                );
                                if let Err(message) = reconcile_lan_runtime(
                                    &services,
                                    &events,
                                    if force_restart {
                                        LanReconcileRequest::RefreshDiscovery(policy)
                                    } else {
                                        LanReconcileRequest::Apply(policy)
                                    },
                                    &mut lan_runtime_enabled,
                                    &mut lan_listener,
                                    &mut lan_discovery,
                                    &mut lan_local_addresses,
                                )
                                .await
                                {
                                    send(&events, WorkerEvent::Error(message));
                                }
                            }
                            Err(error) => {
                                let policy = lan_runtime_policy(
                                    desktop_config.lan_preference == LanPreference::Enabled,
                                    None,
                                );
                                if let Err(message) = reconcile_lan_runtime(
                                    &services,
                                    &events,
                                    LanReconcileRequest::Apply(policy),
                                    &mut lan_runtime_enabled,
                                    &mut lan_listener,
                                    &mut lan_discovery,
                                    &mut lan_local_addresses,
                                )
                                .await
                                {
                                    send(&events, WorkerEvent::Error(message));
                                }
                                send(
                                    &events,
                                    WorkerEvent::ConnectivityPlatformUpdated {
                                        request_id,
                                        result: Err(error.into()),
                                    },
                                );
                                let diagnostic_id = Uuid::new_v4();
                                active_connectivity_request = Some(diagnostic_id);
                                spawn_connectivity_diagnostic(&mut background, diagnostic_id);
                            }
                        }
                    }
                    Some(Ok(BackgroundCompletion::WikiHealth {
                        request_id,
                        generation,
                        result,
                    })) => {
                        send(
                            &events,
                            WorkerEvent::WikiHealthUpdated {
                                request_id,
                                generation,
                                result,
                            },
                        );
                    }
                    Some(Ok(BackgroundCompletion::GuidedWikiRepairPrepared {
                        request_id,
                        collection_id,
                        result,
                    })) => {
                        if active_guided_repair_request == Some(request_id) {
                            active_guided_repair_request = None;
                        }
                        send(
                            &events,
                            WorkerEvent::GuidedWikiRepairPrepared {
                                request_id,
                                collection_id,
                                result,
                            },
                        );
                    }
                    Some(Ok(BackgroundCompletion::GuidedWikiRepairFinished {
                        request_id,
                        collection_id,
                        result,
                    })) => {
                        if active_guided_repair_request == Some(request_id) {
                            active_guided_repair_request = None;
                        }
                        send(
                            &events,
                            WorkerEvent::GuidedWikiRepairFinished {
                                request_id,
                                collection_id,
                                result,
                            },
                        );
                        refresh_content_views(&services, &events);
                        spawn_next_wiki_health(
                            &services,
                            &mut background,
                            &mut wiki_health_generation,
                            Uuid::new_v4(),
                        );
                    }
                    Some(Ok(BackgroundCompletion::ModelsEnabled { model_id, result })) => {
                        if model_lifecycle != ModelLifecycle::Enabling {
                            tracing::warn!(
                                ?model_lifecycle,
                                "ignored stale model enable completion"
                            );
                            continue;
                        }
                        if enabling_model_id.as_deref() != Some(model_id.as_str()) {
                            tracing::warn!(
                                expected = ?enabling_model_id,
                                actual = %model_id,
                                "model enable completion did not match pending state"
                            );
                            continue;
                        }
                        enabling_model_id = None;
                        match result {
                            Ok(()) => {
                                model_lifecycle = ModelLifecycle::Ready;
                                desktop_config.active_selection = Some(model_id.clone());
                                if desktop_config.pending_selection.as_deref()
                                    == Some(model_id.as_str())
                                {
                                    desktop_config.pending_selection = None;
                                }
                                let active_was_persisted =
                                    persist_config(&desktop_config, &paths, &events);
                                send(&events, WorkerEvent::ModelsReady);
                                send(
                                    &events,
                                    WorkerEvent::Notice(if active_was_persisted {
                                        "Modelos locales verificados y listos".into()
                                    } else {
                                        "El modelo está activo solo durante esta sesión; repara el guardado de configuración antes de reiniciar".into()
                                    }),
                                );
                                refresh_content_views(&services, &events);
                                if let Err(error) = schedule_all_scans(
                                    &services,
                                    &watchers,
                                    &mut scan_scheduler,
                                    &mut background,
                                    &events,
                                ) {
                                    send(
                                        &events,
                                        WorkerEvent::Error(format!(
                                            "No se pudieron programar los escaneos: {error:#}"
                                        )),
                                    );
                                }
                                send_model_state_with_known_plan(
                                    &mut lifecycle,
                                    &events,
                                    &asset_manager,
                                    &desktop_config,
                                    &recommendation,
                                    verified_model_plan.take(),
                                );
                                if should_probe_profile_activation(
                                    &desktop_config,
                                    &recommendation,
                                ) && let Some(selection) = recommendation.selection.clone()
                                {
                                    spawn_profile_activation_probe(
                                        &mut lifecycle,
                                        asset_manager.clone(),
                                        selection,
                                        internal_tx.clone(),
                                    );
                                }
                                if queued_install
                                    && recommendation
                                        .selection
                                        .as_ref()
                                        .is_some_and(|selection| selection.model_id != model_id)
                                    && let Some(selection) = recommendation.selection.clone()
                                {
                                    queued_install = false;
                                    let previous_config = desktop_config.clone();
                                    accept_selection_licenses(&mut desktop_config, &selection);
                                    if persist_config(&desktop_config, &paths, &events) {
                                        start_install(
                                            &mut lifecycle,
                                            &asset_manager,
                                            selection,
                                            &events,
                                            &internal_tx,
                                            &mut install_cancel,
                                        );
                                        model_lifecycle = ModelLifecycle::Installing;
                                    } else {
                                        desktop_config = previous_config;
                                        send(&events, WorkerEvent::InstallStopped);
                                    }
                                } else {
                                    if queued_install {
                                        send(&events, WorkerEvent::InstallStopped);
                                    }
                                    queued_install = false;
                                }
                            }
                            Err(error) => {
                                send(
                                    &events,
                                    WorkerEvent::Error(format!(
                                        "Los modelos verificados no pudieron habilitarse: {error}"
                                    )),
                                );
                                let fallback = desktop_config
                                    .active_selection
                                    .as_deref()
                                    .filter(|active| *active != model_id)
                                    .and_then(|active| {
                                        selection_for_model(
                                            desktop_config.profile,
                                            active,
                                            "Fallback tras fallar el smoke test pendiente",
                                        )
                                    });
                                if let Some(active) = fallback {
                                    desktop_config.pending_selection = None;
                                    let _ = persist_config(&desktop_config, &paths, &events);
                                    verifying_selection = active;
                                    model_lifecycle = ModelLifecycle::Verifying;
                                    spawn_verification(
                                        &mut lifecycle,
                                        asset_manager.clone(),
                                        verifying_selection.clone(),
                                        internal_tx.clone(),
                                    );
                                } else {
                                    if queued_install {
                                        queued_install = false;
                                        send(&events, WorkerEvent::InstallStopped);
                                    }
                                    model_lifecycle = ModelLifecycle::Missing;
                                    send(&events, WorkerEvent::ModelsMissing);
                                }
                                send_model_state_with_known_plan(
                                    &mut lifecycle,
                                    &events,
                                    &asset_manager,
                                    &desktop_config,
                                    &recommendation,
                                    verified_model_plan.take(),
                                );
                            }
                        }
                    }
                    Some(Err(error)) => send(
                        &events,
                        WorkerEvent::Error(format!(
                            "Una tarea de fondo terminó inesperadamente: {error}"
                        )),
                    ),
                    None => {}
                }
            }
            network = network_events.recv(), if network_open => {
                match network {
                    Ok(sequenced) => {
                        if !services.network_event_is_current(sequenced.generation) {
                            continue;
                        }
                        let generation = sequenced.generation;
                        let event = sequenced.event;
                        let runtime_state_changed = match &event {
                            NetworkEvent::Listening { address } => {
                                lan_listener = LanListenerView::Listening;
                                lan_local_addresses.clear();
                                current_lan_listener = Some(address.clone());
                                spawn_lan_address_resolution(
                                    &services,
                                    &mut background,
                                    generation,
                                    address.clone(),
                                );
                                true
                            }
                            NetworkEvent::ListenerUnavailable => {
                                lan_listener = LanListenerView::Failed;
                                invalidate_lan_address_resolution(
                                    &mut current_lan_listener,
                                    &mut lan_local_addresses,
                                );
                                schedule_lan_runtime_restart(
                                    &mut background,
                                    &mut active_connectivity_request,
                                    &mut restart_lan_request,
                                );
                                true
                            }
                            NetworkEvent::DiscoveryStarted => {
                                lan_discovery = LanDiscoveryView::Active;
                                true
                            }
                            _ => false,
                        };
                        if runtime_state_changed {
                            send_lan_runtime(
                                &events,
                                lan_listener,
                                lan_discovery,
                                &lan_local_addresses,
                            );
                        }
                        match services.handle_network_event(event) {
                            Ok(effect) => {
                                if let Some(notice) = effect.notice { send(&events, WorkerEvent::Notice(notice)); }
                                if let Some(warning) = effect.warning { send(&events, WorkerEvent::Error(warning)); }
                                if effect.peers_changed { refresh_peers(&services, &events); }
                            }
                            Err(error) => send(&events, WorkerEvent::Error(format!("No se pudo persistir el evento LAN: {error:#}"))),
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!(count, "LAN presentation events were coalesced; restarting discovery");
                        send(&events, WorkerEvent::Notice("La búsqueda de equipos se está sincronizando de nuevo".to_owned()));
                        lan_listener = LanListenerView::Starting;
                        lan_discovery = LanDiscoveryView::Starting;
                        invalidate_lan_address_resolution(
                            &mut current_lan_listener,
                            &mut lan_local_addresses,
                        );
                        send_lan_runtime(
                            &events,
                            lan_listener,
                            lan_discovery,
                            &lan_local_addresses,
                        );
                        schedule_lan_runtime_restart(
                            &mut background,
                            &mut active_connectivity_request,
                            &mut restart_lan_request,
                        );
                        refresh_peers(&services, &events);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        network_open = false;
                        lan_listener = LanListenerView::Failed;
                        lan_discovery = LanDiscoveryView::Failed;
                        invalidate_lan_address_resolution(
                            &mut current_lan_listener,
                            &mut lan_local_addresses,
                        );
                        send_lan_runtime(
                            &events,
                            lan_listener,
                            lan_discovery,
                            &lan_local_addresses,
                        );
                        send(&events, WorkerEvent::Error("El runtime LAN se detuvo".into()));
                    }
                }
            }
        }
    }
    if let Some(cancel) = install_cancel {
        cancel.cancel();
    }
    lifecycle.abort_all();
    while lifecycle.join_next().await.is_some() {}
    // JoinSets abort every in-flight search, scan and model operation. Joining
    // releases their Arc references before services are consumed for cleanup.
    background.abort_all();
    while background.join_next().await.is_some() {}
    drop(watchers);
    match Arc::try_unwrap(services) {
        Ok(services) => {
            if services.shutdown().await.is_err() {
                tracing::warn!(
                    error_kind = "shutdown",
                    "background services did not stop cleanly"
                );
            }
        }
        Err(_) => tracing::error!(
            error_kind = "shutdown",
            "background jobs retained the service graph during shutdown"
        ),
    }
}

fn send_ready(services: &DesktopServices, events: &Sender<WorkerEvent>) {
    match (
        services.collection_views(),
        services.review_views(),
        services.source_issue_views(),
        services.blocked_public_publishers(),
    ) {
        (Ok(collections), Ok(reviews), Ok(source_issues), Ok(blocked_public_publishers)) => send(
            events,
            WorkerEvent::Ready {
                node_id: services.node_id().to_owned(),
                mcp_url: services.mcp_endpoint().to_owned(),
                collections,
                reviews,
                source_issues,
                blocked_public_publishers,
            },
        ),
        (Err(error), _, _, _)
        | (_, Err(error), _, _)
        | (_, _, Err(error), _)
        | (_, _, _, Err(error)) => send(
            events,
            WorkerEvent::Error(format!("No se pudo cargar el estado local: {error:#}")),
        ),
    }
}

fn spawn_autostart(
    background: &mut JoinSet<BackgroundCompletion>,
    request_id: Uuid,
    enabled: Option<bool>,
) {
    background.spawn_blocking(move || {
        let result = std::env::current_exe()
            .map_err(|error| format!("No se pudo identificar el ejecutable instalado: {error}"))
            .and_then(|executable| {
                let manager = AutostartManager::new(executable);
                match enabled {
                    Some(true) => manager.enable(),
                    Some(false) => manager.disable(),
                    None => manager.status(),
                }
                .map_err(|error| error.to_string())
            });
        BackgroundCompletion::Autostart { request_id, result }
    });
}

fn send_lan_runtime(
    events: &Sender<WorkerEvent>,
    listener: LanListenerView,
    discovery: LanDiscoveryView,
    local_addresses: &[String],
) {
    send(
        events,
        WorkerEvent::LanRuntimeUpdated {
            request_id: Uuid::nil(),
            listener,
            discovery,
            local_addresses: local_addresses.to_vec(),
        },
    );
}

async fn reconcile_lan_runtime(
    services: &DesktopServices,
    events: &Sender<WorkerEvent>,
    request: LanReconcileRequest,
    runtime_enabled: &mut bool,
    listener: &mut LanListenerView,
    discovery: &mut LanDiscoveryView,
    local_addresses: &mut Vec<String>,
) -> Result<(), String> {
    let policy = request.policy();
    if *runtime_enabled && request.force_restart() {
        if services.disable_lan().await.is_err() {
            tracing::warn!(
                error_kind = "lan_manual_refresh_cleanup",
                "LAN runtime did not stop cleanly before discovery refresh"
            );
        }
        *runtime_enabled = false;
        local_addresses.clear();
    }
    if *runtime_enabled {
        match services.lan_runtime_is_healthy() {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                if services.disable_lan().await.is_err() {
                    tracing::warn!(
                        error_kind = "lan_stale_runtime_cleanup",
                        "stale LAN runtime did not shut down cleanly"
                    );
                }
                *runtime_enabled = false;
                local_addresses.clear();
            }
        }
    }

    if policy.should_run() {
        if *runtime_enabled {
            return Ok(());
        }
        *listener = LanListenerView::Starting;
        *discovery = LanDiscoveryView::Starting;
        local_addresses.clear();
        send_lan_runtime(events, *listener, *discovery, local_addresses);
        if services.enable_lan().await.is_err() {
            *listener = LanListenerView::Failed;
            *discovery = LanDiscoveryView::Failed;
            send_lan_runtime(events, *listener, *discovery, local_addresses);
            tracing::warn!(
                error_kind = "lan_runtime_start",
                "optional LAN runtime could not start"
            );
            return Err(
                "La conexión con otros equipos no pudo iniciarse; el conocimiento local y los chats continúan disponibles"
                    .to_owned(),
            );
        }
        *runtime_enabled = true;
        return Ok(());
    }

    let shutdown_failed = if *runtime_enabled {
        services.disable_lan().await.is_err()
    } else {
        false
    };
    *runtime_enabled = false;
    local_addresses.clear();
    (*listener, *discovery) = if policy == LanRuntimePolicy::WaitingForDiagnostic {
        (LanListenerView::Starting, LanDiscoveryView::Starting)
    } else {
        (LanListenerView::Stopped, LanDiscoveryView::Disabled)
    };
    send_lan_runtime(events, *listener, *discovery, local_addresses);
    if shutdown_failed {
        tracing::warn!(
            error_kind = "lan_runtime_stop",
            "optional LAN runtime did not stop cleanly"
        );
        Err(
            "La conexión con otros equipos se desactivó, pero una tarea de red no terminó limpiamente"
                .to_owned(),
        )
    } else {
        Ok(())
    }
}

fn spawn_connectivity_diagnostic(background: &mut JoinSet<BackgroundCompletion>, request_id: Uuid) {
    background.spawn_blocking(move || BackgroundCompletion::ConnectivityDiagnosed {
        request_id,
        result: Ok(diagnose_connectivity()),
    });
}

fn spawn_advanced_firewall_rules(background: &mut JoinSet<BackgroundCompletion>, request_id: Uuid) {
    background.spawn_blocking(move || BackgroundCompletion::ConnectivityDiagnosed {
        request_id,
        result: open_advanced_firewall_rules()
            .map(|()| diagnose_connectivity())
            .map_err(ConnectivityIssueCode::from),
    });
}

fn schedule_lan_runtime_restart(
    background: &mut JoinSet<BackgroundCompletion>,
    active_connectivity_request: &mut Option<Uuid>,
    restart_lan_request: &mut Option<Uuid>,
) {
    let request_id = if let Some(request_id) = *active_connectivity_request {
        request_id
    } else {
        let request_id = Uuid::new_v4();
        *active_connectivity_request = Some(request_id);
        spawn_connectivity_diagnostic(background, request_id);
        request_id
    };
    *restart_lan_request = Some(request_id);
}

fn invalidate_lan_address_resolution(
    current_listener: &mut Option<airwiki_network::Multiaddr>,
    local_addresses: &mut Vec<String>,
) {
    *current_listener = None;
    local_addresses.clear();
}

fn lan_address_resolution_is_current(
    generation_is_current: bool,
    current_listener: Option<&airwiki_network::Multiaddr>,
    resolved_listener: &airwiki_network::Multiaddr,
    runtime_enabled: bool,
) -> bool {
    generation_is_current && current_listener == Some(resolved_listener) && runtime_enabled
}

fn spawn_lan_address_resolution(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    generation: u64,
    listener: airwiki_network::Multiaddr,
) {
    let services = Arc::clone(services);
    background.spawn_blocking(move || BackgroundCompletion::LanAddressesResolved {
        generation,
        listener: listener.clone(),
        result: services
            .advertised_lan_addresses(&listener)
            .map_err(|_| "LAN fallback address unavailable".to_owned()),
    });
}

fn spawn_firewall_configuration(
    background: &mut JoinSet<BackgroundCompletion>,
    request_id: Uuid,
    install: bool,
) {
    background.spawn_blocking(move || {
        let result = if install {
            let before = diagnose_connectivity();
            firewall_install_preflight(before).and_then(|decision| match decision {
                FirewallInstallDecision::AlreadyReady => Ok(before),
                FirewallInstallDecision::Configure => {
                    install_firewall_rules().map(|()| diagnose_connectivity())
                }
            })
        } else {
            remove_firewall_rules().map(|()| diagnose_connectivity())
        };
        BackgroundCompletion::FirewallConfigured { request_id, result }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirewallInstallDecision {
    AlreadyReady,
    Configure,
}

fn firewall_install_preflight(
    snapshot: ConnectivityPlatformSnapshot,
) -> Result<FirewallInstallDecision, FirewallActionError> {
    if !matches!(
        snapshot.network_profile,
        NetworkProfileState::Private | NetworkProfileState::Domain
    ) {
        return Err(FirewallActionError::StateChanged);
    }
    if snapshot.firewall == FirewallDiagnosticState::Ready {
        return Ok(FirewallInstallDecision::AlreadyReady);
    }
    match snapshot.firewall {
        FirewallDiagnosticState::ManagedPolicy => Err(FirewallActionError::ManagedPolicy),
        FirewallDiagnosticState::Conflict => Err(FirewallActionError::Conflict),
        FirewallDiagnosticState::Unsupported => Err(FirewallActionError::Unsupported),
        FirewallDiagnosticState::RulesMissing
            if snapshot.firewall_helper == FirewallHelperState::Verified =>
        {
            Ok(FirewallInstallDecision::Configure)
        }
        _ => Err(FirewallActionError::StateChanged),
    }
}

fn spawn_wiki_maintenance(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    collection_id: Uuid,
) {
    let services = Arc::clone(services);
    background.spawn_blocking(move || BackgroundCompletion::WikiMaintenance {
        collection_id,
        result: services
            .maintain_derived_wiki(collection_id)
            .map_err(|error| format!("{error:#}")),
    });
}

fn spawn_next_wiki_health(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    generation: &mut u64,
    request_id: Uuid,
) {
    *generation = generation.saturating_add(1);
    spawn_wiki_health(services, background, request_id, *generation);
}

fn spawn_wiki_health(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    request_id: Uuid,
    generation: u64,
) {
    let services = Arc::clone(services);
    background.spawn_blocking(move || {
        let result = services
            .wiki_health_rollup()
            .map(|counts| wiki_health_summary(counts, SystemTime::now()))
            .map_err(|error| format!("{error:#}"));
        BackgroundCompletion::WikiHealth {
            request_id,
            generation,
            result,
        }
    });
}

fn wiki_health_summary(rollup: WikiHealthRollup, checked_at: SystemTime) -> WikiHealthSummaryView {
    WikiHealthSummaryView {
        error_count: rollup.error_count,
        warning_count: rollup.warning_count,
        updating_count: rollup.updating_count,
        attention_collection_id: rollup.attention_collection_id,
        checked_at: Some(checked_at),
    }
}

fn spawn_guided_repair_preview(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    request_id: Uuid,
    collection_id: Uuid,
) {
    let services = Arc::clone(services);
    background.spawn_blocking(move || {
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            services.prepare_guided_wiki_repair(collection_id)
        }))
        .map_err(|_| "wiki_repair_worker_panicked".to_owned())
        .and_then(|result| result.map_err(|error| guided_repair_error_code(&error)));
        BackgroundCompletion::GuidedWikiRepairPrepared {
            request_id,
            collection_id,
            result,
        }
    });
}

fn spawn_guided_repair_execution(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    request_id: Uuid,
    preview: GuidedRepairPreview,
) {
    let collection_id = preview.collection_id;
    let services = Arc::clone(services);
    background.spawn_blocking(move || {
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            services.execute_guided_wiki_repair(&preview)
        }))
        .map_err(|_| "wiki_repair_worker_panicked".to_owned())
        .and_then(|result| result.map_err(|error| guided_repair_error_code(&error)));
        BackgroundCompletion::GuidedWikiRepairFinished {
            request_id,
            collection_id,
            result,
        }
    });
}

fn guided_repair_error_code(error: &anyhow::Error) -> String {
    let code = match error.downcast_ref::<WikiRepairError>() {
        Some(WikiRepairError::HistoryRepairRequiresHumanRecovery) => {
            "wiki_repair_history_requires_human"
        }
        Some(WikiRepairError::BundleUpdating) => "wiki_repair_bundle_updating",
        Some(WikiRepairError::StalePlan) => "wiki_repair_stale_preview",
        Some(WikiRepairError::ConfirmationRequired { .. }) => "wiki_repair_confirmation_required",
        Some(WikiRepairError::UnresolvedGuidedScope) => "wiki_repair_unresolved_scope",
        Some(WikiRepairError::UnsafeBundleLayout) => "wiki_repair_unsafe_layout",
        Some(WikiRepairError::SnapshotTooLarge { .. }) => "wiki_repair_snapshot_too_large",
        Some(WikiRepairError::GuidedPostValidation { .. }) => "wiki_repair_post_validation_failed",
        Some(WikiRepairError::RollbackFailed { .. }) => "wiki_repair_rollback_failed",
        _ => "wiki_repair_failed",
    };
    tracing::warn!(error_kind = code, "guided Wiki repair did not complete");
    code.to_owned()
}

fn queue_updater_operation(
    background: &mut JoinSet<BackgroundCompletion>,
    events: &Sender<WorkerEvent>,
    updater: Option<&Arc<Mutex<UpdaterService<PackagerUpdateBackend>>>>,
    disabled_reason: Option<UpdaterDisabledReason>,
    active_request: &mut Option<Uuid>,
    request_id: Uuid,
    operation: UpdaterOperation,
) {
    if active_request.is_some() {
        send(
            events,
            WorkerEvent::UpdaterUpdated {
                request_id,
                result: Err("Ya hay una operación de actualización en curso".to_owned()),
            },
        );
        return;
    }
    let Some(updater) = updater.cloned() else {
        send(
            events,
            WorkerEvent::UpdaterUpdated {
                request_id,
                result: Ok(UpdaterWorkerView::Disabled(
                    disabled_reason.unwrap_or(UpdaterDisabledReason::NotConfigured),
                )),
            },
        );
        return;
    };
    *active_request = Some(request_id);
    background.spawn_blocking(move || {
        let result = updater
            .lock()
            .map_err(|_| "El estado del actualizador no está disponible".to_owned())
            .and_then(|mut updater| {
                match operation {
                    UpdaterOperation::Check => updater.check_blocking(),
                    UpdaterOperation::Download => {
                        let confirmation = updater
                            .confirm_download()
                            .map_err(|error| error.to_string())?;
                        updater
                            .download_blocking(confirmation)
                            .map_err(|error| error.to_string())?;
                    }
                    UpdaterOperation::Install => {
                        let confirmation = updater
                            .confirm_install()
                            .map_err(|error| error.to_string())?;
                        updater
                            .install_blocking(confirmation)
                            .map_err(|error| error.to_string())?;
                    }
                }
                Ok(updater.view().clone())
            });
        BackgroundCompletion::Updater { request_id, result }
    });
}

#[must_use]
fn persist_config(config: &DesktopConfig, paths: &AppPaths, events: &Sender<WorkerEvent>) -> bool {
    if let Err(error) = config.save_atomic(&paths.config) {
        send(
            events,
            WorkerEvent::Error(format!(
                "No se pudo guardar la configuración de modelos: {error:#}"
            )),
        );
        false
    } else {
        true
    }
}

fn send_model_state(
    lifecycle: &mut JoinSet<()>,
    events: &Sender<WorkerEvent>,
    manager: &AssetManager,
    config: &DesktopConfig,
    decision: &ModelDecision,
) {
    send_model_state_with_known_plan(lifecycle, events, manager, config, decision, None);
}

fn send_model_state_with_known_plan(
    lifecycle: &mut JoinSet<()>,
    events: &Sender<WorkerEvent>,
    manager: &AssetManager,
    config: &DesktopConfig,
    decision: &ModelDecision,
    known_plan: Option<InstallPlan>,
) {
    let state_sequence = MODEL_STATE_SEQUENCE.fetch_add(1, Ordering::SeqCst);
    let events = events.clone();
    let manager = manager.clone();
    let config = config.clone();
    let decision = decision.clone();
    let known_plan = matching_known_install_plan(known_plan, &decision);
    lifecycle.spawn(async move {
        // Snapshot verification can hash several GiB. Serialize it so rapid profile changes
        // discard queued stale requests instead of starting redundant filesystem work.
        let _plan_guard = MODEL_STATE_PLAN_GATE.lock().await;
        if !model_state_request_is_current(
            state_sequence,
            MODEL_STATE_SEQUENCE.load(Ordering::SeqCst),
        ) {
            return;
        }

        let mut issues = decision.issues.clone();
        let mut download_bytes = 0;
        let mut required_free_bytes = 0;
        let mut fits_available_disk = false;
        let mut recommended_assets_installed = false;
        if let Some(selection) = decision.selection.as_ref() {
            let plan = match known_plan {
                Some(plan) => Ok(plan),
                None => manager.build_install_plan_async(selection).await,
            };
            match plan {
                Ok(plan) => {
                    recommended_assets_installed = plan.artifact_ids.is_empty();
                    download_bytes = plan.download_bytes;
                    required_free_bytes = plan.required_free_bytes;
                    fits_available_disk = plan.fits_available_disk;
                    if !plan.fits_available_disk {
                        issues.push(format!(
                            "La instalación requiere {:.1} GiB libres incluyendo margen",
                            plan.required_free_bytes as f64 / 1024_f64.powi(3)
                        ));
                    }
                }
                Err(error) => issues.push(format!("No se pudo preparar la descarga: {error:#}")),
            }
        }
        if !model_state_request_is_current(
            state_sequence,
            MODEL_STATE_SEQUENCE.load(Ordering::SeqCst),
        ) {
            return;
        }

        let selected = decision.selection.as_ref();
        send(
            &events,
            WorkerEvent::ModelState(ModelStateView {
                state_sequence,
                profile: config.profile,
                recommended_model_id: selected.map(|selection| selection.model_id.to_owned()),
                recommended_display_name: selected
                    .map(|selection| selection.manifest.display_name.to_owned()),
                recommendation_reason: selected.map(|selection| selection.reason.clone()),
                degraded: selected.is_some_and(|selection| selection.degraded),
                issues,
                active_model_id: config.active_selection.clone(),
                pending_model_id: config.pending_selection.clone(),
                recommended_assets_installed,
                download_bytes,
                required_free_bytes,
                fits_available_disk,
                license: selected.map(|selection| selection.manifest.artifact.license.to_owned()),
                license_url: selected
                    .map(|selection| selection.manifest.artifact.license_url.to_owned()),
                revision: selected.map(|selection| selection.manifest.artifact.revision.to_owned()),
                license_accepted: selected
                    .is_some_and(|selection| selection_licenses_accepted(&config, selection)),
            }),
        );
    });
}

fn matching_known_install_plan(
    known_plan: Option<InstallPlan>,
    decision: &ModelDecision,
) -> Option<InstallPlan> {
    let recommended = decision
        .selection
        .as_ref()
        .map(|selection| selection.model_id)?;
    known_plan.filter(|plan| plan.selection.model_id == recommended)
}

fn refresh_content_views(services: &DesktopServices, events: &Sender<WorkerEvent>) {
    match services.collection_views() {
        Ok(collections) => send(events, WorkerEvent::Collections(collections)),
        Err(error) => send(
            events,
            WorkerEvent::Error(format!(
                "No se pudieron refrescar las colecciones: {error:#}"
            )),
        ),
    }
    match services.review_views() {
        Ok(reviews) => send(events, WorkerEvent::Reviews(reviews)),
        Err(error) => send(
            events,
            WorkerEvent::Error(format!("No se pudo refrescar la revisión: {error:#}")),
        ),
    }
    match services.source_issue_views() {
        Ok(issues) => send(events, WorkerEvent::SourceIssues(issues)),
        Err(error) => send(
            events,
            WorkerEvent::Error(format!(
                "No se pudieron refrescar los archivos pendientes: {error:#}"
            )),
        ),
    }
}

fn refresh_peers(services: &DesktopServices, events: &Sender<WorkerEvent>) {
    match services.peer_views() {
        Ok(peers) => send(events, WorkerEvent::Peers(peers)),
        Err(error) => send(
            events,
            WorkerEvent::Error(format!("No se pudo refrescar la confianza LAN: {error:#}")),
        ),
    }
}

fn ensure_watchers(
    services: &DesktopServices,
    watchers: &mut HashMap<Uuid, CollectionWatcherHandle>,
    watch_tx: &UnboundedSender<CollectionWatchEvent>,
) -> anyhow::Result<WatcherSetup> {
    let mut started = Vec::new();
    let mut failures = Vec::new();
    for collection in services.collection_views()? {
        if let std::collections::hash_map::Entry::Vacant(entry) = watchers.entry(collection.id) {
            match CollectionWatcherHandle::spawn(collection.id, collection.folder, watch_tx.clone())
            {
                Ok(watcher) => {
                    entry.insert(watcher);
                    started.push(collection.id);
                }
                Err(error) => failures.push((collection.id, format!("{error:#}"))),
            }
        }
    }
    Ok(WatcherSetup { started, failures })
}

fn watcher_failure_summary(failures: &[(Uuid, String)]) -> String {
    failures
        .iter()
        .map(|(collection_id, error)| format!("{collection_id}: {error}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn request_quarantine(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    watcher_quarantined: &mut HashSet<Uuid>,
    collection_id: Uuid,
    reason: &str,
) {
    if watcher_quarantined.insert(collection_id) {
        spawn_quarantine(services, background, collection_id, reason.to_owned());
    }
}

fn spawn_quarantine(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    collection_id: Uuid,
    reason: String,
) {
    let services = Arc::clone(services);
    background.spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            services.quarantine_collection(collection_id, &reason)
        })
        .await
        .map_err(|error| format!("falló el worker de cuarentena: {error}"))
        .and_then(|result| result.map_err(|error| format!("{error:#}")));
        BackgroundCompletion::Quarantine {
            collection_id,
            result,
        }
    });
}

fn spawn_verification(
    lifecycle: &mut JoinSet<()>,
    manager: AssetManager,
    selection: ModelSelection,
    completion_tx: UnboundedSender<InternalEvent>,
) {
    lifecycle.spawn(async move {
        let result = AssertUnwindSafe(manager.verify_selection(&selection))
            .catch_unwind()
            .await
            .map_err(panic_message)
            .and_then(|result| result.map_err(|error| format!("{error:#}")));
        let _ = completion_tx.send(InternalEvent::VerificationFinished(result));
    });
}

fn spawn_profile_activation_probe(
    lifecycle: &mut JoinSet<()>,
    manager: AssetManager,
    selection: ModelSelection,
    completion_tx: UnboundedSender<InternalEvent>,
) {
    lifecycle.spawn(async move {
        let model_id = selection.model_id.to_owned();
        let result = AssertUnwindSafe(manager.verify_selection(&selection))
            .catch_unwind()
            .await
            .map_err(panic_message)
            .and_then(|result| result.map_err(|error| format!("{error:#}")));
        let _ = completion_tx.send(InternalEvent::ProfileActivationProbed { model_id, result });
    });
}

fn start_install(
    lifecycle: &mut JoinSet<()>,
    manager: &AssetManager,
    selection: ModelSelection,
    events: &Sender<WorkerEvent>,
    completion_tx: &UnboundedSender<InternalEvent>,
    install_cancel: &mut Option<CancellationToken>,
) {
    const MAX_TRANSIENT_RETRIES: u32 = 2;
    debug_assert!(install_cancel.is_none());
    let cancel = CancellationToken::new();
    *install_cancel = Some(cancel.clone());
    let manager = manager.clone();
    let progress_tx = events.clone();
    let completion_tx = completion_tx.clone();
    lifecycle.spawn(async move {
        let mut transient_retries = 0_u32;
        let result = loop {
            let attempt_progress = progress_tx.clone();
            let install =
                manager.install_selection_checked(&selection, cancel.clone(), move |event| {
                    send(&attempt_progress, WorkerEvent::InstallProgress(event))
                });
            match AssertUnwindSafe(install).catch_unwind().await {
                Ok(Ok(outcome)) => break Ok(outcome),
                Ok(Err(error))
                    if should_retry_transient_install(
                        install_failure_is_transient(&error),
                        transient_retries,
                        MAX_TRANSIENT_RETRIES,
                        cancel.is_cancelled(),
                    ) =>
                {
                    transient_retries += 1;
                    send(
                        &progress_tx,
                        WorkerEvent::Notice(
                            "La descarga se interrumpió temporalmente; se reintentará sin perder el progreso"
                                .to_owned(),
                        ),
                    );
                    let retry_delay = Duration::from_secs(5 * u64::from(transient_retries));
                    tokio::select! {
                        () = cancel.cancelled() => {
                            break Err("model_install_cancelled".to_owned());
                        }
                        () = tokio::time::sleep(retry_delay) => {}
                    }
                }
                Ok(Err(error)) => {
                    let code = if install_failure_is_transient(&error) {
                        "model_install_network_unavailable"
                    } else {
                        "model_install_action_required"
                    };
                    break Err(code.to_owned());
                }
                Err(_) => break Err("model_install_internal_failure".to_owned()),
            }
        };
        let _ = completion_tx.send(InternalEvent::InstallFinished(result));
    });
}

fn should_retry_transient_install(
    transient: bool,
    retries_completed: u32,
    maximum_retries: u32,
    cancelled: bool,
) -> bool {
    transient && retries_completed < maximum_retries && !cancelled
}

fn spawn_model_enable(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    paths: ModelRuntimePaths,
) {
    let services = Arc::clone(services);
    let model_id = paths.selection.model_id.to_owned();
    background.spawn(async move {
        let result = AssertUnwindSafe(services.enable_models(paths))
            .catch_unwind()
            .await
            .map_err(panic_message)
            .and_then(|result| result.map_err(|error| format!("{error:#}")));
        BackgroundCompletion::ModelsEnabled { model_id, result }
    });
}

fn settled_model_lifecycle(services: &DesktopServices) -> ModelLifecycle {
    if services.models_ready() {
        ModelLifecycle::Ready
    } else {
        ModelLifecycle::Missing
    }
}

fn can_change_model_profile(lifecycle: ModelLifecycle) -> bool {
    matches!(lifecycle, ModelLifecycle::Missing | ModelLifecycle::Ready)
}

/// A restart may only activate the model recommended by the persisted profile. Changing away from
/// a staged model cancels that activation but never removes its verified local artifacts; returning
/// to the matching profile lets the activation probe restore it without a download.
fn config_with_profile(
    config: &DesktopConfig,
    profile: ModelProfile,
    decision: &ModelDecision,
) -> (DesktopConfig, bool) {
    let mut updated = config.clone();
    updated.profile = profile;
    let pending_matches = updated
        .pending_selection
        .as_deref()
        .zip(
            decision
                .selection
                .as_ref()
                .map(|selection| selection.model_id),
        )
        .is_some_and(|(pending, recommended)| pending == recommended);
    let cleared_pending = updated.pending_selection.is_some() && !pending_matches;
    if cleared_pending {
        updated.pending_selection = None;
    }
    (updated, cleared_pending)
}

fn should_probe_profile_activation(config: &DesktopConfig, decision: &ModelDecision) -> bool {
    let Some(selection) = decision.selection.as_ref() else {
        return false;
    };
    config.pending_selection.is_none()
        && config.active_selection.as_deref() != Some(selection.model_id)
        && selection_licenses_accepted(config, selection)
}

fn license_bundle_revision(selection: &ModelSelection) -> String {
    format!(
        "{}+e5:{E5_REVISION}+mmarco:{MMARCO_REVISION}+llama:{LLAMA_CPP_BUILD}",
        selection.manifest.artifact.revision
    )
}

fn selection_licenses_accepted(config: &DesktopConfig, selection: &ModelSelection) -> bool {
    config.accepts(selection.model_id, &license_bundle_revision(selection))
}

fn accept_selection_licenses(config: &mut DesktopConfig, selection: &ModelSelection) {
    let licenses = format!(
        "generative={}; multilingual-e5-small={}; mMARCO={}; llama.cpp=MIT",
        selection.manifest.artifact.license, E5_FILES[0].license, MMARCO_COMMON_FILES[0].license,
    );
    config.accept_license(
        selection.model_id,
        &license_bundle_revision(selection),
        &licenses,
    );
}

fn should_stage_for_restart(
    models_ready: bool,
    active_model_id: Option<&str>,
    installed_model_id: &str,
) -> bool {
    models_ready && active_model_id != Some(installed_model_id)
}

const fn internal_event_name(event: &InternalEvent) -> &'static str {
    match event {
        InternalEvent::VerificationFinished(_) => "verification_finished",
        InternalEvent::ProfileActivationProbed { .. } => "profile_activation_probe_finished",
        InternalEvent::InstallFinished(_) => "install_finished",
    }
}

fn schedule_all_scans(
    services: &Arc<DesktopServices>,
    watchers: &HashMap<Uuid, CollectionWatcherHandle>,
    scheduler: &mut ScanScheduler,
    background: &mut JoinSet<BackgroundCompletion>,
    events: &Sender<WorkerEvent>,
) -> anyhow::Result<()> {
    for collection in services.collection_views()? {
        if watchers.contains_key(&collection.id)
            && !services.startup_preflight_blocks_automatic_scan(collection.id)?
        {
            request_scan(services, scheduler, background, events, collection.id);
        }
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct PeriodicScanSummary {
    watched: usize,
    scheduled: usize,
    already_pending: usize,
}

/// Schedules a safety reconciliation only for collections whose watcher is
/// currently healthy. Existing queued, active or dirty work is sufficient: a
/// periodic tick must not manufacture an otherwise unnecessary follow-up scan.
fn schedule_idle_scans(
    services: &Arc<DesktopServices>,
    watchers: &HashMap<Uuid, CollectionWatcherHandle>,
    scheduler: &mut ScanScheduler,
    background: &mut JoinSet<BackgroundCompletion>,
    events: &Sender<WorkerEvent>,
) -> anyhow::Result<PeriodicScanSummary> {
    let mut collection_ids = Vec::new();
    for collection in services.collection_views()? {
        if watchers.contains_key(&collection.id)
            && !services.startup_preflight_blocks_automatic_scan(collection.id)?
        {
            collection_ids.push(collection.id);
        }
    }
    collection_ids.sort_unstable();

    let mut summary = PeriodicScanSummary {
        watched: collection_ids.len(),
        ..PeriodicScanSummary::default()
    };
    for collection_id in collection_ids {
        let Some(ready) = scheduler.request_if_idle(collection_id) else {
            summary.already_pending += 1;
            continue;
        };
        if !ready.contains(&collection_id) {
            publish_scan_state(events, scheduler, collection_id);
        }
        spawn_ready_scans(services, background, events, ready);
        summary.scheduled += 1;
    }
    Ok(summary)
}

fn request_scan(
    services: &Arc<DesktopServices>,
    scheduler: &mut ScanScheduler,
    background: &mut JoinSet<BackgroundCompletion>,
    events: &Sender<WorkerEvent>,
    collection_id: Uuid,
) -> bool {
    let ready = scheduler.request(collection_id);
    let requested_started = ready.contains(&collection_id);
    if !requested_started {
        publish_scan_state(events, scheduler, collection_id);
    }
    spawn_ready_scans(services, background, events, ready);
    requested_started
}

fn publish_scan_state(
    events: &Sender<WorkerEvent>,
    scheduler: &ScanScheduler,
    collection_id: Uuid,
) {
    send(
        events,
        WorkerEvent::CollectionScan {
            collection_id,
            state: scheduler.state(collection_id),
        },
    );
}

fn spawn_ready_scans(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    events: &Sender<WorkerEvent>,
    ready: Vec<Uuid>,
) {
    for collection_id in ready {
        send(
            events,
            WorkerEvent::CollectionScan {
                collection_id,
                state: Some(CollectionScanState::Scanning),
            },
        );
        spawn_scan(services, background, collection_id);
    }
}

fn request_preflight(
    services: &Arc<DesktopServices>,
    scheduler: &mut ScanScheduler,
    background: &mut JoinSet<BackgroundCompletion>,
    collection_id: Uuid,
) {
    for ready in scheduler.request(collection_id) {
        spawn_preflight(services, background, ready);
    }
}

fn spawn_preflight(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    collection_id: Uuid,
) {
    let services = Arc::clone(services);
    background.spawn(async move {
        let preflight_services = Arc::clone(&services);
        let result = tokio::task::spawn_blocking(move || {
            preflight_services.preflight_collection(collection_id)
        })
        .await
        .map_err(|error| format!("falló el worker de prevalidación: {error}"))
        .and_then(|result| result.map_err(|error| format!("{error:#}")));
        if result.is_ok()
            && services
                .sync_public_collection(collection_id)
                .await
                .is_err()
        {
            tracing::warn!(
                error_kind = "public_manifest_sync_after_preflight",
                "public collection withdrawal could not be announced"
            );
        }
        BackgroundCompletion::Preflight {
            collection_id,
            result,
        }
    });
}

fn spawn_scan(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    collection_id: Uuid,
) {
    let services = Arc::clone(services);
    background.spawn(async move {
        let result = AssertUnwindSafe(services.scan_collection(collection_id))
            .catch_unwind()
            .await
            .map_err(panic_message)
            .and_then(|result| result.map_err(|error| format!("{error:#}")));
        BackgroundCompletion::Scan {
            collection_id,
            result,
        }
    });
}

fn spawn_review_reanalysis(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    concept_id: Uuid,
) {
    let services = Arc::clone(services);
    background.spawn(async move {
        let result = AssertUnwindSafe(services.reanalyze_review(concept_id))
            .catch_unwind()
            .await
            .map_err(panic_message)
            .and_then(|result| result.map_err(|error| format!("{error:#}")));
        BackgroundCompletion::ReanalyzeReview { concept_id, result }
    });
}

fn spawn_review_approval(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    concept_id: Uuid,
    expected_review_version: ReviewVersionToken,
    draft: EnrichmentDraft,
) {
    let services = Arc::clone(services);
    background.spawn(async move {
        let approval_services = Arc::clone(&services);
        let result = tokio::task::spawn_blocking(move || {
            approval_services.approve_review(concept_id, &expected_review_version, draft)
        })
        .await
        .map_err(|error| format!("falló el worker de publicación: {error}"))
        .and_then(|result| result.map_err(|error| format!("{error:#}")));
        let result = match result {
            Ok(collection_id) => {
                if services
                    .sync_public_collection(collection_id)
                    .await
                    .is_err()
                {
                    tracing::warn!(
                        error_kind = "public_manifest_sync_after_publication",
                        "published collection manifest could not be refreshed"
                    );
                }
                Ok(())
            }
            Err(error) => Err(error),
        };
        BackgroundCompletion::Approve { concept_id, result }
    });
}

fn spawn_review_evidence(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    request_id: Uuid,
    concept_id: Uuid,
    expected_source_revision: u32,
    expected_review_version: Option<ReviewVersionToken>,
    after_ordinal: Option<u32>,
) {
    let services = Arc::clone(services);
    background.spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            services.load_review_evidence(
                concept_id,
                expected_source_revision,
                expected_review_version.as_ref(),
                after_ordinal,
            )
        })
        .await
        .unwrap_or(Err(ReviewEvidenceErrorView::Unavailable));
        BackgroundCompletion::ReviewEvidence {
            request_id,
            concept_id,
            expected_source_revision,
            result,
        }
    });
}

fn knowledge_bundle_loaded_event(
    request_id: Uuid,
    collection_id: Uuid,
    result: Result<KnowledgeBundleView, String>,
) -> WorkerEvent {
    WorkerEvent::KnowledgeBundleLoaded {
        request_id,
        collection_id,
        result,
    }
}

fn knowledge_page_loaded_event(
    request_id: Uuid,
    collection_id: Uuid,
    page_id: KnowledgePageId,
    result: Result<KnowledgePageView, String>,
) -> WorkerEvent {
    WorkerEvent::KnowledgePageLoaded {
        request_id,
        collection_id,
        page_id,
        result,
    }
}

fn spawn_knowledge_bundle(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    request_id: Uuid,
    collection_id: Uuid,
) {
    let services = Arc::clone(services);
    background.spawn(async move {
        let result =
            tokio::task::spawn_blocking(move || services.load_knowledge_bundle(collection_id))
                .await
                .map_err(|error| format!("falló el worker del visor de conocimiento: {error}"))
                .and_then(|result| result.map_err(|error| format!("{error:#}")));
        BackgroundCompletion::KnowledgeBundle {
            request_id,
            collection_id,
            result,
        }
    });
}

fn spawn_knowledge_page(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    request_id: Uuid,
    collection_id: Uuid,
    page_id: KnowledgePageId,
    expected_fingerprint: String,
) {
    let services = Arc::clone(services);
    let completed_page_id = page_id;
    background.spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            services.load_knowledge_page(collection_id, page_id, &expected_fingerprint)
        })
        .await
        .map_err(|error| format!("falló el worker de lectura de página OKF: {error}"))
        .and_then(|result| result.map_err(|error| format!("{error:#}")));
        BackgroundCompletion::KnowledgePage {
            request_id,
            collection_id,
            page_id: completed_page_id,
            result,
        }
    });
}

struct SearchTask {
    request_id: Uuid,
    question: String,
    top_k: u8,
    purpose: SearchPurpose,
    public_network: bool,
}

fn spawn_search(
    services: &Arc<DesktopServices>,
    background: &mut JoinSet<BackgroundCompletion>,
    events: &Sender<WorkerEvent>,
    task: SearchTask,
) {
    let services = Arc::clone(services);
    let events = events.clone();
    background.spawn(async move {
        let SearchTask {
            request_id,
            question,
            top_k,
            purpose,
            public_network,
        } = task;
        let search = async {
            if public_network {
                let (partial_tx, mut partial_rx) = tokio::sync::mpsc::channel(4);
                let search = services.search_with_public(question, top_k, purpose, partial_tx);
                tokio::pin!(search);
                loop {
                    tokio::select! {
                        result = &mut search => break result,
                        partial = partial_rx.recv() => {
                            if let Some(partial) = partial {
                                send(&events, WorkerEvent::SearchPartial {
                                    request_id,
                                    hits: partial.hits,
                                });
                            }
                        }
                    }
                }
            } else {
                services.search(question, top_k, purpose).await
            }
        };
        let result = AssertUnwindSafe(search)
            .catch_unwind()
            .await
            .map_err(panic_message)
            .and_then(|result| result.map_err(|error| error.to_string()));
        let route_kind = if public_network {
            services.public_route_kind()
        } else {
            PublicRouteKind::Offline
        };
        BackgroundCompletion::Search {
            request_id,
            result,
            route_kind,
        }
    });
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        format!("panic en tarea de fondo: {message}")
    } else if let Some(message) = payload.downcast_ref::<String>() {
        format!("panic en tarea de fondo: {message}")
    } else {
        "panic en tarea de fondo".into()
    }
}

fn manual_rescan_summary(outcomes: &[IngestOutcome]) -> String {
    let mut analyzed = 0_usize;
    let mut unchanged = 0_usize;
    let mut renamed = 0_usize;
    let mut deleted = 0_usize;
    let mut failed = 0_usize;
    for outcome in outcomes {
        match outcome {
            IngestOutcome::NeedsReview { .. } => analyzed += 1,
            IngestOutcome::Unchanged { .. } => unchanged += 1,
            IngestOutcome::Renamed { .. } => renamed += 1,
            IngestOutcome::Deleted { .. } => deleted += 1,
            IngestOutcome::Failed { .. } => failed += 1,
        }
    }
    format!(
        "Reescaneo completado: {analyzed} analizado(s), {unchanged} sin cambios, \
         {renamed} renombrado(s), {deleted} eliminado(s), {failed} con error"
    )
}

fn take_manual_rescan_summary(
    manual_rescans: &mut HashSet<Uuid>,
    scheduler: &ScanScheduler,
    collection_id: Uuid,
    summary: Option<String>,
) -> Option<String> {
    if scheduler.state(collection_id).is_some() || !manual_rescans.remove(&collection_id) {
        return None;
    }
    summary
}

fn clear_manual_rescan(manual_rescans: &mut HashSet<Uuid>, collection_id: Uuid) {
    manual_rescans.remove(&collection_id);
}

fn report_ingest_outcomes(outcomes: &[IngestOutcome], events: &Sender<WorkerEvent>) {
    let mut awaiting_review = 0_usize;
    for outcome in outcomes {
        match outcome {
            IngestOutcome::Failed { .. } => {}
            IngestOutcome::NeedsReview { .. } => awaiting_review += 1,
            IngestOutcome::Unchanged { .. }
            | IngestOutcome::Renamed { .. }
            | IngestOutcome::Deleted { .. } => {}
        }
    }
    if awaiting_review > 0 {
        send(
            events,
            WorkerEvent::Notice(format!(
                "{awaiting_review} documento(s) quedaron listos para revisión humana"
            )),
        );
    }
}

fn send(events: &Sender<WorkerEvent>, event: WorkerEvent) {
    let _ = events.send(event);
}

fn search_coverage_view(response: &SearchResponse) -> SearchCoverageView {
    let offline_count = response.offline_nodes.iter().collect::<HashSet<_>>().len();
    if offline_count > 0 {
        return SearchCoverageView::OfflineDevices {
            count: offline_count,
        };
    }
    if response.warnings.len() == 1 && response.warnings[0] == "federation_disabled" {
        return SearchCoverageView::FederationDisabled;
    }
    if response.partial || !response.warnings.is_empty() {
        SearchCoverageView::Partial
    } else {
        SearchCoverageView::Complete
    }
}

fn mcp_activity_is_recent(observed_at: SystemTime, now: SystemTime) -> bool {
    now.duration_since(observed_at)
        .is_ok_and(|age| age <= MCP_CLIENT_ACTIVITY_RECENT)
}

fn apply_recent_mcp_activities(
    integrations: &mut [IntegrationView],
    activities: impl IntoIterator<Item = McpClientActivity>,
    now: SystemTime,
) -> bool {
    let mut claude_activity_recent = false;
    for activity in activities {
        if !mcp_activity_is_recent(activity.observed_at, now) {
            continue;
        }
        let client = match activity.client {
            McpClientKind::ChatGptDesktop => ChatClientKind::ChatGptDesktop,
            McpClientKind::ClaudeDesktop => ChatClientKind::ClaudeDesktop,
            McpClientKind::GeminiCli => ChatClientKind::GeminiCli,
        };
        if let Some(view) = integrations.iter_mut().find(|view| view.client == client) {
            view.activity_recent = true;
            claude_activity_recent |= client == ChatClientKind::ClaudeDesktop;
        }
    }
    claude_activity_recent
}

fn update_claude_approval_after_action(state: &mut ClaudeApprovalState, action: IntegrationAction) {
    match action {
        IntegrationAction::Connect(ChatClientKind::ClaudeDesktop) => {
            *state = ClaudeApprovalState::Awaiting;
        }
        IntegrationAction::ConfirmClaudeInstalled => {
            *state = ClaudeApprovalState::Confirmed;
        }
        IntegrationAction::Refresh
        | IntegrationAction::Connect(_)
        | IntegrationAction::Disconnect(_)
        | IntegrationAction::OpenClaudeSettings => {}
    }
}

fn apply_claude_approval_state(
    integrations: &mut [IntegrationView],
    state: &mut ClaudeApprovalState,
    activity_recent: bool,
) {
    if activity_recent {
        *state = ClaudeApprovalState::Confirmed;
    }
    let Some(claude) = integrations
        .iter_mut()
        .find(|view| view.client == ChatClientKind::ClaudeDesktop)
    else {
        return;
    };
    match state {
        ClaudeApprovalState::NotRequested => {}
        ClaudeApprovalState::Awaiting => {
            claude.status = IntegrationStatus::AwaitingClientApproval;
            claude.detail =
                "Completa la aprobación en Claude o confirma aquí cuando termine.".to_owned();
        }
        ClaudeApprovalState::Confirmed => {
            claude.status = IntegrationStatus::Configured;
            claude.detail = if activity_recent {
                "Claude utilizó recientemente el puente MCP local.".to_owned()
            } else {
                "Instalación confirmada por el usuario para esta sesión; no es una señal de autenticación."
                    .to_owned()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_health_summary_keeps_the_completed_check_time() {
        let checked_at = SystemTime::UNIX_EPOCH + Duration::from_secs(42);

        assert_eq!(
            wiki_health_summary(
                WikiHealthRollup {
                    error_count: 3,
                    warning_count: 2,
                    updating_count: 1,
                    attention_collection_id: Some(Uuid::nil()),
                },
                checked_at,
            ),
            WikiHealthSummaryView {
                error_count: 3,
                warning_count: 2,
                updating_count: 1,
                attention_collection_id: Some(Uuid::nil()),
                checked_at: Some(checked_at),
            }
        );
    }

    #[test]
    fn review_evidence_debug_redacts_content_and_identity() {
        let concept_id = Uuid::new_v4();
        let secret = "DO-NOT-LOG-REVIEW-EVIDENCE";
        let page = ReviewEvidencePageView {
            concept_id,
            source_revision: 2,
            review_version: ReviewVersionToken::from_digest([11; 32]),
            excerpts: vec![ReviewEvidenceExcerptView {
                ordinal: 0,
                heading_or_page: secret.to_owned(),
                text: secret.to_owned(),
                truncated: false,
            }],
            total_chunks: 1,
            next_ordinal: None,
        };

        let debug = format!("{page:?}");

        assert!(!debug.contains(secret));
        assert!(!debug.contains(&concept_id.to_string()));
        assert!(debug.contains("excerpt_count"));
    }

    fn tracked_firewall_operation() -> FirewallOperationTracker {
        FirewallOperationTracker {
            request_id: Uuid::new_v4(),
            started_at: Instant::now(),
            slow_notice_sent: false,
        }
    }

    #[test]
    fn firewall_slow_notice_fires_once_at_thirty_seconds() {
        assert!(!slow_notice_is_due(Duration::from_secs(29), false));
        assert!(slow_notice_is_due(Duration::from_secs(30), false));
        assert!(!slow_notice_is_due(Duration::from_secs(31), true));
    }

    #[test]
    fn tracked_firewall_operation_rejects_configure_remove_and_refresh() {
        let tracker = tracked_firewall_operation();
        assert!(firewall_request_is_busy(None, Some(tracker)));
        assert!(firewall_request_is_busy(Some(Uuid::new_v4()), None,));
    }

    #[test]
    fn periodic_connectivity_tick_is_suppressed_while_firewall_is_tracked() {
        assert!(firewall_request_is_busy(
            None,
            Some(tracked_firewall_operation()),
        ));
    }

    #[test]
    fn firewall_and_update_install_overlap_is_blocked_in_both_orders() {
        assert!(firewall_update_overlap_is_busy(
            Some(tracked_firewall_operation()),
            None,
        ));
        assert!(firewall_update_overlap_is_busy(None, Some(Uuid::new_v4()),));
        assert!(!firewall_update_overlap_is_busy(None, None));
    }

    #[test]
    fn only_matching_firewall_completion_is_authoritative() {
        let tracker = tracked_firewall_operation();

        assert!(firewall_completion_is_authoritative(
            Some(tracker),
            tracker.request_id,
        ));
        assert!(!firewall_completion_is_authoritative(
            Some(tracker),
            Uuid::new_v4(),
        ));
        assert!(!firewall_completion_is_authoritative(
            None,
            tracker.request_id,
        ));
    }

    fn firewall_snapshot(
        network_profile: NetworkProfileState,
        firewall: FirewallDiagnosticState,
        firewall_helper: FirewallHelperState,
    ) -> ConnectivityPlatformSnapshot {
        ConnectivityPlatformSnapshot {
            system_permission: crate::connectivity_platform::SystemPermissionState::NotApplicable,
            network_profile,
            firewall,
            firewall_helper,
        }
    }

    #[test]
    fn firewall_preflight_revalidates_profile_rules_and_helper_before_elevation() {
        let configure = firewall_snapshot(
            NetworkProfileState::Private,
            FirewallDiagnosticState::RulesMissing,
            FirewallHelperState::Verified,
        );
        assert_eq!(
            firewall_install_preflight(configure),
            Ok(FirewallInstallDecision::Configure)
        );

        let already_ready = firewall_snapshot(
            NetworkProfileState::Domain,
            FirewallDiagnosticState::Ready,
            FirewallHelperState::Verified,
        );
        assert_eq!(
            firewall_install_preflight(already_ready),
            Ok(FirewallInstallDecision::AlreadyReady)
        );

        for stale in [
            firewall_snapshot(
                NetworkProfileState::Public,
                FirewallDiagnosticState::RulesMissing,
                FirewallHelperState::Verified,
            ),
            firewall_snapshot(
                NetworkProfileState::Private,
                FirewallDiagnosticState::RulesMissing,
                FirewallHelperState::Untrusted,
            ),
            firewall_snapshot(
                NetworkProfileState::Private,
                FirewallDiagnosticState::LegacyExposure,
                FirewallHelperState::Verified,
            ),
        ] {
            assert_eq!(
                firewall_install_preflight(stale),
                Err(FirewallActionError::StateChanged)
            );
        }
    }

    #[test]
    fn firewall_action_errors_cross_the_worker_boundary_as_stable_codes() {
        let cases = [
            (
                FirewallActionError::Cancelled,
                ConnectivityIssueCode::FirewallCancelled,
            ),
            (
                FirewallActionError::ManagedPolicy,
                ConnectivityIssueCode::FirewallManagedPolicy,
            ),
            (
                FirewallActionError::InboundBlocked,
                ConnectivityIssueCode::FirewallInboundBlocked,
            ),
            (
                FirewallActionError::Conflict,
                ConnectivityIssueCode::FirewallConflict,
            ),
            (
                FirewallActionError::InvalidLayoutOrSignature,
                ConnectivityIssueCode::FirewallInstallationInvalid,
            ),
            (
                FirewallActionError::Unsupported,
                ConnectivityIssueCode::FirewallUnsupported,
            ),
            (
                FirewallActionError::StateChanged,
                ConnectivityIssueCode::FirewallStateChanged,
            ),
            (
                FirewallActionError::Internal,
                ConnectivityIssueCode::FirewallInternal,
            ),
        ];

        for (error, code) in cases {
            assert_eq!(ConnectivityIssueCode::from(error), code);
        }
    }

    #[test]
    fn search_coverage_reduces_backend_diagnostics_without_peer_ids() {
        let peer = "12D3KooWGyQTCFum8387gase2VQ5RoE4nmNbDcckPAPqUimoimNn";
        let mut response = SearchResponse::empty(Uuid::new_v4());
        response.partial = true;
        response.offline_nodes = vec![peer.to_owned(), peer.to_owned()];
        response
            .warnings
            .push(format!("peer {peer}: remote search unavailable"));

        assert_eq!(
            search_coverage_view(&response),
            SearchCoverageView::OfflineDevices { count: 1 }
        );
    }

    #[test]
    fn search_coverage_recognizes_disabled_federation_as_a_typed_state() {
        let mut response = SearchResponse::empty(Uuid::new_v4());
        response.partial = true;
        response.warnings.push("federation_disabled".to_owned());

        assert_eq!(
            search_coverage_view(&response),
            SearchCoverageView::FederationDisabled
        );

        response.warnings.push("another backend gap".to_owned());
        assert_eq!(search_coverage_view(&response), SearchCoverageView::Partial);
    }

    #[test]
    fn lan_runtime_dto_preserves_only_explicit_advanced_fallback_addresses() {
        let (events, receiver) = mpsc::channel();
        let address = "/ip4/192.168.1.25/tcp/61743/p2p/test".to_owned();

        send_lan_runtime(
            &events,
            LanListenerView::Listening,
            LanDiscoveryView::Active,
            std::slice::from_ref(&address),
        );

        match receiver.recv().unwrap() {
            WorkerEvent::LanRuntimeUpdated {
                listener,
                discovery,
                local_addresses,
                ..
            } => {
                assert_eq!(listener, LanListenerView::Listening);
                assert_eq!(discovery, LanDiscoveryView::Active);
                assert_eq!(local_addresses, [address]);
            }
            event => panic!("unexpected event: {event:?}"),
        }
    }

    #[test]
    fn stale_lan_address_resolution_is_rejected_after_restart_invalidation() {
        let listener: airwiki_network::Multiaddr = "/ip4/192.168.1.25/tcp/61743".parse().unwrap();
        let mut current_listener = Some(listener.clone());
        let mut local_addresses = vec!["/ip4/192.168.1.25/tcp/61743/p2p/test".to_owned()];
        assert!(lan_address_resolution_is_current(
            true,
            current_listener.as_ref(),
            &listener,
            true,
        ));

        invalidate_lan_address_resolution(&mut current_listener, &mut local_addresses);

        assert!(current_listener.is_none());
        assert!(local_addresses.is_empty());
        assert!(!lan_address_resolution_is_current(
            true,
            current_listener.as_ref(),
            &listener,
            true,
        ));
    }

    #[test]
    fn worker_join_reports_completion_after_runtime_stops() {
        let (finished_tx, finished_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let _ = finished_tx.send(());
        });

        assert_eq!(
            join_worker_with_timeout(thread, &finished_rx, Duration::from_secs(1)),
            WorkerJoinOutcome::Joined
        );
    }

    #[test]
    fn worker_join_timeout_detaches_a_blocked_thread() {
        let (finished_tx, finished_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (exited_tx, exited_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let _ = release_rx.recv();
            drop(finished_tx);
            let _ = exited_tx.send(());
        });

        assert_eq!(
            join_worker_with_timeout(thread, &finished_rx, Duration::ZERO),
            WorkerJoinOutcome::TimedOut
        );
        assert!(release_tx.send(()).is_ok());
        assert_eq!(exited_rx.recv_timeout(Duration::from_secs(1)), Ok(()));
    }

    #[test]
    fn model_state_request_is_current_only_for_latest_reserved_sequence() {
        assert!(model_state_request_is_current(7, 8));
        assert!(!model_state_request_is_current(7, 7));
        assert!(!model_state_request_is_current(7, 9));
        assert!(model_state_request_is_current(u64::MAX, 0));
    }

    #[test]
    fn only_fresh_model_config_schedules_state_before_service_startup() {
        assert!(should_schedule_initial_model_state(
            &DesktopConfig::default()
        ));

        let active = DesktopConfig {
            active_selection: Some("qwen3-1.7b-q8".into()),
            ..DesktopConfig::default()
        };
        assert!(!should_schedule_initial_model_state(&active));

        let pending = DesktopConfig {
            pending_selection: Some("gemma-4-e4b-q4".into()),
            ..DesktopConfig::default()
        };
        assert!(!should_schedule_initial_model_state(&pending));
    }

    #[test]
    fn mcp_activity_is_recent_only_inside_the_diagnostic_window() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);

        assert!(mcp_activity_is_recent(
            now - MCP_CLIENT_ACTIVITY_RECENT,
            now
        ));
        assert!(!mcp_activity_is_recent(
            now - MCP_CLIENT_ACTIVITY_RECENT - Duration::from_secs(1),
            now
        ));
        assert!(!mcp_activity_is_recent(now + Duration::from_secs(1), now));
    }

    fn integration_view(client: ChatClientKind) -> IntegrationView {
        IntegrationView {
            client,
            status: IntegrationStatus::Available,
            detected_version: None,
            detail: "available".to_owned(),
            planned_path: None,
            activity_recent: false,
            restart_required: false,
        }
    }

    fn claude_view() -> IntegrationView {
        integration_view(ChatClientKind::ClaudeDesktop)
    }

    #[test]
    fn per_client_activity_marks_every_recent_view_without_old_entries_clearing_it() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let recent = now - Duration::from_secs(1);
        let old = now - MCP_CLIENT_ACTIVITY_RECENT - Duration::from_secs(1);
        let mut all_recent = ChatClientKind::ALL
            .into_iter()
            .map(integration_view)
            .collect::<Vec<_>>();
        let activities = [
            McpClientActivity {
                client: McpClientKind::ChatGptDesktop,
                observed_at: recent,
            },
            McpClientActivity {
                client: McpClientKind::ClaudeDesktop,
                observed_at: recent,
            },
            McpClientActivity {
                client: McpClientKind::GeminiCli,
                observed_at: recent,
            },
        ];

        let claude_recent = apply_recent_mcp_activities(&mut all_recent, activities, now);

        assert!(claude_recent);
        assert!(all_recent.iter().all(|view| view.activity_recent));

        let mut mixed = ChatClientKind::ALL
            .into_iter()
            .map(integration_view)
            .collect::<Vec<_>>();
        let mixed_activities = [
            McpClientActivity {
                client: McpClientKind::ChatGptDesktop,
                observed_at: recent,
            },
            McpClientActivity {
                client: McpClientKind::ChatGptDesktop,
                observed_at: old,
            },
            McpClientActivity {
                client: McpClientKind::ClaudeDesktop,
                observed_at: old,
            },
            McpClientActivity {
                client: McpClientKind::GeminiCli,
                observed_at: recent,
            },
        ];

        let claude_recent = apply_recent_mcp_activities(&mut mixed, mixed_activities, now);

        assert!(!claude_recent);
        assert!(mixed[0].activity_recent);
        assert!(!mixed[1].activity_recent);
        assert!(mixed[2].activity_recent);
    }

    #[test]
    fn claude_awaiting_state_survives_refreshes() {
        let mut state = ClaudeApprovalState::NotRequested;
        update_claude_approval_after_action(
            &mut state,
            IntegrationAction::Connect(ChatClientKind::ClaudeDesktop),
        );
        let mut first = vec![claude_view()];
        apply_claude_approval_state(&mut first, &mut state, false);
        let mut refreshed = vec![claude_view()];
        apply_claude_approval_state(&mut refreshed, &mut state, false);

        assert_eq!(state, ClaudeApprovalState::Awaiting);
        assert_eq!(
            refreshed[0].status,
            IntegrationStatus::AwaitingClientApproval
        );
    }

    #[test]
    fn explicit_claude_confirmation_is_session_only_configuration_state() {
        let mut state = ClaudeApprovalState::Awaiting;
        update_claude_approval_after_action(&mut state, IntegrationAction::ConfirmClaudeInstalled);
        let mut views = vec![claude_view()];
        apply_claude_approval_state(&mut views, &mut state, false);

        assert_eq!(state, ClaudeApprovalState::Confirmed);
        assert_eq!(views[0].status, IntegrationStatus::Configured);
        assert!(views[0].detail.contains("no es una señal de autenticación"));
    }

    #[test]
    fn recent_claude_activity_confirms_without_user_override() {
        let mut state = ClaudeApprovalState::Awaiting;
        let mut views = vec![claude_view()];

        apply_claude_approval_state(&mut views, &mut state, true);

        assert_eq!(state, ClaudeApprovalState::Confirmed);
        assert_eq!(views[0].status, IntegrationStatus::Configured);
    }

    #[test]
    fn knowledge_results_preserve_request_and_resource_identity_inline() {
        let request_id = Uuid::new_v4();
        let collection_id = Uuid::new_v4();
        match knowledge_bundle_loaded_event(request_id, collection_id, Err("bundle error".into())) {
            WorkerEvent::KnowledgeBundleLoaded {
                request_id: actual_request,
                collection_id: actual_collection,
                result,
            } => {
                assert_eq!(actual_request, request_id);
                assert_eq!(actual_collection, collection_id);
                assert_eq!(result.unwrap_err(), "bundle error");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let page_id = KnowledgePageId::Concept(Uuid::new_v4());
        match knowledge_page_loaded_event(
            request_id,
            collection_id,
            page_id,
            Err("page error".into()),
        ) {
            WorkerEvent::KnowledgePageLoaded {
                request_id: actual_request,
                collection_id: actual_collection,
                page_id: actual_page,
                result,
            } => {
                assert_eq!(actual_request, request_id);
                assert_eq!(actual_collection, collection_id);
                assert_eq!(actual_page, page_id);
                assert_eq!(result.unwrap_err(), "page error");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn guided_repair_failures_expose_only_stable_sanitized_codes() {
        let history = anyhow::Error::new(WikiRepairError::HistoryRepairRequiresHumanRecovery);
        assert_eq!(
            guided_repair_error_code(&history),
            "wiki_repair_history_requires_human"
        );

        let rollback = anyhow::Error::new(WikiRepairError::RollbackFailed {
            cause: "/private/source/path.pdf".to_owned(),
            rollback: "secret document title".to_owned(),
        });
        let code = guided_repair_error_code(&rollback);
        assert_eq!(code, "wiki_repair_rollback_failed");
        assert!(!code.contains("private"));
        assert!(!code.contains("secret"));
    }

    #[test]
    fn manual_rescan_summary_counts_every_outcome_without_document_content() {
        let source = Uuid::new_v4();
        let outcomes = vec![
            IngestOutcome::NeedsReview {
                source_document_id: source,
                concept_id: Uuid::new_v4(),
                used_fallback_metadata: false,
            },
            IngestOutcome::Unchanged {
                source_document_id: source,
            },
            IngestOutcome::Renamed {
                source_document_id: source,
            },
            IngestOutcome::Deleted {
                source_document_id: source,
            },
            IngestOutcome::Failed {
                source_document_id: Some(source),
                path: PathBuf::from("private-name.pdf"),
                code: airwiki_core::SourceIssueCode::ProcessingFailed,
                error: "private failure".into(),
            },
        ];

        let summary = manual_rescan_summary(&outcomes);
        assert_eq!(
            summary,
            "Reescaneo completado: 1 analizado(s), 1 sin cambios, 1 renombrado(s), 1 eliminado(s), 1 con error"
        );
        assert!(!summary.contains("private-name"));
        assert!(!summary.contains("private failure"));
    }

    #[test]
    fn ingest_failure_is_presented_by_the_typed_issue_list_not_a_raw_notice() {
        let outcomes = vec![IngestOutcome::Failed {
            source_document_id: None,
            path: PathBuf::from("/private/customer/secret-report.pdf"),
            code: airwiki_core::SourceIssueCode::InvalidPdf,
            error: "parser failed on customer secret".into(),
        }];
        let (sender, receiver) = std::sync::mpsc::channel();

        report_ingest_outcomes(&outcomes, &sender);

        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn manual_confirmation_waits_for_the_coalesced_final_scan_and_is_sent_once() {
        let collection = Uuid::new_v4();
        let mut scheduler = ScanScheduler::new(1);
        let mut manual_rescans = HashSet::from([collection]);
        assert_eq!(scheduler.request(collection), vec![collection]);
        assert!(scheduler.request(collection).is_empty());

        assert_eq!(scheduler.finish(collection), vec![collection]);
        assert_eq!(
            take_manual_rescan_summary(
                &mut manual_rescans,
                &scheduler,
                collection,
                Some("prematuro".into()),
            ),
            None
        );
        assert!(manual_rescans.contains(&collection));

        assert!(scheduler.finish(collection).is_empty());
        assert_eq!(
            take_manual_rescan_summary(
                &mut manual_rescans,
                &scheduler,
                collection,
                Some("final".into()),
            ),
            Some("final".into())
        );
        assert!(!manual_rescans.contains(&collection));
        assert_eq!(
            take_manual_rescan_summary(
                &mut manual_rescans,
                &scheduler,
                collection,
                Some("duplicado".into()),
            ),
            None
        );
    }

    #[test]
    fn automatic_scan_never_produces_a_manual_confirmation() {
        let collection = Uuid::new_v4();
        let mut scheduler = ScanScheduler::new(1);
        let mut manual_rescans = HashSet::new();
        assert_eq!(scheduler.request(collection), vec![collection]);
        assert!(scheduler.finish(collection).is_empty());
        assert_eq!(
            take_manual_rescan_summary(
                &mut manual_rescans,
                &scheduler,
                collection,
                Some("automático".into()),
            ),
            None
        );
    }

    #[test]
    fn cancellation_or_error_clears_the_pending_manual_confirmation() {
        let collection = Uuid::new_v4();
        let mut scheduler = ScanScheduler::new(1);
        let mut manual_rescans = HashSet::from([collection]);
        assert_eq!(scheduler.request(collection), vec![collection]);

        clear_manual_rescan(&mut manual_rescans, collection);
        assert!(!manual_rescans.contains(&collection));
        assert!(scheduler.finish(collection).is_empty());
        assert_eq!(
            take_manual_rescan_summary(
                &mut manual_rescans,
                &scheduler,
                collection,
                Some("no debe mostrarse".into()),
            ),
            None
        );
    }

    #[test]
    fn periodic_tick_has_a_stable_small_initial_jitter() {
        let first = periodic_reconcile_first_delay("peer-mac");
        assert_eq!(first, periodic_reconcile_first_delay("peer-mac"));
        assert!(first >= PERIODIC_RECONCILE_INTERVAL);
        assert!(first <= PERIODIC_RECONCILE_INTERVAL + PERIODIC_RECONCILE_MAX_JITTER);
        assert_ne!(first, periodic_reconcile_first_delay("peer-windows"));
        assert_eq!(
            periodic_reconcile_jitter("peer-mac", Duration::ZERO),
            Duration::ZERO
        );
    }

    #[test]
    fn periodic_tick_does_not_duplicate_active_or_queued_scans() {
        let active = Uuid::new_v4();
        let queued = Uuid::new_v4();
        let mut scheduler = ScanScheduler::new(1);
        assert_eq!(scheduler.request(active), vec![active]);
        assert!(scheduler.request(queued).is_empty());

        assert_eq!(scheduler.request_if_idle(active), None);
        assert_eq!(scheduler.request_if_idle(queued), None);

        assert_eq!(scheduler.finish(active), vec![queued]);
        assert!(scheduler.finish(queued).is_empty());
        assert_eq!(scheduler.state(active), None);
        assert_eq!(scheduler.state(queued), None);
    }

    #[test]
    fn repeated_periodic_ticks_queue_an_idle_collection_once() {
        let active = Uuid::new_v4();
        let periodic = Uuid::new_v4();
        let mut scheduler = ScanScheduler::new(1);
        assert_eq!(scheduler.request(active), vec![active]);

        assert_eq!(scheduler.request_if_idle(periodic), Some(Vec::new()));
        assert_eq!(scheduler.state(periodic), Some(CollectionScanState::Queued));
        assert_eq!(scheduler.request_if_idle(periodic), None);

        assert_eq!(scheduler.finish(active), vec![periodic]);
        assert!(scheduler.finish(periodic).is_empty());
    }

    #[test]
    fn duplicate_active_scan_coalesces_to_one_follow_up() {
        let collection = Uuid::new_v4();
        let mut scheduler = ScanScheduler::new(1);
        assert_eq!(scheduler.request(collection), vec![collection]);
        assert!(scheduler.request(collection).is_empty());
        assert!(scheduler.request(collection).is_empty());
        assert_eq!(scheduler.finish(collection), vec![collection]);
        assert!(scheduler.finish(collection).is_empty());
    }

    #[test]
    fn queued_collections_are_fair_and_do_not_duplicate() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let third = Uuid::new_v4();
        let mut scheduler = ScanScheduler::new(1);
        assert_eq!(scheduler.request(first), vec![first]);
        assert_eq!(scheduler.state(first), Some(CollectionScanState::Scanning));
        assert!(scheduler.request(second).is_empty());
        assert_eq!(scheduler.state(second), Some(CollectionScanState::Queued));
        assert!(scheduler.request(second).is_empty());
        assert!(scheduler.request(third).is_empty());
        assert_eq!(scheduler.finish(first), vec![second]);
        assert_eq!(scheduler.state(first), None);
        assert_eq!(scheduler.state(second), Some(CollectionScanState::Scanning));
        assert_eq!(scheduler.state(third), Some(CollectionScanState::Queued));
        assert_eq!(scheduler.finish(second), vec![third]);
        assert_eq!(scheduler.state(second), None);
        assert_eq!(scheduler.state(third), Some(CollectionScanState::Scanning));
        assert!(scheduler.finish(third).is_empty());
        assert_eq!(scheduler.state(third), None);
    }

    #[test]
    fn dirty_scan_requeues_behind_waiting_collection() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut scheduler = ScanScheduler::new(1);
        assert_eq!(scheduler.request(first), vec![first]);
        assert!(scheduler.request(second).is_empty());
        assert!(scheduler.request(first).is_empty());
        assert_eq!(scheduler.state(first), Some(CollectionScanState::Scanning));
        assert_eq!(scheduler.state(second), Some(CollectionScanState::Queued));
        assert_eq!(scheduler.finish(first), vec![second]);
        assert_eq!(scheduler.state(first), Some(CollectionScanState::Queued));
        assert_eq!(scheduler.state(second), Some(CollectionScanState::Scanning));
        assert_eq!(scheduler.finish(second), vec![first]);
        assert_eq!(scheduler.state(first), Some(CollectionScanState::Scanning));
        assert_eq!(scheduler.state(second), None);
        assert!(scheduler.finish(first).is_empty());
        assert_eq!(scheduler.state(first), None);
    }

    #[test]
    fn cancelling_collection_discards_its_dirty_follow_up_and_releases_slot() {
        let failed_watcher = Uuid::new_v4();
        let healthy = Uuid::new_v4();
        let mut scheduler = ScanScheduler::new(1);
        assert_eq!(scheduler.request(failed_watcher), vec![failed_watcher]);
        assert!(scheduler.request(failed_watcher).is_empty());
        assert!(scheduler.request(healthy).is_empty());

        assert_eq!(scheduler.cancel(failed_watcher), vec![healthy]);
        assert_eq!(scheduler.state(failed_watcher), None);
        assert_eq!(
            scheduler.state(healthy),
            Some(CollectionScanState::Scanning)
        );
        assert!(scheduler.finish(failed_watcher).is_empty());
        assert!(scheduler.finish(healthy).is_empty());
        assert_eq!(scheduler.state(healthy), None);
    }

    #[test]
    fn profile_changes_wait_for_model_operations() {
        assert!(can_change_model_profile(ModelLifecycle::Missing));
        assert!(can_change_model_profile(ModelLifecycle::Ready));
        assert!(!can_change_model_profile(ModelLifecycle::Verifying));
        assert!(!can_change_model_profile(ModelLifecycle::Installing));
        assert!(!can_change_model_profile(ModelLifecycle::Enabling));
    }

    #[test]
    fn changing_profile_clears_only_a_mismatched_pending_activation() {
        let config = DesktopConfig {
            profile: ModelProfile::Automatic,
            active_selection: Some("qwen3-1.7b-q8".into()),
            pending_selection: Some("gemma-4-e4b-q4".into()),
            ..DesktopConfig::default()
        };
        let efficient_decision = ModelDecision {
            selection: selection_for_model(ModelProfile::Efficient, "gemma-4-e2b-q4", "test"),
            issues: Vec::new(),
        };
        let automatic_decision = ModelDecision {
            selection: selection_for_model(ModelProfile::Automatic, "gemma-4-e4b-q4", "test"),
            issues: Vec::new(),
        };

        let (efficient, cleared) =
            config_with_profile(&config, ModelProfile::Efficient, &efficient_decision);
        assert!(cleared);
        assert_eq!(efficient.profile, ModelProfile::Efficient);
        assert!(efficient.pending_selection.is_none());
        assert_eq!(efficient.active_selection, config.active_selection);

        let (automatic, cleared) =
            config_with_profile(&efficient, ModelProfile::Automatic, &automatic_decision);
        assert!(!cleared);
        assert_eq!(automatic.profile, ModelProfile::Automatic);
        assert!(automatic.pending_selection.is_none());

        let (quality, cleared) =
            config_with_profile(&config, ModelProfile::Quality, &automatic_decision);
        assert!(!cleared);
        assert_eq!(quality.pending_selection, config.pending_selection);
    }

    #[test]
    fn a_licensed_downloaded_recommendation_is_eligible_for_pending_recovery() {
        let decision = ModelDecision {
            selection: selection_for_model(ModelProfile::Automatic, "gemma-4-e4b-q4", "test"),
            issues: Vec::new(),
        };
        let selection = decision.selection.as_ref().unwrap();
        let mut config = DesktopConfig {
            active_selection: Some("qwen3-1.7b-q8".into()),
            ..DesktopConfig::default()
        };

        assert!(!should_probe_profile_activation(&config, &decision));
        accept_selection_licenses(&mut config, selection);
        assert!(should_probe_profile_activation(&config, &decision));

        config.pending_selection = Some(selection.model_id.into());
        assert!(!should_probe_profile_activation(&config, &decision));
        config.pending_selection = None;
        config.active_selection = Some(selection.model_id.into());
        assert!(!should_probe_profile_activation(&config, &decision));
    }

    #[test]
    fn model_state_reuses_only_a_verified_plan_for_the_recommendation() {
        let recommendation = ModelDecision {
            selection: selection_for_model(ModelProfile::Automatic, "gemma-4-e4b-q4", "test"),
            issues: Vec::new(),
        };
        let matching = recommendation.selection.as_ref().unwrap().clone();
        let matching_plan = InstallPlan {
            selection: matching,
            artifact_ids: Vec::new(),
            download_bytes: 0,
            required_free_bytes: airwiki_inference::INSTALL_HEADROOM_BYTES,
            fits_available_disk: true,
        };

        assert!(matching_known_install_plan(Some(matching_plan), &recommendation).is_some());

        let different_plan = InstallPlan {
            selection: ModelSelection::legacy_qwen(),
            artifact_ids: Vec::new(),
            download_bytes: 0,
            required_free_bytes: airwiki_inference::INSTALL_HEADROOM_BYTES,
            fits_available_disk: true,
        };
        assert!(matching_known_install_plan(Some(different_plan), &recommendation).is_none());
    }

    #[tokio::test]
    async fn verified_model_state_does_not_reinspect_artifact_paths() {
        let temp = tempfile::tempdir().unwrap();
        let manager = AssetManager::new(temp.path())
            .unwrap()
            .with_bundled_runtime(Some(temp.path().join("missing-llama-server")));
        let recommendation = ModelDecision {
            selection: selection_for_model(ModelProfile::Automatic, "gemma-4-e4b-q4", "test"),
            issues: Vec::new(),
        };
        let plan = InstallPlan {
            selection: recommendation.selection.as_ref().unwrap().clone(),
            artifact_ids: Vec::new(),
            download_bytes: 0,
            required_free_bytes: airwiki_inference::INSTALL_HEADROOM_BYTES,
            fits_available_disk: true,
        };
        let (events, receiver) = mpsc::channel();
        let mut lifecycle = JoinSet::new();

        send_model_state_with_known_plan(
            &mut lifecycle,
            &events,
            &manager,
            &DesktopConfig::default(),
            &recommendation,
            Some(plan),
        );

        let event = tokio::task::spawn_blocking(move || {
            receiver.recv_timeout(Duration::from_secs(2)).unwrap()
        })
        .await
        .unwrap();
        let WorkerEvent::ModelState(state) = event else {
            panic!("expected a model-state event");
        };
        assert!(state.recommended_assets_installed);
        assert!(state.issues.is_empty());
        assert_eq!(state.download_bytes, 0);
    }

    #[test]
    fn legacy_model_only_license_acceptance_does_not_cover_auxiliary_snapshots() {
        let decision = ModelDecision {
            selection: selection_for_model(ModelProfile::Automatic, "gemma-4-e4b-q4", "test"),
            issues: Vec::new(),
        };
        let selection = decision.selection.as_ref().unwrap();
        let mut config = DesktopConfig::default();
        config.accept_license(
            selection.model_id,
            selection.manifest.artifact.revision,
            selection.manifest.artifact.license,
        );

        assert!(!selection_licenses_accepted(&config, selection));
        accept_selection_licenses(&mut config, selection);
        assert!(selection_licenses_accepted(&config, selection));
    }

    #[test]
    fn a_different_installed_model_is_staged_when_one_is_already_ready() {
        assert!(should_stage_for_restart(
            true,
            Some("qwen3-1.7b-q8"),
            "gemma-4-e4b-q4"
        ));
        assert!(!should_stage_for_restart(
            true,
            Some("gemma-4-e4b-q4"),
            "gemma-4-e4b-q4"
        ));
        assert!(!should_stage_for_restart(false, None, "gemma-4-e4b-q4"));
    }

    #[test]
    fn transient_model_failures_retry_only_within_the_bounded_window() {
        assert!(should_retry_transient_install(true, 0, 2, false));
        assert!(should_retry_transient_install(true, 1, 2, false));
        assert!(!should_retry_transient_install(true, 2, 2, false));
        assert!(!should_retry_transient_install(false, 0, 2, false));
        assert!(!should_retry_transient_install(true, 0, 2, true));
    }
}
