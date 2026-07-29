use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    time::{Duration, SystemTime},
};

use airwiki_inference::{
    E5_FILES, HardwareReport, InstallEvent, MMARCO_COMMON_FILES, ModelProfile,
};
use airwiki_network::{ManualLanAddress, PublicCollectionAvailability, PublicRouteKind};
use airwiki_types::{
    ConceptType, DEFAULT_TOP_K, EnrichmentDraft, PublicCollectionSummary, PublicConceptSummary,
    SearchHit, SearchPurpose, SuggestedEntity, SuggestedLink,
};
use chrono::Datelike;
use eframe::egui::{self, Color32, RichText};
use egui_extras::{Size, StripBuilder};
use fluent_bundle::FluentArgs;
use uuid::Uuid;

mod first_knowledge;
mod integrations;
mod knowledge;
mod review;

use self::integrations::{ChatIntegrationsUi, IntegrationsUiAction};
use self::knowledge::{KnowledgeAction, KnowledgeUi, RecentConceptView, SearchEvidenceTarget};
use self::review::{
    REVIEW_QUEUE_WIDTH, ReviewEvidenceAction, ReviewEvidencePanelIntent, ReviewEvidenceUi,
    ReviewLayoutMode, review_action_bar_height, review_layout_mode, show_review_evidence_panel,
};

use crate::{
    activation::{ActivationAction, LaunchMode, PrimaryInstance},
    autostart::AutostartStatus,
    connectivity_platform::{
        ConnectivityPlatformSnapshot, FirewallDiagnosticState, NetworkProfileState,
        SystemPermissionState,
    },
    desktop_shell::{ClosePolicy, DesktopShell},
    i18n::{Localization, LocalizationError, UiLocale},
    layout::{ResponsiveLayout, SIDEBAR_WIDTH, STATUS_BAR_HEIGHT},
    model_config::{CloseBehavior, LanPreference, LocalePreference, ONBOARDING_VERSION},
    paths::AppPaths,
    readiness::{
        ConnectivityInput, ConnectivityPreference, DiscoveryState, FirewallState,
        FirstKnowledgeCta, FirstKnowledgeJourneyView, FirstKnowledgeStage, FirstKnowledgeStepState,
        ListenerState, NetworkProfile, OptionalFeatureState, ReadinessComponent, ReadinessInput,
        ReadinessStatus, RecommendedAction, SystemPermission, derive_first_knowledge_journey,
        derive_readiness,
    },
    updater::{UpdateIssueCode, UpdaterDisabledReason, UpdaterStatus},
    worker::{
        CollectionScanState, CollectionView, ConnectivityIssueCode, DesktopPreferencesUpdate,
        DesktopPreferencesView, FirewallOperationView, LanDiscoveryView, LanListenerView,
        ModelStateView, PeerActivityState, PeerTrustState, PeerView, PublicAnnouncementStatusView,
        ReviewItemView, SearchCoverageView, SourceIssueView, UpdaterWorkerView,
        WikiHealthSummaryView, WorkerCommand, WorkerEvent, WorkerHandle,
    },
};

