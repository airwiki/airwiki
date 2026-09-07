use airwiki_core::{DesktopWorkspaceState, KnowledgePageId, LocalReadingSelection};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use ts_rs::TS;

use crate::{AppRuntime, UiError, parse_uuid, send_command, worker::WorkerCommand};

#[derive(Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub(crate) struct WorkspaceStateDto {
    selection: Option<WorkspaceSelectionDto>,
    sidebar_width: u16,
    sidebar_collapsed: bool,
}

#[derive(Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub(crate) struct WorkspaceSelectionDto {
    wiki_id: String,
    page: WorkspacePageDto,
}

#[derive(Clone, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum WorkspacePageDto {
    Index,
    Log,
    Concept {
        #[serde(rename = "conceptId")]
        #[ts(rename = "conceptId")]
        concept_id: String,
    },
}

impl TryFrom<WorkspaceStateDto> for DesktopWorkspaceState {
    type Error = UiError;

    fn try_from(value: WorkspaceStateDto) -> Result<Self, UiError> {
        let selection = value
            .selection
            .map(|selection| {
                Ok(LocalReadingSelection {
                    wiki_id: parse_uuid(&selection.wiki_id)?,
                    page: match selection.page {
                        WorkspacePageDto::Index => KnowledgePageId::Index,
                        WorkspacePageDto::Log => KnowledgePageId::Log,
                        WorkspacePageDto::Concept { concept_id } => {
                            KnowledgePageId::Concept(parse_uuid(&concept_id)?)
                        }
                    },
                })
            })
            .transpose()?;
        let state = Self {
            selection,
            sidebar_width: value.sidebar_width,
            sidebar_collapsed: value.sidebar_collapsed,
        };
        state
            .validate()
            .map_err(|_| UiError::invalid("workspaceStateInvalid"))?;
        Ok(state)
    }
}

impl From<DesktopWorkspaceState> for WorkspaceStateDto {
    fn from(value: DesktopWorkspaceState) -> Self {
        Self {
            selection: value.selection.map(|selection| WorkspaceSelectionDto {
                wiki_id: selection.wiki_id.to_string(),
                page: match selection.page {
                    KnowledgePageId::Index => WorkspacePageDto::Index,
                    KnowledgePageId::Log => WorkspacePageDto::Log,
                    KnowledgePageId::Concept(id) => WorkspacePageDto::Concept {
                        concept_id: id.to_string(),
                    },
                },
            }),
            sidebar_width: value.sidebar_width,
            sidebar_collapsed: value.sidebar_collapsed,
        }
    }
}

#[tauri::command]
pub(crate) async fn load_desktop_workspace(
    runtime: tauri::State<'_, AppRuntime>,
) -> Result<Option<WorkspaceStateDto>, UiError> {
    let (completed, completion) = oneshot::channel();
    send_command(&runtime, WorkerCommand::LoadDesktopWorkspace { completed }).await?;
    completion
        .await
        .map_err(|_| UiError::internal())?
        .map(|state| state.map(Into::into))
        .map_err(|_| UiError::internal())
}

#[tauri::command]
pub(crate) async fn save_desktop_workspace(
    runtime: tauri::State<'_, AppRuntime>,
    state: WorkspaceStateDto,
) -> Result<(), UiError> {
    let state = state.try_into()?;
    let (completed, completion) = oneshot::channel();
    send_command(
        &runtime,
        WorkerCommand::SaveDesktopWorkspace { state, completed },
    )
    .await?;
    completion
        .await
        .map_err(|_| UiError::internal())?
        .map_err(|_| UiError::internal())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_input_rejects_paths_remote_destinations_and_unbounded_layouts()
    -> anyhow::Result<()> {
        let valid =
            serde_json::json!({"selection": null, "sidebarWidth":224, "sidebarCollapsed":false});
        assert!(
            DesktopWorkspaceState::try_from(serde_json::from_value::<WorkspaceStateDto>(
                valid.clone()
            )?)
            .is_ok()
        );
        let mut invalid = valid.clone();
        invalid["sidebarWidth"] = serde_json::json!(361);
        assert!(
            DesktopWorkspaceState::try_from(serde_json::from_value::<WorkspaceStateDto>(invalid)?)
                .is_err()
        );
        let mut invalid = valid.clone();
        invalid["selection"] = serde_json::json!({"wikiId":"00000000-0000-4000-8000-000000000001","page":{"kind":"concept","conceptId":"../../synthetic.md"}});
        assert!(
            DesktopWorkspaceState::try_from(serde_json::from_value::<WorkspaceStateDto>(invalid)?)
                .is_err()
        );
        for field in ["query", "sourcePath", "publisherId"] {
            let mut invalid = valid.clone();
            invalid[field] = serde_json::json!("synthetic");
            assert!(serde_json::from_value::<WorkspaceStateDto>(invalid).is_err());
        }
        Ok(())
    }
}
