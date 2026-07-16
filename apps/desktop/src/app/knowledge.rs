use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use airwiki_core::{
    GuidedRepairChange, GuidedRepairPreview, GuidedRepairResult, HealthSeverity,
    KnowledgeBundleState, KnowledgeBundleView, KnowledgeConceptView, KnowledgeLinkDisposition,
    KnowledgePageId, KnowledgePageView, RepairAuthority,
};
use eframe::egui::{self, Color32, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_extras::{Size, StripBuilder};
use egui_graphs::{
    Graph, GraphView, LayoutHierarchical, LayoutStateHierarchical, SettingsInteraction,
    SettingsNavigation, SettingsStyle, reset_metadata, set_layout_state,
};
use uuid::Uuid;

use crate::i18n::Localization;

const MAX_GRAPH_CONCEPTS: usize = 500;
const GRAPH_LAYOUT_FRAME_BUDGET: Duration = Duration::from_millis(4);
// Keep a margin for the persisted egui_graphs layout state around our own step.
const GRAPH_LAYOUT_WORK_BUDGET: Duration = Duration::from_millis(3);
const MAX_LAYOUT_NODES_PER_FRAME: usize = 64;
const UPDATING_RETRY_DELAY: Duration = Duration::from_millis(750);
const TREE_WIDTH: f32 = 270.0;
const NARROW_TREE_WIDTH: f32 = 220.0;
const DETAILS_WIDTH: f32 = 310.0;
const NARROW_WIKI_THRESHOLD: f32 = 760.0;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KnowledgeTab {
    Wiki,
    Graph,
    Health,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NarrowWikiPane {
    Page,
    Details,
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
    widget_id: String,
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
    narrow_wiki_pane: NarrowWikiPane,
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
            narrow_wiki_pane: NarrowWikiPane::Page,
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
            guided_repair_prepare_pending: None,
            guided_repair_execute_pending: None,
            guided_repair_preview: None,
            guided_repair_error: None,
            guided_repair_result: None,
        }
    }
}

