use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    time::{Duration, SystemTime},
};

use airwiki_inference::{
    E5_FILES, HardwareReport, InstallEvent, MMARCO_COMMON_FILES, ModelProfile,
};
use airwiki_network::ManualLanAddress;
use airwiki_types::{
    ConceptType, DEFAULT_TOP_K, EnrichmentDraft, SearchHit, SearchPurpose, SuggestedEntity,
    SuggestedLink,
};
use eframe::egui::{self, Color32, RichText};
use egui_extras::{Size, StripBuilder};
use fluent_bundle::FluentArgs;
use uuid::Uuid;

mod first_knowledge;
mod integrations;
mod knowledge;
mod review;

use self::first_knowledge::JourneyStepState;
use self::integrations::{ChatIntegrationsUi, IntegrationsUiAction};
use self::knowledge::{KnowledgeAction, KnowledgeUi, SearchEvidenceTarget};
use self::review::{
    REVIEW_ACTION_BAR_HEIGHT, REVIEW_PANEL_GAP, REVIEW_QUEUE_WIDTH, ReviewEvidenceAction,
    ReviewEvidencePanelIntent, ReviewEvidenceUi, ReviewLayoutMode, review_comparison_widths,
    review_layout_mode, show_review_evidence_panel,
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
    layout::ResponsiveLayout,
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
        ModelStateView, PERIODIC_RECONCILE_INTERVAL, PeerActivityState, PeerTrustState, PeerView,
        ReviewItemView, SearchCoverageView, SourceIssueView, UpdaterWorkerView,
        WikiHealthSummaryView, WorkerCommand, WorkerEvent, WorkerHandle,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Setup,
    Models,
    Collections,
    Review,
    Knowledge,
    Search,
    Integrations,
    Nodes,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardingPage {
    Welcome,
    Model,
    Collection,
    Processing,
    Review,
    Search,
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
    search_question: String,
    search_top_k: u8,
    search_hits: Vec<SearchHit>,
    search_coverage: SearchCoverageView,
    search_request_id: Option<Uuid>,
    search_completed: bool,
    search_error: Option<String>,
    new_collection_name: String,
    new_collection_folder: Option<PathBuf>,
    manual_multiaddress: String,
    notices: VecDeque<(bool, String)>,
    selected_review: Option<Uuid>,
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
            search_question: String::new(),
            search_top_k: DEFAULT_TOP_K,
            search_hits: Vec::new(),
            search_coverage: SearchCoverageView::Complete,
            search_request_id: None,
            search_completed: false,
            search_error: None,
            new_collection_name: String::new(),
            new_collection_folder: None,
            manual_multiaddress: String::new(),
            notices: VecDeque::new(),
            selected_review: None,
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
                } => {
                    self.node_id = node_id;
                    self.mcp_url = mcp_url;
                    self.collections = collections;
                    self.selected_review =
                        selected_review_after_refresh(self.selected_review, &reviews);
                    self.reviews = reviews;
                    self.source_issues = source_issues;
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
                WorkerEvent::SearchFinished { request_id, result } => {
                    if !search_result_applies(self.search_request_id, request_id) {
                        continue;
                    }
                    self.search_request_id = None;
                    match result {
                        Ok((hits, coverage)) => {
                            self.search_completed = true;
                            self.search_hits = hits;
                            self.search_coverage = coverage;
                            self.search_error = None;
                        }
                        Err(error) => {
                            self.search_completed = false;
                            self.search_error = Some(sanitized_error_code(&error).to_owned());
                        }
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
        let review = format!(
            "{}  ({})",
            self.localization.text("nav-review"),
            self.reviews.len().saturating_add(self.source_issues.len())
        );
        let wiki = self.localization.text("nav-wiki");
        let search = self.localization.text("nav-search");
        let integrations = self.localization.text("nav-integrations");
        let devices = self.localization.text("nav-devices");
        let settings = self.localization.text("nav-settings");
        let model_status = if self.models_ready {
            format!("● {}", self.localization.text("models-ready"))
        } else {
            format!("○ {}", self.localization.text("models-pending"))
        };
        egui::Panel::left("navigation")
            .exact_size(205.0)
            .show(root, |ui| {
                ui.add_space(18.0);
                ui.heading(RichText::new("AirWiki").size(22.0));
                ui.add_space(22.0);
                nav(ui, &mut self.screen, Screen::Setup, &home);
                nav(ui, &mut self.screen, Screen::Collections, &collections);
                nav(ui, &mut self.screen, Screen::Review, &review);
                nav(ui, &mut self.screen, Screen::Knowledge, &wiki);
                nav(ui, &mut self.screen, Screen::Search, &search);
                nav(ui, &mut self.screen, Screen::Integrations, &integrations);
                nav(ui, &mut self.screen, Screen::Nodes, &devices);
                nav(ui, &mut self.screen, Screen::Settings, &settings);
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.label(model_status);
                });
            });
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
                    egui::Frame::group(ui.style()).show(ui, |ui| {
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
                            ui.colored_label(crate::theme::ERROR_CORAL, issue);
                        }
                    });
                } else {
                    ui.spinner();
                    ui.label(self.localization.text("models-diagnosing"));
                }
                ui.add_space(14.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
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
                                crate::theme::WARNING_AMBER,
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
                            ui.colored_label(crate::theme::ERROR_CORAL, issue);
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
                                    crate::theme::VERIFIED_GREEN,
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
                        ui.colored_label(crate::theme::VERIFIED_GREEN, message);
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(self.localization.text("models-multimodal-future"))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                });
            });
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        let layout = ResponsiveLayout::from_available(ui.available_size());
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

        first_knowledge::show_journey_header(
            ui,
            &self.localization,
            visible_journey_states(journey),
            layout.density,
        );
        ui.add_space(if layout.is_compact() { 10.0 } else { 18.0 });
        first_knowledge::work_surface(ui, layout.density, |ui| {
            if self.home_wiki_incident(ui) {
                ui.add_space(10.0);
            }
            ui.label(
                RichText::new(self.localization.text("home-next-step"))
                    .small()
                    .strong()
                    .color(ui.visuals().weak_text_color()),
            );
            match journey.cta {
                Some(FirstKnowledgeCta::Recommended(action)) => {
                    ui.heading(primary_action_title(&self.localization, action));
                    ui.label(primary_action_explanation(&self.localization, action));
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
                    ui.heading(self.localization.text("onboarding-search-title"));
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
                    let (title, body) =
                        journey_stage_copy(&self.localization, journey.current_stage);
                    ui.heading(title);
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
                                    first_knowledge_readiness_status(journey.current_state),
                                )
                                .0,
                            );
                        });
                    }
                }
            }
        });

        ui.add_space(if layout.is_compact() { 8.0 } else { 16.0 });
        first_knowledge::privacy_note(ui, &self.localization);
        ui.add_space(if layout.is_compact() { 8.0 } else { 16.0 });
        ui.collapsing(self.localization.text("home-optional-title"), |ui| {
            ui.label(self.localization.text("home-optional-body"));
            ui.add_space(8.0);
            egui::Grid::new("optional_readiness_components")
                .num_columns(2)
                .spacing([24.0, 8.0])
                .show(ui, |ui| {
                    for component in readiness.components.iter().filter(|component| {
                        matches!(
                            component.component,
                            ReadinessComponent::Lan
                                | ReadinessComponent::Chat
                                | ReadinessComponent::Background
                                | ReadinessComponent::Updates
                        )
                    }) {
                        ui.label(readiness_component_label(
                            &self.localization,
                            component.component,
                        ));
                        let (label, color) =
                            readiness_status_presentation(&self.localization, component.status);
                        ui.colored_label(color, label);
                        ui.end_row();
                    }
                });
        });

        ui.horizontal(|ui| {
            let checked = match (&self.wiki_health_check, readiness.last_checked_at) {
                (WikiHealthCheckState::Loading, _) => self.localization.text("home-wiki-checking"),
                (WikiHealthCheckState::Failed(_), _) => self.localization.text("home-wiki-failed"),
                (WikiHealthCheckState::Ready, Some(checked_at)) => {
                    let minutes = elapsed_minutes(checked_at, SystemTime::now());
                    let when = if minutes == 0 {
                        self.localization.text("home-checked-now")
                    } else {
                        let mut arguments = FluentArgs::new();
                        arguments.set("minutes", minutes);
                        self.localization
                            .text_with("home-checked-minutes", Some(&arguments))
                    };
                    let mut arguments = FluentArgs::new();
                    arguments.set("when", when);
                    self.localization
                        .text_with("home-last-checked", Some(&arguments))
                }
                (WikiHealthCheckState::Ready, None) => {
                    self.localization.text("home-wiki-not-checked")
                }
            };
            ui.label(
                RichText::new(checked)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            let action = if matches!(self.wiki_health_check, WikiHealthCheckState::Failed(_)) {
                "action-retry"
            } else {
                "action-refresh"
            };
            if ui
                .add_enabled(
                    self.wiki_health_request_id.is_none()
                        && !matches!(self.wiki_health_check, WikiHealthCheckState::Loading),
                    egui::Button::new(self.localization.text(action)),
                )
                .clicked()
            {
                let request_id = Uuid::new_v4();
                self.wiki_health_request_id = Some(request_id);
                self.wiki_health_check = WikiHealthCheckState::Loading;
                self.wiki_health_error_dismissed = false;
                self.worker
                    .send(WorkerCommand::RefreshWikiHealth { request_id });
            }
        });
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
        page_title(
            ui,
            &self.localization.text("collections-title"),
            &self.localization.text("collections-subtitle"),
        );
        let mut monitoring_arguments = FluentArgs::new();
        monitoring_arguments.set("minutes", PERIODIC_RECONCILE_INTERVAL.as_secs() / 60);
        ui.label(
            RichText::new(
                self.localization
                    .text_with("collections-monitoring", Some(&monitoring_arguments)),
            )
            .small()
            .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(8.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.heading(self.localization.text("collections-new"));
            ui.horizontal(|ui| {
                let name_label = ui.label(self.localization.text("collections-name"));
                ui.text_edit_singleline(&mut self.new_collection_name)
                    .labelled_by(name_label.id);
                if ui
                    .button(self.localization.text("collections-choose-folder"))
                    .clicked()
                {
                    self.new_collection_folder = rfd::FileDialog::new().pick_folder();
                }
            });
            if let Some(path) = &self.new_collection_folder {
                wrap_monospace(ui, path.display().to_string());
            }
            let enabled =
                !self.new_collection_name.trim().is_empty() && self.new_collection_folder.is_some();
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(self.localization.text("collections-create-scan")),
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
        ui.add_space(12.0);
        if self.collections.is_empty() {
            empty_state(
                ui,
                &self.localization.text("collections-empty-title"),
                &self.localization.text("collections-empty-body"),
            );
        }
        let technical_details = self.localization.text("action-details");
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
        let list_height = ui.available_height().max(0.0);
        egui::ScrollArea::vertical()
            .id_salt("collections_list")
            .max_height(list_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for collection in &mut self.collections {
                    let collection_issues = self
                        .source_issues
                        .iter()
                        .filter(|issue| issue.collection_id == collection.id)
                        .collect::<Vec<_>>();
                    let scan_state = self.collection_scans.get(&collection.id).copied();
                    let mut counts_arguments = FluentArgs::new();
                    counts_arguments.set("documents", collection.document_count);
                    counts_arguments.set("published", collection.published_count);
                    let counts = self
                        .localization
                        .text_with("collections-counts", Some(&counts_arguments));
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.heading(&collection.name);
                                ui.label(&linked);
                                ui.collapsing(&technical_details, |ui| {
                                    wrap_monospace(ui, collection.folder.display().to_string());
                                });
                                ui.label(&counts);
                                if !collection_issues.is_empty() {
                                    let mut issues_arguments = FluentArgs::new();
                                    issues_arguments.set("count", collection_issues.len() as i64);
                                    ui.label(
                                        RichText::new(self.localization.text_with(
                                            "review-issues-group",
                                            Some(&issues_arguments),
                                        ))
                                        .color(crate::theme::WARNING_AMBER),
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
                                    );
                                    ui.colored_label(color, label);
                                    if let Some(finished) = maintenance.last_finished_at {
                                        let mut arguments = FluentArgs::new();
                                        arguments.set(
                                            "time",
                                            finished.format("%Y-%m-%d %H:%M").to_string(),
                                        );
                                        ui.label(
                                            RichText::new(self.localization.text_with(
                                                "collections-last-scan",
                                                Some(&arguments),
                                            ))
                                            .small()
                                            .color(ui.visuals().weak_text_color()),
                                        );
                                    }
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
                                if ui
                                    .add_enabled(scan_state.is_none(), egui::Button::new(&retry))
                                    .clicked()
                                {
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
                        ui.separator();
                        let external_ai_before = collection.allow_external_ai;
                        let peer_changed = ui
                            .checkbox(&mut collection.peer_shareable, &share_peers)
                            .changed();
                        let external_ai_changed = ui
                            .checkbox(&mut collection.allow_external_ai, &allow_chat)
                            .changed();
                        let external_ai_change = classify_external_ai_policy_change(
                            external_ai_before,
                            collection.allow_external_ai,
                        );
                        if external_ai_change == ExternalAiPolicyChange::ConfirmEnable {
                            collection.allow_external_ai = false;
                            requested_external_ai_confirmation = Some(collection.id);
                        }
                        collection.local_only =
                            !collection.peer_shareable && !collection.allow_external_ai;
                        if collection.local_only {
                            ui.label(
                                RichText::new(&local_only)
                                    .small()
                                    .color(crate::theme::VERIFIED_GREEN),
                            );
                        }
                        if collection.allow_external_ai {
                            ui.colored_label(crate::theme::WARNING_AMBER, &cloud_warning);
                        }
                        let external_ai_applies = external_ai_changed
                            && external_ai_change == ExternalAiPolicyChange::ApplyDisable;
                        if peer_changed || external_ai_applies {
                            self.worker.send(WorkerCommand::UpdateCollectionPolicy {
                                collection_id: collection.id,
                                local_only: collection.local_only,
                                peer_shareable: collection.peer_shareable,
                                allow_external_ai: collection.allow_external_ai,
                            });
                        }
                    });
                    ui.add_space(8.0);
                }
            });
        if let Some(collection_id) = requested_external_ai_confirmation {
            self.external_ai_confirmation = Some(collection_id);
        }
        self.external_ai_confirmation_window(ui.ctx());
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
                ui.colored_label(crate::theme::WARNING_AMBER, warning);
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
        });
    }

    fn review(&mut self, ui: &mut egui::Ui) {
        page_title(
            ui,
            &self.localization.text("review-title"),
            &self.localization.text("review-subtitle"),
        );
        self.review_content(ui);
    }

    fn review_content(&mut self, ui: &mut egui::Ui) {
        if self.reviews.is_empty() && self.source_issues.is_empty() {
            self.review_evidence.sync_selection(None, false);
            empty_state(
                ui,
                &self.localization.text("review-empty-title"),
                &self.localization.text("review-empty-body"),
            );
            return;
        }
        let issues = self.source_issues.clone();
        let mut requested_rescan = None;
        match review_layout_mode(ui.available_width()) {
            ReviewLayoutMode::CompactCompare => {
                self.compact_review_selector(ui, &issues, &mut requested_rescan);
                ui.separator();
                self.review_comparison(ui, &issues);
            }
            ReviewLayoutMode::QueueCompare => {
                StripBuilder::new(ui)
                    .size(Size::exact(REVIEW_QUEUE_WIDTH))
                    .size(Size::exact(REVIEW_PANEL_GAP))
                    .size(Size::remainder())
                    .clip(true)
                    .horizontal(|mut strip| {
                        strip.cell(|ui| {
                            self.review_queue(ui, &issues, &mut requested_rescan);
                        });
                        strip.cell(|_| {});
                        strip.cell(|ui| self.review_comparison(ui, &issues));
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
            ui.label(RichText::new(self.localization.text("review-document-selector")).strong());
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
        let queue_height = ui.available_height().max(0.0);
        egui::ScrollArea::vertical()
            .id_salt("review_queue")
            .max_height(queue_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if !self.reviews.is_empty() {
                    let mut arguments = FluentArgs::new();
                    arguments.set("count", self.reviews.len() as i64);
                    ui.label(
                        RichText::new(
                            self.localization
                                .text_with("review-ready-group", Some(&arguments)),
                        )
                        .strong(),
                    );
                    ui.add_space(4.0);
                    for item in &self.reviews {
                        let selected = self.selected_review == Some(item.concept_id);
                        if ui
                            .selectable_label(
                                selected,
                                format!("{}\n{}", item.source_name, item.collection_name),
                            )
                            .clicked()
                        {
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
                    ui.label(
                        RichText::new(
                            self.localization
                                .text_with("review-issues-group", Some(&arguments)),
                        )
                        .strong()
                        .color(crate::theme::WARNING_AMBER),
                    );
                    ui.add_space(4.0);
                    for issue in issues {
                        let scanning = self.collection_scans.contains_key(&issue.collection_id);
                        if show_review_issue(ui, &self.localization, issue, scanning) {
                            *requested_rescan = Some(issue.collection_id);
                        }
                        ui.add_space(6.0);
                    }
                }
            });
    }

    fn review_comparison(&mut self, ui: &mut egui::Ui, issues: &[SourceIssueView]) {
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
        let page = self.review_evidence.page_for(concept_id, source_revision);
        let mut evidence_intent = None;
        let mut approve = false;
        let mut reject = false;
        let mut reanalyze = false;
        let (evidence_width, _) = review_comparison_widths(ui.available_width());
        StripBuilder::new(ui)
            .size(Size::remainder())
            .size(Size::exact(REVIEW_ACTION_BAR_HEIGHT))
            .clip(true)
            .vertical(|mut strip| {
                strip.cell(|ui| {
                    StripBuilder::new(ui)
                        .size(Size::exact(evidence_width))
                        .size(Size::exact(REVIEW_PANEL_GAP))
                        .size(Size::remainder())
                        .clip(true)
                        .horizontal(|mut strip| {
                            strip.cell(|ui| {
                                evidence_intent = show_review_evidence_panel(
                                    ui,
                                    &self.localization,
                                    concept_id,
                                    source_revision,
                                    page,
                                    error,
                                    loading,
                                );
                            });
                            strip.cell(|_| {});
                            strip.cell(|ui| {
                                let editor_height = ui.available_height().max(0.0);
                                egui::ScrollArea::vertical()
                                    .id_salt(("review_editor", concept_id, source_revision))
                                    .max_height(editor_height)
                                    .auto_shrink([false; 2])
                                    .show(ui, |ui| {
                                        edit_draft(
                                            ui,
                                            &self.localization,
                                            &mut self.reviews[selected_index].draft,
                                        );
                                    });
                            });
                        });
                });
                strip.cell(|ui| {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        approve = ui
                            .add_enabled(
                                approval_ready,
                                first_knowledge::primary_button(
                                    self.localization.text("review-approve"),
                                ),
                            )
                            .clicked();
                        reject = ui
                            .add_enabled(
                                !is_reanalyzing,
                                egui::Button::new(self.localization.text("review-reject")),
                            )
                            .clicked();
                        reanalyze = ui
                            .add_enabled(
                                self.models_ready && !is_reanalyzing,
                                egui::Button::new(self.localization.text("review-reanalyze")),
                            )
                            .on_hover_text(self.localization.text("review-reanalyze-help"))
                            .clicked();
                    });
                    if is_reanalyzing {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(self.localization.text("review-analyzing"));
                        });
                    } else if !approval_ready {
                        ui.label(
                            RichText::new(
                                self.localization.text("review-evidence-approval-blocked"),
                            )
                            .small()
                            .color(ui.visuals().weak_text_color()),
                        );
                    } else if !self.models_ready {
                        ui.label(
                            RichText::new(self.localization.text("review-model-required"))
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                });
            });

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
        page_title(
            ui,
            &self.localization.text("search-title"),
            &self.localization.text("search-subtitle"),
        );
        self.search_form(ui, true);
        if let Some(target) = self.search_feedback(ui, true) {
            self.open_search_evidence(target);
        }
    }

    fn search_form(&mut self, ui: &mut egui::Ui, show_top_k: bool) {
        let layout = ResponsiveLayout::from_available(ui.available_size());
        let search_running = self.search_request_id.is_some();
        let enabled = !self.search_question.trim().is_empty() && !search_running;
        let mut submit_clicked = false;
        let response = if layout.is_narrow() {
            let question_label = ui.label(self.localization.text("search-question"));
            let response = ui
                .add_enabled_ui(!search_running, |ui| {
                    ui.add_sized(
                        [ui.available_width(), 36.0],
                        egui::TextEdit::singleline(&mut self.search_question)
                            .hint_text(self.localization.text("search-placeholder")),
                    )
                })
                .inner
                .labelled_by(question_label.id);
            ui.horizontal(|ui| {
                if show_top_k {
                    ui.add_enabled(
                        !search_running,
                        egui::DragValue::new(&mut self.search_top_k)
                            .range(1..=10)
                            .prefix("Top "),
                    );
                }
                submit_clicked = ui
                    .add_enabled(
                        enabled,
                        first_knowledge::primary_button(self.localization.text("search-action")),
                    )
                    .clicked();
            });
            response
        } else {
            ui.horizontal(|ui| {
                let question_label = ui.label(self.localization.text("search-question"));
                let reserved_width = if show_top_k { 190.0 } else { 100.0 };
                let field_width = (ui.available_width() - reserved_width).clamp(220.0, 520.0);
                let response = ui
                    .add_enabled_ui(!search_running, |ui| {
                        ui.add_sized(
                            [field_width, 36.0],
                            egui::TextEdit::singleline(&mut self.search_question)
                                .hint_text(self.localization.text("search-placeholder")),
                        )
                    })
                    .inner
                    .labelled_by(question_label.id);
                if show_top_k {
                    ui.add_enabled(
                        !search_running,
                        egui::DragValue::new(&mut self.search_top_k)
                            .range(1..=10)
                            .prefix("Top "),
                    );
                }
                submit_clicked = ui
                    .add_enabled(
                        enabled,
                        first_knowledge::primary_button(self.localization.text("search-action")),
                    )
                    .clicked();
                response
            })
            .inner
        };
        let submit = submit_clicked
            || (enabled
                && response.lost_focus()
                && ui.input(|input| input.key_pressed(egui::Key::Enter)));
        if response.changed() {
            self.search_completed = false;
            self.search_hits.clear();
            self.search_error = None;
        }
        if submit {
            let request_id = Uuid::new_v4();
            self.search_request_id = Some(request_id);
            self.search_completed = false;
            self.search_error = None;
            self.search_hits.clear();
            self.search_coverage = SearchCoverageView::Complete;
            self.worker.send(WorkerCommand::Search {
                request_id,
                question: self.search_question.trim().to_owned(),
                top_k: self.search_top_k,
                purpose: SearchPurpose::LocalAssistant,
            });
        }
    }

    fn search_feedback(
        &mut self,
        ui: &mut egui::Ui,
        show_empty_state: bool,
    ) -> Option<SearchEvidenceTarget> {
        let mut selected_evidence = None;
        if self.search_request_id.is_some() {
            ui.spinner();
            ui.label(self.localization.text("search-running"));
        }
        self.search_error_feedback(ui);
        if let Some(message) = search_coverage_message(&self.localization, self.search_coverage) {
            ui.colored_label(crate::theme::WARNING_AMBER, message);
        }
        if show_empty_state && self.search_completed && self.search_hits.is_empty() {
            empty_state(
                ui,
                &self.localization.text("search-empty-title"),
                &self.localization.text("search-empty-body"),
            );
        }
        let results_height = ui.available_height().max(0.0);
        egui::ScrollArea::vertical()
            .id_salt("search_results")
            .max_height(results_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for hit in &self.search_hits {
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
                    ui.add_space(8.0);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.heading(format!("{}. {}", hit.rank, hit.title));
                        ui.label(RichText::new(&hit.heading_or_page).strong());
                        ui.label(&hit.snippet);
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
                                    crate::theme::WARNING_AMBER,
                                    self.localization.text("search-local-unavailable"),
                                );
                            }
                            SearchResultAvailability::Remote { .. } => {
                                ui.label(
                                    RichText::new(&origin)
                                        .small()
                                        .color(ui.visuals().weak_text_color()),
                                );
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
            });
        selected_evidence
    }

    fn open_search_evidence(&mut self, target: SearchEvidenceTarget) {
        let scan_active = self.collection_scans.contains_key(&target.collection_id());
        let action = self.knowledge.open_search_evidence(target, scan_active);
        self.screen = Screen::Knowledge;
        if let Some(action) = action {
            self.send_knowledge_action(action);
        }
    }

    fn search_error_feedback(&self, ui: &mut egui::Ui) {
        let Some(error) = &self.search_error else {
            return;
        };
        ui.colored_label(
            crate::theme::ERROR_CORAL,
            self.localization.text("search-error-title"),
        );
        ui.collapsing(self.localization.text("technical-details"), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("search_error_details")
                .max_height(88.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(egui::Label::new(error).wrap());
                });
        });
    }

    fn integrations(&mut self, ui: &mut egui::Ui) {
        let actions = self.integrations.show(ui, &self.localization);
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
        page_title(
            ui,
            &self.localization.text("devices-title"),
            &self.localization.text("devices-subtitle"),
        );
        self.connectivity_panel(ui);
        ui.add_space(10.0);
        ui.collapsing(self.localization.text("devices-manual-advanced"), |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
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
                        crate::theme::ERROR_CORAL,
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
        let matches = self.localization.text("devices-code-matches");
        let does_not_match = self.localization.text("devices-code-does-not-match");
        let revoke = self.localization.text("devices-revoke");
        let blocked_message = self.localization.text("devices-blocked-message");
        let pair_again = self.localization.text("devices-pair-again");
        let list_height = ui.available_height().max(0.0);
        egui::ScrollArea::vertical()
            .id_salt("peer_list")
            .max_height(list_height)
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for peer in &mut self.peers {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.heading(peer.device_name.as_deref().unwrap_or(&nearby_device));
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
                            if let Some(words) = &peer.sas_words {
                                ui.heading(words.join("  "));
                                ui.horizontal(|ui| {
                                    if ui.button(&matches).clicked() {
                                        self.worker.send(WorkerCommand::ConfirmPairing {
                                            peer_id: peer.peer_id.clone(),
                                            accepted: true,
                                        });
                                    }
                                    if ui.button(&does_not_match).clicked() {
                                        self.worker.send(WorkerCommand::ConfirmPairing {
                                            peer_id: peer.peer_id.clone(),
                                            accepted: false,
                                        });
                                    }
                                });
                            }
                        } else {
                            match peer.trust {
                                PeerTrustState::Unpaired => {
                                    if ui.button(&pair).clicked() {
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
                                    if ui.button(&revoke).clicked() {
                                        self.worker.send(WorkerCommand::RevokePeer {
                                            peer_id: peer.peer_id.clone(),
                                        });
                                    }
                                }
                                PeerTrustState::Blocked => {
                                    ui.colored_label(Color32::RED, &blocked_message);
                                    if ui.button(&pair_again).clicked() {
                                        self.worker.send(WorkerCommand::Pair {
                                            peer_id: peer.peer_id.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    });
                }
            });
        self.firewall_confirmation(ui.ctx());
    }

    fn connectivity_panel(&mut self, ui: &mut egui::Ui) {
        let preference = self
            .preferences
            .map_or(LanPreference::Undecided, |preferences| {
                preferences.lan_preference
            });
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.heading(self.localization.text("connectivity-title"));
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
                                crate::theme::ERROR_CORAL,
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
                                crate::theme::ERROR_CORAL,
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
                                crate::theme::WARNING_AMBER,
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
                                crate::theme::ERROR_CORAL,
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
                                crate::theme::WARNING_AMBER,
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
                                crate::theme::ERROR_CORAL,
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
                                crate::theme::WARNING_AMBER,
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
                            .button(self.localization.text("connectivity-disable"))
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
                ui.horizontal(|ui| {
                    let language_label = ui.label(self.localization.text("settings-language"));
                    egui::ComboBox::from_id_salt("ui_locale")
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
                });
                if self
                    .preferences
                    .is_some_and(|current| current.locale != locale)
                {
                    self.update_preferences(|preferences| preferences.locale = locale, false);
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let mut arguments = FluentArgs::new();
                    arguments.set(
                        "status",
                        autostart_status_label(&self.localization, self.autostart_status),
                    );
                    ui.label(
                        self.localization
                            .text_with("settings-login-status", Some(&arguments)),
                    );
                    let operation_idle = self.autostart_request_id.is_none();
                    if ui
                        .add_enabled(
                            operation_idle,
                            egui::Button::new(self.localization.text("action-enable")),
                        )
                        .clicked()
                    {
                        self.request_autostart(true);
                    }
                    if ui
                        .add_enabled(
                            operation_idle,
                            egui::Button::new(self.localization.text("action-disable")),
                        )
                        .clicked()
                    {
                        self.request_autostart(false);
                    }
                    if ui
                        .add_enabled(
                            operation_idle,
                            egui::Button::new(self.localization.text("settings-refresh-status")),
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
                ui.add_space(12.0);
                self.update_settings(ui);
                ui.add_space(12.0);
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
                ui.add_space(12.0);
                if let Some(state) = self.model_state.clone() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.heading(self.localization.text("settings-local-ai"));
                        let mut profile_arguments = FluentArgs::new();
                        profile_arguments
                            .set("profile", profile_label(&self.localization, state.profile));
                        ui.label(
                            self.localization
                                .text_with("settings-model-profile", Some(&profile_arguments)),
                        );
                        let mut active_arguments = FluentArgs::new();
                        active_arguments.set(
                            "model",
                            state
                                .active_model_id
                                .clone()
                                .unwrap_or_else(|| self.localization.text("settings-model-none")),
                        );
                        ui.label(
                            self.localization
                                .text_with("settings-model-active", Some(&active_arguments)),
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
                    });
                    ui.add_space(12.0);
                }
                ui.label(self.localization.text("settings-mcp-boundary"));
            });
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
                        crate::theme::ERROR_CORAL
                    } else {
                        crate::theme::VERIFIED_GREEN
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
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    crate::theme::ERROR_CORAL,
                    human_error_summary(&self.localization, &message),
                );
                if ui
                    .small_button(self.localization.text("action-dismiss"))
                    .clicked()
                {
                    dismiss = true;
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
                    crate::theme::ERROR_CORAL,
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
        egui::Frame::group(ui.style()).show(ui, |ui| {
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
                        ui.colored_label(crate::theme::WARNING_AMBER, message);
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

        ui.label(
            RichText::new(self.localization.text("onboarding-model-recommended"))
                .small()
                .strong()
                .color(ui.visuals().weak_text_color()),
        );
        if let Some(display_name) = &state.recommended_display_name {
            ui.heading(RichText::new(display_name).size(21.0));
        }
        ui.add(egui::Label::new(self.localization.text("onboarding-model-private")).wrap());

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
                crate::theme::ERROR_CORAL,
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
                crate::theme::VERIFIED_GREEN,
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
                if ui
                    .add_enabled(
                        can_install,
                        first_knowledge::primary_button(model_action_label(
                            &self.localization,
                            state.recommended_assets_installed,
                            self.models_ready,
                        )),
                    )
                    .clicked()
                {
                    self.worker.send(WorkerCommand::InstallModels);
                }
                if self.install_label.is_some()
                    && ui.button(self.localization.text("action-cancel")).clicked()
                {
                    self.worker.send(WorkerCommand::CancelInstall);
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
        let readiness = self.readiness_view();
        let published_count = self
            .collections
            .iter()
            .map(|collection| collection.published_count)
            .sum();
        let journey = derive_first_knowledge_journey(&readiness, published_count);
        let layout = ResponsiveLayout::from_available(ui.available_size());

        ui.set_width(ui.available_width().min(940.0));
        StripBuilder::new(ui)
            .size(Size::exact(first_knowledge::journey_header_height(
                layout.density,
            )))
            .size(Size::remainder())
            .size(Size::exact(first_knowledge::footer_height(layout.density)))
            .clip(true)
            .vertical(|mut strip| {
                strip.cell(|ui| {
                    first_knowledge::show_journey_header(
                        ui,
                        &self.localization,
                        visible_journey_states(journey),
                        layout.density,
                    );
                });
                strip.cell(|ui| {
                    let content_height = (ui.available_height()
                        - f32::from(first_knowledge::surface_margin(layout.density)) * 2.0)
                        .max(0.0);
                    first_knowledge::work_surface(ui, layout.density, |ui| {
                        ui.set_min_height(content_height);
                        match page {
                            OnboardingPage::Welcome => {
                                onboarding_title(
                                    ui,
                                    &self.localization.text("onboarding-welcome-title"),
                                    &self.localization.text("onboarding-welcome-body"),
                                    layout.density,
                                );
                                ui.heading(
                                    RichText::new(
                                        self.localization.text("onboarding-privacy-title"),
                                    )
                                    .size(18.0),
                                );
                                ui.label(self.localization.text("onboarding-privacy-local"));
                                ui.label(self.localization.text("onboarding-privacy-review"));
                                ui.add_space(16.0);
                                if ui
                                    .add(first_knowledge::primary_button(
                                        self.localization.text("onboarding-next"),
                                    ))
                                    .clicked()
                                {
                                    self.onboarding_page = Some(self.next_onboarding_page());
                                }
                            }
                            OnboardingPage::Model => {
                                onboarding_title(
                                    ui,
                                    &self.localization.text("onboarding-model-title"),
                                    &self.localization.text("onboarding-model-body"),
                                    layout.density,
                                );
                                self.onboarding_model(ui);
                                ui.add_space(16.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .button(self.localization.text("onboarding-back"))
                                        .clicked()
                                    {
                                        self.onboarding_page = Some(OnboardingPage::Welcome);
                                    }
                                    if ui
                                        .add_enabled(
                                            self.models_ready,
                                            first_knowledge::primary_button(
                                                self.localization.text("onboarding-next"),
                                            ),
                                        )
                                        .clicked()
                                    {
                                        self.onboarding_page = Some(self.next_onboarding_page());
                                    }
                                });
                            }
                            OnboardingPage::Collection => {
                                onboarding_title(
                                    ui,
                                    &self.localization.text("onboarding-collection-title"),
                                    &self.localization.text("onboarding-collection-body"),
                                    layout.density,
                                );
                                if !self.collections.is_empty() {
                                    ui.colored_label(
                                        crate::theme::VERIFIED_GREEN,
                                        self.localization.text("onboarding-collection-linked"),
                                    );
                                }
                                ui.add_space(12.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(first_knowledge::primary_button(
                                            self.localization.text("collections-choose-folder"),
                                        ))
                                        .clicked()
                                    {
                                        self.choose_and_add_collection();
                                    }
                                    if !self.collections.is_empty()
                                        && ui
                                            .add(first_knowledge::primary_button(
                                                self.localization.text("onboarding-next"),
                                            ))
                                            .clicked()
                                    {
                                        self.onboarding_page = Some(OnboardingPage::Processing);
                                    }
                                });
                                ui.add_space(14.0);
                                if ui
                                    .link(self.localization.text("onboarding-skip-folder"))
                                    .clicked()
                                {
                                    self.finish_onboarding();
                                }
                                ui.label(
                                    RichText::new(
                                        self.localization.text("onboarding-skip-folder-help"),
                                    )
                                    .small()
                                    .color(ui.visuals().weak_text_color()),
                                );
                            }
                            OnboardingPage::Processing => {
                                onboarding_title(
                                    ui,
                                    &self.localization.text("onboarding-processing-title"),
                                    &self.localization.text("onboarding-processing-body"),
                                    layout.density,
                                );
                                let document_count = self
                                    .collections
                                    .iter()
                                    .map(|collection| collection.document_count)
                                    .sum::<usize>();
                                let published_count = self
                                    .collections
                                    .iter()
                                    .map(|collection| collection.published_count)
                                    .sum::<usize>();
                                let needs_review_count = self
                                    .collections
                                    .iter()
                                    .map(|collection| collection.needs_review_count)
                                    .sum::<usize>();
                                let failed_count = self
                                    .collections
                                    .iter()
                                    .map(|collection| collection.failed_count)
                                    .sum::<usize>();
                                first_knowledge::show_processing_progress(
                                    ui,
                                    &self.localization,
                                    first_knowledge::processing_progress(
                                        document_count,
                                        published_count,
                                        needs_review_count,
                                        failed_count,
                                        self.source_issues.len(),
                                    ),
                                );
                                ui.add_space(8.0);
                                let failed_collections = self
                                    .collections
                                    .iter()
                                    .filter(|collection| {
                                        collection.maintenance.as_ref().is_some_and(|maintenance| {
                                            collection_maintenance_needs_recovery(
                                                maintenance.status,
                                            )
                                        })
                                    })
                                    .map(|collection| collection.id)
                                    .collect::<Vec<_>>();
                                let scan_finished = self.collection_scans.is_empty()
                                    && self.collections.iter().any(|collection| {
                                        collection.maintenance.as_ref().is_some_and(|maintenance| {
                            maintenance.status != airwiki_core::CollectionMaintenanceStatus::Never
                        })
                                    });
                                if scan_finished && !failed_collections.is_empty() {
                                    empty_state(
                                        ui,
                                        &self.localization.text("primary-resolve-folder-title"),
                                        &self.localization.text("primary-folder-explanation"),
                                    );
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add(first_knowledge::primary_button(
                                                self.localization.text("action-retry"),
                                            ))
                                            .clicked()
                                        {
                                            for collection_id in &failed_collections {
                                                self.collection_scans.insert(
                                                    *collection_id,
                                                    CollectionScanState::Queued,
                                                );
                                                self.knowledge
                                                    .collection_scan_started(*collection_id);
                                                self.worker.send(WorkerCommand::RescanCollection(
                                                    *collection_id,
                                                ));
                                            }
                                        }
                                        if ui
                                            .button(
                                                self.localization
                                                    .text("onboarding-processing-open-folder"),
                                            )
                                            .clicked()
                                        {
                                            self.screen = Screen::Collections;
                                            self.finish_onboarding();
                                        }
                                    });
                                } else if scan_finished && document_count == 0 {
                                    empty_state(
                                        ui,
                                        &self
                                            .localization
                                            .text("onboarding-processing-empty-title"),
                                        &self.localization.text("onboarding-processing-empty-body"),
                                    );
                                    if ui
                                        .button(
                                            self.localization
                                                .text("onboarding-processing-open-folder"),
                                        )
                                        .clicked()
                                    {
                                        self.screen = Screen::Collections;
                                        self.finish_onboarding();
                                    }
                                } else {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(if self.collection_scans.is_empty() {
                                            self.localization
                                                .text("onboarding-processing-enriching")
                                        } else {
                                            self.localization.text("onboarding-processing-scanning")
                                        });
                                    });
                                }
                            }
                            OnboardingPage::Review => {
                                onboarding_title(
                                    ui,
                                    &self.localization.text("onboarding-review-title"),
                                    &self.localization.text("onboarding-review-body"),
                                    layout.density,
                                );
                                if onboarding_review_requires_recovery(
                                    self.reviews.len(),
                                    self.source_issues.len(),
                                ) {
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add(first_knowledge::primary_button(
                                                self.localization
                                                    .text("onboarding-review-choose-folder"),
                                            ))
                                            .clicked()
                                        {
                                            self.choose_and_add_collection();
                                        }
                                        if ui
                                            .button(
                                                self.localization
                                                    .text("onboarding-review-continue"),
                                            )
                                            .clicked()
                                        {
                                            self.finish_onboarding();
                                        }
                                    });
                                    ui.add_space(8.0);
                                }
                                self.review_content(ui);
                            }
                            OnboardingPage::Search => {
                                onboarding_title(
                                    ui,
                                    &self.localization.text("onboarding-search-title"),
                                    &self.localization.text("onboarding-search-body"),
                                    layout.density,
                                );
                                self.search_form(ui, false);
                                ui.add_space(if layout.is_compact() { 8.0 } else { 16.0 });
                                let selected_evidence = if self.search_request_id.is_some() {
                                    self.search_feedback(ui, false)
                                } else if self.search_error.is_some() {
                                    self.search_error_feedback(ui);
                                    ui.add_space(8.0);
                                    if ui
                                        .add_enabled(
                                            !self.onboarding_finishing,
                                            egui::Button::new(
                                                self.localization
                                                    .text("onboarding-search-finish-later"),
                                            ),
                                        )
                                        .clicked()
                                    {
                                        self.finish_onboarding();
                                    }
                                    None
                                } else {
                                    let (title_id, body_id, button_id) =
                                        onboarding_search_completion(
                                            self.search_completed,
                                            !self.search_hits.is_empty(),
                                        );
                                    ui.heading(self.localization.text(title_id));
                                    ui.label(self.localization.text(body_id));
                                    if ui
                                        .add_enabled(
                                            !self.onboarding_finishing,
                                            first_knowledge::primary_button(
                                                self.localization.text(button_id),
                                            ),
                                        )
                                        .clicked()
                                    {
                                        self.finish_onboarding();
                                    }
                                    self.search_feedback(ui, false)
                                };
                                if let Some(target) = selected_evidence {
                                    self.open_search_evidence(target);
                                    self.finish_onboarding();
                                }
                            }
                        }
                    });
                });
                strip.cell(|ui| {
                    first_knowledge::privacy_note(ui, &self.localization);
                    if self.onboarding_finishing {
                        ui.spinner();
                    }
                });
            });
    }

    fn next_onboarding_page(&self) -> OnboardingPage {
        onboarding_page_for_state(
            self.models_ready,
            self.collections.len(),
            self.reviews.len(),
            self.source_issues.len(),
            self.collections
                .iter()
                .map(|collection| collection.published_count)
                .sum(),
        )
    }

    fn choose_and_add_collection(&self) {
        let Some(folder) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| self.localization.text("onboarding-default-folder-name"));
        self.worker
            .send(WorkerCommand::AddCollection { name, folder });
    }

    fn reconcile_onboarding_page(&mut self) {
        if self.onboarding_finishing {
            return;
        }
        self.onboarding_page = advance_onboarding_page(
            self.onboarding_page,
            self.collections.len(),
            self.reviews.len(),
            self.source_issues.len(),
            self.collections
                .iter()
                .map(|collection| collection.published_count)
                .sum(),
        );
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

fn readiness_component_label(localization: &Localization, component: ReadinessComponent) -> String {
    localization.text(match component {
        ReadinessComponent::LocalAi => "component-local-ai",
        ReadinessComponent::Collections => "component-collections",
        ReadinessComponent::Review => "component-review",
        ReadinessComponent::Wiki => "component-wiki",
        ReadinessComponent::Lan => "component-lan",
        ReadinessComponent::Chat => "component-chat",
        ReadinessComponent::Background => "component-background",
        ReadinessComponent::Updates => "component-updates",
    })
}

fn readiness_status_presentation(
    localization: &Localization,
    status: ReadinessStatus,
) -> (String, Color32) {
    let (message, color) = match status {
        ReadinessStatus::Ready => ("status-ready", crate::theme::VERIFIED_GREEN),
        ReadinessStatus::Working => ("status-working", crate::theme::AIR_BLUE),
        ReadinessStatus::NeedsPermission => {
            ("status-needs-permission", crate::theme::WARNING_AMBER)
        }
        ReadinessStatus::NeedsAttention => ("status-needs-attention", crate::theme::ERROR_CORAL),
        ReadinessStatus::OptionalDisabled => (
            "status-optional-disabled",
            crate::theme::secondary_text(true),
        ),
    };
    (localization.text(message), color)
}

fn maintenance_status_presentation(
    localization: &Localization,
    status: airwiki_core::CollectionMaintenanceStatus,
) -> (String, Color32) {
    let (message, color) = match status {
        airwiki_core::CollectionMaintenanceStatus::Never => {
            ("maintenance-never", crate::theme::secondary_text(true))
        }
        airwiki_core::CollectionMaintenanceStatus::Success => {
            ("maintenance-success", crate::theme::VERIFIED_GREEN)
        }
        airwiki_core::CollectionMaintenanceStatus::Partial => {
            ("maintenance-partial", crate::theme::WARNING_AMBER)
        }
        airwiki_core::CollectionMaintenanceStatus::Failed => {
            ("maintenance-failed", crate::theme::ERROR_CORAL)
        }
        airwiki_core::CollectionMaintenanceStatus::Quarantined => {
            ("maintenance-quarantined", crate::theme::ERROR_CORAL)
        }
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
        .fill(Color32::from_rgba_unmultiplied(230, 160, 35, 18))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(155, 105, 25)))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(RichText::new(&issue.source_name).strong());
            ui.label(
                RichText::new(&issue.collection_name)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.label(
                RichText::new(localization.text("review-issue-status"))
                    .small()
                    .strong()
                    .color(crate::theme::WARNING_AMBER),
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
        OnboardingPage::Welcome => false,
        OnboardingPage::Model => {
            normalized == "local_ai_unavailable"
                || contains_any(&[
                    "model", "modelo", "infer", "asset", "artifact", "hash", "memory", "memoria",
                    "disk", "space", "espacio",
                ])
        }
        OnboardingPage::Collection | OnboardingPage::Processing | OnboardingPage::Review => {
            contains_any(&[
                "collection",
                "colección",
                "document",
                "wiki",
                "markdown",
                "pdf",
                "chunk",
            ])
        }
        OnboardingPage::Search => {
            normalized == "search_unavailable"
                || contains_any(&["search", "búsqueda", "index", "índice", "fts", "embedding"])
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
        crate::layout::LayoutDensity::Compact => (20.0, 14.0, 8.0),
        crate::layout::LayoutDensity::Comfortable => (24.0, 15.0, 16.0),
    };
    ui.heading(RichText::new(title).size(title_size).strong());
    ui.add(egui::Label::new(RichText::new(body).size(body_size)).wrap());
    ui.add_space(gap);
}

fn onboarding_page_for_state(
    models_ready: bool,
    collection_count: usize,
    review_count: usize,
    issue_count: usize,
    published_count: usize,
) -> OnboardingPage {
    if !models_ready {
        OnboardingPage::Model
    } else if collection_count == 0 {
        OnboardingPage::Collection
    } else if published_count > 0 {
        OnboardingPage::Search
    } else if review_count > 0 || issue_count > 0 {
        OnboardingPage::Review
    } else {
        OnboardingPage::Processing
    }
}

fn advance_onboarding_page(
    current: Option<OnboardingPage>,
    collection_count: usize,
    review_count: usize,
    issue_count: usize,
    published_count: usize,
) -> Option<OnboardingPage> {
    match current {
        Some(OnboardingPage::Collection) if collection_count > 0 => {
            Some(OnboardingPage::Processing)
        }
        Some(OnboardingPage::Processing | OnboardingPage::Review) if published_count > 0 => {
            Some(OnboardingPage::Search)
        }
        Some(OnboardingPage::Processing) if review_count > 0 || issue_count > 0 => {
            Some(OnboardingPage::Review)
        }
        other => other,
    }
}

fn onboarding_review_requires_recovery(review_count: usize, issue_count: usize) -> bool {
    review_count == 0 && issue_count > 0
}

fn collection_maintenance_needs_recovery(
    status: airwiki_core::CollectionMaintenanceStatus,
) -> bool {
    matches!(
        status,
        airwiki_core::CollectionMaintenanceStatus::Failed
            | airwiki_core::CollectionMaintenanceStatus::Quarantined
    )
}

fn onboarding_search_completion(
    search_completed: bool,
    has_hits: bool,
) -> (&'static str, &'static str, &'static str) {
    if has_hits {
        (
            "onboarding-search-ready-title",
            "onboarding-search-ready-body",
            "onboarding-search-finish",
        )
    } else if search_completed {
        (
            "onboarding-search-empty-title",
            "onboarding-search-empty-body",
            "onboarding-search-finish-later",
        )
    } else {
        (
            "onboarding-search-optional-title",
            "onboarding-search-optional-body",
            "onboarding-search-finish-later",
        )
    }
}

fn selected_review_after_refresh(
    selected: Option<Uuid>,
    reviews: &[ReviewItemView],
) -> Option<Uuid> {
    selected
        .filter(|selected| reviews.iter().any(|review| review.concept_id == *selected))
        .or_else(|| reviews.first().map(|review| review.concept_id))
}

fn visible_journey_states(journey: FirstKnowledgeJourneyView) -> [JourneyStepState; 5] {
    let choose_folder =
        visible_journey_state(journey.stage_state(FirstKnowledgeStage::ChooseKnowledgeFolder));
    let process = visible_journey_state(journey.stage_state(FirstKnowledgeStage::ProcessKnowledge));
    [
        visible_journey_state(journey.stage_state(FirstKnowledgeStage::PrepareLocalAi)),
        merge_read_state(choose_folder, process),
        visible_journey_state(journey.stage_state(FirstKnowledgeStage::ReviewKnowledge)),
        visible_journey_state(journey.stage_state(FirstKnowledgeStage::PublishReady)),
        visible_journey_state(journey.stage_state(FirstKnowledgeStage::SearchKnowledge)),
    ]
}

fn visible_journey_state(state: FirstKnowledgeStepState) -> JourneyStepState {
    match state {
        FirstKnowledgeStepState::Complete => JourneyStepState::Complete,
        FirstKnowledgeStepState::Current | FirstKnowledgeStepState::Working => {
            JourneyStepState::Current
        }
        FirstKnowledgeStepState::NeedsPermission | FirstKnowledgeStepState::NeedsAttention => {
            JourneyStepState::Attention
        }
        FirstKnowledgeStepState::Pending => JourneyStepState::Upcoming,
    }
}

fn merge_read_state(
    choose_folder: JourneyStepState,
    process: JourneyStepState,
) -> JourneyStepState {
    if matches!(
        (choose_folder, process),
        (JourneyStepState::Attention, _) | (_, JourneyStepState::Attention)
    ) {
        JourneyStepState::Attention
    } else if matches!(
        (choose_folder, process),
        (JourneyStepState::Current, _) | (_, JourneyStepState::Current)
    ) {
        JourneyStepState::Current
    } else if choose_folder == JourneyStepState::Complete && process == JourneyStepState::Complete {
        JourneyStepState::Complete
    } else {
        JourneyStepState::Upcoming
    }
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
        self.reconcile_onboarding_page();
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
        if self.onboarding_page.is_some() {
            egui::CentralPanel::default().show(root, |ui| {
                self.onboarding(ui);
            });
            self.onboarding_notices(root);
            self.close_confirmation(root.ctx());
            return;
        }
        self.sidebar(root);
        if self.screen != Screen::Setup {
            self.notices(root);
        }
        egui::CentralPanel::default().show(root, |ui| match self.screen {
            Screen::Setup => self.home(ui),
            Screen::Models => self.setup(ui),
            Screen::Collections => self.collections(ui),
            Screen::Review => self.review(ui),
            Screen::Knowledge => self.knowledge(ui),
            Screen::Search => self.search(ui),
            Screen::Integrations => self.integrations(ui),
            Screen::Nodes => self.nodes(ui),
            Screen::Settings => self.settings(ui),
        });
        self.close_confirmation(root.ctx());
    }
}

fn configure_style(context: &egui::Context) {
    crate::theme::apply(context);
}

fn nav(ui: &mut egui::Ui, current: &mut Screen, target: Screen, label: &str) {
    if ui
        .add_sized(
            [178.0, 34.0],
            egui::Button::selectable(*current == target, label),
        )
        .clicked()
    {
        *current = target;
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

fn page_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.heading(RichText::new(title).size(28.0));
    ui.label(RichText::new(subtitle).color(crate::theme::secondary_text(ui.visuals().dark_mode)));
    ui.add_space(18.0);
}

fn empty_state(ui: &mut egui::Ui, title: &str, body: &str) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.heading(title);
            ui.label(body);
        });
        ui.add_space(20.0);
    });
}

