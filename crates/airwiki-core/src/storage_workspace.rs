//! Bounded desktop continuity in the operational database. This is a preference,
//! never authority to read a page: the caller must load the current OKF bundle.

use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::{Database, KnowledgePageId};

pub const WORKSPACE_SIDEBAR_MIN: u16 = 200;
pub const WORKSPACE_SIDEBAR_MAX: u16 = 360;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LocalReadingSelection {
    pub wiki_id: Uuid,
    pub page: KnowledgePageId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DesktopWorkspaceState {
    pub selection: Option<LocalReadingSelection>,
    pub sidebar_width: u16,
    pub sidebar_collapsed: bool,
}

impl std::fmt::Debug for DesktopWorkspaceState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DesktopWorkspaceState")
            .field("has_selection", &self.selection.is_some())
            .field("sidebar_width", &self.sidebar_width)
            .field("sidebar_collapsed", &self.sidebar_collapsed)
            .finish()
    }
}

impl DesktopWorkspaceState {
    pub fn validate(&self) -> Result<()> {
        if !(WORKSPACE_SIDEBAR_MIN..=WORKSPACE_SIDEBAR_MAX).contains(&self.sidebar_width) {
            bail!("desktop workspace panel is outside supported bounds");
        }
        if let Some(selection) = self.selection
            && (selection.wiki_id.is_nil()
                || matches!(selection.page, KnowledgePageId::Concept(id) if id.is_nil()))
        {
            bail!("desktop workspace selection is invalid");
        }
        Ok(())
    }
}