const CONNECTIONS_CHAT_FOOTER_HEIGHT: f32 = 58.0;
const TODAY_COLUMN_GAP: f32 = 56.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Setup,
    Models,
    Collections,
    Review,
    Knowledge,
    Search,
    Public,
    Integrations,
    Nodes,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavIcon {
    Today,
    Library,
    Review,
    Wiki,
    Ask,
    Public,
    Connections,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardingPage {
    Welcome,
    Model,
    Permissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateConfirmationKind {
    Download,
    Install,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalAiPolicyChange {
    None,
    ApplyDisable,
    ConfirmEnable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WikiHealthCheckState {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SearchResultAvailability {
    LocalAvailable,
    LocalUnavailable,
    Remote { device_name: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SearchSurface {
    Ask,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AskScopePresentation {
    paired_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorialTagTone {
    Accent,
    Attention,
    Neutral,
    Outline,
}

#[derive(Debug)]
struct SearchViewState {
    question: String,
    top_k: u8,
    hits: Vec<SearchHit>,
    coverage: SearchCoverageView,
    completed: bool,
    error: Option<String>,
    submitted_public_network: bool,
    route_kind: PublicRouteKind,
}

impl SearchViewState {
    fn new() -> Self {
        Self {
            question: String::new(),
            top_k: DEFAULT_TOP_K,
            hits: Vec::new(),
            coverage: SearchCoverageView::Complete,
            completed: false,
            error: None,
            submitted_public_network: false,
            route_kind: PublicRouteKind::Offline,
        }
    }

    fn clear_feedback(&mut self) {
        self.hits.clear();
        self.coverage = SearchCoverageView::Complete;
        self.completed = false;
        self.error = None;
    }

    fn begin_search(&mut self, public_network: bool) {
        self.clear_feedback();
        self.submitted_public_network = public_network;
    }

    fn complete(
        &mut self,
        hits: Vec<SearchHit>,
        coverage: SearchCoverageView,
        route_kind: PublicRouteKind,
    ) {
        self.completed = true;
        self.hits = hits;
        self.coverage = coverage;
        self.error = None;
        self.route_kind = route_kind;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveSearch {
    request_id: Uuid,
    surface: SearchSurface,
}

pub struct AirWikiApp {
    instance: PrimaryInstance,
    shell: DesktopShell,
    localization: Localization,
    preferences: Option<DesktopPreferencesView>,
    preference_request_id: Option<Uuid>,
    autostart_status: Option<AutostartStatus>,
    autostart_request_id: Option<Uuid>,
    updater: Option<UpdaterWorkerView>,
    updater_request_id: Option<Uuid>,
    update_confirmation: Option<UpdateConfirmationKind>,
    exit_after_update_launch: bool,
    connectivity_platform: Option<ConnectivityPlatformSnapshot>,
    connectivity_request_id: Option<Uuid>,
    firewall_operation: Option<FirewallOperationView>,
    lan_listener: LanListenerView,
    lan_discovery: LanDiscoveryView,
    lan_local_addresses: Vec<String>,
    firewall_confirmation: bool,
    wiki_health: WikiHealthSummaryView,
    wiki_health_request_id: Option<Uuid>,
    wiki_health_generation: u64,
    wiki_health_check: WikiHealthCheckState,
    wiki_health_error_dismissed: bool,
    external_ai_confirmation: Option<Uuid>,
    public_collection_confirmation: Option<Uuid>,
    public_confirmation_return_focus: Option<egui::Id>,
    onboarding_page: Option<OnboardingPage>,
    onboarding_finishing: bool,
    paths: AppPaths,
    worker: WorkerHandle,
    screen: Screen,
    hardware: Option<HardwareReport>,
    model_state: Option<ModelStateView>,
    model_state_sequence: u64,
    accepted_licenses: bool,
    restart_required: Option<String>,
    models_ready: bool,
    install_label: Option<String>,
    install_progress: f32,
    node_id: String,
    mcp_url: String,
    collections: Vec<CollectionView>,
    collection_scans: HashMap<Uuid, CollectionScanState>,
    reviews: Vec<ReviewItemView>,
    source_issues: Vec<SourceIssueView>,
    peers: Vec<PeerView>,
    pairing_decisions_pending: HashSet<String>,
    pairing_modal_peer: Option<String>,
    pairing_confirmation_return_focus: Option<egui::Id>,
    ask_search: SearchViewState,
    public_search: SearchViewState,
    active_search: Option<ActiveSearch>,
    search_public_network: bool,
    blocked_public_publishers: Vec<String>,
    enabled_community_federation_index_count: usize,
    community_indexes_confirmation: bool,
    community_indexes_confirmation_return_focus: Option<egui::Id>,
    community_indexes_disable_request_id: Option<Uuid>,
    public_browse_request_id: Option<Uuid>,
    public_browse_publisher: String,
    public_browse_collection: Option<Uuid>,
    public_browse_summary: Option<PublicCollectionSummary>,
    public_browse_availability: PublicCollectionAvailability,
    public_browse_concepts: Vec<PublicConceptSummary>,
    public_browse_next_cursor: Option<String>,
    public_browse_error: Option<String>,
    public_browse_open: bool,
    new_collection_name: String,
    new_collection_folder: Option<PathBuf>,
    manual_multiaddress: String,
    notices: VecDeque<(bool, String)>,
    selected_review: Option<Uuid>,
    review_metadata_editor: Option<Uuid>,
    reanalyzing_reviews: HashSet<Uuid>,
    review_evidence: ReviewEvidenceUi,
    integrations: ChatIntegrationsUi,
    knowledge: KnowledgeUi,
}

impl AirWikiApp {
    pub fn new(
        context: &eframe::CreationContext<'_>,
        paths: AppPaths,
        launch_mode: LaunchMode,
        instance: PrimaryInstance,
    ) -> Result<Self, LocalizationError> {
        configure_style(&context.egui_ctx);
        Ok(Self {
            instance,
            shell: DesktopShell::new(launch_mode == LaunchMode::Background),
            localization: Localization::new(UiLocale::from_system())?,
            preferences: None,
            preference_request_id: None,
            autostart_status: None,
            autostart_request_id: None,
            updater: None,
            updater_request_id: None,
            update_confirmation: None,
            exit_after_update_launch: false,
            connectivity_platform: None,
            connectivity_request_id: None,
            firewall_operation: None,
            lan_listener: LanListenerView::Stopped,
            lan_discovery: LanDiscoveryView::Disabled,
            lan_local_addresses: Vec::new(),
            firewall_confirmation: false,
            wiki_health: WikiHealthSummaryView::default(),
            wiki_health_request_id: None,
            wiki_health_generation: 0,
            wiki_health_check: WikiHealthCheckState::Loading,
            wiki_health_error_dismissed: false,
            external_ai_confirmation: None,
            public_collection_confirmation: None,
            public_confirmation_return_focus: None,
            onboarding_page: None,
            onboarding_finishing: false,
            worker: WorkerHandle::spawn(paths.clone()),
            paths,
            screen: Screen::Setup,
            hardware: None,
            model_state: None,
            model_state_sequence: 0,
            accepted_licenses: false,
            restart_required: None,
            models_ready: false,
            install_label: None,
            install_progress: 0.0,
            node_id: "—".into(),
            mcp_url: "http://127.0.0.1:43123/mcp".into(),
            collections: Vec::new(),
            collection_scans: HashMap::new(),
            reviews: Vec::new(),
            source_issues: Vec::new(),
            peers: Vec::new(),
            pairing_decisions_pending: HashSet::new(),
            pairing_modal_peer: None,
            pairing_confirmation_return_focus: None,
            ask_search: SearchViewState::new(),
            public_search: SearchViewState::new(),
            active_search: None,
            search_public_network: false,
            blocked_public_publishers: Vec::new(),
            enabled_community_federation_index_count: 0,
            community_indexes_confirmation: false,
            community_indexes_confirmation_return_focus: None,
            community_indexes_disable_request_id: None,
            public_browse_request_id: None,
            public_browse_publisher: String::new(),
            public_browse_collection: None,
            public_browse_summary: None,
            public_browse_availability: PublicCollectionAvailability::Offline,
            public_browse_concepts: Vec::new(),
            public_browse_next_cursor: None,
            public_browse_error: None,
            public_browse_open: false,
            new_collection_name: String::new(),
            new_collection_folder: None,
            manual_multiaddress: String::new(),
            notices: VecDeque::new(),
            selected_review: None,
            review_metadata_editor: None,
            reanalyzing_reviews: HashSet::new(),
            review_evidence: ReviewEvidenceUi::default(),
            integrations: ChatIntegrationsUi::default(),
            knowledge: KnowledgeUi::default(),
        })
    }

    fn drain_events(&mut self) {
        let events: Vec<_> = self.worker.try_events().collect();
        for event in events {
            match event {
                WorkerEvent::Ready {
                    node_id,
                    mcp_url,
                    collections,
                    reviews,
                    source_issues,
                    blocked_public_publishers,
                    enabled_community_federation_index_count,
                } => {
                    self.node_id = node_id;
                    self.mcp_url = mcp_url;
                    self.collections = collections;
                    self.selected_review =
                        selected_review_after_refresh(self.selected_review, &reviews);
                    self.reviews = reviews;
                    self.source_issues = source_issues;
                    self.blocked_public_publishers = blocked_public_publishers;
                    self.enabled_community_federation_index_count =
                        enabled_community_federation_index_count;
                    self.refresh_integrations_if_needed();
                }
                WorkerEvent::Hardware(report) => self.hardware = Some(report),
                WorkerEvent::ModelState(state) => {
                    if state.state_sequence < self.model_state_sequence {
                        continue;
                    }
                    self.model_state_sequence = state.state_sequence;
                    if state.pending_model_id.is_none() {
                        self.restart_required = None;
                    }
                    let changed = self
                        .model_state
                        .as_ref()
                        .and_then(|current| current.recommended_model_id.as_deref())
                        != state.recommended_model_id.as_deref();
                    if changed {
                        self.accepted_licenses = state.license_accepted;
                    } else if state.license_accepted {
                        self.accepted_licenses = true;
                    }
                    self.model_state = Some(state);
                }
                WorkerEvent::DesktopPreferencesUpdated { request_id, result } => {
                    if request_id != Uuid::nil() && self.preference_request_id != Some(request_id) {
                        continue;
                    }
                    if request_id != Uuid::nil() {
                        self.preference_request_id = None;
                    }
                    match result {
                        Ok(preferences) => {
                            self.localization
                                .set_locale(effective_locale(preferences.locale));
                            self.preferences = Some(preferences);
                            if preferences
                                .completed_onboarding_version
                                .is_some_and(|version| version >= ONBOARDING_VERSION)
                            {
                                self.onboarding_page = None;
                                self.onboarding_finishing = false;
                            } else if self.onboarding_page.is_none() && !self.onboarding_finishing {
                                self.onboarding_page = Some(OnboardingPage::Welcome);
                            }
                        }
                        Err(error) => self
                            .notices
                            .push_back((true, sanitized_error_code(&error).to_owned())),
                    }
                }
                WorkerEvent::AutostartUpdated { request_id, result } => {
                    if request_id != Uuid::nil() && self.autostart_request_id != Some(request_id) {
                        continue;
                    }
                    if request_id != Uuid::nil() {
                        self.autostart_request_id = None;
                    }
                    match result {
                        Ok(status) => self.autostart_status = Some(status),
                        Err(error) => self
                            .notices
                            .push_back((true, sanitized_error_code(&error).to_owned())),
                    }
                }
                WorkerEvent::UpdaterUpdated { request_id, result } => {
                    if self.updater_request_id.is_some()
                        && self.updater_request_id != Some(request_id)
                    {
                        continue;
                    }
                    if self.updater_request_id == Some(request_id) {
                        self.updater_request_id = None;
                    }
                    match result {
                        Ok(view) => {
                            self.exit_after_update_launch = updater_launched_installer(&view);
                            self.updater = Some(view);
                        }
                        Err(error) => self
                            .notices
                            .push_back((true, sanitized_error_code(&error).to_owned())),
                    }
                }
                WorkerEvent::ConnectivityPlatformUpdated { request_id, result } => {
                    if self.connectivity_request_id.is_some()
                        && self.connectivity_request_id != Some(request_id)
                    {
                        continue;
                    }
                    if self.connectivity_request_id == Some(request_id) {
                        self.connectivity_request_id = None;
                    }
                    match result {
                        Ok(snapshot) => self.connectivity_platform = Some(snapshot),
                        Err(error) => self.notices.push_back((
                            true,
                            connectivity_issue_message(&self.localization, error),
                        )),
                    }
                }
                WorkerEvent::FirewallOperationUpdated { request_id, state } => {
                    if firewall_operation_update_applies(
                        self.connectivity_request_id,
                        request_id,
                        state,
                    ) {
                        self.firewall_operation = state;
                    }
                }
                WorkerEvent::LanRuntimeUpdated {
                    request_id,
                    listener,
                    discovery,
                    local_addresses,
                } => {
                    if request_id != Uuid::nil() {
                        continue;
                    }
                    self.lan_listener = listener;
                    self.lan_discovery = discovery;
                    self.lan_local_addresses = local_addresses;
                }
                WorkerEvent::WikiHealthUpdated {
                    request_id,
                    generation,
                    result,
                } => {
                    if self.wiki_health_request_id == Some(request_id) {
                        self.wiki_health_request_id = None;
                    }
                    if !wiki_health_result_applies(self.wiki_health_generation, generation) {
                        continue;
                    }
                    self.wiki_health_generation = generation;
                    match result {
                        Ok(summary) => {
                            self.wiki_health = summary;
                            self.wiki_health_check = WikiHealthCheckState::Ready;
                            self.wiki_health_error_dismissed = false;
                        }
                        Err(error) => {
                            self.wiki_health_check = WikiHealthCheckState::Failed(
                                sanitized_error_code(&error).to_owned(),
                            );
                            self.wiki_health_error_dismissed = false;
                        }
                    }
                }
                WorkerEvent::WikiMaintenanceFinished {
                    collection_id,
                    repaired,
                } => {
                    if repaired {
                        self.notices.push_back((
                            false,
                            self.localization.text("knowledge-maintenance-complete"),
                        ));
                    }
                    let reload_now = self.screen == Screen::Knowledge;
                    if let Some(action) = self
                        .knowledge
                        .mark_snapshot_stale(Some(collection_id), reload_now)
                    {
                        self.send_knowledge_action(action);
                    }
                }
                WorkerEvent::GuidedWikiRepairPrepared {
                    request_id,
                    collection_id,
                    result,
                } => {
                    self.knowledge
                        .guided_repair_prepared(request_id, collection_id, result);
                }
                WorkerEvent::GuidedWikiRepairFinished {
                    request_id,
                    collection_id,
                    result,
                } => {
                    let reload_now = self.screen == Screen::Knowledge;
                    if let Some(action) = self.knowledge.guided_repair_finished(
                        request_id,
                        collection_id,
                        result,
                        reload_now,
                    ) {
                        self.send_knowledge_action(action);
                    }
                }
                WorkerEvent::ModelsMissing => self.models_ready = false,
                WorkerEvent::InstallStopped => {
                    self.install_label = None;
                    self.install_progress = 0.0;
                }
                WorkerEvent::InstallQueued(message) => {
                    self.install_label =
                        Some(localized_worker_notice(&self.localization, &message));
                    self.install_progress = 0.0;
                }
                WorkerEvent::ModelsReady => {
                    self.models_ready = true;
                    self.install_label = None;
                    self.install_progress = 1.0;
                    self.notices
                        .push_back((false, self.localization.text("models-installed-notice")));
                }
                WorkerEvent::RestartRequired(message) => {
                    let message = localized_worker_notice(&self.localization, &message);
                    self.restart_required = Some(message.clone());
                    self.notices.push_back((false, message));
                }
                WorkerEvent::InstallProgress(event) => self.apply_install_event(event),
                WorkerEvent::Collections(collections) => {
                    self.collections = collections;
                    self.integrations.collections_changed();
                    self.refresh_integrations_if_needed();
                    let active_scans = self
                        .collection_scans
                        .keys()
                        .copied()
                        .collect::<HashSet<_>>();
                    let reload_now = self.screen == Screen::Knowledge;
                    if let Some(action) = self
                        .knowledge
                        .collections_changed(&active_scans, reload_now)
                    {
                        self.send_knowledge_action(action);
                    }
                }
                WorkerEvent::CollectionScan {
                    collection_id,
                    state,
                } => {
                    if let Some(state) = state {
                        let newly_active =
                            self.collection_scans.insert(collection_id, state).is_none();
                        if newly_active {
                            self.knowledge.collection_scan_started(collection_id);
                        }
                    } else {
                        let was_active = self.collection_scans.remove(&collection_id).is_some();
                        if was_active {
                            let reload_now = self.screen == Screen::Knowledge;
                            if let Some(action) = self
                                .knowledge
                                .collection_scan_finished(collection_id, reload_now)
                            {
                                self.send_knowledge_action(action);
                            }
                        }
                    }
                }
                WorkerEvent::Reviews(reviews) => {
                    self.selected_review =
                        selected_review_after_refresh(self.selected_review, &reviews);
                    self.reviews = reviews;
                }
                WorkerEvent::SourceIssues(source_issues) => {
                    self.source_issues = source_issues;
                }
                WorkerEvent::ReviewReanalysis {
                    concept_id,
                    running,
                } => {
                    if running {
                        self.reanalyzing_reviews.insert(concept_id);
                    } else {
                        self.reanalyzing_reviews.remove(&concept_id);
                    }
                    if let Some(action) =
                        self.review_evidence.reanalysis_changed(concept_id, running)
                    {
                        self.send_review_evidence_action(action);
                    }
                }
                WorkerEvent::ReviewEvidenceLoaded {
                    request_id,
                    concept_id,
                    expected_source_revision,
                    result,
                } => {
                    self.review_evidence.apply_loaded(
                        request_id,
                        concept_id,
                        expected_source_revision,
                        result,
                    );
                }
                WorkerEvent::KnowledgeBundleLoaded {
                    request_id,
                    collection_id,
                    result,
                } => {
                    if let Some(action) =
                        self.knowledge
                            .bundle_loaded(request_id, collection_id, result)
                    {
                        self.send_knowledge_action(action);
                    }
                }
                WorkerEvent::KnowledgePageLoaded {
                    request_id,
                    collection_id,
                    page_id,
                    result,
                } => {
                    if let Some(action) =
                        self.knowledge
                            .page_loaded(request_id, collection_id, page_id, result)
                    {
                        self.send_knowledge_action(action);
                    }
                }
                WorkerEvent::SearchPartial { request_id, hits } => {
                    let Some(surface) = search_response_surface(self.active_search, request_id)
                    else {
                        continue;
                    };
                    let search = self.search_state_mut(surface);
                    search.hits = hits;
                    search.coverage = SearchCoverageView::Partial;
                    search.error = None;
                }
                WorkerEvent::SearchFinished { request_id, result } => {
                    let Some(surface) = search_response_surface(self.active_search, request_id)
                    else {
                        continue;
                    };
                    self.active_search = None;
                    match result {
                        Ok((hits, coverage, route_kind)) => {
                            self.search_state_mut(surface)
                                .complete(hits, coverage, route_kind);
                        }
                        Err(error) => {
                            let search = self.search_state_mut(surface);
                            search.completed = false;
                            search.error = Some(sanitized_error_code(&error).to_owned());
                        }
                    }
                }
                WorkerEvent::PublicBrowseFinished { request_id, result } => {
                    if self.public_browse_request_id != Some(request_id) {
                        continue;
                    }
                    self.public_browse_request_id = None;
                    match result {
                        Ok(result) => {
                            self.public_browse_summary = Some(result.summary);
                            self.public_browse_availability = result.availability;
                            if let Some(mut page) = result.page {
                                self.public_browse_concepts.append(&mut page.concepts);
                                self.public_browse_next_cursor = page.next_cursor;
                            }
                            self.public_browse_error = None;
                        }
                        Err(error) => {
                            self.public_browse_error =
                                Some(sanitized_error_code(&error).to_owned());
                        }
                    }
                }
                WorkerEvent::CommunityFederationIndexesDisabled { request_id, result } => {
                    if self.community_indexes_disable_request_id != Some(request_id) {
                        continue;
                    }
                    self.community_indexes_disable_request_id = None;
                    match result {
                        Ok(disabled_count) => {
                            self.enabled_community_federation_index_count = 0;
                            let mut arguments = FluentArgs::new();
                            arguments.set("count", disabled_count as i64);
                            self.notices.push_back((
                                false,
                                self.localization.text_with(
                                    "public-community-indexes-disabled",
                                    Some(&arguments),
                                ),
                            ));
                        }
                        Err(error) => self
                            .notices
                            .push_back((true, sanitized_error_code(&error).to_owned())),
                    }
                }
                WorkerEvent::ChatIntegrationsUpdated { request_id, result } => {
                    self.integrations.apply_result(request_id, result);
                }
                WorkerEvent::Peers(peers) => self.peers = peers,
                WorkerEvent::Notice(message) => self
                    .notices
                    .push_back((false, localized_worker_notice(&self.localization, &message))),
                WorkerEvent::Error(message) => self
                    .notices
                    .push_back((true, sanitized_error_code(&message).to_owned())),
            }
        }
        deduplicate_notices(&mut self.notices);
        while self.notices.len() > 4 {
            self.notices.pop_front();
        }
    }

    fn apply_install_event(&mut self, event: InstallEvent) {
        match event {
            InstallEvent::Started { artifact, .. } => {
                self.install_label = Some(localized_model_progress(
                    &self.localization,
                    "models-downloading",
                    &artifact,
                ));
                self.install_progress = 0.0;
            }
            InstallEvent::Progress {
                artifact,
                downloaded,
                total_bytes,
            } => {
                self.install_label = Some(localized_model_progress(
                    &self.localization,
                    "models-downloading",
                    &artifact,
                ));
                self.install_progress = if total_bytes == 0 {
                    0.0
                } else {
                    downloaded as f32 / total_bytes as f32
                };
            }
            InstallEvent::Verifying { artifact } => {
                self.install_label = Some(localized_model_progress(
                    &self.localization,
                    "models-verifying",
                    &artifact,
                ))
            }
            InstallEvent::Extracting { artifact } => {
                self.install_label = Some(localized_model_progress(
                    &self.localization,
                    "models-installing",
                    &artifact,
                ))
            }
            InstallEvent::Complete { artifact } => {
                self.install_label = Some(localized_model_progress(
                    &self.localization,
                    "models-complete",
                    &artifact,
                ))
            }
        }
    }

    fn sidebar(&mut self, root: &mut egui::Ui) {
        let home = self.localization.text("nav-home");
        let collections = self.localization.text("nav-collections");
        let review_count = self.reviews.len().saturating_add(self.source_issues.len());
        let review = self.localization.text("nav-review");
        let wiki = self.localization.text("nav-wiki");
        let search = self.localization.text("nav-search");
        let public = self.localization.text("nav-public");
        let devices = self.localization.text("nav-devices");
        let settings = self.localization.text("nav-settings");
        egui::Panel::left("navigation")
            .exact_size(SIDEBAR_WIDTH)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::paper(root.visuals().dark_mode))
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(root, |ui| {
                ui.painter().vline(
                    ui.max_rect().right(),
                    ui.max_rect().y_range(),
                    egui::Stroke::new(1.0, crate::theme::border(ui.visuals().dark_mode)),
                );
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    ui.heading(
                        RichText::new("AirWiki")
                            .size(21.0)
                            .family(crate::theme::semibold_font_family()),
                    );
                });
                ui.add_space(24.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                nav(
                    ui,
                    &mut self.screen,
                    Screen::Setup,
                    NavIcon::Today,
                    &home,
                    None,
                );
                nav(
                    ui,
                    &mut self.screen,
                    Screen::Collections,
                    NavIcon::Library,
                    &collections,
                    None,
                );
                nav(
                    ui,
                    &mut self.screen,
                    Screen::Review,
                    NavIcon::Review,
                    &review,
                    Some(review_count),
                );
                nav(
                    ui,
                    &mut self.screen,
                    Screen::Knowledge,
                    NavIcon::Wiki,
                    &wiki,
                    None,
                );
                nav(
                    ui,
                    &mut self.screen,
                    Screen::Search,
                    NavIcon::Ask,
                    &search,
                    None,
                );
                nav(
                    ui,
                    &mut self.screen,
                    Screen::Public,
                    NavIcon::Public,
                    &public,
                    None,
                );
                nav(
                    ui,
                    &mut self.screen,
                    Screen::Nodes,
                    NavIcon::Connections,
                    &devices,
                    None,
                );
                nav(
                    ui,
                    &mut self.screen,
                    Screen::Settings,
                    NavIcon::Settings,
                    &settings,
                    None,
                );
            });
    }

    #[cfg(target_os = "macos")]
    fn title_bar(&self, root: &mut egui::Ui) {
        egui::Panel::top("editorial_titlebar")
            .exact_size(crate::layout::TITLE_BAR_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::paper(root.visuals().dark_mode))
                    .stroke(egui::Stroke::NONE),
            )
            .show(root, |ui| {
                let rect = ui.available_rect_before_wrap();
                let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                ui.painter().hline(
                    rect.x_range(),
                    rect.bottom(),
                    egui::Stroke::new(1.0, crate::theme::border(ui.visuals().dark_mode)),
                );
                if response.drag_started() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "AirWiki",
                    egui::FontId::new(13.0, crate::theme::semibold_font_family()),
                    crate::theme::ink(ui.visuals().dark_mode),
                );
            });
    }

    fn status_bar(&mut self, root: &mut egui::Ui) {
        let review_count = self.reviews.len().saturating_add(self.source_issues.len());
        let model_status = if self.models_ready {
            self.localization.text("models-ready")
        } else {
            self.localization.text("models-pending")
        };
        let mut review_arguments = FluentArgs::new();
        review_arguments.set("count", review_count);
        let review_status = self
            .localization
            .text_with("status-review-count", Some(&review_arguments));

        let wiki_status = match (
            &self.wiki_health_check,
            self.readiness_view().last_checked_at,
        ) {
            (WikiHealthCheckState::Ready, Some(checked_at)) => {
                let minutes = elapsed_minutes(checked_at, SystemTime::now());
                if minutes == 0 {
                    self.localization.text("home-checked-now")
                } else {
                    let mut arguments = FluentArgs::new();
                    arguments.set("minutes", minutes);
                    self.localization
                        .text_with("home-checked-minutes", Some(&arguments))
                }
            }
            (WikiHealthCheckState::Loading, _) => self.localization.text("home-wiki-checking"),
            (WikiHealthCheckState::Failed(_), _) => self.localization.text("home-wiki-failed"),
            (WikiHealthCheckState::Ready, None) => self.localization.text("home-wiki-not-checked"),
        };
        let public_count = self
            .collections
            .iter()
            .filter(|collection| collection.internet_public)
            .count();
        let compact_status = root.available_width() < 1000.0;

        let mut refresh_wiki_health = false;
        egui::Panel::bottom("editorial_status")
            .exact_size(STATUS_BAR_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::paper(root.visuals().dark_mode))
                    .stroke(egui::Stroke::NONE)
                    .inner_margin(egui::Margin::symmetric(20, 4)),
            )
            .show(root, |ui| {
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.max_rect().top(),
                    egui::Stroke::new(1.0, crate::theme::border(ui.visuals().dark_mode)),
                );
                ui.spacing_mut().item_spacing.x = 18.0;
                ui.horizontal(|ui| {
                    let model_color = if self.models_ready {
                        crate::theme::AIR_CYAN
                    } else {
                        crate::theme::attention(ui.visuals().dark_mode)
                    };
                    ui.horizontal(|ui| {
                        ui.colored_label(model_color, "●");
                        ui.label(RichText::new(model_status).small());
                    });
                    if compact_status {
                        ui.label(
                            RichText::new(&review_status)
                                .small()
                                .color(crate::theme::attention(ui.visuals().dark_mode)),
                        );
                        ui.menu_button(
                            RichText::new(self.localization.text("status-details")).small(),
                            |ui| {
                                if wiki_health_can_refresh(&self.wiki_health_check) {
                                    if ui
                                        .button(&wiki_status)
                                        .on_hover_text(self.localization.text("action-refresh"))
                                        .clicked()
                                    {
                                        refresh_wiki_health = true;
                                        ui.close();
                                    }
                                } else {
                                    ui.label(&wiki_status);
                                }
                                if public_count > 0 {
                                    ui.label(format!(
                                        "{}: {public_count}",
                                        self.localization.text("nav-public")
                                    ));
                                }
                                ui.label(format!(
                                    "v{} · {}",
                                    env!("CARGO_PKG_VERSION"),
                                    self.localization.text("status-development-build")
                                ));
                            },
                        );
                    } else {
                        if wiki_health_can_refresh(&self.wiki_health_check) {
                            if ui
                                .small_button(RichText::new(&wiki_status).small())
                                .on_hover_text(self.localization.text("action-refresh"))
                                .clicked()
                            {
                                refresh_wiki_health = true;
                            }
                        } else {
                            ui.label(RichText::new(&wiki_status).small());
                        }
                        ui.label(
                            RichText::new(&review_status)
                                .small()
                                .color(crate::theme::attention(ui.visuals().dark_mode)),
                        );
                        if public_count > 0 {
                            ui.label(
                                RichText::new(format!(
                                    "{}: {public_count}",
                                    self.localization.text("nav-public")
                                ))
                                .small(),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "v{} · {}",
                                    env!("CARGO_PKG_VERSION"),
                                    self.localization.text("status-development-build")
                                ))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                            );
                        });
                    }
                });
            });
        if refresh_wiki_health {
            let request_id = Uuid::new_v4();
            self.wiki_health_request_id = Some(request_id);
            self.wiki_health_check = WikiHealthCheckState::Loading;
            self.worker
                .send(WorkerCommand::RefreshWikiHealth { request_id });
        }
    }

    fn setup(&mut self, ui: &mut egui::Ui) {
        if ui
            .button(self.localization.text("models-back-home"))
            .clicked()
        {
            self.screen = Screen::Setup;
        }
        page_title(
            ui,
            &self.localization.text("models-title"),
            &self.localization.text("models-subtitle"),
        );
        let details_height = ui.available_height().max(0.0);
        egui::ScrollArea::vertical()
            .id_salt("models_configuration")
            .max_height(details_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if let Some(report) = &self.hardware {
                    crate::theme::surface_frame(ui.visuals().dark_mode).show(ui, |ui| {
                        ui.heading(self.localization.text("models-diagnostics"));
                        egui::Grid::new("diagnostic_grid")
                            .num_columns(2)
                            .spacing([24.0, 8.0])
                            .show(ui, |ui| {
                                ui.label(self.localization.text("models-platform"));
                                ui.label(format!("{} {}", report.os, report.architecture));
                                ui.end_row();
                                ui.label(self.localization.text("models-memory"));
                                let mut arguments = FluentArgs::new();
                                arguments.set(
                                    "total",
                                    format!(
                                        "{:.1}",
                                        report.total_memory_bytes as f64 / 1024_f64.powi(3)
                                    ),
                                );
                                arguments.set(
                                    "available",
                                    format!(
                                        "{:.1}",
                                        report.available_memory_bytes as f64 / 1024_f64.powi(3)
                                    ),
                                );
                                ui.label(
                                    self.localization
                                        .text_with("models-memory-value", Some(&arguments)),
                                );
                                ui.end_row();
                                ui.label(self.localization.text("models-free-space"));
                                ui.label(format!(
                                    "{:.1} GiB",
                                    report.available_disk_bytes as f64 / 1024_f64.powi(3)
                                ));
                                ui.end_row();
                                ui.label(self.localization.text("models-avx2"));
                                ui.label(if report.avx2 {
                                    self.localization.text("models-available")
                                } else if report.os == "windows" {
                                    self.localization.text("models-unavailable")
                                } else {
                                    self.localization.text("models-not-required")
                                });
                                ui.end_row();
                                ui.label(self.localization.text("models-acceleration"));
                                ui.label(if report.metal_available {
                                    "Metal".to_owned()
                                } else if report.os == "windows" {
                                    self.localization.text("models-cpu")
                                } else {
                                    self.localization.text("models-unavailable")
                                });
                                ui.end_row();
                            });
                        for issue in &report.issues {
                            ui.colored_label(
                                crate::theme::error_text(ui.visuals().dark_mode),
                                issue,
                            );
                        }
                    });
                } else {
                    ui.spinner();
                    ui.label(self.localization.text("models-diagnosing"));
                }
                ui.add_space(14.0);
                crate::theme::surface_frame(ui.visuals().dark_mode).show(ui, |ui| {
                    ui.heading(self.localization.text("models-local-title"));
                    ui.label(self.localization.text("models-local-body"));
                    if let Some(state) = self.model_state.clone() {
                        ui.horizontal(|ui| {
                            for profile in [
                                ModelProfile::Automatic,
                                ModelProfile::Efficient,
                                ModelProfile::Quality,
                            ] {
                                let label = profile_label(&self.localization, profile);
                                if ui
                                    .selectable_label(state.profile == profile, label)
                                    .clicked()
                                    && state.profile != profile
                                {
                                    self.accepted_licenses = false;
                                    self.worker.send(WorkerCommand::SetModelProfile(profile));
                                }
                            }
                        });
                        ui.add_space(6.0);
                        if let Some(display_name) = &state.recommended_display_name {
                            ui.heading(RichText::new(display_name).size(18.0));
                        }
                        if let Some(reason) = &state.recommendation_reason {
                            ui.label(reason);
                        }
                        if state.degraded {
                            ui.colored_label(
                                crate::theme::warning_text(ui.visuals().dark_mode),
                                self.localization.text("models-profile-reduced"),
                            );
                        }
                        if let Some(active) = &state.active_model_id {
                            let mut arguments = FluentArgs::new();
                            arguments.set("model", active.as_str());
                            ui.label(
                                self.localization
                                    .text_with("models-active", Some(&arguments)),
                            );
                        }
                        if let Some(pending) = &state.pending_model_id {
                            let mut arguments = FluentArgs::new();
                            arguments.set("model", pending.as_str());
                            ui.label(
                                self.localization
                                    .text_with("models-pending-restart", Some(&arguments)),
                            );
                        }
                        let mut arguments = FluentArgs::new();
                        arguments.set(
                            "download",
                            format!("{:.2}", state.download_bytes as f64 / 1024_f64.powi(3)),
                        );
                        arguments.set(
                            "required",
                            format!("{:.2}", state.required_free_bytes as f64 / 1024_f64.powi(3)),
                        );
                        ui.label(
                            self.localization
                                .text_with("models-download-size", Some(&arguments)),
                        );
                        for issue in &state.issues {
                            ui.colored_label(
                                crate::theme::error_text(ui.visuals().dark_mode),
                                issue,
                            );
                        }
                        if let (Some(license), Some(url), Some(revision)) =
                            (&state.license, &state.license_url, &state.revision)
                        {
                            ui.horizontal_wrapped(|ui| {
                                ui.hyperlink_to(
                                    localized_license(&self.localization, license),
                                    url,
                                );
                                ui.separator();
                                let mut revision_arguments = FluentArgs::new();
                                revision_arguments
                                    .set("revision", &revision[..revision.len().min(12)]);
                                ui.label(
                                    self.localization
                                        .text_with("models-revision", Some(&revision_arguments)),
                                );
                                ui.separator();
                                ui.hyperlink_to(
                                    localized_license(&self.localization, E5_FILES[0].license),
                                    E5_FILES[0].license_url,
                                );
                                ui.separator();
                                ui.hyperlink_to(
                                    localized_license(
                                        &self.localization,
                                        MMARCO_COMMON_FILES[0].license,
                                    ),
                                    MMARCO_COMMON_FILES[0].license_url,
                                );
                                ui.separator();
                                ui.hyperlink_to(
                                    localized_license(&self.localization, "llama.cpp"),
                                    "https://github.com/ggml-org/llama.cpp/blob/b9946/LICENSE",
                                );
                            });
                        }
                        ui.checkbox(
                            &mut self.accepted_licenses,
                            self.localization.text("models-accept-licenses"),
                        );
                        let recommended = state.recommended_model_id.as_deref();
                        let already_active =
                            self.models_ready && state.active_model_id.as_deref() == recommended;
                        let already_pending = state.pending_model_id.as_deref() == recommended;
                        let can_install = recommended.is_some()
                            && !already_active
                            && !already_pending
                            && self.accepted_licenses
                            && state.fits_available_disk
                            && state.issues.is_empty()
                            && self.install_label.is_none();
                        ui.horizontal(|ui| {
                            let label = model_action_label(
                                &self.localization,
                                state.recommended_assets_installed,
                                self.models_ready,
                            );
                            if ui
                                .add_enabled(can_install, egui::Button::new(label))
                                .clicked()
                            {
                                self.worker.send(WorkerCommand::InstallModels);
                            }
                            if self.install_label.is_some()
                                && ui.button(self.localization.text("action-cancel")).clicked()
                            {
                                self.worker.send(WorkerCommand::CancelInstall);
                            }
                            if already_active {
                                ui.colored_label(
                                    crate::theme::verified_text(ui.visuals().dark_mode),
                                    self.localization.text("models-recommended-active"),
                                );
                            } else if already_pending {
                                ui.label(self.localization.text("models-restart-to-activate"));
                            } else if state.recommended_assets_installed {
                                ui.label(self.localization.text("models-already-downloaded"));
                            }
                        });
                    } else {
                        ui.spinner();
                        ui.label(self.localization.text("models-calculating"));
                    }
                    if let Some(label) = &self.install_label {
                        ui.label(label);
                        ui.add(
                            egui::ProgressBar::new(self.install_progress.clamp(0.0, 1.0))
                                .show_percentage(),
                        );
                    }
                    if let Some(message) = &self.restart_required {
                        ui.colored_label(
                            crate::theme::verified_text(ui.visuals().dark_mode),
                            message,
                        );
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(self.localization.text("models-multimodal-future"))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                });
                scroll_newly_focused_control_into_view(ui);
            });
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        let layout = ResponsiveLayout::from_available(ui.clip_rect().size());
        let readiness = self.readiness_view();
        let published_count = self
            .collections
            .iter()
            .map(|collection| collection.published_count)
            .sum();
        let document_count = self
            .collections
            .iter()
            .map(|collection| collection.document_count)
            .sum::<usize>();
        let journey = derive_first_knowledge_journey(&readiness, published_count);
        let collection_options = self
            .collections
            .iter()
            .map(|collection| (collection.id, collection.name.clone()))
            .collect::<Vec<_>>();
        let active_scans = self
            .collection_scans
            .keys()
            .copied()
            .collect::<HashSet<_>>();
        let knowledge_actions = self
            .knowledge
            .prepare_recent_concepts(&collection_options, &active_scans);
        for action in knowledge_actions {
            self.send_knowledge_action(action);
        }
        let recent_concepts = self.knowledge.recent_concepts();

        first_knowledge::show_today_header(
            ui,
            &self.localization,
            &localized_today_date(self.localization.locale()),
            self.collections.len(),
            published_count,
            layout.density,
        );
        ui.add_space(if layout.is_compact() { 20.0 } else { 36.0 });
        if layout.is_narrow() {
            first_knowledge::work_surface(ui, layout.density, |ui| {
                self.home_primary_story(ui, journey, document_count);
            });
            ui.add_space(30.0);
            self.home_ask_column(ui, &recent_concepts);
        } else {
            let (lead_width, side_width) = today_column_widths(ui.available_width());
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(lead_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            first_knowledge::work_surface(ui, layout.density, |ui| {
                                self.home_primary_story(ui, journey, document_count);
                            });
                        },
                    );
                    ui.add_space(TODAY_COLUMN_GAP);
                    ui.allocate_ui_with_layout(
                        egui::vec2(side_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| self.home_ask_column(ui, &recent_concepts),
                    );
                });
            });
        }
    }

    fn home_primary_story(
        &mut self,
        ui: &mut egui::Ui,
        journey: FirstKnowledgeJourneyView,
        document_count: usize,
    ) {
        editorial_card_kicker(
            ui,
            self.localization.text("home-lead-kicker").to_uppercase(),
            crate::theme::accent_text(ui.visuals().dark_mode),
        );
        match journey.cta {
            Some(FirstKnowledgeCta::Recommended(action)) => {
                let (title, explanation) = if action == RecommendedAction::ReviewPendingKnowledge {
                    let mut arguments = FluentArgs::new();
                    arguments.set("count", self.reviews.len() as i64);
                    (
                        self.localization
                            .text_with("home-review-drafts-title", Some(&arguments)),
                        self.localization.text("home-review-drafts-body"),
                    )
                } else {
                    (
                        primary_action_title(&self.localization, action),
                        primary_action_explanation(&self.localization, action),
                    )
                };
                ui.heading(
                    RichText::new(title)
                        .size(30.0)
                        .family(crate::theme::semibold_font_family()),
                );
                ui.add(egui::Label::new(explanation).wrap());
                if !self.reviews.is_empty() {
                    ui.label(
                        RichText::new(
                            self.reviews
                                .iter()
                                .take(3)
                                .map(|item| item.draft.title.as_str())
                                .collect::<Vec<_>>()
                                .join(" · "),
                        )
                        .color(ui.visuals().weak_text_color()),
                    );
                }
                ui.add_space(10.0);
                if ui
                    .add(first_knowledge::primary_button(primary_action_button(
                        &self.localization,
                        action,
                    )))
                    .clicked()
                {
                    self.open_readiness_action(action);
                }
            }
            Some(FirstKnowledgeCta::SearchKnowledge) => {
                ui.heading(
                    RichText::new(self.localization.text("onboarding-search-title")).size(30.0),
                );
                ui.label(self.localization.text("onboarding-search-body"));
                ui.add_space(10.0);
                if ui
                    .add(first_knowledge::primary_button(
                        self.localization.text("search-action"),
                    ))
                    .clicked()
                {
                    self.screen = Screen::Search;
                }
            }
            None => {
                let (title, body) = journey_stage_copy(&self.localization, journey.current_stage);
                ui.heading(RichText::new(title).size(30.0));
                ui.label(body);
                ui.add_space(10.0);
                if journey.current_stage == FirstKnowledgeStage::ProcessKnowledge
                    && document_count == 0
                {
                    if ui
                        .button(self.localization.text("onboarding-processing-open-folder"))
                        .clicked()
                    {
                        self.screen = Screen::Collections;
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            readiness_status_presentation(
                                &self.localization,
                                first_knowledge_readiness_status(
                                    journey.stage_state(journey.current_stage),
                                ),
                                ui.visuals().dark_mode,
                            )
                            .0,
                        );
                    });
                }
            }
        }
        if self.home_source_issue_story(ui) {
            ui.add_space(18.0);
        }
        self.home_wiki_incident(ui);
    }

    fn home_source_issue_story(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(first_issue) = self.source_issues.first() else {
            return false;
        };
        let collection_name = first_issue.collection_name.clone();
        let issue_count = self
            .source_issues
            .iter()
            .filter(|issue| issue.collection_id == first_issue.collection_id)
            .count();
        ui.add_space(44.0);
        editorial_card_kicker(
            ui,
            self.localization
                .text("home-attention-kicker")
                .to_uppercase(),
            crate::theme::attention(ui.visuals().dark_mode),
        );
        let mut title_arguments = FluentArgs::new();
        title_arguments.set("collection", collection_name);
        ui.heading(
            RichText::new(
                self.localization
                    .text_with("home-source-issue-title", Some(&title_arguments)),
            )
            .size(22.0)
            .family(crate::theme::semibold_font_family()),
        );
        let mut body_arguments = FluentArgs::new();
        body_arguments.set("count", issue_count as i64);
        ui.add(
            egui::Label::new(
                self.localization
                    .text_with("home-source-issue-body", Some(&body_arguments)),
            )
            .wrap(),
        );
        ui.add_space(10.0);
        if ui
            .button(self.localization.text("home-source-issue-action"))
            .clicked()
        {
            self.screen = Screen::Collections;
        }
        true
    }

    fn home_ask_column(&mut self, ui: &mut egui::Ui, recent_concepts: &[RecentConceptView]) {
        let ask_label = editorial_card_kicker(
            ui,
            self.localization.text("home-ask-kicker").to_uppercase(),
            crate::theme::accent_text(ui.visuals().dark_mode),
        );
        ui.add_sized(
            [ui.available_width(), 36.0],
            egui::TextEdit::singleline(&mut self.ask_search.question)
                .hint_text(self.localization.text("home-ask-placeholder")),
        )
        .labelled_by(ask_label.id);
        if ui.button(self.localization.text("search-action")).clicked() {
            self.screen = Screen::Search;
        }
        ui.add_space(28.0);
        editorial_card_kicker(
            ui,
            self.localization
                .text("home-recently-published")
                .to_uppercase(),
            crate::theme::accent_text(ui.visuals().dark_mode),
        );
        let mut requested_concept = None;
        for concept in recent_concepts {
            ui.add_space(6.0);
            let reviewed_at =
                (!concept.reviewed_at.is_empty()).then_some(concept.reviewed_at.as_str());
            let concept_button =
                editorial_title_row_button(ui, &concept.title, reviewed_at, 15.0).on_hover_text(
                    format!("{} · {}", concept.concept_type, concept.collection_name),
                );
            if concept_button.clicked() {
                requested_concept = Some(concept.id);
            }
            if !concept.summary.trim().is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(&concept.summary)
                            .size(13.0)
                            .color(ui.visuals().weak_text_color()),
                    )
                    .wrap(),
                );
            }
            ui.separator();
        }
        if recent_concepts.is_empty() {
            ui.label(
                RichText::new(self.localization.text("home-no-recently-published"))
                    .color(ui.visuals().weak_text_color()),
            );
        }
        if let Some(concept_id) = requested_concept
            && let Some(action) = self.knowledge.open_recent_concept(concept_id)
        {
            self.screen = Screen::Knowledge;
            self.send_knowledge_action(action);
        }
    }

    fn readiness_view(&self) -> crate::readiness::NodeReadinessView {
        let preference = match self
            .preferences
            .map(|preferences| preferences.lan_preference)
        {
            Some(LanPreference::Enabled) => ConnectivityPreference::Enabled,
            Some(LanPreference::Disabled) => ConnectivityPreference::Disabled,
            Some(LanPreference::Undecided) | None => ConnectivityPreference::Undecided,
        };
        let network_enabled = preference == ConnectivityPreference::Enabled;
        let platform = self.connectivity_platform;
        let system_permission = if !network_enabled {
            SystemPermission::NotRequired
        } else {
            match platform.map(|snapshot| snapshot.system_permission) {
                Some(SystemPermissionState::NotApplicable) => SystemPermission::NotRequired,
                Some(SystemPermissionState::Granted) => SystemPermission::Granted,
                Some(SystemPermissionState::Denied) => SystemPermission::Denied,
                Some(SystemPermissionState::Unknown)
                    if self
                        .peers
                        .iter()
                        .any(|peer| peer.activity != PeerActivityState::NotObserved) =>
                {
                    SystemPermission::Granted
                }
                Some(SystemPermissionState::Unknown) => SystemPermission::Pending,
                None => SystemPermission::Unknown,
            }
        };
        let network_profile = match platform.map(|snapshot| snapshot.network_profile) {
            Some(NetworkProfileState::NotApplicable) => NetworkProfile::NotApplicable,
            Some(NetworkProfileState::Private) => NetworkProfile::Private,
            Some(NetworkProfileState::Domain) => NetworkProfile::Domain,
            Some(NetworkProfileState::Public) => NetworkProfile::Public,
            Some(NetworkProfileState::Unknown) | None => NetworkProfile::Unknown,
        };
        let firewall = match platform.map(|snapshot| snapshot.firewall) {
            Some(FirewallDiagnosticState::NotApplicable) => FirewallState::NotRequired,
            Some(FirewallDiagnosticState::Ready) => FirewallState::Ready,
            Some(FirewallDiagnosticState::FirewallDisabled) => FirewallState::Disabled,
            Some(FirewallDiagnosticState::BlockAllInbound) => FirewallState::BlockAllInbound,
            Some(FirewallDiagnosticState::RulesMissing)
                if platform
                    .is_some_and(|snapshot| !snapshot.firewall_helper.can_request_elevation()) =>
            {
                FirewallState::HelperUnavailable
            }
            Some(FirewallDiagnosticState::RulesMissing) => FirewallState::RulesMissing,
            Some(FirewallDiagnosticState::LegacyExposure) => FirewallState::LegacyExposure,
            Some(FirewallDiagnosticState::Conflict) => FirewallState::Conflict,
            Some(FirewallDiagnosticState::ManagedPolicy) => FirewallState::Managed,
            Some(FirewallDiagnosticState::Unsupported) => FirewallState::Unsupported,
            Some(FirewallDiagnosticState::Error) => FirewallState::Error,
            Some(FirewallDiagnosticState::Unknown) | None => FirewallState::Unknown,
        };
        let background = match self.autostart_status {
            Some(AutostartStatus::Enabled) => OptionalFeatureState::Ready,
            Some(AutostartStatus::RequiresApproval) => OptionalFeatureState::NeedsPermission,
            Some(AutostartStatus::Conflict) => OptionalFeatureState::NeedsAttention,
            Some(AutostartStatus::Disabled | AutostartStatus::Unsupported) => {
                OptionalFeatureState::Disabled
            }
            None => OptionalFeatureState::Working,
        };
        let updates = match &self.updater {
            Some(UpdaterWorkerView::Disabled(_)) => OptionalFeatureState::Disabled,
            Some(UpdaterWorkerView::Ready(view)) if view.last_issue.is_some() => {
                OptionalFeatureState::NeedsAttention
            }
            Some(UpdaterWorkerView::Ready(view))
                if matches!(
                    view.status,
                    UpdaterStatus::Checking
                        | UpdaterStatus::Downloading(_)
                        | UpdaterStatus::Installing(_)
                ) =>
            {
                OptionalFeatureState::Working
            }
            Some(UpdaterWorkerView::Ready(_)) => OptionalFeatureState::Ready,
            None => OptionalFeatureState::Working,
        };
        let (wiki_working, wiki_issue_count) =
            wiki_health_readiness_inputs(&self.wiki_health_check, &self.wiki_health);
        derive_readiness(ReadinessInput {
            models_ready: self.models_ready,
            models_working: self.install_label.is_some(),
            model_issue_count: self
                .model_state
                .as_ref()
                .map_or(0, |state| state.issues.len()),
            models_need_permission: false,
            collection_count: self.collections.len(),
            collections_working: !self.collection_scans.is_empty(),
            collection_issue_count: self
                .collections
                .iter()
                .filter(|collection| {
                    collection.maintenance.as_ref().is_some_and(|maintenance| {
                        matches!(
                            maintenance.status,
                            airwiki_core::CollectionMaintenanceStatus::Failed
                                | airwiki_core::CollectionMaintenanceStatus::Quarantined
                        )
                    })
                })
                .count(),
            pending_review_count: self.reviews.len().saturating_add(self.source_issues.len()),
            wiki_working,
            wiki_issue_count,
            connectivity: ConnectivityInput {
                preference,
                system_permission,
                network_profile,
                firewall,
                listener: match self.lan_listener {
                    LanListenerView::Stopped => ListenerState::Stopped,
                    LanListenerView::Starting => ListenerState::Starting,
                    LanListenerView::Listening => ListenerState::Listening,
                    LanListenerView::Failed => ListenerState::Failed,
                },
                discovery: match self.lan_discovery {
                    LanDiscoveryView::Disabled => DiscoveryState::Disabled,
                    LanDiscoveryView::Starting => DiscoveryState::Starting,
                    LanDiscoveryView::Active => DiscoveryState::Active,
                    LanDiscoveryView::Failed => DiscoveryState::Failed,
                },
                peer_count: self
                    .peers
                    .iter()
                    .filter(|peer| peer.trust == PeerTrustState::Trusted)
                    .count(),
            },
            chat: self.integrations.readiness_state(),
            background,
            updates,
            last_checked_at: self.wiki_health.checked_at,
        })
    }

    fn open_readiness_action(&mut self, action: RecommendedAction) {
        let knowledge_action = (action == RecommendedAction::InspectWikiHealth).then(|| {
            let collection_id = self
                .wiki_health
                .attention_collection_id
                .filter(|candidate| {
                    self.collections
                        .iter()
                        .any(|collection| collection.id == *candidate)
                });
            let scan_active = collection_id
                .is_some_and(|collection_id| self.collection_scans.contains_key(&collection_id));
            self.knowledge.select_health(collection_id, scan_active)
        });
        self.screen =
            match action {
                RecommendedAction::PrepareLocalAi | RecommendedAction::ResolveLocalAiIssue => {
                    Screen::Models
                }
                RecommendedAction::AddKnowledgeFolder
                | RecommendedAction::ResolveCollectionIssue => Screen::Collections,
                RecommendedAction::ReviewPendingKnowledge => Screen::Review,
                RecommendedAction::InspectWikiHealth => Screen::Knowledge,
                RecommendedAction::ExplainLan
                | RecommendedAction::RequestSystemPermission
                | RecommendedAction::ChangeNetworkProfile
                | RecommendedAction::ConfigureFirewall
                | RecommendedAction::OpenFirewallSettings
                | RecommendedAction::ReviewLegacyFirewallRules
                | RecommendedAction::RepairConnectivityInstallation
                | RecommendedAction::ContactAdministrator
                | RecommendedAction::RetryConnectivity => Screen::Nodes,
                RecommendedAction::ResolveChatIssue => Screen::Integrations,
                RecommendedAction::ResolveBackgroundIssue
                | RecommendedAction::ResolveUpdateIssue => Screen::Settings,
            };
        if let Some(Some(action)) = knowledge_action {
            self.send_knowledge_action(action);
        }
    }

    fn collections(&mut self, ui: &mut egui::Ui) {
        ui.set_max_width(866.0);
        page_title(
            ui,
            &self.localization.text("collections-title"),
            &self.localization.text("collections-subtitle"),
        );
        if self.collections.is_empty() {
            empty_state(
                ui,
                &self.localization.text("collections-empty-title"),
                &self.localization.text("collections-empty-body"),
            );
        }
        let linked = self.localization.text("collections-linked");
        let queued = self.localization.text("collections-scan-queued");
        let scanning = self.localization.text("collections-scan-running");
        let relink = self.localization.text("collections-relink");
        let retry = self.localization.text("collections-retry");
        let share_peers = self.localization.text("collections-policy-peers");
        let allow_chat = self.localization.text("collections-policy-chat");
        let local_only = self.localization.text("collections-local-only");
        let cloud_warning = self.localization.text("collections-cloud-warning");
        let mut requested_external_ai_confirmation = None;
        let mut requested_public_confirmation = None;
        ui.vertical(|ui| {
            for collection in &mut self.collections {
                let pending_documents = self
                    .reviews
                    .iter()
                    .filter(|review| review.collection_name == collection.name)
                    .map(|review| review.source_name.clone())
                    .collect::<Vec<_>>();
                let collection_issues = self
                    .source_issues
                    .iter()
                    .filter(|issue| issue.collection_id == collection.id)
                    .collect::<Vec<_>>();
                let scan_state = self.collection_scans.get(&collection.id).copied();
                let scan_needs_attention =
                    !collection_issues.is_empty() || collection.failed_count > 0;
                let scan_label = if scan_needs_attention {
                    self.localization.text("review-scan-again")
                } else {
                    retry.clone()
                };
                let mut counts_arguments = FluentArgs::new();
                counts_arguments.set("documents", collection.document_count);
                counts_arguments.set("published", collection.published_count);
                let counts = self
                    .localization
                    .text_with("collections-counts", Some(&counts_arguments));
                ui.push_id(collection.id, |ui| {
                    egui::Frame::new().show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.heading(
                                        RichText::new(&collection.name)
                                            .size(24.0)
                                            .family(crate::theme::semibold_font_family()),
                                    );
                                    editorial_tag(ui, &linked, EditorialTagTone::Accent);
                                    if collection.internet_public {
                                        editorial_tag(
                                            ui,
                                            &self.localization.text("public-visible"),
                                            EditorialTagTone::Outline,
                                        );
                                    }
                                    if collection.needs_review_count > 0
                                        || collection.failed_count > 0
                                    {
                                        let mut attention_arguments = FluentArgs::new();
                                        attention_arguments
                                            .set("review", collection.needs_review_count);
                                        attention_arguments.set("failed", collection.failed_count);
                                        editorial_tag(
                                            ui,
                                            &self.localization.text_with(
                                                "collections-attention-counts",
                                                Some(&attention_arguments),
                                            ),
                                            EditorialTagTone::Attention,
                                        );
                                    }
                                });
                                let mut collection_metadata =
                                    vec![collection.folder.display().to_string(), counts.clone()];
                                if let Some(finished) = collection
                                    .maintenance
                                    .as_ref()
                                    .and_then(|maintenance| maintenance.last_finished_at)
                                {
                                    let mut arguments = FluentArgs::new();
                                    arguments
                                        .set("time", finished.format("%Y-%m-%d %H:%M").to_string());
                                    collection_metadata.push(
                                        self.localization
                                            .text_with("collections-last-scan", Some(&arguments)),
                                    );
                                }
                                ui.label(
                                    RichText::new(collection_metadata.join(" · "))
                                        .small()
                                        .color(ui.visuals().weak_text_color()),
                                );
                                if !pending_documents.is_empty() || !collection_issues.is_empty() {
                                    ui.add_space(10.0);
                                    editorial_card_kicker(
                                        ui,
                                        self.localization
                                            .text("collections-documents-attention")
                                            .to_uppercase(),
                                        crate::theme::secondary_text(ui.visuals().dark_mode),
                                    );
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(
                                                self.localization
                                                    .text("collections-table-document"),
                                            )
                                            .small()
                                            .family(crate::theme::semibold_font_family()),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    RichText::new(
                                                        self.localization
                                                            .text("collections-table-status"),
                                                    )
                                                    .small()
                                                    .family(crate::theme::semibold_font_family()),
                                                );
                                            },
                                        );
                                    });
                                    ui.separator();
                                    for source_name in &pending_documents {
                                        ui.horizontal(|ui| {
                                            ui.label(source_name);
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(
                                                            self.localization
                                                                .text("collections-status-review"),
                                                        )
                                                        .small()
                                                        .color(crate::theme::attention(
                                                            ui.visuals().dark_mode,
                                                        )),
                                                    );
                                                },
                                            );
                                        });
                                    }
                                    for issue in &collection_issues {
                                        ui.horizontal(|ui| {
                                            ui.label(&issue.source_name);
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(
                                                            self.localization
                                                                .text("review-issue-status"),
                                                        )
                                                        .small()
                                                        .color(crate::theme::warning_text(
                                                            ui.visuals().dark_mode,
                                                        )),
                                                    );
                                                },
                                            );
                                        });
                                    }
                                }
                                if !collection_issues.is_empty() {
                                    let mut issues_arguments = FluentArgs::new();
                                    issues_arguments.set("count", collection_issues.len() as i64);
                                    ui.label(
                                        RichText::new(self.localization.text_with(
                                            "review-issues-group",
                                            Some(&issues_arguments),
                                        ))
                                        .color(crate::theme::warning_text(ui.visuals().dark_mode)),
                                    );
                                    let issue_list_height =
                                        (collection_issues.len().min(6) as f32) * 56.0 + 12.0;
                                    egui::ScrollArea::vertical()
                                        .id_salt(format!("collection_issues_{}", collection.id))
                                        .max_height(issue_list_height)
                                        .auto_shrink([false; 2])
                                        .show(ui, |ui| {
                                            for issue in collection_issues.iter() {
                                                ui.add_space(2.0);
                                                wrap_rich_text(
                                                    ui,
                                                    RichText::new(format!(
                                                        "• {}",
                                                        issue.source_name
                                                    ))
                                                    .small()
                                                    .color(ui.visuals().weak_text_color()),
                                                );
                                                let cause_message = source_issue_cause_message(
                                                    &self.localization,
                                                    issue,
                                                    issue.code,
                                                )
                                                .unwrap_or_else(|| {
                                                    self.localization
                                                        .text("review-issue-cause-unknown")
                                                });
                                                wrap_rich_text(
                                                    ui,
                                                    RichText::new(format!("  {cause_message}"))
                                                        .small()
                                                        .color(ui.visuals().weak_text_color()),
                                                );
                                            }
                                        });
                                    if ui
                                        .small_button(self.localization.text("action-open"))
                                        .clicked()
                                    {
                                        self.screen = Screen::Review;
                                    }
                                }
                                if let Some(maintenance) = &collection.maintenance {
                                    let (label, color) = maintenance_status_presentation(
                                        &self.localization,
                                        maintenance.status,
                                        ui.visuals().dark_mode,
                                    );
                                    ui.colored_label(color, label);
                                    if let Some(summary) = maintenance_issue_summary(
                                        &self.localization,
                                        maintenance.issue_code.as_deref(),
                                        maintenance.issue_summary.as_deref(),
                                    ) {
                                        ui.label(summary);
                                    }
                                }
                                if let Some(state) = scan_state {
                                    ui.horizontal(|ui| {
                                        if state == CollectionScanState::Scanning {
                                            ui.spinner();
                                        }
                                        ui.label(match state {
                                            CollectionScanState::Queued => &queued,
                                            CollectionScanState::Scanning => &scanning,
                                        });
                                    });
                                }
                            });
                            ui.horizontal_wrapped(|ui| {
                                let scan_button = if scan_needs_attention {
                                    first_knowledge::primary_button(scan_label.clone())
                                } else {
                                    crate::theme::focus_button(
                                        egui::Button::new(scan_label.clone()),
                                        crate::theme::AIR_CYAN,
                                    )
                                };
                                if ui.add_enabled(scan_state.is_none(), scan_button).clicked() {
                                    self.collection_scans
                                        .insert(collection.id, CollectionScanState::Queued);
                                    self.knowledge.collection_scan_started(collection.id);
                                    self.worker
                                        .send(WorkerCommand::RescanCollection(collection.id));
                                }
                                if ui.button(&relink).clicked()
                                    && let Some(folder) = rfd::FileDialog::new().pick_folder()
                                {
                                    self.worker.send(WorkerCommand::RelinkCollection {
                                        collection_id: collection.id,
                                        folder,
                                    });
                                }
                            });
                        });
                        ui.collapsing(
                            self.localization.text("collections-sharing-details"),
                            |ui| {
                                ui.separator();
                                let external_ai_before = collection.allow_external_ai;
                                let peer_changed = ui
                                    .checkbox(&mut collection.peer_shareable, &share_peers)
                                    .changed();
                                let external_ai_changed = ui
                                    .checkbox(&mut collection.allow_external_ai, &allow_chat)
                                    .changed();
                                let public_before = collection.internet_public;
                                let public_response = ui.checkbox(
                                    &mut collection.internet_public,
                                    self.localization.text("collections-public-network"),
                                );
                                let public_changed = public_response.changed();
                                if !public_before && collection.internet_public {
                                    collection.internet_public = false;
                                    requested_public_confirmation =
                                        Some((collection.id, public_response.id));
                                }
                                let external_ai_change = classify_external_ai_policy_change(
                                    external_ai_before,
                                    collection.allow_external_ai,
                                );
                                if external_ai_change == ExternalAiPolicyChange::ConfirmEnable {
                                    collection.allow_external_ai = false;
                                    requested_external_ai_confirmation = Some(collection.id);
                                }
                                collection.local_only = !collection.peer_shareable
                                    && !collection.allow_external_ai
                                    && !collection.internet_public;
                                if collection.local_only {
                                    ui.label(RichText::new(&local_only).small().color(
                                        crate::theme::verified_text(ui.visuals().dark_mode),
                                    ));
                                }
                                if collection.allow_external_ai {
                                    ui.colored_label(
                                        crate::theme::warning_text(ui.visuals().dark_mode),
                                        &cloud_warning,
                                    );
                                }
                                if collection.internet_public {
                                    let announcement_times = match collection.public_announcement {
                                        PublicAnnouncementStatusView::Offline => {
                                            ui.label(
                                                self.localization.text(
                                                    "collections-public-announcement-offline",
                                                ),
                                            );
                                            None
                                        }
                                        PublicAnnouncementStatusView::Advertised {
                                            accepted_indexes,
                                            last_announced_at,
                                            expires_at,
                                        } => {
                                            let mut status_args = FluentArgs::new();
                                            status_args.set(
                                                "indexes",
                                                i64::try_from(accepted_indexes).unwrap_or(i64::MAX),
                                            );
                                            ui.label(self.localization.text_with(
                                                "collections-public-announcement-online",
                                                Some(&status_args),
                                            ));
                                            Some((last_announced_at, expires_at))
                                        }
                                        PublicAnnouncementStatusView::Expired {
                                            last_announced_at,
                                            expires_at,
                                        } => {
                                            ui.colored_label(
                                                crate::theme::warning_text(ui.visuals().dark_mode),
                                                self.localization.text(
                                                    "collections-public-announcement-expired",
                                                ),
                                            );
                                            Some((last_announced_at, expires_at))
                                        }
                                    };
                                    if let Some((last, expiry)) = announcement_times {
                                        let mut last_args = FluentArgs::new();
                                        last_args.set(
                                            "timestamp",
                                            last.format("%Y-%m-%d %H:%M").to_string(),
                                        );
                                        ui.label(
                                            RichText::new(self.localization.text_with(
                                                "collections-public-last-renewal",
                                                Some(&last_args),
                                            ))
                                            .small()
                                            .color(ui.visuals().weak_text_color()),
                                        );
                                        let mut expiry_args = FluentArgs::new();
                                        expiry_args.set(
                                            "timestamp",
                                            expiry.format("%Y-%m-%d %H:%M").to_string(),
                                        );
                                        ui.label(
                                            RichText::new(self.localization.text_with(
                                                "collections-public-expiry",
                                                Some(&expiry_args),
                                            ))
                                            .small()
                                            .color(ui.visuals().weak_text_color()),
                                        );
                                    }
                                    ui.label(
                                        self.localization.text("collections-public-description"),
                                    );
                                    ui.text_edit_multiline(&mut collection.public_description);
                                    ui.label(
                                        self.localization.text("collections-public-languages"),
                                    );
                                    ui.text_edit_singleline(&mut collection.public_languages);
                                    if ui
                                        .button(
                                            self.localization
                                                .text("collections-public-profile-save"),
                                        )
                                        .clicked()
                                    {
                                        let languages = collection
                                            .public_languages
                                            .split(',')
                                            .map(str::trim)
                                            .filter(|language| !language.is_empty())
                                            .map(str::to_owned)
                                            .collect();
                                        self.worker.send(
                                            WorkerCommand::UpdatePublicCollectionProfile {
                                                collection_id: collection.id,
                                                description: collection.public_description.clone(),
                                                languages,
                                            },
                                        );
                                    }
                                }
                                let external_ai_applies = external_ai_changed
                                    && external_ai_change == ExternalAiPolicyChange::ApplyDisable;
                                let public_disable =
                                    public_changed && public_before && !collection.internet_public;
                                if peer_changed || external_ai_applies || public_disable {
                                    self.worker.send(WorkerCommand::UpdateCollectionPolicy {
                                        collection_id: collection.id,
                                        local_only: collection.local_only,
                                        peer_shareable: collection.peer_shareable,
                                        allow_external_ai: collection.allow_external_ai,
                                        internet_public: collection.internet_public,
                                    });
                                }
                            },
                        );
                    });
                });
                ui.separator();
                ui.add_space(14.0);
            }
        });
        ui.add_space(30.0);
        ui.heading(
            RichText::new(self.localization.text("collections-new"))
                .size(20.0)
                .family(crate::theme::semibold_font_family()),
        );
        ui.label(
            RichText::new(self.localization.text("collections-new-body"))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let name_label = ui.label(self.localization.text("collections-name"));
            ui.text_edit_singleline(&mut self.new_collection_name)
                .labelled_by(name_label.id);
            if ui
                .button(self.localization.text("collections-choose-folder"))
                .clicked()
            {
                self.new_collection_folder = rfd::FileDialog::new().pick_folder();
            }
            let enabled =
                !self.new_collection_name.trim().is_empty() && self.new_collection_folder.is_some();
            if ui
                .add_enabled(
                    enabled,
                    first_knowledge::primary_button(
                        self.localization.text("collections-create-scan"),
                    ),
                )
                .clicked()
                && let Some(folder) = self.new_collection_folder.take()
            {
                self.worker.send(WorkerCommand::AddCollection {
                    name: self.new_collection_name.trim().to_owned(),
                    folder,
                });
                self.new_collection_name.clear();
            }
        });
        if let Some(path) = &self.new_collection_folder {
            wrap_monospace(ui, path.display().to_string());
        }
        if let Some(collection_id) = requested_external_ai_confirmation {
            self.external_ai_confirmation = Some(collection_id);
        }
        if let Some((collection_id, return_focus)) = requested_public_confirmation {
            self.public_collection_confirmation = Some(collection_id);
            self.public_confirmation_return_focus = Some(return_focus);
        }
        self.external_ai_confirmation_window(ui.ctx());
        self.public_collection_confirmation_window(ui.ctx());
    }

    fn external_ai_confirmation_window(&mut self, context: &egui::Context) {
        let Some(collection_id) = self.external_ai_confirmation else {
            return;
        };
        let Some(collection_name) = self
            .collections
            .iter()
            .find(|collection| collection.id == collection_id)
            .map(|collection| collection.name.clone())
        else {
            self.external_ai_confirmation = None;
            return;
        };
        let title = self.localization.text("collections-chat-confirm-title");
        let body = self.localization.text("collections-chat-confirm-body");
        let warning = self.localization.text("collections-cloud-warning");
        let cancel = self.localization.text("action-cancel");
        let confirm = self.localization.text("action-confirm");
        let mut decision = None;
        egui::Window::new(title)
            .id(egui::Id::new("external_ai_collection_confirmation"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.heading(collection_name);
                ui.label(body);
                ui.colored_label(crate::theme::warning_text(ui.visuals().dark_mode), warning);
                ui.horizontal(|ui| {
                    if ui.button(cancel).clicked() {
                        decision = Some(false);
                    }
                    if ui.button(confirm).clicked() {
                        decision = Some(true);
                    }
                });
            });
        let Some(confirmed) = decision else {
            return;
        };
        self.external_ai_confirmation = None;
        if !confirmed {
            return;
        }
        let Some(collection) = self
            .collections
            .iter_mut()
            .find(|collection| collection.id == collection_id)
        else {
            return;
        };
        collection.allow_external_ai = true;
        collection.local_only = !collection.peer_shareable;
        self.worker.send(WorkerCommand::UpdateCollectionPolicy {
            collection_id: collection.id,
            local_only: collection.local_only,
            peer_shareable: collection.peer_shareable,
            allow_external_ai: true,
            internet_public: collection.internet_public,
        });
    }

    fn public_collection_confirmation_window(&mut self, context: &egui::Context) {
        let Some(collection_id) = self.public_collection_confirmation else {
            return;
        };
        let modal_id = egui::Id::new(("public_collection_confirmation", collection_id));
        let focus_id = modal_id.with("initial_focus");
        let Some(collection_name) = self
            .collections
            .iter()
            .find(|collection| collection.id == collection_id)
            .map(|collection| collection.name.clone())
        else {
            context.data_mut(|data| data.remove::<bool>(focus_id));
            self.public_collection_confirmation = None;
            restore_modal_focus(context, self.public_confirmation_return_focus.take());
            return;
        };
        let newly_opened =
            !context.data_mut(|data| data.get_temp::<bool>(focus_id).unwrap_or(false));
        if newly_opened {
            egui::Popup::close_all(context);
            context.data_mut(|data| data.insert_temp(focus_id, true));
        }
        let mut title_args = FluentArgs::new();
        title_args.set("name", collection_name);
        let title = self
            .localization
            .text_with("collections-public-confirm-title", Some(&title_args));
        let dark_mode = context.global_style().visuals.dark_mode;
        let response = egui::Modal::new(modal_id)
            .frame(editorial_modal_frame(dark_mode))
            .backdrop_color(egui::Color32::from_black_alpha(128))
            .show(context, |ui| {
                ui.set_width(400.0);
                editorial_card_kicker(
                    ui,
                    self.localization.text("public-title").to_uppercase(),
                    crate::theme::attention(dark_mode),
                );
                ui.add_space(4.0);
                ui.heading(
                    RichText::new(title)
                        .size(20.0)
                        .family(crate::theme::semibold_font_family()),
                );
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(self.localization.text("collections-public-confirm-body"))
                        .wrap(),
                );
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(
                        self.localization
                            .text("collections-public-confirm-withdrawal"),
                    )
                    .wrap(),
                );
                ui.add_space(18.0);
                let mut decision = None;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(first_knowledge::primary_button(
                            self.localization.text("collections-public-confirm-action"),
                        ))
                        .clicked()
                    {
                        decision = Some(true);
                    }
                    let cancel = ui.add(crate::theme::ghost_button(
                        self.localization.text("action-cancel"),
                        ui.visuals().dark_mode,
                    ));
                    if newly_opened {
                        cancel.request_focus();
                    }
                    if cancel.clicked() {
                        decision = Some(false);
                    }
                });
                decision
            });
        let escaped = response.is_top_modal
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let decision = blocking_modal_decision(response.inner, escaped);
        let Some(confirmed) = decision else {
            return;
        };
        context.data_mut(|data| data.remove::<bool>(focus_id));
        self.public_collection_confirmation = None;
        restore_modal_focus(context, self.public_confirmation_return_focus.take());
        if !confirmed {
            return;
        }
        let still_publishable = self
            .collections
            .iter()
            .find(|collection| collection.id == collection_id)
            .map(|collection| (collection.needs_review_count, collection.failed_count));
        if !public_confirmation_can_commit(still_publishable) {
            self.notices.push_back((
                true,
                self.localization
                    .text("public-publish-blocked-by-attention"),
            ));
            return;
        }
        let Some(collection) = self
            .collections
            .iter_mut()
            .find(|collection| collection.id == collection_id)
        else {
            return;
        };
        collection.internet_public = true;
        collection.local_only = false;
        self.worker.send(WorkerCommand::UpdateCollectionPolicy {
            collection_id,
            local_only: false,
            peer_shareable: collection.peer_shareable,
            allow_external_ai: collection.allow_external_ai,
            internet_public: true,
        });
    }

    fn review(&mut self, ui: &mut egui::Ui) {
        self.review_content(ui);
    }

    fn review_content(&mut self, ui: &mut egui::Ui) {
        if self.reviews.is_empty() && self.source_issues.is_empty() {
            self.review_evidence.sync_selection(None, false);
            egui::Frame::new()
                .inner_margin(egui::Margin::same(52))
                .show(ui, |ui| {
                    empty_state(
                        ui,
                        &self.localization.text("review-empty-title"),
                        &self.localization.text("review-empty-body"),
                    );
                });
            return;
        }
        let issues = self.source_issues.clone();
        let mut requested_rescan = None;
        match review_layout_mode(ui.available_width()) {
            ReviewLayoutMode::CompactCompare => {
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(28))
                    .show(ui, |ui| {
                        ui.heading(
                            RichText::new(self.localization.text("review-title")).size(30.0),
                        );
                        ui.label(
                            RichText::new(self.localization.text("review-subtitle"))
                                .color(ui.visuals().weak_text_color()),
                        );
                        ui.add_space(14.0);
                        self.compact_review_selector(ui, &issues, &mut requested_rescan);
                        ui.separator();
                        self.review_comparison(ui, &issues, ReviewLayoutMode::CompactCompare);
                    });
            }
            ReviewLayoutMode::QueueCompare => {
                StripBuilder::new(ui)
                    .size(Size::exact(REVIEW_QUEUE_WIDTH))
                    .size(Size::remainder())
                    .clip(true)
                    .horizontal(|mut strip| {
                        strip.cell(|ui| {
                            self.review_queue(ui, &issues, &mut requested_rescan);
                        });
                        strip.cell(|ui| {
                            self.review_comparison(ui, &issues, ReviewLayoutMode::QueueCompare);
                        });
                    });
            }
        }
        if let Some(collection_id) = requested_rescan {
            self.collection_scans
                .insert(collection_id, CollectionScanState::Queued);
            self.knowledge.collection_scan_started(collection_id);
            self.worker
                .send(WorkerCommand::RescanCollection(collection_id));
        }
    }

    fn compact_review_selector(
        &mut self,
        ui: &mut egui::Ui,
        issues: &[SourceIssueView],
        requested_rescan: &mut Option<Uuid>,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(self.localization.text("review-document-selector"))
                    .family(crate::theme::semibold_font_family()),
            );
            let selected_text = self
                .selected_review
                .and_then(|id| self.reviews.iter().find(|item| item.concept_id == id))
                .map(|item| item.source_name.clone())
                .unwrap_or_else(|| self.localization.text("review-select-document"));
            egui::ComboBox::from_id_salt("compact_review_selector")
                .selected_text(selected_text)
                .width(230.0)
                .show_ui(ui, |ui| {
                    for item in &self.reviews {
                        ui.selectable_value(
                            &mut self.selected_review,
                            Some(item.concept_id),
                            format!("{} · {}", item.source_name, item.collection_name),
                        );
                    }
                });
            if !issues.is_empty() {
                let mut arguments = FluentArgs::new();
                arguments.set("count", issues.len() as i64);
                let title = self
                    .localization
                    .text_with("review-issues-group", Some(&arguments));
                ui.menu_button(title, |ui| {
                    ui.set_min_width(320.0);
                    egui::ScrollArea::vertical()
                        .id_salt("compact_review_issues")
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for issue in issues {
                                let scanning =
                                    self.collection_scans.contains_key(&issue.collection_id);
                                if show_review_issue(ui, &self.localization, issue, scanning) {
                                    *requested_rescan = Some(issue.collection_id);
                                }
                                ui.add_space(6.0);
                            }
                        });
                });
            }
        });
    }

    fn review_queue(
        &mut self,
        ui: &mut egui::Ui,
        issues: &[SourceIssueView],
        requested_rescan: &mut Option<Uuid>,
    ) {
        let queue = egui::Frame::new()
            .fill(crate::theme::paper(ui.visuals().dark_mode))
            .stroke(egui::Stroke::NONE)
            .inner_margin(egui::Margin::ZERO)
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                ui.add_space(36.0);
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(22, 0))
                    .show(ui, |ui| {
                        ui.heading(
                            RichText::new(self.localization.text("review-title"))
                                .size(20.0)
                                .family(crate::theme::semibold_font_family()),
                        );
                        ui.label(
                            RichText::new(self.localization.text("review-queue-subtitle"))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    });
                ui.add_space(12.0);
                let queue_height = ui.available_height().max(0.0);
                egui::ScrollArea::vertical()
                    .id_salt("review_queue")
                    .max_height(queue_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if !self.reviews.is_empty() {
                            for item in &self.reviews {
                                let selected = self.selected_review == Some(item.concept_id);
                                let response = review_queue_button(
                                    ui,
                                    &item.draft.title,
                                    &format!(
                                        "{} · {} {}",
                                        item.source_name,
                                        self.localization.text("review-revision-short"),
                                        item.source_revision
                                    ),
                                    selected,
                                );
                                if response.clicked() {
                                    self.selected_review = Some(item.concept_id);
                                }
                            }
                        }
                        if !issues.is_empty() {
                            if !self.reviews.is_empty() {
                                ui.add_space(14.0);
                            }
                            let mut arguments = FluentArgs::new();
                            arguments.set("count", issues.len() as i64);
                            egui::Frame::new()
                                .inner_margin(egui::Margin::symmetric(22, 0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(
                                            self.localization
                                                .text_with("review-issues-group", Some(&arguments)),
                                        )
                                        .family(crate::theme::semibold_font_family())
                                        .color(crate::theme::warning_text(ui.visuals().dark_mode)),
                                    );
                                    ui.add_space(4.0);
                                    for issue in issues {
                                        let scanning = self
                                            .collection_scans
                                            .contains_key(&issue.collection_id);
                                        if show_review_issue(
                                            ui,
                                            &self.localization,
                                            issue,
                                            scanning,
                                        ) {
                                            *requested_rescan = Some(issue.collection_id);
                                        }
                                        ui.add_space(6.0);
                                    }
                                });
                        }
                    });
            });
        let separator_x = queue.response.rect.right();
        ui.painter().line_segment(
            [
                egui::pos2(separator_x, queue.response.rect.top()),
                egui::pos2(separator_x, queue.response.rect.bottom()),
            ],
            egui::Stroke::new(1.0, crate::theme::border(ui.visuals().dark_mode)),
        );
    }

    fn review_comparison(
        &mut self,
        ui: &mut egui::Ui,
        issues: &[SourceIssueView],
        layout_mode: ReviewLayoutMode,
    ) {
        let Some(selected_index) = self.selected_review.and_then(|selected| {
            self.reviews
                .iter()
                .position(|item| item.concept_id == selected)
        }) else {
            self.review_evidence.sync_selection(None, false);
            let message = if issues.is_empty() {
                self.localization.text("review-select-document")
            } else {
                self.localization.text("review-only-issues")
            };
            ui.label(message);
            return;
        };
        let concept_id = self.reviews[selected_index].concept_id;
        let source_revision = self.reviews[selected_index].source_revision;
        let is_reanalyzing = self.reanalyzing_reviews.contains(&concept_id);
        if let Some(action) = self
            .review_evidence
            .sync_selection(Some((concept_id, source_revision)), is_reanalyzing)
        {
            self.send_review_evidence_action(action);
        }

        let approval_ready = self
            .review_evidence
            .approval_ready(concept_id, source_revision);
        let loading = self.review_evidence.is_loading(concept_id, source_revision);
        let error = self.review_evidence.error_for(concept_id, source_revision);
        let page = self
            .review_evidence
            .page_for(concept_id, source_revision)
            .cloned();
        let source_name = self.reviews[selected_index].source_name.clone();
        let collection_name = self.reviews[selected_index].collection_name.clone();
        let title = self.reviews[selected_index].draft.title.clone();
        let summary = if self.reviews[selected_index].draft.summary.trim().is_empty() {
            self.reviews[selected_index].draft.description.clone()
        } else {
            self.reviews[selected_index].draft.summary.clone()
        };
        let mut evidence_intent = None;
        let mut approve = false;
        let mut reject = false;
        let mut reanalyze = false;
        let mut metadata_editor_open = self.review_metadata_editor == Some(concept_id);
        let compact_actions = layout_mode == ReviewLayoutMode::CompactCompare;
        let horizontal_margin: i8 = if compact_actions { 28 } else { 52 };
        let action_bar_height = review_action_bar_height(layout_mode);
        StripBuilder::new(ui)
            .size(Size::remainder())
            .size(Size::exact(action_bar_height))
            .vertical(|mut strip| {
                strip.cell(|ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: horizontal_margin,
                            right: horizontal_margin,
                            top: if compact_actions { 24 } else { 44 },
                            bottom: 24,
                        })
                        .show(ui, |ui| {
                            let reader_height = ui.available_height().max(0.0);
                            egui::ScrollArea::vertical()
                                .id_salt(("review_reader", concept_id, source_revision))
                                .max_height(reader_height)
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    ui.set_max_width(680.0);
                                    let kicker = format!(
                                        "{} · {} · {} · {} {}",
                                        self.localization.text("review-draft-kicker"),
                                        collection_name,
                                        source_name,
                                        self.localization.text("review-revision-label"),
                                        source_revision
                                    )
                                    .to_uppercase();
                                    editorial_card_kicker(
                                        ui,
                                        kicker,
                                        crate::theme::accent_text(ui.visuals().dark_mode),
                                    );
                                    ui.heading(
                                        RichText::new(&title)
                                            .size(30.0)
                                            .family(crate::theme::semibold_font_family()),
                                    );
                                    ui.add(egui::Label::new(&summary).wrap().selectable(true));
                                    ui.add_space(22.0);
                                    if is_reanalyzing {
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label(self.localization.text("review-analyzing"));
                                        });
                                    } else {
                                        evidence_intent = show_review_evidence_panel(
                                            ui,
                                            &self.localization,
                                            concept_id,
                                            source_revision,
                                            page.as_ref(),
                                            error,
                                            loading,
                                        );
                                    }
                                    ui.add_space(20.0);
                                    ui.separator();
                                    ui.add_space(10.0);
                                    let metadata_label = if metadata_editor_open {
                                        self.localization.text("review-close-metadata")
                                    } else {
                                        self.localization.text("review-edit-metadata")
                                    };
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.button(metadata_label).clicked() {
                                            metadata_editor_open = !metadata_editor_open;
                                        }
                                        reanalyze = ui
                                            .add_enabled(
                                                self.models_ready && !is_reanalyzing,
                                                crate::theme::ghost_button(
                                                    self.localization.text("review-reanalyze"),
                                                    ui.visuals().dark_mode,
                                                )
                                                .small(),
                                            )
                                            .on_hover_text(self.localization.text(
                                                if self.models_ready {
                                                    "review-reanalyze-help"
                                                } else {
                                                    "review-model-required"
                                                },
                                            ))
                                            .clicked();
                                    });
                                    if metadata_editor_open {
                                        ui.add_space(12.0);
                                        edit_draft(
                                            ui,
                                            &self.localization,
                                            &mut self.reviews[selected_index].draft,
                                        );
                                    }
                                    scroll_newly_focused_control_into_view(ui);
                                });
                        });
                });
                strip.cell(|ui| {
                    let footer_rect = ui.available_rect_before_wrap();
                    let footer_inset = f32::from(horizontal_margin);
                    ui.painter().line_segment(
                        [
                            egui::pos2(footer_rect.left() + footer_inset, footer_rect.top()),
                            egui::pos2(footer_rect.right() - footer_inset, footer_rect.top()),
                        ],
                        egui::Stroke::new(1.0, crate::theme::border(ui.visuals().dark_mode)),
                    );
                    egui::Frame::new()
                        .stroke(egui::Stroke::NONE)
                        .inner_margin(egui::Margin {
                            left: horizontal_margin,
                            right: horizontal_margin,
                            top: 24,
                            bottom: 12,
                        })
                        .show(ui, |ui| {
                            let mut counter_args = FluentArgs::new();
                            counter_args.set("current", (selected_index + 1) as i64);
                            counter_args.set("total", self.reviews.len() as i64);
                            let counter = self
                                .localization
                                .text_with("review-draft-counter", Some(&counter_args));
                            let mut show_actions = |ui: &mut egui::Ui| {
                                approve = ui
                                    .add_enabled(
                                        approval_ready,
                                        first_knowledge::primary_button(
                                            self.localization.text("review-approve"),
                                        ),
                                    )
                                    .on_disabled_hover_text(
                                        self.localization.text("review-evidence-approval-blocked"),
                                    )
                                    .clicked();
                                ui.add_enabled(
                                    false,
                                    egui::Button::new(self.localization.text("review-open-source")),
                                )
                                .on_disabled_hover_text(
                                    self.localization.text("review-open-source-unavailable"),
                                );
                                reject = ui
                                    .add_enabled(
                                        !is_reanalyzing,
                                        crate::theme::ghost_button(
                                            self.localization.text("review-reject"),
                                            ui.visuals().dark_mode,
                                        ),
                                    )
                                    .clicked();
                            };
                            if compact_actions {
                                ui.horizontal_wrapped(|ui| show_actions(ui));
                                ui.add_space(5.0);
                                ui.horizontal(|ui| {
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(counter);
                                        },
                                    );
                                });
                            } else {
                                ui.horizontal(|ui| {
                                    show_actions(ui);
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(counter);
                                        },
                                    );
                                });
                            }
                        });
                });
            });
        self.review_metadata_editor = metadata_editor_open.then_some(concept_id);

        if let Some(action) = match evidence_intent {
            Some(ReviewEvidencePanelIntent::LoadMore) => self.review_evidence.request_more(),
            Some(ReviewEvidencePanelIntent::Retry) => self.review_evidence.retry(),
            None => None,
        } {
            self.send_review_evidence_action(action);
        }
        if approve
            && let Some(expected_review_version) = self
                .review_evidence
                .approval_version(concept_id, source_revision)
        {
            self.worker.send(WorkerCommand::Approve {
                concept_id,
                expected_review_version,
                draft: self.reviews[selected_index].draft.clone(),
            });
        }
        if reject {
            self.worker.send(WorkerCommand::Reject { concept_id });
        }
        if reanalyze {
            self.worker
                .send(WorkerCommand::ReanalyzeReview { concept_id });
        }
    }

    fn send_review_evidence_action(&self, action: ReviewEvidenceAction) {
        self.worker.send(WorkerCommand::LoadReviewEvidence {
            request_id: action.request_id,
            concept_id: action.concept_id,
            expected_source_revision: action.expected_source_revision,
            expected_review_version: action.expected_review_version,
            after_ordinal: action.after_ordinal,
        });
    }

    fn search(&mut self, ui: &mut egui::Ui) {
        ui.set_max_width(866.0);
        page_title(
            ui,
            &self.localization.text("search-title"),
            &self.localization.text("search-subtitle"),
        );
        self.search_form(ui, false, SearchSurface::Ask, true, None);
        ui.set_max_width(620.0);
        if let Some(target) = self.search_feedback(ui, true, SearchSurface::Ask) {
            self.open_search_evidence(target);
        }
    }

    fn search_state(&self, surface: SearchSurface) -> &SearchViewState {
        match surface {
            SearchSurface::Ask => &self.ask_search,
            SearchSurface::Public => &self.public_search,
        }
    }

    fn search_state_mut(&mut self, surface: SearchSurface) -> &mut SearchViewState {
        match surface {
            SearchSurface::Ask => &mut self.ask_search,
            SearchSurface::Public => &mut self.public_search,
        }
    }

    fn search_form(
        &mut self,
        ui: &mut egui::Ui,
        show_top_k: bool,
        surface: SearchSurface,
        allow_ask_public_network: bool,
        external_label: Option<egui::Id>,
    ) {
        let layout = ResponsiveLayout::from_available(ui.available_size());
        let search_running = self.active_search.is_some();
        let (question_label, placeholder, action_label, max_width, primary_action) = match surface {
            SearchSurface::Ask => (
                Some(self.localization.text("search-question")),
                self.localization.text("search-placeholder"),
                self.localization.text("search-action"),
                680.0,
                true,
            ),
            SearchSurface::Public => (
                None,
                self.localization.text("public-search-placeholder"),
                self.localization.text("public-search-action"),
                560.0,
                false,
            ),
        };
        let (response, submit_clicked) = ui
            .push_id(surface, |ui| {
                show_search_inputs(
                    ui,
                    layout,
                    self.search_state_mut(surface),
                    search_running,
                    show_top_k,
                    SearchInputText {
                        question: question_label.as_deref(),
                        external_label,
                        placeholder: &placeholder,
                        action: &action_label,
                        max_width,
                        primary_action,
                    },
                )
            })
            .inner;
        ui.add_space(4.0);
        if surface == SearchSurface::Ask && allow_ask_public_network {
            let scope = ask_scope_presentation(
                self.preferences
                    .map_or(LanPreference::Undecided, |preferences| {
                        preferences.lan_preference
                    }),
                self.peers
                    .iter()
                    .any(|peer| peer.trust == PeerTrustState::Trusted),
            );
            let scope_note = if self.search_public_network {
                "search-scope-note-public"
            } else if scope.paired_available {
                "search-scope-note-paired"
            } else {
                "search-scope-note-device"
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(self.localization.text("search-scope-searches"))
                        .size(13.0)
                        .family(crate::theme::semibold_font_family()),
                );
                editorial_tag(
                    ui,
                    &self.localization.text("search-scope-device"),
                    EditorialTagTone::Neutral,
                );
                if scope.paired_available {
                    editorial_tag(
                        ui,
                        &self.localization.text("search-scope-paired"),
                        EditorialTagTone::Neutral,
                    );
                }
                ui.checkbox(
                    &mut self.search_public_network,
                    self.localization.text("search-scope-include-public"),
                );
                ui.label(
                    RichText::new(self.localization.text(scope_note))
                        .size(13.0)
                        .color(crate::theme::secondary_text(ui.visuals().dark_mode)),
                );
            });
            ui.add_space(20.0);
        }
        let public_network = effective_public_search(
            surface == SearchSurface::Public,
            allow_ask_public_network,
            self.search_public_network,
        );
        let route_feedback_visible = self
            .active_search
            .is_some_and(|active| active.surface == surface)
            || self.search_state(surface).completed
            || self.search_state(surface).error.is_some();
        if public_network && route_feedback_visible {
            let status = match self.search_state(surface).route_kind {
                PublicRouteKind::Offline => "search-public-route-offline",
                PublicRouteKind::Relay => "search-public-route-relay",
                PublicRouteKind::Direct => "search-public-route-direct",
            };
            ui.label(self.localization.text(status));
        }
        if public_network && !self.blocked_public_publishers.is_empty() {
            ui.collapsing(
                self.localization.text("search-public-publisher-controls"),
                |ui| {
                    ui.label(self.localization.text("search-public-unblock-help"));
                    let mut unblock_publisher = None;
                    for (index, publisher_id) in self.blocked_public_publishers.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let mut arguments = FluentArgs::new();
                            arguments.set("number", (index + 1) as i64);
                            ui.label(
                                self.localization
                                    .text_with("search-public-blocked-publisher", Some(&arguments)),
                            );
                            if ui
                                .button(self.localization.text("search-public-unblock-publisher"))
                                .clicked()
                            {
                                unblock_publisher = Some(publisher_id.clone());
                            }
                        });
                    }
                    if let Some(publisher_id) = unblock_publisher {
                        self.worker.send(WorkerCommand::SetPublicPublisherBlocked {
                            publisher_id,
                            blocked: false,
                        });
                    }
                },
            );
        }
        let submit = submit_clicked
            || (!self.search_state(surface).question.trim().is_empty()
                && !search_running
                && response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if response.changed() {
            self.search_state_mut(surface).clear_feedback();
        }
        if submit {
            let request_id = Uuid::new_v4();
            let search = self.search_state_mut(surface);
            search.begin_search(public_network);
            let question = search.question.trim().to_owned();
            let top_k = search.top_k;
            self.active_search = Some(ActiveSearch {
                request_id,
                surface,
            });
            self.worker.send(WorkerCommand::Search {
                request_id,
                question,
                top_k,
                purpose: SearchPurpose::LocalAssistant,
                public_network,
            });
        }
    }

    fn search_feedback(
        &mut self,
        ui: &mut egui::Ui,
        show_empty_state: bool,
        surface: SearchSurface,
    ) -> Option<SearchEvidenceTarget> {
        let mut selected_evidence = None;
        let mut requested_public_browse = None;
        let mut open_chat_integrations = false;
        let mut inline_integration_actions = Vec::new();
        if self
            .active_search
            .is_some_and(|active| active.surface == surface)
        {
            ui.spinner();
            ui.label(self.localization.text(match surface {
                SearchSurface::Ask => "search-running",
                SearchSurface::Public => "public-search-running",
            }));
        }
        self.search_error_feedback(ui, surface);
        let search = self.search_state(surface);
        if let Some(message) = search_coverage_message(&self.localization, search.coverage) {
            ui.colored_label(crate::theme::warning_text(ui.visuals().dark_mode), message);
        }
        if show_empty_state && search.completed && search.hits.is_empty() {
            empty_state(
                ui,
                &self.localization.text("search-empty-title"),
                &self.localization.text("search-empty-body"),
            );
        }
        let results_height = ui.available_height().max(0.0);
        egui::ScrollArea::vertical()
            .id_salt(("search_results", surface))
            .max_height(results_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if surface == SearchSurface::Ask
                    && let Some(top_hit) = self.search_state(surface).hits.first()
                {
                    ui.add_space(18.0);
                    editorial_card_kicker(
                        ui,
                        self.localization.text("search-top-passage").to_uppercase(),
                        crate::theme::accent_text(ui.visuals().dark_mode),
                    );
                    ui.add_space(8.0);
                    ui.add(
                        egui::Label::new(RichText::new(&top_hit.snippet).size(17.0))
                            .wrap()
                            .selectable(true),
                    );
                    ui.add_space(18.0);
                    editorial_section_label(
                        ui,
                        self.localization.text("search-sources").to_uppercase(),
                        crate::theme::secondary_text(ui.visuals().dark_mode),
                    );
                }
                for hit in &self.search_state(surface).hits {
                    let collection_exists = self
                        .collections
                        .iter()
                        .any(|collection| collection.id == hit.collection_id);
                    let remote_device_name = self
                        .peers
                        .iter()
                        .find(|peer| peer.peer_id == hit.node_id)
                        .and_then(|peer| peer.device_name.as_deref());
                    let availability = classify_search_result(
                        &self.node_id,
                        &hit.node_id,
                        collection_exists,
                        remote_device_name,
                    );
                    let origin = search_result_origin_label(&self.localization, &availability);
                    ui.add_space(if surface == SearchSurface::Ask {
                        10.0
                    } else {
                        12.0
                    });
                    if surface == SearchSurface::Public {
                        ui.separator();
                        ui.add_space(8.0);
                    }
                    ui.push_id(("search_hit", surface, hit.rank), |ui| {
                        if surface == SearchSurface::Ask {
                            ui.horizontal_wrapped(|ui| {
                                editorial_tag(ui, &hit.rank.to_string(), EditorialTagTone::Accent);
                                ui.label(
                                    RichText::new(&hit.title)
                                        .size(14.0)
                                        .family(crate::theme::semibold_font_family()),
                                );
                                ui.label(
                                    RichText::new(format!("{} · {}", hit.heading_or_page, origin))
                                        .size(13.0)
                                        .color(ui.visuals().weak_text_color()),
                                );
                            });
                        } else {
                            let status = if matches!(
                                self.search_state(surface).route_kind,
                                PublicRouteKind::Offline
                            ) {
                                "public-status-offline"
                            } else {
                                "public-status-reachable"
                            };
                            ui.label(
                                RichText::new(self.localization.text(status))
                                    .size(11.0)
                                    .family(crate::theme::semibold_font_family())
                                    .color(if status == "public-status-reachable" {
                                        crate::theme::accent_text(ui.visuals().dark_mode)
                                    } else {
                                        crate::theme::warning_text(ui.visuals().dark_mode)
                                    }),
                            );
                            ui.heading(
                                RichText::new(format!("{}. {}", hit.rank, hit.title))
                                    .size(20.0)
                                    .family(crate::theme::semibold_font_family()),
                            );
                            ui.label(
                                RichText::new(&hit.heading_or_page)
                                    .family(crate::theme::semibold_font_family()),
                            );
                            ui.add(
                                egui::Label::new(RichText::new(&hit.snippet).size(15.0))
                                    .wrap()
                                    .selectable(true),
                            );
                        }
                        ui.horizontal_wrapped(|ui| match &availability {
                            SearchResultAvailability::LocalAvailable => {
                                ui.label(
                                    RichText::new(&origin)
                                        .small()
                                        .color(ui.visuals().weak_text_color()),
                                );
                                if ui
                                    .button(self.localization.text("search-open-wiki"))
                                    .clicked()
                                {
                                    selected_evidence = Some(SearchEvidenceTarget::from(hit));
                                }
                            }
                            SearchResultAvailability::LocalUnavailable => {
                                ui.colored_label(
                                    crate::theme::warning_text(ui.visuals().dark_mode),
                                    self.localization.text("search-local-unavailable"),
                                );
                            }
                            SearchResultAvailability::Remote { .. } => {
                                ui.label(
                                    RichText::new(&origin)
                                        .small()
                                        .color(ui.visuals().weak_text_color()),
                                );
                                if self.search_state(surface).submitted_public_network
                                    && ui
                                        .button(self.localization.text("search-browse-public"))
                                        .clicked()
                                {
                                    requested_public_browse =
                                        Some((hit.node_id.clone(), hit.collection_id));
                                }
                            }
                        });
                        ui.collapsing(self.localization.text("search-citation-details"), |ui| {
                            let mut arguments = FluentArgs::new();
                            arguments.set("revision", hit.source_revision);
                            ui.label(
                                self.localization
                                    .text_with("search-revision", Some(&arguments)),
                            );
                            wrap_monospace(
                                ui,
                                format!(
                                    "{}… · {}",
                                    &hit.source_sha256[..hit.source_sha256.len().min(12)],
                                    origin
                                ),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&hit.logical_resource_uri).monospace(),
                                )
                                .selectable(true)
                                .wrap(),
                            );
                        });
                    });
                }
                if surface == SearchSurface::Ask {
                    ui.add_space(36.0);
                    editorial_section_label(
                        ui,
                        self.localization.text("search-chat-title").to_uppercase(),
                        crate::theme::secondary_text(ui.visuals().dark_mode),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(self.localization.text("search-chat-body"))
                                .size(13.0)
                                .color(ui.visuals().weak_text_color()),
                        )
                        .wrap(),
                    );
                    ui.add_space(8.0);
                    inline_integration_actions =
                        self.integrations.show_compact(ui, &self.localization);
                    if ui
                        .add(crate::theme::ghost_button(
                            self.localization.text("search-chat-action"),
                            ui.visuals().dark_mode,
                        ))
                        .clicked()
                    {
                        open_chat_integrations = true;
                    }
                }
            });
        self.dispatch_integration_actions(inline_integration_actions);
        if open_chat_integrations {
            self.screen = Screen::Integrations;
        }
        if let Some((publisher_id, collection_id)) = requested_public_browse {
            self.public_browse_open = true;
            self.public_browse_publisher = publisher_id;
            self.public_browse_collection = Some(collection_id);
            self.public_browse_summary = None;
            self.public_browse_availability = PublicCollectionAvailability::Offline;
            self.public_browse_concepts.clear();
            self.public_browse_next_cursor = None;
            self.public_browse_error = None;
            self.request_public_browse(None);
            self.screen = Screen::Public;
        }
        selected_evidence
    }

    fn request_public_browse(&mut self, cursor: Option<String>) {
        let Some(collection_id) = self.public_browse_collection else {
            return;
        };
        let request_id = Uuid::new_v4();
        self.public_browse_request_id = Some(request_id);
        self.worker.send(WorkerCommand::BrowsePublicCollection {
            request_id,
            publisher_id: self.public_browse_publisher.clone(),
            collection_id,
            cursor,
        });
    }

    fn public_browse_detail(&mut self, ui: &mut egui::Ui) {
        if !self.public_browse_open {
            return;
        }
        let mut load_more = false;
        let mut block_publisher = false;
        ui.set_max_width(760.0);
        if ui
            .add(crate::theme::ghost_button(
                format!("← {}", self.localization.text("search-public-browse-title")),
                ui.visuals().dark_mode,
            ))
            .clicked()
        {
            self.public_browse_open = false;
            return;
        }
        ui.add_space(18.0);
        let (availability_key, route_key, availability_tone) = match self.public_browse_availability
        {
            PublicCollectionAvailability::Available(PublicRouteKind::Direct) => (
                "public-status-reachable",
                Some("search-public-route-direct"),
                EditorialTagTone::Accent,
            ),
            PublicCollectionAvailability::Available(PublicRouteKind::Relay) => (
                "public-status-reachable",
                Some("search-public-route-relay"),
                EditorialTagTone::Accent,
            ),
            PublicCollectionAvailability::Available(PublicRouteKind::Offline)
            | PublicCollectionAvailability::Offline => {
                ("public-status-offline", None, EditorialTagTone::Attention)
            }
            PublicCollectionAvailability::Expired => {
                ("public-status-expired", None, EditorialTagTone::Attention)
            }
        };
        if let Some(summary) = &self.public_browse_summary {
            editorial_card_kicker(
                ui,
                self.localization.text("public-reader-view").to_uppercase(),
                crate::theme::accent_text(ui.visuals().dark_mode),
            );
            ui.horizontal_wrapped(|ui| {
                ui.heading(
                    RichText::new(&summary.name)
                        .size(32.0)
                        .family(crate::theme::semibold_font_family()),
                );
                editorial_tag(
                    ui,
                    &self.localization.text(availability_key),
                    availability_tone,
                );
            });
            if !summary.description.is_empty() {
                ui.add(egui::Label::new(&summary.description).wrap());
            }
            let mut profile_args = FluentArgs::new();
            profile_args.set("languages", summary.languages.join(", "));
            profile_args.set("concepts", i64::from(summary.concept_count));
            ui.label(
                RichText::new(
                    self.localization
                        .text_with("search-public-collection-profile", Some(&profile_args)),
                )
                .small()
                .color(ui.visuals().weak_text_color()),
            );
        } else {
            ui.heading(self.localization.text("search-public-browse-title"));
            editorial_tag(
                ui,
                &self.localization.text(availability_key),
                availability_tone,
            );
        }
        if let Some(route_key) = route_key {
            ui.label(
                RichText::new(self.localization.text(route_key))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        }
        ui.label(
            RichText::new(self.localization.text("search-public-provenance"))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled(
                false,
                egui::Button::new(
                    self.localization
                        .text("search-public-search-this-collection"),
                ),
            )
            .on_disabled_hover_text(
                self.localization
                    .text("search-public-search-this-collection-unavailable"),
            );
            if ui
                .add(crate::theme::ghost_button(
                    self.localization.text("search-public-block-publisher"),
                    ui.visuals().dark_mode,
                ))
                .clicked()
            {
                block_publisher = true;
            }
        });
        ui.add_space(20.0);
        if self.public_browse_request_id.is_some() {
            ui.spinner();
        }
        if let Some(error) = &self.public_browse_error {
            ui.colored_label(crate::theme::error_text(ui.visuals().dark_mode), error);
        }
        for concept in &self.public_browse_concepts {
            ui.separator();
            ui.add_space(10.0);
            ui.heading(
                RichText::new(&concept.title)
                    .size(20.0)
                    .family(crate::theme::semibold_font_family()),
            );
            ui.label(
                RichText::new(format!(
                    "{} · {} · {}",
                    concept.concept_type,
                    concept.language,
                    concept.tags.join(" · ")
                ))
                .small()
                .color(ui.visuals().weak_text_color()),
            );
            let mut revision_arguments = FluentArgs::new();
            revision_arguments.set("revision", concept.source_revision);
            ui.label(
                RichText::new(
                    self.localization
                        .text_with("search-public-source-revision", Some(&revision_arguments)),
                )
                .small()
                .color(ui.visuals().weak_text_color()),
            );
            ui.add(egui::Label::new(&concept.summary).wrap());
        }
        ui.add_space(16.0);
        ui.horizontal_wrapped(|ui| {
            if self.public_browse_next_cursor.is_some()
                && self.public_browse_request_id.is_none()
                && ui
                    .button(self.localization.text("search-public-browse-more"))
                    .clicked()
            {
                load_more = true;
            }
        });
        if load_more {
            self.request_public_browse(self.public_browse_next_cursor.clone());
        }
        if block_publisher {
            let publisher_id = self.public_browse_publisher.clone();
            self.worker.send(WorkerCommand::SetPublicPublisherBlocked {
                publisher_id: publisher_id.clone(),
                blocked: true,
            });
            remove_blocked_publisher_hits(
                &mut self.ask_search,
                &mut self.public_search,
                &publisher_id,
            );
            self.public_browse_open = false;
        }
    }

    fn open_search_evidence(&mut self, target: SearchEvidenceTarget) {
        let scan_active = self.collection_scans.contains_key(&target.collection_id());
        let action = self.knowledge.open_search_evidence(target, scan_active);
        self.screen = Screen::Knowledge;
        if let Some(action) = action {
            self.send_knowledge_action(action);
        }
    }

    fn search_error_feedback(&self, ui: &mut egui::Ui, surface: SearchSurface) {
        let Some(error) = &self.search_state(surface).error else {
            return;
        };
        ui.colored_label(
            crate::theme::error_text(ui.visuals().dark_mode),
            self.localization.text("search-error-title"),
        );
        ui.collapsing(self.localization.text("technical-details"), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("search_error_details", surface))
                .max_height(88.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(egui::Label::new(error).wrap());
                });
        });
    }

    fn integrations(&mut self, ui: &mut egui::Ui) {
        ui.set_max_width(680.0);
        if ui
            .add(crate::theme::ghost_button(
                format!("← {}", self.localization.text("search-title")),
                ui.visuals().dark_mode,
            ))
            .clicked()
        {
            self.screen = Screen::Search;
            return;
        }
        ui.add_space(8.0);
        let actions = self.integrations.show(ui, &self.localization);
        self.dispatch_integration_actions(actions);
    }

    fn dispatch_integration_actions(&mut self, actions: Vec<IntegrationsUiAction>) {
        for action in actions {
            match action {
                IntegrationsUiAction::Run { request_id, action } => {
                    self.worker
                        .send(WorkerCommand::ManageChatIntegration { request_id, action });
                }
                IntegrationsUiAction::OpenCollections => self.screen = Screen::Collections,
            }
        }
    }

    fn public_network(&mut self, ui: &mut egui::Ui) {
        if self.public_browse_open {
            self.public_browse_detail(ui);
            return;
        }
        ui.set_max_width(860.0);
        ui.horizontal_wrapped(|ui| {
            ui.heading(
                RichText::new(self.localization.text("public-title"))
                    .size(32.0)
                    .family(crate::theme::semibold_font_family()),
            );
            editorial_tag(
                ui,
                &self.localization.text("public-experimental"),
                EditorialTagTone::Outline,
            );
        });
        ui.add(
            egui::Label::new(
                RichText::new(self.localization.text("public-subtitle"))
                    .size(15.0)
                    .color(crate::theme::secondary_text(ui.visuals().dark_mode)),
            )
            .wrap(),
        );
        ui.add_space(30.0);
        editorial_section_label(
            ui,
            self.localization
                .text("public-collections-title")
                .to_uppercase(),
            ui.visuals().weak_text_color(),
        );
        ui.add_space(6.0);
        if self.collections.is_empty() {
            ui.label(
                RichText::new(self.localization.text("public-collections-empty"))
                    .color(ui.visuals().weak_text_color()),
            );
        } else {
            let mut requested_public_confirmation = None;
            let mut requested_stop_publishing = None;
            for collection in &self.collections {
                ui.push_id(collection.id, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.heading(
                            RichText::new(&collection.name)
                                .size(20.0)
                                .family(crate::theme::semibold_font_family()),
                        );
                        if collection.internet_public {
                            editorial_tag(
                                ui,
                                &self.localization.text("public-visible"),
                                EditorialTagTone::Accent,
                            );
                        } else {
                            editorial_tag(
                                ui,
                                &self.localization.text("public-private"),
                                EditorialTagTone::Neutral,
                            );
                        }
                    });
                    if collection.internet_public {
                        let (status, status_color) = match collection.public_announcement {
                            PublicAnnouncementStatusView::Offline => (
                                "public-status-offline",
                                crate::theme::warning_text(ui.visuals().dark_mode),
                            ),
                            PublicAnnouncementStatusView::Advertised { .. } => (
                                "public-status-reachable",
                                crate::theme::accent_text(ui.visuals().dark_mode),
                            ),
                            PublicAnnouncementStatusView::Expired { .. } => (
                                "public-status-expired",
                                crate::theme::attention(ui.visuals().dark_mode),
                            ),
                        };
                        ui.label(
                            RichText::new(self.localization.text(status))
                                .small()
                                .family(crate::theme::semibold_font_family())
                                .color(status_color),
                        );
                        if !collection.public_description.trim().is_empty() {
                            ui.add(
                                egui::Label::new(&collection.public_description)
                                    .wrap()
                                    .selectable(false),
                            );
                        }
                        let mut concept_arguments = FluentArgs::new();
                        concept_arguments.set("count", collection.published_count);
                        let mut profile = vec![
                            self.localization
                                .text_with("knowledge-concept-count", Some(&concept_arguments)),
                        ];
                        if !collection.public_languages.trim().is_empty() {
                            profile.push(collection.public_languages.clone());
                        }
                        ui.label(
                            RichText::new(profile.join(" · "))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                        if let PublicAnnouncementStatusView::Advertised {
                            accepted_indexes, ..
                        } = collection.public_announcement
                        {
                            let mut arguments = FluentArgs::new();
                            arguments.set("indexes", accepted_indexes as i64);
                            ui.label(
                                RichText::new(self.localization.text_with(
                                    "collections-public-announcement-online",
                                    Some(&arguments),
                                ))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                            );
                        }
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add(crate::theme::ghost_button(
                                    self.localization.text("public-stop-publishing"),
                                    ui.visuals().dark_mode,
                                ))
                                .clicked()
                            {
                                requested_stop_publishing = Some(collection.id);
                            }
                        });
                    } else {
                        if collection_can_make_public(
                            collection.needs_review_count,
                            collection.failed_count,
                        ) {
                            ui.label(
                                RichText::new(self.localization.text("public-private-summary"))
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                            );
                            let make_public = ui.add(first_knowledge::primary_button(
                                self.localization.text("public-make-public"),
                            ));
                            if make_public.clicked() {
                                requested_public_confirmation =
                                    Some((collection.id, make_public.id));
                            }
                        } else {
                            ui.label(
                                RichText::new(
                                    self.localization.text("public-private-attention-summary"),
                                )
                                .small()
                                .color(crate::theme::attention(ui.visuals().dark_mode)),
                            );
                        }
                    }
                });
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(12.0);
            }
            if let Some((collection_id, return_focus)) = requested_public_confirmation {
                self.public_collection_confirmation = Some(collection_id);
                self.public_confirmation_return_focus = Some(return_focus);
            }
            if let Some(collection_id) = requested_stop_publishing
                && let Some(collection) = self
                    .collections
                    .iter_mut()
                    .find(|collection| collection.id == collection_id)
            {
                collection.internet_public = false;
                collection.local_only = !collection.peer_shareable && !collection.allow_external_ai;
                self.worker.send(WorkerCommand::UpdateCollectionPolicy {
                    collection_id,
                    local_only: collection.local_only,
                    peer_shareable: collection.peer_shareable,
                    allow_external_ai: collection.allow_external_ai,
                    internet_public: false,
                });
            }
        }
        if ui
            .button(self.localization.text("public-manage-collections"))
            .clicked()
        {
            self.screen = Screen::Collections;
        }
        if self.enabled_community_federation_index_count > 0 {
            ui.add_space(24.0);
            egui::Frame::new()
                .fill(crate::theme::surface(ui.visuals().dark_mode))
                .stroke(egui::Stroke::new(
                    1.0,
                    crate::theme::attention(ui.visuals().dark_mode),
                ))
                .corner_radius(egui::CornerRadius::same(2))
                .inner_margin(egui::Margin::same(18))
                .show(ui, |ui| {
                    editorial_card_kicker(
                        ui,
                        self.localization
                            .text("public-community-indexes-kicker")
                            .to_uppercase(),
                        crate::theme::attention(ui.visuals().dark_mode),
                    );
                    ui.add_space(4.0);
                    ui.heading(
                        RichText::new(
                            self.localization
                                .text("public-community-indexes-recovery-title"),
                        )
                        .size(18.0)
                        .family(crate::theme::semibold_font_family()),
                    );
                    let mut arguments = FluentArgs::new();
                    arguments.set(
                        "count",
                        self.enabled_community_federation_index_count as i64,
                    );
                    ui.add(
                        egui::Label::new(
                            self.localization.text_with(
                                "public-community-indexes-recovery-body",
                                Some(&arguments),
                            ),
                        )
                        .wrap(),
                    );
                    ui.add_space(10.0);
                    let action = ui.add_enabled(
                        self.community_indexes_disable_request_id.is_none(),
                        crate::theme::ghost_button(
                            self.localization
                                .text("public-community-indexes-disable-action"),
                            ui.visuals().dark_mode,
                        ),
                    );
                    if action.clicked() {
                        self.community_indexes_confirmation = true;
                        self.community_indexes_confirmation_return_focus = Some(action.id);
                    }
                    if self.community_indexes_disable_request_id.is_some() {
                        ui.spinner();
                    }
                });
        }
        ui.add_space(24.0);
        let public_search_label = editorial_section_label(
            ui,
            self.localization
                .text("public-discover-title")
                .to_uppercase(),
            crate::theme::secondary_text(ui.visuals().dark_mode),
        );
        ui.label(
            RichText::new(self.localization.text("public-discover-body"))
                .color(ui.visuals().weak_text_color()),
        );
        self.search_form(
            ui,
            false,
            SearchSurface::Public,
            false,
            Some(public_search_label.id),
        );
        ui.scope(|ui| {
            ui.set_max_width(680.0);
            if let Some(target) = self.search_feedback(ui, true, SearchSurface::Public) {
                self.open_search_evidence(target);
            }
        });
        self.public_collection_confirmation_window(ui.ctx());
    }

    fn community_indexes_confirmation_window(&mut self, context: &egui::Context) {
        if !self.community_indexes_confirmation {
            return;
        }
        if self.enabled_community_federation_index_count == 0 {
            self.community_indexes_confirmation = false;
            restore_modal_focus(
                context,
                self.community_indexes_confirmation_return_focus.take(),
            );
            return;
        }

        let modal_id = egui::Id::new("community_indexes_disable_confirmation");
        let focus_id = modal_id.with("initial_focus");
        let newly_opened =
            !context.data_mut(|data| data.get_temp::<bool>(focus_id).unwrap_or(false));
        if newly_opened {
            egui::Popup::close_all(context);
            context.data_mut(|data| data.insert_temp(focus_id, true));
        }
        let dark_mode = context.global_style().visuals.dark_mode;
        let mut body_arguments = FluentArgs::new();
        body_arguments.set(
            "count",
            self.enabled_community_federation_index_count as i64,
        );
        let response = egui::Modal::new(modal_id)
            .frame(editorial_modal_frame(dark_mode))
            .backdrop_color(egui::Color32::from_black_alpha(128))
            .show(context, |ui| {
                ui.set_width(400.0);
                editorial_card_kicker(
                    ui,
                    self.localization
                        .text("public-community-indexes-kicker")
                        .to_uppercase(),
                    crate::theme::attention(dark_mode),
                );
                ui.add_space(4.0);
                ui.heading(
                    RichText::new(
                        self.localization
                            .text("public-community-indexes-confirm-title"),
                    )
                    .size(20.0)
                    .family(crate::theme::semibold_font_family()),
                );
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(self.localization.text_with(
                        "public-community-indexes-confirm-body",
                        Some(&body_arguments),
                    ))
                    .wrap(),
                );
                ui.add_space(18.0);
                let mut decision = None;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(first_knowledge::primary_button(
                            self.localization
                                .text("public-community-indexes-confirm-action"),
                        ))
                        .clicked()
                    {
                        decision = Some(true);
                    }
                    let cancel = ui.add(crate::theme::ghost_button(
                        self.localization.text("action-cancel"),
                        ui.visuals().dark_mode,
                    ));
                    if newly_opened {
                        cancel.request_focus();
                    }
                    if cancel.clicked() {
                        decision = Some(false);
                    }
                });
                decision
            });
        let escaped = response.is_top_modal
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let Some(confirmed) = blocking_modal_decision(response.inner, escaped) else {
            return;
        };
        context.data_mut(|data| data.remove::<bool>(focus_id));
        self.community_indexes_confirmation = false;
        restore_modal_focus(
            context,
            self.community_indexes_confirmation_return_focus.take(),
        );
        if !confirmed {
            return;
        }

        let request_id = Uuid::new_v4();
        self.community_indexes_disable_request_id = Some(request_id);
        self.worker
            .send(WorkerCommand::DisableCommunityFederationIndexes { request_id });
    }

    fn refresh_integrations_if_needed(&mut self) {
        let Some(IntegrationsUiAction::Run { request_id, action }) =
            self.integrations.refresh_if_idle()
        else {
            return;
        };
        self.worker
            .send(WorkerCommand::ManageChatIntegration { request_id, action });
    }

    fn nodes(&mut self, ui: &mut egui::Ui) {
        ui.set_max_width(600.0);
        page_title(
            ui,
            &self.localization.text("devices-title"),
            &self.localization.text("devices-subtitle"),
        );
        self.connectivity_panel(ui);
        ui.add_space(10.0);
        ui.collapsing(self.localization.text("devices-manual-advanced"), |ui| {
            ui.scope(|ui| {
                if !self.lan_local_addresses.is_empty() {
                    ui.label(self.localization.text("devices-this-address"));
                    for address in &self.lan_local_addresses {
                        ui.horizontal_wrapped(|ui| {
                            wrap_monospace(ui, address);
                            if ui
                                .small_button(self.localization.text("action-copy"))
                                .clicked()
                            {
                                ui.ctx().copy_text(address.clone());
                            }
                        });
                    }
                    ui.add_space(8.0);
                }
                let manual_address = parse_manual_ipv4_address(&self.manual_multiaddress);
                let manual_connection_available = self.preferences.is_some_and(|preferences| {
                    preferences.lan_preference == LanPreference::Enabled
                }) && self.lan_listener
                    == LanListenerView::Listening;
                ui.horizontal_wrapped(|ui| {
                    let field_width = (ui.available_width() - 110.0).clamp(180.0, 560.0);
                    ui.add_sized(
                        [field_width, 28.0],
                        egui::TextEdit::singleline(&mut self.manual_multiaddress)
                            .hint_text("/ip4/192.168.1.20/tcp/12345/p2p/12D3Koo…"),
                    );
                    if ui
                        .add_enabled(
                            manual_connection_available && manual_address.is_some(),
                            egui::Button::new(self.localization.text("action-connect")),
                        )
                        .clicked()
                        && let Some(address) = &manual_address
                    {
                        self.worker.send(WorkerCommand::Dial {
                            address: address.to_string(),
                        });
                    }
                });
                if !self.manual_multiaddress.trim().is_empty() && manual_address.is_none() {
                    ui.colored_label(
                        crate::theme::error_text(ui.visuals().dark_mode),
                        self.localization.text("devices-manual-invalid"),
                    );
                }
                if !manual_connection_available {
                    ui.label(self.localization.text("devices-manual-requires-lan"));
                }
            });
        });
        ui.add_space(10.0);
        if self.peers.is_empty() {
            empty_state(
                ui,
                &self.localization.text("devices-empty-title"),
                &self.localization.text("devices-empty-body"),
            );
        }
        let nearby_device = self.localization.text("devices-nearby");
        let technical_details = self.localization.text("action-details");
        let pair = self.localization.text("devices-pair");
        let revoke = self.localization.text("devices-revoke");
        let blocked_message = self.localization.text("devices-blocked-message");
        let pair_again = self.localization.text("devices-pair-again");
        let list_height = connections_peer_list_height(ui.available_height());
        egui::ScrollArea::vertical()
            .id_salt("peer_list")
            .max_height(list_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for peer in &mut self.peers {
                    ui.push_id(&peer.peer_id, |ui| {
                        ui.separator();
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.heading(
                                    RichText::new(
                                        peer.device_name.as_deref().unwrap_or(&nearby_device),
                                    )
                                    .size(24.0)
                                    .family(crate::theme::semibold_font_family()),
                                );
                                ui.collapsing(&technical_details, |ui| {
                                    wrap_monospace(ui, &peer.peer_id);
                                    wrap_monospace(ui, &peer.address);
                                });
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.vertical(|ui| {
                                        ui.label(peer_trust_label(&self.localization, peer.trust));
                                        ui.small(peer_activity_label(
                                            &self.localization,
                                            peer.trust,
                                            peer.activity,
                                        ));
                                    });
                                },
                            );
                        });
                        if should_present_pairing_controls(peer.activity) {
                            ui.label(
                                RichText::new(self.localization.text("peer-activity-pairing"))
                                    .color(crate::theme::accent_text(ui.visuals().dark_mode)),
                            );
                        } else {
                            match peer.trust {
                                PeerTrustState::Unpaired => {
                                    let pair_button =
                                        ui.add(first_knowledge::primary_button(pair.clone()));
                                    if pair_button.clicked() {
                                        self.pairing_confirmation_return_focus =
                                            Some(pair_button.id);
                                        self.worker.send(WorkerCommand::Pair {
                                            peer_id: peer.peer_id.clone(),
                                        });
                                    }
                                }
                                PeerTrustState::Trusted => {
                                    for collection in &self.collections {
                                        if collection.local_only || !collection.peer_shareable {
                                            continue;
                                        }
                                        let mut granted =
                                            peer.granted_collections.contains(&collection.id);
                                        let mut arguments = FluentArgs::new();
                                        arguments.set("name", collection.name.as_str());
                                        if ui
                                            .checkbox(
                                                &mut granted,
                                                self.localization
                                                    .text_with("devices-grant", Some(&arguments)),
                                            )
                                            .changed()
                                        {
                                            if granted {
                                                peer.granted_collections.insert(collection.id);
                                            } else {
                                                peer.granted_collections.remove(&collection.id);
                                            }
                                            self.worker.send(WorkerCommand::GrantCollection {
                                                peer_id: peer.peer_id.clone(),
                                                collection_id: collection.id,
                                                granted,
                                            });
                                        }
                                    }
                                    if ui
                                        .add(crate::theme::ghost_button(
                                            revoke.as_str(),
                                            ui.visuals().dark_mode,
                                        ))
                                        .clicked()
                                    {
                                        self.worker.send(WorkerCommand::RevokePeer {
                                            peer_id: peer.peer_id.clone(),
                                        });
                                    }
                                }
                                PeerTrustState::Blocked => {
                                    ui.colored_label(
                                        crate::theme::error_text(ui.visuals().dark_mode),
                                        &blocked_message,
                                    );
                                    let pair_button =
                                        ui.add(first_knowledge::primary_button(pair_again.clone()));
                                    if pair_button.clicked() {
                                        self.pairing_confirmation_return_focus =
                                            Some(pair_button.id);
                                        self.worker.send(WorkerCommand::Pair {
                                            peer_id: peer.peer_id.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    });
                }
                scroll_newly_focused_control_into_view(ui);
            });
        ui.add_space(20.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(self.localization.text("connections-chat-summary"));
            if ui
                .button(self.localization.text("connections-open-chats"))
                .clicked()
            {
                self.screen = Screen::Integrations;
            }
        });
        self.pairing_window(ui.ctx());
        self.firewall_confirmation(ui.ctx());
    }

    fn pairing_window(&mut self, context: &egui::Context) {
        let active_pairings = self
            .peers
            .iter()
            .filter(|peer| should_present_pairing_controls(peer.activity))
            .map(|peer| peer.peer_id.clone())
            .collect::<HashSet<_>>();
        if let Some(previous_peer) = self.pairing_modal_peer.clone()
            && !active_pairings.contains(&previous_peer)
        {
            let focus_id =
                egui::Id::new(("pairing_confirmation", &previous_peer)).with("initial_focus");
            context.data_mut(|data| data.remove::<bool>(focus_id));
            self.pairing_modal_peer = None;
            restore_modal_focus(context, self.pairing_confirmation_return_focus.take());
        }
        self.pairing_decisions_pending
            .retain(|peer_id| active_pairings.contains(peer_id));
        let unknown_device = self.localization.text("devices-nearby");
        let Some((peer_id, device_name, words)) = self.peers.iter().find_map(|peer| {
            (should_present_pairing_controls(peer.activity)
                && !self.pairing_decisions_pending.contains(&peer.peer_id))
            .then(|| {
                peer.sas_words.as_ref().map(|words| {
                    (
                        peer.peer_id.clone(),
                        peer.device_name
                            .clone()
                            .unwrap_or_else(|| unknown_device.clone()),
                        words.clone(),
                    )
                })
            })
            .flatten()
        }) else {
            return;
        };
        self.pairing_modal_peer = Some(peer_id.clone());
        let modal_id = egui::Id::new(("pairing_confirmation", &peer_id));
        let focus_id = modal_id.with("initial_focus");
        let newly_opened =
            !context.data_mut(|data| data.get_temp::<bool>(focus_id).unwrap_or(false));
        if newly_opened {
            egui::Popup::close_all(context);
            context.data_mut(|data| data.insert_temp(focus_id, true));
        }
        let dark_mode = context.global_style().visuals.dark_mode;
        let response = egui::Modal::new(modal_id)
            .frame(editorial_modal_frame(dark_mode))
            .backdrop_color(egui::Color32::from_black_alpha(128))
            .show(context, |ui| {
                ui.set_width(400.0);
                editorial_card_kicker(
                    ui,
                    self.localization
                        .text("devices-pairing-title")
                        .to_uppercase(),
                    crate::theme::accent_text(dark_mode),
                );
                ui.add_space(4.0);
                ui.heading(
                    RichText::new(self.localization.text("devices-code-compare"))
                        .size(20.0)
                        .family(crate::theme::semibold_font_family()),
                );
                ui.add_space(14.0);
                for row in words.chunks(3) {
                    ui.label(
                        RichText::new(row.join(" · "))
                            .size(24.0)
                            .family(crate::theme::semibold_font_family()),
                    );
                }
                ui.add_space(10.0);
                let mut args = FluentArgs::new();
                args.set("device", device_name.as_str());
                ui.add(
                    egui::Label::new(
                        self.localization
                            .text_with("devices-pairing-warning", Some(&args)),
                    )
                    .wrap(),
                );
                ui.add_space(18.0);
                let mut accepted = None;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(first_knowledge::primary_button(
                            self.localization.text("devices-code-matches"),
                        ))
                        .clicked()
                    {
                        accepted = Some(true);
                    }
                    let cancel = ui.add(crate::theme::ghost_button(
                        self.localization.text("action-cancel"),
                        ui.visuals().dark_mode,
                    ));
                    if newly_opened {
                        cancel.request_focus();
                    }
                    if cancel.clicked() {
                        accepted = Some(false);
                    }
                });
                accepted
            });
        let escaped = response.is_top_modal
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let accepted = blocking_modal_decision(response.inner, escaped);
        if let Some(accepted) = accepted {
            context.data_mut(|data| data.remove::<bool>(focus_id));
            self.pairing_modal_peer = None;
            restore_modal_focus(context, self.pairing_confirmation_return_focus.take());
            self.pairing_decisions_pending.insert(peer_id.clone());
            self.worker
                .send(WorkerCommand::ConfirmPairing { peer_id, accepted });
        }
    }

    fn connectivity_panel(&mut self, ui: &mut egui::Ui) {
        let preference = self
            .preferences
            .map_or(LanPreference::Undecided, |preferences| {
                preferences.lan_preference
            });
        ui.scope(|ui| {
            ui.heading(
                RichText::new(self.localization.text("connectivity-title"))
                    .size(24.0)
                    .family(crate::theme::semibold_font_family()),
            );
            match preference {
                LanPreference::Undecided => {
                    ui.label(self.localization.text("connectivity-undecided"));
                    ui.horizontal(|ui| {
                        if ui
                            .button(self.localization.text("connectivity-enable"))
                            .clicked()
                        {
                            self.update_preferences(
                                |preferences| preferences.lan_preference = LanPreference::Enabled,
                                false,
                            );
                        }
                        if ui
                            .button(self.localization.text("connectivity-local-only"))
                            .clicked()
                        {
                            self.update_preferences(
                                |preferences| preferences.lan_preference = LanPreference::Disabled,
                                false,
                            );
                        }
                    });
                }
                LanPreference::Disabled => {
                    ui.label(self.localization.text("connectivity-disabled"));
                    if ui
                        .button(self.localization.text("connectivity-activate"))
                        .clicked()
                    {
                        self.update_preferences(
                            |preferences| preferences.lan_preference = LanPreference::Enabled,
                            false,
                        );
                    }
                }
                LanPreference::Enabled => {
                    let readiness = self.readiness_view();
                    let (status, color) = readiness_status_presentation(
                        &self.localization,
                        readiness.status(ReadinessComponent::Lan),
                        ui.visuals().dark_mode,
                    );
                    ui.colored_label(color, status);
                    if let Some(operation) = self.firewall_operation {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(self.localization.text(match operation {
                                FirewallOperationView::AwaitingWindows => {
                                    "connectivity-firewall-awaiting-windows"
                                }
                                FirewallOperationView::TakingLonger => {
                                    "connectivity-firewall-taking-longer"
                                }
                            }));
                        });
                    }
                    match self.connectivity_platform {
                        Some(snapshot)
                            if snapshot.network_profile == NetworkProfileState::Public =>
                        {
                            ui.colored_label(
                                crate::theme::error_text(ui.visuals().dark_mode),
                                self.localization.text("connectivity-public-network"),
                            );
                            if ui
                                .button(
                                    self.localization.text("connectivity-open-network-settings"),
                                )
                                .clicked()
                            {
                                ui.ctx()
                                    .open_url(egui::OpenUrl::same_tab("ms-settings:network-status"));
                            }
                        }
                        Some(snapshot)
                            if matches!(
                                snapshot.firewall,
                                FirewallDiagnosticState::FirewallDisabled
                                    | FirewallDiagnosticState::BlockAllInbound
                            ) =>
                        {
                            let message = if snapshot.firewall
                                == FirewallDiagnosticState::FirewallDisabled
                            {
                                "connectivity-firewall-disabled"
                            } else {
                                "connectivity-firewall-block-all-inbound"
                            };
                            ui.colored_label(
                                crate::theme::error_text(ui.visuals().dark_mode),
                                self.localization.text(message),
                            );
                            if ui
                                .button(
                                    self.localization
                                        .text("connectivity-open-firewall-settings"),
                                )
                                .clicked()
                            {
                                ui.ctx().open_url(egui::OpenUrl::same_tab(
                                    "ms-settings:windowsdefender",
                                ));
                            }
                        }
                        Some(snapshot)
                            if snapshot.firewall == FirewallDiagnosticState::RulesMissing
                                && snapshot.firewall_helper.can_request_elevation() =>
                        {
                            ui.label(self.localization.text("connectivity-firewall-needed"));
                            if ui
                                .add_enabled(
                                    self.connectivity_request_id.is_none(),
                                    egui::Button::new(
                                        self.localization.text("connectivity-configure-firewall"),
                                    ),
                                )
                                .clicked()
                            {
                                self.firewall_confirmation = true;
                            }
                        }
                        Some(snapshot)
                            if snapshot.firewall == FirewallDiagnosticState::RulesMissing =>
                        {
                            ui.colored_label(
                                crate::theme::warning_text(ui.visuals().dark_mode),
                                self.localization
                                    .text("connectivity-firewall-helper-repair"),
                            );
                        }
                        Some(snapshot)
                            if firewall_state_offers_advanced_recovery(snapshot.firewall) =>
                        {
                            let warning = if snapshot.firewall
                                == FirewallDiagnosticState::LegacyExposure
                            {
                                "connectivity-firewall-legacy-exposure"
                            } else {
                                "connectivity-issue-firewall-conflict"
                            };
                            ui.colored_label(
                                crate::theme::error_text(ui.visuals().dark_mode),
                                self.localization.text(warning),
                            );
                            ui.label(
                                self.localization
                                    .text("connectivity-firewall-advanced-guidance"),
                            );
                            if ui
                                .add_enabled(
                                    self.connectivity_request_id.is_none(),
                                    egui::Button::new(
                                        self.localization
                                            .text("connectivity-open-advanced-firewall"),
                                    ),
                                )
                                .clicked()
                            {
                                let request_id = Uuid::new_v4();
                                self.connectivity_request_id = Some(request_id);
                                self.worker
                                    .send(WorkerCommand::OpenAdvancedFirewall { request_id });
                            }
                        }
                        Some(snapshot)
                            if snapshot.firewall == FirewallDiagnosticState::ManagedPolicy =>
                        {
                            ui.colored_label(
                                crate::theme::warning_text(ui.visuals().dark_mode),
                                self.localization.text("connectivity-admin-needed"),
                            );
                        }
                        _ if self.lan_listener == LanListenerView::Starting => {
                            ui.spinner();
                            ui.label(self.localization.text("connectivity-starting"));
                        }
                        _ if self.lan_listener == LanListenerView::Failed
                            || self.lan_discovery == LanDiscoveryView::Failed =>
                        {
                            ui.colored_label(
                                crate::theme::error_text(ui.visuals().dark_mode),
                                self.localization.text("connectivity-failed"),
                            );
                            #[cfg(target_os = "macos")]
                            if ui
                                .button(
                                    self.localization
                                        .text("connectivity-open-local-network-settings"),
                                )
                                .clicked()
                            {
                                ui.ctx().open_url(egui::OpenUrl::same_tab(
                                    "x-help-action://openPrefPane?bundleId=com.apple.settings.PrivacySecurity.extension",
                                ));
                            }
                        }
                        _ if connectivity_runtime_is_active(
                            self.connectivity_platform,
                            self.lan_listener,
                            self.lan_discovery,
                        ) => {
                            ui.label(self.localization.text("connectivity-active"));
                        }
                        _ => {
                            ui.colored_label(
                                crate::theme::warning_text(ui.visuals().dark_mode),
                                self.localization.text("connectivity-not-ready"),
                            );
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.connectivity_request_id.is_none(),
                                egui::Button::new(
                                    self.localization.text("connectivity-check-again"),
                                ),
                            )
                            .clicked()
                        {
                            self.request_connectivity_refresh();
                        }
                        if ui
                            .add(crate::theme::ghost_button(
                                self.localization.text("connectivity-disable"),
                                ui.visuals().dark_mode,
                            ))
                            .clicked()
                        {
                            self.update_preferences(
                                |preferences| preferences.lan_preference = LanPreference::Disabled,
                                false,
                            );
                        }
                    });
                }
            }
        });
    }

    fn request_connectivity_refresh(&mut self) {
        let request_id = Uuid::new_v4();
        self.connectivity_request_id = Some(request_id);
        self.worker
            .send(WorkerCommand::RefreshConnectivity { request_id });
    }

    fn firewall_confirmation(&mut self, context: &egui::Context) {
        if !self.firewall_confirmation {
            return;
        }
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(self.localization.text("firewall-dialog-title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.label(self.localization.text("firewall-dialog-intro"));
                ui.label(self.localization.text("firewall-dialog-tcp"));
                ui.label(self.localization.text("firewall-dialog-udp"));
                ui.label(self.localization.text("firewall-dialog-exclusions"));
                ui.horizontal(|ui| {
                    if ui
                        .button(self.localization.text("action-continue"))
                        .clicked()
                    {
                        confirm = true;
                    }
                    if ui.button(self.localization.text("action-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if confirm {
            self.firewall_confirmation = false;
            let preference = self
                .preferences
                .map_or(LanPreference::Undecided, |preferences| {
                    preferences.lan_preference
                });
            if firewall_configuration_is_current(
                preference,
                self.connectivity_platform,
                self.preference_request_id.is_some() || self.connectivity_request_id.is_some(),
            ) {
                let request_id = Uuid::new_v4();
                self.connectivity_request_id = Some(request_id);
                self.worker.send(WorkerCommand::ConfigureFirewall {
                    request_id,
                    install: true,
                });
            } else {
                self.notices
                    .push_back((true, "connectivity_state_changed".to_owned()));
                if self.connectivity_request_id.is_none() {
                    self.request_connectivity_refresh();
                }
            }
        } else if cancel {
            self.firewall_confirmation = false;
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.set_max_width(560.0);
        page_title(
            ui,
            &self.localization.text("settings-title"),
            &self.localization.text("settings-subtitle"),
        );
        let settings_height = ui.available_height().max(0.0);
        egui::ScrollArea::vertical()
            .id_salt("settings_sections")
            .max_height(settings_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let mut locale = self
                    .preferences
                    .map_or(LocalePreference::System, |preferences| preferences.locale);
                let language_label = ui.label(self.localization.text("settings-language"));
                egui::ComboBox::from_id_salt("ui_locale")
                    .width(280.0)
                    .selected_text(match locale {
                        LocalePreference::System => self.localization.text("language-system"),
                        LocalePreference::Es => self.localization.text("language-spanish"),
                        LocalePreference::En => self.localization.text("language-english"),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut locale,
                            LocalePreference::System,
                            self.localization.text("language-system"),
                        );
                        ui.selectable_value(
                            &mut locale,
                            LocalePreference::Es,
                            self.localization.text("language-spanish"),
                        );
                        ui.selectable_value(
                            &mut locale,
                            LocalePreference::En,
                            self.localization.text("language-english"),
                        );
                    })
                    .response
                    .labelled_by(language_label.id);
                if self
                    .preferences
                    .is_some_and(|current| current.locale != locale)
                {
                    self.update_preferences(|preferences| preferences.locale = locale, false);
                }
                ui.add_space(24.0);
                ui.heading(
                    RichText::new(self.localization.text("settings-background"))
                        .size(20.0)
                        .family(crate::theme::semibold_font_family()),
                );
                let mut arguments = FluentArgs::new();
                arguments.set(
                    "status",
                    autostart_status_label(&self.localization, self.autostart_status),
                );
                ui.label(
                    self.localization
                        .text_with("settings-login-status", Some(&arguments)),
                );
                ui.horizontal_wrapped(|ui| {
                    let operation_idle = self.autostart_request_id.is_none();
                    let autostart_enabled = self.autostart_status == Some(AutostartStatus::Enabled);
                    let action = if autostart_enabled {
                        self.localization.text("action-disable")
                    } else {
                        self.localization.text("action-enable")
                    };
                    if ui
                        .add_enabled(operation_idle, first_knowledge::primary_button(action))
                        .clicked()
                    {
                        self.request_autostart(!autostart_enabled);
                    }
                    if ui
                        .add_enabled(
                            operation_idle,
                            crate::theme::ghost_button(
                                self.localization.text("settings-refresh-status"),
                                ui.visuals().dark_mode,
                            ),
                        )
                        .clicked()
                    {
                        let request_id = Uuid::new_v4();
                        self.autostart_request_id = Some(request_id);
                        self.worker
                            .send(WorkerCommand::RefreshAutostart { request_id });
                    }
                    if self.autostart_request_id.is_some() {
                        ui.spinner();
                    }
                });
                ui.add_space(30.0);
                self.settings_local_ai(ui);
                ui.add_space(30.0);
                self.update_settings(ui);
                ui.add_space(30.0);
                ui.collapsing(
                    self.localization.text("settings-advanced-diagnostics"),
                    |ui| {
                        egui::Grid::new("settings")
                            .num_columns(2)
                            .spacing([24.0, 12.0])
                            .show(ui, |ui| {
                                ui.label(self.localization.text("diagnostics-local-identity"));
                                wrap_monospace(ui, &self.node_id);
                                ui.end_row();
                                ui.label(self.localization.text("diagnostics-local-mcp"));
                                wrap_monospace(ui, &self.mcp_url);
                                ui.end_row();
                                ui.label(self.localization.text("diagnostics-database"));
                                wrap_monospace(ui, self.paths.database.display().to_string());
                                ui.end_row();
                                ui.label(self.localization.text("diagnostics-okf-bundles"));
                                wrap_monospace(ui, self.paths.vaults.display().to_string());
                                ui.end_row();
                                ui.label(self.localization.text("diagnostics-sanitized-logs"));
                                wrap_monospace(ui, self.paths.logs.display().to_string());
                                ui.end_row();
                                ui.label(self.localization.text("diagnostics-configuration"));
                                wrap_monospace(ui, self.paths.config.display().to_string());
                                ui.end_row();
                            });
                    },
                );
                ui.label(self.localization.text("settings-mcp-boundary"));
                scroll_newly_focused_control_into_view(ui);
            });
    }

    fn settings_local_ai(&mut self, ui: &mut egui::Ui) {
        let Some(state) = self.model_state.clone() else {
            return;
        };
        ui.heading(
            RichText::new(self.localization.text("settings-local-ai"))
                .size(20.0)
                .family(crate::theme::semibold_font_family()),
        );
        let mut profile_arguments = FluentArgs::new();
        profile_arguments.set("profile", profile_label(&self.localization, state.profile));
        let profile = self
            .localization
            .text_with("settings-model-profile", Some(&profile_arguments));
        let mut active_arguments = FluentArgs::new();
        active_arguments.set(
            "model",
            state
                .active_model_id
                .clone()
                .unwrap_or_else(|| self.localization.text("settings-model-none")),
        );
        let active = self
            .localization
            .text_with("settings-model-active", Some(&active_arguments));
        ui.label(format!("{profile} · {active}"));
        ui.add(
            egui::Label::new(
                RichText::new(self.localization.text("settings-local-ai-body"))
                    .color(crate::theme::secondary_text(ui.visuals().dark_mode)),
            )
            .wrap(),
        );
        if let Some(pending) = &state.pending_model_id {
            let mut arguments = FluentArgs::new();
            arguments.set("model", pending.as_str());
            ui.label(
                self.localization
                    .text_with("models-pending-restart", Some(&arguments)),
            );
        }
        if ui
            .button(self.localization.text("settings-manage-models"))
            .clicked()
        {
            self.screen = Screen::Models;
        }
    }

    fn knowledge(&mut self, ui: &mut egui::Ui) {
        let collections = self
            .collections
            .iter()
            .map(|collection| (collection.id, collection.name.clone()))
            .collect::<Vec<_>>();
        let active_scans = self
            .collection_scans
            .keys()
            .copied()
            .collect::<HashSet<_>>();
        let actions = self
            .knowledge
            .show(ui, &self.localization, &collections, &active_scans);
        for action in actions {
            self.send_knowledge_action(action);
        }
    }

    fn send_knowledge_action(&self, action: KnowledgeAction) {
        let command = match action {
            KnowledgeAction::LoadBundle {
                request_id,
                collection_id,
            } => WorkerCommand::LoadKnowledgeBundle {
                request_id,
                collection_id,
            },
            KnowledgeAction::LoadPage {
                request_id,
                collection_id,
                page_id,
                expected_fingerprint,
            } => WorkerCommand::LoadKnowledgePage {
                request_id,
                collection_id,
                page_id,
                expected_fingerprint,
            },
            KnowledgeAction::PrepareGuidedRepair {
                request_id,
                collection_id,
            } => WorkerCommand::PrepareGuidedWikiRepair {
                request_id,
                collection_id,
            },
            KnowledgeAction::ExecuteGuidedRepair {
                request_id,
                preview,
            } => WorkerCommand::ExecuteGuidedWikiRepair {
                request_id,
                preview,
            },
        };
        self.worker.send(command);
    }

    fn notices(&mut self, root: &mut egui::Ui) {
        if !self.notices.is_empty() {
            egui::Panel::bottom("notices").show(root, |ui| {
                for (error, message) in &self.notices {
                    let color = if *error {
                        crate::theme::error_text(ui.visuals().dark_mode)
                    } else {
                        crate::theme::verified_text(ui.visuals().dark_mode)
                    };
                    let summary = if *error {
                        human_error_summary(&self.localization, message)
                    } else {
                        message.clone()
                    };
                    ui.colored_label(color, summary);
                    if *error {
                        ui.collapsing(self.localization.text("technical-details"), |ui| {
                            ui.label(message);
                        });
                    }
                }
            });
        }
    }

    fn home_wiki_incident(&mut self, ui: &mut egui::Ui) -> bool {
        if self.wiki_health_error_dismissed {
            return false;
        }
        let WikiHealthCheckState::Failed(message) = &self.wiki_health_check else {
            return false;
        };
        let message = message.clone();
        let mut dismiss = false;
        let mut retry = false;
        crate::theme::surface_frame(ui.visuals().dark_mode).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    crate::theme::error_text(ui.visuals().dark_mode),
                    human_error_summary(&self.localization, &message),
                );
                if ui
                    .small_button(self.localization.text("action-dismiss"))
                    .clicked()
                {
                    dismiss = true;
                }
                if ui
                    .add_enabled(
                        self.wiki_health_request_id.is_none(),
                        egui::Button::new(self.localization.text("action-retry")),
                    )
                    .clicked()
                {
                    retry = true;
                }
            });
            ui.collapsing(self.localization.text("technical-details"), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("home_wiki_health_error")
                    .max_height(88.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(&message).wrap());
                    });
            });
        });
        if dismiss {
            self.wiki_health_error_dismissed = true;
        }
        if retry {
            let request_id = Uuid::new_v4();
            self.wiki_health_request_id = Some(request_id);
            self.wiki_health_check = WikiHealthCheckState::Loading;
            self.worker
                .send(WorkerCommand::RefreshWikiHealth { request_id });
        }
        true
    }

    fn onboarding_notices(&self, root: &mut egui::Ui) {
        let Some(page) = self.onboarding_page else {
            return;
        };
        let relevant = self
            .notices
            .iter()
            .filter(|(error, message)| *error && onboarding_error_is_relevant(page, message))
            .collect::<Vec<_>>();
        if relevant.is_empty() {
            return;
        }
        egui::Panel::bottom("onboarding_notices").show(root, |ui| {
            for (_, message) in relevant {
                ui.colored_label(
                    crate::theme::error_text(ui.visuals().dark_mode),
                    human_error_summary(&self.localization, message),
                );
                ui.collapsing(self.localization.text("technical-details"), |ui| {
                    ui.label(message);
                });
            }
        });
    }

    fn update_preferences(
        &mut self,
        mutate: impl FnOnce(&mut DesktopPreferencesUpdate),
        complete_onboarding: bool,
    ) {
        let Some(current) = self.preferences else {
            return;
        };
        let mut update = DesktopPreferencesUpdate {
            locale: current.locale,
            lan_preference: current.lan_preference,
            close_behavior: current.close_behavior,
            automatic_update_checks: current.automatic_update_checks,
            complete_onboarding,
        };
        mutate(&mut update);
        let request_id = Uuid::new_v4();
        self.preference_request_id = Some(request_id);
        self.worker
            .send(WorkerCommand::UpdateDesktopPreferences { request_id, update });
    }

    fn update_settings(&mut self, ui: &mut egui::Ui) {
        ui.scope(|ui| {
            ui.heading(self.localization.text("updates-title"));
            if let Some(preferences) = self.preferences {
                let mut automatic = preferences.automatic_update_checks;
                if ui
                    .checkbox(&mut automatic, self.localization.text("updates-automatic"))
                    .changed()
                {
                    self.update_preferences(
                        |preferences| preferences.automatic_update_checks = automatic,
                        false,
                    );
                }
            }
            let operation_idle = self.updater_request_id.is_none();
            match self.updater.clone() {
                Some(UpdaterWorkerView::Disabled(reason)) => {
                    ui.label(updater_disabled_label(&self.localization, reason));
                }
                Some(UpdaterWorkerView::Ready(view)) => {
                    if let Some(issue) = view.last_issue {
                        let message = update_issue_label(&self.localization, issue.code);
                        ui.colored_label(
                            crate::theme::warning_text(ui.visuals().dark_mode),
                            message,
                        );
                    }
                    match view.status {
                        UpdaterStatus::Idle => {
                            ui.label(self.localization.text("updates-idle"));
                        }
                        UpdaterStatus::Checking => {
                            ui.spinner();
                            ui.label(self.localization.text("updates-checking"));
                        }
                        UpdaterStatus::UpToDate => {
                            ui.label(self.localization.text("updates-current"));
                        }
                        UpdaterStatus::Available(update) => {
                            ui.label(localized_update_version(
                                &self.localization,
                                "updates-available",
                                &update.version,
                            ));
                            if let Some(notes) = update.release_notes {
                                ui.label(notes);
                            }
                            if ui
                                .add_enabled(
                                    operation_idle,
                                    egui::Button::new(self.localization.text("updates-download")),
                                )
                                .clicked()
                            {
                                self.update_confirmation = Some(UpdateConfirmationKind::Download);
                            }
                        }
                        UpdaterStatus::Downloading(update) => {
                            ui.spinner();
                            ui.label(localized_update_version(
                                &self.localization,
                                "updates-downloading",
                                &update.version,
                            ));
                        }
                        UpdaterStatus::ReadyToInstall(update) => {
                            ui.label(localized_update_version(
                                &self.localization,
                                "updates-ready-install",
                                &update.version,
                            ));
                            if ui
                                .add_enabled(
                                    operation_idle,
                                    egui::Button::new(self.localization.text("updates-install")),
                                )
                                .clicked()
                            {
                                self.update_confirmation = Some(UpdateConfirmationKind::Install);
                            }
                        }
                        UpdaterStatus::Installing(update) => {
                            ui.spinner();
                            ui.label(localized_update_version(
                                &self.localization,
                                "updates-installing",
                                &update.version,
                            ));
                        }
                        UpdaterStatus::Installed(update) => {
                            ui.label(localized_update_version(
                                &self.localization,
                                "updates-installed",
                                &update.version,
                            ));
                        }
                    }
                }
                None => {
                    ui.spinner();
                    ui.label(self.localization.text("updates-loading"));
                }
            }
            if ui
                .add_enabled(
                    operation_idle,
                    egui::Button::new(self.localization.text("updates-check-now")),
                )
                .clicked()
            {
                self.request_update(|request_id| WorkerCommand::CheckUpdates { request_id });
            }
        });

        let Some(kind) = self.update_confirmation else {
            return;
        };
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new(match kind {
            UpdateConfirmationKind::Download => self.localization.text("updates-confirm-download"),
            UpdateConfirmationKind::Install => self.localization.text("updates-confirm-install"),
        })
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ui.ctx(), |ui| {
            ui.label(self.localization.text(match kind {
                UpdateConfirmationKind::Download => "updates-confirm-download-body",
                UpdateConfirmationKind::Install => "updates-confirm-install-body",
            }));
            ui.horizontal(|ui| {
                if ui
                    .button(self.localization.text("action-confirm"))
                    .clicked()
                {
                    confirmed = true;
                }
                if ui.button(self.localization.text("action-cancel")).clicked() {
                    cancelled = true;
                }
            });
        });
        if confirmed {
            self.update_confirmation = None;
            match kind {
                UpdateConfirmationKind::Download => {
                    self.request_update(|request_id| WorkerCommand::DownloadUpdate { request_id })
                }
                UpdateConfirmationKind::Install => {
                    self.request_update(|request_id| WorkerCommand::InstallUpdate { request_id })
                }
            }
        } else if cancelled {
            self.update_confirmation = None;
        }
    }

    fn request_update(&mut self, command: impl FnOnce(Uuid) -> WorkerCommand) {
        let request_id = Uuid::new_v4();
        self.updater_request_id = Some(request_id);
        self.worker.send(command(request_id));
    }

    fn request_autostart(&mut self, enabled: bool) {
        let request_id = Uuid::new_v4();
        self.autostart_request_id = Some(request_id);
        self.worker.send(WorkerCommand::SetAutostart {
            request_id,
            enabled,
        });
    }

    fn close_policy(&self) -> ClosePolicy {
        match self
            .preferences
            .map(|preferences| preferences.close_behavior)
        {
            Some(CloseBehavior::HideToTray) => ClosePolicy::HideToTray,
            Some(CloseBehavior::Quit) => ClosePolicy::Quit,
            Some(CloseBehavior::Ask) | None => ClosePolicy::Ask,
        }
    }

    fn close_confirmation(&mut self, context: &egui::Context) {
        if !self.shell.close_confirmation_requested() {
            return;
        }
        let mut decision = None;
        egui::Window::new(self.localization.text("close-dialog-title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.label(self.localization.text("close-dialog-body"));
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.shell.tray_ready(),
                            egui::Button::new(self.localization.text("close-dialog-background")),
                        )
                        .clicked()
                    {
                        decision = Some(CloseBehavior::HideToTray);
                    }
                    if ui.button(self.localization.text("tray-quit")).clicked() {
                        decision = Some(CloseBehavior::Quit);
                    }
                    if ui.button(self.localization.text("action-cancel")).clicked() {
                        self.shell.cancel_close_confirmation();
                    }
                });
            });
        if let Some(close_behavior) = decision {
            self.update_preferences(
                |preferences| preferences.close_behavior = close_behavior,
                false,
            );
            self.shell.resolve_close(
                context,
                match close_behavior {
                    CloseBehavior::HideToTray => ClosePolicy::HideToTray,
                    CloseBehavior::Quit | CloseBehavior::Ask => ClosePolicy::Quit,
                },
            );
        }
    }

    fn onboarding_model(&mut self, ui: &mut egui::Ui) {
        let Some(state) = self.model_state.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(self.localization.text("models-calculating"));
            });
            return;
        };

        for profile in [
            ModelProfile::Automatic,
            ModelProfile::Efficient,
            ModelProfile::Quality,
        ] {
            let label = match profile {
                ModelProfile::Automatic => {
                    self.localization.text("onboarding-model-profile-automatic")
                }
                ModelProfile::Efficient => self.localization.text("onboarding-model-profile-small"),
                ModelProfile::Quality => self.localization.text("onboarding-model-profile-quality"),
            };
            let response = ui.radio(
                state.profile == profile,
                RichText::new(label).family(crate::theme::semibold_font_family()),
            );
            if response.clicked() && state.profile != profile {
                self.accepted_licenses = false;
                self.worker.send(WorkerCommand::SetModelProfile(profile));
            }
            let description = match profile {
                ModelProfile::Automatic => {
                    let mut arguments = FluentArgs::new();
                    arguments.set(
                        "model",
                        state.recommended_display_name.as_deref().unwrap_or("—"),
                    );
                    self.localization
                        .text_with("onboarding-model-profile-automatic-body", Some(&arguments))
                }
                ModelProfile::Efficient => self
                    .localization
                    .text("onboarding-model-profile-small-body"),
                ModelProfile::Quality => self
                    .localization
                    .text("onboarding-model-profile-quality-body"),
            };
            ui.indent(("onboarding_model_profile", profile), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(description)
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    )
                    .wrap(),
                );
            });
            ui.add_space(5.0);
        }
        ui.add(
            egui::Label::new(
                RichText::new(self.localization.text("onboarding-model-download-note"))
                    .color(ui.visuals().weak_text_color()),
            )
            .wrap(),
        );
        ui.add_space(8.0);

        let mut size_arguments = FluentArgs::new();
        size_arguments.set(
            "download",
            format!("{:.2}", state.download_bytes as f64 / 1024_f64.powi(3)),
        );
        size_arguments.set(
            "required",
            format!("{:.2}", state.required_free_bytes as f64 / 1024_f64.powi(3)),
        );
        ui.add(
            egui::Label::new(
                self.localization
                    .text_with("models-download-size", Some(&size_arguments)),
            )
            .wrap(),
        );

        if !state.issues.is_empty() {
            ui.colored_label(
                crate::theme::error_text(ui.visuals().dark_mode),
                self.localization.text("error-local-ai"),
            );
            ui.collapsing(self.localization.text("technical-details"), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("onboarding_model_issues")
                    .max_height(96.0)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for issue in &state.issues {
                            ui.label(issue);
                        }
                    });
            });
        }

        ui.collapsing(self.localization.text("onboarding-model-details"), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("onboarding_model_licenses")
                .max_height(110.0)
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    if let (Some(license), Some(url), Some(revision)) =
                        (&state.license, &state.license_url, &state.revision)
                    {
                        ui.hyperlink_to(localized_license(&self.localization, license), url);
                        let mut revision_arguments = FluentArgs::new();
                        revision_arguments.set("revision", &revision[..revision.len().min(12)]);
                        ui.label(
                            self.localization
                                .text_with("models-revision", Some(&revision_arguments)),
                        );
                    }
                    ui.hyperlink_to(
                        localized_license(&self.localization, E5_FILES[0].license),
                        E5_FILES[0].license_url,
                    );
                    ui.hyperlink_to(
                        localized_license(&self.localization, MMARCO_COMMON_FILES[0].license),
                        MMARCO_COMMON_FILES[0].license_url,
                    );
                    ui.hyperlink_to(
                        localized_license(&self.localization, "llama.cpp"),
                        "https://github.com/ggml-org/llama.cpp/blob/b9946/LICENSE",
                    );
                });
        });

        let recommended = state.recommended_model_id.as_deref();
        let already_active = self.models_ready && state.active_model_id.as_deref() == recommended;
        let already_pending = state.pending_model_id.as_deref() == recommended;
        if already_active {
            ui.colored_label(
                crate::theme::verified_text(ui.visuals().dark_mode),
                self.localization.text("models-recommended-active"),
            );
        } else {
            ui.checkbox(
                &mut self.accepted_licenses,
                self.localization.text("models-accept-licenses"),
            );
            let can_install = recommended.is_some()
                && !already_pending
                && self.accepted_licenses
                && state.fits_available_disk
                && state.issues.is_empty()
                && self.install_label.is_none();
            ui.horizontal(|ui| {
                let install_response = ui.add_enabled(
                    can_install,
                    first_knowledge::primary_button(model_action_label(
                        &self.localization,
                        state.recommended_assets_installed,
                        self.models_ready,
                    )),
                );
                if install_response.clicked() {
                    self.worker.send(WorkerCommand::InstallModels);
                }
                if self.install_label.is_some() {
                    let cancel_response = ui.button(self.localization.text("action-cancel"));
                    if cancel_response.clicked() {
                        self.worker.send(WorkerCommand::CancelInstall);
                    }
                }
                if already_pending {
                    ui.label(self.localization.text("models-restart-to-activate"));
                }
            });
        }

        if let Some(label) = &self.install_label {
            ui.label(label);
            ui.add(egui::ProgressBar::new(self.install_progress.clamp(0.0, 1.0)).show_percentage());
        }
        ui.label(
            RichText::new(self.localization.text("onboarding-model-change-later"))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    }

    fn onboarding(&mut self, ui: &mut egui::Ui) {
        let Some(page) = self.onboarding_page else {
            return;
        };
        let layout = ResponsiveLayout::from_available(ui.available_size());

        ui.set_width(ui.available_width().min(560.0));
        StripBuilder::new(ui)
            .size(Size::exact(first_knowledge::journey_header_height(
                layout.density,
            )))
            .size(Size::remainder())
            .size(Size::exact(first_knowledge::footer_height(layout.density)))
            .clip(true)
            .vertical(|mut strip| {
                strip.cell(|ui| {
                    let mut arguments = FluentArgs::new();
                    arguments.set("current", onboarding_intro_step(page));
                    arguments.set("total", 3);
                    editorial_card_kicker(
                        ui,
                        self.localization
                            .text_with("onboarding-progress", Some(&arguments))
                            .to_uppercase(),
                        crate::theme::accent_text(ui.visuals().dark_mode),
                    );
                });
                strip.cell(|ui| {
                    let body_width = ui.available_width();
                    egui::ScrollArea::vertical()
                        .id_salt(("onboarding-body", onboarding_intro_step(page)))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_min_width(body_width);
                            first_knowledge::work_surface(ui, layout.density, |ui| match page {
                                OnboardingPage::Welcome => {
                                    onboarding_title(
                                        ui,
                                        &self.localization.text("onboarding-welcome-title"),
                                        &self.localization.text("onboarding-welcome-body"),
                                        layout.density,
                                    );
                                    ui.heading(
                                        RichText::new(
                                            self.localization
                                                .text("onboarding-machine-check-title"),
                                        )
                                        .size(20.0)
                                        .family(crate::theme::semibold_font_family()),
                                    );
                                    if let Some(report) = self.hardware.as_ref() {
                                        let machine_checks = onboarding_machine_checks(report);
                                        let mut platform_arguments = FluentArgs::new();
                                        platform_arguments.set(
                                            "platform",
                                            hardware_platform_name(
                                                &report.os,
                                                &report.architecture,
                                            ),
                                        );
                                        let mut memory_arguments = FluentArgs::new();
                                        memory_arguments
                                            .set("memory", rounded_gib(report.total_memory_bytes));
                                        let mut disk_arguments = FluentArgs::new();
                                        disk_arguments
                                            .set("disk", rounded_gib(report.available_disk_bytes));
                                        for (message, ready) in [
                                            (
                                                self.localization.text_with(
                                                    "onboarding-machine-platform",
                                                    Some(&platform_arguments),
                                                ),
                                                machine_checks.0,
                                            ),
                                            (
                                                self.localization.text_with(
                                                    "onboarding-machine-memory",
                                                    Some(&memory_arguments),
                                                ),
                                                machine_checks.1,
                                            ),
                                            (
                                                self.localization.text_with(
                                                    "onboarding-machine-free-disk",
                                                    Some(&disk_arguments),
                                                ),
                                                machine_checks.2,
                                            ),
                                        ] {
                                            onboarding_machine_row(ui, &message, Some(ready));
                                        }
                                    } else {
                                        for message_id in [
                                            "onboarding-machine-checking-platform",
                                            "onboarding-machine-checking-memory",
                                            "onboarding-machine-checking-disk",
                                        ] {
                                            onboarding_machine_row(
                                                ui,
                                                &self.localization.text(message_id),
                                                None,
                                            );
                                        }
                                    }
                                    ui.add_space(14.0);
                                    let machine_status =
                                        onboarding_machine_status(self.hardware.as_ref().map(
                                            |report| (report.supported_target, report.can_install),
                                        ));
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(self.localization.text(machine_status))
                                                .color(ui.visuals().weak_text_color()),
                                        )
                                        .wrap(),
                                    );
                                }
                                OnboardingPage::Model => {
                                    onboarding_title(
                                        ui,
                                        &self.localization.text("onboarding-model-title"),
                                        &self.localization.text("onboarding-model-body"),
                                        layout.density,
                                    );
                                    self.onboarding_model(ui);
                                    ui.add_space(10.0);
                                    if ui
                                        .add(crate::theme::ghost_button(
                                            self.localization.text("onboarding-skip"),
                                            ui.visuals().dark_mode,
                                        ))
                                        .clicked()
                                    {
                                        self.screen = Screen::Setup;
                                        self.finish_onboarding();
                                    }
                                }
                                OnboardingPage::Permissions => {
                                    onboarding_title(
                                        ui,
                                        &self.localization.text("onboarding-permissions-title"),
                                        &self.localization.text("onboarding-permissions-body"),
                                        layout.density,
                                    );
                                    onboarding_permission_row(
                                        ui,
                                        OnboardingPermissionIcon::Folder,
                                        &self
                                            .localization
                                            .text("onboarding-permissions-folder-title"),
                                        &self
                                            .localization
                                            .text("onboarding-permissions-folder-body"),
                                    );
                                    onboarding_permission_row(
                                        ui,
                                        OnboardingPermissionIcon::Network,
                                        &self
                                            .localization
                                            .text("onboarding-permissions-network-title"),
                                        &self
                                            .localization
                                            .text("onboarding-permissions-network-body"),
                                    );
                                    onboarding_permission_row(
                                        ui,
                                        OnboardingPermissionIcon::Power,
                                        &self
                                            .localization
                                            .text("onboarding-permissions-login-title"),
                                        &self
                                            .localization
                                            .text("onboarding-permissions-login-body"),
                                    );
                                }
                            });
                            scroll_newly_focused_control_into_view(ui);
                        });
                });
                strip.cell(|ui| {
                    let available = ui.available_rect_before_wrap();
                    let button_size = egui::vec2(108.0, 42.0);
                    let (back_button, next_button) =
                        onboarding_footer_button_rects(available, button_size);
                    match page {
                        OnboardingPage::Welcome => {
                            if ui
                                .put(
                                    next_button,
                                    first_knowledge::primary_button(
                                        self.localization.text("onboarding-next"),
                                    ),
                                )
                                .clicked()
                            {
                                self.onboarding_page = Some(OnboardingPage::Model);
                            }
                        }
                        OnboardingPage::Model => {
                            if ui
                                .put(
                                    back_button,
                                    crate::theme::ghost_button(
                                        self.localization.text("onboarding-back"),
                                        ui.visuals().dark_mode,
                                    ),
                                )
                                .clicked()
                            {
                                self.onboarding_page = Some(OnboardingPage::Welcome);
                            }
                            let continue_response = ui
                                .add_enabled_ui(self.models_ready, |ui| {
                                    ui.put(
                                        next_button,
                                        first_knowledge::primary_button(
                                            self.localization.text("onboarding-next"),
                                        ),
                                    )
                                })
                                .inner
                                .on_disabled_hover_text(
                                    self.localization.text("onboarding-model-required"),
                                );
                            if continue_response.clicked() {
                                self.onboarding_page = Some(OnboardingPage::Permissions);
                            }
                        }
                        OnboardingPage::Permissions => {
                            if ui
                                .put(
                                    back_button,
                                    crate::theme::ghost_button(
                                        self.localization.text("onboarding-back"),
                                        ui.visuals().dark_mode,
                                    ),
                                )
                                .clicked()
                            {
                                self.onboarding_page = Some(OnboardingPage::Model);
                            }
                            if ui
                                .put(
                                    next_button,
                                    first_knowledge::primary_button(
                                        self.localization.text("onboarding-finish"),
                                    ),
                                )
                                .clicked()
                            {
                                self.screen = Screen::Setup;
                                self.finish_onboarding();
                            }
                        }
                    }
                    if self.onboarding_finishing {
                        ui.spinner();
                    }
                });
            });
    }

    fn finish_onboarding(&mut self) {
        if self.onboarding_finishing || self.preferences.is_none() {
            return;
        }
        self.onboarding_finishing = true;
        self.update_preferences(|_| {}, true);
    }
}

