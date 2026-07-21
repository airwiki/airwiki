use eframe::egui::{self, Color32};

pub(crate) const AIR_BLUE: Color32 = Color32::from_rgb(34, 151, 245);
pub(crate) const EVIDENCE_CYAN: Color32 = Color32::from_rgb(22, 199, 215);
pub(crate) const VERIFIED_GREEN: Color32 = Color32::from_rgb(87, 200, 137);
pub(crate) const WARNING_AMBER: Color32 = Color32::from_rgb(230, 162, 60);
pub(crate) const ERROR_CORAL: Color32 = Color32::from_rgb(255, 123, 117);

const INK_DARK: Color32 = Color32::from_rgb(18, 28, 42);
const SURFACE_DARK: Color32 = Color32::from_rgb(24, 37, 54);
const BORDER_DARK: Color32 = Color32::from_rgb(58, 80, 105);
const TEXT_DARK: Color32 = Color32::from_rgb(238, 244, 250);
const SECONDARY_DARK: Color32 = Color32::from_rgb(184, 199, 217);

const INK_LIGHT: Color32 = Color32::from_rgb(247, 250, 252);
const SURFACE_LIGHT: Color32 = Color32::WHITE;
const BORDER_LIGHT: Color32 = Color32::from_rgb(204, 218, 232);
const TEXT_LIGHT: Color32 = Color32::from_rgb(24, 42, 61);
const SECONDARY_LIGHT: Color32 = Color32::from_rgb(73, 97, 120);

pub(crate) fn apply(context: &egui::Context) {
    let mut style = (*context.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 36.0;
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(28.0));
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(12.5));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.0));
    style.visuals.selection.bg_fill = AIR_BLUE;
    style.visuals.selection.stroke = egui::Stroke::new(1.0, Color32::WHITE);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
    if style.visuals.dark_mode {
        style.visuals.panel_fill = INK_DARK;
        style.visuals.window_fill = SURFACE_DARK;
        style.visuals.window_stroke.color = BORDER_DARK;
        style.visuals.override_text_color = Some(TEXT_DARK);
        style.visuals.weak_text_color = Some(SECONDARY_DARK);
    } else {
        style.visuals.panel_fill = INK_LIGHT;
        style.visuals.window_fill = SURFACE_LIGHT;
        style.visuals.window_stroke.color = BORDER_LIGHT;
        style.visuals.override_text_color = Some(TEXT_LIGHT);
        style.visuals.weak_text_color = Some(SECONDARY_LIGHT);
    }
    context.set_global_style(style);
}

pub(crate) fn surface(dark_mode: bool) -> Color32 {
    if dark_mode {
        SURFACE_DARK
    } else {
        SURFACE_LIGHT
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_tokens_meet_normal_text_contrast_in_both_themes() {
        for (foreground, background) in [
            (TEXT_DARK, INK_DARK),
            (SECONDARY_DARK, INK_DARK),
            (TEXT_LIGHT, INK_LIGHT),
            (SECONDARY_LIGHT, INK_LIGHT),
            (ERROR_CORAL, INK_DARK),
            (VERIFIED_GREEN, INK_DARK),
            (WARNING_AMBER, INK_DARK),
        ] {
            assert!(contrast_ratio(foreground, background) >= 4.5);
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
