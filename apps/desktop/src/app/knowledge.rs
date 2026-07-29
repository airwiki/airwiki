use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use airwiki_core::{
    BundleHealthReport, GuidedRepairChange, GuidedRepairPreview, GuidedRepairResult,
    HealthRecovery, HealthSeverity, KnowledgeBundleState, KnowledgeBundleView,
    KnowledgeConceptView, KnowledgeLinkDisposition, KnowledgePageId, KnowledgePageView,
    RepairAuthority,
};
use airwiki_types::SearchHit;
use eframe::egui::{self, Color32, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_graphs::Graph;
use uuid::Uuid;

use super::wrap_monospace;
use crate::{i18n::Localization, layout::ResponsiveLayout};

const MAX_GRAPH_CONCEPTS: usize = 500;
const MAX_EDITORIAL_GRAPH_NODES: usize = 12;
const INDEX_FILTER_THRESHOLD: usize = 12;
// Keep a margin for the persisted egui_graphs layout state around our own step.
const GRAPH_LAYOUT_WORK_BUDGET: Duration = Duration::from_millis(3);
const MAX_LAYOUT_NODES_PER_FRAME: usize = 64;
const UPDATING_RETRY_DELAY: Duration = Duration::from_millis(750);
const NARROW_WIKI_THRESHOLD: f32 = 760.0;
const WIKI_INDEX_COLUMN_GAP: f32 = 56.0;

#[derive(Debug, Clone)]
pub(super) enum KnowledgeAction {
    LoadBundle {
        request_id: Uuid,
        collection_id: Uuid,
    },
    LoadPage {
        request_id: Uuid,
        collection_id: Uuid,
        page_id: KnowledgePageId,
        expected_fingerprint: String,
    },
    PrepareGuidedRepair {
        request_id: Uuid,
        collection_id: Uuid,
    },
    ExecuteGuidedRepair {
        request_id: Uuid,
        preview: GuidedRepairPreview,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SearchEvidenceTarget {
    collection_id: Uuid,
    concept_id: Uuid,
    heading_or_page: String,
    logical_resource_uri: String,
    source_revision: u32,
    source_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecentConceptView {
    pub(super) id: Uuid,
    pub(super) title: String,
    pub(super) summary: String,
    pub(super) concept_type: String,
    pub(super) collection_name: String,
    pub(super) reviewed_at: String,
}

impl From<&SearchHit> for SearchEvidenceTarget {
    fn from(hit: &SearchHit) -> Self {
        Self {
            collection_id: hit.collection_id,
            concept_id: hit.concept_id,
            heading_or_page: hit.heading_or_page.clone(),
            logical_resource_uri: hit.logical_resource_uri.clone(),
            source_revision: hit.source_revision,
            source_sha256: hit.source_sha256.clone(),
        }
    }
}

impl SearchEvidenceTarget {
    pub(super) fn collection_id(&self) -> Uuid {
        self.collection_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KnowledgeTab {
    Wiki,
    Graph,
    Health,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConceptHealthTone {
    Healthy,
    Attention,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WikiLayoutMode {
    Compact,
    Wide,
}

const fn wiki_layout_mode(available_width: f32) -> WikiLayoutMode {
    if available_width < NARROW_WIKI_THRESHOLD {
        WikiLayoutMode::Compact
    } else {
        WikiLayoutMode::Wide
    }
}

#[derive(Debug, Clone)]
struct PendingBundle {
    request_id: Uuid,
    collection_id: Uuid,
}

#[derive(Debug, Clone)]
struct PendingPage {
    request_id: Uuid,
    collection_id: Uuid,
    page_id: KnowledgePageId,
    expected_fingerprint: String,
}

#[derive(Debug, Clone)]
struct KnowledgeGraphNode {
    page_id: KnowledgePageId,
    title: String,
    concept_type: String,
    tags: Vec<String>,
}

type KnowledgeGraph = Graph<KnowledgeGraphNode, ()>;

#[derive(Debug, Clone)]
struct KnowledgeGraphCache {
    key: String,
    graph: KnowledgeGraph,
    layout: IncrementalGraphLayout,
}

#[derive(Debug, Clone, Default)]
struct IncrementalGraphLayout {
    next_node: usize,
    stable: bool,
}

impl IncrementalGraphLayout {
    fn advance(&mut self, graph: &mut KnowledgeGraph) -> usize {
        self.advance_with_limits(graph, GRAPH_LAYOUT_WORK_BUDGET, MAX_LAYOUT_NODES_PER_FRAME)
    }

    fn advance_with_limits(
        &mut self,
        graph: &mut KnowledgeGraph,
        time_budget: Duration,
        node_budget: usize,
    ) -> usize {
        if self.stable || graph.node_count() == 0 || node_budget == 0 || time_budget.is_zero() {
            return 0;
        }

        let started = Instant::now();
        let total_nodes = graph.node_count();
        let mut processed = 0usize;
        let pending = graph
            .g()
            .node_indices()
            .skip(self.next_node)
            .take(node_budget)
            .collect::<Vec<_>>();

        for node_index in pending {
            if processed > 0 && started.elapsed() >= time_budget {
                break;
            }
            let position = deterministic_graph_position(self.next_node, total_nodes);
            graph
                .node_mut(node_index)
                .expect("the graph node selected for layout exists")
                .set_location(position);
            self.next_node += 1;
            processed += 1;
        }

        self.stable = self.next_node >= total_nodes;
        processed
    }
}

pub(super) struct KnowledgeUi {
    tab: KnowledgeTab,
    collection_id: Option<Uuid>,
    bundle: Option<Arc<KnowledgeBundleView>>,
    bundle_pending: Option<PendingBundle>,
    bundle_error: Option<String>,
    selected_page: Option<KnowledgePageId>,
    page: Option<Arc<KnowledgePageView>>,
    page_pending: Option<PendingPage>,
    page_error: Option<String>,
    query_filter: String,
    type_filter: Option<String>,
    tag_filter: Option<String>,
    markdown_cache: CommonMarkCache,
    graph: Option<KnowledgeGraphCache>,
    pending_external_url: Option<String>,
    link_notice: Option<(bool, String)>,
    snapshot_stale: bool,
    retry_bundle_at: Option<Instant>,
    page_recovery_attempted: bool,
    search_evidence: Option<SearchEvidenceTarget>,
    search_evidence_focus_pending: bool,
    guided_repair_prepare_pending: Option<PendingBundle>,
    guided_repair_execute_pending: Option<PendingBundle>,
    guided_repair_preview: Option<GuidedRepairPreview>,
    guided_repair_error: Option<(Uuid, String)>,
    guided_repair_result: Option<GuidedRepairResult>,
}

impl Default for KnowledgeUi {
    fn default() -> Self {
        Self {
            tab: KnowledgeTab::Wiki,
            collection_id: None,
            bundle: None,
            bundle_pending: None,
            bundle_error: None,
            selected_page: None,
            page: None,
            page_pending: None,
            page_error: None,
            query_filter: String::new(),
            type_filter: None,
            tag_filter: None,
            markdown_cache: CommonMarkCache::default(),
            graph: None,
            pending_external_url: None,
            link_notice: None,
            snapshot_stale: false,
            retry_bundle_at: None,
            page_recovery_attempted: false,
            search_evidence: None,
            search_evidence_focus_pending: false,
            guided_repair_prepare_pending: None,
            guided_repair_execute_pending: None,
            guided_repair_preview: None,
            guided_repair_error: None,
            guided_repair_result: None,
        }
    }
}

impl KnowledgeUi {
    pub(super) fn prepare_recent_concepts(
        &mut self,
        collections: &[(Uuid, String)],
        active_scans: &HashSet<Uuid>,
    ) -> Vec<KnowledgeAction> {
        let mut actions = Vec::new();
        self.ensure_collection(collections, active_scans, &mut actions);
        actions
    }

    pub(super) fn recent_concepts(&self) -> Vec<RecentConceptView> {
        let Some(bundle) = self.bundle.as_ref() else {
            return Vec::new();
        };
        let mut concepts = bundle.concepts.iter().collect::<Vec<_>>();
        concepts.sort_by(|left, right| {
            right
                .reviewed_at
                .as_ref()
                .or(right.timestamp.as_ref())
                .cmp(&left.reviewed_at.as_ref().or(left.timestamp.as_ref()))
                .then_with(|| left.title.cmp(&right.title))
        });
        concepts
            .into_iter()
            .take(4)
            .map(|concept| RecentConceptView {
                id: concept.id,
                title: concept.title.clone(),
                summary: concept.description.clone(),
                concept_type: concept.concept_type.clone(),
                collection_name: bundle.collection_name.clone(),
                reviewed_at: concept
                    .reviewed_at
                    .as_ref()
                    .or(concept.timestamp.as_ref())
                    .map(|date| date.format("%Y-%m-%d").to_string())
                    .unwrap_or_default(),
            })
            .collect()
    }

    pub(super) fn open_recent_concept(&mut self, concept_id: Uuid) -> Option<KnowledgeAction> {
        self.tab = KnowledgeTab::Wiki;
        self.request_page(KnowledgePageId::Concept(concept_id))
    }

    pub(super) fn open_search_evidence(
        &mut self,
        target: SearchEvidenceTarget,
        scan_active: bool,
    ) -> Option<KnowledgeAction> {
        self.tab = KnowledgeTab::Wiki;
        self.query_filter.clear();
        self.type_filter = None;
        self.tag_filter = None;
        self.link_notice = None;
        self.page_recovery_attempted = false;

        let collection_changed = self.collection_id != Some(target.collection_id);
        if collection_changed {
            self.collection_id = Some(target.collection_id);
            self.clear_snapshot();
        } else if scan_active {
            self.invalidate_snapshot_preserving_selection();
            self.snapshot_stale = true;
        }

        self.selected_page = Some(KnowledgePageId::Concept(target.concept_id));
        self.search_evidence = Some(target);
        self.search_evidence_focus_pending = true;
        if scan_active || self.bundle_pending.is_some() {
            return None;
        }
        if self.bundle.is_some() {
            return self.queue_verified_search_evidence();
        }
        self.collection_id.map(|id| self.request_bundle(id))
    }

    pub(super) fn select_health(
        &mut self,
        collection_id: Option<Uuid>,
        scan_active: bool,
    ) -> Option<KnowledgeAction> {
        self.tab = KnowledgeTab::Health;
        let collection_id = collection_id?;
        if self.collection_id == Some(collection_id) {
            if scan_active {
                return None;
            }
            self.clear_snapshot();
            return Some(self.request_bundle(collection_id));
        }
        self.select_collection(collection_id, scan_active)
    }

    pub(super) fn bundle_loaded(
        &mut self,
        request_id: Uuid,
        collection_id: Uuid,
        result: Result<KnowledgeBundleView, String>,
    ) -> Option<KnowledgeAction> {
        let is_current = self.bundle_pending.as_ref().is_some_and(|pending| {
            pending.request_id == request_id && pending.collection_id == collection_id
        });
        if !is_current {
            return None;
        }
        self.bundle_pending = None;

        match result {
            Ok(bundle) => {
                if bundle.collection_id != collection_id {
                    self.bundle = None;
                    self.page = None;
                    self.page_pending = None;
                    self.bundle_error = Some("knowledge-error-wrong-collection".to_owned());
                    return None;
                }
                if matches!(bundle.state, KnowledgeBundleState::Updating) {
                    self.bundle_error = None;
                    self.bundle = Some(Arc::new(bundle));
                    self.page = None;
                    self.page_pending = None;
                    self.page_error = None;
                    self.graph = None;
                    self.snapshot_stale = true;
                    self.retry_bundle_at = Some(Instant::now() + UPDATING_RETRY_DELAY);
                    return None;
                }
                self.bundle_error = None;
                self.graph = None;
                self.snapshot_stale = false;
                self.retry_bundle_at = None;
                let search_target_pending = self
                    .search_evidence
                    .as_ref()
                    .is_some_and(|target| target.collection_id == collection_id);
                if search_target_pending {
                    self.bundle = Some(Arc::new(bundle));
                    self.page = None;
                    self.page_error = None;
                    return self.queue_verified_search_evidence();
                }
                let selected = self
                    .selected_page
                    .filter(|page_id| page_fingerprint(&bundle, *page_id).is_some())
                    .or_else(|| default_page(&bundle));
                self.bundle = Some(Arc::new(bundle));
                self.page = None;
                self.page_error = None;
                self.selected_page = selected;
                let action = selected.and_then(|page_id| self.queue_page(page_id));
                if action.is_none() {
                    self.page_recovery_attempted = false;
                    self.link_notice = None;
                }
                action
            }
            Err(error) => {
                self.bundle = None;
                self.page = None;
                self.page_pending = None;
                self.bundle_error = Some(error);
                self.retry_bundle_at = None;
                None
            }
        }
    }

    pub(super) fn page_loaded(
        &mut self,
        request_id: Uuid,
        collection_id: Uuid,
        page_id: KnowledgePageId,
        result: Result<KnowledgePageView, String>,
    ) -> Option<KnowledgeAction> {
        let pending = self.page_pending.as_ref()?;
        let is_current = {
            pending.request_id == request_id
                && pending.collection_id == collection_id
                && pending.page_id == page_id
        };
        if !is_current {
            return None;
        }
        let expected_fingerprint = pending.expected_fingerprint.clone();
        self.page_pending = None;
        match result {
            Ok(page) => {
                if page.collection_id != collection_id
                    || page.page_id != page_id
                    || page.fingerprint != expected_fingerprint
                {
                    return self.recover_page_after_stale(
                        collection_id,
                        "knowledge-error-stale-snapshot".to_owned(),
                    );
                }
                self.page_error = None;
                self.selected_page = Some(page_id);
                self.page = Some(Arc::new(page));
                self.page_recovery_attempted = false;
                self.link_notice = None;
                None
            }
            Err(error) => self.recover_page_after_stale(collection_id, error),
        }
    }

    pub(super) fn mark_snapshot_stale(
        &mut self,
        collection_id: Option<Uuid>,
        reload_now: bool,
    ) -> Option<KnowledgeAction> {
        let selected_collection = self.collection_id?;
        if collection_id.is_some_and(|changed| changed != selected_collection) {
            return None;
        }
        if self.snapshot_stale && (self.bundle_pending.is_some() || self.retry_bundle_at.is_some())
        {
            return None;
        }

        self.invalidate_snapshot_preserving_selection();
        self.snapshot_stale = true;
        reload_now.then(|| self.request_bundle(selected_collection))
    }

    pub(super) fn guided_repair_prepared(
        &mut self,
        request_id: Uuid,
        collection_id: Uuid,
        result: Result<GuidedRepairPreview, String>,
    ) {
        let is_current = self
            .guided_repair_prepare_pending
            .as_ref()
            .is_some_and(|pending| {
                pending.request_id == request_id && pending.collection_id == collection_id
            });
        if !is_current {
            return;
        }
        self.guided_repair_prepare_pending = None;
        match result {
            Ok(preview) if preview.collection_id == collection_id => {
                self.guided_repair_error = None;
                self.guided_repair_preview = Some(preview);
            }
            Ok(_) => {
                self.guided_repair_error = Some((
                    collection_id,
                    "wiki_repair_preview_wrong_collection".to_owned(),
                ));
            }
            Err(error) => self.guided_repair_error = Some((collection_id, error)),
        }
    }

    pub(super) fn guided_repair_finished(
        &mut self,
        request_id: Uuid,
        collection_id: Uuid,
        result: Result<GuidedRepairResult, String>,
        reload_now: bool,
    ) -> Option<KnowledgeAction> {
        let is_current = self
            .guided_repair_execute_pending
            .as_ref()
            .is_some_and(|pending| {
                pending.request_id == request_id && pending.collection_id == collection_id
            });
        if !is_current {
            return None;
        }
        self.guided_repair_execute_pending = None;
        self.guided_repair_preview = None;
        match result {
            Ok(result) if result.collection_id == collection_id => {
                self.guided_repair_error = None;
                self.guided_repair_result = Some(result);
                self.mark_snapshot_stale(Some(collection_id), reload_now)
            }
            Ok(_) => {
                self.guided_repair_error = Some((
                    collection_id,
                    "wiki_repair_result_wrong_collection".to_owned(),
                ));
                None
            }
            Err(error) => {
                self.guided_repair_error = Some((collection_id, error));
                self.mark_snapshot_stale(Some(collection_id), reload_now)
            }
        }
    }

    pub(super) fn collection_scan_started(&mut self, collection_id: Uuid) {
        if self.collection_id != Some(collection_id) {
            return;
        }

        // A scan can start while an inspector request is still in flight. Clear
        // its request id as well as the visible snapshot so any late response is
        // ignored and cannot expose a half-updated bundle.
        self.invalidate_snapshot_preserving_selection();
        self.snapshot_stale = true;
    }

    pub(super) fn collection_scan_finished(
        &mut self,
        collection_id: Uuid,
        reload_now: bool,
    ) -> Option<KnowledgeAction> {
        self.mark_snapshot_stale(Some(collection_id), reload_now)
    }

    pub(super) fn collections_changed(
        &mut self,
        active_scans: &HashSet<Uuid>,
        reload_now: bool,
    ) -> Option<KnowledgeAction> {
        let selected_collection = self.collection_id?;
        if active_scans.contains(&selected_collection) {
            return None;
        }
        self.mark_snapshot_stale(None, reload_now)
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        collections: &[(Uuid, String)],
        active_scans: &HashSet<Uuid>,
    ) -> Vec<KnowledgeAction> {
        let mut actions = Vec::new();
        self.ensure_collection(collections, active_scans, &mut actions);
        self.header(ui, localization, collections, active_scans, &mut actions);
        if let Some(action) = self.guided_repair_confirmation(ui.ctx(), localization) {
            actions.push(action);
        }

        if let Some((error, message)) = &self.link_notice {
            ui.colored_label(
                if *error {
                    crate::theme::error_text(ui.visuals().dark_mode)
                } else {
                    crate::theme::warning_text(ui.visuals().dark_mode)
                },
                localized_knowledge_notice(localization, message),
            );
            ui.add_space(6.0);
        }

        if collections.is_empty() {
            empty_state(
                ui,
                &localization.text("knowledge-no-collections-title"),
                &localization.text("knowledge-no-collections-body"),
            );
            self.external_link_confirmation(ui.ctx(), localization);
            return actions;
        }

        if let Some(error) = &self.bundle_error {
            error_state(
                ui,
                localization,
                &localization.text("knowledge-bundle-error-title"),
                error,
            );
            self.external_link_confirmation(ui.ctx(), localization);
            return actions;
        }

        let selected_is_scanning = self
            .collection_id
            .is_some_and(|collection_id| active_scans.contains(&collection_id));
        let Some(bundle) = self.bundle.clone() else {
            if selected_is_scanning {
                empty_state(
                    ui,
                    &localization.text("knowledge-rescanning-title"),
                    &localization.text("knowledge-rescanning-body"),
                );
                self.external_link_confirmation(ui.ctx(), localization);
                return actions;
            }
            ui.centered_and_justified(|ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(localization.text("knowledge-inspecting"));
                });
            });
            self.external_link_confirmation(ui.ctx(), localization);
            return actions;
        };

        if matches!(bundle.state, KnowledgeBundleState::Empty) && self.tab != KnowledgeTab::Health {
            empty_state(
                ui,
                &localization.text("knowledge-empty-title"),
                &localization.text("knowledge-empty-body"),
            );
            self.external_link_confirmation(ui.ctx(), localization);
            return actions;
        }
        if matches!(bundle.state, KnowledgeBundleState::Updating)
            && self.tab != KnowledgeTab::Health
        {
            empty_state(
                ui,
                &localization.text("knowledge-updating-title"),
                &localization.text("knowledge-updating-body"),
            );
            self.external_link_confirmation(ui.ctx(), localization);
            return actions;
        }

        let requested_page = match self.tab {
            KnowledgeTab::Wiki => self.show_wiki(ui, localization, &bundle),
            KnowledgeTab::Graph => {
                let mut requested_page = None;
                let graph_height = ui.available_height().max(0.0);
                egui::ScrollArea::vertical()
                    .id_salt("knowledge_editorial_graph")
                    .max_height(graph_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        requested_page = self.show_graph(ui, localization, &bundle);
                        scroll_newly_focused_control_into_view(ui);
                    });
                requested_page
            }
            KnowledgeTab::Health => {
                let (page, action) = self.show_health(ui, localization, &bundle);
                if let Some(action) = action {
                    actions.push(action);
                }
                page
            }
        };
        if let Some(page_id) = requested_page
            && let Some(action) = self.request_page(page_id)
        {
            actions.push(action);
        }

        self.external_link_confirmation(ui.ctx(), localization);
        actions
    }

    fn ensure_collection(
        &mut self,
        collections: &[(Uuid, String)],
        active_scans: &HashSet<Uuid>,
        actions: &mut Vec<KnowledgeAction>,
    ) {
        let valid = self
            .collection_id
            .is_some_and(|id| collections.iter().any(|(candidate, _)| *candidate == id));
        if !valid {
            self.collection_id = collections.first().map(|(id, _)| *id);
            self.clear_snapshot();
        }
        if let Some(collection_id) = self.collection_id
            && !active_scans.contains(&collection_id)
            && self.bundle.is_none()
            && self.bundle_pending.is_none()
            && self.bundle_error.is_none()
        {
            actions.push(self.request_bundle(collection_id));
        }
        if let Some(retry_at) = self.retry_bundle_at
            && Instant::now() >= retry_at
            && self.bundle_pending.is_none()
            && let Some(collection_id) = self.collection_id
            && !active_scans.contains(&collection_id)
        {
            self.retry_bundle_at = None;
            actions.push(self.request_bundle(collection_id));
        }
    }

    fn header(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        collections: &[(Uuid, String)],
        active_scans: &HashSet<Uuid>,
        actions: &mut Vec<KnowledgeAction>,
    ) {
        let narrow = ResponsiveLayout::from_available(ui.available_size()).is_narrow();
        let refresh_enabled = self.collection_id.is_some_and(|collection_id| {
            !active_scans.contains(&collection_id) && self.bundle_pending.is_none()
        });
        let before = self.collection_id;
        let mut selected_collection = before;
        let selected_collection_name = self
            .collection_id
            .and_then(|selected| {
                collections
                    .iter()
                    .find(|(id, _)| *id == selected)
                    .map(|(_, name)| name.clone())
            })
            .unwrap_or_else(|| localization.text("knowledge-select-collection"));
        let mut refresh_requested = false;

        let title = |ui: &mut egui::Ui| {
            ui.heading(
                RichText::new(localization.text("knowledge-title"))
                    .size(32.0)
                    .family(crate::theme::semibold_font_family()),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(localization.text("knowledge-subtitle"))
                        .color(ui.visuals().weak_text_color()),
                )
                .wrap(),
            );
        };
        if narrow {
            ui.vertical(|ui| {
                title(ui);
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("knowledge_collection")
                        .width(230.0)
                        .selected_text(&selected_collection_name)
                        .show_ui(ui, |ui| {
                            for (id, name) in collections {
                                ui.selectable_value(&mut selected_collection, Some(*id), name);
                            }
                        });
                    refresh_requested = ui
                        .add_enabled(
                            refresh_enabled,
                            egui::Button::new(localization.text("action-refresh")),
                        )
                        .clicked();
                });
            });
        } else {
            ui.horizontal(|ui| {
                ui.vertical(title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    refresh_requested = ui
                        .add_enabled(
                            refresh_enabled,
                            egui::Button::new(localization.text("action-refresh")),
                        )
                        .clicked();
                    egui::ComboBox::from_id_salt("knowledge_collection")
                        .width(230.0)
                        .selected_text(&selected_collection_name)
                        .show_ui(ui, |ui| {
                            for (id, name) in collections {
                                ui.selectable_value(&mut selected_collection, Some(*id), name);
                            }
                        });
                });
            });
        }
        if refresh_requested && let Some(collection_id) = self.collection_id {
            self.clear_snapshot();
            actions.push(self.request_bundle(collection_id));
        }
        if selected_collection != before
            && let Some(collection_id) = selected_collection
            && let Some(action) =
                self.select_collection(collection_id, active_scans.contains(&collection_id))
        {
            actions.push(action);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            tab_button(
                ui,
                &mut self.tab,
                KnowledgeTab::Wiki,
                &localization.text("knowledge-tab-wiki"),
            );
            tab_button(
                ui,
                &mut self.tab,
                KnowledgeTab::Graph,
                &localization.text("knowledge-tab-graph"),
            );
            tab_button(
                ui,
                &mut self.tab,
                KnowledgeTab::Health,
                &localization.text("knowledge-tab-health"),
            );
            if let Some(bundle) = &self.bundle {
                ui.separator();
                bundle_state_badge(ui, localization, bundle);
                let mut arguments = fluent_bundle::FluentArgs::new();
                arguments.set("count", bundle.concepts.len());
                ui.label(
                    RichText::new(
                        localization.text_with("knowledge-concept-count", Some(&arguments)),
                    )
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
            }
        });
        ui.separator();
        ui.add_space(4.0);
    }

    fn show_wiki(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        bundle: &KnowledgeBundleView,
    ) -> Option<KnowledgePageId> {
        let mut requested_page = None;
        if wiki_layout_mode(ui.available_width()) == WikiLayoutMode::Compact {
            let current_label = self
                .selected_page
                .map(|page| page_label(localization, bundle, page))
                .unwrap_or_else(|| localization.text("knowledge-all-concepts"));
            egui::ComboBox::from_id_salt("knowledge_compact_concept")
                .selected_text(current_label)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            !wiki_reader_page_selected(self.selected_page),
                            localization.text("knowledge-all-concepts"),
                        )
                        .clicked()
                    {
                        requested_page = Some(KnowledgePageId::Index);
                    }
                    if page_fingerprint(bundle, KnowledgePageId::Log).is_some()
                        && ui
                            .selectable_label(
                                self.selected_page == Some(KnowledgePageId::Log),
                                localization.text("knowledge-open-bundle-log"),
                            )
                            .clicked()
                    {
                        requested_page = Some(KnowledgePageId::Log);
                    }
                    for concept in &bundle.concepts {
                        let page_id = KnowledgePageId::Concept(concept.id);
                        if ui
                            .selectable_label(self.selected_page == Some(page_id), &concept.title)
                            .clicked()
                        {
                            requested_page = Some(page_id);
                        }
                    }
                });
            ui.add_space(10.0);
            if wiki_reader_page_selected(self.selected_page) {
                let detail_height = ui.available_height().max(0.0);
                egui::ScrollArea::vertical()
                    .id_salt("knowledge_compact_detail")
                    .max_height(detail_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        requested_page =
                            requested_page.or(self.wiki_page(ui, localization, bundle));
                        ui.add_space(20.0);
                        requested_page =
                            requested_page.or(self.wiki_details(ui, localization, bundle));
                        scroll_newly_focused_control_into_view(ui);
                    });
            } else {
                let (types, tags) = filter_values(bundle);
                normalize_filter(&mut self.type_filter, &types);
                normalize_filter(&mut self.tag_filter, &tags);
                ui.add(
                    egui::TextEdit::singleline(&mut self.query_filter)
                        .desired_width(ui.available_width())
                        .hint_text(localization.text("knowledge-filter-pages")),
                );
                ui.horizontal_wrapped(|ui| {
                    filter_combo(
                        ui,
                        localization,
                        "knowledge-filter-type",
                        &mut self.type_filter,
                        &types,
                    );
                    filter_combo(
                        ui,
                        localization,
                        "knowledge-filter-tag",
                        &mut self.tag_filter,
                        &tags,
                    );
                });
                ui.add_space(8.0);
                let list_height = ui.available_height().max(0.0);
                egui::ScrollArea::vertical()
                    .id_salt("knowledge_compact_index")
                    .max_height(list_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for concept in filtered_concepts(
                            bundle,
                            &self.query_filter,
                            self.type_filter.as_deref(),
                            self.tag_filter.as_deref(),
                        ) {
                            ui.separator();
                            let page_id = KnowledgePageId::Concept(concept.id);
                            let (health, tone) = match concept_health_tone(&bundle.health, page_id)
                            {
                                ConceptHealthTone::Healthy => (
                                    localization.text("knowledge-concept-healthy"),
                                    KnowledgePillTone::Accent,
                                ),
                                ConceptHealthTone::Attention => (
                                    localization.text("knowledge-concept-attention"),
                                    KnowledgePillTone::Attention,
                                ),
                            };
                            let reviewed = concept
                                .reviewed_at
                                .as_ref()
                                .or(concept.timestamp.as_ref())
                                .map(|date| date.format("%Y-%m-%d").to_string());
                            let mut metadata = vec![bundle.collection_name.clone()];
                            if !concept.tags.is_empty() {
                                metadata.push(concept.tags.join(" · "));
                            }
                            if let Some(reviewed) = reviewed {
                                metadata.push(reviewed);
                            }
                            let metadata = metadata.join("  ·  ");
                            if knowledge_index_entry(
                                ui,
                                KnowledgeIndexEntry {
                                    id: concept.id,
                                    title: &concept.title,
                                    title_size: 19.0,
                                    concept_type: &concept.concept_type,
                                    health_label: &health,
                                    health_tone: tone,
                                    description: &concept.description,
                                    metadata: &metadata,
                                },
                            )
                            .clicked()
                            {
                                requested_page = Some(page_id);
                            }
                            ui.add_space(12.0);
                        }
                        if bundle_log_summary(ui, localization, bundle) {
                            requested_page = Some(KnowledgePageId::Log);
                        }
                        scroll_newly_focused_control_into_view(ui);
                    });
            }
        } else if wiki_reader_page_selected(self.selected_page) {
            ui.set_max_width(720.0);
            if ui
                .add(crate::theme::ghost_button(
                    format!("← {}", localization.text("knowledge-all-concepts")),
                    ui.visuals().dark_mode,
                ))
                .clicked()
            {
                self.selected_page = Some(KnowledgePageId::Index);
                self.page = None;
            } else {
                ui.add_space(8.0);
                let detail_height = ui.available_height().max(0.0);
                egui::ScrollArea::vertical()
                    .id_salt("knowledge_editorial_detail")
                    .max_height(detail_height)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.set_max_width(620.0);
                        requested_page =
                            requested_page.or(self.wiki_page(ui, localization, bundle));
                        ui.add_space(24.0);
                        requested_page =
                            requested_page.or(self.wiki_details(ui, localization, bundle));
                        scroll_newly_focused_control_into_view(ui);
                    });
            }
        } else {
            let index_height = ui.available_height().max(0.0);
            egui::ScrollArea::vertical()
                .id_salt("knowledge_wide_index")
                .max_height(index_height)
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.set_max_width(860.0);
                    let (types, tags) = filter_values(bundle);
                    normalize_filter(&mut self.type_filter, &types);
                    normalize_filter(&mut self.tag_filter, &tags);
                    let filter_controls_visible = bundle.concepts.len() > INDEX_FILTER_THRESHOLD
                        || !self.query_filter.trim().is_empty()
                        || self.type_filter.is_some()
                        || self.tag_filter.is_some();
                    if filter_controls_visible {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(crate::theme::card_kicker_job(
                                localization.text("knowledge-index-kicker"),
                                crate::theme::accent_text(ui.visuals().dark_mode),
                            ));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    filter_combo(
                                        ui,
                                        localization,
                                        "knowledge-filter-tag",
                                        &mut self.tag_filter,
                                        &tags,
                                    );
                                    filter_combo(
                                        ui,
                                        localization,
                                        "knowledge-filter-type",
                                        &mut self.type_filter,
                                        &types,
                                    );
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.query_filter)
                                            .desired_width(220.0)
                                            .hint_text(localization.text("knowledge-filter-pages")),
                                    );
                                },
                            );
                        });
                        ui.add_space(12.0);
                    }
                    let filtered = filtered_concepts(
                        bundle,
                        &self.query_filter,
                        self.type_filter.as_deref(),
                        self.tag_filter.as_deref(),
                    );
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.x = WIKI_INDEX_COLUMN_GAP;
                        ui.columns(2, |columns| {
                            for (index, concept) in filtered.into_iter().enumerate() {
                                let column = &mut columns[index % 2];
                                column.push_id(concept.id, |ui| {
                                    ui.separator();
                                    ui.add_space(8.0);
                                    let health_tone = concept_health_tone(
                                        &bundle.health,
                                        KnowledgePageId::Concept(concept.id),
                                    );
                                    let (health_label, health_tone) = match health_tone {
                                        ConceptHealthTone::Healthy => (
                                            localization.text("knowledge-concept-healthy"),
                                            KnowledgePillTone::Accent,
                                        ),
                                        ConceptHealthTone::Attention => (
                                            localization.text("knowledge-concept-attention"),
                                            KnowledgePillTone::Attention,
                                        ),
                                    };
                                    let reviewed = concept
                                        .reviewed_at
                                        .as_ref()
                                        .or(concept.timestamp.as_ref())
                                        .map(|date| date.format("%Y-%m-%d").to_string());
                                    let mut metadata = vec![bundle.collection_name.clone()];
                                    if !concept.tags.is_empty() {
                                        metadata.push(concept.tags.join(" · "));
                                    }
                                    if let Some(reviewed) = reviewed {
                                        metadata.push(reviewed);
                                    }
                                    let metadata = metadata.join("  ·  ");
                                    if knowledge_index_entry(
                                        ui,
                                        KnowledgeIndexEntry {
                                            id: concept.id,
                                            title: &concept.title,
                                            title_size: 18.0,
                                            concept_type: &concept.concept_type,
                                            health_label: &health_label,
                                            health_tone,
                                            description: &concept.description,
                                            metadata: &metadata,
                                        },
                                    )
                                    .clicked()
                                    {
                                        requested_page = Some(KnowledgePageId::Concept(concept.id));
                                    }
                                    ui.add_space(14.0);
                                });
                            }
                        });
                    });
                    if bundle_log_summary(ui, localization, bundle) {
                        requested_page = Some(KnowledgePageId::Log);
                    }
                    scroll_newly_focused_control_into_view(ui);
                });
        }
        requested_page
    }

    fn wiki_page(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        bundle: &KnowledgeBundleView,
    ) -> Option<KnowledgePageId> {
        let mut requested = None;
        ui.scope(|ui| {
            if self.page_pending.is_some() {
                ui.centered_and_justified(|ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(localization.text("knowledge-page-loading"));
                    });
                });
                return;
            }
            if let Some(error) = &self.page_error {
                error_state(
                    ui,
                    localization,
                    &localization.text("knowledge-page-error-title"),
                    error,
                );
                return;
            }
            let Some(page) = self.page.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.label(localization.text("knowledge-select-page"));
                });
                return;
            };

            if let Some(target) = self.search_evidence.as_ref().filter(|target| {
                page.page_id == KnowledgePageId::Concept(target.concept_id)
                    && page.collection_id == target.collection_id
            }) {
                let request_focus = self.search_evidence_focus_pending;
                search_evidence_trace(ui, localization, target, request_focus);
                self.search_evidence_focus_pending = false;
                ui.add_space(8.0);
            }

            let concept = match page.page_id {
                KnowledgePageId::Concept(concept_id) => bundle
                    .concepts
                    .iter()
                    .find(|concept| concept.id == concept_id),
                KnowledgePageId::Index | KnowledgePageId::Log => None,
            };
            if let Some(concept) = concept {
                let tone = concept_health_tone(&bundle.health, page.page_id);
                let (health, pill_tone) = match tone {
                    ConceptHealthTone::Healthy => (
                        localization.text("knowledge-concept-healthy"),
                        KnowledgePillTone::Accent,
                    ),
                    ConceptHealthTone::Attention => (
                        localization.text("knowledge-concept-attention"),
                        KnowledgePillTone::Attention,
                    ),
                };
                ui.horizontal_wrapped(|ui| {
                    knowledge_pill(ui, &concept.concept_type, KnowledgePillTone::Neutral);
                    knowledge_pill(ui, &health, pill_tone);
                });
            }
            ui.heading(
                RichText::new(&page.title)
                    .size(32.0)
                    .family(crate::theme::semibold_font_family()),
            );
            if let Some(concept) = concept {
                let reviewed = concept
                    .reviewed_at
                    .as_ref()
                    .or(concept.timestamp.as_ref())
                    .map(|date| date.format("%Y-%m-%d").to_string());
                let mut metadata = vec![bundle.collection_name.clone()];
                if let Some(reviewed) = reviewed {
                    metadata.push(reviewed);
                }
                if !concept.tags.is_empty() {
                    metadata.push(concept.tags.join(" · "));
                }
                ui.label(
                    RichText::new(metadata.join("  ·  "))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            }
            if page.truncated {
                ui.colored_label(
                    crate::theme::warning_text(ui.visuals().dark_mode),
                    localization.text("knowledge-page-truncated"),
                );
            }
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);

            let command_start = ui.ctx().output(|output| output.commands.len());
            let source_id = format!(
                "knowledge-markdown-{}-{}",
                page.collection_id, page.fingerprint
            );
            CommonMarkViewer::new()
                .explicit_image_uri_scheme(true)
                .enable_scroll_to_heading(true)
                .show_scrollable(source_id, ui, &mut self.markdown_cache, &page.body_markdown);
            let clicked_urls = capture_open_urls(ui.ctx(), command_start);
            for url in clicked_urls {
                if let Some(page_id) = self.handle_markdown_url(localization, bundle, &page, &url) {
                    requested = Some(page_id);
                }
            }
        });
        requested
    }

    fn wiki_details(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        bundle: &KnowledgeBundleView,
    ) -> Option<KnowledgePageId> {
        let mut requested = None;
        ui.scope(|ui| {
            let Some(page) = self.page.clone() else {
                ui.label(localization.text("knowledge-details-placeholder"));
                return;
            };
            ui.label(crate::theme::section_label_job(
                localization.text("knowledge-metadata").to_uppercase(),
                crate::theme::secondary_text(ui.visuals().dark_mode),
            ));
            let concept = match page.page_id {
                KnowledgePageId::Concept(concept_id) => bundle
                    .concepts
                    .iter()
                    .find(|concept| concept.id == concept_id),
                KnowledgePageId::Index | KnowledgePageId::Log => None,
            };
            if let Some(revision) = concept.and_then(|concept| concept.revision) {
                let mut arguments = fluent_bundle::FluentArgs::new();
                arguments.set("revision", i64::from(revision));
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            localization.text_with("knowledge-source-revision", Some(&arguments)),
                        )
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    )
                    .wrap(),
                );
            } else {
                ui.label(
                    RichText::new(localization.text("knowledge-source-metadata-unavailable"))
                        .color(ui.visuals().weak_text_color()),
                );
            }
            let additional_metadata = page
                .metadata
                .iter()
                .filter(|(key, _)| editorial_metadata_key(key))
                .collect::<Vec<_>>();
            if !additional_metadata.is_empty() {
                ui.collapsing(localization.text("knowledge-additional-metadata"), |ui| {
                    for (key, value) in additional_metadata {
                        ui.label(
                            RichText::new(key)
                                .small()
                                .family(crate::theme::semibold_font_family()),
                        );
                        ui.add(
                            egui::Label::new(RichText::new(value).small())
                                .selectable(true)
                                .wrap(),
                        );
                    }
                });
            }

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(10.0);
            let mut link_arguments = fluent_bundle::FluentArgs::new();
            link_arguments.set("count", page.outgoing_links.len());
            ui.label(crate::theme::section_label_job(
                localization
                    .text_with("knowledge-links", Some(&link_arguments))
                    .to_uppercase(),
                crate::theme::secondary_text(ui.visuals().dark_mode),
            ));
            if page.outgoing_links.is_empty() {
                ui.label(
                    RichText::new(localization.text("knowledge-no-linked-concepts"))
                        .color(ui.visuals().weak_text_color()),
                );
            }
            for link in &page.outgoing_links {
                let label = if link.label.is_empty() {
                    &link.raw_target
                } else {
                    &link.label
                };
                match &link.disposition {
                    KnowledgeLinkDisposition::Internal(target) => {
                        if ui.link(label).clicked() {
                            requested = Some(*target);
                        }
                    }
                    _ => {
                        let (status, color) =
                            link_status(localization, &link.disposition, ui.visuals().dark_mode);
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(color, status);
                            ui.label(label);
                        });
                    }
                }
            }

            ui.add_space(16.0);
            ui.separator();
            ui.add_space(10.0);
            let mut backlink_arguments = fluent_bundle::FluentArgs::new();
            backlink_arguments.set("count", page.backlinks.len());
            ui.label(crate::theme::section_label_job(
                localization
                    .text_with("knowledge-backlinks", Some(&backlink_arguments))
                    .to_uppercase(),
                crate::theme::secondary_text(ui.visuals().dark_mode),
            ));
            if page.backlinks.is_empty() {
                ui.label(
                    RichText::new(localization.text("knowledge-no-backlinks"))
                        .color(ui.visuals().weak_text_color()),
                );
            }
            for backlink in &page.backlinks {
                let label = page_label(localization, bundle, *backlink);
                if ui.link(label).clicked() {
                    requested = Some(*backlink);
                }
            }
        });
        requested
    }

    fn show_graph(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        bundle: &KnowledgeBundleView,
    ) -> Option<KnowledgePageId> {
        let (types, tags) = filter_values(bundle);
        normalize_filter(&mut self.type_filter, &types);
        normalize_filter(&mut self.tag_filter, &tags);
        let filter_controls_visible = graph_requires_filter(bundle.concepts.len())
            || !self.query_filter.trim().is_empty()
            || self.type_filter.is_some()
            || self.tag_filter.is_some();
        if filter_controls_visible {
            ui.collapsing(localization.text("knowledge-graph-filter-title"), |ui| {
                graph_filter_controls(
                    ui,
                    localization,
                    &mut self.query_filter,
                    &mut self.type_filter,
                    &mut self.tag_filter,
                    &types,
                    &tags,
                );
            });
        }

        let filtered = filtered_concepts(
            bundle,
            &self.query_filter,
            self.type_filter.as_deref(),
            self.tag_filter.as_deref(),
        );
        if graph_requires_filter(filtered.len()) {
            self.graph = None;
            let mut arguments = fluent_bundle::FluentArgs::new();
            arguments.set("count", filtered.len());
            arguments.set("limit", MAX_GRAPH_CONCEPTS);
            empty_state(
                ui,
                &localization.text("knowledge-graph-filter-title"),
                &localization.text_with("knowledge-graph-filter-body", Some(&arguments)),
            );
            return None;
        }

        self.ensure_graph(localization, bundle);
        let cache = self.graph.as_mut()?;
        let _ = cache.layout.advance(&mut cache.graph);
        let canvas_width = ui.available_width().min(720.0);
        let visible_node_limit = editorial_graph_node_limit(canvas_width);
        let graph_nodes = cache
            .graph
            .nodes_iter()
            .filter_map(|(_, node)| {
                let payload = node.payload();
                (payload.page_id != KnowledgePageId::Index).then(|| {
                    (
                        payload.page_id,
                        payload.title.clone(),
                        payload.concept_type.clone(),
                        payload.tags.clone(),
                        node.color().unwrap_or(crate::theme::AIR_CYAN),
                    )
                })
            })
            .take(visible_node_limit)
            .collect::<Vec<_>>();
        let shown_node_count = graph_nodes.len();
        let graph_edges = cache
            .graph
            .edges_iter()
            .filter_map(|(edge_index, _)| {
                let (source, target) = cache.graph.edge_endpoints(edge_index)?;
                let source = cache.graph.node(source)?.payload().page_id;
                let target = cache.graph.node(target)?.payload().page_id;
                (source != KnowledgePageId::Index && target != KnowledgePageId::Index)
                    .then_some((source, target))
            })
            .collect::<Vec<_>>();

        let canvas_height = if canvas_width < 620.0 { 340.0 } else { 400.0 };
        let (canvas, _) = ui.allocate_exact_size(
            egui::vec2(canvas_width, canvas_height),
            egui::Sense::hover(),
        );
        let positions = graph_nodes
            .iter()
            .enumerate()
            .map(|(ordinal, (page_id, ..))| {
                (
                    *page_id,
                    editorial_graph_position(canvas, ordinal, graph_nodes.len()),
                )
            })
            .collect::<HashMap<_, _>>();

        for (source, target) in graph_edges {
            let (Some(source), Some(target)) = (positions.get(&source), positions.get(&target))
            else {
                continue;
            };
            ui.painter().line_segment(
                [*source, *target],
                egui::Stroke::new(1.0, crate::theme::border(ui.visuals().dark_mode)),
            );
        }

        let mut graph_page = None;
        for (page_id, title, concept_type, tags, color) in graph_nodes {
            let Some(position) = positions.get(&page_id).copied() else {
                continue;
            };
            let hit_rect = egui::Rect::from_center_size(
                position + egui::vec2(0.0, 20.0),
                egui::vec2(150.0, 62.0),
            );
            let response = ui.interact(
                hit_rect,
                ui.id().with(("graph_node", page_id)),
                egui::Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("{title}, {concept_type}"),
                )
            });
            if response.clicked() {
                graph_page = Some(page_id);
            }
            if response.has_focus() {
                ui.painter().rect_stroke(
                    hit_rect,
                    1.0,
                    egui::Stroke::new(2.0, crate::theme::AIR_CYAN),
                    egui::StrokeKind::Inside,
                );
            }
            ui.painter().circle_filled(position, 8.0, color);
            ui.painter().text(
                position + egui::vec2(0.0, 14.0),
                egui::Align2::CENTER_TOP,
                truncate_chars(&title, 28),
                egui::FontId::new(13.0, crate::theme::semibold_font_family()),
                crate::theme::ink(ui.visuals().dark_mode),
            );
            ui.painter().text(
                position + egui::vec2(0.0, 31.0),
                egui::Align2::CENTER_TOP,
                truncate_chars(&concept_type, 24),
                egui::FontId::proportional(12.0),
                crate::theme::secondary_text(ui.visuals().dark_mode),
            );
            response.on_hover_ui(|ui| {
                ui.label(RichText::new(&title).family(crate::theme::semibold_font_family()));
                ui.label(&concept_type);
                if !tags.is_empty() {
                    ui.label(
                        RichText::new(tags.join(", "))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                }
            });
        }

        ui.add_space(8.0);
        ui.label(
            RichText::new(localization.text("knowledge-graph-caption"))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
        if filtered.len() > shown_node_count {
            let mut arguments = fluent_bundle::FluentArgs::new();
            arguments.set("shown", shown_node_count);
            arguments.set("total", filtered.len());
            ui.label(
                RichText::new(
                    localization.text_with("knowledge-graph-visible-limit", Some(&arguments)),
                )
                .small()
                .color(ui.visuals().weak_text_color()),
            );
        }
        let mut keyboard_page = None;
        ui.collapsing(localization.text("knowledge-graph-node-list"), |ui| {
            for concept in filtered_concepts(
                bundle,
                &self.query_filter,
                self.type_filter.as_deref(),
                self.tag_filter.as_deref(),
            ) {
                if ui.link(&concept.title).clicked() {
                    keyboard_page = Some(KnowledgePageId::Concept(concept.id));
                }
            }
        });

        let page_id = keyboard_page.or(graph_page);
        let page_id = page_id.filter(|page_id| page_fingerprint(bundle, *page_id).is_some());
        if page_id.is_some() {
            self.tab = KnowledgeTab::Wiki;
        }
        page_id
    }

    fn show_health(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        bundle: &KnowledgeBundleView,
    ) -> (Option<KnowledgePageId>, Option<KnowledgeAction>) {
        if matches!(bundle.state, KnowledgeBundleState::Updating) {
            empty_state(
                ui,
                &localization.text("knowledge-health-updating-title"),
                &localization.text("knowledge-health-updating-body"),
            );
            return (None, None);
        }
        let report = &bundle.health;
        if matches!(bundle.state, KnowledgeBundleState::Empty)
            && !empty_bundle_has_health_findings(bundle)
        {
            empty_state(
                ui,
                &localization.text("knowledge-health-empty-title"),
                &localization.text("knowledge-health-empty-body"),
            );
            return (None, None);
        }
        if matches!(bundle.state, KnowledgeBundleState::Empty) {
            ui.colored_label(
                crate::theme::warning_text(ui.visuals().dark_mode),
                localization.text("knowledge-health-empty-warning"),
            );
            ui.add_space(8.0);
        }
        let mut requested = None;
        ui.horizontal(|ui| {
            health_card(
                ui,
                &localization.text("knowledge-health-concepts"),
                report.total_concepts,
                crate::theme::accent_text(ui.visuals().dark_mode),
            );
            health_card(
                ui,
                &localization.text("knowledge-health-warnings"),
                report.warning_count,
                crate::theme::warning_text(ui.visuals().dark_mode),
            );
            health_card(
                ui,
                &localization.text("knowledge-health-errors"),
                report.error_count,
                crate::theme::error_text(ui.visuals().dark_mode),
            );
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(localization.text("knowledge-health-last-check"))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                ui.label(
                    report
                        .checked_at
                        .format("%Y-%m-%d %H:%M:%S UTC")
                        .to_string(),
                );
            });
        });
        ui.add_space(10.0);
        let history_recovery = health_requires_history_recovery(bundle);
        let content_repair = health_has_guided_content_repair(bundle);
        let manual_recovery = health_has_manual_intervention(bundle);
        let guided_repair_available = content_repair && !history_recovery && !manual_recovery;
        if history_recovery {
            ui.colored_label(
                crate::theme::warning_text(ui.visuals().dark_mode),
                localization.text("knowledge-repair-history-blocked"),
            );
            ui.label(
                RichText::new(localization.text("knowledge-repair-history-body"))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        }
        if manual_recovery {
            ui.colored_label(
                crate::theme::warning_text(ui.visuals().dark_mode),
                localization.text("knowledge-repair-manual-title"),
            );
            ui.label(
                RichText::new(localization.text("knowledge-repair-manual-body"))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        }
        if let Some((_, error)) = self
            .guided_repair_error
            .as_ref()
            .filter(|(collection_id, _)| *collection_id == bundle.collection_id)
        {
            ui.colored_label(
                crate::theme::error_text(ui.visuals().dark_mode),
                localized_guided_repair_error(localization, error),
            );
            ui.collapsing(localization.text("action-details"), |ui| {
                wrap_monospace(ui, sanitized_knowledge_error_code(error));
            });
        }
        if let Some(result) = self
            .guided_repair_result
            .as_ref()
            .filter(|result| result.collection_id == bundle.collection_id)
        {
            let mut arguments = fluent_bundle::FluentArgs::new();
            arguments.set("reviewed", result.concepts_returned_to_review.len());
            arguments.set("orphans", result.orphan_concepts_removed.len());
            ui.colored_label(
                crate::theme::verified_text(ui.visuals().dark_mode),
                localization.text_with("knowledge-repair-complete", Some(&arguments)),
            );
        }
        let mut repair_action = None;
        if guided_repair_available {
            if self.guided_repair_prepare_pending.is_some()
                || self.guided_repair_execute_pending.is_some()
            {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(localization.text("knowledge-repair-working"));
                });
            } else if ui
                .button(localization.text("knowledge-repair-review-action"))
                .clicked()
            {
                repair_action = Some(self.begin_guided_repair(bundle.collection_id));
            }
            ui.label(
                RichText::new(localization.text("knowledge-repair-review-help"))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        }
        ui.add_space(10.0);
        ui.separator();
        let mut issue_arguments = fluent_bundle::FluentArgs::new();
        issue_arguments.set("count", report.issues.len());
        ui.heading(localization.text_with("knowledge-health-findings", Some(&issue_arguments)));
        if report.issues.is_empty() {
            empty_state(
                ui,
                &localization.text("knowledge-health-ready-title"),
                &localization.text("knowledge-health-ready-body"),
            );
            return (None, repair_action);
        }

        egui::ScrollArea::vertical()
            .id_salt("knowledge_health")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                for issue in &report.issues {
                    crate::theme::surface_frame(ui.visuals().dark_mode).show(ui, |ui| {
                        let (label, color) =
                            severity_visual(localization, &issue.severity, ui.visuals().dark_mode);
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                color,
                                RichText::new(label).family(crate::theme::semibold_font_family()),
                            );
                            if let Some(page_id) = issue.page {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let label = page_label(localization, bundle, page_id);
                                        if health_issue_page_available(bundle, page_id) {
                                            if ui.link(label).clicked() {
                                                requested = Some(page_id);
                                            }
                                        } else {
                                            ui.label(
                                                RichText::new(label)
                                                    .small()
                                                    .color(ui.visuals().weak_text_color()),
                                            )
                                            .on_hover_text(
                                                localization.text("knowledge-page-missing"),
                                            );
                                        }
                                    },
                                );
                            }
                        });
                        ui.label(health_issue_summary(localization, &issue.code));
                        ui.label(
                            RichText::new(localization.text(health_recovery_message_id(
                                issue.recovery(),
                                guided_repair_available,
                            )))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                        );
                        ui.collapsing(localization.text("action-details"), |ui| {
                            wrap_monospace(ui, &issue.code);
                            ui.label(&issue.message);
                        });
                    });
                    ui.add_space(6.0);
                }
            });
        if requested.is_some() {
            self.tab = KnowledgeTab::Wiki;
        }
        (requested, repair_action)
    }

    fn begin_guided_repair(&mut self, collection_id: Uuid) -> KnowledgeAction {
        let request_id = Uuid::new_v4();
        self.guided_repair_prepare_pending = Some(PendingBundle {
            request_id,
            collection_id,
        });
        self.guided_repair_preview = None;
        self.guided_repair_error = None;
        self.guided_repair_result = None;
        KnowledgeAction::PrepareGuidedRepair {
            request_id,
            collection_id,
        }
    }

    fn guided_repair_confirmation(
        &mut self,
        context: &egui::Context,
        localization: &Localization,
    ) -> Option<KnowledgeAction> {
        let preview = self.guided_repair_preview.clone()?;
        let executing = self.guided_repair_execute_pending.is_some();
        let mut cancel = false;
        let mut confirm = false;
        egui::Window::new(localization.text("knowledge-repair-confirm-title"))
            .id(egui::Id::new("knowledge_guided_repair_confirmation"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .show(context, |ui| {
                ui.colored_label(
                    crate::theme::warning_text(ui.visuals().dark_mode),
                    RichText::new(localization.text("knowledge-repair-confirm-warning"))
                        .family(crate::theme::semibold_font_family()),
                );
                ui.label(localization.text("knowledge-repair-confirm-body"));
                ui.add_space(8.0);
                ui.label(
                    RichText::new(localization.text("knowledge-repair-authority-title"))
                        .family(crate::theme::semibold_font_family()),
                );
                for authority in &preview.authorities {
                    ui.label(format!(
                        "• {}",
                        localized_repair_authority(localization, *authority)
                    ));
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(localization.text("knowledge-repair-changes-title"))
                        .family(crate::theme::semibold_font_family()),
                );
                egui::ScrollArea::vertical()
                    .id_salt("knowledge_guided_repair_files")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for file in &preview.files {
                            ui.horizontal_wrapped(|ui| {
                                wrap_monospace(ui, file.page.relative_path());
                                ui.label("—");
                                ui.label(localized_repair_change(localization, file.change));
                            });
                        }
                    });
                ui.add_space(8.0);
                ui.label(
                    RichText::new(localization.text("knowledge-repair-snapshot-note"))
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                if executing {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(localization.text("knowledge-repair-working"));
                    });
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !executing,
                            egui::Button::new(localization.text("action-cancel")),
                        )
                        .clicked()
                    {
                        cancel = true;
                    }
                    if ui
                        .add_enabled(
                            !executing,
                            egui::Button::new(localization.text("knowledge-repair-confirm-action")),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });
        if cancel {
            self.cancel_guided_repair_preview();
            return None;
        }
        if !confirm {
            return None;
        }
        self.confirm_guided_repair_preview()
    }

    fn cancel_guided_repair_preview(&mut self) {
        if self.guided_repair_execute_pending.is_none() {
            self.guided_repair_preview = None;
        }
    }

    fn confirm_guided_repair_preview(&mut self) -> Option<KnowledgeAction> {
        if self.guided_repair_execute_pending.is_some() {
            return None;
        }
        let preview = self.guided_repair_preview.clone()?;
        let request_id = Uuid::new_v4();
        self.guided_repair_execute_pending = Some(PendingBundle {
            request_id,
            collection_id: preview.collection_id,
        });
        Some(KnowledgeAction::ExecuteGuidedRepair {
            request_id,
            preview,
        })
    }

    fn handle_markdown_url(
        &mut self,
        localization: &Localization,
        _bundle: &KnowledgeBundleView,
        page: &KnowledgePageView,
        url: &str,
    ) -> Option<KnowledgePageId> {
        if let Some(link) = page
            .outgoing_links
            .iter()
            .find(|link| link.raw_target == url)
        {
            match &link.disposition {
                KnowledgeLinkDisposition::Internal(page_id) => return Some(*page_id),
                KnowledgeLinkDisposition::Broken => {
                    self.link_notice = Some((
                        true,
                        localized_url_notice(localization, "knowledge-link-broken", url),
                    ));
                    return None;
                }
                KnowledgeLinkDisposition::Unsafe => {
                    self.link_notice = Some((
                        true,
                        localized_url_notice(localization, "knowledge-link-unsafe", url),
                    ));
                    return None;
                }
                KnowledgeLinkDisposition::External => {}
            }
        }

        if let Some(url) = normalized_http_url(url) {
            self.pending_external_url = Some(url.to_owned());
            self.link_notice = None;
        } else {
            self.link_notice = Some((
                true,
                localized_url_notice(localization, "knowledge-link-disallowed", url),
            ));
        }
        None
    }

    fn external_link_confirmation(&mut self, context: &egui::Context, localization: &Localization) {
        let Some(url) = self.pending_external_url.clone() else {
            return;
        };
        egui::Window::new(localization.text("knowledge-external-title"))
            .id(egui::Id::new("knowledge_external_link_confirmation"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(localization.text("knowledge-external-warning"));
                ui.add(
                    egui::Label::new(RichText::new(&url).monospace())
                        .selectable(true)
                        .wrap(),
                );
                ui.horizontal(|ui| {
                    if ui.button(localization.text("action-cancel")).clicked() {
                        self.pending_external_url = None;
                    }
                    if ui
                        .button(localization.text("knowledge-open-browser"))
                        .clicked()
                    {
                        context.open_url(egui::OpenUrl::new_tab(&url));
                        self.pending_external_url = None;
                    }
                });
            });
    }

    fn ensure_graph(&mut self, localization: &Localization, bundle: &KnowledgeBundleView) {
        let key = format!(
            "{}:{}:{}:{}:{}",
            bundle.collection_id,
            bundle.fingerprint,
            self.query_filter.trim().to_lowercase(),
            self.type_filter.as_deref().unwrap_or(""),
            self.tag_filter.as_deref().unwrap_or("")
        );
        if self.graph.as_ref().is_some_and(|cache| cache.key == key) {
            return;
        }
        self.graph = Some(build_graph(
            localization,
            bundle,
            key,
            &self.query_filter,
            self.type_filter.as_deref(),
            self.tag_filter.as_deref(),
        ));
    }

    fn request_bundle(&mut self, collection_id: Uuid) -> KnowledgeAction {
        let request_id = Uuid::new_v4();
        self.bundle_pending = Some(PendingBundle {
            request_id,
            collection_id,
        });
        self.bundle_error = None;
        KnowledgeAction::LoadBundle {
            request_id,
            collection_id,
        }
    }

    fn select_collection(
        &mut self,
        collection_id: Uuid,
        scan_active: bool,
    ) -> Option<KnowledgeAction> {
        if self.collection_id == Some(collection_id) {
            return None;
        }
        self.guided_repair_prepare_pending = None;
        self.guided_repair_preview = None;
        self.guided_repair_error = None;
        self.guided_repair_result = None;
        self.collection_id = Some(collection_id);
        self.clear_snapshot();
        (!scan_active).then(|| self.request_bundle(collection_id))
    }

    fn request_page(&mut self, page_id: KnowledgePageId) -> Option<KnowledgeAction> {
        self.search_evidence = None;
        self.search_evidence_focus_pending = false;
        self.link_notice = None;
        self.page_recovery_attempted = false;
        self.queue_page(page_id)
    }

    fn queue_verified_search_evidence(&mut self) -> Option<KnowledgeAction> {
        let target = self.search_evidence.clone()?;
        let matches = self.bundle.as_ref().is_some_and(|bundle| {
            bundle.collection_id == target.collection_id
                && bundle.concepts.iter().any(|concept| {
                    concept.id == target.concept_id
                        && concept.revision == Some(target.source_revision)
                        && concept.source_sha256.as_deref() == Some(target.source_sha256.as_str())
                        && concept.resource.as_deref() == Some(target.logical_resource_uri.as_str())
                })
        });
        if !matches {
            self.selected_page = None;
            self.page = None;
            self.page_pending = None;
            self.page_error = None;
            self.search_evidence = None;
            self.search_evidence_focus_pending = false;
            self.link_notice = Some((true, "knowledge-search-evidence-stale".to_owned()));
            return None;
        }

        self.queue_page(KnowledgePageId::Concept(target.concept_id))
    }

    fn queue_page(&mut self, page_id: KnowledgePageId) -> Option<KnowledgeAction> {
        let bundle = self.bundle.as_ref()?;
        let collection_id = bundle.collection_id;
        let expected_fingerprint = page_fingerprint(bundle, page_id)?.to_owned();
        let request_id = Uuid::new_v4();
        self.selected_page = Some(page_id);
        self.page = None;
        self.page_error = None;
        self.page_pending = Some(PendingPage {
            request_id,
            collection_id,
            page_id,
            expected_fingerprint: expected_fingerprint.clone(),
        });
        Some(KnowledgeAction::LoadPage {
            request_id,
            collection_id,
            page_id,
            expected_fingerprint,
        })
    }

    fn recover_page_after_stale(
        &mut self,
        collection_id: Uuid,
        error: String,
    ) -> Option<KnowledgeAction> {
        if self.page_recovery_attempted {
            self.page = None;
            self.page_error = Some(error);
            self.link_notice = None;
            return None;
        }

        self.page_recovery_attempted = true;
        self.invalidate_snapshot_preserving_selection();
        self.snapshot_stale = true;
        self.link_notice = Some((false, "knowledge-snapshot-changed".to_owned()));
        Some(self.request_bundle(collection_id))
    }

    fn invalidate_snapshot_preserving_selection(&mut self) {
        self.bundle = None;
        self.bundle_pending = None;
        self.bundle_error = None;
        self.page = None;
        self.page_pending = None;
        self.page_error = None;
        self.graph = None;
        self.retry_bundle_at = None;
        self.pending_external_url = None;
    }

    fn clear_snapshot(&mut self) {
        self.invalidate_snapshot_preserving_selection();
        self.selected_page = None;
        self.search_evidence = None;
        self.search_evidence_focus_pending = false;
        self.link_notice = None;
        self.snapshot_stale = false;
        self.page_recovery_attempted = false;
    }
}

