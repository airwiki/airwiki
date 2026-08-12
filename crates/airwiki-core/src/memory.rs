use std::path::{Path, PathBuf};

use airwiki_types::{CollectionPolicy, ConceptType};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::okf::atomic_write;
use crate::storage::ApplicationWikiRole;
use crate::{
    CollectionRecord, Database, IndexingMode, OkfConceptProjectionRecord, OkfImportedConcept,
    WikiOrigin,
};

pub const AI_MEMORY_CONCEPT_MAX_BYTES: usize = 48 * 1024;

#[derive(Debug, Clone)]
pub struct AiMemoryService {
    database: Database,
    vaults: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiMemoryConceptInput {
    pub title: String,
    pub description: String,
    pub concept_type: ConceptType,
    pub tags: Vec<String>,
    pub body_markdown: String,
}

impl AiMemoryService {
    pub fn new(database: Database, vaults: impl Into<PathBuf>) -> Self {
        Self {
            database,
            vaults: vaults.into(),
        }
    }

    pub fn list(&self, app_id: Uuid) -> Result<Vec<CollectionRecord>> {
        self.database.application_memory_wikis(app_id)
    }

    pub fn create(&self, app_id: Uuid, app_name: &str, name: &str) -> Result<CollectionRecord> {
        self.database.consume_application_rate_limit(app_id, true)?;
        let staging_id = Uuid::new_v4();
        let staging = self.vaults.join(format!(".memory-{staging_id}"));
        std::fs::create_dir_all(&staging).context("could not create AI memory staging")?;
        let index = format!(
            "---\nokf_version: \"0.2\"\ngenerated:\n  by: \"airwiki:{}\"\n  at: \"{}\"\n---\n\n# {}\n",
            app_id,
            Utc::now().to_rfc3339(),
            safe_heading(name),
        );
        if let Err(error) = atomic_write(&staging.join("index.md"), index.as_bytes()) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        let placeholder = self.vaults.join(format!(".pending-{staging_id}"));
        let record = match self.database.create_collection_with_origin(
            name,
            &placeholder,
            &placeholder,
            CollectionPolicy::local_only(),
            WikiOrigin::AiMemory,
            IndexingMode::NotApplicable,
        ) {
            Ok(record) => record,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(error);
            }
        };
        let bundle = self.vaults.join(record.id.to_string());
        if let Err(error) = std::fs::rename(&staging, &bundle) {
            let _ = self.database.delete_collection_record(record.id);
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error).context("could not activate AI memory bundle");
        }
        self.database
            .update_collection_paths(record.id, &bundle, &bundle)?;
        self.database.set_application_wiki_role(
            app_id,
            record.id,
            Some(ApplicationWikiRole::Owner),
        )?;
        let _ = app_name;
        self.database
            .collection(record.id)?
            .context("created AI memory disappeared")
    }

    pub fn get(&self, app_id: Uuid, wiki_id: Uuid) -> Result<Vec<OkfConceptProjectionRecord>> {
        self.require_role(app_id, wiki_id, false)?;
        self.database.list_okf_concept_projection(wiki_id)
    }

    pub fn write(
        &self,
        app_id: Uuid,
        app_name: &str,
        wiki_id: Uuid,
        concept_id: Option<Uuid>,
        expected_fingerprint: Option<&str>,
        input: &AiMemoryConceptInput,
    ) -> Result<OkfConceptProjectionRecord> {
        self.require_role(app_id, wiki_id, true)?;
        self.database
            .consume_application_rate_limit(app_id, false)?;
        validate_input(input)?;
        let existing = self.database.list_okf_concept_projection(wiki_id)?;
        let existing = concept_id
            .map(|id| {
                existing
                    .into_iter()
                    .find(|concept| concept.concept_id == id)
                    .context("AI memory concept does not exist")
            })
            .transpose()?;
        match (&existing, expected_fingerprint) {
            (Some(existing), Some(expected)) if existing.fingerprint == expected => {}
            (Some(_), _) => bail!("AI memory concept fingerprint is stale"),
            (None, Some(_)) => bail!("new AI memory concepts cannot have an expected fingerprint"),
            (None, None) => {}
        }
        let logical_path = existing.as_ref().map_or_else(
            || format!("concepts/{}.md", Uuid::new_v4()),
            |concept| concept.logical_path.clone(),
        );
        let wiki = self
            .database
            .collection(wiki_id)?
            .context("AI memory wiki does not exist")?;
        let rendered = render_concept(app_id, app_name, input, "stable")?;
        if rendered.len() > AI_MEMORY_CONCEPT_MAX_BYTES {
            bail!("AI memory concept exceeds the size limit");
        }
        let path = checked_memory_path(&wiki.wiki_folder, &logical_path)?;
        atomic_write(&path, rendered.as_bytes())?;
        let imported =
            imported_concept(&logical_path, input, "stable", &rendered, app_id, app_name);
        self.database
            .upsert_okf_concept_projection(wiki_id, &imported)
    }

    pub fn deprecate(
        &self,
        app_id: Uuid,
        app_name: &str,
        wiki_id: Uuid,
        concept_id: Uuid,
        expected_fingerprint: &str,
    ) -> Result<OkfConceptProjectionRecord> {
        self.require_role(app_id, wiki_id, true)?;
        self.database
            .consume_application_rate_limit(app_id, false)?;
        let concept = self
            .database
            .list_okf_concept_projection(wiki_id)?
            .into_iter()
            .find(|concept| concept.concept_id == concept_id)
            .context("AI memory concept does not exist")?;
        if concept.fingerprint != expected_fingerprint {
            bail!("AI memory concept fingerprint is stale");
        }
        let wiki = self
            .database
            .collection(wiki_id)?
            .context("AI memory wiki does not exist")?;
        let input = AiMemoryConceptInput {
            title: concept.title.clone(),
            description: concept.description.clone(),
            concept_type: concept.concept_type.clone(),
            tags: concept.tags.clone(),
            body_markdown: read_body(&checked_memory_path(
                &wiki.wiki_folder,
                &concept.logical_path,
            )?)?,
        };
        let rendered = render_concept(app_id, app_name, &input, "deprecated")?;
        atomic_write(
            &checked_memory_path(&wiki.wiki_folder, &concept.logical_path)?,
            rendered.as_bytes(),
        )?;
        let imported = imported_concept(
            &concept.logical_path,
            &input,
            "deprecated",
            &rendered,
            app_id,
            app_name,
        );
        self.database
            .upsert_okf_concept_projection(wiki_id, &imported)
    }

    fn require_role(
        &self,
        app_id: Uuid,
        wiki_id: Uuid,
        write: bool,
    ) -> Result<ApplicationWikiRole> {
        let role = self
            .database
            .application_wiki_role(app_id, wiki_id)?
            .context("application is not authorized for this AI memory")?;
        if write && role == ApplicationWikiRole::Reader {
            bail!("application has read-only access to this AI memory");
        }
        Ok(role)
    }
}