fn deduplicate_notices(notices: &mut VecDeque<(bool, String)>) {
    let mut seen = HashSet::new();
    notices.retain(|notice| seen.insert(notice.clone()));
}

fn search_result_applies(active_request_id: Option<Uuid>, event_request_id: Uuid) -> bool {
    active_request_id == Some(event_request_id)
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
    ui.heading(localization.text("review-metadata-title"));
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
        let render_fields = |ui: &mut egui::Ui| {
            ui.add(
                egui::TextEdit::singleline(&mut entity.name)
                    .hint_text(localization.text("review-entity-name")),
            );
            ui.add(
                egui::TextEdit::singleline(&mut entity.kind)
                    .hint_text(localization.text("review-entity-type")),
            );
            if ui
                .small_button(localization.text("action-remove"))
                .clicked()
            {
                remove_entity = Some(index);
            }
        };
        if review_fields_stack(ui.available_width()) {
            ui.vertical(render_fields);
        } else {
            ui.horizontal(render_fields);
        }
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
        let render_fields = |ui: &mut egui::Ui| {
            ui.add(
                egui::TextEdit::singleline(&mut link.label)
                    .hint_text(localization.text("review-link-label")),
            );
            ui.add(
                egui::TextEdit::singleline(&mut link.target)
                    .hint_text(localization.text("review-link-target")),
            );
            if ui
                .small_button(localization.text("action-remove"))
                .clicked()
            {
                remove_link = Some(index);
            }
        };
        if review_fields_stack(ui.available_width()) {
            ui.vertical(render_fields);
        } else {
            ui.horizontal(render_fields);
        }
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

#[cfg(test)]
mod tests {
    use fluent_bundle::FluentArgs;

    use super::{
        ExternalAiPolicyChange, OnboardingPage, SearchResultAvailability, WikiHealthCheckState,
        advance_onboarding_page, classify_external_ai_policy_change, classify_search_result,
        collection_maintenance_needs_recovery, connectivity_runtime_is_active, deduplicate_notices,
        elapsed_minutes, firewall_configuration_is_current, firewall_operation_update_applies,
        firewall_state_offers_advanced_recovery, human_error_summary, localized_worker_notice,
        model_action_label, onboarding_error_is_relevant, onboarding_page_for_state,
        onboarding_review_requires_recovery, onboarding_search_completion,
        parse_manual_ipv4_address, peer_activity_message_id, primary_action_explanation,
        primary_action_title, review_fields_stack, sanitized_error_code, search_coverage_message,
        search_result_applies, search_result_origin_label, should_present_pairing_controls,
        updater_launched_installer, visible_journey_states, wiki_health_readiness_inputs,
        wiki_health_result_applies,
    };
    use crate::connectivity_platform::{
        ConnectivityPlatformSnapshot, FirewallDiagnosticState, FirewallHelperState,
        NetworkProfileState, SystemPermissionState,
    };
    use crate::i18n::{Localization, UiLocale};
    use crate::model_config::LanPreference;
    use crate::readiness::{
        FirstKnowledgeCta, FirstKnowledgeJourneyView, FirstKnowledgeStage, FirstKnowledgeStepState,
        RecommendedAction,
    };
    use crate::updater::{UpdateSummary, UpdaterStatus, UpdaterView};
    use crate::worker::{
        FirewallOperationView, LanDiscoveryView, LanListenerView, PeerActivityState,
        PeerTrustState, SearchCoverageView, SourceIssueView, UpdaterWorkerView,
        WikiHealthSummaryView,
    };
    use airwiki_core::SourceIssueCode;
    use std::{
        collections::VecDeque,
        time::{Duration, SystemTime},
    };
    use uuid::Uuid;

    #[test]
    fn first_run_targets_the_earliest_unfinished_knowledge_step() {
        for (models, collections, reviews, issues, published, expected) in [
            (false, 0, 0, 0, 0, OnboardingPage::Model),
            (true, 0, 0, 0, 0, OnboardingPage::Collection),
            (true, 1, 0, 0, 0, OnboardingPage::Processing),
            (true, 1, 1, 0, 0, OnboardingPage::Review),
            (true, 1, 0, 1, 0, OnboardingPage::Review),
            (true, 1, 0, 0, 1, OnboardingPage::Search),
        ] {
            assert_eq!(
                onboarding_page_for_state(models, collections, reviews, issues, published),
                expected
            );
        }
    }

    #[test]
    fn first_run_advances_from_real_worker_state_without_optional_setup() {
        assert_eq!(
            advance_onboarding_page(Some(OnboardingPage::Collection), 1, 0, 0, 0),
            Some(OnboardingPage::Processing)
        );
        assert_eq!(
            advance_onboarding_page(Some(OnboardingPage::Processing), 1, 1, 0, 0),
            Some(OnboardingPage::Review)
        );
        assert_eq!(
            advance_onboarding_page(Some(OnboardingPage::Review), 1, 0, 0, 1),
            Some(OnboardingPage::Search)
        );
        assert_eq!(
            advance_onboarding_page(Some(OnboardingPage::Welcome), 1, 1, 0, 1),
            Some(OnboardingPage::Welcome)
        );
        assert_eq!(
            advance_onboarding_page(Some(OnboardingPage::Processing), 1, 0, 1, 0),
            Some(OnboardingPage::Review)
        );
    }

    #[test]
    fn issue_only_onboarding_offers_a_safe_exit() {
        assert!(onboarding_review_requires_recovery(0, 1));
        assert!(!onboarding_review_requires_recovery(1, 1));
        assert!(!onboarding_review_requires_recovery(0, 0));
    }

    #[test]
    fn zero_result_onboarding_can_finish_and_search_later() {
        assert_eq!(
            onboarding_search_completion(true, false),
            (
                "onboarding-search-empty-title",
                "onboarding-search-empty-body",
                "onboarding-search-finish-later",
            )
        );
    }

    #[test]
    fn unsearched_onboarding_does_not_claim_that_evidence_is_missing() {
        assert_eq!(
            onboarding_search_completion(false, false).0,
            "onboarding-search-optional-title"
        );
    }

    #[test]
    fn successful_onboarding_search_keeps_the_ready_completion() {
        assert_eq!(
            onboarding_search_completion(true, true).2,
            "onboarding-search-finish"
        );
    }

    #[test]
    fn terminal_collection_states_offer_onboarding_recovery() {
        let states = [
            airwiki_core::CollectionMaintenanceStatus::Failed,
            airwiki_core::CollectionMaintenanceStatus::Quarantined,
            airwiki_core::CollectionMaintenanceStatus::Success,
        ]
        .map(collection_maintenance_needs_recovery);

        assert_eq!(states, [true, true, false]);
    }

    #[test]
    fn review_fields_stack_in_the_minimum_window_editor() {
        assert!(review_fields_stack(337.0));
        assert!(!review_fields_stack(520.0));
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
            OnboardingPage::Processing,
            "the collection could not process a PDF"
        ));
        assert!(onboarding_error_is_relevant(
            OnboardingPage::Welcome,
            "No se pudieron iniciar los servicios privados"
        ));
    }

    #[test]
    fn six_internal_conditions_render_as_five_human_steps() {
        let journey = FirstKnowledgeJourneyView {
            current_stage: FirstKnowledgeStage::ReviewKnowledge,
            current_state: FirstKnowledgeStepState::Current,
            cta: Some(FirstKnowledgeCta::Recommended(
                RecommendedAction::ReviewPendingKnowledge,
            )),
        };

        assert_eq!(
            visible_journey_states(journey),
            [
                super::JourneyStepState::Complete,
                super::JourneyStepState::Complete,
                super::JourneyStepState::Current,
                super::JourneyStepState::Upcoming,
                super::JourneyStepState::Upcoming,
            ]
        );
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

            assert!(
                offline.contains('2') && offline.contains(expected_offline),
                "unexpected localized coverage message: {offline:?}"
            );
            assert!(!offline.contains("12D3Koo"));
            assert!(!disabled.contains("federation_disabled"));
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
    fn search_results_apply_only_to_the_active_request() {
        let active = Uuid::new_v4();

        assert!(search_result_applies(Some(active), active));
        assert!(!search_result_applies(Some(active), Uuid::new_v4()));
        assert!(!search_result_applies(None, active));
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