fn build_graph(
    localization: &Localization,
    bundle: &KnowledgeBundleView,
    key: String,
    query: &str,
    concept_type: Option<&str>,
    tag: Option<&str>,
) -> KnowledgeGraphCache {
    let mut graph: KnowledgeGraph = Graph::new(Default::default());
    let mut nodes = HashMap::new();

    let index = graph.add_node_with_label(
        KnowledgeGraphNode {
            page_id: KnowledgePageId::Index,
            title: localization.text("knowledge-index-title"),
            concept_type: localization.text("knowledge-index-type"),
            tags: Vec::new(),
        },
        "index.md".to_owned(),
    );
    graph
        .node_mut(index)
        .expect("new graph node exists")
        .set_color(crate::theme::AIR_CYAN);
    nodes.insert(KnowledgePageId::Index, index);

    let filtered = filtered_concepts(bundle, query, concept_type, tag);
    debug_assert!(filtered.len() <= MAX_GRAPH_CONCEPTS);
    for concept in filtered {
        let page_id = KnowledgePageId::Concept(concept.id);
        let node = graph.add_node_with_label(
            KnowledgeGraphNode {
                page_id,
                title: concept.title.clone(),
                concept_type: concept.concept_type.clone(),
                tags: concept.tags.clone(),
            },
            truncate_chars(&concept.title, 56),
        );
        graph
            .node_mut(node)
            .expect("new graph node exists")
            .set_color(concept_graph_color(bundle, page_id));
        nodes.insert(page_id, node);
    }

    for link in &bundle.links {
        let KnowledgeLinkDisposition::Internal(target) = &link.disposition else {
            continue;
        };
        let (Some(source_node), Some(target_node)) = (nodes.get(&link.source), nodes.get(target))
        else {
            continue;
        };
        let label = truncate_chars(&link.label, 40);
        graph.add_edge_with_label(*source_node, *target_node, (), label);
    }

    KnowledgeGraphCache {
        key,
        graph,
        layout: IncrementalGraphLayout::default(),
    }
}

