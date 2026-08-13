use std::path::{Path, PathBuf};

use airwiki_types::{ActorId, ConceptAssurance, ConceptType, FreshnessState, TrustTier};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::okf::atomic_write;
use crate::okf_import::append_human_verification;
use crate::storage::ApplicationWikiRole;
use crate::{
    CollectionRecord, Database, ManagedBundleMutationState, NewManagedCollection,
    OkfConceptProjectionRecord, OkfImportValidator, OkfImportedConcept, WikiOrigin,
};

pub const AI_MEMORY_CONCEPT_MAX_BYTES: usize = 48 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ManagedBundleRecoveryReport {
    pub recovered: usize,
    pub pending: usize,
}

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

struct AttributedMemoryWrite<'a> {
    quota_owner_app_id: Uuid,
    wiki_id: Uuid,
    concept_id: Option<Uuid>,
    expected_fingerprint: Option<&'a str>,
    input: &'a AiMemoryConceptInput,
    producer: &'a ActorId,
    verifier: Option<&'a ActorId>,
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

    pub fn recover_pending(&self) -> Result<ManagedBundleRecoveryReport> {
        let mut report = ManagedBundleRecoveryReport::default();
        for mutation in self.database.pending_managed_bundle_mutations()? {
            let Some(wiki) = self.database.collection(mutation.collection_id)? else {
                self.database
                    .complete_managed_bundle_mutation(mutation.id)?;
                continue;
            };
            if !matches!(wiki.origin, WikiOrigin::AiMemory | WikiOrigin::ImportedOkf) {
                self.database.set_managed_bundle_mutation_state(
                    mutation.id,
                    ManagedBundleMutationState::RecoveryRequired,
                )?;
                report.pending = report.pending.saturating_add(1);
                continue;
            }
            if mutation.state == ManagedBundleMutationState::Prepared
                && !wiki.wiki_folder.exists()
                && mutation.logical_path == "."
            {
                cleanup_managed_staging(&self.vaults, wiki.id)?;
                self.database.delete_collection_record(wiki.id)?;
                report.recovered = report.recovered.saturating_add(1);
                continue;
            }
            let recovered = (|| {
                let imported = OkfImportValidator::validate_directory(&wiki.wiki_folder)?;
                self.database
                    .replace_okf_concept_projection(wiki.id, &imported.concepts)?;
                if wiki.origin == WikiOrigin::AiMemory {
                    let concepts = self.database.list_okf_concept_projection(wiki.id)?;
                    regenerate_index(&wiki.wiki_folder, &wiki.name, &concepts)?;
                }
                let managed_size = managed_bundle_size(&wiki.wiki_folder)?;
                self.database.update_collection_okf_metadata(
                    wiki.id,
                    imported.declared_okf_version.as_deref(),
                    &imported.compatibility,
                    managed_size,
                )?;
                self.database.complete_managed_bundle_mutation(mutation.id)
            })();
            if recovered.is_ok() {
                report.recovered = report.recovered.saturating_add(1);
            } else {
                self.database.set_managed_bundle_mutation_state(
                    mutation.id,
                    ManagedBundleMutationState::RecoveryRequired,
                )?;
                report.pending = report.pending.saturating_add(1);
            }
        }
        Ok(report)
    }

    pub fn create(&self, app_id: Uuid, name: &str) -> Result<CollectionRecord> {
        self.database.consume_application_rate_limit(app_id, true)?;
        let _producer = self.application_producer(app_id)?;
        let wiki_id = Uuid::new_v4();
        let staging = self.vaults.join(format!(".memory-{wiki_id}"));
        std::fs::create_dir_all(&staging).context("could not create AI memory staging")?;
        let index = render_index(name, &[]);
        if let Err(error) = atomic_write(&staging.join("index.md"), index.as_bytes()) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        let bundle = self.vaults.join(wiki_id.to_string());
        let (record, mutation) =
            match self
                .database
                .create_managed_collection_with_mutation(NewManagedCollection {
                    id: wiki_id,
                    name: name.to_owned(),
                    bundle_root: bundle.clone(),
                    origin: WikiOrigin::AiMemory,
                    replacement_fingerprint: hex::encode(Sha256::digest(index.as_bytes())),
                    owner_app_id: Some(app_id),
                }) {
                Ok(prepared) => prepared,
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&staging);
                    return Err(error);
                }
            };
        if let Err(error) = std::fs::rename(&staging, &bundle) {
            let _ = self.database.delete_collection_record(record.id);
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error).context("could not activate AI memory bundle");
        }
        self.database.set_managed_bundle_mutation_state(
            mutation.id,
            ManagedBundleMutationState::FileReplaced,
        )?;
        let managed_size = managed_bundle_size(&bundle)?;
        if let Err(error) =
            self.database
                .ensure_application_managed_size(app_id, record.id, managed_size)
        {
            let _ = self.database.delete_collection_record(record.id);
            let _ = std::fs::remove_dir_all(&bundle);
            return Err(error);
        }
        self.database.update_collection_okf_metadata(
            record.id,
            Some("0.2"),
            &airwiki_types::OkfCompatibility::DeclaredV02,
            managed_size,
        )?;
        self.database
            .complete_managed_bundle_mutation(mutation.id)?;
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
        wiki_id: Uuid,
        concept_id: Option<Uuid>,
        expected_fingerprint: Option<&str>,
        input: &AiMemoryConceptInput,
    ) -> Result<OkfConceptProjectionRecord> {
        self.require_role(app_id, wiki_id, true)?;
        self.database
            .consume_application_rate_limit(app_id, false)?;
        let producer = self.application_producer(app_id)?;
        self.write_attributed(AttributedMemoryWrite {
            quota_owner_app_id: app_id,
            wiki_id,
            concept_id,
            expected_fingerprint,
            input,
            producer: &producer,
            verifier: None,
        })
    }

    /// Saves a user-confirmed computation receipt as a new AI-memory concept.
    ///
    /// This path is intentionally unavailable to MCP applications. The desktop
    /// runtime invokes it only after a second native confirmation and fixes the
    /// process actor and machine verification itself.
    pub fn save_process_result(
        &self,
        wiki_id: Uuid,
        title: &str,
        receipt: &serde_json::Value,
    ) -> Result<OkfConceptProjectionRecord> {
        let wiki = self
            .database
            .collection(wiki_id)?
            .context("result wiki does not exist")?;
        if wiki.origin != WikiOrigin::AiMemory {
            bail!("computation results can only be saved to an AI memory wiki");
        }
        let owner_app_id = self
            .database
            .list_application_wiki_grants()?
            .into_iter()
            .find(|grant| {
                grant.collection_id == wiki_id && grant.role == ApplicationWikiRole::Owner
            })
            .map(|grant| grant.app_id)
            .context("AI memory owner is unavailable")?;
        let actor = ActorId::parse("process:airwiki-wasm")?;
        let body_markdown = format!(
            "```json\n{}\n```",
            serde_json::to_string_pretty(receipt)
                .context("could not serialize computation receipt")?
        );
        let input = AiMemoryConceptInput {
            title: title.to_owned(),
            description: "Result saved from an attested computation.".to_owned(),
            concept_type: ConceptType::parse("Computation Result")
                .context("computation result concept type is invalid")?,
            tags: vec!["attested-computation".to_owned()],
            body_markdown,
        };
        self.write_attributed(AttributedMemoryWrite {
            quota_owner_app_id: owner_app_id,
            wiki_id,
            concept_id: None,
            expected_fingerprint: None,
            input: &input,
            producer: &actor,
            verifier: Some(&actor),
        })
    }

    fn write_attributed(
        &self,
        write: AttributedMemoryWrite<'_>,
    ) -> Result<OkfConceptProjectionRecord> {
        let AttributedMemoryWrite {
            quota_owner_app_id,
            wiki_id,
            concept_id,
            expected_fingerprint,
            input,
            producer,
            verifier,
        } = write;
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
        let generated_at = Utc::now();
        let rendered = render_concept(producer, generated_at, input, "stable", verifier)?;
        if rendered.len() > AI_MEMORY_CONCEPT_MAX_BYTES {
            bail!("AI memory concept exceeds the size limit");
        }
        let path = checked_memory_path(&wiki.wiki_folder, &logical_path)?;
        let mut projected = self.database.list_okf_concept_projection(wiki_id)?;
        let provisional = imported_concept(
            &logical_path,
            input,
            "stable",
            &rendered,
            producer,
            generated_at,
            verifier,
        );
        let provisional_id = Uuid::new_v5(&wiki_id, logical_path.as_bytes());
        projected.retain(|concept| concept.concept_id != provisional_id);
        projected.push(projection_preview(wiki_id, provisional_id, &provisional)?);
        let next_index = render_index(&wiki.name, &projected);
        let managed_size =
            projected_bundle_size(&wiki.wiki_folder, &path, rendered.len(), next_index.len())?;
        self.database
            .ensure_application_managed_size(quota_owner_app_id, wiki_id, managed_size)?;
        let mutation = self.database.begin_managed_bundle_mutation(
            wiki_id,
            &logical_path,
            existing
                .as_ref()
                .map(|concept| concept.fingerprint.as_str()),
            &provisional.fingerprint,
        )?;
        if let Err(error) = atomic_write(&path, rendered.as_bytes()) {
            let _ = self.database.complete_managed_bundle_mutation(mutation.id);
            return Err(error);
        }
        let committed: Result<OkfConceptProjectionRecord> = (|| {
            self.database.set_managed_bundle_mutation_state(
                mutation.id,
                ManagedBundleMutationState::FileReplaced,
            )?;
            let stored = self
                .database
                .upsert_okf_concept_projection(wiki_id, &provisional)?;
            atomic_write(&wiki.wiki_folder.join("index.md"), next_index.as_bytes())?;
            self.database.update_collection_okf_metadata(
                wiki_id,
                Some("0.2"),
                &airwiki_types::OkfCompatibility::DeclaredV02,
                managed_size,
            )?;
            self.database
                .complete_managed_bundle_mutation(mutation.id)?;
            Ok(stored)
        })();
        let stored = match committed {
            Ok(stored) => stored,
            Err(error) => {
                let _ = self.database.set_managed_bundle_mutation_state(
                    mutation.id,
                    ManagedBundleMutationState::RecoveryRequired,
                );
                return Err(error).context("AI memory mutation requires recovery");
            }
        };
        Ok(stored)
    }

    pub fn deprecate(
        &self,
        app_id: Uuid,
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
        let producer = self.application_producer(app_id)?;
        let generated_at = Utc::now();
        let rendered = render_concept(&producer, generated_at, &input, "deprecated", None)?;
        let path = checked_memory_path(&wiki.wiki_folder, &concept.logical_path)?;
        let imported = imported_concept(
            &concept.logical_path,
            &input,
            "deprecated",
            &rendered,
            &producer,
            generated_at,
            None,
        );
        let mut projected = self.database.list_okf_concept_projection(wiki_id)?;
        projected.retain(|projected| projected.concept_id != concept.concept_id);
        projected.push(projection_preview(wiki_id, concept.concept_id, &imported)?);
        let next_index = render_index(&wiki.name, &projected);
        let managed_size =
            projected_bundle_size(&wiki.wiki_folder, &path, rendered.len(), next_index.len())?;
        self.database
            .ensure_application_managed_size(app_id, wiki_id, managed_size)?;
        let mutation = self.database.begin_managed_bundle_mutation(
            wiki_id,
            &concept.logical_path,
            Some(&concept.fingerprint),
            &imported.fingerprint,
        )?;
        if let Err(error) = atomic_write(&path, rendered.as_bytes()) {
            let _ = self.database.complete_managed_bundle_mutation(mutation.id);
            return Err(error);
        }
        let committed: Result<OkfConceptProjectionRecord> = (|| {
            self.database.set_managed_bundle_mutation_state(
                mutation.id,
                ManagedBundleMutationState::FileReplaced,
            )?;
            let stored = self
                .database
                .upsert_okf_concept_projection(wiki_id, &imported)?;
            atomic_write(&wiki.wiki_folder.join("index.md"), next_index.as_bytes())?;
            self.database.update_collection_okf_metadata(
                wiki_id,
                Some("0.2"),
                &airwiki_types::OkfCompatibility::DeclaredV02,
                managed_size,
            )?;
            self.database
                .complete_managed_bundle_mutation(mutation.id)?;
            Ok(stored)
        })();
        let stored = match committed {
            Ok(stored) => stored,
            Err(error) => {
                let _ = self.database.set_managed_bundle_mutation_state(
                    mutation.id,
                    ManagedBundleMutationState::RecoveryRequired,
                );
                return Err(error).context("AI memory mutation requires recovery");
            }
        };
        Ok(stored)
    }

    pub fn verify_managed_concept(
        &self,
        wiki_id: Uuid,
        logical_path: &str,
        expected_fingerprint: &str,
    ) -> Result<OkfConceptProjectionRecord> {
        let wiki = self
            .database
            .collection(wiki_id)?
            .context("managed wiki does not exist")?;
        if !matches!(wiki.origin, WikiOrigin::ImportedOkf | WikiOrigin::AiMemory) {
            bail!("folder wiki concepts are verified through the review workflow");
        }
        if !wiki.okf_compatibility.permits_external_disclosure() {
            bail!("this OKF compatibility level cannot be verified");
        }
        if !self
            .database
            .pending_managed_bundle_mutations_for_collection(wiki_id)?
            .is_empty()
        {
            bail!("managed OKF bundle requires recovery before verification");
        }
        let existing = self
            .database
            .list_okf_concept_projection(wiki_id)?
            .into_iter()
            .find(|concept| concept.logical_path == logical_path)
            .context("managed OKF concept does not exist")?;
        if existing.fingerprint != expected_fingerprint {
            bail!("managed OKF concept fingerprint is stale");
        }
        let path = checked_memory_path(&wiki.wiki_folder, logical_path)?;
        let original = std::fs::read_to_string(&path).context("could not read managed concept")?;
        if hex::encode(Sha256::digest(original.as_bytes())) != expected_fingerprint {
            bail!("managed OKF concept changed outside the current snapshot");
        }
        let verified_at = Utc::now();
        let actor = ActorId::parse("human:airwiki-user")?;
        let rendered = append_human_verification(&original, &actor, verified_at)?;
        if rendered.len() > MAX_CONCEPT_BYTES_FOR_MANAGED_VERIFICATION {
            bail!("verified OKF concept exceeds the size limit");
        }
        let managed_size = managed_bundle_size(&wiki.wiki_folder)?
            .checked_sub(u64::try_from(original.len())?)
            .and_then(|size| size.checked_add(rendered.len() as u64))
            .context("managed OKF size overflow")?;
        if managed_size > MAX_MANAGED_BUNDLE_BYTES {
            bail!("verified OKF bundle exceeds the size limit");
        }
        if wiki.origin == WikiOrigin::AiMemory {
            let owner_app_id = self
                .database
                .list_application_wiki_grants()?
                .into_iter()
                .find(|grant| {
                    grant.collection_id == wiki_id && grant.role == ApplicationWikiRole::Owner
                })
                .map(|grant| grant.app_id)
                .context("AI memory owner is unavailable")?;
            self.database
                .ensure_application_managed_size(owner_app_id, wiki_id, managed_size)?;
        }
        let replacement_fingerprint = hex::encode(Sha256::digest(rendered.as_bytes()));
        let mutation = self.database.begin_managed_bundle_mutation(
            wiki_id,
            logical_path,
            Some(expected_fingerprint),
            &replacement_fingerprint,
        )?;
        if let Err(error) = atomic_write(&path, rendered.as_bytes()) {
            let _ = self.database.complete_managed_bundle_mutation(mutation.id);
            return Err(error);
        }
        let committed = (|| {
            self.database.set_managed_bundle_mutation_state(
                mutation.id,
                ManagedBundleMutationState::FileReplaced,
            )?;
            let imported = OkfImportValidator::validate_directory(&wiki.wiki_folder)?;
            let verified = imported
                .concepts
                .iter()
                .find(|concept| concept.logical_path == logical_path)
                .context("verified OKF concept disappeared during reprojection")?;
            let stored = self
                .database
                .upsert_okf_concept_projection(wiki_id, verified)?;
            self.database.update_collection_okf_metadata(
                wiki_id,
                imported.declared_okf_version.as_deref(),
                &imported.compatibility,
                managed_size,
            )?;
            self.database
                .complete_managed_bundle_mutation(mutation.id)?;
            Ok::<_, anyhow::Error>(stored)
        })();
        match committed {
            Ok(stored) => Ok(stored),
            Err(error) => {
                let _ = self.database.set_managed_bundle_mutation_state(
                    mutation.id,
                    ManagedBundleMutationState::RecoveryRequired,
                );
                Err(error).context("managed verification requires recovery")
            }
        }
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

    fn application_producer(&self, app_id: Uuid) -> Result<ActorId> {
        let capability = self
            .database
            .application_capability_by_app_id(app_id)?
            .context("application capability is unavailable")?;
        capability
            .producer
            .parse()
            .context("application producer is invalid")
    }
}