impl KnowledgeUi {
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
                        "La página recibida no corresponde al snapshot solicitado; actualiza la wiki"
                            .to_owned(),
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
                    Color32::from_rgb(220, 85, 85)
                } else {
                    Color32::from_rgb(205, 145, 30)
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
            KnowledgeTab::Graph => self.show_graph(ui, localization, &bundle),
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
            self.narrow_wiki_pane = NarrowWikiPane::Page;
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
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(RichText::new(localization.text("knowledge-title")).size(28.0));
                ui.label(
                    RichText::new(localization.text("knowledge-subtitle")).color(Color32::GRAY),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let refresh_enabled = self.collection_id.is_some_and(|collection_id| {
                    !active_scans.contains(&collection_id) && self.bundle_pending.is_none()
                });
                if ui
                    .add_enabled(
                        refresh_enabled,
                        egui::Button::new(localization.text("action-refresh")),
                    )
                    .clicked()
                    && let Some(collection_id) = self.collection_id
                {
                    self.clear_snapshot();
                    actions.push(self.request_bundle(collection_id));
                }

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
                egui::ComboBox::from_id_salt("knowledge_collection")
                    .width(230.0)
                    .selected_text(selected_collection_name)
                    .show_ui(ui, |ui| {
                        for (id, name) in collections {
                            ui.selectable_value(&mut selected_collection, Some(*id), name);
                        }
                    });
                if selected_collection != before
                    && let Some(collection_id) = selected_collection
                    && let Some(action) =
                        self.select_collection(collection_id, active_scans.contains(&collection_id))
                {
                    actions.push(action);
                }
            });
        });
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
                bundle_state_badge(ui, localization, bundle.state);
                let mut arguments = fluent_bundle::FluentArgs::new();
                arguments.set("count", bundle.concepts.len());
                ui.label(
                    RichText::new(
                        localization.text_with("knowledge-concept-count", Some(&arguments)),
                    )
                    .small()
                    .color(Color32::GRAY),
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
        if ui.available_width() < NARROW_WIKI_THRESHOLD {
            StripBuilder::new(ui)
                .size(Size::exact(NARROW_TREE_WIDTH))
                .size(Size::remainder().at_least(260.0))
                .clip(true)
                .horizontal(|mut strip| {
                    strip.cell(|ui| {
                        requested_page =
                            requested_page.or(self.wiki_tree(ui, localization, bundle));
                    });
                    strip.cell(|ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_value(
                                &mut self.narrow_wiki_pane,
                                NarrowWikiPane::Page,
                                localization.text("knowledge-tab-wiki"),
                            );
                            ui.selectable_value(
                                &mut self.narrow_wiki_pane,
                                NarrowWikiPane::Details,
                                localization.text("action-details"),
                            );
                        });
                        ui.separator();
                        requested_page = requested_page.or(match self.narrow_wiki_pane {
                            NarrowWikiPane::Page => self.wiki_page(ui, localization, bundle),
                            NarrowWikiPane::Details => self.wiki_details(ui, localization, bundle),
                        });
                    });
                });
        } else {
            StripBuilder::new(ui)
                .size(Size::exact(TREE_WIDTH))
                .size(Size::remainder().at_least(260.0))
                .size(Size::exact(DETAILS_WIDTH))
                .clip(true)
                .horizontal(|mut strip| {
                    strip.cell(|ui| {
                        requested_page =
                            requested_page.or(self.wiki_tree(ui, localization, bundle));
                    });
                    strip.cell(|ui| {
                        requested_page =
                            requested_page.or(self.wiki_page(ui, localization, bundle));
                    });
                    strip.cell(|ui| {
                        requested_page =
                            requested_page.or(self.wiki_details(ui, localization, bundle));
                    });
                });
        }
        requested_page
    }

    fn wiki_tree(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        bundle: &KnowledgeBundleView,
    ) -> Option<KnowledgePageId> {
        let mut requested = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            ui.heading(localization.text("knowledge-pages"));
            ui.add(
                egui::TextEdit::singleline(&mut self.query_filter)
                    .hint_text(localization.text("knowledge-filter-pages")),
            );

            let (types, tags) = filter_values(bundle);
            normalize_filter(&mut self.type_filter, &types);
            normalize_filter(&mut self.tag_filter, &tags);
            ui.horizontal(|ui| {
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
            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt("knowledge_tree")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    if page_button(
                        ui,
                        localization,
                        bundle.index_fingerprint.is_some(),
                        self.selected_page == Some(KnowledgePageId::Index),
                        "⌂  index.md",
                    ) {
                        requested = Some(KnowledgePageId::Index);
                    }
                    if page_button(
                        ui,
                        localization,
                        bundle.log_fingerprint.is_some(),
                        self.selected_page == Some(KnowledgePageId::Log),
                        "≡  log.md",
                    ) {
                        requested = Some(KnowledgePageId::Log);
                    }
                    ui.add_space(6.0);

                    let filtered = filtered_concepts(
                        bundle,
                        &self.query_filter,
                        self.type_filter.as_deref(),
                        self.tag_filter.as_deref(),
                    );
                    let mut grouped = BTreeMap::<String, Vec<&KnowledgeConceptView>>::new();
                    for concept in filtered {
                        grouped
                            .entry(concept.concept_type.to_string())
                            .or_default()
                            .push(concept);
                    }
                    for (concept_type, concepts) in grouped {
                        egui::CollapsingHeader::new(format!(
                            "{concept_type}  ({})",
                            concepts.len()
                        ))
                        .default_open(true)
                        .show(ui, |ui| {
                            for concept in concepts {
                                let page_id = KnowledgePageId::Concept(concept.id);
                                if page_button(
                                    ui,
                                    localization,
                                    true,
                                    self.selected_page == Some(page_id),
                                    &concept.title,
                                ) {
                                    requested = Some(page_id);
                                }
                            }
                        });
                    }
                });
        });
        requested
    }

    fn wiki_page(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        bundle: &KnowledgeBundleView,
    ) -> Option<KnowledgePageId> {
        let mut requested = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(ui.available_height());
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

            ui.horizontal(|ui| {
                ui.heading(&page.title);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(short_fingerprint(&page.fingerprint))
                            .monospace()
                            .small()
                            .color(Color32::GRAY),
                    );
                });
            });
            if page.truncated {
                ui.colored_label(
                    Color32::from_rgb(205, 145, 30),
                    localization.text("knowledge-page-truncated"),
                );
            }
            ui.separator();

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
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(ui.available_height());
            let Some(page) = self.page.clone() else {
                ui.label(localization.text("knowledge-details-placeholder"));
                return;
            };
            egui::ScrollArea::vertical()
                .id_salt("knowledge_details")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.heading(localization.text("knowledge-metadata"));
                    for (key, value) in &page.metadata {
                        ui.label(RichText::new(key).small().strong());
                        ui.add(
                            egui::Label::new(RichText::new(value).monospace().small())
                                .selectable(true)
                                .wrap(),
                        );
                        ui.add_space(5.0);
                    }

                    ui.separator();
                    let mut backlink_arguments = fluent_bundle::FluentArgs::new();
                    backlink_arguments.set("count", page.backlinks.len());
                    ui.heading(
                        localization.text_with("knowledge-backlinks", Some(&backlink_arguments)),
                    );
                    if page.backlinks.is_empty() {
                        ui.label(
                            RichText::new(localization.text("knowledge-no-backlinks"))
                                .color(Color32::GRAY),
                        );
                    }
                    for backlink in &page.backlinks {
                        let label = page_label(localization, bundle, *backlink);
                        if ui.link(label).clicked() {
                            requested = Some(*backlink);
                        }
                    }

                    ui.separator();
                    let mut link_arguments = fluent_bundle::FluentArgs::new();
                    link_arguments.set("count", page.outgoing_links.len());
                    ui.heading(localization.text_with("knowledge-links", Some(&link_arguments)));
                    for link in &page.outgoing_links {
                        let (status, color) = link_status(localization, &link.disposition);
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(color, status);
                            ui.label(if link.label.is_empty() {
                                &link.raw_target
                            } else {
                                &link.label
                            });
                        });
                    }
                });
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
        ui.horizontal(|ui| {
            ui.label(localization.text("knowledge-graph-description"));
            ui.separator();
            ui.add(
                egui::TextEdit::singleline(&mut self.query_filter)
                    .desired_width(260.0)
                    .hint_text(localization.text("knowledge-filter-concepts")),
            );
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

        let filtered_count = filtered_concepts(
            bundle,
            &self.query_filter,
            self.type_filter.as_deref(),
            self.tag_filter.as_deref(),
        )
        .len();
        if graph_requires_filter(filtered_count) {
            self.graph = None;
            let mut arguments = fluent_bundle::FluentArgs::new();
            arguments.set("count", filtered_count);
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
        let mut layout_advanced = cache.layout.advance(&mut cache.graph) > 0;

        ui.horizontal(|ui| {
            let mut graph_arguments = fluent_bundle::FluentArgs::new();
            graph_arguments.set("nodes", cache.graph.node_count());
            graph_arguments.set("links", cache.graph.edge_count());
            ui.label(localization.text_with("knowledge-graph-counts", Some(&graph_arguments)));
            if !cache.layout.stable {
                ui.spinner();
                let mut arguments = fluent_bundle::FluentArgs::new();
                arguments.set("milliseconds", GRAPH_LAYOUT_FRAME_BUDGET.as_millis() as u64);
                ui.label(localization.text_with("knowledge-graph-organizing", Some(&arguments)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(localization.text("knowledge-graph-reset"))
                    .clicked()
                {
                    reset_metadata(ui, Some(cache.widget_id.clone()));
                    cache.layout = IncrementalGraphLayout::default();
                    layout_advanced = true;
                }
            });
        });
        ui.separator();

        let interactions = SettingsInteraction::default()
            .with_dragging_enabled(true)
            .with_node_selection_enabled(true);
        let navigation = SettingsNavigation::default()
            .with_fit_to_screen_enabled(layout_advanced)
            .with_zoom_and_pan_enabled(true)
            .with_fit_to_screen_padding(0.08);
        let style = SettingsStyle::default()
            .with_labels_always(cache.graph.node_count() <= 120)
            .with_node_stroke_hook(|selected, _, color, mut stroke, _| {
                if let Some(color) = color {
                    stroke.color = color;
                }
                if selected {
                    stroke.width = 3.0;
                }
                stroke
            });
        let graph_response = {
            // Node positions are computed incrementally above. Keep egui_graphs' built-in
            // hierarchical pass disabled so it cannot replace them with one monolithic step.
            set_layout_state(
                ui,
                LayoutStateHierarchical {
                    triggered: true,
                    ..LayoutStateHierarchical::default()
                },
                Some(cache.widget_id.clone()),
            );
            let mut graph_view =
                GraphView::<_, _, _, _, _, _, LayoutStateHierarchical, LayoutHierarchical>::new(
                    &mut cache.graph,
                )
                .with_id(Some(cache.widget_id.clone()))
                .with_interactions(&interactions)
                .with_navigations(&navigation)
                .with_styles(&style);
            ui.add(&mut graph_view)
        };
        if let Some(payload) = cache
            .graph
            .hovered_node()
            .and_then(|index| cache.graph.node(index))
            .map(|node| node.payload().clone())
        {
            graph_response.on_hover_ui(|ui| {
                ui.label(RichText::new(payload.title).strong());
                ui.label(payload.concept_type);
                if !payload.tags.is_empty() {
                    ui.label(
                        RichText::new(payload.tags.join(", "))
                            .small()
                            .color(Color32::GRAY),
                    );
                }
            });
        }

        let selected = cache.graph.selected_nodes().last().copied();
        let page_id =
            selected.and_then(|index| cache.graph.node(index).map(|node| node.payload().page_id));
        if page_id.is_some() {
            cache.graph.set_selected_nodes(Vec::new());
        }
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
                Color32::from_rgb(205, 145, 30),
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
                Color32::from_rgb(80, 145, 205),
            );
            health_card(
                ui,
                &localization.text("knowledge-health-warnings"),
                report.warning_count,
                Color32::from_rgb(205, 145, 30),
            );
            health_card(
                ui,
                &localization.text("knowledge-health-errors"),
                report.error_count,
                Color32::from_rgb(220, 75, 75),
            );
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(localization.text("knowledge-health-last-check"))
                        .small()
                        .color(Color32::GRAY),
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
        if history_recovery {
            ui.colored_label(
                Color32::from_rgb(205, 145, 30),
                localization.text("knowledge-repair-history-blocked"),
            );
            ui.label(
                RichText::new(localization.text("knowledge-repair-history-body"))
                    .small()
                    .color(Color32::GRAY),
            );
        }
        if let Some((_, error)) = self
            .guided_repair_error
            .as_ref()
            .filter(|(collection_id, _)| *collection_id == bundle.collection_id)
        {
            ui.colored_label(
                Color32::from_rgb(220, 75, 75),
                localized_guided_repair_error(localization, error),
            );
            ui.collapsing(localization.text("action-details"), |ui| {
                ui.monospace(error);
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
                Color32::from_rgb(70, 160, 110),
                localization.text_with("knowledge-repair-complete", Some(&arguments)),
            );
        }
        let mut repair_action = None;
        if content_repair && !history_recovery {
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
                    .color(Color32::GRAY),
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
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        let (label, color) = severity_visual(localization, &issue.severity);
                        ui.horizontal(|ui| {
                            ui.colored_label(color, RichText::new(label).strong());
                            if let Some(page_id) = issue.page {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .link(page_label(localization, bundle, page_id))
                                            .clicked()
                                        {
                                            requested = Some(page_id);
                                        }
                                    },
                                );
                            }
                        });
                        ui.label(health_issue_summary(localization, &issue.code));
                        ui.collapsing(localization.text("action-details"), |ui| {
                            ui.monospace(&issue.code);
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
                    Color32::from_rgb(205, 145, 30),
                    RichText::new(localization.text("knowledge-repair-confirm-warning")).strong(),
                );
                ui.label(localization.text("knowledge-repair-confirm-body"));
                ui.add_space(8.0);
                ui.label(
                    RichText::new(localization.text("knowledge-repair-authority-title")).strong(),
                );
                for authority in &preview.authorities {
                    ui.label(format!(
                        "• {}",
                        localized_repair_authority(localization, *authority)
                    ));
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(localization.text("knowledge-repair-changes-title")).strong(),
                );
                egui::ScrollArea::vertical()
                    .id_salt("knowledge_guided_repair_files")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for file in &preview.files {
                            ui.horizontal_wrapped(|ui| {
                                ui.monospace(file.page.relative_path());
                                ui.label("—");
                                ui.label(localized_repair_change(localization, file.change));
                            });
                        }
                    });
                ui.add_space(8.0);
                ui.label(
                    RichText::new(localization.text("knowledge-repair-snapshot-note"))
                        .small()
                        .color(Color32::GRAY),
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
        self.page_recovery_attempted = false;
        self.queue_page(page_id)
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
        .set_color(Color32::from_rgb(70, 150, 215));
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
            .set_color(concept_color(&concept.concept_type.to_string()));
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

    let widget_id = format!("knowledge-graph-{key}");
    KnowledgeGraphCache {
        key,
        widget_id,
        graph,
        layout: IncrementalGraphLayout::default(),
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
        _ => notice.to_owned(),
    }
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
        issue.severity != HealthSeverity::Info
            && (issue.page == Some(KnowledgePageId::Log)
                || issue.code.contains("history")
                || issue.code.contains("log"))
    })
}

