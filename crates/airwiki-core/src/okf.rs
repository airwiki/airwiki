use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use airwiki_types::{ConceptType, DocumentStatus, EnrichmentDraft};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::config::CollectionPaths;
use crate::storage::{ConceptRecord, SourceDocumentRecord};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkfConcept {
    #[serde(rename = "type")]
    pub concept_type: ConceptType,
    pub title: String,
    pub description: String,
    pub resource: String,
    pub tags: Vec<String>,
    pub timestamp: DateTime<Utc>,
    pub airwiki: AirWikiProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirWikiProfile {
    pub profile_version: u32,
    pub id: Uuid,
    pub collection_id: Uuid,
    pub source_sha256: String,
    pub revision: u32,
    pub language: String,
    pub status: String,
    pub generator_model: String,
    pub reviewed_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum OkfValidationError {
    #[error("title is required")]
    MissingTitle,
    #[error("description is required")]
    MissingDescription,
    #[error("logical resource must be a airwiki URN")]
    InvalidResource,
    #[error("published concepts require a human review timestamp")]
    MissingReview,
    #[error("source SHA-256 must be 64 lowercase hexadecimal characters")]
    InvalidSha256,
    #[error("profile version {0} is unsupported")]
    UnsupportedProfile(u32),
    #[error("language is required")]
    MissingLanguage,
    #[error("at most ten tags are allowed")]
    TooManyTags,
    #[error("frontmatter delimiters are missing")]
    MissingFrontmatter,
    #[error("invalid YAML frontmatter: {0}")]
    InvalidYaml(String),
}

impl OkfConcept {
    pub fn from_records(
        concept: &ConceptRecord,
        source: &SourceDocumentRecord,
        reviewed_at: DateTime<Utc>,
    ) -> Self {
        Self {
            concept_type: concept.draft.concept_type,
            title: concept.draft.title.clone(),
            description: concept.draft.description.clone(),
            resource: concept.logical_resource_uri.clone(),
            tags: concept.draft.tags.clone(),
            // Human approval creates the visible OKF revision. `updated_at` is
            // operational state and changes again when two-phase publication
            // commits, so it is not a stable "last meaningful change" value.
            timestamp: reviewed_at,
            airwiki: AirWikiProfile {
                profile_version: 1,
                id: concept.id,
                collection_id: concept.collection_id,
                source_sha256: source.source_sha256.clone(),
                revision: source.revision,
                language: concept.draft.language.clone(),
                status: "published".into(),
                generator_model: concept.generator_model.clone(),
                reviewed_at,
            },
        }
    }

    pub fn validate(&self) -> std::result::Result<(), OkfValidationError> {
        if self.title.trim().is_empty() {
            return Err(OkfValidationError::MissingTitle);
        }
        if self.description.trim().is_empty() {
            return Err(OkfValidationError::MissingDescription);
        }
        if !self.resource.starts_with("urn:airwiki:") {
            return Err(OkfValidationError::InvalidResource);
        }
        if self.airwiki.profile_version != 1 {
            return Err(OkfValidationError::UnsupportedProfile(
                self.airwiki.profile_version,
            ));
        }
        if self.airwiki.language.trim().is_empty() {
            return Err(OkfValidationError::MissingLanguage);
        }
        if self.tags.len() > 10 {
            return Err(OkfValidationError::TooManyTags);
        }
        let sha = &self.airwiki.source_sha256;
        if sha.len() != 64
            || !sha
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(OkfValidationError::InvalidSha256);
        }
        Ok(())
    }

    pub fn render(&self, draft: &EnrichmentDraft) -> Result<String> {
        self.validate()?;
        let yaml = serde_yaml::to_string(self).context("could not serialize OKF frontmatter")?;
        let mut output = format!(
            "---\n{yaml}---\n\n# {}\n\n{}\n",
            self.title,
            draft.summary.trim()
        );
        if !draft.entities.is_empty() {
            output.push_str("\n## Entidades sugeridas\n\n");
            for entity in &draft.entities {
                output.push_str(&format!("- {} ({})\n", entity.name, entity.kind));
            }
        }
        if !draft.links.is_empty() {
            output.push_str("\n## Enlaces sugeridos\n\n");
            for link in &draft.links {
                output.push_str(&format!("- [{}]({})\n", link.label, link.target));
            }
        }
        Ok(output)
    }

    pub fn parse(markdown: &str) -> std::result::Result<Self, OkfValidationError> {
        let rest = markdown
            .strip_prefix("---\n")
            .ok_or(OkfValidationError::MissingFrontmatter)?;
        let end = rest
            .find("\n---\n")
            .ok_or(OkfValidationError::MissingFrontmatter)?;
        let value: Self = serde_yaml::from_str(&rest[..end])
            .map_err(|error| OkfValidationError::InvalidYaml(error.to_string()))?;
        value.validate()?;
        Ok(value)
    }
}

#[derive(Debug, Clone)]
pub struct OkfPublisher {
    paths: CollectionPaths,
}

impl OkfPublisher {
    pub fn new(wiki_root: impl AsRef<Path>) -> Self {
        Self {
            paths: CollectionPaths::at(wiki_root),
        }
    }

    pub fn concept_path(&self, id: Uuid) -> PathBuf {
        self.paths.concepts.join(format!("{id}.md"))
    }

    pub fn validate_candidate(
        &self,
        concept: &ConceptRecord,
        source: &SourceDocumentRecord,
        reviewed_at: DateTime<Utc>,
    ) -> Result<String> {
        if concept.status != DocumentStatus::NeedsReview
            && concept.status != DocumentStatus::Publishing
            && concept.status != DocumentStatus::Published
        {
            bail!("only reviewed candidates can be rendered as OKF");
        }
        let profile = OkfConcept::from_records(concept, source, reviewed_at);
        let rendered = profile.render(&concept.draft)?;
        OkfConcept::parse(&rendered)?;
        Ok(rendered)
    }

    pub fn publish(
        &self,
        concept: &ConceptRecord,
        source: &SourceDocumentRecord,
        all_published: &[ConceptRecord],
        action: &str,
    ) -> Result<PathBuf> {
        let path = self.write_concept(concept, source)?;
        self.regenerate_index(all_published)?;
        self.append_publication_log(action, concept, source)?;
        Ok(path)
    }

    pub(crate) fn write_concept(
        &self,
        concept: &ConceptRecord,
        source: &SourceDocumentRecord,
    ) -> Result<PathBuf> {
        let reviewed_at = concept
            .reviewed_at
            .context("published concept has no review timestamp")?;
        let rendered = self.validate_candidate(concept, source, reviewed_at)?;
        self.paths.ensure()?;
        let path = self.concept_path(concept.id);
        atomic_write(&path, rendered.as_bytes())?;
        Ok(path)
    }

    pub fn remove(
        &self,
        concept_id: Uuid,
        source_sha256: &str,
        remaining_published: &[ConceptRecord],
    ) -> Result<()> {
        if self.deletion_is_complete(concept_id, source_sha256)? {
            return Ok(());
        }
        self.remove_concept_file(concept_id)?;
        self.regenerate_index(remaining_published)?;
        if !self.deletion_logged(concept_id, source_sha256)? {
            self.prepend_log_entry(
                format!(
                    "* **Deprecation**: Removed concept `{concept_id}` for source hash `{source_sha256}`."
                ),
                deletion_marker(concept_id, source_sha256),
            )?;
        }
        Ok(())
    }

    /// Removes a partially written publication without recording a semantic
    /// deletion. This is used when filesystem publication failed after SQLite
    /// had already performed its guarded transition to `published`.
    pub fn discard_failed_publication(
        &self,
        concept_id: Uuid,
        remaining_published: &[ConceptRecord],
    ) -> Result<()> {
        self.remove_concept_file(concept_id)?;
        self.regenerate_index(remaining_published)
    }

    pub fn regenerate_index(&self, published: &[ConceptRecord]) -> Result<()> {
        self.paths.ensure()?;
        let index = Self::render_index(published)?;
        atomic_write(&self.paths.index, index.as_bytes())
    }

    pub(crate) fn render_index(published: &[ConceptRecord]) -> Result<String> {
        let mut concepts = published.to_vec();
        concepts.sort_by(|left, right| {
            left.draft
                .title
                .to_lowercase()
                .cmp(&right.draft.title.to_lowercase())
        });
        let mut index = String::from("# Conceptos publicados\n\n");
        for concept in concepts {
            index.push_str(&format!(
                "* [{}](concepts/{}.md) - {}\n",
                markdown_label(&concept.draft.title),
                concept.id,
                markdown_inline(&concept.draft.description)
            ));
        }
        validate_index_content(&index)?;
        Ok(index)
    }

    pub(crate) fn append_publication_log(
        &self,
        action: &str,
        concept: &ConceptRecord,
        source: &SourceDocumentRecord,
    ) -> Result<()> {
        let (verb, prose_action) = match action {
            "replaced" => ("Update", "Replaced"),
            _ => ("Creation", "Published"),
        };
        self.prepend_log_entry(
            format!(
                "* **{verb}**: {prose_action} [{}](concepts/{}.md), revision {} with source hash `{}`.",
                markdown_label(&concept.draft.title),
                concept.id,
                source.revision,
                source.source_sha256
            ),
            publication_marker(action, concept.id, source.revision, &source.source_sha256),
        )
    }

    fn prepend_log_entry(&self, entry: String, marker: String) -> Result<()> {
        self.paths.ensure()?;
        let existing = match fs::read_to_string(&self.paths.log) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                "# Directory Update Log\n".to_owned()
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not read {}", self.paths.log.display()));
            }
        };
        if existing.contains(&marker) {
            return Ok(());
        }
        let mut groups = parse_log_groups(&existing)?;
        groups
            .entry(Utc::now().date_naive())
            .or_default()
            .insert(0, format!("{entry} {marker}"));
        let rendered = render_log_groups(&groups);
        validate_log_content(&rendered)?;
        atomic_write(&self.paths.log, rendered.as_bytes())
    }

    fn remove_concept_file(&self, concept_id: Uuid) -> Result<()> {
        let path = self.concept_path(concept_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("could not remove {}", path.display()))
            }
        }
    }

    fn deletion_is_complete(&self, concept_id: Uuid, source_sha256: &str) -> Result<bool> {
        if self.concept_path(concept_id).exists() || !self.paths.index.is_file() {
            return Ok(false);
        }
        let index = fs::read_to_string(&self.paths.index)
            .with_context(|| format!("could not read {}", self.paths.index.display()))?;
        Ok(!index.contains(&concept_id.to_string())
            && self.deletion_logged(concept_id, source_sha256)?)
    }

    fn deletion_logged(&self, concept_id: Uuid, source_sha256: &str) -> Result<bool> {
        let log = match fs::read_to_string(&self.paths.log) {
            Ok(log) => log,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not read {}", self.paths.log.display()));
            }
        };
        Ok(log.contains(&deletion_marker(concept_id, source_sha256)))
    }
}

