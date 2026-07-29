use eframe::egui::{self, Color32, RichText, Stroke};
use fluent_bundle::FluentArgs;

use crate::i18n::Localization;
use crate::{layout::LayoutDensity, theme};

pub(super) const fn journey_header_height(density: LayoutDensity) -> f32 {
    match density {
        LayoutDensity::Compact => 30.0,
        LayoutDensity::Comfortable => 32.0,
    }
}

pub(super) const fn footer_height(density: LayoutDensity) -> f32 {
    match density {
        LayoutDensity::Compact => 72.0,
        LayoutDensity::Comfortable => 88.0,
    }
}

pub(super) fn show_today_header(
    ui: &mut egui::Ui,
    localization: &Localization,
    date: &str,
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
        ui.spacing_mut().item_spacing.x = 24.0;
        ui.label(
            RichText::new(date)
                .small()
                .family(theme::semibold_font_family()),
        );
        ui.label(
            RichText::new(
                localization.text_with("home-collection-count", Some(&collection_arguments)),
            )
            .small()
            .color(theme::secondary_text(ui.visuals().dark_mode)),
        );
        ui.label(
            RichText::new(
                localization.text_with("home-published-count", Some(&published_arguments)),
            )
            .small()
            .color(theme::secondary_text(ui.visuals().dark_mode)),
        );
        if density == LayoutDensity::Comfortable {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(localization.text("home-private-default"))
                        .small()
                        .color(theme::secondary_text(ui.visuals().dark_mode)),
                );
            });
        }
    });
    ui.add_space(8.0);
    ui.painter().hline(
        ui.available_rect_before_wrap().x_range(),
        ui.cursor().top(),
        Stroke::new(1.0, ink),
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

pub(super) fn primary_button(label: String) -> theme::FocusButton {
    theme::focus_button(
        egui::Button::new(
            RichText::new(label)
                .family(theme::semibold_font_family())
                .color(Color32::WHITE),
        )
        .fill(theme::AIR_BLUE)
        .stroke(Stroke::new(1.0, theme::AIR_BLUE))
        .corner_radius(egui::CornerRadius::same(2)),
        theme::AIR_CYAN,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_journey_reserves_less_vertical_space() {
        assert!(
            journey_header_height(LayoutDensity::Compact)
                < journey_header_height(LayoutDensity::Comfortable)
        );
    }
}