fn health_has_guided_content_repair(bundle: &KnowledgeBundleView) -> bool {
    const DERIVED_INDEX_CODES: &[&str] = &[
        "broken_index_link",
        "index_missing_concept",
        "invalid_index_structure",
        "missing_index",
        "stale_index_metadata",
    ];
    bundle.health.issues.iter().any(|issue| {
        issue.severity != HealthSeverity::Info
            && !DERIVED_INDEX_CODES.contains(&issue.code.as_str())
            && issue.page != Some(KnowledgePageId::Log)
            && !issue.code.contains("history")
            && !issue.code.contains("log")
    })
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

fn short_fingerprint(value: &str) -> String {
    value.chars().take(12).collect()
}

fn tab_button(ui: &mut egui::Ui, selected: &mut KnowledgeTab, value: KnowledgeTab, label: &str) {
    if ui
        .add_sized(
            [96.0, 30.0],
            egui::Button::selectable(*selected == value, label),
        )
        .clicked()
    {
        *selected = value;
    }
}

fn page_button(
    ui: &mut egui::Ui,
    localization: &Localization,
    enabled: bool,
    selected: bool,
    label: &str,
) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::selectable(selected, truncate_chars(label, 44))
            .min_size(egui::vec2(ui.available_width(), 27.0)),
    )
    .on_disabled_hover_text(localization.text("knowledge-page-missing"))
    .on_hover_text(label)
    .clicked()
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

