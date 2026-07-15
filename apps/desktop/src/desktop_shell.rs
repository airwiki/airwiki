//! Native tray integration and a pure reducer for window lifecycle decisions.

use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowLifecycle {
    Visible,
    HiddenToTray,
    ExitRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrayAction {
    Open,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClosePolicy {
    Ask,
    HideToTray,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleEffect {
    None,
    Hide,
    Show,
    Close,
    Ask,
}

pub(crate) struct DesktopShell {
    lifecycle: WindowLifecycle,
    tray: TrayState,
    close_confirmation_requested: bool,
}

enum TrayState {
    Pending,
    Ready(DesktopTray),
    Unavailable,
}

impl DesktopShell {
    pub(crate) fn new(hidden: bool) -> Self {
        Self {
            lifecycle: if hidden {
                WindowLifecycle::HiddenToTray
            } else {
                WindowLifecycle::Visible
            },
            tray: TrayState::Pending,
            close_confirmation_requested: false,
        }
    }

    pub(crate) fn ensure_tray(&mut self) -> Option<String> {
        if !matches!(self.tray, TrayState::Pending) {
            return None;
        }
        match DesktopTray::try_new() {
            Ok(tray) => {
                self.tray = TrayState::Ready(tray);
                None
            }
            Err(error) => {
                self.tray = TrayState::Unavailable;
                Some(error.to_string())
            }
        }
    }

    pub(crate) fn tray_ready(&self) -> bool {
        matches!(self.tray, TrayState::Ready(_))
    }

    pub(crate) fn set_status(&self, status: &str) {
        if let TrayState::Ready(tray) = &self.tray {
            tray.set_status(status);
        }
    }

    pub(crate) fn set_labels(&self, open: &str, quit: &str) {
        if let TrayState::Ready(tray) = &self.tray {
            tray.set_labels(open, quit);
        }
    }

    pub(crate) fn handle_frame(&mut self, context: &egui::Context, close_policy: ClosePolicy) {
        let actions = match &self.tray {
            TrayState::Ready(tray) => tray.drain_actions(),
            TrayState::Pending | TrayState::Unavailable => Vec::new(),
        };
        for action in actions {
            self.apply_effect(context, reduce_tray_action(self.lifecycle, action));
        }

        if context.input(|input| input.viewport().close_requested()) {
            let effect = reduce_close_request(self.lifecycle, close_policy, self.tray_ready());
            self.apply_effect(context, effect);
        }
    }

    pub(crate) fn show(&mut self, context: &egui::Context) {
        self.apply_effect(
            context,
            reduce_tray_action(self.lifecycle, TrayAction::Open),
        );
    }

    /// Requests a complete application exit after an explicitly confirmed
    /// native installer has been launched successfully.
    pub(crate) fn request_exit(&mut self, context: &egui::Context) {
        self.apply_effect(context, LifecycleEffect::Close);
    }

    pub(crate) fn hidden(&self) -> bool {
        self.lifecycle == WindowLifecycle::HiddenToTray
    }

    pub(crate) fn close_confirmation_requested(&self) -> bool {
        self.close_confirmation_requested
    }

    pub(crate) fn cancel_close_confirmation(&mut self) {
        self.close_confirmation_requested = false;
    }

    pub(crate) fn resolve_close(&mut self, context: &egui::Context, close_policy: ClosePolicy) {
        self.close_confirmation_requested = false;
        let effect = reduce_close_request(self.lifecycle, close_policy, self.tray_ready());
        self.apply_effect(context, effect);
    }

    fn apply_effect(&mut self, context: &egui::Context, effect: LifecycleEffect) {
        match effect {
            LifecycleEffect::None => {}
            LifecycleEffect::Hide => {
                context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.lifecycle = WindowLifecycle::HiddenToTray;
            }
            LifecycleEffect::Show => {
                context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                context.send_viewport_cmd(egui::ViewportCommand::Focus);
                self.lifecycle = WindowLifecycle::Visible;
            }
            LifecycleEffect::Close => {
                self.lifecycle = WindowLifecycle::ExitRequested;
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            LifecycleEffect::Ask => {
                context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_confirmation_requested = true;
            }
        }
    }
}

fn reduce_close_request(
    lifecycle: WindowLifecycle,
    close_policy: ClosePolicy,
    tray_ready: bool,
) -> LifecycleEffect {
    if lifecycle == WindowLifecycle::ExitRequested {
        return LifecycleEffect::None;
    }
    match close_policy {
        ClosePolicy::Ask => LifecycleEffect::Ask,
        ClosePolicy::HideToTray if tray_ready => LifecycleEffect::Hide,
        ClosePolicy::HideToTray | ClosePolicy::Quit => LifecycleEffect::Close,
    }
}

fn reduce_tray_action(lifecycle: WindowLifecycle, action: TrayAction) -> LifecycleEffect {
    match (lifecycle, action) {
        (WindowLifecycle::ExitRequested, _) => LifecycleEffect::None,
        (WindowLifecycle::Visible, TrayAction::Open) => LifecycleEffect::None,
        (_, TrayAction::Open) => LifecycleEffect::Show,
        (_, TrayAction::Quit) => LifecycleEffect::Close,
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
struct DesktopTray {
    _icon: tray_icon::TrayIcon,
    status: tray_icon::menu::MenuItem,
    open_item: tray_icon::menu::MenuItem,
    quit_item: tray_icon::menu::MenuItem,
    open_id: tray_icon::menu::MenuId,
    quit_id: tray_icon::menu::MenuId,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl DesktopTray {
    fn try_new() -> anyhow::Result<Self> {
        use tray_icon::{
            Icon, TrayIconBuilder,
            menu::{Menu, MenuItem, PredefinedMenuItem},
        };

        let status = MenuItem::new("AirWiki · Ready", false, None);
        let open = MenuItem::new("Open AirWiki", true, None);
        let separator = PredefinedMenuItem::separator();
        let quit = MenuItem::new("Quit AirWiki", true, None);
        let menu = Menu::with_items(&[&status, &open, &separator, &quit])?;
        let icon = Icon::from_rgba(tray_pixels(24), 24, 24)?;
        let tray = TrayIconBuilder::new()
            .with_id("airwiki")
            .with_menu(Box::new(menu))
            .with_tooltip("AirWiki")
            .with_icon(icon)
            .with_icon_as_template(cfg!(target_os = "macos"))
            .build()?;
        Ok(Self {
            _icon: tray,
            status,
            open_id: open.id().clone(),
            quit_id: quit.id().clone(),
            open_item: open,
            quit_item: quit,
        })
    }

    fn drain_actions(&self) -> Vec<TrayAction> {
        tray_icon::menu::MenuEvent::receiver()
            .try_iter()
            .filter_map(|event| {
                if event.id == self.open_id {
                    Some(TrayAction::Open)
                } else if event.id == self.quit_id {
                    Some(TrayAction::Quit)
                } else {
                    None
                }
            })
            .collect()
    }

    fn set_status(&self, status: &str) {
        self.status.set_text(status);
    }

    fn set_labels(&self, open: &str, quit: &str) {
        self.open_item.set_text(open);
        self.quit_item.set_text(quit);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct DesktopTray;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl DesktopTray {
    fn try_new() -> anyhow::Result<Self> {
        anyhow::bail!("the desktop tray is unsupported on this platform")
    }

    fn drain_actions(&self) -> Vec<TrayAction> {
        Vec::new()
    }

    fn set_status(&self, _status: &str) {}

    fn set_labels(&self, _open: &str, _quit: &str) {}
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn tray_pixels(size: usize) -> Vec<u8> {
    let center = (size as f32 - 1.0) / 2.0;
    let outer = center - 1.0;
    let inner = outer * 0.58;
    let mut pixels = vec![0_u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            let ring = distance <= outer && distance >= inner;
            let bridge = x > size / 2 && x < size - 2 && y.abs_diff(size / 2) <= 2;
            if ring || bridge {
                let index = (y * size + x) * 4;
                #[cfg(target_os = "windows")]
                {
                    pixels[index] = 22;
                    pixels[index + 1] = 132;
                    pixels[index + 2] = 190;
                }
                pixels[index + 3] = 255;
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_hides_when_tray_is_ready() {
        let effect = reduce_close_request(WindowLifecycle::Visible, ClosePolicy::HideToTray, true);

        assert_eq!(effect, LifecycleEffect::Hide);
    }

    #[test]
    fn ask_policy_defers_the_close_decision() {
        let effect = reduce_close_request(WindowLifecycle::Visible, ClosePolicy::Ask, true);

        assert_eq!(effect, LifecycleEffect::Ask);
    }

    #[test]
    fn close_exits_when_tray_is_unavailable() {
        let effect = reduce_close_request(WindowLifecycle::Visible, ClosePolicy::HideToTray, false);

        assert_eq!(effect, LifecycleEffect::Close);
    }

    #[test]
    fn quit_is_not_cancelled_by_close_to_tray() {
        let effect = reduce_close_request(
            WindowLifecycle::ExitRequested,
            ClosePolicy::HideToTray,
            true,
        );

        assert_eq!(effect, LifecycleEffect::None);
    }

    #[test]
    fn open_restores_a_hidden_window() {
        let effect = reduce_tray_action(WindowLifecycle::HiddenToTray, TrayAction::Open);

        assert_eq!(effect, LifecycleEffect::Show);
    }

    #[test]
    fn quit_closes_a_hidden_window() {
        let effect = reduce_tray_action(WindowLifecycle::HiddenToTray, TrayAction::Quit);

        assert_eq!(effect, LifecycleEffect::Close);
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn tray_asset_has_the_expected_dimensions() {
        assert_eq!(tray_pixels(24).len(), 24 * 24 * 4);
    }
}