fn editorial_graph_position(canvas: egui::Rect, ordinal: usize, node_count: usize) -> egui::Pos2 {
    const REFERENCE_POSITIONS: [(f32, f32); 5] = [
        (0.44, 0.20),
        (0.20, 0.58),
        (0.72, 0.52),
        (0.83, 0.18),
        (0.46, 0.82),
    ];
    let normalized = if canvas.width() >= 620.0 && node_count <= REFERENCE_POSITIONS.len() {
        REFERENCE_POSITIONS[ordinal]
    } else {
        let columns = (node_count as f32).sqrt().ceil() as usize;
        let row = ordinal / columns;
        let column = ordinal % columns;
        let row_count = node_count.div_ceil(columns);
        (
            (column as f32 + 0.5) / columns as f32,
            (row as f32 + 0.5) / row_count as f32,
        )
    };
    egui::pos2(
        egui::lerp(canvas.x_range(), normalized.0)
            .clamp(canvas.left() + 75.0, canvas.right() - 75.0),
        egui::lerp(canvas.y_range(), normalized.1)
            .clamp(canvas.top() + 20.0, canvas.bottom() - 60.0),
    )
}

fn editorial_graph_node_limit(canvas_width: f32) -> usize {
    if canvas_width < 620.0 {
        4
    } else {
        MAX_EDITORIAL_GRAPH_NODES
    }
}

