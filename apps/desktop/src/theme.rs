use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily};

/// Broadsheet's restrained cyan plate, darkened so white button text remains legible.
pub(crate) const AIR_BLUE: Color32 = Color32::from_rgb(0, 103, 134);
pub(crate) const EVIDENCE_CYAN: Color32 = Color32::from_rgb(0, 169, 209);
const ATTENTION_MAGENTA_LIGHT: Color32 = Color32::from_rgb(170, 11, 86);
const ATTENTION_MAGENTA_DARK: Color32 = Color32::from_rgb(255, 117, 181);
pub(crate) const VERIFIED_GREEN: Color32 = Color32::from_rgb(87, 200, 137);
pub(crate) const WARNING_AMBER: Color32 = Color32::from_rgb(230, 162, 60);
pub(crate) const ERROR_CORAL: Color32 = Color32::from_rgb(255, 123, 117);
const VERIFIED_GREEN_LIGHT: Color32 = Color32::from_rgb(39, 100, 72);
const WARNING_AMBER_LIGHT: Color32 = Color32::from_rgb(122, 78, 0);
const ERROR_CORAL_LIGHT: Color32 = Color32::from_rgb(168, 50, 50);

const PAPER_DARK: Color32 = Color32::from_rgb(32, 30, 29);
const SURFACE_DARK: Color32 = Color32::from_rgb(45, 43, 43);
const BORDER_DARK: Color32 = Color32::from_rgb(96, 93, 93);
const TEXT_DARK: Color32 = Color32::from_rgb(248, 244, 244);
const SECONDARY_DARK: Color32 = Color32::from_rgb(186, 182, 182);

const PAPER_LIGHT: Color32 = Color32::from_rgb(243, 242, 242);
const SURFACE_LIGHT: Color32 = Color32::from_rgb(234, 233, 233);
const BORDER_LIGHT: Color32 = Color32::from_rgb(196, 193, 193);
const TEXT_LIGHT: Color32 = Color32::from_rgb(32, 30, 29);
const SECONDARY_LIGHT: Color32 = Color32::from_rgb(96, 93, 93);
const EDITORIAL_REGULAR_FONT: &str = "Source Serif 4 Regular";
const EDITORIAL_SEMIBOLD_FONT: &str = "Source Serif 4 Semibold";

pub(crate) fn apply(context: &egui::Context) {
    install_editorial_font(context);

    let mut style = (*context.global_style()).clone();
    // The authoritative AirWiki design is a paper-like light interface. Keep
    // the presentation stable across operating-system appearance settings.
    style.visuals = egui::Visuals::light();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 36.0;
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(32.0, semibold_font_family()),
    );
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(14.0, semibold_font_family()),
    );
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.5));
    style.visuals.selection.bg_fill = AIR_BLUE;
    style.visuals.selection.stroke = egui::Stroke::new(2.0, Color32::WHITE);
    style.visuals.hyperlink_color = hyperlink(style.visuals.dark_mode);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(2);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(2);
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(2);
    style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(2);
    style.visuals.window_corner_radius = egui::CornerRadius::same(2);
    if style.visuals.dark_mode {
        style.visuals.panel_fill = PAPER_DARK;
        style.visuals.window_fill = SURFACE_DARK;
        style.visuals.window_stroke.color = BORDER_DARK;
        style.visuals.widgets.noninteractive.bg_fill = SURFACE_DARK;
        style.visuals.widgets.noninteractive.bg_stroke.color = BORDER_DARK;
        style.visuals.widgets.inactive.weak_bg_fill = SURFACE_DARK;
        style.visuals.widgets.inactive.bg_stroke.color = BORDER_DARK;
        style.visuals.faint_bg_color = SURFACE_DARK;
        style.visuals.extreme_bg_color = PAPER_DARK;
        style.visuals.override_text_color = Some(TEXT_DARK);
        style.visuals.weak_text_color = Some(SECONDARY_DARK);
    } else {
        style.visuals.panel_fill = PAPER_LIGHT;
        style.visuals.window_fill = SURFACE_LIGHT;
        style.visuals.window_stroke.color = BORDER_LIGHT;
        style.visuals.widgets.noninteractive.bg_fill = SURFACE_LIGHT;
        style.visuals.widgets.noninteractive.bg_stroke.color = BORDER_LIGHT;
        style.visuals.widgets.inactive.weak_bg_fill = SURFACE_LIGHT;
        style.visuals.widgets.inactive.bg_stroke.color = BORDER_LIGHT;
        style.visuals.faint_bg_color = SURFACE_LIGHT;
        style.visuals.extreme_bg_color = PAPER_LIGHT;
        style.visuals.override_text_color = Some(TEXT_LIGHT);
        style.visuals.weak_text_color = Some(SECONDARY_LIGHT);
    }
    context.set_global_style(style);
}

fn install_editorial_font(context: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        EDITORIAL_REGULAR_FONT.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/SourceSerif4-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        EDITORIAL_SEMIBOLD_FONT.to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/SourceSerif4-Semibold.ttf"
        ))),
    );
    if let Some(proportional) = fonts.families.get_mut(&FontFamily::Proportional) {
        proportional.insert(0, EDITORIAL_REGULAR_FONT.to_owned());
    }
    fonts.families.insert(
        semibold_font_family(),
        vec![EDITORIAL_SEMIBOLD_FONT.to_owned()],
    );
    context.set_fonts(fonts);
}

