use std::{collections::BTreeMap, fmt};

use airwiki_core::ReviewVersionToken;
use eframe::egui::{self, RichText};
use fluent_bundle::FluentArgs;
use uuid::Uuid;

use crate::i18n::Localization;
use crate::worker::{ReviewEvidenceErrorView, ReviewEvidencePageView};

const NARROW_REVIEW_THRESHOLD: f32 = 760.0;
pub(super) const REVIEW_QUEUE_WIDTH: f32 = 250.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReviewLayoutMode {
    CompactCompare,
    QueueCompare,
}

pub(super) const fn review_layout_mode(available_width: f32) -> ReviewLayoutMode {
    if available_width < NARROW_REVIEW_THRESHOLD {
        ReviewLayoutMode::CompactCompare
    } else {
        ReviewLayoutMode::QueueCompare
    }
}

pub(super) const fn review_action_bar_height(mode: ReviewLayoutMode) -> f32 {
    match mode {
        ReviewLayoutMode::CompactCompare => 124.0,
        ReviewLayoutMode::QueueCompare => 94.0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewEvidenceAction {
    pub(super) request_id: Uuid,
    pub(super) concept_id: Uuid,
    pub(super) expected_source_revision: u32,
    pub(super) expected_review_version: Option<ReviewVersionToken>,
    pub(super) after_ordinal: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReviewSelection {
    concept_id: Uuid,
    source_revision: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingEvidence {
    action: ReviewEvidenceAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FailedEvidence {
    selection: ReviewSelection,
    after_ordinal: Option<u32>,
    error: ReviewEvidenceErrorView,
}

#[derive(Default)]
pub(super) struct ReviewEvidenceUi {
    selection: Option<ReviewSelection>,
    reanalyzing: bool,
    pending: Option<PendingEvidence>,
    page: Option<ReviewEvidencePageView>,
    failure: Option<FailedEvidence>,
}

impl fmt::Debug for ReviewEvidenceUi {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReviewEvidenceUi")
            .field("has_selection", &self.selection.is_some())
            .field("reanalyzing", &self.reanalyzing)
            .field("loading", &self.pending.is_some())
            .field(
                "loaded_excerpt_count",
                &self.page.as_ref().map_or(0, |page| page.excerpts.len()),
            )
            .field("has_error", &self.failure.is_some())
            .finish()
    }
}

impl ReviewEvidenceUi {
    pub(super) fn sync_selection(
        &mut self,
        selection: Option<(Uuid, u32)>,
        reanalyzing: bool,
    ) -> Option<ReviewEvidenceAction> {
        let selection = selection.map(|(concept_id, source_revision)| ReviewSelection {
            concept_id,
            source_revision,
        });
        if self.selection != selection {
            self.selection = selection;
            self.reanalyzing = reanalyzing;
            self.clear_loaded_state();
            return (!reanalyzing).then(|| self.start_request(None)).flatten();
        }

        let reanalysis_finished = self.reanalyzing && !reanalyzing;
        if !self.reanalyzing && reanalyzing {
            // A response for the pre-reanalysis draft must not become visible
            // after the worker starts replacing its evidence.
            self.clear_loaded_state();
        }
        self.reanalyzing = reanalyzing;

        if reanalysis_finished {
            self.clear_loaded_state();
            return self.start_request(None);
        }
        if reanalyzing || self.page.is_some() || self.pending.is_some() || self.failure.is_some() {
            return None;
        }
        self.start_request(None)
    }

    pub(super) fn request_more(&mut self) -> Option<ReviewEvidenceAction> {
        if self.reanalyzing || self.pending.is_some() || self.failure.is_some() {
            return None;
        }
        let selection = self.selection?;
        let page = self.page_for(selection.concept_id, selection.source_revision)?;
        let after_ordinal = page.next_ordinal?;
        if page.excerpts.last().map(|excerpt| excerpt.ordinal) != Some(after_ordinal) {
            return None;
        }
        self.start_request(Some(after_ordinal))
    }

    /// Applies the worker lifecycle immediately, even when the Review screen
    /// is not rendered. This prevents a complete off-screen reanalysis cycle
    /// from leaving the previous evidence approvable.
    pub(super) fn reanalysis_changed(
        &mut self,
        concept_id: Uuid,
        running: bool,
    ) -> Option<ReviewEvidenceAction> {
        let selection = self
            .selection
            .filter(|selection| selection.concept_id == concept_id)?;
        if running {
            self.reanalyzing = true;
            self.clear_loaded_state();
            return None;
        }

        let should_reload = self.reanalyzing;
        self.reanalyzing = false;
        if should_reload {
            self.clear_loaded_state();
            self.selection = Some(selection);
            return self.start_request(None);
        }
        None
    }

    pub(super) fn retry(&mut self) -> Option<ReviewEvidenceAction> {
        if self.reanalyzing || self.pending.is_some() {
            return None;
        }
        let selection = self.selection?;
        let failure = self
            .failure
            .filter(|failure| failure.selection == selection)?;
        self.failure = None;
        let can_resume_pagination = failure.after_ordinal.is_some()
            && failure.error == ReviewEvidenceErrorView::Unavailable
            && self
                .page_for(selection.concept_id, selection.source_revision)
                .is_some();
        if failure.after_ordinal.is_some() && !can_resume_pagination {
            self.page = None;
            return self.start_request(None);
        }
        self.start_request(failure.after_ordinal)
    }

    pub(super) fn apply_loaded(
        &mut self,
        request_id: Uuid,
        concept_id: Uuid,
        expected_source_revision: u32,
        result: Result<ReviewEvidencePageView, ReviewEvidenceErrorView>,
    ) -> bool {
        let Some(pending) = self.pending.clone() else {
            return false;
        };
        let response_matches = pending.action.request_id == request_id
            && pending.action.concept_id == concept_id
            && pending.action.expected_source_revision == expected_source_revision;
        let selection = ReviewSelection {
            concept_id,
            source_revision: expected_source_revision,
        };
        if !response_matches || self.selection != Some(selection) || self.reanalyzing {
            return false;
        }
        self.pending = None;

        match result {
            Ok(page)
                if page.concept_id == concept_id
                    && page.source_revision == expected_source_revision
                    && pending
                        .action
                        .expected_review_version
                        .as_ref()
                        .is_none_or(|expected| expected == &page.review_version) =>
            {
                if !self.merge_page(page, pending.action.after_ordinal) {
                    self.page = None;
                    self.failure = Some(FailedEvidence {
                        selection,
                        after_ordinal: pending.action.after_ordinal,
                        error: ReviewEvidenceErrorView::Unavailable,
                    });
                    return true;
                }
                self.failure = None;
                true
            }
            Ok(_) => {
                // A structurally incoherent response must fail closed. In
                // particular, a load-more response cannot leave an older page
                // visible and silently reactivate approval.
                self.page = None;
                self.failure = Some(FailedEvidence {
                    selection,
                    after_ordinal: pending.action.after_ordinal,
                    error: ReviewEvidenceErrorView::Unavailable,
                });
                true
            }
            Err(error) => {
                if error == ReviewEvidenceErrorView::NoLongerPending
                    && pending.action.after_ordinal.is_some()
                {
                    self.page = None;
                }
                self.failure = Some(FailedEvidence {
                    selection,
                    after_ordinal: pending.action.after_ordinal,
                    error,
                });
                true
            }
        }
    }

    pub(super) fn page_for(
        &self,
        concept_id: Uuid,
        source_revision: u32,
    ) -> Option<&ReviewEvidencePageView> {
        self.page
            .as_ref()
            .filter(|page| page.concept_id == concept_id && page.source_revision == source_revision)
    }

    pub(super) fn error_for(
        &self,
        concept_id: Uuid,
        source_revision: u32,
    ) -> Option<ReviewEvidenceErrorView> {
        let selection = ReviewSelection {
            concept_id,
            source_revision,
        };
        self.failure
            .filter(|failure| failure.selection == selection)
            .map(|failure| failure.error)
    }

    pub(super) fn is_loading(&self, concept_id: Uuid, source_revision: u32) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.action.concept_id == concept_id
                && pending.action.expected_source_revision == source_revision
        })
    }

    pub(super) fn approval_ready(&self, concept_id: Uuid, source_revision: u32) -> bool {
        !self.reanalyzing
            && !self.is_loading(concept_id, source_revision)
            && self.error_for(concept_id, source_revision).is_none()
            && self
                .page_for(concept_id, source_revision)
                .is_some_and(|page| !page.excerpts.is_empty())
    }

    pub(super) fn approval_version(
        &self,
        concept_id: Uuid,
        source_revision: u32,
    ) -> Option<ReviewVersionToken> {
        self.approval_ready(concept_id, source_revision)
            .then(|| {
                self.page_for(concept_id, source_revision)
                    .map(|page| page.review_version.clone())
            })
            .flatten()
    }

    fn start_request(&mut self, after_ordinal: Option<u32>) -> Option<ReviewEvidenceAction> {
        let selection = self.selection?;
        let expected_review_version = after_ordinal.and_then(|_| {
            self.page_for(selection.concept_id, selection.source_revision)
                .map(|page| page.review_version.clone())
        });
        if after_ordinal.is_some() && expected_review_version.is_none() {
            return None;
        }
        let action = ReviewEvidenceAction {
            request_id: Uuid::new_v4(),
            concept_id: selection.concept_id,
            expected_source_revision: selection.source_revision,
            expected_review_version,
            after_ordinal,
        };
        self.pending = Some(PendingEvidence {
            action: action.clone(),
        });
        Some(action)
    }

    fn clear_loaded_state(&mut self) {
        self.pending = None;
        self.page = None;
        self.failure = None;
    }

    fn merge_page(
        &mut self,
        mut incoming: ReviewEvidencePageView,
        after_ordinal: Option<u32>,
    ) -> bool {
        if !normalize_and_validate_page(&mut incoming, after_ordinal) {
            return false;
        }
        if after_ordinal.is_none() {
            self.page = Some(incoming);
            return true;
        }

        let Some(after_ordinal) = after_ordinal else {
            return false;
        };
        let Some(current) = self.page.as_mut().filter(|current| {
            current.concept_id == incoming.concept_id
                && current.source_revision == incoming.source_revision
                && current.review_version == incoming.review_version
                && current.total_chunks == incoming.total_chunks
                && current.next_ordinal == Some(after_ordinal)
                && current.excerpts.last().map(|excerpt| excerpt.ordinal) == Some(after_ordinal)
        }) else {
            return false;
        };
        let Some(merged_count) = current.excerpts.len().checked_add(incoming.excerpts.len()) else {
            return false;
        };
        if merged_count > current.total_chunks
            || incoming.next_ordinal.is_none() && merged_count != current.total_chunks
            || incoming.next_ordinal.is_some() && merged_count >= current.total_chunks
        {
            return false;
        }
        current.excerpts.append(&mut incoming.excerpts);
        current.next_ordinal = incoming.next_ordinal;
        true
    }
}

fn normalize_and_validate_page(
    page: &mut ReviewEvidencePageView,
    after_ordinal: Option<u32>,
) -> bool {
    if page.total_chunks == 0 || page.excerpts.is_empty() || page.excerpts.len() > page.total_chunks
    {
        return false;
    }

    let original_len = page.excerpts.len();
    let mut ordered = BTreeMap::new();
    for excerpt in page.excerpts.drain(..) {
        ordered.entry(excerpt.ordinal).or_insert(excerpt);
    }
    page.excerpts = ordered.into_values().collect();
    if page.excerpts.len() != original_len
        || after_ordinal.is_some_and(|cursor| {
            page.excerpts
                .first()
                .is_none_or(|excerpt| excerpt.ordinal <= cursor)
        })
        || page.next_ordinal.is_some_and(|cursor| {
            page.excerpts.last().map(|excerpt| excerpt.ordinal) != Some(cursor)
        })
    {
        return false;
    }

    if after_ordinal.is_none() {
        match page.next_ordinal {
            Some(_) => page.excerpts.len() < page.total_chunks,
            None => page.excerpts.len() == page.total_chunks,
        }
    } else {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReviewEvidencePanelIntent {
    LoadMore,
    Retry,
}

pub(super) fn show_review_evidence_panel(
    ui: &mut egui::Ui,
    localization: &Localization,
    concept_id: Uuid,
    source_revision: u32,
    page: Option<&ReviewEvidencePageView>,
    error: Option<ReviewEvidenceErrorView>,
    loading: bool,
) -> Option<ReviewEvidencePanelIntent> {
    ui.add(
        egui::Label::new(
            RichText::new(localization.text("review-evidence-editorial-title"))
                .size(13.0)
                .family(crate::theme::semibold_font_family())
                .color(crate::theme::secondary_text(ui.visuals().dark_mode)),
        )
        .wrap(),
    );

    let mut intent = None;
    ui.push_id(("review_evidence", concept_id, source_revision), |ui| {
        if let Some(page) = page {
            for excerpt in &page.excerpts {
                ui.add_space(8.0);
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 24,
                        ..egui::Margin::ZERO
                    })
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("“{}”", excerpt.text)).italics(),
                            )
                            .wrap()
                            .selectable(true),
                        );
                        let heading = if excerpt.heading_or_page.trim().is_empty() {
                            localization.text("review-evidence-untitled-section")
                        } else {
                            excerpt.heading_or_page.clone()
                        };
                        let mut revision_args = FluentArgs::new();
                        revision_args.set("revision", i64::from(source_revision));
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                heading,
                                localization
                                    .text_with("review-evidence-revision", Some(&revision_args))
                            ))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                        );
                        if excerpt.truncated {
                            ui.label(
                                RichText::new(localization.text("review-evidence-truncated"))
                                    .small()
                                    .italics()
                                    .color(ui.visuals().weak_text_color()),
                            );
                        }
                    });
            }

            let mut progress_args = FluentArgs::new();
            progress_args.set("shown", page.excerpts.len() as i64);
            progress_args.set("total", page.total_chunks as i64);
            ui.label(
                RichText::new(
                    localization.text_with("review-evidence-progress", Some(&progress_args)),
                )
                .small()
                .color(ui.visuals().weak_text_color()),
            );
        }

        if loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(localization.text("review-evidence-loading"));
            });
        } else if let Some(error) = error {
            let message_id = match error {
                ReviewEvidenceErrorView::NoLongerPending => "review-evidence-no-longer-pending",
                ReviewEvidenceErrorView::MissingEvidence => "review-evidence-missing",
                ReviewEvidenceErrorView::Unavailable => "review-evidence-unavailable",
            };
            ui.colored_label(
                crate::theme::warning_text(ui.visuals().dark_mode),
                localization.text(message_id),
            );
            if ui
                .button(localization.text("review-evidence-retry"))
                .clicked()
            {
                intent = Some(ReviewEvidencePanelIntent::Retry);
            }
        } else if page.is_some_and(|page| page.next_ordinal.is_some())
            && ui
                .button(localization.text("review-evidence-load-more"))
                .clicked()
        {
            intent = Some(ReviewEvidencePanelIntent::LoadMore);
        }
    });
    intent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::ReviewEvidenceExcerptView;

    #[test]
    fn width_below_threshold_uses_compact_compare_layout() {
        assert_eq!(review_layout_mode(759.0), ReviewLayoutMode::CompactCompare);
    }

    #[test]
    fn threshold_width_keeps_the_queue_visible() {
        assert_eq!(review_layout_mode(760.0), ReviewLayoutMode::QueueCompare);
    }

    #[test]
    fn compact_review_footer_reserves_space_for_wrapped_actions() {
        assert_eq!(
            review_action_bar_height(ReviewLayoutMode::CompactCompare),
            124.0
        );
        assert_eq!(
            review_action_bar_height(ReviewLayoutMode::QueueCompare),
            94.0
        );
    }

    #[test]
    fn canonical_queue_keeps_wide_reader_footer_after_column_split() {
        let canonical_content_width = 970.0;
        let mode = review_layout_mode(canonical_content_width);
        let reader_width = canonical_content_width - REVIEW_QUEUE_WIDTH;

        assert_eq!(reader_width, 720.0);
        assert!(reader_width < NARROW_REVIEW_THRESHOLD);
        assert_eq!(mode, ReviewLayoutMode::QueueCompare);
        assert_eq!(review_action_bar_height(mode), 94.0);
    }

    #[test]
    fn approval_remains_closed_while_evidence_is_loading() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();

        let action = ui.sync_selection(Some((concept_id, 1)), false);

        assert!(action.is_some() && !ui.approval_ready(concept_id, 1));
    }

    #[test]
    fn approval_remains_closed_when_evidence_is_missing() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let action = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should request evidence");

        ui.apply_loaded(
            action.request_id,
            concept_id,
            1,
            Err(ReviewEvidenceErrorView::MissingEvidence),
        );

        assert!(!ui.approval_ready(concept_id, 1));
    }

    #[test]
    fn approval_opens_after_nonempty_evidence_loads() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let action = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should request evidence");

        ui.apply_loaded(
            action.request_id,
            concept_id,
            1,
            Ok(page(concept_id, 1, &[(0, "evidence")])),
        );

        assert!(ui.approval_ready(concept_id, 1));
    }

    #[test]
    fn response_for_previous_selection_is_ignored() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let old = ui
            .sync_selection(Some((first, 1)), false)
            .expect("first selection should load");
        ui.sync_selection(Some((second, 1)), false);

        let applied = ui.apply_loaded(
            old.request_id,
            first,
            1,
            Ok(page(first, 1, &[(0, "stale")])),
        );

        assert!(!applied && ui.page_for(first, 1).is_none());
    }

    #[test]
    fn response_for_previous_revision_is_ignored() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let old = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("first revision should load");
        ui.sync_selection(Some((concept_id, 2)), false);

        let applied = ui.apply_loaded(
            old.request_id,
            concept_id,
            1,
            Ok(page(concept_id, 1, &[(0, "stale")])),
        );

        assert!(!applied && ui.page_for(concept_id, 1).is_none());
    }

    #[test]
    fn pagination_merge_orders_complete_pages() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let initial = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should load");
        let mut first_page = page(concept_id, 1, &[(1, "one"), (0, "zero")]);
        first_page.total_chunks = 4;
        first_page.next_ordinal = Some(1);
        assert!(ui.apply_loaded(initial.request_id, concept_id, 1, Ok(first_page)));
        let more = ui.request_more().expect("another page should be available");
        let mut second_page = page(concept_id, 1, &[(3, "three"), (2, "two")]);
        second_page.total_chunks = 4;

        assert!(ui.apply_loaded(more.request_id, concept_id, 1, Ok(second_page)));
        let ordinals = ui
            .page_for(concept_id, 1)
            .expect("merged evidence should remain visible")
            .excerpts
            .iter()
            .map(|excerpt| excerpt.ordinal)
            .collect::<Vec<_>>();
        assert_eq!(ordinals, vec![0, 1, 2, 3]);
    }

    #[test]
    fn same_token_page_at_cursor_fails_closed_and_retry_starts_fresh() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let initial = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should load");
        let mut first = page(concept_id, 1, &[(0, "first")]);
        first.total_chunks = 2;
        first.next_ordinal = Some(0);
        assert!(ui.apply_loaded(initial.request_id, concept_id, 1, Ok(first)));
        let more = ui.request_more().expect("next page should be requested");
        let mut repeated = page(concept_id, 1, &[(0, "repeated")]);
        repeated.total_chunks = 2;

        assert!(ui.apply_loaded(more.request_id, concept_id, 1, Ok(repeated)));
        assert!(ui.page_for(concept_id, 1).is_none());
        assert_eq!(
            ui.error_for(concept_id, 1),
            Some(ReviewEvidenceErrorView::Unavailable)
        );
        let retry = ui
            .retry()
            .expect("structural failure should reload from the start");
        assert!(retry.after_ordinal.is_none());
        assert!(retry.expected_review_version.is_none());
    }

    #[test]
    fn same_token_page_with_changed_total_fails_closed() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let initial = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should load");
        let mut first = page(concept_id, 1, &[(0, "first")]);
        first.total_chunks = 2;
        first.next_ordinal = Some(0);
        assert!(ui.apply_loaded(initial.request_id, concept_id, 1, Ok(first)));
        let more = ui.request_more().expect("next page should be requested");
        let mut changed = page(concept_id, 1, &[(1, "second")]);
        changed.total_chunks = 3;

        assert!(ui.apply_loaded(more.request_id, concept_id, 1, Ok(changed)));
        assert!(ui.page_for(concept_id, 1).is_none());
        assert!(!ui.approval_ready(concept_id, 1));
    }

    #[test]
    fn stale_load_more_retry_restarts_without_cursor_or_token() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let initial = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should load");
        let mut first = page(concept_id, 1, &[(0, "first")]);
        first.total_chunks = 2;
        first.next_ordinal = Some(0);
        assert!(ui.apply_loaded(initial.request_id, concept_id, 1, Ok(first)));
        let more = ui.request_more().expect("next page should be requested");

        assert!(ui.apply_loaded(
            more.request_id,
            concept_id,
            1,
            Err(ReviewEvidenceErrorView::NoLongerPending),
        ));
        let retry = ui.retry().expect("stale pagination should restart");
        assert!(retry.after_ordinal.is_none());
        assert!(retry.expected_review_version.is_none());
    }

    #[test]
    fn transient_load_more_retry_preserves_cursor_and_token() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let initial = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should load");
        let mut first = page(concept_id, 1, &[(0, "first")]);
        first.total_chunks = 2;
        first.next_ordinal = Some(0);
        let expected_review_version = first.review_version.clone();
        assert!(ui.apply_loaded(initial.request_id, concept_id, 1, Ok(first)));
        let more = ui.request_more().expect("next page should be requested");

        assert!(ui.apply_loaded(
            more.request_id,
            concept_id,
            1,
            Err(ReviewEvidenceErrorView::Unavailable),
        ));
        let retry = ui
            .retry()
            .expect("transient failure should resume pagination");
        assert_eq!(retry.after_ordinal, Some(0));
        assert_eq!(retry.expected_review_version, Some(expected_review_version));
    }

    #[test]
    fn completed_reanalysis_requests_fresh_evidence() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let initial = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should load");
        assert!(ui.apply_loaded(
            initial.request_id,
            concept_id,
            1,
            Ok(page(concept_id, 1, &[(0, "old")])),
        ));
        assert!(ui.sync_selection(Some((concept_id, 1)), true).is_none());
        assert!(ui.page_for(concept_id, 1).is_none());
        assert!(!ui.approval_ready(concept_id, 1));

        let refreshed = ui.sync_selection(Some((concept_id, 1)), false);

        assert!(refreshed.is_some() && ui.page_for(concept_id, 1).is_none());
    }

    #[test]
    fn offscreen_reanalysis_cycle_invalidates_old_evidence_and_reloads() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let initial = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should load");
        assert!(ui.apply_loaded(
            initial.request_id,
            concept_id,
            1,
            Ok(page(concept_id, 1, &[(0, "old")]))
        ));

        assert!(ui.reanalysis_changed(concept_id, true).is_none());
        let refreshed = ui
            .reanalysis_changed(concept_id, false)
            .expect("completion should request the replacement evidence");

        assert!(refreshed.expected_review_version.is_none());
        assert!(ui.page_for(concept_id, 1).is_none());
        assert!(!ui.approval_ready(concept_id, 1));
    }

    #[test]
    fn incoherent_load_more_response_fails_closed() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let initial = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should load");
        let mut first = page(concept_id, 1, &[(0, "first")]);
        first.total_chunks = 2;
        first.next_ordinal = Some(0);
        assert!(ui.apply_loaded(initial.request_id, concept_id, 1, Ok(first)));
        let more = ui.request_more().expect("next page should be requested");
        let mut mismatched = page(concept_id, 1, &[(1, "second")]);
        mismatched.review_version = ReviewVersionToken::from_digest([8; 32]);

        assert!(ui.apply_loaded(more.request_id, concept_id, 1, Ok(mismatched)));
        assert!(ui.page_for(concept_id, 1).is_none());
        assert_eq!(
            ui.error_for(concept_id, 1),
            Some(ReviewEvidenceErrorView::Unavailable)
        );
        assert!(!ui.approval_ready(concept_id, 1));
    }

    #[test]
    fn approval_returns_the_exact_loaded_review_version() {
        let concept_id = Uuid::new_v4();
        let mut ui = ReviewEvidenceUi::default();
        let initial = ui
            .sync_selection(Some((concept_id, 1)), false)
            .expect("selection should load");
        let loaded = page(concept_id, 1, &[(0, "evidence")]);
        let expected = loaded.review_version.clone();
        assert!(ui.apply_loaded(initial.request_id, concept_id, 1, Ok(loaded)));

        assert_eq!(ui.approval_version(concept_id, 1), Some(expected));
    }

    fn page(
        concept_id: Uuid,
        source_revision: u32,
        excerpts: &[(u32, &str)],
    ) -> ReviewEvidencePageView {
        ReviewEvidencePageView {
            concept_id,
            source_revision,
            review_version: ReviewVersionToken::from_digest([7; 32]),
            excerpts: excerpts
                .iter()
                .map(|(ordinal, text)| ReviewEvidenceExcerptView {
                    ordinal: *ordinal,
                    heading_or_page: format!("Section {ordinal}"),
                    text: (*text).to_owned(),
                    truncated: false,
                })
                .collect(),
            total_chunks: excerpts.len(),
            next_ordinal: None,
        }
    }
}
