use eframe::egui::{self, Color32, RichText, Stroke};
use fluent_bundle::FluentArgs;

use crate::i18n::Localization;
use crate::layout::LayoutDensity;

pub(super) const AIR_BLUE: Color32 = Color32::from_rgb(34, 151, 245);
const AIR_AQUA: Color32 = Color32::from_rgb(22, 199, 215);
const AIR_INK: Color32 = Color32::from_rgb(13, 47, 95);
const AIR_SLATE: Color32 = Color32::from_rgb(105, 119, 138);
const AIR_AMBER: Color32 = Color32::from_rgb(227, 163, 60);

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
    let color = if ui.visuals().dark_mode {
        AIR_BLUE
    } else {
        AIR_INK
    };
    ui.add(
        egui::Label::new(
            RichText::new(localization.text_with("onboarding-processing-counts", Some(&arguments)))
                .strong()
                .color(color),
        )
        .wrap(),
    );
}

pub(super) const fn journey_header_height(density: LayoutDensity) -> f32 {
    match density {
        LayoutDensity::Compact => 172.0,
        LayoutDensity::Comfortable => 208.0,
    }
}

pub(super) const fn footer_height(density: LayoutDensity) -> f32 {
    match density {
        LayoutDensity::Compact => 42.0,
        LayoutDensity::Comfortable => 50.0,
    }
}

pub(super) const fn surface_margin(density: LayoutDensity) -> i8 {
    match density {
        LayoutDensity::Compact => 14,
        LayoutDensity::Comfortable => 22,
    }
}

pub(super) fn show_journey_header(
    ui: &mut egui::Ui,
    localization: &Localization,
    states: [JourneyStepState; 5],
    density: LayoutDensity,
) {
    show_header(ui, localization, density);
    ui.add_space(match density {
        LayoutDensity::Compact => 8.0,
        LayoutDensity::Comfortable => 20.0,
    });
    show_route(ui, localization, states, density);
}

fn show_header(ui: &mut egui::Ui, localization: &Localization, density: LayoutDensity) {
    let (eyebrow_size, title_size, subtitle_size, title_gap) = match density {
        LayoutDensity::Compact => (11.0, 26.0, 14.0, 2.0),
        LayoutDensity::Comfortable => (12.0, 32.0, 16.0, 4.0),
    };
    ui.label(
        RichText::new(localization.text("first-knowledge-eyebrow"))
            .size(eyebrow_size)
            .strong()
            .color(AIR_AQUA),
    );
    ui.add_space(title_gap);
    ui.heading(
        RichText::new(localization.text("first-knowledge-title"))
            .size(title_size)
            .strong(),
    );
    ui.add(
        egui::Label::new(
            RichText::new(localization.text("first-knowledge-subtitle"))
                .size(subtitle_size)
                .color(ui.visuals().weak_text_color()),
        )
        .wrap(),
    );
}

pub(super) fn show_route(
    ui: &mut egui::Ui,
    localization: &Localization,
    states: [JourneyStepState; 5],
    density: LayoutDensity,
) {
    let labels = [
        localization.text("journey-prepare"),
        localization.text("journey-read"),
        localization.text("journey-review"),
        localization.text("journey-publish"),
        localization.text("journey-ask"),
    ];
    let accessible_summary = labels
        .iter()
        .zip(states)
        .map(|(label, state)| {
            let state = localization.text(match state {
                JourneyStepState::Complete => "journey-step-done",
                JourneyStepState::Current | JourneyStepState::Attention => "journey-step-current",
                JourneyStepState::Upcoming => "journey-step-next",
            });
            format!("{label}: {state}")
        })
        .collect::<Vec<_>>()
        .join(". ");

    ui.label(
        RichText::new(localization.text("journey-title"))
            .size(13.0)
            .strong()
            .color(ui.visuals().weak_text_color()),
    );
    ui.add_space(match density {
        LayoutDensity::Compact => 4.0,
        LayoutDensity::Comfortable => 8.0,
    });

    let route_height = match density {
        LayoutDensity::Compact => 58.0,
        LayoutDensity::Comfortable => 78.0,
    };
    let desired_size = egui::vec2(ui.available_width().max(1.0), route_height);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, true, accessible_summary.clone())
    });

    let painter = ui.painter_at(rect);
    let side_padding = 34.0;
    let y = rect.top()
        + match density {
            LayoutDensity::Compact => 18.0,
            LayoutDensity::Comfortable => 22.0,
        };
    let usable_width = (rect.width() - side_padding * 2.0).max(0.0);
    let spacing = usable_width / 4.0;
    let points: [egui::Pos2; 5] = std::array::from_fn(|index| {
        egui::pos2(rect.left() + side_padding + spacing * index as f32, y)
    });

    for index in 0..4 {
        let completed = matches!(
            (states[index], states[index + 1]),
            (JourneyStepState::Complete, JourneyStepState::Complete)
                | (JourneyStepState::Complete, JourneyStepState::Current)
                | (JourneyStepState::Complete, JourneyStepState::Attention)
        );
        painter.line_segment(
            [points[index], points[index + 1]],
            Stroke::new(3.0, if completed { AIR_BLUE } else { route_muted(ui) }),
        );
    }

    for ((point, state), label) in points.into_iter().zip(states).zip(labels.iter()) {
        paint_node(&painter, point, state);
        let label_color = if matches!(state, JourneyStepState::Upcoming) {
            ui.visuals().weak_text_color()
        } else {
            ui.visuals().text_color()
        };
        painter.text(
            egui::pos2(
                point.x,
                rect.top()
                    + match density {
                        LayoutDensity::Compact => 42.0,
                        LayoutDensity::Comfortable => 50.0,
                    },
            ),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(match density {
                LayoutDensity::Compact => 12.0,
                LayoutDensity::Comfortable => 13.0,
            }),
            label_color,
        );
    }
}