pub(crate) fn semibold_font_family() -> FontFamily {
    FontFamily::Name(EDITORIAL_SEMIBOLD_FONT.into())
}

pub(crate) fn paper(dark_mode: bool) -> Color32 {
    if dark_mode { PAPER_DARK } else { PAPER_LIGHT }
}

pub(crate) fn surface(dark_mode: bool) -> Color32 {
    if dark_mode {
        SURFACE_DARK
    } else {
        SURFACE_LIGHT
    }
}

pub(crate) fn ink(dark_mode: bool) -> Color32 {
    if dark_mode { TEXT_DARK } else { TEXT_LIGHT }
}

pub(crate) fn accent_text(dark_mode: bool) -> Color32 {
    if dark_mode { EVIDENCE_CYAN } else { AIR_BLUE }
}

pub(crate) fn hyperlink(dark_mode: bool) -> Color32 {
    accent_text(dark_mode)
}

pub(crate) fn verified_text(dark_mode: bool) -> Color32 {
    if dark_mode {
        VERIFIED_GREEN
    } else {
        VERIFIED_GREEN_LIGHT
    }
}

pub(crate) fn warning_text(dark_mode: bool) -> Color32 {
    if dark_mode {
        WARNING_AMBER
    } else {
        WARNING_AMBER_LIGHT
    }
}

pub(crate) fn error_text(dark_mode: bool) -> Color32 {
    if dark_mode {
        ERROR_CORAL
    } else {
        ERROR_CORAL_LIGHT
    }
}

pub(crate) fn attention(dark_mode: bool) -> Color32 {
    if dark_mode {
        ATTENTION_MAGENTA_DARK
    } else {
        ATTENTION_MAGENTA_LIGHT
    }
}

pub(crate) fn border(dark_mode: bool) -> Color32 {
    if dark_mode { BORDER_DARK } else { BORDER_LIGHT }
}

pub(crate) fn secondary_text(dark_mode: bool) -> Color32 {
    if dark_mode {
        SECONDARY_DARK
    } else {
        SECONDARY_LIGHT
    }
}

pub(crate) fn surface_frame(dark_mode: bool) -> egui::Frame {
    egui::Frame::new()
        .fill(surface(dark_mode))
        .stroke(egui::Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(2))
        .inner_margin(egui::Margin::same(15))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoritative_presentation_uses_light_paper_tokens() {
        let context = egui::Context::default();

        apply(&context);
        let style = context.global_style();

        assert!(!style.visuals.dark_mode);
        assert_eq!(style.visuals.panel_fill, PAPER_LIGHT);
        assert_eq!(style.visuals.override_text_color, Some(TEXT_LIGHT));
    }

    #[test]
    fn headings_and_buttons_use_the_explicit_semibold_family() {
        let context = egui::Context::default();

        apply(&context);
        let style = context.global_style();

        assert_eq!(
            style.text_styles[&egui::TextStyle::Heading].family,
            semibold_font_family()
        );
        assert_eq!(
            style.text_styles[&egui::TextStyle::Button].family,
            semibold_font_family()
        );
        assert!(!include_bytes!("../assets/SourceSerif4-Semibold.ttf").is_empty());
    }

    #[test]
    fn text_tokens_meet_normal_text_contrast_in_both_themes() {
        for (foreground, background) in [
            (TEXT_DARK, PAPER_DARK),
            (SECONDARY_DARK, PAPER_DARK),
            (TEXT_LIGHT, PAPER_LIGHT),
            (SECONDARY_LIGHT, PAPER_LIGHT),
            (Color32::WHITE, AIR_BLUE),
            (ATTENTION_MAGENTA_DARK, PAPER_DARK),
            (ATTENTION_MAGENTA_LIGHT, PAPER_LIGHT),
        ] {
            assert!(contrast_ratio(foreground, background) >= 4.5);
        }
    }

    #[test]
    fn hyperlinks_meet_normal_text_contrast_on_real_backgrounds() {
        for dark_mode in [false, true] {
            for background in [paper(dark_mode), surface(dark_mode)] {
                assert!(contrast_ratio(hyperlink(dark_mode), background) >= 4.5);
            }
        }
    }

    #[test]
    fn accent_text_meets_normal_text_contrast_on_real_backgrounds() {
        for dark_mode in [false, true] {
            for background in [paper(dark_mode), surface(dark_mode)] {
                assert!(contrast_ratio(accent_text(dark_mode), background) >= 4.5);
            }
        }
    }

    #[test]
    fn status_text_meets_normal_text_contrast_on_real_backgrounds() {
        for dark_mode in [false, true] {
            for background in [paper(dark_mode), surface(dark_mode)] {
                for foreground in [
                    verified_text(dark_mode),
                    warning_text(dark_mode),
                    error_text(dark_mode),
                ] {
                    assert!(contrast_ratio(foreground, background) >= 4.5);
                }
            }
        }
    }

    fn contrast_ratio(left: Color32, right: Color32) -> f32 {
        let (lighter, darker) = {
            let left = relative_luminance(left);
            let right = relative_luminance(right);
            if left >= right {
                (left, right)
            } else {
                (right, left)
            }
        };
        (lighter + 0.05) / (darker + 0.05)
    }

    fn relative_luminance(color: Color32) -> f32 {
        let [red, green, blue, _] = color.to_array();
        0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
    }

    fn linear(channel: u8) -> f32 {
        let value = f32::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
}