#[derive(Serialize)]
struct MemoryFrontmatter<'a> {
    r#type: String,
    title: &'a str,
    description: &'a str,
    tags: &'a [String],
    status: &'a str,
    generated: Generated<'a>,
}

#[derive(Serialize)]
struct Generated<'a> {
    by: String,
    at: String,
    application: &'a str,
}

fn render_concept(
    app_id: Uuid,
    app_name: &str,
    input: &AiMemoryConceptInput,
    status: &str,
) -> Result<String> {
    let frontmatter = MemoryFrontmatter {
        r#type: input.concept_type.to_string(),
        title: input.title.trim(),
        description: input.description.trim(),
        tags: &input.tags,
        status,
        generated: Generated {
            by: format!("airwiki:{app_id}"),
            at: Utc::now().to_rfc3339(),
            application: app_name,
        },
    };
    Ok(format!(
        "---\n{}---\n\n{}\n",
        serde_yaml::to_string(&frontmatter)?,
        input.body_markdown.trim()
    ))
}

fn imported_concept(
    path: &str,
    input: &AiMemoryConceptInput,
    status: &str,
    rendered: &str,
    app_id: Uuid,
    app_name: &str,
) -> OkfImportedConcept {
    OkfImportedConcept {
        logical_path: path.to_owned(),
        concept_type: input.concept_type.to_string(),
        title: input.title.trim().to_owned(),
        description: input.description.trim().to_owned(),
        tags: input.tags.clone(),
        lifecycle_status: status.to_owned(),
        generated: serde_yaml::to_value(Generated {
            by: format!("airwiki:{app_id}"),
            at: Utc::now().to_rfc3339(),
            application: app_name,
        })
        .ok(),
        verified: None,
        sources: None,
        version: None,
        unknown_frontmatter: serde_yaml::Value::Mapping(Default::default()),
        fingerprint: hex::encode(Sha256::digest(rendered.as_bytes())),
        search_text: input.body_markdown.clone(),
    }
}