fn effective_locale(preference: LocalePreference) -> UiLocale {
    match preference {
        LocalePreference::System => UiLocale::from_system(),
        LocalePreference::Es => UiLocale::Es,
        LocalePreference::En => UiLocale::EnUs,
    }
}

fn classify_external_ai_policy_change(current: bool, proposed: bool) -> ExternalAiPolicyChange {
    match (current, proposed) {
        (false, true) => ExternalAiPolicyChange::ConfirmEnable,
        (true, false) => ExternalAiPolicyChange::ApplyDisable,
        (false, false) | (true, true) => ExternalAiPolicyChange::None,
    }
}

fn autostart_status_label(localization: &Localization, status: Option<AutostartStatus>) -> String {
    localization.text(match status {
        Some(AutostartStatus::Enabled) => "autostart-enabled",
        Some(AutostartStatus::Disabled) => "autostart-disabled",
        Some(AutostartStatus::RequiresApproval) => "autostart-needs-approval",
        Some(AutostartStatus::Conflict) => "autostart-conflict",
        Some(AutostartStatus::Unsupported) => "autostart-unsupported",
        None => "autostart-checking",
    })
}

fn updater_disabled_label(localization: &Localization, reason: UpdaterDisabledReason) -> String {
    localization.text(match reason {
        UpdaterDisabledReason::NotConfigured => "updates-disabled-not-configured",
        UpdaterDisabledReason::InvalidEndpoint => "updates-disabled-endpoint",
        UpdaterDisabledReason::InvalidPublicKey => "updates-disabled-key",
        UpdaterDisabledReason::InvalidCurrentVersion => "updates-disabled-version",
        UpdaterDisabledReason::UnsupportedPlatform => "updates-disabled-platform",
    })
}

