use eframe::egui::{self, Color32, RichText};
use uuid::Uuid;

use crate::i18n::Localization;
use crate::integrations::{
    ChatClientKind, ChatIntegrationsSnapshot, IntegrationAction, IntegrationStatus, IntegrationView,
};
use crate::readiness::OptionalFeatureState;

use super::{page_title, wrap_monospace};

#[derive(Debug)]
pub(super) enum IntegrationsUiAction {
    Run {
        request_id: Uuid,
        action: IntegrationAction,
    },
    OpenCollections,
}

#[derive(Debug, Clone)]
struct Confirmation {
    client: ChatClientKind,
    action: IntegrationAction,
    path: Option<std::path::PathBuf>,
}

#[derive(Debug, Default)]
pub(super) struct ChatIntegrationsUi {
    snapshot: Option<ChatIntegrationsSnapshot>,
    latest_request_id: Option<Uuid>,
    pending_request_id: Option<Uuid>,
    snapshot_before_request: Option<ChatIntegrationsSnapshot>,
    confirmation: Option<Confirmation>,
    inline_error: Option<String>,
}

impl ChatIntegrationsUi {
    pub(super) fn refresh_if_idle(&mut self) -> Option<IntegrationsUiAction> {
        (self.snapshot.is_none() && self.pending_request_id.is_none())
            .then(|| self.start_request(IntegrationAction::Refresh))
    }