fn validate_input(input: &AiMemoryConceptInput) -> Result<()> {
    if input.title.trim().is_empty()
        || input.title.chars().count() > 200
        || input.description.chars().count() > 2_000
        || input.tags.len() > 20
    {
        bail!("AI memory concept metadata is invalid");
    }
    Ok(())
}

fn checked_memory_path(root: &Path, logical_path: &str) -> Result<PathBuf> {
    if logical_path.contains('\\')
        || logical_path.starts_with('/')
        || logical_path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        bail!("AI memory path is invalid");
    }
    let path = root.join(logical_path);
    if !path.starts_with(root) {
        bail!("AI memory path escapes its bundle");
    }
    Ok(path)
}

fn read_body(path: &Path) -> Result<String> {
    let markdown = std::fs::read_to_string(path)?;
    let rest = markdown
        .strip_prefix("---\n")
        .context("AI memory frontmatter is missing")?;
    let end = rest
        .find("\n---\n")
        .context("AI memory frontmatter is incomplete")?;
    Ok(rest[end + 5..].trim().to_owned())
}

fn safe_heading(value: &str) -> String {
    value.trim().replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> (tempfile::TempDir, Database, AiMemoryService, Uuid) {
        let temp = tempfile::tempdir().unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = Uuid::new_v4();
        database
            .create_application_capability(
                app_id,
                "Codex",
                "codex",
                "0123456789abcdef",
                &"a".repeat(64),
            )
            .unwrap();
        let service = AiMemoryService::new(database.clone(), temp.path());
        (temp, database, service, app_id)
    }

    #[test]
    fn owner_can_create_edit_and_deprecate_a_memory() {
        let (_temp, _database, service, app_id) = service();
        let wiki = service.create(app_id, "Codex", "Project memory").unwrap();
        let input = AiMemoryConceptInput {
            title: "Decision".to_owned(),
            description: "Architecture decision".to_owned(),
            concept_type: ConceptType::Other("Decision".to_owned()),
            tags: vec!["architecture".to_owned()],
            body_markdown: "Use a local-first design.".to_owned(),
        };

        let created = service
            .write(app_id, "Codex", wiki.id, None, None, &input)
            .unwrap();
        let deprecated = service
            .deprecate(
                app_id,
                "Codex",
                wiki.id,
                created.concept_id,
                &created.fingerprint,
            )
            .unwrap();

        assert_eq!(deprecated.lifecycle_status, "deprecated");
        assert_eq!(service.list(app_id).unwrap().len(), 1);
    }

    #[test]
    fn stale_fingerprint_and_cross_application_access_fail_closed() {
        let (_temp, database, service, app_id) = service();
        let wiki = service.create(app_id, "Codex", "Private memory").unwrap();
        let input = AiMemoryConceptInput {
            title: "Fact".to_owned(),
            description: String::new(),
            concept_type: ConceptType::Reference,
            tags: Vec::new(),
            body_markdown: "Private fact".to_owned(),
        };
        let created = service
            .write(app_id, "Codex", wiki.id, None, None, &input)
            .unwrap();
        assert!(
            service
                .write(
                    app_id,
                    "Codex",
                    wiki.id,
                    Some(created.concept_id),
                    Some("stale"),
                    &input,
                )
                .is_err()
        );
        let other = Uuid::new_v4();
        database
            .create_application_capability(
                other,
                "Claude",
                "claude",
                "fedcba9876543210",
                &"b".repeat(64),
            )
            .unwrap();
        assert!(service.get(other, wiki.id).is_err());
    }
}
