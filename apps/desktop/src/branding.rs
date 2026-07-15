use eframe::egui;

const WINDOW_ICON_SIDE: u32 = 128;
const WINDOW_ICON_RGBA: &[u8; 65_536] =
    include_bytes!("../../../resources/branding/airwiki-window.rgba");

pub(crate) const TRAY_ICON_SIDE: u32 = 24;
const TRAY_ICON_RGBA: &[u8; 2_304] =
    include_bytes!("../../../resources/branding/airwiki-tray.rgba");

pub(crate) fn window_icon() -> egui::IconData {
    egui::IconData {
        rgba: WINDOW_ICON_RGBA.to_vec(),
        width: WINDOW_ICON_SIDE,
        height: WINDOW_ICON_SIDE,
    }
}

pub(crate) fn tray_icon_rgba() -> Vec<u8> {
    TRAY_ICON_RGBA.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icons_have_the_expected_dimensions() {
        let window = window_icon();

        assert_eq!(window.width, WINDOW_ICON_SIDE);
        assert_eq!(window.height, WINDOW_ICON_SIDE);
        assert_eq!(window.rgba.len(), 128 * 128 * 4);
        assert_eq!(tray_icon_rgba().len(), 24 * 24 * 4);
    }
}