fn updater_launched_installer(view: &UpdaterWorkerView) -> bool {
    matches!(
        view,
        UpdaterWorkerView::Ready(view) if matches!(&view.status, UpdaterStatus::Installed(_))
    )
}

fn localized_today_date(locale: UiLocale) -> String {
    let today = chrono::Local::now().date_naive();
    let weekday_index = today.weekday().num_days_from_monday() as usize;
    let month_index = today.month0() as usize;
    match locale {
        UiLocale::EnUs => {
            const WEEKDAYS: [&str; 7] = [
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ];
            const MONTHS: [&str; 12] = [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];
            let weekday = WEEKDAYS.get(weekday_index).copied().unwrap_or("Today");
            let month = MONTHS.get(month_index).copied().unwrap_or("January");
            format!("{}, {} {}, {}", weekday, month, today.day(), today.year())
        }
        UiLocale::Es => {
            const WEEKDAYS: [&str; 7] = [
                "lunes",
                "martes",
                "miércoles",
                "jueves",
                "viernes",
                "sábado",
                "domingo",
            ];
            const MONTHS: [&str; 12] = [
                "enero",
                "febrero",
                "marzo",
                "abril",
                "mayo",
                "junio",
                "julio",
                "agosto",
                "septiembre",
                "octubre",
                "noviembre",
                "diciembre",
            ];
            let weekday = WEEKDAYS.get(weekday_index).copied().unwrap_or("Hoy");
            let month = MONTHS.get(month_index).copied().unwrap_or("enero");
            format!(
                "{}, {} de {} de {}",
                weekday,
                today.day(),
                month,
                today.year()
            )
        }
    }
}

