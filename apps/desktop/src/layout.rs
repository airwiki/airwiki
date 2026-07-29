use eframe::egui;

pub(crate) const INITIAL_WINDOW_SIZE: [f32; 2] = [1180.0, 760.0];
pub(crate) const MINIMUM_WINDOW_SIZE: [f32; 2] = [860.0, 600.0];

const COMPACT_HEIGHT_THRESHOLD: f32 = 700.0;
const NARROW_WIDTH_THRESHOLD: f32 = 760.0;
const WIDE_CONTENT_MARGIN: i8 = 52;
const NARROW_CONTENT_MARGIN: i8 = 28;
const WIDE_CONTENT_TOP_MARGIN: i8 = 40;
const COMPACT_CONTENT_TOP_MARGIN: i8 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayoutDensity {
    Compact,
    Comfortable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WidthClass {
    Narrow,
    Wide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResponsiveLayout {
    pub(crate) density: LayoutDensity,
    pub(crate) width: WidthClass,
}

impl ResponsiveLayout {
    pub(crate) fn from_available(size: egui::Vec2) -> Self {
        Self {
            density: if size.y < COMPACT_HEIGHT_THRESHOLD {
                LayoutDensity::Compact
            } else {
                LayoutDensity::Comfortable
            },
            width: if size.x < NARROW_WIDTH_THRESHOLD {
                WidthClass::Narrow
            } else {
                WidthClass::Wide
            },
        }
    }

    pub(crate) fn is_compact(self) -> bool {
        self.density == LayoutDensity::Compact
    }

    pub(crate) fn is_narrow(self) -> bool {
        self.width == WidthClass::Narrow
    }

    pub(crate) fn content_margin(self) -> egui::Margin {
        egui::Margin {
            left: if self.is_narrow() {
                NARROW_CONTENT_MARGIN
            } else {
                WIDE_CONTENT_MARGIN
            },
            right: if self.is_narrow() {
                NARROW_CONTENT_MARGIN
            } else {
                WIDE_CONTENT_MARGIN
            },
            top: if self.is_compact() {
                COMPACT_CONTENT_TOP_MARGIN
            } else {
                WIDE_CONTENT_TOP_MARGIN
            },
            bottom: if self.is_compact() {
                COMPACT_CONTENT_TOP_MARGIN
            } else {
                WIDE_CONTENT_TOP_MARGIN
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_window_uses_comfortable_wide_layout() {
        let layout = ResponsiveLayout::from_available(egui::vec2(1180.0, 760.0));

        assert_eq!(
            layout,
            ResponsiveLayout {
                density: LayoutDensity::Comfortable,
                width: WidthClass::Wide,
            }
        );
    }

    #[test]
    fn minimum_content_area_uses_compact_narrow_layout() {
        let layout = ResponsiveLayout::from_available(egui::vec2(635.0, 540.0));

        assert_eq!(
            layout,
            ResponsiveLayout {
                density: LayoutDensity::Compact,
                width: WidthClass::Narrow,
            }
        );
    }

    #[test]
    fn layout_thresholds_are_stable_at_the_boundary() {
        let layout = ResponsiveLayout::from_available(egui::vec2(760.0, 700.0));

        assert_eq!(
            layout,
            ResponsiveLayout {
                density: LayoutDensity::Comfortable,
                width: WidthClass::Wide,
            }
        );
    }

    #[test]
    fn content_margins_match_the_broadsheet_shell_and_compact_window() {
        let wide = ResponsiveLayout::from_available(egui::vec2(970.0, 690.0));
        let compact = ResponsiveLayout::from_available(egui::vec2(650.0, 540.0));

        assert_eq!(wide.content_margin(), egui::Margin::symmetric(52, 24));
        assert_eq!(
            compact.content_margin(),
            egui::Margin {
                left: 28,
                right: 28,
                top: 24,
                bottom: 24,
            }
        );
    }
}