const MAX_CONCEPT_BYTES_FOR_MANAGED_VERIFICATION: usize = 1024 * 1024;
const MAX_MANAGED_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;

fn cleanup_managed_staging(vaults: &Path, wiki_id: Uuid) -> Result<()> {
    for prefix in [".import-", ".memory-"] {
        let path = vaults.join(format!("{prefix}{wiki_id}"));
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not inspect managed staging {prefix}"));
            }
        };
        if metadata.file_type().is_symlink() || metadata.is_file() {
            std::fs::remove_file(&path)
                .with_context(|| format!("could not remove managed staging {prefix}"))?;
        } else if metadata.is_dir() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("could not remove managed staging {prefix}"))?;
        } else {
            bail!("managed staging has an unsupported file type");
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct MemoryFrontmatter<'a> {
    r#type: String,
    title: &'a str,
    description: &'a str,
    tags: &'a [String],
    status: &'a str,
    generated: Generated<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verified: Option<Verified<'a>>,
}

#[derive(Serialize)]
struct Generated<'a> {
    by: &'a str,
    at: String,
}

#[derive(Serialize)]
struct Verified<'a> {
    by: &'a str,
    at: String,
}

fn render_concept(
    producer: &ActorId,
    generated_at: chrono::DateTime<Utc>,
    input: &AiMemoryConceptInput,
    status: &str,
    verifier: Option<&ActorId>,
) -> Result<String> {
    let frontmatter = MemoryFrontmatter {
        r#type: input.concept_type.to_string(),
        title: input.title.trim(),
        description: input.description.trim(),
        tags: &input.tags,
        status,
        generated: Generated {
            by: producer.as_str(),
            at: generated_at.to_rfc3339(),
        },
        verified: verifier.map(|actor| Verified {
            by: actor.as_str(),
            at: generated_at.to_rfc3339(),
        }),
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
    producer: &ActorId,
    generated_at: chrono::DateTime<Utc>,
    verifier: Option<&ActorId>,
) -> OkfImportedConcept {
    OkfImportedConcept {
        logical_path: path.to_owned(),
        concept_type: input.concept_type.to_string(),
        title: input.title.trim().to_owned(),
        description: input.description.trim().to_owned(),
        tags: input.tags.clone(),
        lifecycle_status: status.to_owned(),
        generated: serde_yaml::to_value(Generated {
            by: producer.as_str(),
            at: generated_at.to_rfc3339(),
        })
        .ok(),
        verified: verifier.and_then(|actor| {
            serde_yaml::to_value(Verified {
                by: actor.as_str(),
                at: generated_at.to_rfc3339(),
            })
            .ok()
        }),
        sources: None,
        stale_after: None,
        version: None,
        unknown_frontmatter: serde_yaml::Value::Mapping(Default::default()),
        attested_computation: None,
        fingerprint: hex::encode(Sha256::digest(rendered.as_bytes())),
        search_text: input.body_markdown.clone(),
        assurance: ConceptAssurance {
            trust: verifier.map_or(TrustTier::Unverified, |_| TrustTier::MachineConfirmed),
            freshness: FreshnessState::NotDeclared,
            verification_outdated: false,
        },
        warnings: Vec::new(),
    }
}

fn projection_preview(
    collection_id: Uuid,
    concept_id: Uuid,
    concept: &OkfImportedConcept,
) -> Result<OkfConceptProjectionRecord> {
    Ok(OkfConceptProjectionRecord {
        collection_id,
        concept_id,
        logical_path: concept.logical_path.clone(),
        concept_type: ConceptType::parse(concept.concept_type.clone())
            .context("AI memory concept type is invalid")?,
        title: concept.title.clone(),
        description: concept.description.clone(),
        tags: concept.tags.clone(),
        lifecycle_status: concept.lifecycle_status.clone(),
        generation: concept.generated.as_ref().and_then(yaml_to_json),
        verifications: concept
            .verified
            .as_ref()
            .and_then(yaml_to_json)
            .unwrap_or_else(|| serde_json::json!([])),
        provenance: concept
            .sources
            .as_ref()
            .and_then(yaml_to_json)
            .unwrap_or_else(|| serde_json::json!([])),
        stale_after: concept.stale_after.clone(),
        version: concept.version.clone(),
        fingerprint: concept.fingerprint.clone(),
        unknown_frontmatter: yaml_to_json(&concept.unknown_frontmatter)
            .unwrap_or_else(|| serde_json::json!({})),
        attested_computation: concept.attested_computation.clone(),
        assurance: concept.assurance,
        warnings: concept.warnings.clone(),
        indexed_at: Utc::now(),
    })
}

fn yaml_to_json(value: &serde_yaml::Value) -> Option<serde_json::Value> {
    serde_json::to_value(value).ok()
}

fn render_index(name: &str, concepts: &[OkfConceptProjectionRecord]) -> String {
    let mut index = format!(
        "---\nokf_version: \"0.2\"\n---\n\n# {}\n",
        safe_heading(name)
    );
    for concept in concepts {
        index.push_str(&format!(
            "\n- [{}]({})\n",
            safe_heading(&concept.title),
            concept.logical_path
        ));
    }
    index
}

fn regenerate_index(
    root: &Path,
    name: &str,
    concepts: &[OkfConceptProjectionRecord],
) -> Result<()> {
    atomic_write(
        &root.join("index.md"),
        render_index(name, concepts).as_bytes(),
    )
}

fn managed_bundle_size(root: &Path) -> Result<u64> {
    walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .try_fold(0_u64, |total, entry| {
            let entry = entry?;
            if entry.file_type().is_file() {
                total
                    .checked_add(entry.metadata()?.len())
                    .context("AI memory size overflow")
            } else {
                Ok(total)
            }
        })
}

fn projected_bundle_size(
    root: &Path,
    concept_path: &Path,
    replacement_concept_bytes: usize,
    replacement_index_bytes: usize,
) -> Result<u64> {
    let current = managed_bundle_size(root)?;
    let previous_concept = std::fs::metadata(concept_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let previous_index = std::fs::metadata(root.join("index.md"))
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    current
        .checked_sub(previous_concept)
        .and_then(|size| size.checked_sub(previous_index))
        .and_then(|size| size.checked_add(replacement_concept_bytes as u64))
        .and_then(|size| size.checked_add(replacement_index_bytes as u64))
        .context("AI memory size overflow")
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
                "codex/test",
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
        let wiki = service.create(app_id, "Project memory").unwrap();
        let input = AiMemoryConceptInput {
            title: "Decision".to_owned(),
            description: "Architecture decision".to_owned(),
            concept_type: ConceptType::Other("Decision".to_owned()),
            tags: vec!["architecture".to_owned()],
            body_markdown: "Use a local-first design.".to_owned(),
        };

        let created = service.write(app_id, wiki.id, None, None, &input).unwrap();
        let deprecated = service
            .deprecate(app_id, wiki.id, created.concept_id, &created.fingerprint)
            .unwrap();

        assert_eq!(deprecated.lifecycle_status, "deprecated");
        assert_eq!(service.list(app_id).unwrap().len(), 1);
    }

    #[test]
    fn confirmed_computation_result_is_machine_confirmed_by_the_process() {
        let (_temp, _database, service, app_id) = service();
        let wiki = service.create(app_id, "Computed memory").unwrap();

        let stored = service
            .save_process_result(wiki.id, "Revenue result", &serde_json::json!({"value": 42}))
            .unwrap();
        let markdown =
            std::fs::read_to_string(wiki.wiki_folder.join(&stored.logical_path)).unwrap();

        assert_eq!(stored.assurance.trust, TrustTier::MachineConfirmed);
        assert_eq!(
            stored
                .generation
                .as_ref()
                .and_then(|generation| generation.get("by"))
                .and_then(serde_json::Value::as_str),
            Some("process:airwiki-wasm")
        );
        assert!(markdown.contains("by: process:airwiki-wasm"));
        assert!(markdown.contains("\"value\": 42"));
        assert!(!markdown.contains("human:"));
    }

    #[test]
    fn user_can_verify_a_current_managed_concept_without_granting_mcp_authority() {
        let (_temp, _database, service, app_id) = service();
        let wiki = service.create(app_id, "Verified memory").unwrap();
        let input = AiMemoryConceptInput {
            title: "Decision".to_owned(),
            description: "Reviewed decision".to_owned(),
            concept_type: ConceptType::Other("Decision".to_owned()),
            tags: Vec::new(),
            body_markdown: "Keep the data local.".to_owned(),
        };
        let created = service.write(app_id, wiki.id, None, None, &input).unwrap();

        let verified = service
            .verify_managed_concept(wiki.id, &created.logical_path, &created.fingerprint)
            .unwrap();
        let markdown =
            std::fs::read_to_string(wiki.wiki_folder.join(&verified.logical_path)).unwrap();

        assert_eq!(verified.assurance.trust, TrustTier::HumanReviewed);
        assert!(!verified.assurance.verification_outdated);
        assert!(markdown.contains("by: human:airwiki-user"));
        assert!(
            service
                .verify_managed_concept(wiki.id, &created.logical_path, &created.fingerprint)
                .is_err()
        );
    }

    #[test]
    fn computation_result_cannot_be_saved_to_a_folder_wiki() {
        let (temp, database, service, _app_id) = service();
        let source = temp.path().join("source");
        let bundle = temp.path().join("bundle");
        std::fs::create_dir_all(&source).unwrap();
        let wiki = database
            .create_collection(
                "Folder wiki",
                &source,
                &bundle,
                airwiki_types::CollectionPolicy::local_only(),
            )
            .unwrap();

        assert!(
            service
                .save_process_result(wiki.id, "Blocked result", &serde_json::json!({"value": 42}))
                .is_err()
        );
    }

    #[test]
    fn stale_fingerprint_and_cross_application_access_fail_closed() {
        let (_temp, database, service, app_id) = service();
        let wiki = service.create(app_id, "Private memory").unwrap();
        let input = AiMemoryConceptInput {
            title: "Fact".to_owned(),
            description: String::new(),
            concept_type: ConceptType::Reference,
            tags: Vec::new(),
            body_markdown: "Private fact".to_owned(),
        };
        let created = service.write(app_id, wiki.id, None, None, &input).unwrap();
        assert!(
            service
                .write(
                    app_id,
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
                "claude/test",
                "fedcba9876543210",
                &"b".repeat(64),
            )
            .unwrap();
        assert!(service.get(other, wiki.id).is_err());
    }

    #[test]
    fn reader_editor_and_revocation_are_enforced_immediately() {
        let (_temp, database, service, owner) = service();
        let wiki = service.create(owner, "Shared memory").unwrap();
        let collaborator = Uuid::new_v4();
        database
            .create_application_capability(
                collaborator,
                "Claude",
                "claude",
                "claude/test",
                "fedcba9876543210",
                &"b".repeat(64),
            )
            .unwrap();
        let input = AiMemoryConceptInput {
            title: "Shared decision".to_owned(),
            description: String::new(),
            concept_type: ConceptType::Other("Decision".to_owned()),
            tags: Vec::new(),
            body_markdown: "Keep access explicit.".to_owned(),
        };

        database
            .set_application_wiki_role(collaborator, wiki.id, Some(ApplicationWikiRole::Reader))
            .unwrap();
        assert!(service.get(collaborator, wiki.id).is_ok());
        assert!(
            service
                .write(collaborator, wiki.id, None, None, &input)
                .is_err()
        );

        database
            .set_application_wiki_role(collaborator, wiki.id, Some(ApplicationWikiRole::Editor))
            .unwrap();
        assert!(
            service
                .write(collaborator, wiki.id, None, None, &input)
                .is_ok()
        );

        database
            .set_application_capability_revoked(collaborator, true)
            .unwrap();
        assert!(service.get(collaborator, wiki.id).is_err());
    }

    #[test]
    fn recovery_removes_prepared_wiki_and_its_staging_directory() {
        let (temp, database, service, app_id) = service();
        let wiki_id = Uuid::new_v4();
        let staging = temp.path().join(format!(".memory-{wiki_id}"));
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("index.md"), "incomplete").unwrap();
        let bundle = temp.path().join(wiki_id.to_string());
        database
            .create_managed_collection_with_mutation(NewManagedCollection {
                id: wiki_id,
                name: "Interrupted memory".to_owned(),
                bundle_root: bundle,
                origin: WikiOrigin::AiMemory,
                replacement_fingerprint: "f".repeat(64),
                owner_app_id: Some(app_id),
            })
            .unwrap();

        let report = service.recover_pending().unwrap();

        assert_eq!(report.recovered, 1);
        assert_eq!(report.pending, 0);
        assert!(!staging.exists());
        assert!(database.collection(wiki_id).unwrap().is_none());
        assert!(
            database
                .pending_managed_bundle_mutations()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn producer_cannot_change_when_a_capability_rotates() {
        let (_temp, database, _service, app_id) = service();

        database
            .rotate_application_capability(
                app_id,
                "codex/test",
                "1111111111111111",
                &"c".repeat(64),
            )
            .unwrap();
        assert!(
            database
                .rotate_application_capability(
                    app_id,
                    "codex/other",
                    "2222222222222222",
                    &"d".repeat(64),
                )
                .is_err()
        );
        assert_eq!(
            database
                .application_capability_by_app_id(app_id)
                .unwrap()
                .unwrap()
                .producer,
            "codex/test"
        );
    }
}