fn update_issue_label(localization: &Localization, issue: UpdateIssueCode) -> String {
    localization.text(match issue {
        UpdateIssueCode::Offline => "updates-issue-offline",
        UpdateIssueCode::InvalidManifest => "updates-issue-manifest",
        UpdateIssueCode::InvalidSignature => "updates-issue-signature",
        UpdateIssueCode::Unsupported => "updates-issue-unsupported",
        UpdateIssueCode::Internal => "updates-issue-internal",
    })
}

fn localized_update_version(
    localization: &Localization,
    message_id: &str,
    version: &str,
) -> String {
    let mut arguments = FluentArgs::new();
    arguments.set("version", version);
    localization.text_with(message_id, Some(&arguments))
}

fn readiness_status_presentation(
    localization: &Localization,
    status: ReadinessStatus,
    dark_mode: bool,
) -> (String, Color32) {
    let (message, color) = match status {
        ReadinessStatus::Ready => ("status-ready", crate::theme::verified_text(dark_mode)),
        ReadinessStatus::Working => ("status-working", crate::theme::accent_text(dark_mode)),
        ReadinessStatus::NeedsPermission => (
            "status-needs-permission",
            crate::theme::warning_text(dark_mode),
        ),
        ReadinessStatus::NeedsAttention => (
            "status-needs-attention",
            crate::theme::error_text(dark_mode),
        ),
        ReadinessStatus::OptionalDisabled => (
            "status-optional-disabled",
            crate::theme::secondary_text(dark_mode),
        ),
    };
    (localization.text(message), color)
}