fn deterministic_graph_position(ordinal: usize, total_nodes: usize) -> egui::Pos2 {
    if ordinal == 0 {
        return egui::pos2(0.0, 0.0);
    }

    let concept_count = total_nodes.saturating_sub(1).max(1);
    let columns = (concept_count as f32).sqrt().ceil() as usize;
    let concept_ordinal = ordinal - 1;
    let row = concept_ordinal / columns;
    let column = concept_ordinal % columns;
    let centered_column = column as f32 - (columns.saturating_sub(1) as f32 / 2.0);
    egui::pos2(centered_column * 145.0, 120.0 + row as f32 * 90.0)
}

fn graph_requires_filter(filtered_concepts: usize) -> bool {
    filtered_concepts > MAX_GRAPH_CONCEPTS
}

fn scroll_newly_focused_control_into_view(ui: &egui::Ui) {
    let response = ui
        .memory(|memory| memory.focused())
        .and_then(|focused| ui.ctx().read_response(focused));
    if let Some(response) = response
        && response.gained_focus()
        && !ui.clip_rect().contains_rect(response.rect)
    {
        response.scroll_to_me(None);
    }
}

fn filtered_concepts<'a>(
    bundle: &'a KnowledgeBundleView,
    query: &str,
    concept_type: Option<&str>,
    tag: Option<&str>,
) -> Vec<&'a KnowledgeConceptView> {
    let query = query.trim().to_lowercase();
    let mut concepts = bundle
        .concepts
        .iter()
        .filter(|concept| {
            let type_matches =
                concept_type.is_none_or(|value| concept.concept_type.as_str() == value);
            let tag_matches = tag.is_none_or(|value| concept.tags.iter().any(|tag| tag == value));
            let query_matches = query.is_empty()
                || concept.title.to_lowercase().contains(&query)
                || concept.description.to_lowercase().contains(&query)
                || concept
                    .resource
                    .as_deref()
                    .is_some_and(|resource| resource.to_lowercase().contains(&query))
                || concept
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&query));
            type_matches && tag_matches && query_matches
        })
        .collect::<Vec<_>>();
    concepts.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    concepts
}