fn bundle_state_badge(ui: &mut egui::Ui, localization: &Localization, state: KnowledgeBundleState) {
    let (message, color) = match state {
        KnowledgeBundleState::Empty => ("knowledge-state-empty", Color32::GRAY),
        KnowledgeBundleState::Ready => ("knowledge-state-ready", Color32::from_rgb(70, 170, 110)),
        KnowledgeBundleState::Updating => {
            ("knowledge-state-updating", Color32::from_rgb(205, 145, 30))
        }
    };
    let label = localization.text(message);
    ui.colored_label(color, RichText::new(format!("● {label}")).strong());
}

fn empty_bundle_has_health_findings(bundle: &KnowledgeBundleView) -> bool {
    matches!(bundle.state, KnowledgeBundleState::Empty) && !bundle.health.issues.is_empty()
}

fn link_status(
    localization: &Localization,
    disposition: &KnowledgeLinkDisposition,
) -> (String, Color32) {
    let (message, color) = match disposition {
        KnowledgeLinkDisposition::Internal(_) => {
            ("knowledge-link-internal", Color32::from_rgb(70, 170, 110))
        }
        KnowledgeLinkDisposition::External => {
            ("knowledge-link-external", Color32::from_rgb(80, 145, 205))
        }
        KnowledgeLinkDisposition::Broken => (
            "knowledge-link-broken-status",
            Color32::from_rgb(205, 145, 30),
        ),
        KnowledgeLinkDisposition::Unsafe => (
            "knowledge-link-blocked-status",
            Color32::from_rgb(220, 75, 75),
        ),
    };
    (localization.text(message), color)
}