fn publication_marker(action: &str, concept_id: Uuid, revision: u32, sha256: &str) -> String {
    format!("<!-- airwiki:event:{action}:{concept_id}:{revision}:{sha256} -->")
}

fn deletion_marker(concept_id: Uuid, sha256: &str) -> String {
    format!("<!-- airwiki:event:deleted:{concept_id}:{sha256} -->")
}

fn markdown_label(value: &str) -> String {
    collapse_whitespace(value)
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn markdown_inline(value: &str) -> String {
    collapse_whitespace(value)
        .replace('\\', "\\\\")
        .replace('<', "\\<")
        .replace('>', "\\>")
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_index_content(content: &str) -> Result<()> {
    if content.starts_with("---") {
        bail!("OKF index.md cannot contain concept frontmatter");
    }
    let mut lines = content.lines().filter(|line| !line.trim().is_empty());
    if !lines.next().is_some_and(|line| line.starts_with("# ")) {
        bail!("OKF index.md must begin with a section heading");
    }
    for line in lines {
        if !(line.starts_with("# ") || line.starts_with("* [")) {
            bail!("OKF index.md contains an invalid directory entry");
        }
    }
    Ok(())
}

fn parse_log_groups(content: &str) -> Result<BTreeMap<NaiveDate, Vec<String>>> {
    validate_log_content(content)?;
    let mut groups = BTreeMap::<NaiveDate, Vec<String>>::new();
    let mut current = None;
    for line in content.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(date) = line.strip_prefix("## ") {
            current = Some(NaiveDate::parse_from_str(date, "%Y-%m-%d")?);
        } else if line.starts_with("* ") {
            groups
                .entry(current.context("OKF log entry appears before its date heading")?)
                .or_default()
                .push(line.to_owned());
        }
    }
    Ok(groups)
}

fn render_log_groups(groups: &BTreeMap<NaiveDate, Vec<String>>) -> String {
    let mut output = String::from("# Directory Update Log\n\n");
    for (date, entries) in groups.iter().rev() {
        output.push_str(&format!("## {}\n\n", date.format("%Y-%m-%d")));
        for entry in entries {
            output.push_str(entry);
            output.push('\n');
        }
        output.push('\n');
    }
    output
}

fn validate_log_content(content: &str) -> Result<()> {
    if content.starts_with("---") {
        bail!("OKF log.md cannot contain frontmatter");
    }
    let mut lines = content.lines();
    if lines.next() != Some("# Directory Update Log") {
        bail!("OKF log.md must begin with '# Directory Update Log'");
    }
    let mut current_date = None;
    let mut saw_date = false;
    for line in lines.map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(date) = line.strip_prefix("## ") {
            let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .context("OKF log date heading must use YYYY-MM-DD")?;
            if current_date.is_some_and(|previous| date >= previous) {
                bail!("OKF log date groups must be unique and newest-first");
            }
            current_date = Some(date);
            saw_date = true;
        } else if line.starts_with("* ") && current_date.is_some() {
            continue;
        } else {
            bail!("OKF log.md contains an invalid date group or prose entry");
        }
    }
    if !saw_date && content.trim() != "# Directory Update Log" {
        bail!("OKF log.md contains content without a date group");
    }
    Ok(())
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("target path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        Uuid::new_v4()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let mut temporary_guard = TemporaryFileGuard::new(&temporary);
    let write_result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    write_result.with_context(|| "could not persist temporary OKF file")?;

    replace_atomically(&temporary, path).with_context(|| {
        format!(
            "could not atomically move {} to {}",
            temporary.display(),
            path.display()
        )
    })?;
    temporary_guard.disarm();
    sync_parent_directory(parent).with_context(|| "could not persist the OKF directory entry")?;
    Ok(())
}

struct TemporaryFileGuard<'a> {
    path: &'a Path,
    armed: bool,
}

