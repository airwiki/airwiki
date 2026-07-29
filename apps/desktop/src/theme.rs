use std::sync::Arc;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily};

/// Broadsheet's semantic cyan for active text and links.
pub(crate) const AIR_BLUE: Color32 = Color32::from_rgb(0, 103, 134);
/// Broadsheet's brighter cyan for focus and decorative accents.
pub(crate) const AIR_CYAN: Color32 = Color32::from_rgb(0, 136, 176);
pub(crate) const EVIDENCE_CYAN: Color32 = Color32::from_rgb(0, 169, 209);
const ATTENTION_MAGENTA_LIGHT: Color32 = Color32::from_rgb(170, 11, 86);
const ATTENTION_MAGENTA_STRONG_LIGHT: Color32 = Color32::from_rgb(121, 14, 61);
const ATTENTION_MAGENTA_DARK: Color32 = Color32::from_rgb(255, 117, 181);
const ACCENT_TAG_TEXT_LIGHT: Color32 = Color32::from_rgb(0, 73, 97);
const NEUTRAL_TAG_TEXT_LIGHT: Color32 = Color32::from_rgb(68, 65, 65);
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
const BORDER_LIGHT: Color32 = Color32::from_rgb(210, 208, 207);
const HOVER_LIGHT: Color32 = Color32::from_rgb(232, 231, 231);
const PRESSED_LIGHT: Color32 = Color32::from_rgb(213, 211, 211);
const TEXT_LIGHT: Color32 = Color32::from_rgb(32, 30, 29);
const SECONDARY_LIGHT: Color32 = Color32::from_rgb(96, 93, 93);
const ACCENT_TINT_LIGHT: Color32 = Color32::from_rgb(233, 248, 255);
const ATTENTION_TINT_LIGHT: Color32 = Color32::from_rgb(255, 241, 244);
const NEUTRAL_TINT_LIGHT: Color32 = Color32::from_rgb(248, 244, 244);
const ACCENT_TINT_DARK: Color32 = Color32::from_rgb(10, 48, 62);
const ATTENTION_TINT_DARK: Color32 = Color32::from_rgb(75, 21, 40);
const NEUTRAL_TINT_DARK: Color32 = Color32::from_rgb(68, 65, 65);
const EDITORIAL_REGULAR_FONT: &str = "Source Serif 4 Regular";
const EDITORIAL_SEMIBOLD_FONT: &str = "Source Serif 4 Semibold";

pub(crate) fn apply(context: &egui::Context) {
    install_editorial_font(context);

    let mut style = (*context.global_style()).clone();
    // The authoritative AirWiki design is a paper-like light interface. Keep
    // the presentation stable across operating-system appearance settings.
    style.visuals = egui::Visuals::light();
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(18.0, 10.0);
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
    // The brighter reference cyan does not meet 4.5:1 with white. Use the
    // reference link cyan for selected text while keeping the brighter cyan
    // for focus and non-text accents.
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
        style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        style.visuals.widgets.inactive.bg_stroke.color = BORDER_DARK;
        style.visuals.widgets.hovered.weak_bg_fill = NEUTRAL_TINT_DARK;
        style.visuals.widgets.active.weak_bg_fill = BORDER_DARK;
        style.visuals.faint_bg_color = SURFACE_DARK;
        style.visuals.extreme_bg_color = SURFACE_DARK;
        style.visuals.override_text_color = Some(TEXT_DARK);
        style.visuals.weak_text_color = Some(SECONDARY_DARK);
    } else {
        style.visuals.panel_fill = PAPER_LIGHT;
        style.visuals.window_fill = SURFACE_LIGHT;
        style.visuals.window_stroke.color = BORDER_LIGHT;
        style.visuals.widgets.noninteractive.bg_fill = SURFACE_LIGHT;
        style.visuals.widgets.noninteractive.bg_stroke.color = BORDER_LIGHT;
        style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        style.visuals.widgets.inactive.bg_stroke.color = BORDER_LIGHT;
        style.visuals.widgets.hovered.weak_bg_fill = HOVER_LIGHT;
        style.visuals.widgets.active.weak_bg_fill = PRESSED_LIGHT;
        style.visuals.faint_bg_color = SURFACE_LIGHT;
        style.visuals.extreme_bg_color = SURFACE_LIGHT;
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

pub(crate) fn attention_strong(dark_mode: bool) -> Color32 {
    if dark_mode {
        ATTENTION_MAGENTA_DARK
    } else {
        ATTENTION_MAGENTA_STRONG_LIGHT
    }
}

pub(crate) fn accent_tag_text(dark_mode: bool) -> Color32 {
    if dark_mode {
        EVIDENCE_CYAN
    } else {
        ACCENT_TAG_TEXT_LIGHT
    }
}

pub(crate) fn neutral_tag_text(dark_mode: bool) -> Color32 {
    if dark_mode {
        SECONDARY_DARK
    } else {
        NEUTRAL_TAG_TEXT_LIGHT
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

pub(crate) fn accent_tint(dark_mode: bool) -> Color32 {
    if dark_mode {
        ACCENT_TINT_DARK
    } else {
        ACCENT_TINT_LIGHT
    }
}

pub(crate) fn attention_tint(dark_mode: bool) -> Color32 {
    if dark_mode {
        ATTENTION_TINT_DARK
    } else {
        ATTENTION_TINT_LIGHT
    }
}

pub(crate) fn neutral_tint(dark_mode: bool) -> Color32 {
    if dark_mode {
        NEUTRAL_TINT_DARK
    } else {
        NEUTRAL_TINT_LIGHT
    }
}

pub(crate) fn truncated_title_job(
    text: &str,
    font_id: egui::FontId,
    color: Color32,
    max_width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font_id, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));
    job
}

fn tracked_job(
    text: impl Into<String>,
    font_id: egui::FontId,
    tracking: f32,
    color: Color32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(text.into(), font_id, color);
    if let Some(section) = job.sections.first_mut() {
        section.format.extra_letter_spacing = tracking;
    }
    job
}

pub(crate) fn card_kicker_job(text: impl Into<String>, color: Color32) -> egui::text::LayoutJob {
    tracked_job(text, egui::FontId::proportional(10.0), 1.0, color)
}

pub(crate) fn section_label_job(text: impl Into<String>, color: Color32) -> egui::text::LayoutJob {
    tracked_job(
        text,
        egui::FontId::new(13.0, semibold_font_family()),
        1.04,
        color,
    )
}

pub(crate) struct FocusButton {
    button: egui::Button<'static>,
    focus_color: Color32,
}

impl FocusButton {
    pub(crate) fn small(mut self) -> Self {
        self.button = self.button.small();
        self
    }
}

impl egui::Widget for FocusButton {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let response = ui.add(self.button);
        if response.has_focus() {
            ui.painter().rect_stroke(
                response.rect.expand(2.0),
                egui::CornerRadius::same(3),
                egui::Stroke::new(2.0, self.focus_color),
                egui::StrokeKind::Inside,
            );
        }
        response
    }
}