fn maintenance_status_presentation(
    localization: &Localization,
    status: airwiki_core::CollectionMaintenanceStatus,
    dark_mode: bool,
) -> (String, Color32) {
    let (message, color) = match status {
        airwiki_core::CollectionMaintenanceStatus::Never => {
            ("maintenance-never", crate::theme::secondary_text(dark_mode))
        }
        airwiki_core::CollectionMaintenanceStatus::Success => (
            "maintenance-success",
            crate::theme::verified_text(dark_mode),
        ),
        airwiki_core::CollectionMaintenanceStatus::Partial => {
            ("maintenance-partial", crate::theme::warning_text(dark_mode))
        }
        airwiki_core::CollectionMaintenanceStatus::Failed => {
            ("maintenance-failed", crate::theme::error_text(dark_mode))
        }
        airwiki_core::CollectionMaintenanceStatus::Quarantined => (
            "maintenance-quarantined",
            crate::theme::error_text(dark_mode),
        ),
    };
    (localization.text(message), color)
}

fn source_issue_message(
    localization: &Localization,
    code: airwiki_core::SourceIssueCode,
) -> String {
    let message_id = match code {
        airwiki_core::SourceIssueCode::FileTooLarge => "review-issue-file-too-large",
        airwiki_core::SourceIssueCode::Unreadable => "review-issue-unreadable",
        airwiki_core::SourceIssueCode::InvalidUtf8 => "review-issue-invalid-utf8",
        airwiki_core::SourceIssueCode::InvalidPdf => "review-issue-invalid-pdf",
        airwiki_core::SourceIssueCode::EncryptedPdf => "review-issue-encrypted-pdf",
        airwiki_core::SourceIssueCode::TooManyPages => "review-issue-too-many-pages",
        airwiki_core::SourceIssueCode::NoTextLayer => "review-issue-no-text-layer",
        airwiki_core::SourceIssueCode::TooManyCharacters => "review-issue-too-many-characters",
        airwiki_core::SourceIssueCode::Superseded
        | airwiki_core::SourceIssueCode::ProcessingFailed => "review-issue-processing-failed",
    };
    localization.text(message_id)
}

fn source_issue_cause_message(
    localization: &Localization,
    issue: &SourceIssueView,
    code: airwiki_core::SourceIssueCode,
) -> Option<String> {
    let cause = issue.reason.as_deref().unwrap_or("");
    let message = match cause {
        "file-too-large" => "review-issue-cause-file-too-large",
        "unreadable" => "review-issue-cause-unreadable",
        "invalid-utf8" => "review-issue-cause-invalid-utf8",
        "invalid-pdf" => "review-issue-cause-invalid-pdf",
        "encrypted-pdf" => "review-issue-cause-encrypted-pdf",
        "too-many-pages" => "review-issue-cause-too-many-pages",
        "no-text-layer" => "review-issue-cause-no-text-layer",
        "too-many-characters" => "review-issue-cause-too-many-characters",
        "source-missing" => "review-issue-cause-source-missing",
        "permission-denied" => "review-issue-cause-permission-denied",
        "processing-failed" => "review-issue-cause-processing-failed",
        _ => "",
    };
    if message.is_empty() {
        if code == airwiki_core::SourceIssueCode::Superseded
            || code == airwiki_core::SourceIssueCode::ProcessingFailed
        {
            return Some(localization.text("review-issue-cause-processing-failed"));
        }
        if let Some(reason) = source_issue_raw_reason_preview(issue.reason.as_deref(), 120) {
            let mut arguments = FluentArgs::new();
            arguments.set("reason", reason);
            return Some(localization.text_with("review-issue-cause-unmapped", Some(&arguments)));
        }
        return Some(localization.text("review-issue-cause-unknown"));
    }

    Some(localization.text(message))
}

fn maintenance_issue_summary(
    localization: &Localization,
    issue_code: Option<&str>,
    persisted_summary: Option<&str>,
) -> Option<String> {
    let message = match issue_code {
        Some("collection_scan_partial") => Some("collections-maintenance-partial"),
        Some("collection_scan_failed") => Some("collections-maintenance-failed"),
        Some("collection_quarantined") => Some("collections-maintenance-quarantined"),
        _ => None,
    };
    message
        .map(|message| localization.text(message))
        .or_else(|| persisted_summary.map(str::to_owned))
}

fn source_issue_raw_reason_preview(reason: Option<&str>, max_chars: usize) -> Option<String> {
    let reason = reason?.trim();
    if reason.is_empty() {
        return None;
    }
    let collapsed = reason
        .replace(['\n', '\r', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let collapsed = collapsed.trim();
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() <= max_chars {
        return Some(collapsed.to_owned());
    }
    let truncated = collapsed.chars().take(max_chars).collect::<String>();
    Some(format!("{truncated}…"))
}

fn show_review_issue(
    ui: &mut egui::Ui,
    localization: &Localization,
    issue: &SourceIssueView,
    scanning: bool,
) -> bool {
    let mut requested_rescan = false;
    egui::Frame::new()
        .fill(crate::theme::paper(ui.visuals().dark_mode))
        .stroke(egui::Stroke::new(
            1.0,
            crate::theme::border(ui.visuals().dark_mode),
        ))
        .corner_radius(egui::CornerRadius::ZERO)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                RichText::new(&issue.source_name).family(crate::theme::semibold_font_family()),
            );
            ui.label(
                RichText::new(&issue.collection_name)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.label(
                RichText::new(localization.text("review-issue-status"))
                    .small()
                    .family(crate::theme::semibold_font_family())
                    .color(crate::theme::warning_text(ui.visuals().dark_mode)),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(source_issue_message(localization, issue.code)).small(),
                )
                .wrap(),
            );
            if let Some(cause_message) = source_issue_cause_message(localization, issue, issue.code)
            {
                ui.add(
                    egui::Label::new(
                        RichText::new({
                            let mut arguments = FluentArgs::new();
                            arguments.set("cause", cause_message);
                            localization.text_with("review-issue-cause", Some(&arguments))
                        })
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    )
                    .wrap(),
                );
            }
            requested_rescan = ui
                .add_enabled(
                    !scanning,
                    egui::Button::new(localization.text("review-scan-again")).small(),
                )
                .clicked();
        });
    requested_rescan
}

fn peer_trust_label(localization: &Localization, trust: PeerTrustState) -> String {
    localization.text(match trust {
        PeerTrustState::Unpaired => "peer-trust-unpaired",
        PeerTrustState::Trusted => "peer-trust-trusted",
        PeerTrustState::Blocked => "peer-trust-blocked",
    })
}

fn peer_activity_label(
    localization: &Localization,
    trust: PeerTrustState,
    activity: PeerActivityState,
) -> String {
    localization.text(peer_activity_message_id(trust, activity))
}

const fn should_present_pairing_controls(activity: PeerActivityState) -> bool {
    matches!(activity, PeerActivityState::Pairing)
}

const fn peer_activity_message_id(
    trust: PeerTrustState,
    activity: PeerActivityState,
) -> &'static str {
    match (trust, activity) {
        (PeerTrustState::Trusted, PeerActivityState::NotObserved) => "peer-activity-not-observed",
        (_, PeerActivityState::NotObserved) => "peer-activity-unavailable",
        (_, PeerActivityState::Discovered) => "peer-activity-discovered",
        (_, PeerActivityState::Pairing) => "peer-activity-pairing",
        (_, PeerActivityState::Connected) => "peer-activity-connected",
    }
}

fn search_coverage_message(
    localization: &Localization,
    coverage: SearchCoverageView,
) -> Option<String> {
    match coverage {
        SearchCoverageView::Complete => None,
        SearchCoverageView::FederationDisabled => {
            Some(localization.text("search-coverage-federation-disabled"))
        }
        SearchCoverageView::OfflineDevices { count } => {
            let mut arguments = FluentArgs::new();
            arguments.set("count", count);
            Some(localization.text_with("search-coverage-offline-devices", Some(&arguments)))
        }
        SearchCoverageView::PublicNetworkOffline => {
            Some(localization.text("search-coverage-public-offline"))
        }
        SearchCoverageView::Partial => Some(localization.text("search-coverage-partial")),
    }
}

fn classify_search_result(
    local_node_id: &str,
    hit_node_id: &str,
    local_collection_exists: bool,
    remote_device_name: Option<&str>,
) -> SearchResultAvailability {
    if hit_node_id == local_node_id {
        if local_collection_exists {
            SearchResultAvailability::LocalAvailable
        } else {
            SearchResultAvailability::LocalUnavailable
        }
    } else {
        SearchResultAvailability::Remote {
            device_name: remote_device_name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        }
    }
}

fn search_result_origin_label(
    localization: &Localization,
    availability: &SearchResultAvailability,
) -> String {
    match availability {
        SearchResultAvailability::LocalAvailable | SearchResultAvailability::LocalUnavailable => {
            localization.text("search-origin-local")
        }
        SearchResultAvailability::Remote { device_name } => {
            let Some(device) = device_name.as_deref() else {
                return localization.text("search-origin-remote-fallback");
            };
            let mut arguments = FluentArgs::new();
            arguments.set("device", device);
            localization.text_with("search-origin-remote", Some(&arguments))
        }
    }
}

fn connectivity_issue_message(localization: &Localization, issue: ConnectivityIssueCode) -> String {
    localization.text(match issue {
        ConnectivityIssueCode::Busy => "connectivity-issue-busy",
        ConnectivityIssueCode::FirewallCancelled => "connectivity-issue-firewall-cancelled",
        ConnectivityIssueCode::FirewallManagedPolicy => "connectivity-issue-firewall-managed",
        ConnectivityIssueCode::FirewallInboundBlocked => {
            "connectivity-issue-firewall-inbound-blocked"
        }
        ConnectivityIssueCode::FirewallConflict => "connectivity-issue-firewall-conflict",
        ConnectivityIssueCode::FirewallInstallationInvalid => {
            "connectivity-issue-firewall-installation"
        }
        ConnectivityIssueCode::FirewallUnsupported => "connectivity-issue-firewall-unsupported",
        ConnectivityIssueCode::FirewallStateChanged => "connectivity-issue-firewall-state-changed",
        ConnectivityIssueCode::FirewallInternal => "connectivity-issue-firewall-internal",
    })
}

fn firewall_operation_update_applies(
    presentation_request_id: Option<Uuid>,
    event_request_id: Uuid,
    state: Option<FirewallOperationView>,
) -> bool {
    state.is_none() || presentation_request_id == Some(event_request_id)
}

fn sanitized_error_code(message: &str) -> &'static str {
    let normalized = message.to_lowercase();
    if normalized.contains("private services")
        || normalized.contains("servicios privados")
        || normalized.contains("device identity")
        || normalized.contains("identidad ed25519")
        || normalized.contains("keychain")
        || normalized.contains("llavero")
    {
        "startup_services_unavailable"
    } else if normalized.contains("modelo")
        || normalized.contains("model")
        || normalized.contains("inferencia")
        || normalized.contains("local_ai")
    {
        "local_ai_unavailable"
    } else if normalized.contains("colección")
        || normalized.contains("collection")
        || normalized.contains("carpeta")
        || normalized.contains("folder")
        || normalized.contains("scan")
        || normalized.contains("watcher")
    {
        "collection_unavailable"
    } else if normalized.contains("lan")
        || normalized.contains("red local")
        || normalized.contains("network")
        || normalized.contains("connectivity")
        || normalized.contains("firewall")
        || normalized.contains("empareja")
        || normalized.contains("pairing")
    {
        "connectivity_unavailable"
    } else if normalized.contains("search") || normalized.contains("búsqueda") {
        "search_unavailable"
    } else if normalized.contains("integración")
        || normalized.contains("integration")
        || normalized.contains("mcp")
        || normalized.contains("chat")
    {
        "chat_integration_unavailable"
    } else if normalized.contains("actualiza") || normalized.contains("update") {
        "update_unavailable"
    } else {
        "operation_failed"
    }
}

fn human_error_summary(localization: &Localization, message: &str) -> String {
    let message_id = match sanitized_error_code(message) {
        "local_ai_unavailable" => "error-local-ai",
        "collection_unavailable" => "error-collection",
        "connectivity_unavailable" => "error-connectivity",
        "chat_integration_unavailable" => "error-chat",
        "update_unavailable" => "error-update",
        _ => "error-generic",
    };
    localization.text(message_id)
}

fn localized_worker_notice(localization: &Localization, message: &str) -> String {
    if localization.locale() == UiLocale::Es {
        return message.to_owned();
    }
    let normalized = message.to_lowercase();
    let message_id = if normalized.contains("modelo")
        || normalized.contains("model")
        || normalized.contains("descarga")
        || normalized.contains("instal")
        || normalized.contains("verifica")
    {
        "notice-model-updated"
    } else if normalized.contains("documento")
        || normalized.contains("borrador")
        || normalized.contains("public")
        || normalized.contains("wiki")
        || normalized.contains("colección")
    {
        "notice-knowledge-updated"
    } else if normalized.contains("lan")
        || normalized.contains("red local")
        || normalized.contains("equipo")
        || normalized.contains("peer")
        || normalized.contains("empareja")
        || normalized.contains("sas")
    {
        "notice-connectivity-updated"
    } else {
        "notice-operation-complete"
    };
    localization.text(message_id)
}

fn onboarding_error_is_relevant(page: OnboardingPage, message: &str) -> bool {
    let normalized = message.to_lowercase();
    let contains_any = |terms: &[&str]| terms.iter().any(|term| normalized.contains(term));
    if normalized == "startup_services_unavailable"
        || contains_any(&[
            "private services",
            "servicios privados",
            "device identity",
            "identidad ed25519",
            "keychain",
            "llavero",
        ])
    {
        return true;
    }
    match page {
        OnboardingPage::Welcome | OnboardingPage::Permissions => false,
        OnboardingPage::Model => {
            normalized == "local_ai_unavailable"
                || contains_any(&[
                    "model", "modelo", "infer", "asset", "artifact", "hash", "memory", "memoria",
                    "disk", "space", "espacio",
                ])
        }
    }
}

fn primary_action_title(localization: &Localization, action: RecommendedAction) -> String {
    localization.text(match action {
        RecommendedAction::PrepareLocalAi => "primary-prepare-ai-title",
        RecommendedAction::ResolveLocalAiIssue => "primary-resolve-ai-title",
        RecommendedAction::AddKnowledgeFolder => "primary-add-folder-title",
        RecommendedAction::ResolveCollectionIssue => "primary-resolve-folder-title",
        RecommendedAction::ReviewPendingKnowledge => "primary-review-title",
        RecommendedAction::InspectWikiHealth => "primary-wiki-title",
        RecommendedAction::ExplainLan => "primary-explain-lan-title",
        RecommendedAction::RequestSystemPermission => "primary-permission-title",
        RecommendedAction::ChangeNetworkProfile => "primary-profile-title",
        RecommendedAction::ConfigureFirewall => "primary-firewall-title",
        RecommendedAction::OpenFirewallSettings => "primary-firewall-system-title",
        RecommendedAction::ReviewLegacyFirewallRules => "primary-firewall-legacy-title",
        RecommendedAction::RepairConnectivityInstallation => {
            "primary-connectivity-installation-title"
        }
        RecommendedAction::ContactAdministrator => "primary-connectivity-admin-title",
        RecommendedAction::RetryConnectivity => "primary-connectivity-title",
        RecommendedAction::ResolveChatIssue => "primary-chat-title",
        RecommendedAction::ResolveBackgroundIssue => "primary-background-title",
        RecommendedAction::ResolveUpdateIssue => "primary-updates-title",
    })
}

fn primary_action_explanation(localization: &Localization, action: RecommendedAction) -> String {
    localization.text(match action {
        RecommendedAction::PrepareLocalAi | RecommendedAction::ResolveLocalAiIssue => {
            "primary-ai-explanation"
        }
        RecommendedAction::AddKnowledgeFolder | RecommendedAction::ResolveCollectionIssue => {
            "primary-folder-explanation"
        }
        RecommendedAction::ReviewPendingKnowledge => "primary-review-explanation",
        RecommendedAction::InspectWikiHealth => "primary-wiki-explanation",
        RecommendedAction::ExplainLan
        | RecommendedAction::RequestSystemPermission
        | RecommendedAction::ChangeNetworkProfile
        | RecommendedAction::ConfigureFirewall
        | RecommendedAction::OpenFirewallSettings
        | RecommendedAction::RepairConnectivityInstallation
        | RecommendedAction::RetryConnectivity => "primary-lan-explanation",
        RecommendedAction::ReviewLegacyFirewallRules => "primary-firewall-legacy-explanation",
        RecommendedAction::ContactAdministrator => "primary-connectivity-admin-explanation",
        RecommendedAction::ResolveChatIssue => "primary-chat-explanation",
        RecommendedAction::ResolveBackgroundIssue => "primary-background-explanation",
        RecommendedAction::ResolveUpdateIssue => "primary-updates-explanation",
    })
}

fn primary_action_button(localization: &Localization, action: RecommendedAction) -> String {
    localization.text(match action {
        RecommendedAction::PrepareLocalAi => "primary-button-prepare",
        RecommendedAction::ResolveLocalAiIssue
        | RecommendedAction::ResolveCollectionIssue
        | RecommendedAction::RequestSystemPermission
        | RecommendedAction::ChangeNetworkProfile
        | RecommendedAction::ConfigureFirewall
        | RecommendedAction::OpenFirewallSettings
        | RecommendedAction::ReviewLegacyFirewallRules
        | RecommendedAction::RepairConnectivityInstallation
        | RecommendedAction::RetryConnectivity
        | RecommendedAction::ResolveChatIssue
        | RecommendedAction::ResolveBackgroundIssue
        | RecommendedAction::ResolveUpdateIssue => "primary-button-resolve",
        RecommendedAction::AddKnowledgeFolder => "primary-button-add-folder",
        RecommendedAction::ReviewPendingKnowledge => "action-review",
        RecommendedAction::InspectWikiHealth => "primary-button-open-health",
        RecommendedAction::ExplainLan => "primary-button-view-options",
        RecommendedAction::ContactAdministrator => "primary-button-view-diagnostics",
    })
}

fn profile_label(localization: &Localization, profile: ModelProfile) -> String {
    localization.text(match profile {
        ModelProfile::Automatic => "models-profile-automatic",
        ModelProfile::Efficient => "models-profile-efficient",
        ModelProfile::Quality => "models-profile-quality",
    })
}

fn model_action_label(
    localization: &Localization,
    assets_installed: bool,
    models_ready: bool,
) -> String {
    localization.text(if assets_installed {
        "models-action-activate-restart"
    } else if models_ready {
        "models-action-install-update"
    } else {
        "models-action-download"
    })
}

fn localized_model_progress(
    localization: &Localization,
    message_id: &str,
    artifact: &str,
) -> String {
    let mut arguments = FluentArgs::new();
    arguments.set("artifact", artifact);
    localization.text_with(message_id, Some(&arguments))
}

fn localized_license(localization: &Localization, name: &str) -> String {
    let mut arguments = FluentArgs::new();
    arguments.set("name", name);
    localization.text_with("models-license", Some(&arguments))
}

fn onboarding_title(
    ui: &mut egui::Ui,
    title: &str,
    body: &str,
    density: crate::layout::LayoutDensity,
) {
    let (title_size, body_size, gap) = match density {
        crate::layout::LayoutDensity::Compact => (28.0, 14.0, 12.0),
        crate::layout::LayoutDensity::Comfortable => (34.0, 15.0, 20.0),
    };
    ui.heading(
        RichText::new(title)
            .size(title_size)
            .family(crate::theme::semibold_font_family()),
    );
    ui.add(egui::Label::new(RichText::new(body).size(body_size)).wrap());
    ui.add_space(gap);
}

const fn onboarding_intro_step(page: OnboardingPage) -> i64 {
    match page {
        OnboardingPage::Welcome => 1,
        OnboardingPage::Model => 2,
        OnboardingPage::Permissions => 3,
    }
}

const fn onboarding_machine_status(capability: Option<(bool, bool)>) -> &'static str {
    match capability {
        None => "onboarding-machine-checking",
        Some((true, true)) => "onboarding-machine-supported",
        Some((true, false)) => "onboarding-machine-needs-attention",
        Some((false, _)) => "onboarding-machine-unsupported",
    }
}

fn hardware_platform_name(os: &str, architecture: &str) -> String {
    match (os, architecture) {
        ("macos", "aarch64") => "Apple Silicon".to_owned(),
        ("windows", "x86_64") => "Windows x64".to_owned(),
        _ => format!("{os} {architecture}"),
    }
}

fn rounded_gib(bytes: u64) -> u64 {
    (bytes as f64 / 1024_f64.powi(3)).round() as u64
}

fn onboarding_machine_checks(report: &HardwareReport) -> (bool, bool, bool) {
    const GIB: u64 = 1024 * 1024 * 1024;
    let platform_ready = report.supported_target && (report.os != "windows" || report.avx2);
    let memory_ready = report.total_memory_bytes >= 8 * GIB;
    let disk_ready = report.available_disk_bytes >= GIB;
    (platform_ready, memory_ready, disk_ready)
}

fn onboarding_machine_row(ui: &mut egui::Ui, message: &str, ready: Option<bool>) {
    let color = match ready {
        Some(true) => crate::theme::accent_text(ui.visuals().dark_mode),
        Some(false) => crate::theme::warning_text(ui.visuals().dark_mode),
        None => crate::theme::secondary_text(ui.visuals().dark_mode),
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
        if ready == Some(true) {
            ui.painter()
                .circle_stroke(rect.center(), 7.0, egui::Stroke::new(1.2, color));
            ui.painter().line_segment(
                [
                    rect.center() + egui::vec2(-3.5, 0.0),
                    rect.center() + egui::vec2(-0.8, 3.0),
                ],
                egui::Stroke::new(1.2, color),
            );
            ui.painter().line_segment(
                [
                    rect.center() + egui::vec2(-0.8, 3.0),
                    rect.center() + egui::vec2(4.0, -3.5),
                ],
                egui::Stroke::new(1.2, color),
            );
        } else if ready == Some(false) {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "!",
                egui::FontId::new(15.0, crate::theme::semibold_font_family()),
                color,
            );
        } else {
            ui.painter()
                .circle_stroke(rect.center(), 7.0, egui::Stroke::new(1.2, color));
        }
        ui.label(message);
    });
}

fn onboarding_footer_button_rects(
    available: egui::Rect,
    button_size: egui::Vec2,
) -> (egui::Rect, egui::Rect) {
    let vertical_center = available.center().y;
    let back = egui::Rect::from_center_size(
        egui::pos2(available.left() + button_size.x / 2.0, vertical_center),
        button_size,
    );
    let primary = egui::Rect::from_center_size(
        egui::pos2(available.right() - button_size.x / 2.0, vertical_center),
        button_size,
    );
    (back, primary)
}

const fn wiki_health_can_refresh(state: &WikiHealthCheckState) -> bool {
    matches!(state, WikiHealthCheckState::Ready)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardingPermissionIcon {
    Folder,
    Network,
    Power,
}

fn onboarding_permission_row(
    ui: &mut egui::Ui,
    icon: OnboardingPermissionIcon,
    title: &str,
    body: &str,
) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
        paint_onboarding_permission_icon(
            ui.painter(),
            rect,
            icon,
            crate::theme::accent_text(ui.visuals().dark_mode),
        );
        ui.vertical(|ui| {
            ui.heading(
                RichText::new(title)
                    .size(16.0)
                    .family(crate::theme::semibold_font_family()),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(body).color(crate::theme::secondary_text(ui.visuals().dark_mode)),
                )
                .wrap(),
            );
        });
    });
    ui.add_space(16.0);
}