pub(super) fn work_surface<R>(
    ui: &mut egui::Ui,
    density: LayoutDensity,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let fill = if ui.visuals().dark_mode {
        Color32::from_rgb(18, 28, 42)
    } else {
        Color32::from_rgb(247, 250, 252)
    };
    let border = if ui.visuals().dark_mode {
        Color32::from_rgb(48, 70, 94)
    } else {
        Color32::from_rgb(216, 230, 242)
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::same(surface_margin(density)))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if density == LayoutDensity::Compact {
                ui.spacing_mut().item_spacing.y = 5.0;
            }
            add_contents(ui)
        })
        .inner
}

pub(super) fn primary_button(label: String) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
        .fill(AIR_BLUE)
        .stroke(Stroke::new(1.0, AIR_AQUA))
        .corner_radius(egui::CornerRadius::same(8))
}

pub(super) fn privacy_note(ui: &mut egui::Ui, localization: &Localization) {
    ui.horizontal_wrapped(|ui| {
        let center = ui.cursor().left_top() + egui::vec2(8.0, 8.0);
        ui.painter().circle_filled(center, 5.0, AIR_AQUA);
        ui.add_space(18.0);
        ui.label(
            RichText::new(localization.text("onboarding-privacy-local"))
                .small()
                .color(ui.visuals().weak_text_color()),
        );
    });
}

fn route_muted(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(58, 73, 91)
    } else {
        Color32::from_rgb(205, 218, 230)
    }
}

fn paint_node(painter: &egui::Painter, center: egui::Pos2, state: JourneyStepState) {
    match state {
        JourneyStepState::Complete => {
            painter.circle_filled(center, 11.0, AIR_INK);
            painter.line_segment(
                [
                    center + egui::vec2(-4.0, 0.0),
                    center + egui::vec2(-1.0, 3.0),
                ],
                Stroke::new(2.0, Color32::WHITE),
            );
            painter.line_segment(
                [
                    center + egui::vec2(-1.0, 3.0),
                    center + egui::vec2(5.0, -4.0),
                ],
                Stroke::new(2.0, Color32::WHITE),
            );
        }
        JourneyStepState::Current => {
            painter.circle_filled(center, 12.0, AIR_BLUE);
            painter.circle_stroke(center, 16.0, Stroke::new(2.0, AIR_AQUA));
            painter.circle_filled(center, 4.0, Color32::WHITE);
        }
        JourneyStepState::Upcoming => {
            painter.circle_filled(center, 10.0, AIR_SLATE);
            painter.circle_filled(center, 5.0, Color32::WHITE);
        }
        JourneyStepState::Attention => {
            painter.circle_filled(center, 12.0, AIR_AMBER);
            painter.rect_filled(
                egui::Rect::from_center_size(center + egui::vec2(0.0, -2.0), egui::vec2(2.5, 7.0)),
                1.0,
                Color32::WHITE,
            );
            painter.circle_filled(center + egui::vec2(0.0, 4.5), 1.5, Color32::WHITE);
        }
    }
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