    pub(super) fn readiness_state(&self) -> OptionalFeatureState {
        if self.pending_request_id.is_some()
            || self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .integrations
                    .iter()
                    .any(|view| view.status == IntegrationStatus::Configuring)
            })
        {
            return OptionalFeatureState::Working;
        }
        let Some(snapshot) = &self.snapshot else {
            return OptionalFeatureState::Disabled;
        };
        if snapshot
            .integrations
            .iter()
            .any(|view| view.status == IntegrationStatus::Configured)
        {
            return OptionalFeatureState::Ready;
        }
        if snapshot
            .integrations
            .iter()
            .any(|view| matches!(view.status, IntegrationStatus::AwaitingClientApproval))
        {
            return OptionalFeatureState::NeedsPermission;
        }
        if snapshot.integrations.iter().any(|view| {
            matches!(
                view.status,
                IntegrationStatus::UpdateAvailable
                    | IntegrationStatus::Conflict
                    | IntegrationStatus::Error
            )
        }) {
            return OptionalFeatureState::NeedsAttention;
        }
        OptionalFeatureState::Disabled
    }

    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
    ) -> Vec<IntegrationsUiAction> {
        let mut actions = Vec::new();
        if let Some(action) = self.refresh_if_idle() {
            actions.push(action);
        }

        page_title(
            ui,
            &localization.text("integrations-title"),
            &localization.text("integrations-subtitle"),
        );
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.pending_request_id.is_none(),
                    egui::Button::new(localization.text("integrations-refresh")),
                )
                .clicked()
            {
                actions.push(self.start_request(IntegrationAction::Refresh));
            }
            if self.pending_request_id.is_some() {
                ui.spinner();
                ui.label(localization.text("integrations-checking"));
            }
        });

        if let Some(error) = &self.inline_error {
            ui.colored_label(Color32::from_rgb(220, 70, 70), error);
        }
        ui.add_space(8.0);
        self.collection_summary(ui, localization, &mut actions);
        ui.add_space(12.0);

        if let Some(snapshot) = self.snapshot.clone() {
            let list_height = ui.available_height().max(0.0);
            egui::ScrollArea::vertical()
                .id_salt("integration_cards")
                .max_height(list_height)
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    for integration in &snapshot.integrations {
                        self.integration_card(ui, localization, integration, &mut actions);
                        ui.add_space(8.0);
                    }
                });
        } else if self.pending_request_id.is_some() {
            ui.spinner();
        }

        self.confirmation_window(ui.ctx(), localization, &mut actions);
        actions
    }

    pub(super) fn apply_result(
        &mut self,
        request_id: Uuid,
        result: Result<ChatIntegrationsSnapshot, String>,
    ) {
        if self.latest_request_id != Some(request_id) {
            return;
        }
        self.pending_request_id = None;
        match result {
            Ok(snapshot) => {
                self.snapshot = Some(snapshot);
                self.snapshot_before_request = None;
                self.inline_error = None;
            }
            Err(error) => {
                if let Some(snapshot) = self.snapshot_before_request.take() {
                    self.snapshot = Some(snapshot);
                }
                self.inline_error = Some(error);
            }
        }
    }

    pub(super) fn collections_changed(&mut self) {
        if self.pending_request_id.is_none() {
            self.snapshot = None;
        }
    }

    fn collection_summary(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        actions: &mut Vec<IntegrationsUiAction>,
    ) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.heading(localization.text("integrations-privacy-title"));
            let count = self
                .snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.external_ai_collection_count);
            if count == 0 {
                ui.colored_label(
                    Color32::from_rgb(205, 145, 30),
                    localization.text("integrations-no-chat-folders"),
                );
            } else {
                let mut arguments = fluent_bundle::FluentArgs::new();
                arguments.set("count", count);
                ui.label(
                    localization.text_with("integrations-chat-folder-count", Some(&arguments)),
                );
            }
            ui.label(
                RichText::new(localization.text("integrations-permissions-reminder"))
                    .small()
                    .color(Color32::GRAY),
            );
            if ui
                .button(localization.text("integrations-manage-folders"))
                .clicked()
            {
                actions.push(IntegrationsUiAction::OpenCollections);
            }
        });
    }

    fn integration_card(
        &mut self,
        ui: &mut egui::Ui,
        localization: &Localization,
        integration: &IntegrationView,
        actions: &mut Vec<IntegrationsUiAction>,
    ) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(integration.client.display_name());
                let (label, color) = status_presentation(localization, &integration.status);
                ui.colored_label(color, label);
                if integration.activity_recent {
                    ui.colored_label(
                        Color32::from_rgb(70, 160, 110),
                        localization.text("integrations-recent-activity"),
                    );
                }
            });
            if let Some(version) = &integration.detected_version {
                ui.label(RichText::new(version).small().color(Color32::GRAY));
            }
            ui.label(integration_summary(localization, &integration.status));
            ui.collapsing(localization.text("action-details"), |ui| {
                ui.label(&integration.detail);
                if let Some(path) = &integration.planned_path {
                    ui.label(
                        RichText::new(localization.text("integrations-managed-resource"))
                            .small()
                            .color(Color32::GRAY),
                    );
                    wrap_monospace(ui, path.display().to_string());
                }
            });
            if integration.restart_required
                && matches!(
                    integration.status,
                    IntegrationStatus::Configured | IntegrationStatus::UpdateAvailable
                )
            {
                ui.label(
                    RichText::new(localization.text("integrations-restart-chatgpt"))
                        .small()
                        .color(Color32::from_rgb(205, 145, 30)),
                );
            }
            ui.horizontal(|ui| {
                let enabled = self.pending_request_id.is_none();
                match integration.status {
                    IntegrationStatus::Available => {
                        if ui
                            .add_enabled(
                                enabled,
                                egui::Button::new(localization.text("integrations-connect")),
                            )
                            .clicked()
                        {
                            self.confirmation = Some(Confirmation {
                                client: integration.client,
                                action: IntegrationAction::Connect(integration.client),
                                path: integration.planned_path.clone(),
                            });
                        }
                    }
                    IntegrationStatus::Configured => {
                        if integration.client == ChatClientKind::ClaudeDesktop {
                            if ui
                                .add_enabled(
                                    enabled,
                                    egui::Button::new(
                                        localization.text("integrations-open-settings"),
                                    ),
                                )
                                .clicked()
                            {
                                actions.push(
                                    self.start_request(IntegrationAction::OpenClaudeSettings),
                                );
                            }
                        } else if ui
                            .add_enabled(
                                enabled,
                                egui::Button::new(localization.text("integrations-disconnect")),
                            )
                            .clicked()
                        {
                            self.confirmation = Some(Confirmation {
                                client: integration.client,
                                action: IntegrationAction::Disconnect(integration.client),
                                path: integration.planned_path.clone(),
                            });
                        }
                    }
                    IntegrationStatus::UpdateAvailable => {
                        if ui
                            .add_enabled(
                                enabled,
                                egui::Button::new(localization.text("integrations-update")),
                            )
                            .clicked()
                        {
                            self.confirmation = Some(Confirmation {
                                client: integration.client,
                                action: IntegrationAction::Connect(integration.client),
                                path: integration.planned_path.clone(),
                            });
                        }
                    }
                    IntegrationStatus::AwaitingClientApproval => {
                        if ui
                            .add_enabled(
                                enabled,
                                egui::Button::new(localization.text("integrations-installed")),
                            )
                            .on_hover_text(localization.text("integrations-installed-help"))
                            .clicked()
                        {
                            actions.push(
                                self.start_request(IntegrationAction::ConfirmClaudeInstalled),
                            );
                        }
                        if ui
                            .add_enabled(
                                enabled,
                                egui::Button::new(localization.text("integrations-open-claude")),
                            )
                            .clicked()
                        {
                            actions.push(self.start_request(IntegrationAction::OpenClaudeSettings));
                        }
                    }
                    IntegrationStatus::NotInstalled
                    | IntegrationStatus::Configuring
                    | IntegrationStatus::Conflict
                    | IntegrationStatus::Unsupported
                    | IntegrationStatus::Error => {}
                }
            });
        });
    }

    fn confirmation_window(
        &mut self,
        context: &egui::Context,
        localization: &Localization,
        actions: &mut Vec<IntegrationsUiAction>,
    ) {
        let Some(confirmation) = self.confirmation.clone() else {
            return;
        };
        let title = match confirmation.action {
            IntegrationAction::Connect(_) => localization.text("integrations-confirm-connect"),
            IntegrationAction::Disconnect(_) => {
                localization.text("integrations-confirm-disconnect")
            }
            IntegrationAction::Refresh
            | IntegrationAction::ConfirmClaudeInstalled
            | IntegrationAction::OpenClaudeSettings => return,
        };
        egui::Window::new(title)
            .id(egui::Id::new("chat_integration_confirmation"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.heading(confirmation.client.display_name());
                match confirmation.action {
                    IntegrationAction::Connect(_) => {
                        ui.label(localization.text("integrations-confirm-connect-body"));
                        ui.colored_label(
                            Color32::from_rgb(205, 145, 30),
                            localization.text("integrations-confirm-cloud-warning"),
                        );
                        ui.label(localization.text("integrations-confirm-open-reminder"));
                    }
                    IntegrationAction::Disconnect(_) => {
                        ui.label(localization.text("integrations-confirm-disconnect-body"));
                    }
                    IntegrationAction::Refresh
                    | IntegrationAction::ConfirmClaudeInstalled
                    | IntegrationAction::OpenClaudeSettings => {}
                }
                if let Some(path) = &confirmation.path {
                    ui.label(
                        RichText::new(localization.text("integrations-planned-path"))
                            .small()
                            .color(Color32::GRAY),
                    );
                    wrap_monospace(ui, path.display().to_string());
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(localization.text("action-cancel")).clicked() {
                        self.confirmation = None;
                    }
                    if ui.button(localization.text("action-confirm")).clicked() {
                        self.confirmation = None;
                        actions.push(self.start_request(confirmation.action));
                    }
                });
            });
    }

    fn start_request(&mut self, action: IntegrationAction) -> IntegrationsUiAction {
        let request_id = Uuid::new_v4();
        self.snapshot_before_request = self.snapshot.clone();
        self.latest_request_id = Some(request_id);
        self.pending_request_id = Some(request_id);
        self.inline_error = None;
        if let Some(snapshot) = self.snapshot.as_mut()
            && let Some(client) = action_client(action)
            && let Some(view) = snapshot
                .integrations
                .iter_mut()
                .find(|view| view.client == client)
        {
            view.status = IntegrationStatus::Configuring;
        }
        IntegrationsUiAction::Run { request_id, action }
    }
}