fn paint_onboarding_permission_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: OnboardingPermissionIcon,
    color: Color32,
) {
    let stroke = egui::Stroke::new(1.4, color);
    match icon {
        OnboardingPermissionIcon::Folder => {
            let folder = [
                rect.left_top() + egui::vec2(2.0, 6.0),
                rect.left_top() + egui::vec2(8.0, 6.0),
                rect.left_top() + egui::vec2(10.0, 9.0),
                rect.right_top() + egui::vec2(-2.0, 9.0),
                rect.right_bottom() + egui::vec2(-2.0, -3.0),
                rect.left_bottom() + egui::vec2(2.0, -3.0),
            ];
            painter.add(egui::Shape::closed_line(folder.to_vec(), stroke));
        }
        OnboardingPermissionIcon::Network => {
            let center = rect.center_bottom() - egui::vec2(0.0, 4.0);
            for (radius, sweep) in [(9.0, 0.9), (6.0, 0.75), (3.0, 0.55)] {
                painter.add(egui::Shape::line(
                    (0..=12)
                        .map(|step| {
                            let angle =
                                std::f32::consts::PI * (1.0 + (step as f32 / 12.0 - 0.5) * sweep);
                            center + egui::vec2(angle.cos(), angle.sin()) * radius
                        })
                        .collect(),
                    stroke,
                ));
            }
            painter.circle_filled(center, 1.6, color);
        }
        OnboardingPermissionIcon::Power => {
            painter.circle_stroke(rect.center(), 8.0, stroke);
            painter.rect_filled(
                egui::Rect::from_center_size(
                    rect.center_top() + egui::vec2(0.0, 7.0),
                    egui::vec2(5.0, 11.0),
                ),
                0.0,
                crate::theme::paper(false),
            );
            painter.line_segment(
                [
                    rect.center_top() + egui::vec2(0.0, 2.0),
                    rect.center() + egui::vec2(0.0, 2.0),
                ],
                stroke,
            );
        }
    }
}

fn scroll_newly_focused_control_into_view(ui: &egui::Ui) {
    let response = ui
        .memory(|memory| memory.focused())
        .and_then(|focused| ui.ctx().read_response(focused));
    if let Some(response) = response
        && focused_control_needs_scroll(
            response.gained_focus(),
            ui.min_rect(),
            ui.clip_rect(),
            response.rect,
        )
    {
        response.scroll_to_me(None);
    }
}

fn focused_control_needs_scroll(
    gained_focus: bool,
    body_rect: egui::Rect,
    visible_rect: egui::Rect,
    control_rect: egui::Rect,
) -> bool {
    gained_focus
        && body_rect.intersects(control_rect)
        && (control_rect.top() < visible_rect.top()
            || control_rect.bottom() > visible_rect.bottom())
}

fn selected_review_after_refresh(
    selected: Option<Uuid>,
    reviews: &[ReviewItemView],
) -> Option<Uuid> {
    selected
        .filter(|selected| reviews.iter().any(|review| review.concept_id == *selected))
        .or_else(|| reviews.first().map(|review| review.concept_id))
}

fn journey_stage_copy(localization: &Localization, stage: FirstKnowledgeStage) -> (String, String) {
    let (title, body) = match stage {
        FirstKnowledgeStage::PrepareLocalAi => ("onboarding-model-title", "onboarding-model-body"),
        FirstKnowledgeStage::ChooseKnowledgeFolder => {
            ("onboarding-collection-title", "onboarding-collection-body")
        }
        FirstKnowledgeStage::ProcessKnowledge => {
            ("onboarding-processing-title", "onboarding-processing-body")
        }
        FirstKnowledgeStage::ReviewKnowledge => {
            ("onboarding-review-title", "onboarding-review-body")
        }
        FirstKnowledgeStage::PublishReady => {
            ("knowledge-updating-title", "knowledge-updating-body")
        }
        FirstKnowledgeStage::SearchKnowledge => {
            ("onboarding-search-title", "onboarding-search-body")
        }
    };
    (localization.text(title), localization.text(body))
}

fn first_knowledge_readiness_status(state: FirstKnowledgeStepState) -> ReadinessStatus {
    match state {
        FirstKnowledgeStepState::NeedsPermission => ReadinessStatus::NeedsPermission,
        FirstKnowledgeStepState::NeedsAttention => ReadinessStatus::NeedsAttention,
        FirstKnowledgeStepState::Current | FirstKnowledgeStepState::Working => {
            ReadinessStatus::Working
        }
        FirstKnowledgeStepState::Complete | FirstKnowledgeStepState::Pending => {
            ReadinessStatus::Ready
        }
    }
}

impl eframe::App for AirWikiApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(error) = self.shell.ensure_tray() {
            self.notices.push_back((
                true,
                sanitized_error_code(&format!("tray unavailable: {error}")).to_owned(),
            ));
            self.shell.show(context);
        }
        for action in self.instance.try_actions() {
            if action == ActivationAction::Show {
                self.shell.show(context);
            }
        }
        self.shell.handle_frame(context, self.close_policy());
        self.drain_events();
        if self.exit_after_update_launch {
            self.exit_after_update_launch = false;
            self.shell.request_exit(context);
        }
        let readiness = self.readiness_view();
        let tray_status = if readiness.primary_action.is_some() {
            format!(
                "AirWiki · {}",
                self.localization.text("status-needs-attention")
            )
        } else {
            format!("AirWiki · {}", self.localization.text("status-ready"))
        };
        self.shell.set_status(&tray_status);
        self.shell.set_labels(
            &self.localization.text("tray-open"),
            &self.localization.text("tray-quit"),
        );
        context.request_repaint_after(if self.shell.hidden() {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(150)
        });
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        self.title_bar(root);
        if self.onboarding_page.is_some() {
            self.status_bar(root);
            let layout = ResponsiveLayout::from_available(root.available_size());
            let onboarding_margin = if layout.is_narrow() {
                layout.content_margin()
            } else {
                egui::Margin {
                    left: 72,
                    right: 72,
                    top: 56,
                    bottom: 40,
                }
            };
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(crate::theme::paper(root.visuals().dark_mode))
                        .inner_margin(onboarding_margin),
                )
                .show(root, |ui| {
                    self.onboarding(ui);
                });
            self.onboarding_notices(root);
            self.close_confirmation(root.ctx());
            return;
        }
        self.status_bar(root);
        self.sidebar(root);
        if self.screen != Screen::Setup {
            self.notices(root);
        }
        let layout = ResponsiveLayout::from_available(root.available_size());
        let content_margin = if self.screen == Screen::Review {
            egui::Margin::ZERO
        } else {
            layout.content_margin()
        };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(crate::theme::paper(root.visuals().dark_mode))
                    .inner_margin(content_margin),
            )
            .show(root, |ui| match self.screen {
                Screen::Setup => {
                    let viewport = ui.available_size();
                    egui::ScrollArea::vertical()
                        .id_salt("today_scroll")
                        .max_height(viewport.y)
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            ui.set_min_width(viewport.x);
                            self.home(ui);
                            scroll_newly_focused_control_into_view(ui);
                        });
                }
                Screen::Models => self.setup(ui),
                Screen::Collections => {
                    let viewport = ui.available_size();
                    egui::ScrollArea::vertical()
                        .id_salt("collections_page_scroll")
                        .max_height(viewport.y)
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            ui.set_min_width(viewport.x);
                            self.collections(ui);
                            scroll_newly_focused_control_into_view(ui);
                        });
                }
                Screen::Review => self.review(ui),
                Screen::Knowledge => self.knowledge(ui),
                Screen::Search => {
                    let viewport = ui.available_size();
                    egui::ScrollArea::vertical()
                        .id_salt("search_page_scroll")
                        .max_height(viewport.y)
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            ui.set_min_width(viewport.x);
                            self.search(ui);
                            scroll_newly_focused_control_into_view(ui);
                        });
                }
                Screen::Public => {
                    let viewport = ui.available_size();
                    egui::ScrollArea::vertical()
                        .id_salt("public_page_scroll")
                        .max_height(viewport.y)
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            ui.set_min_width(viewport.x);
                            self.public_network(ui);
                            scroll_newly_focused_control_into_view(ui);
                        });
                }
                Screen::Integrations => self.integrations(ui),
                Screen::Nodes => self.nodes(ui),
                Screen::Settings => self.settings(ui),
            });
        self.community_indexes_confirmation_window(root.ctx());
        self.close_confirmation(root.ctx());
    }
}

fn configure_style(context: &egui::Context) {
    crate::theme::apply(context);
}

fn editorial_modal_frame(dark_mode: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(crate::theme::paper(dark_mode))
        .stroke(egui::Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(20))
        .shadow(egui::epaint::Shadow {
            offset: [0, 12],
            blur: 32,
            spread: 0,
            color: Color32::from_black_alpha(56),
        })
}

fn editorial_title_row_button(
    ui: &mut egui::Ui,
    title: &str,
    trailing: Option<&str>,
    title_size: f32,
) -> egui::Response {
    let dark_mode = ui.visuals().dark_mode;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            1.0,
            egui::Stroke::new(2.0, crate::theme::AIR_CYAN),
            egui::StrokeKind::Inside,
        );
    }
    let trailing_galley = trailing.map(|trailing| {
        ui.painter().layout_no_wrap(
            trailing.to_owned(),
            egui::FontId::proportional(12.0),
            crate::theme::secondary_text(dark_mode),
        )
    });
    let trailing_width = trailing_galley
        .as_ref()
        .map_or(0.0, |galley| galley.size().x);
    let title_width = editorial_title_available_width(rect.width(), trailing_width);
    let title_galley = ui.painter().layout_job(crate::theme::truncated_title_job(
        title,
        egui::FontId::new(title_size, crate::theme::semibold_font_family()),
        crate::theme::ink(dark_mode),
        title_width,
    ));
    let title_elided = title_galley.elided;
    ui.painter().galley(
        egui::pos2(
            rect.left() + 4.0,
            rect.center().y - title_galley.size().y / 2.0,
        ),
        title_galley,
        crate::theme::ink(dark_mode),
    );
    if let Some(trailing_galley) = trailing_galley {
        ui.painter().galley(
            egui::pos2(
                rect.right() - 4.0 - trailing_galley.size().x,
                rect.center().y - trailing_galley.size().y / 2.0,
            ),
            trailing_galley,
            crate::theme::secondary_text(dark_mode),
        );
    }
    let response = if title_elided {
        response.on_hover_text(title)
    } else {
        response
    };
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    response
}

fn editorial_title_available_width(row_width: f32, trailing_width: f32) -> f32 {
    let trailing_reservation = if trailing_width > 0.0 {
        trailing_width + 12.0
    } else {
        0.0
    };
    (row_width - 8.0 - trailing_reservation).max(0.0)
}

fn review_queue_button(
    ui: &mut egui::Ui,
    title: &str,
    metadata: &str,
    selected: bool,
) -> egui::Response {
    let dark_mode = ui.visuals().dark_mode;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 58.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, title)
    });
    if selected {
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(0, 136, 176, 22));
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            0.0,
            egui::Stroke::new(2.0, crate::theme::AIR_CYAN),
            egui::StrokeKind::Inside,
        );
    }
    let text_x = rect.left() + 22.0;
    ui.painter().text(
        egui::pos2(text_x, rect.center().y - 9.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::new(15.0, crate::theme::semibold_font_family()),
        if selected {
            crate::theme::accent_text(dark_mode)
        } else {
            crate::theme::ink(dark_mode)
        },
    );
    ui.painter().text(
        egui::pos2(text_x, rect.center().y + 10.0),
        egui::Align2::LEFT_CENTER,
        metadata,
        egui::FontId::proportional(12.0),
        crate::theme::secondary_text(dark_mode),
    );
    let response = response.on_hover_text(format!("{title}\n{metadata}"));
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    response
}

fn nav(
    ui: &mut egui::Ui,
    current: &mut Screen,
    target: Screen,
    icon: NavIcon,
    label: &str,
    badge: Option<usize>,
) {
    let selected = nav_is_selected(*current, target);
    let dark_mode = ui.visuals().dark_mode;
    let color = if selected {
        crate::theme::accent_text(dark_mode)
    } else {
        crate::theme::ink(dark_mode)
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::click());
    let accessible_label = badge
        .filter(|count| *count > 0)
        .map_or_else(|| label.to_owned(), |count| format!("{label} ({count})"));
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &accessible_label)
    });
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            0.0,
            egui::Stroke::new(2.0, crate::theme::AIR_CYAN),
            egui::StrokeKind::Inside,
        );
    }
    paint_nav_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 31.0, rect.center().y),
            egui::vec2(18.0, 18.0),
        ),
        icon,
        color,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 50.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::new(
            15.0,
            if selected {
                crate::theme::semibold_font_family()
            } else {
                egui::FontFamily::Proportional
            },
        ),
        color,
    );
    if let Some(count) = badge.filter(|count| *count > 0) {
        let text = count.to_string();
        let text_color = crate::theme::attention_strong(dark_mode);
        let mut text_job = egui::text::LayoutJob::simple_singleline(
            text,
            egui::FontId::proportional(11.0),
            text_color,
        );
        if let Some(section) = text_job.sections.first_mut() {
            section.format.extra_letter_spacing = 0.22;
        }
        let text_galley = ui.painter().layout_job(text_job);
        let badge_size = text_galley.size() + egui::vec2(20.0, 6.0);
        let badge_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 22.0 - badge_size.x / 2.0, rect.center().y),
            badge_size,
        );
        ui.painter().rect_filled(
            badge_rect,
            egui::CornerRadius::same(2),
            crate::theme::attention_tint(dark_mode),
        );
        ui.painter().galley(
            badge_rect.center() - text_galley.size() / 2.0,
            text_galley,
            text_color,
        );
    }
    if response.clicked() {
        *current = target;
    }
}

fn paint_nav_icon(painter: &egui::Painter, rect: egui::Rect, icon: NavIcon, color: Color32) {
    let stroke = egui::Stroke::new(1.4, color);
    let thin = egui::Stroke::new(1.0, color);
    let [red, green, blue, _] = color.to_array();
    let secondary = Color32::from_rgba_unmultiplied(red, green, blue, 46);
    match icon {
        NavIcon::Today => {
            painter.rect_filled(rect.shrink(1.0), 1.0, secondary);
            painter.rect_stroke(rect.shrink(1.0), 1.0, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    rect.left_top() + egui::vec2(5.5, 1.5),
                    rect.left_bottom() + egui::vec2(5.5, -1.5),
                ],
                thin,
            );
            for y in [5.0, 9.0, 13.0] {
                painter.line_segment(
                    [
                        rect.left_top() + egui::vec2(8.5, y),
                        rect.right_top() + egui::vec2(-2.0, y),
                    ],
                    thin,
                );
            }
        }
        NavIcon::Library => {
            let points = [
                rect.left_top() + egui::vec2(1.0, 4.0),
                rect.left_top() + egui::vec2(6.0, 4.0),
                rect.left_top() + egui::vec2(8.0, 6.0),
                rect.right_top() + egui::vec2(-1.0, 6.0),
                rect.right_bottom() + egui::vec2(-1.0, -2.0),
                rect.left_bottom() + egui::vec2(1.0, -2.0),
            ];
            painter.add(egui::Shape::convex_polygon(
                points.to_vec(),
                secondary,
                egui::Stroke::NONE,
            ));
            painter.add(egui::Shape::closed_line(points.to_vec(), stroke));
        }
        NavIcon::Review => {
            for (index, y) in [3.0, 8.0, 13.0].into_iter().enumerate() {
                let box_rect = egui::Rect::from_min_size(
                    rect.left_top() + egui::vec2(1.0, y),
                    egui::vec2(3.0, 3.0),
                );
                painter.rect_filled(box_rect, 0.0, secondary);
                painter.rect_stroke(box_rect, 0.0, thin, egui::StrokeKind::Inside);
                if index == 0 {
                    painter.line_segment([box_rect.left_center(), box_rect.center_bottom()], thin);
                }
                painter.line_segment(
                    [
                        rect.left_top() + egui::vec2(7.0, y + 1.5),
                        rect.right_top() + egui::vec2(-1.0, y + 1.5),
                    ],
                    thin,
                );
            }
        }
        NavIcon::Wiki => {
            painter.rect_filled(rect.shrink(1.0), 1.0, secondary);
            painter.rect_stroke(rect.shrink(1.0), 1.0, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    rect.center_top() + egui::vec2(0.0, 1.0),
                    rect.center_bottom() - egui::vec2(0.0, 1.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    rect.left_top() + egui::vec2(3.0, 5.0),
                    rect.center_top() + egui::vec2(-2.0, 5.0),
                ],
                thin,
            );
            painter.line_segment(
                [
                    rect.center_top() + egui::vec2(2.0, 5.0),
                    rect.right_top() + egui::vec2(-3.0, 5.0),
                ],
                thin,
            );
        }
        NavIcon::Ask => {
            let bubble = rect.shrink2(egui::vec2(1.0, 3.0));
            painter.rect_filled(bubble, 5.0, secondary);
            painter.rect_stroke(bubble, 5.0, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    bubble.left_bottom() + egui::vec2(4.0, 0.0),
                    bubble.left_bottom() + egui::vec2(2.0, 3.0),
                ],
                stroke,
            );
        }
        NavIcon::Public => {
            painter.circle_filled(rect.center(), 8.0, secondary);
            painter.circle_stroke(rect.center(), 8.0, stroke);
            painter.line_segment([rect.left_center(), rect.right_center()], thin);
            painter.add(egui::Shape::ellipse_stroke(
                rect.center(),
                egui::vec2(3.0, 7.5),
                thin,
            ));
        }
        NavIcon::Connections => {
            let left = egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(1.0, 1.0),
                egui::vec2(9.0, 12.0),
            );
            let right = egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(8.0, 5.0),
                egui::vec2(9.0, 12.0),
            );
            painter.rect_filled(left, 1.0, secondary);
            painter.rect_filled(right, 1.0, secondary);
            painter.rect_stroke(left, 1.0, stroke, egui::StrokeKind::Inside);
            painter.rect_stroke(right, 1.0, stroke, egui::StrokeKind::Inside);
        }
        NavIcon::Settings => {
            painter.circle_filled(rect.center(), 7.0, secondary);
            painter.circle_stroke(rect.center(), 4.0, stroke);
            painter.circle_stroke(rect.center(), 1.2, stroke);
            for direction in [
                egui::vec2(0.0, -1.0),
                egui::vec2(1.0, 0.0),
                egui::vec2(0.0, 1.0),
                egui::vec2(-1.0, 0.0),
            ] {
                painter.line_segment(
                    [
                        rect.center() + direction * 5.0,
                        rect.center() + direction * 8.0,
                    ],
                    stroke,
                );
            }
        }
    }
}

fn nav_is_selected(current: Screen, target: Screen) -> bool {
    current == target || (current == Screen::Integrations && target == Screen::Search)
}

const fn ask_scope_presentation(
    lan_preference: LanPreference,
    has_trusted_peer: bool,
) -> AskScopePresentation {
    let paired_available = has_trusted_peer && matches!(lan_preference, LanPreference::Enabled);
    AskScopePresentation { paired_available }
}

const fn blocking_modal_decision(explicit: Option<bool>, escaped: bool) -> Option<bool> {
    match explicit {
        Some(decision) => Some(decision),
        None if escaped => Some(false),
        None => None,
    }
}

fn restore_modal_focus(context: &egui::Context, return_focus: Option<egui::Id>) {
    if let Some(return_focus) = return_focus {
        context.memory_mut(|memory| memory.request_focus(return_focus));
    }
}

const fn effective_public_search(
    public_only: bool,
    allow_ask_public_network: bool,
    ask_preference: bool,
) -> bool {
    public_only || (allow_ask_public_network && ask_preference)
}

struct SearchInputText<'a> {
    question: Option<&'a str>,
    external_label: Option<egui::Id>,
    placeholder: &'a str,
    action: &'a str,
    max_width: f32,
    primary_action: bool,
}

fn show_search_inputs(
    ui: &mut egui::Ui,
    layout: ResponsiveLayout,
    search: &mut SearchViewState,
    search_running: bool,
    show_top_k: bool,
    text: SearchInputText<'_>,
) -> (egui::Response, bool) {
    let mut submit_clicked = false;
    let max_width = if show_top_k { 760.0 } else { text.max_width };
    ui.scope(|ui| {
        ui.set_max_width(ui.available_width().min(max_width));
        let question_label = text
            .question
            .map(|question| ui.label(question).id)
            .or(text.external_label);
        let response = if layout.is_narrow() {
            let response = ui
                .add_enabled_ui(!search_running, |ui| {
                    ui.add_sized(
                        [ui.available_width(), 36.0],
                        egui::TextEdit::singleline(&mut search.question)
                            .hint_text(text.placeholder),
                    )
                })
                .inner;
            if let Some(question_label) = question_label {
                response.labelled_by(question_label)
            } else {
                response
            }
        } else {
            ui.horizontal(|ui| {
                let reserved_width = if show_top_k { 190.0 } else { 100.0 };
                let field_width = (ui.available_width() - reserved_width).max(220.0);
                let response = ui
                    .add_enabled_ui(!search_running, |ui| {
                        ui.add_sized(
                            [field_width, 36.0],
                            egui::TextEdit::singleline(&mut search.question)
                                .hint_text(text.placeholder),
                        )
                    })
                    .inner;
                let response = if let Some(question_label) = question_label {
                    response.labelled_by(question_label)
                } else {
                    response
                };
                if show_top_k {
                    ui.add_enabled(
                        !search_running,
                        egui::DragValue::new(&mut search.top_k)
                            .range(1..=10)
                            .prefix("Top "),
                    );
                }
                let action = if text.primary_action {
                    first_knowledge::primary_button(text.action.to_owned())
                } else {
                    crate::theme::focus_button(
                        egui::Button::new(text.action.to_owned()),
                        crate::theme::AIR_CYAN,
                    )
                };
                submit_clicked = ui
                    .add_enabled(
                        !search.question.trim().is_empty() && !search_running,
                        action,
                    )
                    .clicked();
                response
            })
            .inner
        };
        if layout.is_narrow() {
            if show_top_k {
                ui.add_enabled(
                    !search_running,
                    egui::DragValue::new(&mut search.top_k)
                        .range(1..=10)
                        .prefix("Top "),
                );
            }
            let action = if text.primary_action {
                first_knowledge::primary_button(text.action.to_owned())
            } else {
                crate::theme::focus_button(
                    egui::Button::new(text.action.to_owned()),
                    crate::theme::AIR_CYAN,
                )
            };
            submit_clicked = ui
                .add_enabled(
                    !search.question.trim().is_empty() && !search_running,
                    action,
                )
                .clicked();
        }
        (response, submit_clicked)
    })
    .inner
}

fn search_response_surface(
    active_search: Option<ActiveSearch>,
    event_request_id: Uuid,
) -> Option<SearchSurface> {
    active_search
        .filter(|active| active.request_id == event_request_id)
        .map(|active| active.surface)
}

fn remove_blocked_publisher_hits(
    ask_search: &mut SearchViewState,
    public_search: &mut SearchViewState,
    publisher_id: &str,
) {
    public_search.hits.retain(|hit| hit.node_id != publisher_id);
    if ask_search.submitted_public_network {
        ask_search.hits.retain(|hit| hit.node_id != publisher_id);
    }
}

fn wrap_monospace(ui: &mut egui::Ui, value: impl AsRef<str>) {
    ui.add(
        egui::Label::new(RichText::new(value.as_ref()).monospace())
            .selectable(false)
            .wrap(),
    );
}

fn wrap_rich_text(ui: &mut egui::Ui, text: RichText) {
    ui.add(egui::Label::new(text).selectable(false).wrap());
}

fn editorial_card_kicker(
    ui: &mut egui::Ui,
    label: impl Into<String>,
    color: Color32,
) -> egui::Response {
    ui.label(crate::theme::card_kicker_job(label, color))
}

fn editorial_section_label(
    ui: &mut egui::Ui,
    label: impl Into<String>,
    color: Color32,
) -> egui::Response {
    ui.label(crate::theme::section_label_job(label, color))
}

fn editorial_tag(ui: &mut egui::Ui, label: &str, tone: EditorialTagTone) -> egui::Response {
    let dark_mode = ui.visuals().dark_mode;
    let (fill, stroke, color) = match tone {
        EditorialTagTone::Accent => (
            crate::theme::accent_tint(dark_mode),
            egui::Stroke::NONE,
            crate::theme::accent_tag_text(dark_mode),
        ),
        EditorialTagTone::Attention => (
            crate::theme::attention_tint(dark_mode),
            egui::Stroke::NONE,
            crate::theme::attention_strong(dark_mode),
        ),
        EditorialTagTone::Neutral => (
            crate::theme::neutral_tint(dark_mode),
            egui::Stroke::NONE,
            crate::theme::neutral_tag_text(dark_mode),
        ),
        EditorialTagTone::Outline => (
            Color32::TRANSPARENT,
            egui::Stroke::new(1.0, crate::theme::accent_text(dark_mode)),
            crate::theme::accent_text(dark_mode),
        ),
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(egui::CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            let mut job = egui::text::LayoutJob::simple_singleline(
                label.to_owned(),
                egui::FontId::proportional(11.0),
                color,
            );
            if let Some(section) = job.sections.first_mut() {
                section.format.extra_letter_spacing = 0.22;
            }
            ui.label(job)
        })
        .inner
}

fn page_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.heading(
        RichText::new(title)
            .size(32.0)
            .family(crate::theme::semibold_font_family()),
    );
    ui.scope(|ui| {
        ui.set_max_width(ui.available_width().min(600.0));
        ui.add(
            egui::Label::new(
                RichText::new(subtitle)
                    .size(15.0)
                    .color(crate::theme::secondary_text(ui.visuals().dark_mode)),
            )
            .wrap(),
        );
    });
    ui.add_space(30.0);
}

fn empty_state(ui: &mut egui::Ui, title: &str, body: &str) {
    egui::Frame::new()
        .fill(crate::theme::surface(ui.visuals().dark_mode))
        .stroke(egui::Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(2))
        .inner_margin(egui::Margin::same(24))
        .show(ui, |ui| {
            ui.heading(RichText::new(title).size(24.0));
            ui.add(egui::Label::new(body).wrap());
        });
}

fn deduplicate_notices(notices: &mut VecDeque<(bool, String)>) {
    let mut seen = HashSet::new();
    notices.retain(|notice| seen.insert(notice.clone()));
}

fn wiki_health_result_applies(last_generation: u64, event_generation: u64) -> bool {
    event_generation > last_generation
}

fn elapsed_minutes(checked_at: SystemTime, now: SystemTime) -> u64 {
    now.duration_since(checked_at)
        .map_or(0, |elapsed| elapsed.as_secs() / 60)
}

fn wiki_health_readiness_inputs(
    check: &WikiHealthCheckState,
    summary: &WikiHealthSummaryView,
) -> (bool, usize) {
    let working = matches!(check, WikiHealthCheckState::Loading) || summary.updating_count > 0;
    let failed_check = usize::from(matches!(check, WikiHealthCheckState::Failed(_)));
    let issues = summary
        .error_count
        .saturating_add(summary.warning_count)
        .saturating_add(failed_check);
    (working, issues)
}

fn connectivity_runtime_is_active(
    snapshot: Option<ConnectivityPlatformSnapshot>,
    listener: LanListenerView,
    discovery: LanDiscoveryView,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    listener == LanListenerView::Listening
        && discovery == LanDiscoveryView::Active
        && matches!(
            snapshot.network_profile,
            NetworkProfileState::NotApplicable
                | NetworkProfileState::Private
                | NetworkProfileState::Domain
        )
        && matches!(
            snapshot.firewall,
            FirewallDiagnosticState::Ready | FirewallDiagnosticState::NotApplicable
        )
        && snapshot.system_permission != crate::connectivity_platform::SystemPermissionState::Denied
}

fn firewall_configuration_is_current(
    preference: LanPreference,
    snapshot: Option<ConnectivityPlatformSnapshot>,
    operation_in_progress: bool,
) -> bool {
    if preference != LanPreference::Enabled || operation_in_progress {
        return false;
    }
    snapshot.is_some_and(|snapshot| {
        matches!(
            snapshot.network_profile,
            NetworkProfileState::Private | NetworkProfileState::Domain
        ) && snapshot.firewall == FirewallDiagnosticState::RulesMissing
            && snapshot.firewall_helper.can_request_elevation()
    })
}

const fn firewall_state_offers_advanced_recovery(state: FirewallDiagnosticState) -> bool {
    matches!(
        state,
        FirewallDiagnosticState::Conflict | FirewallDiagnosticState::LegacyExposure
    )
}

fn parse_manual_ipv4_address(input: &str) -> Option<ManualLanAddress> {
    input
        .trim()
        .parse::<ManualLanAddress>()
        .ok()
        .filter(|address| address.ip_addr().is_ipv4())
}

fn edit_draft(ui: &mut egui::Ui, localization: &Localization, draft: &mut EnrichmentDraft) {
    ui.add(
        egui::Label::new(
            RichText::new(localization.text("review-metadata-title"))
                .size(24.0)
                .family(crate::theme::semibold_font_family()),
        )
        .wrap(),
    );
    egui::ComboBox::from_label(localization.text("review-field-type"))
        .selected_text(draft.concept_type.to_string())
        .show_ui(ui, |ui| {
            for value in [
                ConceptType::Document,
                ConceptType::Policy,
                ConceptType::Procedure,
                ConceptType::Runbook,
                ConceptType::Reference,
                ConceptType::Report,
            ] {
                ui.selectable_value(&mut draft.concept_type, value, value.to_string());
            }
        });
    let title_label = ui.label(localization.text("review-field-title"));
    ui.text_edit_singleline(&mut draft.title)
        .labelled_by(title_label.id);
    let description_label = ui.label(localization.text("review-field-description"));
    ui.text_edit_multiline(&mut draft.description)
        .labelled_by(description_label.id);
    let language_label = ui.label(localization.text("review-field-language"));
    ui.text_edit_singleline(&mut draft.language)
        .labelled_by(language_label.id);
    let mut tags = draft.tags.join(", ");
    let tags_label = ui.label(localization.text("review-field-tags"));
    if ui
        .text_edit_singleline(&mut tags)
        .labelled_by(tags_label.id)
        .changed()
    {
        draft.tags = tags
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .take(10)
            .map(str::to_owned)
            .collect();
    }
    let summary_label = ui.label(localization.text("review-field-summary"));
    ui.text_edit_multiline(&mut draft.summary)
        .labelled_by(summary_label.id);
    ui.separator();
    ui.label(localization.text("review-field-entities"));
    let mut remove_entity = None;
    for (index, entity) in draft.entities.iter_mut().enumerate() {
        let entity_identifier = if entity.name.trim().is_empty() {
            format!("#{}", index + 1)
        } else {
            entity.name.clone()
        };
        let render_fields = |ui: &mut egui::Ui| {
            ui.vertical(|ui| {
                let label = ui.label(localization.text("review-entity-name"));
                ui.add(egui::TextEdit::singleline(&mut entity.name).desired_width(180.0))
                    .labelled_by(label.id);
            });
            ui.vertical(|ui| {
                let label = ui.label(localization.text("review-entity-type"));
                ui.add(egui::TextEdit::singleline(&mut entity.kind).desired_width(160.0))
                    .labelled_by(label.id);
            });
            if ui
                .small_button(format!(
                    "{} {entity_identifier}",
                    localization.text("action-remove")
                ))
                .clicked()
            {
                remove_entity = Some(index);
            }
        };
        ui.push_id(("entity", index), |ui| {
            if review_fields_stack(ui.available_width()) {
                ui.vertical(render_fields);
            } else {
                ui.horizontal(render_fields);
            }
        });
    }
    if let Some(index) = remove_entity {
        draft.entities.remove(index);
    }
    if ui
        .small_button(localization.text("review-add-entity"))
        .clicked()
    {
        draft.entities.push(SuggestedEntity {
            name: String::new(),
            kind: String::new(),
        });
    }
    ui.label(localization.text("review-field-links"));
    let mut remove_link = None;
    for (index, link) in draft.links.iter_mut().enumerate() {
        let link_identifier = if link.label.trim().is_empty() {
            format!("#{}", index + 1)
        } else {
            link.label.clone()
        };
        let render_fields = |ui: &mut egui::Ui| {
            ui.vertical(|ui| {
                let label = ui.label(localization.text("review-link-label"));
                ui.add(egui::TextEdit::singleline(&mut link.label).desired_width(180.0))
                    .labelled_by(label.id);
            });
            ui.vertical(|ui| {
                let label = ui.label(localization.text("review-link-target"));
                ui.add(egui::TextEdit::singleline(&mut link.target).desired_width(220.0))
                    .labelled_by(label.id);
            });
            if ui
                .small_button(format!(
                    "{} {link_identifier}",
                    localization.text("action-remove")
                ))
                .clicked()
            {
                remove_link = Some(index);
            }
        };
        ui.push_id(("link", index), |ui| {
            if review_fields_stack(ui.available_width()) {
                ui.vertical(render_fields);
            } else {
                ui.horizontal(render_fields);
            }
        });
    }
    if let Some(index) = remove_link {
        draft.links.remove(index);
    }
    if ui
        .small_button(localization.text("review-add-link"))
        .clicked()
    {
        draft.links.push(SuggestedLink {
            label: String::new(),
            target: String::new(),
        });
    }
    ui.add(
        egui::Slider::new(&mut draft.classification_confidence, 0.0..=1.0)
            .text(localization.text("review-confidence")),
    );
    let classification_label = ui.label(localization.text("review-classification-explanation"));
    ui.text_edit_multiline(&mut draft.classification_explanation)
        .labelled_by(classification_label.id);
}

fn review_fields_stack(available_width: f32) -> bool {
    available_width < 520.0
}

fn connections_peer_list_height(available_height: f32) -> f32 {
    (available_height - CONNECTIONS_CHAT_FOOTER_HEIGHT).max(0.0)
}

fn today_column_widths(available_width: f32) -> (f32, f32) {
    let usable_width = (available_width - TODAY_COLUMN_GAP).max(0.0);
    let lead_width = (usable_width * 0.6).max(360.0).min(usable_width);
    (lead_width, usable_width - lead_width)
}

