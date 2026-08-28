use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[cfg(test)]
use airwiki_types::TrustTier;
use airwiki_types::{ActorId, ConceptType};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::okf::atomic_write;
use crate::okf_import::{append_human_verification, parse_concept};
use crate::storage::ApplicationWikiRole;
use crate::{
    CollectionRecord, Database, InitialApplicationAccess, ManagedBundleMutationState, MemoryScope,
    NewManagedCollection, OkfConceptProjectionRecord, OkfImportValidator, OkfImportedConcept,
    ProjectMemoryAttachmentState, WikiOrigin,
};

pub const AI_MEMORY_CONCEPT_MAX_BYTES: usize = 48 * 1024;
const AI_MEMORY_WIKI_NAME_MAX_CHARS: usize = 120;
const AI_MEMORY_CONCEPT_TYPE_MAX_CHARS: usize = 120;
const PROCESS_RESULT_CONCEPT_MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AiMemoryError {
    #[error("AI memory concept fingerprint is stale")]
    FingerprintConflict,
}

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

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
    authorized_app_id: Option<Uuid>,
    quota_owner_app_id: Option<Uuid>,
    wiki_id: Uuid,
    concept_id: Option<Uuid>,
    expected_fingerprint: Option<&'a str>,
    new_logical_path: Option<String>,
    input: &'a AiMemoryConceptInput,
    producer: &'a ActorId,
    verifier: Option<&'a ActorId>,
    rendered_size_limit: usize,
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
        let _guard = self.database.managed_bundle_guard()?;
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
            let recovered = if mutation.logical_path == "." {
                self.recover_managed_bundle_creation(&wiki, &mutation)
            } else {
                self.recover_managed_concept_mutation(&wiki, &mutation)
            };
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

    fn recover_managed_bundle_creation(
        &self,
        wiki: &CollectionRecord,
        mutation: &crate::ManagedBundleMutationRecord,
    ) -> Result<()> {
        let imported = OkfImportValidator::validate_directory(&wiki.wiki_folder)?;
        if imported.bundle_fingerprint != mutation.replacement_fingerprint {
            bail!("managed OKF bundle does not match its recovery journal");
        }
        self.database
            .replace_okf_concept_projection(wiki.id, &imported.concepts)?;
        self.database.update_collection_okf_metadata(
            wiki.id,
            imported.declared_okf_version.as_deref(),
            &imported.compatibility,
            imported.uncompressed_bytes,
        )?;
        self.database.complete_managed_bundle_mutation(mutation.id)
    }

    fn recover_managed_concept_mutation(
        &self,
        wiki: &CollectionRecord,
        mutation: &crate::ManagedBundleMutationRecord,
    ) -> Result<()> {
        let current_projection = self
            .database
            .list_okf_concept_projection(wiki.id)?
            .into_iter()
            .find(|concept| concept.logical_path == mutation.logical_path);
        let projected_fingerprint = current_projection
            .as_ref()
            .map(|concept| concept.fingerprint.as_str());
        let path = checked_memory_path(&wiki.wiki_folder, &mutation.logical_path)?;
        let filesystem_fingerprint = managed_file_fingerprint(&path)?;
        let previous = mutation.previous_fingerprint.as_deref();

        if mutation.state != ManagedBundleMutationState::FileReplaced
            && filesystem_fingerprint.as_deref() == previous
            && projected_fingerprint == previous
        {
            return self.database.complete_managed_bundle_mutation(mutation.id);
        }
        if filesystem_fingerprint.as_deref() != Some(&mutation.replacement_fingerprint)
            || !matches!(
                projected_fingerprint,
                value if value == previous || value == Some(&mutation.replacement_fingerprint)
            )
        {
            bail!("managed OKF concept does not match its recovery journal");
        }

        let imported = OkfImportValidator::validate_directory(&wiki.wiki_folder)?;
        let replacement = imported
            .concepts
            .iter()
            .find(|concept| concept.logical_path == mutation.logical_path)
            .filter(|concept| concept.fingerprint == mutation.replacement_fingerprint)
            .context("managed OKF replacement does not match its recovery journal")?;
        self.database
            .upsert_okf_concept_projection(wiki.id, replacement)?;
        if wiki.memory_scope == Some(MemoryScope::Personal) {
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
    }

    pub fn create(&self, app_id: Uuid, name: &str) -> Result<CollectionRecord> {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > AI_MEMORY_WIKI_NAME_MAX_CHARS
            || has_unsupported_text_control(name)
        {
            bail!("AI memory wiki name is invalid");
        }
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
        let replacement_fingerprint = match OkfImportValidator::validate_directory(&staging) {
            Ok(report) => report.bundle_fingerprint,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(error).context("could not validate AI memory staging");
            }
        };
        let bundle = self.vaults.join(wiki_id.to_string());
        let (record, mutation) =
            match self
                .database
                .create_managed_collection_with_mutation(NewManagedCollection {
                    id: wiki_id,
                    name: name.to_owned(),
                    bundle_root: bundle.clone(),
                    policy: airwiki_types::CollectionPolicy::connected_to_ai_apps(),
                    origin: WikiOrigin::AiMemory,
                    replacement_fingerprint,
                    owner_app_id: Some(app_id),
                    initial_application_access: InitialApplicationAccess::None,
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

    /// Reads one application-visible concept and verifies that its managed file
    /// still matches the projected fingerprint before returning the Markdown body.
    pub fn get_concept(
        &self,
        app_id: Uuid,
        wiki_id: Uuid,
        concept_id: Uuid,
    ) -> Result<(OkfConceptProjectionRecord, String)> {
        self.require_role(app_id, wiki_id, false)?;
        let concept = self
            .database
            .list_okf_concept_projection(wiki_id)?
            .into_iter()
            .find(|concept| concept.concept_id == concept_id)
            .context("AI memory concept does not exist")?;
        let wiki = self
            .database
            .collection(wiki_id)?
            .context("AI memory wiki does not exist")?;
        let path = checked_memory_path(&wiki.wiki_folder, &concept.logical_path)?;
        let current = read_current_managed_concept(&path, &concept.fingerprint)?;
        let body_markdown = read_body(&current)?;
        Ok((concept, body_markdown))
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
        let wiki = self
            .database
            .collection(wiki_id)?
            .context("AI memory wiki does not exist")?;
        let quota_owner_app_id = if wiki.memory_scope == Some(MemoryScope::Personal) {
            Some(self.owner_app_id(wiki_id)?)
        } else {
            None
        };
        self.write_attributed(AttributedMemoryWrite {
            authorized_app_id: Some(app_id),
            quota_owner_app_id,
            wiki_id,
            concept_id,
            expected_fingerprint,
            new_logical_path: None,
            input,
            producer: &producer,
            verifier: None,
            rendered_size_limit: AI_MEMORY_CONCEPT_MAX_BYTES,
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
        run_id: Uuid,
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
        let quota_owner_app_id = match wiki.memory_scope {
            Some(MemoryScope::Personal) => Some(self.owner_app_id(wiki_id)?),
            Some(MemoryScope::Project) => None,
            None => bail!("result wiki has no memory scope"),
        };
        let logical_path = format!("results/{run_id}.md");
        if let Some(existing) = self
            .database
            .list_okf_concept_projection(wiki_id)?
            .into_iter()
            .find(|concept| concept.logical_path == logical_path)
        {
            return Ok(existing);
        }
        let actor = ActorId::parse("process:airwiki-wasm")?;
        let body_markdown = format!(
            "```json\n{}\n```",
            serde_json::to_string(receipt).context("could not serialize computation receipt")?
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
            authorized_app_id: None,
            quota_owner_app_id,
            wiki_id,
            concept_id: None,
            expected_fingerprint: None,
            new_logical_path: Some(logical_path),
            input: &input,
            producer: &actor,
            verifier: Some(&actor),
            rendered_size_limit: PROCESS_RESULT_CONCEPT_MAX_BYTES,
        })
    }

    fn write_attributed(
        &self,
        write: AttributedMemoryWrite<'_>,
    ) -> Result<OkfConceptProjectionRecord> {
        let _guard = self.database.managed_bundle_guard()?;
        let AttributedMemoryWrite {
            authorized_app_id,
            quota_owner_app_id,
            wiki_id,
            concept_id,
            expected_fingerprint,
            new_logical_path,
            input,
            producer,
            verifier,
            rendered_size_limit,
        } = write;
        validate_input(input)?;
        if let Some(app_id) = authorized_app_id {
            self.require_role(app_id, wiki_id, true)?;
        }
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
            (Some(_), _) => return Err(AiMemoryError::FingerprintConflict.into()),
            (None, Some(_)) => bail!("new AI memory concepts cannot have an expected fingerprint"),
            (None, None) => {}
        }
        let logical_path = existing.as_ref().map_or_else(
            || new_logical_path.unwrap_or_else(|| format!("concepts/{}.md", Uuid::new_v4())),
            |concept| concept.logical_path.clone(),
        );
        let wiki = self
            .database
            .collection(wiki_id)?
            .context("AI memory wiki does not exist")?;
        let project_memory = wiki.memory_scope == Some(MemoryScope::Project);
        if project_memory
            && self
                .database
                .project_memory_attachment(wiki_id)?
                .is_none_or(|attachment| attachment.state != ProjectMemoryAttachmentState::Active)
        {
            bail!("project memory attachment is unavailable");
        }
        let path = checked_memory_path(&wiki.wiki_folder, &logical_path)?;
        let current = existing
            .as_ref()
            .map(|concept| read_current_managed_concept(&path, &concept.fingerprint))
            .transpose()?;
        let generated_at = Utc::now();
        let verified = verification_metadata(existing.as_ref(), verifier, generated_at)?;
        let rendered = render_concept(
            producer,
            generated_at,
            input,
            "stable",
            verified.as_ref(),
            current.as_deref(),
        )?;
        if rendered.len() > rendered_size_limit {
            bail!("AI memory concept exceeds the size limit");
        }
        ensure_managed_write_target_is_current(
            &path,
            existing
                .as_ref()
                .map(|concept| concept.fingerprint.as_str()),
        )?;
        let mut projected = self.database.list_okf_concept_projection(wiki_id)?;
        let provisional = parse_concept(&logical_path, &rendered, generated_at)?;
        let provisional_id = Uuid::new_v5(&wiki_id, logical_path.as_bytes());
        projected.retain(|concept| concept.concept_id != provisional_id);
        projected.push(projection_preview(wiki_id, provisional_id, &provisional)?);
        let next_index = (!project_memory).then(|| render_index(&wiki.name, &projected));
        let managed_size = projected_bundle_size(
            &wiki.wiki_folder,
            &path,
            rendered.len(),
            next_index.as_ref().map_or(0, String::len),
            next_index.is_some(),
        )?;
        if managed_size > MAX_MANAGED_BUNDLE_BYTES {
            bail!("AI memory bundle exceeds the size limit");
        }
        if let Some(quota_owner_app_id) = quota_owner_app_id {
            self.database.ensure_application_managed_size(
                quota_owner_app_id,
                wiki_id,
                managed_size,
            )?;
        }
        let mutation = self.database.begin_managed_bundle_mutation(
            wiki_id,
            &logical_path,
            existing
                .as_ref()
                .map(|concept| concept.fingerprint.as_str()),
            &provisional.fingerprint,
        )?;
        if let Err(error) = atomic_write(&path, rendered.as_bytes()) {
            let _ = self.database.set_managed_bundle_mutation_state(
                mutation.id,
                ManagedBundleMutationState::RecoveryRequired,
            );
            return Err(error).context("AI memory mutation requires recovery");
        }
        let committed: Result<OkfConceptProjectionRecord> = (|| {
            self.database.set_managed_bundle_mutation_state(
                mutation.id,
                ManagedBundleMutationState::FileReplaced,
            )?;
            let stored = self
                .database
                .upsert_okf_concept_projection(wiki_id, &provisional)?;
            if let Some(next_index) = &next_index {
                atomic_write(&wiki.wiki_folder.join("index.md"), next_index.as_bytes())?;
            }
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
        let _guard = self.database.managed_bundle_guard()?;
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
            return Err(AiMemoryError::FingerprintConflict.into());
        }
        let wiki = self
            .database
            .collection(wiki_id)?
            .context("AI memory wiki does not exist")?;
        let path = checked_memory_path(&wiki.wiki_folder, &concept.logical_path)?;
        let original = read_current_managed_concept(&path, &concept.fingerprint)?;
        let input = AiMemoryConceptInput {
            title: concept.title.clone(),
            description: concept.description.clone(),
            concept_type: concept.concept_type.clone(),
            tags: concept.tags.clone(),
            body_markdown: read_body(&original)?,
        };
        let producer = self.application_producer(app_id)?;
        let generated_at = Utc::now();
        let verified = verification_metadata(Some(&concept), None, generated_at)?;
        let rendered = render_concept(
            &producer,
            generated_at,
            &input,
            "deprecated",
            verified.as_ref(),
            Some(&original),
        )?;
        let imported = parse_concept(&concept.logical_path, &rendered, generated_at)?;
        let mut projected = self.database.list_okf_concept_projection(wiki_id)?;
        projected.retain(|projected| projected.concept_id != concept.concept_id);
        projected.push(projection_preview(wiki_id, concept.concept_id, &imported)?);
        let project_memory = wiki.memory_scope == Some(MemoryScope::Project);
        let next_index = (!project_memory).then(|| render_index(&wiki.name, &projected));
        let managed_size = projected_bundle_size(
            &wiki.wiki_folder,
            &path,
            rendered.len(),
            next_index.as_ref().map_or(0, String::len),
            next_index.is_some(),
        )?;
        if managed_size > MAX_MANAGED_BUNDLE_BYTES {
            bail!("AI memory bundle exceeds the size limit");
        }
        if !project_memory {
            let owner_app_id = self.owner_app_id(wiki_id)?;
            self.database
                .ensure_application_managed_size(owner_app_id, wiki_id, managed_size)?;
        }
        let mutation = self.database.begin_managed_bundle_mutation(
            wiki_id,
            &concept.logical_path,
            Some(&concept.fingerprint),
            &imported.fingerprint,
        )?;
        if let Err(error) = atomic_write(&path, rendered.as_bytes()) {
            let _ = self.database.set_managed_bundle_mutation_state(
                mutation.id,
                ManagedBundleMutationState::RecoveryRequired,
            );
            return Err(error).context("AI memory deprecation requires recovery");
        }
        let committed: Result<OkfConceptProjectionRecord> = (|| {
            self.database.set_managed_bundle_mutation_state(
                mutation.id,
                ManagedBundleMutationState::FileReplaced,
            )?;
            let stored = self
                .database
                .upsert_okf_concept_projection(wiki_id, &imported)?;
            if let Some(next_index) = &next_index {
                atomic_write(&wiki.wiki_folder.join("index.md"), next_index.as_bytes())?;
            }
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
        let _guard = self.database.managed_bundle_guard()?;
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
        if wiki.memory_scope == Some(MemoryScope::Personal) {
            let owner_app_id = self.owner_app_id(wiki_id)?;
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
            let _ = self.database.set_managed_bundle_mutation_state(
                mutation.id,
                ManagedBundleMutationState::RecoveryRequired,
            );
            return Err(error).context("managed verification requires recovery");
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

    fn owner_app_id(&self, wiki_id: Uuid) -> Result<Uuid> {
        self.database
            .list_application_wiki_grants()?
            .into_iter()
            .find(|grant| {
                grant.collection_id == wiki_id && grant.role == ApplicationWikiRole::Owner
            })
            .map(|grant| grant.app_id)
            .context("AI memory owner is unavailable")
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
    verified: Option<&'a serde_yaml::Value>,
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
    verified: Option<&serde_yaml::Value>,
    existing: Option<&str>,
) -> Result<String> {
    let mut frontmatter = if let Some(existing) = existing {
        parse_frontmatter_mapping(existing)?
    } else {
        serde_yaml::Mapping::new()
    };
    let managed = MemoryFrontmatter {
        r#type: input.concept_type.to_string(),
        title: input.title.trim(),
        description: input.description.trim(),
        tags: &input.tags,
        status,
        generated: Generated {
            by: producer.as_str(),
            at: generated_at.to_rfc3339(),
        },
        verified,
    };
    let managed = serde_yaml::to_value(managed)
        .context("could not serialize AI memory frontmatter")?
        .as_mapping()
        .cloned()
        .context("AI memory frontmatter is not a mapping")?;
    for (key, value) in managed {
        frontmatter.insert(key, value);
    }
    Ok(format!(
        "---\n{}---\n\n{}\n",
        serde_yaml::to_string(&frontmatter)?,
        input.body_markdown.trim()
    ))
}

fn parse_frontmatter_mapping(markdown: &str) -> Result<serde_yaml::Mapping> {
    let rest = markdown
        .strip_prefix("---\n")
        .context("AI memory frontmatter is missing")?;
    let end = rest
        .find("\n---\n")
        .context("AI memory frontmatter is incomplete")?;
    serde_yaml::from_str::<serde_yaml::Value>(&rest[..end])?
        .as_mapping()
        .cloned()
        .context("AI memory frontmatter is not a mapping")
}

fn verification_metadata(
    existing: Option<&OkfConceptProjectionRecord>,
    verifier: Option<&ActorId>,
    generated_at: chrono::DateTime<Utc>,
) -> Result<Option<serde_yaml::Value>> {
    if let Some(actor) = verifier {
        return serde_yaml::to_value(Verified {
            by: actor.as_str(),
            at: generated_at.to_rfc3339(),
        })
        .map(Some)
        .context("could not serialize managed verification metadata");
    }
    let Some(verifications) = existing.map(|concept| &concept.verifications) else {
        return Ok(None);
    };
    if verifications.is_null()
        || verifications
            .as_array()
            .is_some_and(std::vec::Vec::is_empty)
    {
        return Ok(None);
    }
    serde_yaml::to_value(verifications)
        .map(Some)
        .context("could not preserve managed verification metadata")
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
    replace_index: bool,
) -> Result<u64> {
    let current = managed_bundle_size(root)?;
    let previous_concept = std::fs::metadata(concept_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let previous_index = if replace_index {
        std::fs::metadata(root.join("index.md"))
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    } else {
        0
    };
    current
        .checked_sub(previous_concept)
        .and_then(|size| size.checked_sub(previous_index))
        .and_then(|size| size.checked_add(replacement_concept_bytes as u64))
        .and_then(|size| size.checked_add(replacement_index_bytes as u64))
        .context("AI memory size overflow")
}

fn validate_input(input: &AiMemoryConceptInput) -> Result<()> {
    let concept_type = input.concept_type.to_string();
    if input.title.trim().is_empty()
        || input.title.chars().count() > 200
        || input.description.chars().count() > 2_000
        || concept_type.chars().count() > AI_MEMORY_CONCEPT_TYPE_MAX_CHARS
        || input.tags.len() > 20
        || [
            input.title.as_str(),
            input.description.as_str(),
            concept_type.as_str(),
            input.body_markdown.as_str(),
        ]
        .into_iter()
        .chain(input.tags.iter().map(String::as_str))
        .any(has_unsupported_text_control)
    {
        bail!("AI memory concept metadata is invalid");
    }
    Ok(())
}

fn has_unsupported_text_control(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
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
    let root_metadata =
        std::fs::symlink_metadata(root).context("AI memory bundle root is unavailable")?;
    if !root_metadata.is_dir() || metadata_is_link_or_reparse(&root_metadata) {
        bail!("AI memory bundle root is not a safe directory");
    }
    let parts = logical_path.split('/').collect::<Vec<_>>();
    let mut path = root.to_path_buf();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata_is_link_or_reparse(&metadata)
                    || index + 1 < parts.len() && !metadata.is_dir()
                    || index + 1 == parts.len() && !metadata.is_file()
                {
                    bail!("AI memory path contains an unsafe filesystem entry");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("AI memory path could not be inspected"),
        }
    }
    Ok(path)
}

fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

fn ensure_managed_write_target_is_current(path: &Path, expected: Option<&str>) -> Result<()> {
    match (std::fs::read(path), expected) {
        (Ok(bytes), Some(expected)) if hex::encode(Sha256::digest(&bytes)) == expected => Ok(()),
        (Ok(_), Some(_) | None) => Err(AiMemoryError::FingerprintConflict.into()),
        (Err(error), Some(_)) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(AiMemoryError::FingerprintConflict.into())
        }
        (Err(error), None) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        (Err(error), _) => Err(error).context("AI memory concept could not be inspected"),
    }
}

fn read_current_managed_concept(path: &Path, expected_fingerprint: &str) -> Result<String> {
    let bytes = std::fs::read(path).context("could not read AI memory concept")?;
    if hex::encode(Sha256::digest(&bytes)) != expected_fingerprint {
        return Err(AiMemoryError::FingerprintConflict.into());
    }
    String::from_utf8(bytes).context("AI memory concept must be UTF-8")
}

fn managed_file_fingerprint(path: &Path) -> Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(hex::encode(Sha256::digest(bytes)))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("managed OKF concept could not be inspected"),
    }
}

fn read_body(markdown: &str) -> Result<String> {
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
    fn application_created_memory_grants_only_its_owner() {
        let (_temp, database, service, owner) = service();
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

        let wiki = service.create(owner, "Owner-only memory").unwrap();
        let grants = database.list_application_wiki_grants().unwrap();

        assert!(grants.iter().any(|grant| {
            grant.app_id == owner
                && grant.collection_id == wiki.id
                && grant.role == ApplicationWikiRole::Owner
        }));
        assert!(
            !grants
                .iter()
                .any(|grant| { grant.app_id == other && grant.collection_id == wiki.id })
        );
        assert!(
            database
                .application_search_collection_ids(other)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn owner_can_read_a_current_memory_concept_body() {
        let (_temp, _database, service, app_id) = service();
        let wiki = service.create(app_id, "Readable memory").unwrap();
        let created = service
            .write(
                app_id,
                wiki.id,
                None,
                None,
                &AiMemoryConceptInput {
                    title: "Decision".to_owned(),
                    description: String::new(),
                    concept_type: ConceptType::Other("Decision".to_owned()),
                    tags: Vec::new(),
                    body_markdown: "Keep durable context readable.".to_owned(),
                },
            )
            .unwrap();

        let (read, body_markdown) = service
            .get_concept(app_id, wiki.id, created.concept_id)
            .unwrap();

        assert_eq!(read.fingerprint, created.fingerprint);
        assert_eq!(body_markdown, "Keep durable context readable.");
        assert!(
            service
                .get_concept(Uuid::new_v4(), wiki.id, created.concept_id)
                .is_err()
        );
    }

    #[test]
    fn memory_write_rejects_non_text_control_characters() {
        let (_temp, _database, service, app_id) = service();
        let wiki = service.create(app_id, "Text memory").unwrap();
        let input = AiMemoryConceptInput {
            title: "Decision".to_owned(),
            description: String::new(),
            concept_type: ConceptType::Other("Decision".to_owned()),
            tags: Vec::new(),
            body_markdown: "Invalid\0body".to_owned(),
        };

        assert!(service.write(app_id, wiki.id, None, None, &input).is_err());
        assert!(service.get(app_id, wiki.id).unwrap().is_empty());
    }

    #[test]
    fn memory_creation_rejects_unbounded_or_controlled_names() {
        let (_temp, _database, service, app_id) = service();

        assert!(service.create(app_id, "\0invalid").is_err());
        assert!(
            service
                .create(app_id, &"x".repeat(AI_MEMORY_WIKI_NAME_MAX_CHARS + 1))
                .is_err()
        );
        assert!(service.list(app_id).unwrap().is_empty());
    }

    #[test]
    fn confirmed_computation_result_is_machine_confirmed_by_the_process() {
        let (_temp, _database, service, app_id) = service();
        let wiki = service.create(app_id, "Computed memory").unwrap();
        let run_id = Uuid::new_v4();

        let stored = service
            .save_process_result(
                wiki.id,
                run_id,
                "Revenue result",
                &serde_json::json!({"value": 42}),
            )
            .unwrap();
        let repeated = service
            .save_process_result(
                wiki.id,
                run_id,
                "Revenue result",
                &serde_json::json!({"value": 42}),
            )
            .unwrap();
        let markdown =
            std::fs::read_to_string(wiki.wiki_folder.join(&stored.logical_path)).unwrap();

        assert_eq!(stored.assurance.trust, TrustTier::MachineConfirmed);
        assert_eq!(stored.concept_id, repeated.concept_id);
        assert_eq!(service.get(app_id, wiki.id).unwrap().len(), 1);
        assert_eq!(
            stored
                .generation
                .as_ref()
                .and_then(|generation| generation.get("by"))
                .and_then(serde_json::Value::as_str),
            Some("process:airwiki-wasm")
        );
        assert!(markdown.contains("by: process:airwiki-wasm"));
        assert!(markdown.contains("\"value\":42"));
        assert!(!markdown.contains("human:"));
    }

    #[test]
    fn valid_large_computation_receipt_can_be_saved() {
        let (_temp, _database, service, app_id) = service();
        let wiki = service.create(app_id, "Large computed memory").unwrap();
        let receipt = serde_json::json!({"payload": "x".repeat(64 * 1024)});

        let stored = service
            .save_process_result(
                wiki.id,
                Uuid::new_v4(),
                "Large computation result",
                &receipt,
            )
            .unwrap();

        let markdown = std::fs::read_to_string(wiki.wiki_folder.join(stored.logical_path)).unwrap();
        assert!(markdown.len() > AI_MEMORY_CONCEPT_MAX_BYTES);
        assert!(markdown.contains("process:airwiki-wasm"));
    }

    #[test]
    fn confirmed_computation_result_can_be_saved_to_active_project_memory() {
        let (temp, database, memories, app_id) = service();
        let project_root = temp.path().join("computed-project");
        std::fs::create_dir(&project_root).unwrap();
        let project_root = std::fs::canonicalize(project_root).unwrap();
        let projects = crate::ProjectMemoryService::new(database);
        let request_id = match projects
            .initialize(app_id, &project_root, "Computed project")
            .unwrap()
        {
            crate::ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected project memory result: {other:?}"),
        };
        let approval = projects.approve(request_id).unwrap();
        projects.reconcile(approval.collection.id).unwrap();
        let index_path = approval.collection.wiki_folder.join("index.md");
        let original_index = std::fs::read(&index_path).unwrap();

        let stored = memories
            .save_process_result(
                approval.collection.id,
                Uuid::new_v4(),
                "Portable computation",
                &serde_json::json!({"value": 7}),
            )
            .unwrap();

        assert_eq!(stored.assurance.trust, TrustTier::MachineConfirmed);
        assert_eq!(std::fs::read(index_path).unwrap(), original_index);
        assert!(
            approval
                .collection
                .wiki_folder
                .join(stored.logical_path)
                .is_file()
        );
    }

    #[test]
    fn computation_result_is_rejected_when_project_memory_is_withheld() {
        let (temp, database, memories, app_id) = service();
        let project_root = temp.path().join("blocked-project");
        std::fs::create_dir(&project_root).unwrap();
        let project_root = std::fs::canonicalize(project_root).unwrap();
        let projects = crate::ProjectMemoryService::new(database);
        let request_id = match projects
            .initialize(app_id, &project_root, "Blocked project")
            .unwrap()
        {
            crate::ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected project memory result: {other:?}"),
        };
        let approval = projects.approve(request_id).unwrap();
        projects.reconcile(approval.collection.id).unwrap();
        projects
            .withhold_watcher_unavailable(approval.collection.id)
            .unwrap();

        assert!(
            memories
                .save_process_result(
                    approval.collection.id,
                    Uuid::new_v4(),
                    "Blocked computation",
                    &serde_json::json!({"value": 7}),
                )
                .is_err()
        );
        assert!(!approval.collection.wiki_folder.join("results").exists());
    }

    #[cfg(unix)]
    #[test]
    fn memory_write_rejects_a_symlinked_concept_directory() {
        use std::os::unix::fs::symlink;

        let (temp, _database, service, app_id) = service();
        let wiki = service.create(app_id, "Confined memory").unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, wiki.wiki_folder.join("concepts")).unwrap();
        let input = AiMemoryConceptInput {
            title: "Escaping concept".to_owned(),
            description: String::new(),
            concept_type: ConceptType::Reference,
            tags: Vec::new(),
            body_markdown: "Must remain inside the managed vault.".to_owned(),
        };

        assert!(service.write(app_id, wiki.id, None, None, &input).is_err());
        assert!(std::fs::read_dir(outside).unwrap().next().is_none());
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
    fn application_mutations_preserve_human_verification_as_outdated() {
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
        let updated_input = AiMemoryConceptInput {
            body_markdown: "Keep the data local and encrypted.".to_owned(),
            ..input
        };

        let updated = service
            .write(
                app_id,
                wiki.id,
                Some(verified.concept_id),
                Some(&verified.fingerprint),
                &updated_input,
            )
            .unwrap();
        let deprecated = service
            .deprecate(app_id, wiki.id, updated.concept_id, &updated.fingerprint)
            .unwrap();
        let markdown =
            std::fs::read_to_string(wiki.wiki_folder.join(&deprecated.logical_path)).unwrap();

        assert_eq!(updated.assurance.trust, TrustTier::HumanReviewed);
        assert!(updated.assurance.verification_outdated);
        assert_eq!(deprecated.assurance.trust, TrustTier::HumanReviewed);
        assert!(deprecated.assurance.verification_outdated);
        assert!(markdown.contains("by: human:airwiki-user"));
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
                .save_process_result(
                    wiki.id,
                    Uuid::new_v4(),
                    "Blocked result",
                    &serde_json::json!({"value": 42}),
                )
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
        let conflict = service
            .write(
                app_id,
                wiki.id,
                Some(created.concept_id),
                Some("stale"),
                &input,
            )
            .expect_err("stale fingerprint must fail");
        assert_eq!(
            conflict.downcast_ref::<AiMemoryError>(),
            Some(&AiMemoryError::FingerprintConflict)
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
    fn external_concept_change_is_never_overwritten_by_an_application() {
        let (_temp, database, service, app_id) = service();
        let wiki = service.create(app_id, "Externally changed memory").unwrap();
        let input = AiMemoryConceptInput {
            title: "Fact".to_owned(),
            description: String::new(),
            concept_type: ConceptType::Reference,
            tags: Vec::new(),
            body_markdown: "Original fact".to_owned(),
        };
        let created = service.write(app_id, wiki.id, None, None, &input).unwrap();
        let path = wiki.wiki_folder.join(&created.logical_path);
        let external = b"---\ntype: Reference\n---\n\nExternally changed fact\n";
        std::fs::write(&path, external).unwrap();
        let replacement = AiMemoryConceptInput {
            body_markdown: "Application replacement".to_owned(),
            ..input
        };

        assert!(
            service
                .write(
                    app_id,
                    wiki.id,
                    Some(created.concept_id),
                    Some(&created.fingerprint),
                    &replacement,
                )
                .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), external);
        assert!(
            database
                .pending_managed_bundle_mutations_for_collection(wiki.id)
                .unwrap()
                .is_empty()
        );
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
        assert!(
            database
                .set_application_wiki_role(
                    collaborator,
                    wiki.id,
                    Some(ApplicationWikiRole::Reader),
                )
                .is_err()
        );
        database
            .set_application_capability_revoked(collaborator, false)
            .unwrap();
        assert_eq!(
            database
                .application_wiki_role(collaborator, wiki.id)
                .unwrap(),
            None
        );
        assert!(service.get(collaborator, wiki.id).is_err());
    }

    #[test]
    fn editor_mutations_use_the_owner_managed_size_quota() {
        let (_temp, database, service, owner) = service();
        let quota_wiki = service.create(owner, "Quota memory").unwrap();
        let shared_wiki = service.create(owner, "Shared memory").unwrap();
        database
            .update_collection_okf_metadata(
                quota_wiki.id,
                Some("0.2"),
                &airwiki_types::OkfCompatibility::DeclaredV02,
                256 * 1024 * 1024,
            )
            .unwrap();
        let editor = Uuid::new_v4();
        database
            .create_application_capability(
                editor,
                "Claude",
                "claude",
                "claude/test",
                "fedcba9876543210",
                &"b".repeat(64),
            )
            .unwrap();
        database
            .set_application_wiki_role(editor, shared_wiki.id, Some(ApplicationWikiRole::Editor))
            .unwrap();
        let input = AiMemoryConceptInput {
            title: "Shared decision".to_owned(),
            description: String::new(),
            concept_type: ConceptType::Other("Decision".to_owned()),
            tags: Vec::new(),
            body_markdown: "Keep the owner's quota authoritative.".to_owned(),
        };

        assert!(
            service
                .write(editor, shared_wiki.id, None, None, &input)
                .is_err()
        );
    }

    #[test]
    fn pending_bundle_mutation_blocks_overlapping_writes() {
        let (_temp, database, service, owner) = service();
        let wiki = service.create(owner, "Serialized memory").unwrap();
        let input = AiMemoryConceptInput {
            title: "Decision".to_owned(),
            description: String::new(),
            concept_type: ConceptType::Other("Decision".to_owned()),
            tags: Vec::new(),
            body_markdown: "Serialize managed writes.".to_owned(),
        };
        let concept = service.write(owner, wiki.id, None, None, &input).unwrap();
        let pending = database
            .begin_managed_bundle_mutation(
                wiki.id,
                &concept.logical_path,
                Some(&concept.fingerprint),
                &"a".repeat(64),
            )
            .unwrap();

        assert!(
            service
                .write(
                    owner,
                    wiki.id,
                    Some(concept.concept_id),
                    Some(&concept.fingerprint),
                    &input,
                )
                .is_err()
        );
        assert!(
            database
                .begin_managed_bundle_mutation(
                    wiki.id,
                    &concept.logical_path,
                    Some("stale"),
                    &"b".repeat(64),
                )
                .is_err()
        );

        database
            .complete_managed_bundle_mutation(pending.id)
            .unwrap();
        assert!(
            database
                .begin_managed_bundle_mutation(
                    wiki.id,
                    &concept.logical_path,
                    Some("stale"),
                    &"b".repeat(64),
                )
                .is_err()
        );
        assert!(
            service
                .write(
                    owner,
                    wiki.id,
                    Some(concept.concept_id),
                    Some(&concept.fingerprint),
                    &input,
                )
                .is_ok()
        );
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
                policy: airwiki_types::CollectionPolicy::connected_to_ai_apps(),
                origin: WikiOrigin::AiMemory,
                replacement_fingerprint: "f".repeat(64),
                owner_app_id: Some(app_id),
                initial_application_access: InitialApplicationAccess::None,
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
    fn recovery_applies_only_the_journaled_concept_replacement() {
        let (_temp, database, service, app_id) = service();
        let wiki = service.create(app_id, "Recoverable memory").unwrap();
        let input = AiMemoryConceptInput {
            title: "Fact".to_owned(),
            description: String::new(),
            concept_type: ConceptType::Reference,
            tags: Vec::new(),
            body_markdown: "Before crash".to_owned(),
        };
        let original = service.write(app_id, wiki.id, None, None, &input).unwrap();
        let replacement = b"---\ntype: Reference\nstatus: stable\n---\n\nAfter crash\n";
        let replacement_fingerprint = hex::encode(Sha256::digest(replacement));
        database
            .begin_managed_bundle_mutation(
                wiki.id,
                &original.logical_path,
                Some(&original.fingerprint),
                &replacement_fingerprint,
            )
            .unwrap();
        std::fs::write(wiki.wiki_folder.join(&original.logical_path), replacement).unwrap();

        let report = service.recover_pending().unwrap();
        let recovered = database
            .list_okf_concept_projection(wiki.id)
            .unwrap()
            .into_iter()
            .find(|concept| concept.logical_path == original.logical_path)
            .unwrap();

        assert_eq!(report.recovered, 1);
        assert_eq!(report.pending, 0);
        assert_eq!(recovered.fingerprint, replacement_fingerprint);
        assert!(
            database
                .pending_managed_bundle_mutations()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn recovery_keeps_ambiguous_concept_bytes_blocked() {
        let (_temp, database, service, app_id) = service();
        let wiki = service.create(app_id, "Ambiguous memory").unwrap();
        let input = AiMemoryConceptInput {
            title: "Fact".to_owned(),
            description: String::new(),
            concept_type: ConceptType::Reference,
            tags: Vec::new(),
            body_markdown: "Before crash".to_owned(),
        };
        let original = service.write(app_id, wiki.id, None, None, &input).unwrap();
        let mutation = database
            .begin_managed_bundle_mutation(
                wiki.id,
                &original.logical_path,
                Some(&original.fingerprint),
                &"a".repeat(64),
            )
            .unwrap();
        let ambiguous = b"---\ntype: Reference\nstatus: stable\n---\n\nUnknown state\n";
        let path = wiki.wiki_folder.join(&original.logical_path);
        std::fs::write(&path, ambiguous).unwrap();

        let report = service.recover_pending().unwrap();
        let projection = database
            .list_okf_concept_projection(wiki.id)
            .unwrap()
            .into_iter()
            .find(|concept| concept.logical_path == original.logical_path)
            .unwrap();
        let pending = database.pending_managed_bundle_mutations().unwrap();

        assert_eq!(report.recovered, 0);
        assert_eq!(report.pending, 1);
        assert_eq!(projection.fingerprint, original.fingerprint);
        assert_eq!(std::fs::read(path).unwrap(), ambiguous);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, mutation.id);
        assert_eq!(
            pending[0].state,
            ManagedBundleMutationState::RecoveryRequired
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

    #[test]
    fn project_memory_writes_keep_index_and_extended_frontmatter() {
        let (temp, database, memories, app_id) = service();
        let project_root = temp.path().join("project");
        std::fs::create_dir(&project_root).unwrap();
        let project_root = std::fs::canonicalize(project_root).unwrap();
        let projects = crate::ProjectMemoryService::new(database.clone());
        let request_id = match projects
            .initialize(app_id, &project_root, "Portable memory")
            .unwrap()
        {
            crate::ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected project memory result: {other:?}"),
        };
        let approval = projects.approve(request_id).unwrap();
        projects.reconcile(approval.collection.id).unwrap();
        let index_path = approval.collection.wiki_folder.join("index.md");
        let original_index = std::fs::read(&index_path).unwrap();
        let created = memories
            .write(
                app_id,
                approval.collection.id,
                None,
                None,
                &AiMemoryConceptInput {
                    title: "Architecture".to_owned(),
                    description: "Durable design".to_owned(),
                    concept_type: ConceptType::parse("Architecture").unwrap(),
                    tags: vec!["portable".to_owned()],
                    body_markdown: "Initial body".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(std::fs::read(&index_path).unwrap(), original_index);

        let concept_path = approval.collection.wiki_folder.join(&created.logical_path);
        let external = std::fs::read_to_string(&concept_path)
            .unwrap()
            .replacen(
                "\n---\n",
                "\nsources:\n  - resource: docs/design.md\nverified:\n  by: human:owner\n  at: 2026-08-23T00:00:00Z\nx-extension: keep\n---\n",
                1,
            );
        std::fs::write(&concept_path, external).unwrap();
        projects.reconcile(approval.collection.id).unwrap();
        let refreshed = memories
            .get(app_id, approval.collection.id)
            .unwrap()
            .into_iter()
            .find(|concept| concept.concept_id == created.concept_id)
            .unwrap();
        memories
            .write(
                app_id,
                approval.collection.id,
                Some(refreshed.concept_id),
                Some(&refreshed.fingerprint),
                &AiMemoryConceptInput {
                    title: "Architecture".to_owned(),
                    description: "Updated durable design".to_owned(),
                    concept_type: ConceptType::parse("Architecture").unwrap(),
                    tags: vec!["portable".to_owned()],
                    body_markdown: "Updated body".to_owned(),
                },
            )
            .unwrap();
        let stored = std::fs::read_to_string(concept_path).unwrap();

        assert_eq!(std::fs::read(index_path).unwrap(), original_index);
        assert!(stored.contains("x-extension: keep"));
        assert!(stored.contains("sources:"));
        assert!(stored.contains("human:owner"));
        assert!(stored.contains("Updated body"));
    }
}