fn action_client(action: IntegrationAction) -> Option<ChatClientKind> {
    match action {
        IntegrationAction::Connect(client) | IntegrationAction::Disconnect(client) => Some(client),
        IntegrationAction::ConfirmClaudeInstalled => Some(ChatClientKind::ClaudeDesktop),
        IntegrationAction::Refresh | IntegrationAction::OpenClaudeSettings => None,
    }
}

fn status_presentation(
    localization: &Localization,
    status: &IntegrationStatus,
) -> (String, Color32) {
    let (message, color) = match status {
        IntegrationStatus::NotInstalled => ("integration-status-not-installed", Color32::GRAY),
        IntegrationStatus::Available => (
            "integration-status-available",
            Color32::from_rgb(70, 140, 210),
        ),
        IntegrationStatus::Configuring => (
            "integration-status-configuring",
            Color32::from_rgb(205, 145, 30),
        ),
        IntegrationStatus::AwaitingClientApproval => (
            "integration-status-awaiting-approval",
            Color32::from_rgb(205, 145, 30),
        ),
        IntegrationStatus::Configured => (
            "integration-status-configured",
            Color32::from_rgb(70, 160, 110),
        ),
        IntegrationStatus::UpdateAvailable => (
            "integration-status-update-available",
            Color32::from_rgb(205, 145, 30),
        ),
        IntegrationStatus::Conflict => (
            "integration-status-conflict",
            Color32::from_rgb(220, 70, 70),
        ),
        IntegrationStatus::Unsupported => (
            "integration-status-unsupported",
            Color32::from_rgb(220, 70, 70),
        ),
        IntegrationStatus::Error => ("integration-status-error", Color32::from_rgb(220, 70, 70)),
    };
    (localization.text(message), color)
}