pub(crate) fn focus_button(button: egui::Button<'static>, focus_color: Color32) -> FocusButton {
    FocusButton {
        button,
        focus_color,
    }
}

pub(crate) fn ghost_button(label: impl Into<String>, dark_mode: bool) -> FocusButton {
    let focus_color = accent_text(dark_mode);
    focus_button(
        egui::Button::new(
            egui::RichText::new(label.into())
                .family(semibold_font_family())
                .color(focus_color),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::NONE)
        .frame(false),
        focus_color,
    )
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
        assert_eq!(style.visuals.extreme_bg_color, SURFACE_LIGHT);
        assert_eq!(
            style.visuals.widgets.inactive.weak_bg_fill,
            Color32::TRANSPARENT
        );
        assert_eq!(style.visuals.widgets.hovered.weak_bg_fill, HOVER_LIGHT);
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
    fn controls_use_the_authoritative_broadsheet_density() {
        let context = egui::Context::default();

        apply(&context);
        let style = context.global_style();

        assert_eq!(style.spacing.button_padding, egui::vec2(18.0, 10.0));
        assert_eq!(style.spacing.interact_size.y, 36.0);
    }

    #[test]
    fn long_editorial_titles_are_configured_for_single_line_elision() {
        let long_title =
            "A deliberately long editorial title that cannot fit beside trailing metadata";
        let job = truncated_title_job(
            long_title,
            egui::FontId::proportional(15.0),
            Color32::BLACK,
            96.0,
        );

        assert_eq!(job.text, long_title);
        assert_eq!(job.wrap.max_width, 96.0);
        assert_eq!(job.wrap.max_rows, 1);
        assert!(job.wrap.break_anywhere);
        assert_eq!(job.wrap.overflow_character, Some('…'));
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
            (ATTENTION_MAGENTA_STRONG_LIGHT, ATTENTION_TINT_LIGHT),
            (ACCENT_TAG_TEXT_LIGHT, ACCENT_TINT_LIGHT),
            (NEUTRAL_TAG_TEXT_LIGHT, NEUTRAL_TINT_LIGHT),
        ] {
            assert!(contrast_ratio(foreground, background) >= 4.5);
        }
    }

    #[test]
    fn attention_tag_uses_the_authoritative_broadsheet_tokens() {
        assert_eq!(
            (attention_tint(false), attention_strong(false)),
            (
                Color32::from_rgb(255, 241, 244),
                Color32::from_rgb(121, 14, 61),
            )
        );
    }

    #[test]
    fn semantic_tags_use_the_authoritative_broadsheet_text_tokens() {
        assert_eq!(
            (
                accent_tag_text(false),
                attention_strong(false),
                neutral_tag_text(false),
            ),
            (
                Color32::from_rgb(0, 73, 97),
                Color32::from_rgb(121, 14, 61),
                Color32::from_rgb(68, 65, 65),
            )
        );
    }

    #[test]
    fn card_kicker_uses_regular_ten_pixel_type_with_tenth_em_tracking() {
        let job = card_kicker_job("PUBLIC NETWORK", AIR_BLUE);

        assert_eq!(job.text, "PUBLIC NETWORK");
        assert_eq!(job.sections.len(), 1);
        assert_eq!(job.sections[0].format.font_id.size, 10.0);
        assert_eq!(
            job.sections[0].format.font_id.family,
            FontFamily::Proportional
        );
        assert_eq!(job.sections[0].format.extra_letter_spacing, 1.0);
    }

    #[test]
    fn section_label_uses_semibold_thirteen_pixel_type_with_eight_percent_tracking() {
        let job = section_label_job("SOURCES", AIR_BLUE);

        assert_eq!(job.text, "SOURCES");
        assert_eq!(job.sections.len(), 1);
        assert_eq!(job.sections[0].format.font_id.size, 13.0);
        assert_eq!(
            job.sections[0].format.font_id.family,
            semibold_font_family()
        );
        assert_eq!(job.sections[0].format.extra_letter_spacing, 1.04);
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
