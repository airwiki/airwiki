use eframe::egui::{self, Color32, RichText, Stroke};
use fluent_bundle::FluentArgs;

use crate::i18n::Localization;
use crate::{layout::LayoutDensity, theme};

pub(super) const AIR_BLUE: Color32 = theme::AIR_BLUE;
const AIR_AQUA: Color32 = theme::EVIDENCE_CYAN;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JourneyStepState {
    Complete,
    Current,
    Upcoming,
    Attention,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProcessingProgress {
    pub documents: usize,
    pub preparing: usize,
    pub ready_for_review: usize,
    pub issues: usize,
}

pub(super) fn processing_progress(
    document_count: usize,
    published_count: usize,
    needs_review_count: usize,
    failed_count: usize,
    visible_issue_count: usize,
) -> ProcessingProgress {
    let preparing = document_count
        .saturating_sub(published_count)
        .saturating_sub(needs_review_count)
        .saturating_sub(failed_count);
    ProcessingProgress {
        documents: document_count,
        preparing,
        ready_for_review: needs_review_count,
        issues: failed_count.max(visible_issue_count),
    }
}

pub(super) fn show_processing_progress(
    ui: &mut egui::Ui,
    localization: &Localization,
    progress: ProcessingProgress,
) {
    let mut arguments = FluentArgs::new();
    arguments.set("documents", progress.documents);
    arguments.set("preparing", progress.preparing);
    arguments.set("ready", progress.ready_for_review);
    arguments.set("issues", progress.issues);
    let color = theme::ink(ui.visuals().dark_mode);
    ui.add(
        egui::Label::new(
            RichText::new(localization.text_with("onboarding-processing-counts", Some(&arguments)))
                .family(theme::semibold_font_family())
                .color(color),
        )
        .wrap(),
    );
}

pub(super) const fn journey_header_height(density: LayoutDensity) -> f32 {
    match density {
        LayoutDensity::Compact => 54.0,
        LayoutDensity::Comfortable => 62.0,
    }
}

pub(super) const fn footer_height(density: LayoutDensity) -> f32 {
    match density {
        LayoutDensity::Compact => 42.0,
        LayoutDensity::Comfortable => 50.0,
    }
}

pub(super) fn show_today_header(
    ui: &mut egui::Ui,
    localization: &Localization,
    collection_count: usize,
    published_count: usize,
    density: LayoutDensity,
) {
    let ink = theme::ink(ui.visuals().dark_mode);
    ui.heading(
        RichText::new(localization.text("home-title"))
            .size(match density {
                LayoutDensity::Compact => 38.0,
                LayoutDensity::Comfortable => 46.0,
            })
            .family(theme::semibold_font_family()),
    );
    ui.add_space(10.0);
    ui.painter().hline(
        ui.available_rect_before_wrap().x_range(),
        ui.cursor().top(),
        Stroke::new(3.0, ink),
    );
    ui.add_space(8.0);

    let mut collection_arguments = FluentArgs::new();
    collection_arguments.set("count", collection_count);
    let mut published_arguments = FluentArgs::new();
    published_arguments.set("count", published_count);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(
                localization.text_with("home-collection-count", Some(&collection_arguments)),
            )
            .small()
            .family(theme::semibold_font_family()),
        );
        ui.separator();
        ui.label(
            RichText::new(
                localization.text_with("home-published-count", Some(&published_arguments)),
            )
            .small()
            .family(theme::semibold_font_family()),
        );
        ui.separator();
        ui.label(
            RichText::new(localization.text("home-private-default"))
                .small()
                .color(theme::secondary_text(ui.visuals().dark_mode)),
        );
    });
    ui.add_space(8.0);
    ui.painter().hline(
        ui.available_rect_before_wrap().x_range(),
        ui.cursor().top(),
        Stroke::new(1.0, ink),
    );
}

pub(super) fn show_journey_header(
    ui: &mut egui::Ui,
    localization: &Localization,
    states: [JourneyStepState; 5],
    density: LayoutDensity,
) {
    let current = states
        .iter()
        .position(|state| {
            matches!(
                state,
                JourneyStepState::Current | JourneyStepState::Attention
            )
        })
        .map_or(states.len(), |index| index + 1);
    let mut arguments = FluentArgs::new();
    arguments.set("current", current);
    arguments.set("total", states.len());
    let ink = theme::ink(ui.visuals().dark_mode);
    ui.painter().hline(
        ui.available_rect_before_wrap().x_range(),
        ui.cursor().top(),
        Stroke::new(3.0, ink),
    );
    ui.add_space(match density {
        LayoutDensity::Compact => 8.0,
        LayoutDensity::Comfortable => 10.0,
    });
    ui.label(
        RichText::new(localization.text_with("onboarding-progress", Some(&arguments)))
            .size(12.0)
            .family(theme::semibold_font_family())
            .color(theme::accent_text(ui.visuals().dark_mode)),
    );
    ui.add_space(10.0);
    ui.painter().hline(
        ui.available_rect_before_wrap().x_range(),
        ui.cursor().top(),
        Stroke::new(1.0, theme::border(ui.visuals().dark_mode)),
    );
}

pub(super) fn work_surface<R>(
    ui: &mut egui::Ui,
    density: LayoutDensity,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.scope(|ui| {
        ui.set_min_width(ui.available_width());
        if density == LayoutDensity::Compact {
            ui.spacing_mut().item_spacing.y = 5.0;
        }
        add_contents(ui)
    })
    .inner
}

pub(super) fn primary_button(label: String) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(label)
            .family(theme::semibold_font_family())
            .color(Color32::WHITE),
    )
    .fill(AIR_BLUE)
    .stroke(Stroke::new(1.0, AIR_BLUE))
    .corner_radius(egui::CornerRadius::same(2))
}

pub(super) fn privacy_note(ui: &mut egui::Ui, localization: &Localization) {
    ui.horizontal_wrapped(|ui| {
        let center = ui.cursor().left_top() + egui::vec2(8.0, 8.0);
        ui.painter().rect_filled(
            egui::Rect::from_center_size(center, egui::vec2(9.0, 9.0)),
            1.0,
            AIR_AQUA,
        );
        ui.add_space(18.0);
        ui.label(
            RichText::new(localization.text("onboarding-privacy-local"))
                .small()
                .color(theme::secondary_text(ui.visuals().dark_mode)),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_states_keep_the_five_real_knowledge_steps() {
        let states = [
            JourneyStepState::Complete,
            JourneyStepState::Current,
            JourneyStepState::Upcoming,
            JourneyStepState::Upcoming,
            JourneyStepState::Upcoming,
        ];

        assert_eq!(states.len(), 5);
        assert_eq!(states[1], JourneyStepState::Current);
    }

    #[test]
    fn compact_journey_reserves_less_vertical_space() {
        assert!(
            journey_header_height(LayoutDensity::Compact)
                < journey_header_height(LayoutDensity::Comfortable)
        );
    }

    #[test]
    fn processing_progress_uses_persisted_document_states() {
        assert_eq!(
            processing_progress(7, 1, 2, 1, 1),
            ProcessingProgress {
                documents: 7,
                preparing: 3,
                ready_for_review: 2,
                issues: 1,
            }
        );
    }

    #[test]
    fn processing_progress_includes_transient_visible_issues() {
        assert_eq!(processing_progress(0, 0, 0, 0, 1).issues, 1);
    }

    #[test]
    fn processing_progress_saturates_inconsistent_snapshots() {
        assert_eq!(processing_progress(1, 1, 1, 1, 0).preparing, 0);
    }
}