fn integration_summary(localization: &Localization, status: &IntegrationStatus) -> String {
    localization.text(match status {
        IntegrationStatus::NotInstalled => "integration-summary-not-installed",
        IntegrationStatus::Available => "integration-summary-available",
        IntegrationStatus::Configuring => "integration-summary-configuring",
        IntegrationStatus::AwaitingClientApproval => "integration-summary-awaiting-approval",
        IntegrationStatus::Configured => "integration-summary-configured",
        IntegrationStatus::UpdateAvailable => "integration-summary-update-available",
        IntegrationStatus::Conflict => "integration-summary-conflict",
        IntegrationStatus::Unsupported => "integration-summary-unsupported",
        IntegrationStatus::Error => "integration-summary-error",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(detail: &str) -> ChatIntegrationsSnapshot {
        ChatIntegrationsSnapshot {
            integrations: vec![IntegrationView {
                client: ChatClientKind::ChatGptDesktop,
                status: IntegrationStatus::Available,
                detected_version: None,
                detail: detail.to_owned(),
                planned_path: None,
                activity_recent: false,
                restart_required: false,
            }],
            external_ai_collection_count: 0,
        }
    }

    #[test]
    fn stale_integration_results_are_discarded() {
        let mut ui = ChatIntegrationsUi::default();
        let first = match ui.start_request(IntegrationAction::Refresh) {
            IntegrationsUiAction::Run { request_id, .. } => request_id,
            IntegrationsUiAction::OpenCollections => panic!("unexpected navigation"),
        };
        let second = match ui.start_request(IntegrationAction::Refresh) {
            IntegrationsUiAction::Run { request_id, .. } => request_id,
            IntegrationsUiAction::OpenCollections => panic!("unexpected navigation"),
        };

        ui.apply_result(first, Ok(snapshot("stale")));
        ui.apply_result(second, Ok(snapshot("current")));

        assert_eq!(
            ui.snapshot.as_ref().unwrap().integrations[0].detail,
            "current"
        );
    }

    #[test]
    fn one_pending_request_blocks_a_second_ui_operation() {
        let mut ui = ChatIntegrationsUi::default();
        let action = ui.start_request(IntegrationAction::Refresh);

        assert!(matches!(action, IntegrationsUiAction::Run { .. }));
        assert!(ui.pending_request_id.is_some());
    }

    #[test]
    fn failed_operation_restores_the_previous_card_state() {
        let mut ui = ChatIntegrationsUi {
            snapshot: Some(snapshot("available")),
            ..ChatIntegrationsUi::default()
        };
        let request_id =
            match ui.start_request(IntegrationAction::Connect(ChatClientKind::ChatGptDesktop)) {
                IntegrationsUiAction::Run { request_id, .. } => request_id,
                IntegrationsUiAction::OpenCollections => panic!("unexpected navigation"),
            };

        ui.apply_result(request_id, Err("failed".to_owned()));

        assert_eq!(
            ui.snapshot.as_ref().unwrap().integrations[0].status,
            IntegrationStatus::Available
        );
    }

    #[test]
    fn readiness_uses_detected_client_state_instead_of_a_fixed_disabled_value() {
        let mut configured = snapshot("configured");
        configured.integrations[0].status = IntegrationStatus::Configured;
        let ui = ChatIntegrationsUi {
            snapshot: Some(configured),
            ..ChatIntegrationsUi::default()
        };
        assert_eq!(ui.readiness_state(), OptionalFeatureState::Ready);

        let mut conflict = snapshot("conflict");
        conflict.integrations[0].status = IntegrationStatus::Conflict;
        let ui = ChatIntegrationsUi {
            snapshot: Some(conflict),
            ..ChatIntegrationsUi::default()
        };
        assert_eq!(ui.readiness_state(), OptionalFeatureState::NeedsAttention);
    }
}