fn filter_values(bundle: &KnowledgeBundleView) -> (BTreeSet<String>, BTreeSet<String>) {
    let types = bundle
        .concepts
        .iter()
        .map(|concept| concept.concept_type.clone())
        .collect();
    let tags = bundle
        .concepts
        .iter()
        .flat_map(|concept| concept.tags.iter().cloned())
        .collect();
    (types, tags)
}

fn normalize_filter(selected: &mut Option<String>, values: &BTreeSet<String>) {
    if selected
        .as_ref()
        .is_some_and(|selected| !values.contains(selected))
    {
        *selected = None;
    }
}

fn page_fingerprint(bundle: &KnowledgeBundleView, page_id: KnowledgePageId) -> Option<&str> {
    bundle.page_fingerprint(page_id)
}

const fn wiki_reader_page_selected(page_id: Option<KnowledgePageId>) -> bool {
    matches!(
        page_id,
        Some(KnowledgePageId::Concept(_) | KnowledgePageId::Log)
    )
}

fn default_page(bundle: &KnowledgeBundleView) -> Option<KnowledgePageId> {
    [KnowledgePageId::Index, KnowledgePageId::Log]
        .into_iter()
        .find(|page_id| page_fingerprint(bundle, *page_id).is_some())
        .or_else(|| {
            bundle
                .concepts
                .first()
                .map(|concept| KnowledgePageId::Concept(concept.id))
        })
}

fn page_label(
    localization: &Localization,
    bundle: &KnowledgeBundleView,
    page_id: KnowledgePageId,
) -> String {
    match page_id {
        KnowledgePageId::Index => "index.md".to_owned(),
        KnowledgePageId::Log => "log.md".to_owned(),
        KnowledgePageId::Concept(id) => bundle
            .concepts
            .iter()
            .find(|concept| concept.id == id)
            .map(|concept| concept.title.clone())
            .unwrap_or_else(|| {
                let mut arguments = fluent_bundle::FluentArgs::new();
                arguments.set("id", id.to_string());
                localization.text_with("knowledge-concept-fallback", Some(&arguments))
            }),
    }
}

fn capture_open_urls(context: &egui::Context, command_start: usize) -> Vec<String> {
    context.output_mut(|output| {
        let split_at = command_start.min(output.commands.len());
        let commands = output.commands.split_off(split_at);
        let mut urls = Vec::new();
        for command in commands {
            match command {
                egui::OutputCommand::OpenUrl(open) => urls.push(open.url),
                other => output.commands.push(other),
            }
        }
        urls
    })
}

fn normalized_http_url(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    (trimmed == value
        && !value.chars().any(char::is_control)
        && (lower.starts_with("https://") || lower.starts_with("http://")))
    .then_some(trimmed)
}

fn localized_knowledge_error(localization: &Localization, error: &str) -> String {
    match error {
        "knowledge-error-wrong-collection" => localization.text("knowledge-error-wrong-collection"),
        _ => localization.text("knowledge-error-generic"),
    }
}

fn localized_knowledge_notice(localization: &Localization, notice: &str) -> String {
    match notice {
        "knowledge-snapshot-changed" => localization.text("knowledge-snapshot-changed"),
        "knowledge-search-evidence-stale" => localization.text("knowledge-search-evidence-stale"),
        _ => notice.to_owned(),
    }
}

fn search_evidence_trace(
    ui: &mut egui::Ui,
    localization: &Localization,
    target: &SearchEvidenceTarget,
    request_focus: bool,
) {
    egui::Frame::new()
        .fill(crate::theme::accent_tint(ui.visuals().dark_mode))
        .stroke(egui::Stroke::new(1.0, crate::theme::AIR_BLUE))
        .corner_radius(egui::CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            let title = ui.add(
                egui::Label::new(
                    RichText::new(localization.text("knowledge-search-evidence-title"))
                        .family(crate::theme::semibold_font_family())
                        .color(crate::theme::accent_text(ui.visuals().dark_mode)),
                )
                .sense(egui::Sense::focusable_noninteractive()),
            );
            if request_focus {
                title.request_focus();
            }
            let fallback = localization.text("knowledge-search-evidence-location-unknown");
            let location = if target.heading_or_page.trim().is_empty() {
                fallback.as_str()
            } else {
                target.heading_or_page.as_str()
            };
            let mut arguments = fluent_bundle::FluentArgs::new();
            arguments.set("location", location);
            arguments.set("revision", target.source_revision);
            ui.label(localization.text_with("knowledge-search-evidence-locator", Some(&arguments)));
            ui.label(
                RichText::new(localization.text("knowledge-search-evidence-help"))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        });
}

fn localized_url_notice(localization: &Localization, message_id: &str, url: &str) -> String {
    let mut arguments = fluent_bundle::FluentArgs::new();
    arguments.set("url", url);
    localization.text_with(message_id, Some(&arguments))
}

fn health_issue_summary(localization: &Localization, code: &str) -> String {
    let message_id = if code.contains("unsafe") || code.contains("traversal") {
        "knowledge-health-issue-unsafe"
    } else if code.contains("broken") {
        "knowledge-health-issue-broken-link"
    } else if code.contains("metadata") || code.contains("frontmatter") {
        "knowledge-health-issue-metadata"
    } else if code.contains("orphan") {
        "knowledge-health-issue-orphan"
    } else if code.contains("log") || code.contains("history") {
        "knowledge-health-issue-history"
    } else if code.contains("missing") {
        "knowledge-health-issue-missing"
    } else {
        "knowledge-health-issue-generic"
    };
    localization.text(message_id)
}

fn health_requires_history_recovery(bundle: &KnowledgeBundleView) -> bool {
    bundle.health.issues.iter().any(|issue| {
        issue.severity != HealthSeverity::Info && issue.recovery() == HealthRecovery::ManualHistory
    })
}

fn health_has_guided_content_repair(bundle: &KnowledgeBundleView) -> bool {
    bundle.health.issues.iter().any(|issue| {
        issue.severity != HealthSeverity::Info && issue.recovery() == HealthRecovery::GuidedContent
    })
}

fn health_has_manual_intervention(bundle: &KnowledgeBundleView) -> bool {
    bundle.health.issues.iter().any(|issue| {
        issue.severity != HealthSeverity::Info
            && issue.recovery() == HealthRecovery::ManualIntervention
    })
}

fn health_recovery_message_id(
    recovery: HealthRecovery,
    guided_repair_available: bool,
) -> &'static str {
    match recovery {
        HealthRecovery::AutomaticDerived => "knowledge-recovery-automatic",
        HealthRecovery::GuidedContent if guided_repair_available => "knowledge-recovery-guided",
        HealthRecovery::GuidedContent => "knowledge-recovery-guided-blocked",
        HealthRecovery::ManualHistory => "knowledge-recovery-history",
        HealthRecovery::ManualIntervention => "knowledge-recovery-manual",
        HealthRecovery::Informational => "knowledge-recovery-informational",
    }
}

fn health_issue_page_available(bundle: &KnowledgeBundleView, page_id: KnowledgePageId) -> bool {
    page_fingerprint(bundle, page_id).is_some()
}

fn localized_guided_repair_error(localization: &Localization, code: &str) -> String {
    let message_id = match code {
        "wiki_repair_history_requires_human" => "knowledge-repair-error-history",
        "wiki_repair_bundle_updating" => "knowledge-repair-error-updating",
        "wiki_repair_stale_preview" => "knowledge-repair-error-stale",
        "wiki_repair_confirmation_required" => "knowledge-repair-error-confirmation",
        "wiki_repair_unresolved_scope" => "knowledge-repair-error-unresolved",
        "wiki_repair_unsafe_layout" => "knowledge-repair-error-layout",
        "wiki_repair_snapshot_too_large" => "knowledge-repair-error-snapshot-large",
        "wiki_repair_post_validation_failed" => "knowledge-repair-error-validation",
        "wiki_repair_rollback_failed" => "knowledge-repair-error-rollback",
        "wiki_repair_operation_in_progress" => "knowledge-repair-error-busy",
        "wiki_repair_worker_panicked" => "knowledge-repair-error-worker",
        "wiki_repair_preview_wrong_collection" | "wiki_repair_result_wrong_collection" => {
            "knowledge-repair-error-stale"
        }
        _ => "knowledge-repair-error-generic",
    };
    localization.text(message_id)
}

fn localized_repair_authority(localization: &Localization, authority: RepairAuthority) -> String {
    localization.text(match authority {
        RepairAuthority::HumanReview => "knowledge-repair-authority-review",
        RepairAuthority::PublishedDatabase => "knowledge-repair-authority-database",
    })
}

fn localized_repair_change(localization: &Localization, change: GuidedRepairChange) -> String {
    localization.text(match change {
        GuidedRepairChange::WithdrawConcept => "knowledge-repair-change-withdraw",
        GuidedRepairChange::RemoveOrphan => "knowledge-repair-change-orphan",
        GuidedRepairChange::RegenerateIndex => "knowledge-repair-change-index",
        GuidedRepairChange::AppendDeprecationHistory => "knowledge-repair-change-history",
    })
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn tab_button(ui: &mut egui::Ui, selected: &mut KnowledgeTab, value: KnowledgeTab, label: &str) {
    let is_selected = *selected == value;
    let width = ((label.chars().count() as f32 * 8.0) + 24.0).max(64.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 34.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, is_selected, label)
    });
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    if is_selected {
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 1.0,
            egui::Stroke::new(2.0, crate::theme::accent_text(ui.visuals().dark_mode)),
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            1.0,
            egui::Stroke::new(2.0, crate::theme::AIR_CYAN),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::new(13.0, crate::theme::semibold_font_family()),
        if is_selected {
            crate::theme::accent_text(ui.visuals().dark_mode)
        } else {
            crate::theme::secondary_text(ui.visuals().dark_mode)
        },
    );
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    if response.clicked() {
        *selected = value;
    }
}

fn bundle_log_summary(
    ui: &mut egui::Ui,
    localization: &Localization,
    bundle: &KnowledgeBundleView,
) -> bool {
    ui.add_space(28.0);
    ui.separator();
    ui.add_space(12.0);
    ui.label(crate::theme::section_label_job(
        localization.text("knowledge-last-checked").to_uppercase(),
        crate::theme::secondary_text(ui.visuals().dark_mode),
    ));
    ui.label(
        RichText::new(format!(
            "{} · {}",
            bundle.collection_name,
            bundle.health.checked_at.format("%Y-%m-%d %H:%M")
        ))
        .small()
        .color(ui.visuals().weak_text_color()),
    );
    page_fingerprint(bundle, KnowledgePageId::Log).is_some()
        && ui
            .button(localization.text("knowledge-open-bundle-log"))
            .clicked()
}

struct KnowledgeIndexEntry<'a> {
    id: Uuid,
    title: &'a str,
    title_size: f32,
    concept_type: &'a str,
    health_label: &'a str,
    health_tone: KnowledgePillTone,
    description: &'a str,
    metadata: &'a str,
}

fn knowledge_index_entry(ui: &mut egui::Ui, entry: KnowledgeIndexEntry<'_>) -> egui::Response {
    let dark_mode = ui.visuals().dark_mode;
    let content = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(4, 7))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    knowledge_pill(ui, entry.health_label, entry.health_tone);
                    knowledge_pill(ui, entry.concept_type, KnowledgePillTone::Neutral);
                    ui.add_sized(
                        [ui.available_width().max(0.0), 28.0],
                        egui::Label::new(
                            RichText::new(entry.title)
                                .size(entry.title_size)
                                .family(crate::theme::semibold_font_family()),
                        )
                        .truncate()
                        .halign(egui::Align::LEFT)
                        .selectable(false),
                    );
                });
            });
            if !entry.description.is_empty() {
                ui.add(egui::Label::new(entry.description).wrap().selectable(false));
            }
            ui.label(
                RichText::new(entry.metadata)
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        });
    let rect = content.response.rect;
    let response = ui
        .interact(
            rect,
            ui.id().with(("knowledge_index_entry", entry.id)),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!(
                "{}, {}, {}",
                entry.title, entry.concept_type, entry.health_label
            ),
        )
    });
    if response.hovered() {
        ui.painter().rect_stroke(
            rect,
            1.0,
            egui::Stroke::new(1.0, crate::theme::border(dark_mode)),
            egui::StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            1.0,
            egui::Stroke::new(2.0, crate::theme::AIR_CYAN),
            egui::StrokeKind::Inside,
        );
    }
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    response
}

#[derive(Debug, Clone, Copy)]
enum KnowledgePillTone {
    Neutral,
    Accent,
    Attention,
}