impl<'a> TemporaryFileGuard<'a> {
    fn new(path: &'a Path) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TemporaryFileGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(self.path);
        }
    }
}

#[cfg(unix)]
fn replace_atomically(temporary: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(temporary, target)
}

#[cfg(windows)]
fn replace_atomically(temporary: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    const MAX_ATTEMPTS: u64 = 5;

    fn nul_terminated(path: &Path) -> std::io::Result<Vec<u16>> {
        let mut encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if encoded.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Windows path contains an interior NUL",
            ));
        }
        encoded.push(0);
        Ok(encoded)
    }

    fn is_retryable(error: &std::io::Error) -> bool {
        matches!(
            error.raw_os_error(),
            Some(code)
                if code == ERROR_SHARING_VIOLATION as i32
                    || code == ERROR_LOCK_VIOLATION as i32
        )
    }

    let temporary = nul_terminated(temporary)?;
    let target = nul_terminated(target)?;
    let mut attempt = 1_u64;
    loop {
        // SAFETY: both pointers reference live, NUL-terminated UTF-16 buffers for the
        // duration of the call. The buffers are not aliased mutably, and the flags
        // request an atomic replacement with write-through semantics.
        let moved = unsafe {
            MoveFileExW(
                temporary.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved != 0 {
            return Ok(());
        }

        let error = std::io::Error::last_os_error();
        if !is_retryable(&error) || attempt == MAX_ATTEMPTS {
            return Err(error);
        }
        std::thread::sleep(Duration::from_millis(20 * attempt));
        attempt += 1;
    }
}

#[cfg(not(any(unix, windows)))]
fn replace_atomically(temporary: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(temporary, target)
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> std::io::Result<()> {
    fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use airwiki_types::{SuggestedEntity, SuggestedLink};

    use super::*;

    fn records() -> (tempfile::TempDir, ConceptRecord, SourceDocumentRecord) {
        let temp = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let collection_id = Uuid::new_v4();
        let source_id = Uuid::new_v4();
        let concept_id = Uuid::new_v4();
        let draft = EnrichmentDraft {
            concept_type: ConceptType::Runbook,
            title: "Recuperación de pagos".into(),
            description: "Procedimiento para restaurar el servicio.".into(),
            language: "es".into(),
            tags: vec!["pagos".into(), "incidentes".into()],
            entities: vec![SuggestedEntity {
                name: "API".into(),
                kind: "service".into(),
            }],
            links: vec![SuggestedLink {
                label: "Panel".into(),
                target: "https://example.invalid".into(),
            }],
            summary: "Reiniciar la cola y comprobar el servicio.".into(),
            classification_confidence: 0.9,
            classification_explanation: "Tiene pasos operativos".into(),
        };
        (
            temp,
            ConceptRecord {
                id: concept_id,
                source_document_id: source_id,
                collection_id,
                draft,
                logical_resource_uri: format!("urn:airwiki:peer:{concept_id}"),
                generator_model: "qwen3-1.7b-q8".into(),
                status: DocumentStatus::Published,
                reviewed_at: Some(now),
                created_at: now,
                updated_at: now,
            },
            SourceDocumentRecord {
                id: source_id,
                collection_id,
                source_path: PathBuf::from("private.md"),
                source_sha256: "a".repeat(64),
                source_format: "markdown".into(),
                byte_size: 10,
                page_count: 0,
                character_count: 10,
                status: DocumentStatus::Published,
                revision: 1,
                concept_id: Some(concept_id),
                last_error: None,
                discovered_at: now,
                updated_at: now,
                deleted_at: None,
            },
        )
    }

    #[test]
    fn rendered_profile_round_trips_and_contains_no_source_path() {
        let (_temp, concept, source) = records();
        let profile = OkfConcept::from_records(&concept, &source, concept.reviewed_at.unwrap());
        let rendered = profile.render(&concept.draft).unwrap();
        let parsed = OkfConcept::parse(&rendered).unwrap();
        assert_eq!(parsed.airwiki.id, concept.id);
        assert!(!rendered.contains("private.md"));
    }

    #[test]
    fn publisher_generates_concept_index_and_log() {
        let (temp, concept, source) = records();
        let publisher = OkfPublisher::new(temp.path());
        let path = publisher
            .publish(
                &concept,
                &source,
                std::slice::from_ref(&concept),
                "published",
            )
            .unwrap();
        assert!(path.is_file());
        assert!(temp.path().join("index.md").is_file());
        assert!(temp.path().join("log.md").is_file());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(OkfConcept::parse(&content).is_ok());
        let index = std::fs::read_to_string(temp.path().join("index.md")).unwrap();
        assert!(!index.starts_with("---"));
        validate_index_content(&index).unwrap();
        let log = std::fs::read_to_string(temp.path().join("log.md")).unwrap();
        validate_log_content(&log).unwrap();
        assert!(log.contains(&format!("## {}", Utc::now().format("%Y-%m-%d"))));
    }

    #[test]
    fn invalid_hash_is_rejected() {
        let (_temp, concept, mut source) = records();
        source.source_sha256 = "bad".into();
        let profile = OkfConcept::from_records(&concept, &source, Utc::now());
        assert!(matches!(
            profile.validate(),
            Err(OkfValidationError::InvalidSha256)
        ));
    }

    #[test]
    fn deletion_cleanup_is_idempotent() {
        let (temp, concept, source) = records();
        let publisher = OkfPublisher::new(temp.path());
        publisher
            .publish(
                &concept,
                &source,
                std::slice::from_ref(&concept),
                "published",
            )
            .unwrap();
        publisher
            .remove(concept.id, &source.source_sha256, &[])
            .unwrap();
        publisher
            .remove(concept.id, &source.source_sha256, &[])
            .unwrap();
        let log = std::fs::read_to_string(temp.path().join("log.md")).unwrap();
        assert_eq!(
            log.matches(&deletion_marker(concept.id, &source.source_sha256))
                .count(),
            1
        );
        validate_log_content(&log).unwrap();
    }

    #[test]
    fn reserved_files_escape_metadata_and_keep_newest_entries_first() {
        let (temp, mut concept, source) = records();
        concept.draft.title = "[Pagos]\n# encabezado inyectado".into();
        concept.draft.description = "Primera línea\n## grupo inyectado".into();
        let publisher = OkfPublisher::new(temp.path());
        publisher
            .publish(
                &concept,
                &source,
                std::slice::from_ref(&concept),
                "published",
            )
            .unwrap();
        publisher
            .publish(
                &concept,
                &source,
                std::slice::from_ref(&concept),
                "replaced",
            )
            .unwrap();

        let index = std::fs::read_to_string(temp.path().join("index.md")).unwrap();
        validate_index_content(&index).unwrap();
        assert_eq!(
            index.lines().filter(|line| line.starts_with("# ")).count(),
            1
        );
        assert!(index.contains("\\[Pagos\\] # encabezado inyectado"));
        assert!(index.contains("Primera línea ## grupo inyectado"));

        let log = std::fs::read_to_string(temp.path().join("log.md")).unwrap();
        validate_log_content(&log).unwrap();
        assert!(log.find("**Update**").unwrap() < log.find("**Creation**").unwrap());
        assert_eq!(
            log.lines().filter(|line| line.starts_with("## ")).count(),
            1
        );
    }

    #[test]
    fn reserved_file_validators_reject_non_okf_structures() {
        assert!(validate_index_content("---\ntype: Index\n---\n# Items\n").is_err());
        assert!(
            validate_log_content(
                "# Directory Update Log\n\n## 2026-01-01\n* First\n## 2026-02-01\n* Later\n"
            )
            .is_err()
        );
    }

    #[test]
    fn collection_record_is_path_only_and_never_serialized_into_concept() {
        let (_temp, concept, source) = records();
        let collection = crate::storage::CollectionRecord {
            id: concept.collection_id,
            name: "Secret".into(),
            source_folder: PathBuf::from("/private/source"),
            wiki_folder: PathBuf::from("/private/wiki"),
            policy: airwiki_types::CollectionPolicy::local_only(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let profile = OkfConcept::from_records(&concept, &source, Utc::now());
        let yaml = serde_yaml::to_string(&profile).unwrap();
        assert!(!yaml.contains(collection.source_folder.to_string_lossy().as_ref()));
    }

    #[test]
    fn atomic_write_replaces_existing_content_without_temporary_files() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("index.md");
        fs::write(&target, b"old").unwrap();

        atomic_write(&target, b"new").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(temporary_files(temp.path()).is_empty());
    }

    #[test]
    fn atomic_write_cleans_up_temporary_file_when_replacement_fails() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("index.md");
        fs::create_dir(&target).unwrap();

        assert!(atomic_write(&target, b"new").is_err());

        assert!(target.is_dir());
        assert!(temporary_files(temp.path()).is_empty());
    }

    fn temporary_files(directory: &Path) -> Vec<PathBuf> {
        fs::read_dir(directory)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.starts_with('.') && name.ends_with(".tmp")
                })
            })
            .collect()
    }
}