const fn collection_can_make_public(needs_review_count: usize, failed_count: usize) -> bool {
    needs_review_count == 0 && failed_count == 0
}

fn public_confirmation_can_commit(collection_counts: Option<(usize, usize)>) -> bool {
    collection_counts.is_some_and(|(needs_review_count, failed_count)| {
        collection_can_make_public(needs_review_count, failed_count)
    })
}

#[cfg(test)]
mod tests {
    use eframe::egui;
    use fluent_bundle::FluentArgs;

    use super::{
        ActiveSearch, ExternalAiPolicyChange, OnboardingPage, Screen, SearchResultAvailability,
        SearchSurface, SearchViewState, TODAY_COLUMN_GAP, WikiHealthCheckState,
        ask_scope_presentation, blocking_modal_decision, classify_external_ai_policy_change,
        classify_search_result, collection_can_make_public, connections_peer_list_height,
        connectivity_runtime_is_active, deduplicate_notices, editorial_title_available_width,
        effective_public_search, elapsed_minutes, firewall_configuration_is_current,
        firewall_operation_update_applies, firewall_state_offers_advanced_recovery,
        focused_control_needs_scroll, hardware_platform_name, human_error_summary,
        localized_worker_notice, model_action_label, nav_is_selected, onboarding_error_is_relevant,
        onboarding_footer_button_rects, onboarding_intro_step, onboarding_machine_checks,
        onboarding_machine_status, parse_manual_ipv4_address, peer_activity_message_id,
        primary_action_explanation, primary_action_title, public_confirmation_can_commit,
        remove_blocked_publisher_hits, review_fields_stack, rounded_gib, sanitized_error_code,
        search_coverage_message, search_response_surface, search_result_origin_label,
        should_present_pairing_controls, today_column_widths, updater_launched_installer,
        wiki_health_can_refresh, wiki_health_readiness_inputs, wiki_health_result_applies,
    };
    use crate::connectivity_platform::{
        ConnectivityPlatformSnapshot, FirewallDiagnosticState, FirewallHelperState,
        NetworkProfileState, SystemPermissionState,
    };
    use crate::i18n::{Localization, UiLocale};
    use crate::model_config::LanPreference;
    use crate::readiness::RecommendedAction;
    use crate::updater::{UpdateSummary, UpdaterStatus, UpdaterView};
    use crate::worker::{
        FirewallOperationView, LanDiscoveryView, LanListenerView, PeerActivityState,
        PeerTrustState, SearchCoverageView, SourceIssueView, UpdaterWorkerView,
        WikiHealthSummaryView,
    };
    use airwiki_core::SourceIssueCode;
    use airwiki_inference::HardwareReport;
    use airwiki_network::PublicRouteKind;
    use airwiki_types::SearchHit;
    use chrono::Utc;
    use std::{
        collections::VecDeque,
        time::{Duration, SystemTime},
    };
    use uuid::Uuid;

    fn synthetic_search_hit(node_id: &str) -> SearchHit {
        SearchHit {
            concept_id: Uuid::new_v4(),
            collection_id: Uuid::new_v4(),
            chunk_id: Uuid::new_v4(),
            title: "Synthetic result".to_owned(),
            snippet: "Synthetic evidence".to_owned(),
            heading_or_page: "Fixture".to_owned(),
            logical_resource_uri: "airwiki://fixture".to_owned(),
            source_revision: 1,
            source_sha256: "fixture-sha256".to_owned(),
            updated_at: Utc::now(),
            rank: 1,
            node_id: node_id.to_owned(),
        }
    }

    #[test]
    fn active_navigation_is_independent_from_keyboard_focus() {
        assert!(nav_is_selected(Screen::Public, Screen::Public));
        assert!(!nav_is_selected(Screen::Public, Screen::Collections));
        assert!(nav_is_selected(Screen::Integrations, Screen::Search));
        assert!(!nav_is_selected(Screen::Integrations, Screen::Nodes));
    }

    #[test]
    fn connections_peer_list_reserves_chat_access() {
        assert_eq!(connections_peer_list_height(420.0), 362.0);
        assert_eq!(connections_peer_list_height(40.0), 0.0);
    }

    #[test]
    fn today_columns_keep_an_exact_fifty_six_pixel_gutter() {
        let available = 900.0;
        let (lead, side) = today_column_widths(available);

        assert!((lead + TODAY_COLUMN_GAP + side - available).abs() < 0.01);
        assert_eq!(TODAY_COLUMN_GAP, 56.0);
    }

    #[test]
    fn today_title_reserves_trailing_metadata_width() {
        assert_eq!(editorial_title_available_width(320.0, 64.0), 236.0);
        assert_eq!(editorial_title_available_width(80.0, 96.0), 0.0);
    }

    #[test]
    fn public_action_is_hidden_while_collection_needs_attention() {
        assert!(collection_can_make_public(0, 0));
        assert!(!collection_can_make_public(1, 0));
        assert!(!collection_can_make_public(0, 1));
    }

    #[test]
    fn public_confirmation_fails_closed_after_attention_refresh() {
        let before_modal = collection_can_make_public(0, 0);
        let handler_recheck_after_refresh = public_confirmation_can_commit(Some((1, 0)));

        assert!(before_modal);
        assert!(!handler_recheck_after_refresh);
        assert!(!public_confirmation_can_commit(None));
    }

    #[test]
    fn ask_scope_exposes_only_real_paired_capability() {
        let device = ask_scope_presentation(LanPreference::Disabled, true);
        assert!(!device.paired_available);
        assert!(!ask_scope_presentation(LanPreference::Enabled, false).paired_available);
        let paired = ask_scope_presentation(LanPreference::Enabled, true);
        assert!(paired.paired_available);
    }

    #[test]
    fn ask_lead_microcopy_does_not_claim_a_derived_answer() {
        let english = Localization::new(UiLocale::EnUs).unwrap();
        let spanish = Localization::new(UiLocale::Es).unwrap();

        assert_eq!(english.text("search-top-passage"), "Top cited passage");
        assert_eq!(
            spanish.text("search-top-passage"),
            "Pasaje citado principal"
        );
        assert_eq!(
            english.text("search-subtitle"),
            "Cited passages come only from published knowledge — here or in your chat app."
        );
        assert_eq!(
            spanish.text("search-subtitle"),
            "Los pasajes citados provienen únicamente de conocimiento publicado, aquí o en tu \
             aplicación de chat."
        );
    }

    #[test]
    fn public_scope_microcopy_discloses_query_routing() {
        let english = Localization::new(UiLocale::EnUs).unwrap();
        let spanish = Localization::new(UiLocale::Es).unwrap();

        assert_eq!(
            english.text("search-scope-note-public"),
            "Local and paired knowledge remain available. Your query is sent to up to three \
             federated indexes and selected publishers; your documents stay on this device."
        );
        assert_eq!(
            spanish.text("search-scope-note-public"),
            "El conocimiento local y emparejado sigue disponible. Tu consulta se envía a un \
             máximo de tres índices federados y a los publicadores seleccionados; tus documentos \
             permanecen en este equipo."
        );
        assert_eq!(
            english.text("public-discover-title"),
            "Find public knowledge"
        );
        assert_eq!(
            spanish.text("public-discover-title"),
            "Encontrar conocimiento público"
        );
    }

    #[test]
    fn local_ai_microcopy_does_not_claim_answer_generation() {
        for locale in [UiLocale::EnUs, UiLocale::Es] {
            let localization = Localization::new(locale).unwrap();
            for key in ["onboarding-model-body", "settings-local-ai-body"] {
                let copy = localization.text(key);
                assert!(!copy.contains("answers") && !copy.contains("respuestas"));
                assert!(
                    copy.contains("cited passages") || copy.contains("pasajes citados"),
                    "unexpected local AI copy: {copy:?}"
                );
            }
        }
    }

    #[test]
    fn public_surface_cannot_route_through_private_wiki_reader() {
        let app_source = include_str!("app.rs");
        let knowledge_source = include_str!("app/knowledge.rs");
        let private_reader_state = ["public_reader_", "collection"].concat();

        assert!(!app_source.contains(&private_reader_state));
        assert!(!knowledge_source.contains("pub(super) fn show_reader"));
    }

    #[test]
    fn blocking_modal_escape_is_fail_closed() {
        assert_eq!(blocking_modal_decision(None, false), None);
        assert_eq!(blocking_modal_decision(None, true), Some(false));
        assert_eq!(blocking_modal_decision(Some(true), true), Some(true));
        assert_eq!(blocking_modal_decision(Some(false), false), Some(false));
    }

    #[test]
    fn public_screen_does_not_change_the_ask_search_preference() {
        for ask_preference in [false, true] {
            let original_preference = ask_preference;

            assert!(effective_public_search(true, false, ask_preference));
            assert_eq!(ask_preference, original_preference);
        }
    }

    #[test]
    fn ask_and_public_keep_independent_drafts_and_feedback() {
        let mut ask = SearchViewState::new();
        let public = SearchViewState::new();

        ask.question = "local draft".to_owned();
        ask.completed = true;
        ask.coverage = SearchCoverageView::Partial;
        ask.error = Some("fixture-error".to_owned());

        assert_eq!(ask.question, "local draft");
        assert!(ask.completed);
        assert!(ask.error.is_some());
        assert!(public.question.is_empty());
        assert!(!public.completed);
        assert_eq!(public.coverage, SearchCoverageView::Complete);
        assert!(public.error.is_none());
    }

    #[test]
    fn public_search_is_always_public_while_ask_remains_opt_in() {
        assert!(effective_public_search(true, false, false));
        assert!(effective_public_search(true, false, true));
        assert!(!effective_public_search(false, true, false));
        assert!(effective_public_search(false, true, true));
    }

    #[test]
    fn onboarding_search_stays_local_without_mutating_the_ask_preference() {
        let ask_preference = true;

        assert!(!effective_public_search(false, false, ask_preference));
        assert!(ask_preference);
    }

    #[test]
    fn onboarding_intro_is_exactly_three_steps_before_the_knowledge_journey() {
        assert_eq!(onboarding_intro_step(OnboardingPage::Welcome), 1);
        assert_eq!(onboarding_intro_step(OnboardingPage::Model), 2);
        assert_eq!(onboarding_intro_step(OnboardingPage::Permissions), 3);
    }

    #[test]
    fn onboarding_footer_matches_the_reference_left_and_right_navigation() {
        let available = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 88.0));
        let (back, primary) = onboarding_footer_button_rects(available, egui::vec2(108.0, 42.0));

        assert_eq!(back.left(), available.left());
        assert_eq!(back.center().y, available.center().y);
        assert_eq!(primary.right(), available.right());
        assert_eq!(primary.center().y, available.center().y);
    }

    #[test]
    fn onboarding_machine_copy_reflects_the_hardware_report() {
        assert_eq!(
            onboarding_machine_status(None),
            "onboarding-machine-checking"
        );
        assert_eq!(
            onboarding_machine_status(Some((true, true))),
            "onboarding-machine-supported"
        );
        assert_eq!(
            onboarding_machine_status(Some((true, false))),
            "onboarding-machine-needs-attention"
        );
        assert_eq!(
            onboarding_machine_status(Some((false, false))),
            "onboarding-machine-unsupported"
        );
        assert_eq!(hardware_platform_name("macos", "aarch64"), "Apple Silicon");
        assert_eq!(hardware_platform_name("windows", "x86_64"), "Windows x64");
        assert_eq!(rounded_gib(12 * 1024 * 1024 * 1024), 12);

        let mut report = HardwareReport {
            os: "macos".to_owned(),
            architecture: "aarch64".to_owned(),
            total_memory_bytes: 16 * 1024 * 1024 * 1024,
            available_memory_bytes: 8 * 1024 * 1024 * 1024,
            available_disk_bytes: 12 * 1024 * 1024 * 1024,
            avx2: false,
            metal_available: true,
            supported_target: true,
            can_install: true,
            issues: Vec::new(),
        };
        assert_eq!(onboarding_machine_checks(&report), (true, true, true));
        report.total_memory_bytes = 4 * 1024 * 1024 * 1024;
        assert_eq!(onboarding_machine_checks(&report), (true, false, true));
        report.available_disk_bytes = 512 * 1024 * 1024;
        assert_eq!(onboarding_machine_checks(&report), (true, false, false));
    }

    #[test]
    fn ready_wiki_health_can_be_refreshed_manually() {
        assert!(wiki_health_can_refresh(&WikiHealthCheckState::Ready));
        assert!(!wiki_health_can_refresh(&WikiHealthCheckState::Loading));
        assert!(!wiki_health_can_refresh(&WikiHealthCheckState::Failed(
            "fixture".to_owned()
        )));
    }

    #[test]
    fn review_fields_stack_in_the_minimum_window_editor() {
        assert!(review_fields_stack(337.0));
        assert!(!review_fields_stack(520.0));
    }

    #[test]
    fn newly_focused_control_inside_onboarding_body_requests_scroll() {
        let body = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 350.0));
        let visible = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 250.0));
        let control = egui::Rect::from_min_size(egui::pos2(20.0, 320.0), egui::vec2(120.0, 36.0));

        assert!(focused_control_needs_scroll(true, body, visible, control));
    }

    #[test]
    fn held_focus_does_not_keep_overriding_manual_scroll() {
        let body = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 350.0));
        let visible = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 250.0));
        let control = egui::Rect::from_min_size(egui::pos2(20.0, 320.0), egui::vec2(120.0, 36.0));

        assert!(!focused_control_needs_scroll(false, body, visible, control));
    }

    #[test]
    fn newly_focused_visible_control_does_not_move_the_body() {
        let body = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 350.0));
        let visible = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 250.0));
        let control = egui::Rect::from_min_size(egui::pos2(20.0, 120.0), egui::vec2(120.0, 36.0));

        assert!(!focused_control_needs_scroll(true, body, visible, control));
    }

    #[test]
    fn horizontal_overflow_does_not_move_a_vertical_scroll_area() {
        let body = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 350.0));
        let visible = egui::Rect::from_min_size(egui::pos2(20.0, 0.0), egui::vec2(560.0, 250.0));
        let control = egui::Rect::from_min_size(egui::pos2(0.0, 120.0), egui::vec2(600.0, 36.0));

        assert!(!focused_control_needs_scroll(true, body, visible, control));
    }

    #[test]
    fn newly_focused_control_outside_onboarding_body_does_not_request_scroll() {
        let body = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 350.0));
        let visible = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 250.0));
        let footer = egui::Rect::from_min_size(egui::pos2(20.0, 380.0), egui::vec2(120.0, 36.0));

        assert!(!focused_control_needs_scroll(true, body, visible, footer));
    }

    #[test]
    fn first_run_hides_optional_system_failures_from_the_core_step() {
        assert!(!onboarding_error_is_relevant(
            OnboardingPage::Model,
            "the bundled macOS launch agent is unavailable"
        ));
        assert!(onboarding_error_is_relevant(
            OnboardingPage::Model,
            "the local model failed integrity verification"
        ));
        assert!(onboarding_error_is_relevant(
            OnboardingPage::Welcome,
            "No se pudieron iniciar los servicios privados"
        ));
    }

    #[test]
    fn conflict_and_legacy_exposure_offer_advanced_inbound_recovery() {
        for state in [
            FirewallDiagnosticState::Conflict,
            FirewallDiagnosticState::LegacyExposure,
        ] {
            assert!(firewall_state_offers_advanced_recovery(state));
        }
        assert!(!firewall_state_offers_advanced_recovery(
            FirewallDiagnosticState::ManagedPolicy
        ));
    }

    #[test]
    fn terminal_firewall_update_clears_presentation_after_a_later_request() {
        let completed = Uuid::new_v4();
        let later = Uuid::new_v4();

        assert!(firewall_operation_update_applies(
            Some(later),
            completed,
            None,
        ));
        assert!(!firewall_operation_update_applies(
            Some(later),
            completed,
            Some(FirewallOperationView::TakingLonger),
        ));
    }

    #[test]
    fn only_a_successfully_launched_installer_requests_desktop_exit() {
        let summary = UpdateSummary {
            version: "0.2.1".to_owned(),
            release_notes: None,
        };
        let installed = UpdaterWorkerView::Ready(UpdaterView {
            status: UpdaterStatus::Installed(summary.clone()),
            last_issue: None,
        });
        let ready = UpdaterWorkerView::Ready(UpdaterView {
            status: UpdaterStatus::ReadyToInstall(summary),
            last_issue: None,
        });

        assert!(updater_launched_installer(&installed));
        assert!(!updater_launched_installer(&ready));
        assert!(!updater_launched_installer(&UpdaterWorkerView::Disabled(
            crate::updater::UpdaterDisabledReason::NotConfigured,
        )));
    }

    #[test]
    fn an_already_downloaded_recommendation_is_not_presented_as_an_install() {
        let localization = Localization::new(UiLocale::Es).unwrap();
        assert_eq!(
            model_action_label(&localization, true, true),
            "Activar al reiniciar"
        );
        assert_eq!(
            model_action_label(&localization, true, false),
            "Activar al reiniciar"
        );
        assert_eq!(
            model_action_label(&localization, false, true),
            "Instalar actualización"
        );
        assert_eq!(
            model_action_label(&localization, false, false),
            "Descargar y verificar"
        );
    }

    #[test]
    fn technical_errors_are_reduced_to_human_categories() {
        let localization = Localization::new(UiLocale::Es).unwrap();
        let raw = "La colección 123e4567-e89b-12d3-a456-426614174000 falló en /private/path";
        assert_eq!(
            human_error_summary(&localization, raw),
            "Una carpeta de conocimiento necesita atención."
        );
        assert_eq!(sanitized_error_code(raw), "collection_unavailable");
        assert!(!sanitized_error_code(raw).contains("private"));
    }

    #[test]
    fn english_worker_notices_never_reuse_spanish_runtime_copy() {
        let localization = Localization::new(UiLocale::EnUs).unwrap();

        assert_eq!(
            localized_worker_notice(&localization, "La red local está lista"),
            "Local connection status updated."
        );
        assert_eq!(
            localized_worker_notice(&localization, "Modelos locales verificados y listos"),
            "Local AI status updated."
        );
    }

    #[test]
    fn legacy_firewall_action_uses_specific_human_copy() {
        let cases = [
            (
                UiLocale::Es,
                "Una o más reglas del firewall permiten demasiado acceso",
                "AirWiki mantendrá apagada la conexión con otros equipos hasta que revises en Windows las reglas que permiten más tráfico del necesario.",
            ),
            (
                UiLocale::EnUs,
                "One or more firewall rules allow too much access",
                "AirWiki will keep connections to other devices off until you review the Windows rules that allow more traffic than necessary.",
            ),
        ];

        for (locale, expected_title, expected_explanation) in cases {
            let localization = Localization::new(locale).unwrap();

            assert_eq!(
                (
                    primary_action_title(
                        &localization,
                        RecommendedAction::ReviewLegacyFirewallRules,
                    ),
                    primary_action_explanation(
                        &localization,
                        RecommendedAction::ReviewLegacyFirewallRules,
                    ),
                ),
                (expected_title.to_owned(), expected_explanation.to_owned())
            );
        }
    }

    #[test]
    fn search_coverage_uses_localized_human_messages() {
        for (locale, expected_offline) in [
            (UiLocale::Es, "equipos no respondieron"),
            (UiLocale::EnUs, "other devices did not respond"),
        ] {
            let localization = Localization::new(locale).unwrap();
            let offline = search_coverage_message(
                &localization,
                SearchCoverageView::OfflineDevices { count: 2 },
            )
            .unwrap();
            let disabled =
                search_coverage_message(&localization, SearchCoverageView::FederationDisabled)
                    .unwrap();
            let public_offline =
                search_coverage_message(&localization, SearchCoverageView::PublicNetworkOffline)
                    .unwrap();

            assert!(
                offline.contains('2') && offline.contains(expected_offline),
                "unexpected localized coverage message: {offline:?}"
            );
            assert!(!offline.contains("12D3Koo"));
            assert!(!disabled.contains("federation_disabled"));
            assert!(!public_offline.contains("public_network_offline"));
        }
        let localization = Localization::new(UiLocale::Es).unwrap();
        assert_eq!(
            search_coverage_message(&localization, SearchCoverageView::Complete),
            None
        );
    }

    #[test]
    fn firewall_confirmation_fails_closed_when_its_context_changes() {
        let eligible = ConnectivityPlatformSnapshot {
            system_permission: SystemPermissionState::NotApplicable,
            network_profile: NetworkProfileState::Private,
            firewall: FirewallDiagnosticState::RulesMissing,
            firewall_helper: FirewallHelperState::Verified,
        };
        assert!(firewall_configuration_is_current(
            LanPreference::Enabled,
            Some(eligible),
            false,
        ));

        for (preference, snapshot, busy) in [
            (LanPreference::Disabled, Some(eligible), false),
            (LanPreference::Enabled, Some(eligible), true),
            (
                LanPreference::Enabled,
                Some(ConnectivityPlatformSnapshot {
                    network_profile: NetworkProfileState::Public,
                    ..eligible
                }),
                false,
            ),
            (
                LanPreference::Enabled,
                Some(ConnectivityPlatformSnapshot {
                    firewall: FirewallDiagnosticState::Ready,
                    ..eligible
                }),
                false,
            ),
            (
                LanPreference::Enabled,
                Some(ConnectivityPlatformSnapshot {
                    firewall_helper: FirewallHelperState::Untrusted,
                    ..eligible
                }),
                false,
            ),
            (LanPreference::Enabled, None, false),
        ] {
            assert!(!firewall_configuration_is_current(
                preference, snapshot, busy
            ));
        }
    }

    #[test]
    fn enabling_external_ai_requires_confirmation_but_disabling_is_immediate() {
        assert_eq!(
            classify_external_ai_policy_change(false, true),
            ExternalAiPolicyChange::ConfirmEnable
        );
        assert_eq!(
            classify_external_ai_policy_change(true, false),
            ExternalAiPolicyChange::ApplyDisable
        );
        assert_eq!(
            classify_external_ai_policy_change(false, false),
            ExternalAiPolicyChange::None
        );
    }

    #[test]
    fn repeated_notices_are_collapsed_without_merging_different_severities() {
        let mut notices = VecDeque::from([
            (true, "same".to_owned()),
            (true, "same".to_owned()),
            (false, "same".to_owned()),
            (true, "different".to_owned()),
        ]);

        deduplicate_notices(&mut notices);

        assert_eq!(
            notices,
            VecDeque::from([
                (true, "same".to_owned()),
                (false, "same".to_owned()),
                (true, "different".to_owned()),
            ])
        );
    }

    #[test]
    fn search_responses_apply_only_to_the_active_request_surface() {
        let request_id = Uuid::new_v4();
        let active = Some(ActiveSearch {
            request_id,
            surface: SearchSurface::Public,
        });

        assert_eq!(
            search_response_surface(active, request_id),
            Some(SearchSurface::Public)
        );
        assert_eq!(search_response_surface(active, Uuid::new_v4()), None);
        assert_eq!(search_response_surface(None, request_id), None);
    }

    #[test]
    fn local_ask_completion_does_not_overwrite_the_last_public_route() {
        let mut ask = SearchViewState::new();
        let mut public = SearchViewState::new();
        public.complete(
            Vec::new(),
            SearchCoverageView::Complete,
            PublicRouteKind::Relay,
        );

        ask.complete(
            Vec::new(),
            SearchCoverageView::Complete,
            PublicRouteKind::Offline,
        );

        assert_eq!(public.route_kind, PublicRouteKind::Relay);
    }

    #[test]
    fn public_ask_completion_keeps_each_surface_route_independent() {
        let mut ask = SearchViewState::new();
        let mut public = SearchViewState::new();
        public.complete(
            Vec::new(),
            SearchCoverageView::Complete,
            PublicRouteKind::Relay,
        );

        ask.complete(
            Vec::new(),
            SearchCoverageView::Complete,
            PublicRouteKind::Direct,
        );

        assert_eq!(
            (ask.route_kind, public.route_kind),
            (PublicRouteKind::Direct, PublicRouteKind::Relay)
        );
    }

    #[test]
    fn blocking_a_publisher_removes_only_publicly_sourced_hits() {
        let blocked_publisher = "blocked-publisher";
        let retained_publisher = "retained-publisher";
        let mut ask = SearchViewState::new();
        let mut public = SearchViewState::new();
        ask.submitted_public_network = true;
        ask.hits = vec![
            synthetic_search_hit(blocked_publisher),
            synthetic_search_hit(retained_publisher),
        ];
        public.hits = vec![
            synthetic_search_hit(blocked_publisher),
            synthetic_search_hit(retained_publisher),
        ];

        remove_blocked_publisher_hits(&mut ask, &mut public, blocked_publisher);

        assert_eq!(ask.hits.len(), 1);
        assert_eq!(ask.hits[0].node_id, retained_publisher);
        assert_eq!(public.hits.len(), 1);
        assert_eq!(public.hits[0].node_id, retained_publisher);
    }

    #[test]
    fn blocking_a_publisher_preserves_local_only_ask_feedback() {
        let blocked_publisher = "blocked-publisher";
        let mut ask = SearchViewState::new();
        let mut public = SearchViewState::new();
        ask.hits.push(synthetic_search_hit(blocked_publisher));
        public.hits.push(synthetic_search_hit(blocked_publisher));

        remove_blocked_publisher_hits(&mut ask, &mut public, blocked_publisher);

        assert_eq!(ask.hits.len(), 1);
        assert!(public.hits.is_empty());
    }

    #[test]
    fn only_an_exact_local_result_with_a_current_collection_can_open_the_wiki() {
        assert_eq!(
            classify_search_result("local", "local", true, Some("ignored")),
            SearchResultAvailability::LocalAvailable
        );
        assert_eq!(
            classify_search_result("local", "local", false, None),
            SearchResultAvailability::LocalUnavailable
        );
        assert_eq!(
            classify_search_result("local", "remote", true, Some("Office PC")),
            SearchResultAvailability::Remote {
                device_name: Some("Office PC".to_owned())
            }
        );
        assert_eq!(
            classify_search_result("local", "remote", true, Some("   ")),
            SearchResultAvailability::Remote { device_name: None }
        );
    }

    #[test]
    fn remote_search_origin_uses_a_human_name_without_exposing_peer_identity() {
        let localization = Localization::new(UiLocale::EnUs).unwrap();
        let known = SearchResultAvailability::Remote {
            device_name: Some("Office PC".to_owned()),
        };
        let unknown = SearchResultAvailability::Remote { device_name: None };

        let known_label = search_result_origin_label(&localization, &known);
        assert!(known_label.starts_with("From "));
        assert!(known_label.contains("Office PC"));
        assert!(!known_label.contains("12D3Koo"));
        assert_eq!(
            search_result_origin_label(&localization, &unknown),
            "From another device"
        );
        assert!(!search_result_origin_label(&localization, &unknown).contains("12D3Koo"));
    }

    #[test]
    fn wiki_health_rejects_older_and_duplicate_generations() {
        assert!(wiki_health_result_applies(4, 5));
        assert!(!wiki_health_result_applies(4, 4));
        assert!(!wiki_health_result_applies(4, 3));
    }

    #[test]
    fn wiki_health_loading_and_failure_feed_readiness() {
        let summary = WikiHealthSummaryView::default();

        assert_eq!(
            wiki_health_readiness_inputs(&WikiHealthCheckState::Loading, &summary),
            (true, 0)
        );
        assert_eq!(
            wiki_health_readiness_inputs(
                &WikiHealthCheckState::Failed("unavailable".to_owned()),
                &summary,
            ),
            (false, 1)
        );
    }

    #[test]
    fn wiki_health_age_uses_completed_snapshot_time() {
        assert_eq!(
            elapsed_minutes(
                SystemTime::UNIX_EPOCH,
                SystemTime::UNIX_EPOCH + Duration::from_secs(125),
            ),
            2
        );
    }

    #[test]
    fn wiki_health_age_tolerates_a_future_system_clock() {
        assert_eq!(
            elapsed_minutes(
                SystemTime::UNIX_EPOCH + Duration::from_secs(1),
                SystemTime::UNIX_EPOCH,
            ),
            0
        );
    }

    #[test]
    fn connectivity_is_active_only_when_platform_and_runtime_are_ready() {
        let ready = ConnectivityPlatformSnapshot {
            system_permission: SystemPermissionState::NotApplicable,
            network_profile: NetworkProfileState::Private,
            firewall: FirewallDiagnosticState::Ready,
            firewall_helper: FirewallHelperState::Verified,
        };
        assert!(connectivity_runtime_is_active(
            Some(ready),
            LanListenerView::Listening,
            LanDiscoveryView::Active,
        ));

        for firewall in [
            FirewallDiagnosticState::Unknown,
            FirewallDiagnosticState::FirewallDisabled,
            FirewallDiagnosticState::BlockAllInbound,
            FirewallDiagnosticState::RulesMissing,
            FirewallDiagnosticState::Conflict,
            FirewallDiagnosticState::LegacyExposure,
            FirewallDiagnosticState::ManagedPolicy,
            FirewallDiagnosticState::Unsupported,
            FirewallDiagnosticState::Error,
        ] {
            assert!(!connectivity_runtime_is_active(
                Some(ConnectivityPlatformSnapshot { firewall, ..ready }),
                LanListenerView::Listening,
                LanDiscoveryView::Active,
            ));
        }
        assert!(!connectivity_runtime_is_active(
            Some(ConnectivityPlatformSnapshot {
                network_profile: NetworkProfileState::Public,
                ..ready
            }),
            LanListenerView::Listening,
            LanDiscoveryView::Active,
        ));
        assert!(!connectivity_runtime_is_active(
            Some(ready),
            LanListenerView::Stopped,
            LanDiscoveryView::Active,
        ));
    }

    #[test]
    fn manual_fallback_accepts_ipv4_and_rejects_ipv6() {
        assert!(parse_manual_ipv4_address("/ip4/192.168.1.25/tcp/61743").is_some());
        assert!(parse_manual_ipv4_address("/ip6/fd42::25/tcp/61743").is_none());
    }

    #[test]
    fn idle_connection_copy_never_promises_reconnect_for_a_blocked_peer() {
        assert_eq!(
            peer_activity_message_id(PeerTrustState::Blocked, PeerActivityState::NotObserved),
            "peer-activity-unavailable"
        );
        assert_eq!(
            peer_activity_message_id(PeerTrustState::Trusted, PeerActivityState::NotObserved),
            "peer-activity-not-observed"
        );
    }

    #[test]
    fn pairing_activity_presents_sas_controls() {
        assert!(should_present_pairing_controls(PeerActivityState::Pairing));
        assert!(!should_present_pairing_controls(
            PeerActivityState::Connected,
        ));
    }

    #[test]
    fn review_source_issue_shows_unknown_cause_when_not_classified() {
        let localization = Localization::new(UiLocale::EnUs).unwrap();
        let issue = SourceIssueView {
            collection_id: Uuid::nil(),
            source_name: "unmapped.txt".to_owned(),
            collection_name: "Collection".to_owned(),
            code: SourceIssueCode::InvalidPdf,
            reason: None,
        };

        assert_eq!(
            super::source_issue_cause_message(&localization, &issue, issue.code).unwrap(),
            localization.text("review-issue-cause-unknown")
        );
    }

    #[test]
    fn review_source_issue_shows_unmapped_reason_when_present() {
        let localization = Localization::new(UiLocale::EnUs).unwrap();
        let issue = SourceIssueView {
            collection_id: Uuid::nil(),
            source_name: "mystery.md".to_owned(),
            collection_name: "Collection".to_owned(),
            code: SourceIssueCode::InvalidPdf,
            reason: Some("custom-engine-fault".to_owned()),
        };
        let mut arguments = FluentArgs::new();
        arguments.set("reason", "custom-engine-fault");

        assert_eq!(
            super::source_issue_cause_message(&localization, &issue, issue.code).unwrap(),
            localization.text_with("review-issue-cause-unmapped", Some(&arguments))
        );
    }

    #[test]
    fn review_source_issue_shows_processing_failure_for_superseded_and_failure() {
        let localization = Localization::new(UiLocale::Es).unwrap();
        let issue = SourceIssueView {
            collection_id: Uuid::nil(),
            source_name: "stale.txt".to_owned(),
            collection_name: "Collection".to_owned(),
            code: SourceIssueCode::Superseded,
            reason: None,
        };

        assert_eq!(
            super::source_issue_cause_message(&localization, &issue, issue.code).unwrap(),
            localization.text("review-issue-cause-processing-failed")
        );
    }

    #[test]
    fn source_issue_raw_reason_preview_truncates_long_reasons() {
        let preview = super::source_issue_raw_reason_preview(
            Some("very long reason with line\nbreaks and spaces"),
            16,
        );
        assert_eq!(preview, Some("very long reason…".to_owned()));
    }

    #[test]
    fn maintenance_issue_summary_localizes_known_persisted_code() {
        let localization = Localization::new(UiLocale::Es).unwrap();

        assert_eq!(
            super::maintenance_issue_summary(
                &localization,
                Some("collection_scan_partial"),
                Some("One or more files could not be processed."),
            ),
            Some(localization.text("collections-maintenance-partial"))
        );
    }

    #[test]
    fn maintenance_issue_summary_preserves_safe_fallback_for_future_code() {
        let localization = Localization::new(UiLocale::EnUs).unwrap();

        assert_eq!(
            super::maintenance_issue_summary(
                &localization,
                Some("future_issue"),
                Some("Future safe summary"),
            ),
            Some("Future safe summary".to_owned())
        );
    }
}