fn knowledge_pill(ui: &mut egui::Ui, label: &str, tone: KnowledgePillTone) {
    let dark_mode = ui.visuals().dark_mode;
    let (fill, color) = match tone {
        KnowledgePillTone::Neutral => (
            crate::theme::neutral_tint(dark_mode),
            crate::theme::secondary_text(dark_mode),
        ),
        KnowledgePillTone::Accent => (
            crate::theme::accent_tint(dark_mode),
            crate::theme::accent_text(dark_mode),
        ),
        KnowledgePillTone::Attention => (
            crate::theme::attention_tint(dark_mode),
            crate::theme::attention(dark_mode),
        ),
    };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(2))
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(11.0).color(color));
        });
}

fn graph_filter_controls(
    ui: &mut egui::Ui,
    localization: &Localization,
    query: &mut String,
    selected_type: &mut Option<String>,
    selected_tag: &mut Option<String>,
    types: &BTreeSet<String>,
    tags: &BTreeSet<String>,
) {
    if ResponsiveLayout::from_available(ui.available_size()).is_narrow() {
        ui.add(egui::Label::new(localization.text("knowledge-graph-description")).wrap());
        ui.add(
            egui::TextEdit::singleline(query)
                .desired_width(ui.available_width())
                .hint_text(localization.text("knowledge-filter-concepts")),
        );
        ui.horizontal_wrapped(|ui| {
            filter_combo(
                ui,
                localization,
                "knowledge-filter-type",
                selected_type,
                types,
            );
            filter_combo(ui, localization, "knowledge-filter-tag", selected_tag, tags);
        });
    } else {
        ui.horizontal(|ui| {
            ui.label(localization.text("knowledge-graph-description"));
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(query)
                    .desired_width(260.0)
                    .hint_text(localization.text("knowledge-filter-concepts")),
            );
            filter_combo(
                ui,
                localization,
                "knowledge-filter-type",
                selected_type,
                types,
            );
            filter_combo(ui, localization, "knowledge-filter-tag", selected_tag, tags);
        });
    }
}

fn filter_combo(
    ui: &mut egui::Ui,
    localization: &Localization,
    label_id: &str,
    selected: &mut Option<String>,
    values: &BTreeSet<String>,
) {
    let label = localization.text(label_id);
    egui::ComboBox::from_id_salt(("knowledge_filter", label_id))
        .width(105.0)
        .selected_text(selected.as_deref().unwrap_or(&label))
        .show_ui(ui, |ui| {
            let mut arguments = fluent_bundle::FluentArgs::new();
            arguments.set("filter", label.as_str());
            ui.selectable_value(
                selected,
                None,
                localization.text_with("knowledge-filter-all", Some(&arguments)),
            );
            for value in values.iter() {
                ui.selectable_value(selected, Some(value.clone()), value);
            }
        });
}

fn bundle_state_badge(
    ui: &mut egui::Ui,
    localization: &Localization,
    bundle: &KnowledgeBundleView,
) {
    let (message, color) = bundle_state_visual(bundle, ui.visuals().dark_mode);
    let label = localization.text(message);
    ui.colored_label(
        color,
        RichText::new(format!("● {label}")).family(crate::theme::semibold_font_family()),
    );
}

fn bundle_state_visual(bundle: &KnowledgeBundleView, dark_mode: bool) -> (&'static str, Color32) {
    match bundle.state {
        KnowledgeBundleState::Empty => (
            "knowledge-state-empty",
            crate::theme::secondary_text(dark_mode),
        ),
        KnowledgeBundleState::Ready if bundle.health.error_count > 0 => (
            "knowledge-state-attention",
            crate::theme::error_text(dark_mode),
        ),
        KnowledgeBundleState::Ready if bundle.health.warning_count > 0 => (
            "knowledge-state-review",
            crate::theme::warning_text(dark_mode),
        ),
        KnowledgeBundleState::Ready => (
            "knowledge-state-ready",
            crate::theme::verified_text(dark_mode),
        ),
        KnowledgeBundleState::Updating => (
            "knowledge-state-updating",
            crate::theme::warning_text(dark_mode),
        ),
    }
}

fn empty_bundle_has_health_findings(bundle: &KnowledgeBundleView) -> bool {
    matches!(bundle.state, KnowledgeBundleState::Empty) && !bundle.health.issues.is_empty()
}

fn link_status(
    localization: &Localization,
    disposition: &KnowledgeLinkDisposition,
    dark_mode: bool,
) -> (String, Color32) {
    let (message, color) = match disposition {
        KnowledgeLinkDisposition::Internal(_) => (
            "knowledge-link-internal",
            crate::theme::verified_text(dark_mode),
        ),
        KnowledgeLinkDisposition::External => (
            "knowledge-link-external",
            crate::theme::accent_text(dark_mode),
        ),
        KnowledgeLinkDisposition::Broken => (
            "knowledge-link-broken-status",
            crate::theme::warning_text(dark_mode),
        ),
        KnowledgeLinkDisposition::Unsafe => (
            "knowledge-link-blocked-status",
            crate::theme::error_text(dark_mode),
        ),
    };
    (localization.text(message), color)
}

fn severity_visual(
    localization: &Localization,
    severity: &HealthSeverity,
    dark_mode: bool,
) -> (String, Color32) {
    let (message, color) = match severity {
        HealthSeverity::Error => (
            "knowledge-severity-error",
            crate::theme::error_text(dark_mode),
        ),
        HealthSeverity::Warning => (
            "knowledge-severity-warning",
            crate::theme::warning_text(dark_mode),
        ),
        HealthSeverity::Info => (
            "knowledge-severity-info",
            crate::theme::accent_text(dark_mode),
        ),
    };
    (localization.text(message), color)
}

fn concept_health_tone(report: &BundleHealthReport, page_id: KnowledgePageId) -> ConceptHealthTone {
    let needs_attention = report.issues.iter().any(|issue| {
        issue.page == Some(page_id)
            && matches!(
                issue.severity,
                HealthSeverity::Warning | HealthSeverity::Error
            )
    });
    if needs_attention {
        ConceptHealthTone::Attention
    } else {
        ConceptHealthTone::Healthy
    }
}

fn concept_graph_color(bundle: &KnowledgeBundleView, page_id: KnowledgePageId) -> Color32 {
    match concept_health_tone(&bundle.health, page_id) {
        ConceptHealthTone::Healthy => crate::theme::AIR_CYAN,
        ConceptHealthTone::Attention => crate::theme::attention(false),
    }
}

fn editorial_metadata_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    !key.contains("uuid")
        && !key.contains("fingerprint")
        && !key.contains("sha256")
        && !key.contains("peer")
        && !key.contains("address")
        && !key.contains("port")
        && key != "resource"
        && key != "id"
        && !key.ends_with(".id")
        && !key.ends_with("_id")
}

fn health_card(ui: &mut egui::Ui, label: &str, value: usize, color: Color32) {
    crate::theme::surface_frame(ui.visuals().dark_mode).show(ui, |ui| {
        ui.set_min_width(130.0);
        ui.label(RichText::new(value.to_string()).size(25.0).color(color));
        ui.label(
            RichText::new(label)
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    });
}

fn empty_state(ui: &mut egui::Ui, title: &str, body: &str) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.heading(title);
            ui.label(RichText::new(body).color(ui.visuals().weak_text_color()));
        });
    });
}

fn error_state(ui: &mut egui::Ui, localization: &Localization, title: &str, error: &str) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.colored_label(
                crate::theme::error_text(ui.visuals().dark_mode),
                RichText::new(title)
                    .size(20.0)
                    .family(crate::theme::semibold_font_family()),
            );
            ui.label(localized_knowledge_error(localization, error));
            if error != "knowledge-error-wrong-collection" {
                ui.collapsing(localization.text("action-details"), |ui| {
                    ui.label(sanitized_knowledge_error_code(error));
                });
            }
        });
    });
}