fn severity_visual(localization: &Localization, severity: &HealthSeverity) -> (String, Color32) {
    let (message, color) = match severity {
        HealthSeverity::Error => ("knowledge-severity-error", Color32::from_rgb(220, 75, 75)),
        HealthSeverity::Warning => (
            "knowledge-severity-warning",
            Color32::from_rgb(205, 145, 30),
        ),
        HealthSeverity::Info => ("knowledge-severity-info", Color32::from_rgb(80, 145, 205)),
    };
    (localization.text(message), color)
}

fn concept_color(concept_type: &str) -> Color32 {
    match concept_type {
        "Policy" => Color32::from_rgb(190, 105, 95),
        "Procedure" => Color32::from_rgb(75, 165, 130),
        "Runbook" => Color32::from_rgb(220, 145, 65),
        "Reference" => Color32::from_rgb(115, 135, 205),
        "Report" => Color32::from_rgb(155, 105, 190),
        _ => Color32::from_rgb(100, 145, 175),
    }
}

fn health_card(ui: &mut egui::Ui, label: &str, value: usize, color: Color32) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(130.0);
        ui.label(RichText::new(value.to_string()).size(25.0).color(color));
        ui.label(RichText::new(label).small().color(Color32::GRAY));
    });
}

fn empty_state(ui: &mut egui::Ui, title: &str, body: &str) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.heading(title);
            ui.label(RichText::new(body).color(Color32::GRAY));
        });
    });
}