impl Database {
    /// Reads at most one record. Invalid state is reported without changing it;
    /// callers can keep reading with their default layout and Library route.
    pub fn load_desktop_workspace(&self) -> Result<Option<DesktopWorkspaceState>> {
        let stored = self
            .connection()?
            .query_row(
                "SELECT version,wiki_id,page_kind,concept_id,sidebar_width,sidebar_collapsed
             FROM desktop_workspace WHERE singleton=1",
                [],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, u16>(4)?,
                        row.get::<_, u8>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((version, wiki_id, kind, concept_id, sidebar_width, collapsed)) = stored else {
            return Ok(None);
        };
        if version != 1 || collapsed > 1 {
            bail!("desktop workspace version or state is unsupported");
        }
        let selection = match (wiki_id, kind.as_deref(), concept_id) {
            (None, None, None) => None,
            (Some(wiki), Some(kind), concept) => {
                let page = match (kind, concept) {
                    ("index", None) => KnowledgePageId::Index,
                    ("log", None) => KnowledgePageId::Log,
                    ("concept", Some(id)) => KnowledgePageId::Concept(
                        Uuid::parse_str(&id).context("invalid workspace concept identifier")?,
                    ),
                    _ => bail!("desktop workspace page is invalid"),
                };
                Some(LocalReadingSelection {
                    wiki_id: Uuid::parse_str(&wiki).context("invalid workspace wiki identifier")?,
                    page,
                })
            }
            _ => bail!("desktop workspace selection is incomplete"),
        };
        let state = DesktopWorkspaceState {
            selection,
            sidebar_width,
            sidebar_collapsed: collapsed == 1,
        };
        state.validate()?;
        Ok(Some(state))
    }

    /// Atomically replaces the single preference after validating its bounds.
    /// A missing Wiki is allowed: reopening revalidates and falls back to Library.
    pub fn save_desktop_workspace(&self, state: DesktopWorkspaceState) -> Result<()> {
        state.validate()?;
        let (wiki_id, kind, concept_id) = match state.selection {
            None => (None, None, None),
            Some(selection) => {
                let (kind, concept) = match selection.page {
                    KnowledgePageId::Index => ("index", None),
                    KnowledgePageId::Log => ("log", None),
                    KnowledgePageId::Concept(id) => ("concept", Some(id.to_string())),
                };
                (Some(selection.wiki_id.to_string()), Some(kind), concept)
            }
        };
        let updated = self.connection()?.execute(
            "INSERT INTO desktop_workspace(singleton,version,wiki_id,page_kind,concept_id,sidebar_width,sidebar_collapsed)
             VALUES(1,1,?1,?2,?3,?4,?5)
             ON CONFLICT(singleton) DO UPDATE SET version=excluded.version,wiki_id=excluded.wiki_id,
             page_kind=excluded.page_kind,concept_id=excluded.concept_id,sidebar_width=excluded.sidebar_width,
             sidebar_collapsed=excluded.sidebar_collapsed WHERE desktop_workspace.version=1",
            params![wiki_id, kind, concept_id, state.sidebar_width, state.sidebar_collapsed],
        )?;
        if updated != 1 {
            bail!("desktop workspace version is unsupported");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> DesktopWorkspaceState {
        DesktopWorkspaceState {
            selection: Some(LocalReadingSelection {
                wiki_id: Uuid::new_v4(),
                page: KnowledgePageId::Concept(Uuid::new_v4()),
            }),
            sidebar_width: 224,
            sidebar_collapsed: false,
        }
    }

    #[test]
    fn workspace_reopens_and_replaces_one_bounded_preference() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("workspace.sqlite3");
        let database = Database::open(&path)?;
        assert_eq!(database.load_desktop_workspace()?, None);
        let original = state();
        database.save_desktop_workspace(original)?;
        drop(database);
        let reopened = Database::open(&path)?;
        assert_eq!(reopened.load_desktop_workspace()?, Some(original));
        let replacement = DesktopWorkspaceState {
            selection: None,
            sidebar_width: 360,
            sidebar_collapsed: true,
        };
        reopened.save_desktop_workspace(replacement)?;
        assert_eq!(reopened.load_desktop_workspace()?, Some(replacement));
        assert_eq!(
            reopened.connection()?.query_row(
                "SELECT count(*) FROM desktop_workspace",
                [],
                |row| row.get::<_, u32>(0)
            )?,
            1
        );
        Ok(())
    }

    #[test]
    fn workspace_invalid_values_and_failed_write_preserve_previous_state() -> Result<()> {
        let database = Database::in_memory()?;
        let original = state();
        database.save_desktop_workspace(original)?;
        for width in [0, 199, 361, u16::MAX] {
            assert!(
                database
                    .save_desktop_workspace(DesktopWorkspaceState {
                        sidebar_width: width,
                        ..original
                    })
                    .is_err()
            );
        }
        let invalid = DesktopWorkspaceState {
            selection: Some(LocalReadingSelection {
                wiki_id: Uuid::nil(),
                page: KnowledgePageId::Index,
            }),
            ..original
        };
        assert!(database.save_desktop_workspace(invalid).is_err());
        database.connection()?.execute_batch("CREATE TRIGGER fail_workspace_update BEFORE UPDATE ON desktop_workspace BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END;")?;
        assert!(
            database
                .save_desktop_workspace(DesktopWorkspaceState {
                    selection: None,
                    ..original
                })
                .is_err()
        );
        assert_eq!(database.load_desktop_workspace()?, Some(original));
        database
            .connection()?
            .execute_batch("DROP TRIGGER fail_workspace_update;")?;
        database.save_desktop_workspace(DesktopWorkspaceState {
            selection: None,
            ..original
        })?;
        assert!(
            database
                .load_desktop_workspace()?
                .is_some_and(|value| value.selection.is_none())
        );
        Ok(())
    }

    #[test]
    fn workspace_rejects_malformed_or_future_stored_state_without_erasing_it() -> Result<()> {
        let database = Database::in_memory()?;
        database.save_desktop_workspace(state())?;
        database
            .connection()?
            .execute("UPDATE desktop_workspace SET wiki_id=?1", ["x".repeat(36)])?;
        assert!(database.load_desktop_workspace().is_err());
        database.connection()?.execute_batch(
            "PRAGMA ignore_check_constraints=ON; UPDATE desktop_workspace SET version=2;",
        )?;
        assert!(database.load_desktop_workspace().is_err());
        assert!(database.save_desktop_workspace(state()).is_err());
        assert_eq!(
            database.connection()?.query_row(
                "SELECT version FROM desktop_workspace",
                [],
                |row| row.get::<_, u32>(0)
            )?,
            2
        );
        Ok(())
    }

    #[test]
    fn migration_nineteen_preserves_existing_collections_and_starts_without_a_selection()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("migration.sqlite3");
        let database = Database::open(&path)?;
        database.connection()?.execute_batch(
            "INSERT INTO collections(id,name,source_folder,wiki_folder,created_at,updated_at)
             VALUES('synthetic-wiki','Synthetic Wiki','synthetic-source','synthetic-vault','2026-01-01','2026-01-01');
             DROP TABLE desktop_workspace; PRAGMA user_version=18;"
        )?;
        drop(database);
        let migrated = Database::open(&path)?;
        assert_eq!(migrated.schema_version()?, 19);
        assert_eq!(migrated.load_desktop_workspace()?, None);
        assert_eq!(migrated.connection()?.query_row(
            "SELECT name,local_only,peer_shareable,allow_external_ai FROM collections WHERE id='synthetic-wiki'", [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?, row.get::<_, bool>(2)?, row.get::<_, bool>(3)?))
        )?, ("Synthetic Wiki".to_owned(), true, false, false));
        Ok(())
    }
}