fn sanitized_knowledge_error_code(error: &str) -> &'static str {
    match error {
        "knowledge-error-wrong-collection" => "knowledge-error-wrong-collection",
        "knowledge-error-stale-snapshot" => "knowledge-error-stale-snapshot",
        "wiki_repair_history_requires_human" => "wiki_repair_history_requires_human",
        "wiki_repair_bundle_updating" => "wiki_repair_bundle_updating",
        "wiki_repair_stale_preview" => "wiki_repair_stale_preview",
        "wiki_repair_confirmation_required" => "wiki_repair_confirmation_required",
        "wiki_repair_unresolved_scope" => "wiki_repair_unresolved_scope",
        "wiki_repair_unsafe_layout" => "wiki_repair_unsafe_layout",
        "wiki_repair_snapshot_too_large" => "wiki_repair_snapshot_too_large",
        "wiki_repair_post_validation_failed" => "wiki_repair_post_validation_failed",
        "wiki_repair_rollback_failed" => "wiki_repair_rollback_failed",
        "wiki_repair_operation_in_progress" => "wiki_repair_operation_in_progress",
        "wiki_repair_worker_panicked" => "wiki_repair_worker_panicked",
        "wiki_repair_preview_wrong_collection" => "wiki_repair_preview_wrong_collection",
        "wiki_repair_result_wrong_collection" => "wiki_repair_result_wrong_collection",
        _ => "knowledge_operation_failed",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashSet},
        sync::Arc,
        time::{Duration, Instant, SystemTime},
    };

    use airwiki_core::{
        BundleHealthReport, GuidedRepairChange, GuidedRepairFilePreview, GuidedRepairPreview,
        GuidedRepairResult, HealthIssue, HealthRecovery, HealthSeverity, KnowledgeBundleState,
        KnowledgeBundleView, KnowledgeConceptView, KnowledgeLinkDisposition, KnowledgeLinkView,
        KnowledgePageId, KnowledgePageView, RepairAuthority, RepairPlanId,
    };
    use airwiki_types::CollectionPolicy;
    use chrono::Utc;
    use uuid::Uuid;

    use crate::i18n::{Localization, UiLocale};

    use super::{
        ConceptHealthTone, GRAPH_LAYOUT_WORK_BUDGET, KnowledgeAction, KnowledgeTab, KnowledgeUi,
        SearchEvidenceTarget, WikiLayoutMode, build_graph, bundle_state_visual,
        concept_health_tone, deterministic_graph_position, editorial_graph_node_limit,
        editorial_graph_position, editorial_metadata_key, empty_bundle_has_health_findings,
        graph_requires_filter, health_has_guided_content_repair, health_has_manual_intervention,
        health_issue_page_available, health_recovery_message_id, link_status, normalized_http_url,
        severity_visual, truncate_chars, wiki_layout_mode, wiki_reader_page_selected,
    };

    fn localization() -> Localization {
        Localization::new(UiLocale::EnUs).unwrap()
    }

    #[test]
    fn compact_wiki_switches_to_one_column_before_760_pixels() {
        assert_eq!(wiki_layout_mode(759.0), WikiLayoutMode::Compact);
        assert_eq!(wiki_layout_mode(760.0), WikiLayoutMode::Wide);
    }

    #[test]
    fn bundle_log_uses_the_existing_page_reader_contract() {
        assert!(wiki_reader_page_selected(Some(KnowledgePageId::Log)));
        assert!(wiki_reader_page_selected(Some(KnowledgePageId::Concept(
            Uuid::new_v4()
        ))));
        assert!(!wiki_reader_page_selected(Some(KnowledgePageId::Index)));
        assert!(!wiki_reader_page_selected(None));
    }

    #[test]
    fn editorial_graph_uses_the_reference_five_node_composition() {
        let canvas = eframe::egui::Rect::from_min_max(
            eframe::egui::pos2(0.0, 0.0),
            eframe::egui::pos2(720.0, 400.0),
        );

        let lead = editorial_graph_position(canvas, 0, 5);
        let final_node = editorial_graph_position(canvas, 4, 5);

        assert!((lead.x - 316.8).abs() < 0.01);
        assert!((lead.y - 80.0).abs() < 0.01);
        assert!((final_node.x - 331.2).abs() < 0.01);
        assert!((final_node.y - 328.0).abs() < 0.01);
    }

    #[test]
    fn compact_graph_keeps_every_interactive_node_inside_the_canvas() {
        let canvas = eframe::egui::Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            eframe::egui::vec2(340.0, 340.0),
        );
        let nodes = (0..4)
            .map(|ordinal| editorial_graph_position(canvas, ordinal, 4))
            .collect::<Vec<_>>();

        for node in &nodes {
            assert!(node.x >= canvas.left() + 75.0);
            assert!(node.x <= canvas.right() - 75.0);
            assert!(node.y >= canvas.top() + 20.0);
            assert!(node.y <= canvas.bottom() - 60.0);
        }
        for (index, node) in nodes.iter().enumerate() {
            let hit_rect = eframe::egui::Rect::from_center_size(
                *node + eframe::egui::vec2(0.0, 20.0),
                eframe::egui::vec2(150.0, 62.0),
            );
            for other in nodes.iter().skip(index + 1) {
                let other_rect = eframe::egui::Rect::from_center_size(
                    *other + eframe::egui::vec2(0.0, 20.0),
                    eframe::egui::vec2(150.0, 62.0),
                );
                assert!(!hit_rect.intersects(other_rect));
            }
        }
    }

    #[test]
    fn editorial_graph_caps_hit_targets_for_wide_and_compact_canvases() {
        assert_eq!(editorial_graph_node_limit(720.0), 12);
        assert_eq!(editorial_graph_node_limit(619.0), 4);
        assert_eq!(editorial_graph_node_limit(340.0), 4);
    }

    #[test]
    fn concept_health_uses_only_page_warning_or_error_findings() {
        let page_id = KnowledgePageId::Concept(Uuid::new_v4());
        let other_page = KnowledgePageId::Concept(Uuid::new_v4());
        let mut report = BundleHealthReport {
            checked_at: Utc::now(),
            total_concepts: 2,
            error_count: 0,
            warning_count: 0,
            issues: vec![
                HealthIssue {
                    severity: HealthSeverity::Info,
                    code: "info".to_owned(),
                    page: Some(page_id),
                    message: "Informational".to_owned(),
                },
                HealthIssue {
                    severity: HealthSeverity::Warning,
                    code: "warning".to_owned(),
                    page: Some(other_page),
                    message: "Other page".to_owned(),
                },
            ],
        };
        assert_eq!(
            concept_health_tone(&report, page_id),
            ConceptHealthTone::Healthy
        );
        report.issues.push(HealthIssue {
            severity: HealthSeverity::Error,
            code: "error".to_owned(),
            page: Some(page_id),
            message: "This page".to_owned(),
        });
        assert_eq!(
            concept_health_tone(&report, page_id),
            ConceptHealthTone::Attention
        );
    }

    #[test]
    fn editorial_metadata_hides_technical_identifiers() {
        assert!(editorial_metadata_key("language"));
        assert!(!editorial_metadata_key("resource"));
        assert!(!editorial_metadata_key("airwiki.id"));
        assert!(!editorial_metadata_key("airwiki.collection_id"));
        assert!(!editorial_metadata_key("source_document_id"));
        assert!(!editorial_metadata_key("source_sha256"));
        assert!(!editorial_metadata_key("publisher peer id"));
        assert!(!editorial_metadata_key("listen address"));
    }

    #[test]
    fn informational_text_consumers_use_the_adaptive_accent() {
        let localization = localization();

        for dark_mode in [false, true] {
            let (_, external_link_color) = link_status(
                &localization,
                &KnowledgeLinkDisposition::External,
                dark_mode,
            );
            let (_, info_severity_color) =
                severity_visual(&localization, &HealthSeverity::Info, dark_mode);

            assert_eq!(
                [external_link_color, info_severity_color],
                [crate::theme::accent_text(dark_mode); 2]
            );
        }
    }

    #[test]
    fn only_http_and_https_are_external_candidates() {
        assert_eq!(
            normalized_http_url("https://example.com/path"),
            Some("https://example.com/path")
        );
        assert_eq!(
            normalized_http_url("HTTP://example.com"),
            Some("HTTP://example.com")
        );
        assert_eq!(normalized_http_url("file:///etc/passwd"), None);
        assert_eq!(normalized_http_url("javascript:alert(1)"), None);
        assert_eq!(
            normalized_http_url("https://example.com\nfile:///tmp/x"),
            None
        );
        assert_eq!(normalized_http_url(" https://example.com"), None);
    }

    #[test]
    fn label_truncation_is_unicode_safe() {
        assert_eq!(truncate_chars("áβ猫", 2), "áβ…");
        assert_eq!(truncate_chars("áβ", 2), "áβ");
    }

    #[test]
    fn matching_search_evidence_opens_the_exact_published_concept() {
        let target = search_target(Uuid::new_v4(), Uuid::new_v4());
        let mut ui = ui_with_bundle(bundle_with_target(&target));
        ui.tab = KnowledgeTab::Health;
        ui.query_filter = "hidden".to_owned();

        let action = ui
            .open_search_evidence(target.clone(), false)
            .expect("matching evidence loads its concept page");
        let request_id = page_request_id(&action);

        assert!(matches!(
            action,
            KnowledgeAction::LoadPage {
                collection_id,
                page_id: KnowledgePageId::Concept(concept_id),
                ..
            } if collection_id == target.collection_id && concept_id == target.concept_id
        ));
        assert_eq!(ui.tab, KnowledgeTab::Wiki);
        assert!(ui.query_filter.is_empty());
        assert_eq!(ui.search_evidence, Some(target.clone()));

        assert!(
            ui.page_loaded(
                request_id,
                target.collection_id,
                KnowledgePageId::Concept(target.concept_id),
                Ok(concept_page(&target)),
            )
            .is_none()
        );
        assert!(ui.search_evidence_focus_pending);
    }

    #[test]
    fn search_evidence_in_another_collection_loads_bundle_then_exact_page() {
        let target = search_target(Uuid::new_v4(), Uuid::new_v4());
        let mut ui = ui_with_bundle(bundle(Uuid::new_v4()));

        let bundle_action = ui
            .open_search_evidence(target.clone(), false)
            .expect("another collection needs a bundle snapshot");
        assert!(matches!(
            bundle_action,
            KnowledgeAction::LoadBundle { collection_id, .. }
                if collection_id == target.collection_id
        ));

        let page_action = ui
            .bundle_loaded(
                bundle_request_id(&bundle_action),
                target.collection_id,
                Ok(bundle_with_target(&target)),
            )
            .expect("the matching snapshot loads the cited concept");
        assert!(matches!(
            page_action,
            KnowledgeAction::LoadPage {
                page_id: KnowledgePageId::Concept(concept_id),
                ..
            } if concept_id == target.concept_id
        ));
    }

    #[test]
    fn stale_search_identity_never_falls_back_to_another_wiki_page() {
        for drift in ["missing", "revision", "hash", "resource"] {
            let target = search_target(Uuid::new_v4(), Uuid::new_v4());
            let mut snapshot = bundle_with_target(&target);
            match drift {
                "missing" => snapshot.concepts.clear(),
                "revision" => snapshot.concepts[0].revision = Some(target.source_revision + 1),
                "hash" => snapshot.concepts[0].source_sha256 = Some("b".repeat(64)),
                "resource" => snapshot.concepts[0].resource = Some("urn:airwiki:other".to_owned()),
                _ => unreachable!(),
            }
            let mut ui = KnowledgeUi::default();
            let request = ui
                .open_search_evidence(target.clone(), false)
                .expect("the target collection must be inspected");

            let action = ui.bundle_loaded(
                bundle_request_id(&request),
                target.collection_id,
                Ok(snapshot),
            );

            assert!(action.is_none(), "{drift} drift must fail closed");
            assert!(ui.selected_page.is_none());
            assert!(ui.page_pending.is_none());
            assert!(ui.search_evidence.is_none());
            assert_eq!(
                ui.link_notice.as_ref().map(|notice| notice.1.as_str()),
                Some("knowledge-search-evidence-stale")
            );
        }
    }

    #[test]
    fn stale_page_recovery_revalidates_the_search_revision() {
        let target = search_target(Uuid::new_v4(), Uuid::new_v4());
        let mut ui = ui_with_bundle(bundle_with_target(&target));
        let page_action = ui.open_search_evidence(target.clone(), false).unwrap();

        let reload = ui
            .page_loaded(
                page_request_id(&page_action),
                target.collection_id,
                KnowledgePageId::Concept(target.concept_id),
                Err("page changed".to_owned()),
            )
            .expect("a stale page reloads the bundle once");
        let mut newer = bundle_with_target(&target);
        newer.concepts[0].revision = Some(target.source_revision + 1);

        let action = ui.bundle_loaded(bundle_request_id(&reload), target.collection_id, Ok(newer));

        assert!(action.is_none());
        assert!(ui.page_pending.is_none());
        assert!(ui.search_evidence.is_none());
        assert_eq!(
            ui.link_notice.as_ref().map(|notice| notice.1.as_str()),
            Some("knowledge-search-evidence-stale")
        );
    }

    #[test]
    fn manual_wiki_navigation_clears_the_search_evidence_trace() {
        let target = search_target(Uuid::new_v4(), Uuid::new_v4());
        let mut ui = ui_with_bundle(bundle_with_target(&target));
        assert!(ui.open_search_evidence(target, false).is_some());
        ui.search_evidence_focus_pending = true;

        let action = ui.request_page(KnowledgePageId::Index);

        assert!(action.is_some());
        assert!(ui.search_evidence.is_none());
        assert!(!ui.search_evidence_focus_pending);
    }

    #[test]
    fn reloading_the_same_search_evidence_does_not_steal_focus_again() {
        let target = search_target(Uuid::new_v4(), Uuid::new_v4());
        let mut ui = ui_with_bundle(bundle_with_target(&target));
        let first_page = ui.open_search_evidence(target.clone(), false).unwrap();
        assert!(
            ui.page_loaded(
                page_request_id(&first_page),
                target.collection_id,
                KnowledgePageId::Concept(target.concept_id),
                Ok(concept_page(&target)),
            )
            .is_none()
        );
        assert!(ui.search_evidence_focus_pending);

        // Rendering the evidence trace consumes the one-shot focus request.
        ui.search_evidence_focus_pending = false;
        let reload = ui
            .mark_snapshot_stale(Some(target.collection_id), true)
            .expect("the changed bundle is reloaded");
        let reloaded_page = ui
            .bundle_loaded(
                bundle_request_id(&reload),
                target.collection_id,
                Ok(bundle_with_target(&target)),
            )
            .expect("the same evidence is still current");
        assert!(
            ui.page_loaded(
                page_request_id(&reloaded_page),
                target.collection_id,
                KnowledgePageId::Concept(target.concept_id),
                Ok(concept_page(&target)),
            )
            .is_none()
        );
        assert!(!ui.search_evidence_focus_pending);
    }

    #[test]
    fn active_scan_defers_search_navigation_until_the_bundle_is_stable() {
        let target = search_target(Uuid::new_v4(), Uuid::new_v4());
        let mut ui = ui_with_bundle(bundle_with_target(&target));

        assert!(ui.open_search_evidence(target.clone(), true).is_none());
        assert!(ui.bundle.is_none());
        assert_eq!(ui.search_evidence, Some(target.clone()));

        let bundle_action = ui
            .collection_scan_finished(target.collection_id, true)
            .expect("scan completion reloads the target bundle");
        let page_action = ui.bundle_loaded(
            bundle_request_id(&bundle_action),
            target.collection_id,
            Ok(bundle_with_target(&target)),
        );
        assert!(matches!(
            page_action,
            Some(KnowledgeAction::LoadPage { .. })
        ));
    }

    #[test]
    fn scan_start_cancels_an_in_flight_search_page_and_revalidates_afterwards() {
        let target = search_target(Uuid::new_v4(), Uuid::new_v4());
        let mut ui = ui_with_bundle(bundle_with_target(&target));
        let stale_page = ui.open_search_evidence(target.clone(), false).unwrap();

        ui.collection_scan_started(target.collection_id);
        assert!(ui.bundle.is_none());
        assert!(ui.page_pending.is_none());
        assert_eq!(ui.search_evidence, Some(target.clone()));
        assert!(
            ui.page_loaded(
                page_request_id(&stale_page),
                target.collection_id,
                KnowledgePageId::Concept(target.concept_id),
                Ok(concept_page(&target)),
            )
            .is_none()
        );
        assert!(ui.page.is_none());

        let reload = ui
            .collection_scan_finished(target.collection_id, true)
            .expect("the stable collection is re-inspected");
        let current_page = ui
            .bundle_loaded(
                bundle_request_id(&reload),
                target.collection_id,
                Ok(bundle_with_target(&target)),
            )
            .expect("the exact target remains published");
        assert!(
            ui.page_loaded(
                page_request_id(&current_page),
                target.collection_id,
                KnowledgePageId::Concept(target.concept_id),
                Ok(concept_page(&target)),
            )
            .is_none()
        );
        assert_eq!(
            ui.page.as_ref().map(|page| page.page_id),
            Some(KnowledgePageId::Concept(target.concept_id))
        );
    }

    #[test]
    fn a_second_search_target_replaces_an_in_flight_page_in_the_same_collection() {
        let collection_id = Uuid::new_v4();
        let first = search_target(collection_id, Uuid::new_v4());
        let second = search_target(collection_id, Uuid::new_v4());
        let mut snapshot = bundle_with_target(&first);
        snapshot
            .concepts
            .extend(bundle_with_target(&second).concepts);
        snapshot.health.total_concepts = 2;
        let mut ui = ui_with_bundle(snapshot);

        let stale_page = ui.open_search_evidence(first.clone(), false).unwrap();
        let current_page = ui.open_search_evidence(second.clone(), false).unwrap();

        assert!(
            ui.page_loaded(
                page_request_id(&stale_page),
                collection_id,
                KnowledgePageId::Concept(first.concept_id),
                Ok(concept_page(&first)),
            )
            .is_none()
        );
        assert!(ui.page.is_none());
        assert_eq!(ui.search_evidence, Some(second.clone()));
        assert!(
            ui.page_loaded(
                page_request_id(&current_page),
                collection_id,
                KnowledgePageId::Concept(second.concept_id),
                Ok(concept_page(&second)),
            )
            .is_none()
        );
        assert_eq!(
            ui.page.as_ref().map(|page| page.page_id),
            Some(KnowledgePageId::Concept(second.concept_id))
        );
    }

    #[test]
    fn a_second_search_target_replaces_an_in_flight_page_in_another_collection() {
        let first = search_target(Uuid::new_v4(), Uuid::new_v4());
        let second = search_target(Uuid::new_v4(), Uuid::new_v4());
        let mut ui = ui_with_bundle(bundle_with_target(&first));

        let stale_page = ui.open_search_evidence(first.clone(), false).unwrap();
        let current_bundle = ui.open_search_evidence(second.clone(), false).unwrap();

        assert!(
            ui.page_loaded(
                page_request_id(&stale_page),
                first.collection_id,
                KnowledgePageId::Concept(first.concept_id),
                Ok(concept_page(&first)),
            )
            .is_none()
        );
        assert!(ui.page.is_none());
        assert_eq!(ui.search_evidence, Some(second.clone()));

        let current_page = ui
            .bundle_loaded(
                bundle_request_id(&current_bundle),
                second.collection_id,
                Ok(bundle_with_target(&second)),
            )
            .expect("the replacement target loads its exact page");
        assert!(
            ui.page_loaded(
                page_request_id(&current_page),
                second.collection_id,
                KnowledgePageId::Concept(second.concept_id),
                Ok(concept_page(&second)),
            )
            .is_none()
        );
        assert_eq!(
            ui.page.as_ref().map(|page| page.page_id),
            Some(KnowledgePageId::Concept(second.concept_id))
        );
    }

    #[test]
    fn late_page_response_cannot_replace_the_current_request() {
        let collection_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(collection_id));
        let first = ui.request_page(KnowledgePageId::Index).unwrap();
        let second = ui.request_page(KnowledgePageId::Index).unwrap();
        let first_request = page_request_id(&first);
        let second_request = page_request_id(&second);

        let recovery = ui.page_loaded(
            first_request,
            collection_id,
            KnowledgePageId::Index,
            Ok(page(collection_id, "index-v1")),
        );

        assert!(recovery.is_none());
        assert_eq!(
            ui.page_pending.as_ref().map(|pending| pending.request_id),
            Some(second_request)
        );
        assert!(ui.page.is_none());
    }

    #[test]
    fn stale_fingerprint_invalidates_and_reloads_the_bundle_once() {
        let collection_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(collection_id));
        let request = ui.request_page(KnowledgePageId::Index).unwrap();

        let recovery = ui
            .page_loaded(
                page_request_id(&request),
                collection_id,
                KnowledgePageId::Index,
                Ok(page(collection_id, "different-fingerprint")),
            )
            .expect("a stale fingerprint reloads the bundle");
        let bundle_request = bundle_request_id(&recovery);
        assert!(ui.bundle.is_none());
        assert!(ui.snapshot_stale);

        let next_page = ui
            .bundle_loaded(bundle_request, collection_id, Ok(bundle(collection_id)))
            .expect("the refreshed bundle reloads the selected page");
        let second_failure = ui.page_loaded(
            page_request_id(&next_page),
            collection_id,
            KnowledgePageId::Index,
            Err("persistent read error".to_owned()),
        );
        assert!(second_failure.is_none());
        assert_eq!(ui.page_error.as_deref(), Some("persistent read error"));
    }

    #[test]
    fn graph_root_is_index_and_never_adds_log_as_an_independent_node() {
        let collection_id = Uuid::new_v4();
        let graph = build_graph(
            &localization(),
            &bundle(collection_id),
            "test".to_owned(),
            "",
            None,
            None,
        );
        assert_eq!(graph.graph.node_count(), 1);
    }

    #[test]
    fn graph_accepts_five_hundred_concepts_plus_the_index_root() {
        let collection_id = Uuid::new_v4();
        let mut bundle = bundle(collection_id);
        bundle.concepts = (0..500).map(concept).collect();

        assert!(!graph_requires_filter(500));
        assert!(graph_requires_filter(501));
        let graph = build_graph(
            &localization(),
            &bundle,
            "five-hundred".to_owned(),
            "",
            None,
            None,
        );
        assert_eq!(graph.graph.node_count(), 501);
    }

    #[test]
    fn graph_projects_every_internal_link_without_a_cap_or_deduplication() {
        let collection_id = Uuid::new_v4();
        let mut bundle = bundle(collection_id);
        bundle.concepts = (0..500).map(concept).collect();
        let target_ids = bundle
            .concepts
            .iter()
            .map(|concept| concept.id)
            .collect::<Vec<_>>();
        bundle.links = (0..4_005)
            .map(|ordinal| {
                let target_id = target_ids[ordinal % target_ids.len()];
                KnowledgeLinkView {
                    source: KnowledgePageId::Index,
                    label: "mismo enlace".to_owned(),
                    raw_target: format!("concepts/{target_id}.md#{ordinal}"),
                    disposition: KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(
                        target_id,
                    )),
                }
            })
            .collect();

        let graph = build_graph(
            &localization(),
            &bundle,
            "all-edges".to_owned(),
            "",
            None,
            None,
        );
        assert_eq!(graph.graph.edge_count(), 4_005);
    }

    #[test]
    fn graph_layout_is_incremental_deterministic_and_stops_when_stable() {
        let collection_id = Uuid::new_v4();
        let mut bundle = bundle(collection_id);
        bundle.concepts = (0..130).map(concept).collect();
        let mut graph = build_graph(
            &localization(),
            &bundle,
            "incremental".to_owned(),
            "",
            None,
            None,
        );

        assert_eq!(
            graph
                .layout
                .advance_with_limits(&mut graph.graph, Duration::from_secs(1), 64),
            64
        );
        assert!(!graph.layout.stable);
        assert_eq!(
            graph
                .layout
                .advance_with_limits(&mut graph.graph, Duration::from_secs(1), 64),
            64
        );
        assert_eq!(
            graph
                .layout
                .advance_with_limits(&mut graph.graph, Duration::from_secs(1), 64),
            3
        );
        assert!(graph.layout.stable);

        let stable_positions = graph
            .graph
            .nodes_iter()
            .map(|(_, node)| node.location())
            .collect::<Vec<_>>();
        assert_eq!(
            stable_positions,
            (0..graph.graph.node_count())
                .map(|ordinal| deterministic_graph_position(ordinal, graph.graph.node_count()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            graph
                .layout
                .advance_with_limits(&mut graph.graph, Duration::from_secs(1), 64),
            0
        );
        assert_eq!(
            stable_positions,
            graph
                .graph
                .nodes_iter()
                .map(|(_, node)| node.location())
                .collect::<Vec<_>>()
        );
        assert!(GRAPH_LAYOUT_WORK_BUDGET < Duration::from_millis(4));
    }

    #[test]
    fn repeated_snapshot_invalidations_coalesce_while_reload_is_pending() {
        let collection_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(collection_id));

        let first = ui.mark_snapshot_stale(None, true);
        let duplicate = ui.mark_snapshot_stale(Some(collection_id), true);

        assert!(matches!(first, Some(KnowledgeAction::LoadBundle { .. })));
        assert!(duplicate.is_none());
        assert!(ui.bundle_pending.is_some());
    }

    #[test]
    fn scan_start_clears_snapshot_and_cancels_in_flight_reads_without_loading() {
        let collection_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(collection_id));
        let pending_page = ui.request_page(KnowledgePageId::Index).unwrap();
        ui.page = Some(Arc::new(page(collection_id, "index-v1")));
        let pending_bundle = ui.request_bundle(collection_id);

        ui.collection_scan_started(collection_id);

        assert!(ui.bundle.is_none());
        assert!(ui.page.is_none());
        assert!(ui.bundle_pending.is_none());
        assert!(ui.page_pending.is_none());
        assert!(ui.snapshot_stale);
        assert!(
            ui.bundle_loaded(
                bundle_request_id(&pending_bundle),
                collection_id,
                Ok(bundle(collection_id)),
            )
            .is_none()
        );
        assert!(
            ui.page_loaded(
                page_request_id(&pending_page),
                collection_id,
                KnowledgePageId::Index,
                Ok(page(collection_id, "index-v1")),
            )
            .is_none()
        );
        assert!(ui.bundle.is_none(), "a late response must stay discarded");
        assert!(ui.page.is_none(), "a late page must stay discarded");
    }

    #[test]
    fn collections_event_does_not_reload_the_selected_active_scan() {
        let collection_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(collection_id));
        let active_scans = HashSet::from([collection_id]);

        let action = ui.collections_changed(&active_scans, true);

        assert!(action.is_none());
        assert!(ui.bundle.is_some());
        assert!(ui.bundle_pending.is_none());
    }

    #[test]
    fn scan_finish_loads_the_selected_bundle_exactly_once() {
        let collection_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(collection_id));
        ui.collection_scan_started(collection_id);

        let first = ui.collection_scan_finished(collection_id, true);
        let duplicate = ui.collection_scan_finished(collection_id, true);

        assert!(matches!(first, Some(KnowledgeAction::LoadBundle { .. })));
        assert!(duplicate.is_none());
        assert!(ui.bundle_pending.is_some());
    }

    #[test]
    fn selecting_a_collection_with_an_active_scan_never_loads_it() {
        let original_id = Uuid::new_v4();
        let scanning_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(original_id));

        let action = ui.select_collection(scanning_id, true);
        let mut follow_up_actions = Vec::new();
        ui.ensure_collection(
            &[
                (original_id, "Original".to_owned()),
                (scanning_id, "Scan".to_owned()),
            ],
            &HashSet::from([scanning_id]),
            &mut follow_up_actions,
        );

        assert!(action.is_none());
        assert!(follow_up_actions.is_empty());
        assert_eq!(ui.collection_id, Some(scanning_id));
        assert!(ui.bundle.is_none());
        assert!(ui.bundle_pending.is_none());
    }

    #[test]
    fn empty_bundle_health_findings_remain_visible() {
        let collection_id = Uuid::new_v4();
        let mut empty = bundle(collection_id);
        empty.state = KnowledgeBundleState::Empty;
        assert!(!empty_bundle_has_health_findings(&empty));

        empty.health.issues.push(HealthIssue {
            severity: HealthSeverity::Error,
            code: "missing_index".to_owned(),
            page: Some(KnowledgePageId::Index),
            message: "Falta index.md".to_owned(),
        });
        empty.health.error_count = 1;

        assert!(empty_bundle_has_health_findings(&empty));
    }

    #[test]
    fn manual_health_findings_never_offer_a_guided_repair() {
        let mut view = bundle(Uuid::new_v4());
        view.health.issues.push(HealthIssue {
            severity: HealthSeverity::Error,
            code: "missing_bundle".to_owned(),
            page: None,
            message: "The managed bundle is missing".to_owned(),
        });

        assert_eq!(
            (
                health_has_manual_intervention(&view),
                health_has_guided_content_repair(&view),
            ),
            (true, false)
        );
    }

    #[test]
    fn verified_concept_drift_keeps_the_guided_repair_action() {
        let mut view = bundle(Uuid::new_v4());
        view.health.issues.push(HealthIssue {
            severity: HealthSeverity::Error,
            code: "metadata_mismatch".to_owned(),
            page: Some(KnowledgePageId::Concept(Uuid::new_v4())),
            message: "Published metadata changed".to_owned(),
        });

        assert_eq!(
            (
                health_has_manual_intervention(&view),
                health_has_guided_content_repair(&view),
            ),
            (false, true)
        );
    }

    #[test]
    fn manual_finding_blocks_the_guided_repair_promise() {
        let mut view = bundle(Uuid::new_v4());
        view.health.issues.extend([
            HealthIssue {
                severity: HealthSeverity::Error,
                code: "metadata_mismatch".to_owned(),
                page: Some(KnowledgePageId::Concept(Uuid::new_v4())),
                message: "Published metadata changed".to_owned(),
            },
            HealthIssue {
                severity: HealthSeverity::Error,
                code: "missing_bundle".to_owned(),
                page: None,
                message: "The managed bundle is missing".to_owned(),
            },
        ]);

        let guided_available =
            health_has_guided_content_repair(&view) && !health_has_manual_intervention(&view);

        assert!(!guided_available);
        assert_eq!(
            health_recovery_message_id(HealthRecovery::GuidedContent, guided_available),
            "knowledge-recovery-guided-blocked"
        );
    }

    #[test]
    fn stable_bundle_with_errors_is_not_badged_as_ready() {
        let mut view = bundle(Uuid::new_v4());
        view.health.error_count = 1;

        assert_eq!(
            bundle_state_visual(&view, false).0,
            "knowledge-state-attention"
        );
    }

    #[test]
    fn stable_bundle_with_warnings_asks_for_review() {
        let mut view = bundle(Uuid::new_v4());
        view.health.warning_count = 1;

        assert_eq!(
            bundle_state_visual(&view, false).0,
            "knowledge-state-review"
        );
    }

    #[test]
    fn health_navigation_selects_the_health_tab() {
        let mut ui = KnowledgeUi::default();

        let action = ui.select_health(None, false);

        assert_eq!(ui.tab, KnowledgeTab::Health);
        assert!(action.is_none());
    }

    #[test]
    fn health_navigation_loads_the_collection_selected_by_the_rollup() {
        let current = Uuid::new_v4();
        let affected = Uuid::new_v4();
        let mut ui = KnowledgeUi {
            collection_id: Some(current),
            ..KnowledgeUi::default()
        };

        let action = ui.select_health(Some(affected), false);

        assert_eq!(ui.collection_id, Some(affected));
        assert!(matches!(
            action,
            Some(KnowledgeAction::LoadBundle { collection_id, .. }) if collection_id == affected
        ));
    }

    #[test]
    fn missing_health_page_is_not_actionable() {
        let view = bundle(Uuid::new_v4());

        assert!(!health_issue_page_available(
            &view,
            KnowledgePageId::Concept(Uuid::new_v4())
        ));
    }

    #[test]
    fn updating_bundle_schedules_an_automatic_retry() {
        let collection_id = Uuid::new_v4();
        let mut ui = KnowledgeUi {
            collection_id: Some(collection_id),
            ..KnowledgeUi::default()
        };
        let request = ui.request_bundle(collection_id);
        let mut updating = bundle(collection_id);
        updating.state = KnowledgeBundleState::Updating;

        assert!(
            ui.bundle_loaded(bundle_request_id(&request), collection_id, Ok(updating))
                .is_none()
        );
        ui.retry_bundle_at = Some(Instant::now() - Duration::from_millis(1));
        let mut actions = Vec::new();
        ui.ensure_collection(
            &[(collection_id, "Prueba".to_owned())],
            &HashSet::new(),
            &mut actions,
        );

        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], KnowledgeAction::LoadBundle { .. }));
    }

    #[test]
    fn guided_repair_discards_stale_preparation_responses() {
        let collection_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(collection_id));
        let action = ui.begin_guided_repair(collection_id);
        let KnowledgeAction::PrepareGuidedRepair { request_id, .. } = action else {
            panic!("expected guided repair preparation");
        };

        ui.guided_repair_prepared(
            Uuid::new_v4(),
            collection_id,
            Ok(guided_preview(collection_id)),
        );

        assert!(ui.guided_repair_prepare_pending.is_some());
        assert!(ui.guided_repair_preview.is_none());

        ui.guided_repair_prepared(request_id, collection_id, Ok(guided_preview(collection_id)));
        assert!(ui.guided_repair_prepare_pending.is_none());
        assert!(ui.guided_repair_preview.is_some());
    }

    #[test]
    fn cancelling_a_guided_preview_never_queues_execution() {
        let collection_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(collection_id));
        ui.guided_repair_preview = Some(guided_preview(collection_id));

        ui.cancel_guided_repair_preview();

        assert!(ui.guided_repair_preview.is_none());
        assert!(ui.guided_repair_execute_pending.is_none());
    }

    #[test]
    fn guided_preview_confirmation_is_single_flight() {
        let collection_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(collection_id));
        ui.guided_repair_preview = Some(guided_preview(collection_id));

        let first = ui.confirm_guided_repair_preview();
        let duplicate = ui.confirm_guided_repair_preview();

        assert!(matches!(
            first,
            Some(KnowledgeAction::ExecuteGuidedRepair { .. })
        ));
        assert!(duplicate.is_none());
        assert!(ui.guided_repair_execute_pending.is_some());
    }

    #[test]
    fn guided_repair_completion_reloads_only_the_matching_request() {
        let collection_id = Uuid::new_v4();
        let mut ui = ui_with_bundle(bundle(collection_id));
        ui.guided_repair_preview = Some(guided_preview(collection_id));
        let action = ui.confirm_guided_repair_preview().unwrap();
        let KnowledgeAction::ExecuteGuidedRepair { request_id, .. } = action else {
            panic!("expected guided repair execution");
        };

        assert!(
            ui.guided_repair_finished(
                Uuid::new_v4(),
                collection_id,
                Ok(guided_result(collection_id)),
                true,
            )
            .is_none()
        );
        assert!(ui.guided_repair_execute_pending.is_some());

        let reload = ui.guided_repair_finished(
            request_id,
            collection_id,
            Ok(guided_result(collection_id)),
            true,
        );
        assert!(matches!(reload, Some(KnowledgeAction::LoadBundle { .. })));
        assert!(ui.guided_repair_execute_pending.is_none());
        assert!(ui.guided_repair_result.is_some());
    }

    fn ui_with_bundle(bundle: KnowledgeBundleView) -> KnowledgeUi {
        KnowledgeUi {
            collection_id: Some(bundle.collection_id),
            bundle: Some(Arc::new(bundle)),
            ..KnowledgeUi::default()
        }
    }

    fn bundle(collection_id: Uuid) -> KnowledgeBundleView {
        KnowledgeBundleView {
            collection_id,
            collection_name: "Prueba".to_owned(),
            collection_policy: CollectionPolicy::default(),
            fingerprint: "bundle-v1".to_owned(),
            state: KnowledgeBundleState::Ready,
            index_fingerprint: Some("index-v1".to_owned()),
            log_fingerprint: Some("log-v1".to_owned()),
            concepts: Vec::new(),
            links: Vec::new(),
            backlinks: BTreeMap::new(),
            health: BundleHealthReport {
                checked_at: SystemTime::UNIX_EPOCH.into(),
                total_concepts: 0,
                error_count: 0,
                warning_count: 0,
                issues: Vec::new(),
            },
        }
    }

    fn search_target(collection_id: Uuid, concept_id: Uuid) -> SearchEvidenceTarget {
        SearchEvidenceTarget {
            collection_id,
            concept_id,
            heading_or_page: "Recovery steps".to_owned(),
            logical_resource_uri: format!("urn:airwiki:test:{concept_id}"),
            source_revision: 1,
            source_sha256: "a".repeat(64),
        }
    }

    fn bundle_with_target(target: &SearchEvidenceTarget) -> KnowledgeBundleView {
        let mut view = bundle(target.collection_id);
        view.concepts.push(KnowledgeConceptView {
            id: target.concept_id,
            relative_path: format!("concepts/{}.md", target.concept_id),
            concept_type: "Runbook".to_owned(),
            title: "Recovery".to_owned(),
            description: "Reviewed recovery steps".to_owned(),
            tags: vec!["recovery".to_owned()],
            resource: Some(target.logical_resource_uri.clone()),
            timestamp: None,
            revision: Some(target.source_revision),
            source_sha256: Some(target.source_sha256.clone()),
            language: Some("en".to_owned()),
            generator_model: None,
            reviewed_at: None,
            extensions: BTreeMap::new(),
            fingerprint: "concept-v1".to_owned(),
        });
        view.health.total_concepts = 1;
        view
    }

    fn page(collection_id: Uuid, fingerprint: &str) -> KnowledgePageView {
        KnowledgePageView {
            collection_id,
            page_id: KnowledgePageId::Index,
            title: "Índice".to_owned(),
            fingerprint: fingerprint.to_owned(),
            body_markdown: "# Índice".to_owned(),
            metadata: Vec::new(),
            outgoing_links: Vec::new(),
            backlinks: Vec::new(),
            truncated: false,
        }
    }

    fn concept_page(target: &SearchEvidenceTarget) -> KnowledgePageView {
        KnowledgePageView {
            collection_id: target.collection_id,
            page_id: KnowledgePageId::Concept(target.concept_id),
            title: "Recovery".to_owned(),
            fingerprint: "concept-v1".to_owned(),
            body_markdown: "# Recovery".to_owned(),
            metadata: Vec::new(),
            outgoing_links: Vec::new(),
            backlinks: Vec::new(),
            truncated: false,
        }
    }

    fn guided_preview(collection_id: Uuid) -> GuidedRepairPreview {
        let orphan_id = Uuid::new_v4();
        GuidedRepairPreview {
            plan_id: RepairPlanId::new(),
            collection_id,
            expected_bundle_fingerprint: "bundle-v1".to_owned(),
            authorities: vec![RepairAuthority::PublishedDatabase],
            files: vec![GuidedRepairFilePreview {
                page: KnowledgePageId::Concept(orphan_id),
                change: GuidedRepairChange::RemoveOrphan,
                before_fingerprint: Some("orphan-v1".to_owned()),
            }],
            concepts_returned_to_review: Vec::new(),
            orphan_concepts_removed: vec![orphan_id],
            impact_code: "guided_repair_withdraws_until_review".to_owned(),
        }
    }

    fn guided_result(collection_id: Uuid) -> GuidedRepairResult {
        GuidedRepairResult {
            plan_id: RepairPlanId::new(),
            collection_id,
            concepts_returned_to_review: Vec::new(),
            orphan_concepts_removed: vec![Uuid::new_v4()],
            snapshot_manifest_sha256: "snapshot".to_owned(),
            bundle_fingerprint: "bundle-v2".to_owned(),
            completed_at: chrono::Utc::now(),
        }
    }

    fn concept(ordinal: usize) -> KnowledgeConceptView {
        let id = Uuid::from_u128(ordinal as u128 + 1);
        KnowledgeConceptView {
            id,
            relative_path: format!("concepts/{id}.md"),
            concept_type: if ordinal.is_multiple_of(2) {
                "Runbook".to_owned()
            } else {
                "Reference".to_owned()
            },
            title: format!("Concepto {ordinal:03}"),
            description: String::new(),
            tags: vec!["prueba".to_owned()],
            resource: Some(format!("urn:airwiki:test:{id}")),
            timestamp: None,
            revision: Some(1),
            source_sha256: None,
            language: Some("es".to_owned()),
            generator_model: None,
            reviewed_at: None,
            extensions: BTreeMap::new(),
            fingerprint: format!("fingerprint-{ordinal}"),
        }
    }

    fn page_request_id(action: &KnowledgeAction) -> Uuid {
        match action {
            KnowledgeAction::LoadPage { request_id, .. } => *request_id,
            _ => panic!("expected page request"),
        }
    }

    fn bundle_request_id(action: &KnowledgeAction) -> Uuid {
        match action {
            KnowledgeAction::LoadBundle { request_id, .. } => *request_id,
            _ => panic!("expected bundle request"),
        }
    }
}