fn error_state(ui: &mut egui::Ui, localization: &Localization, title: &str, error: &str) {
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.colored_label(
                Color32::from_rgb(220, 75, 75),
                RichText::new(title).size(20.0).strong(),
            );
            ui.label(localized_knowledge_error(localization, error));
            if error != "knowledge-error-wrong-collection" {
                ui.collapsing(localization.text("action-details"), |ui| {
                    ui.label(error);
                });
            }
        });
    });
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
        GuidedRepairResult, HealthIssue, HealthSeverity, KnowledgeBundleState, KnowledgeBundleView,
        KnowledgeConceptView, KnowledgeLinkDisposition, KnowledgeLinkView, KnowledgePageId,
        KnowledgePageView, RepairAuthority, RepairPlanId,
    };
    use airwiki_types::CollectionPolicy;
    use uuid::Uuid;

    use crate::i18n::{Localization, UiLocale};

    use super::{
        GRAPH_LAYOUT_FRAME_BUDGET, GRAPH_LAYOUT_WORK_BUDGET, KnowledgeAction, KnowledgeUi,
        build_graph, deterministic_graph_position, empty_bundle_has_health_findings,
        graph_requires_filter, normalized_http_url, short_fingerprint, truncate_chars,
    };

    fn localization() -> Localization {
        Localization::new(UiLocale::EnUs).unwrap()
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
    fn fingerprint_preview_is_unicode_safe() {
        assert_eq!(short_fingerprint("áβ猫0123456789abc"), "áβ猫012345678");
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
        assert!(GRAPH_LAYOUT_WORK_BUDGET < GRAPH_LAYOUT_FRAME_BUDGET);
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
