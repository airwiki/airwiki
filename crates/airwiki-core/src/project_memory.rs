use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use airwiki_types::OkfCompatibility;
use anyhow::{Context, Result, bail};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::okf::atomic_write;
use crate::{
    CollectionRecord, Database, MemorySearchRecord, NewProjectMemoryAttachment, OkfImportValidator,
    ProjectMemoryAttachmentRecord, ProjectMemoryAttachmentState, ProjectMemoryRequestKind,
    ProjectMemoryRequestRecord, ProjectMemoryRequestState,
};

pub const PROJECT_MEMORY_MANIFEST_MAX_BYTES: usize = 64 * 1024;
pub const PROJECT_MEMORY_REQUEST_TTL_MINUTES: i64 = 10;
const PROJECT_MEMORY_NAME_MAX_CHARS: usize = 120;
const PROJECT_DIRECTORY: &str = ".airwiki";
const MANIFEST_FILE: &str = "project.yaml";
const WIKI_DIRECTORY: &str = "wiki";

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectMemoryManifest {
    pub schema_version: u32,
    pub project_id: Uuid,
    pub wiki_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectMemoryOpenResult {
    NotInitialized,
    AwaitingConfirmation {
        request_id: Uuid,
    },
    Ready {
        collection_id: Uuid,
        portable_wiki_id: Uuid,
    },
}

#[derive(Debug, Clone)]
pub struct ProjectMemoryApproval {
    pub collection: CollectionRecord,
    pub portable_wiki_id: Uuid,
    pub created_files: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectMemoryReconciliationReport {
    pub active: usize,
    pub invalid: usize,
    pub missing: usize,
    pub identity_conflicts: usize,
}

#[derive(Debug, Clone)]
pub struct ProjectMemoryService {
    database: Database,
}

struct InspectedProjectMemory {
    wiki_root: PathBuf,
    manifest: ProjectMemoryManifest,
    manifest_fingerprint: String,
    import: crate::okf_import::OkfImportReport,
}

impl ProjectMemoryService {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub fn initialize(
        &self,
        app_id: Uuid,
        project_root: &Path,
        name: &str,
    ) -> Result<ProjectMemoryOpenResult> {
        let root = canonical_project_root(project_root)?;
        validate_project_name(name)?;
        if root.join(PROJECT_DIRECTORY).exists() {
            return self.open(app_id, &root);
        }
        let request = self.database.request_project_memory_confirmation(
            app_id,
            ProjectMemoryRequestKind::Initialize,
            &root,
            Some(name.trim()),
            None,
            Utc::now() + Duration::minutes(PROJECT_MEMORY_REQUEST_TTL_MINUTES),
        )?;
        Ok(ProjectMemoryOpenResult::AwaitingConfirmation {
            request_id: request.id,
        })
    }

    /// Initializes project memory after the desktop has obtained native user confirmation.
    pub fn initialize_native(&self, project_root: &Path, name: &str) -> Result<CollectionRecord> {
        let root = canonical_project_root(project_root)?;
        validate_project_name(name)?;
        let _guard = self.database.managed_bundle_guard()?;
        if root.join(PROJECT_DIRECTORY).exists() {
            bail!("project memory is already initialized; open it instead");
        }
        self.materialize_project_memory_named(name.trim(), &root)?;
        let inspected = inspect_project_memory(&root)?;
        if let Some(existing) = self.database.project_memory_attachment_for_root(&root)? {
            if existing.project_id != inspected.manifest.project_id
                || existing.portable_wiki_id != inspected.manifest.wiki_id
            {
                self.fail_closed(
                    &existing,
                    ProjectMemoryAttachmentState::IdentityConflict,
                    "portable_identity_changed",
                )?;
                bail!("project memory portable identity conflicts with its local attachment");
            }
            self.activate_inspected(&existing, &inspected)?;
            return self
                .database
                .collection(existing.collection_id)?
                .context("project memory collection disappeared");
        }
        let attachment = NewProjectMemoryAttachment {
            collection_id: Uuid::new_v4(),
            project_id: inspected.manifest.project_id,
            portable_wiki_id: inspected.manifest.wiki_id,
            project_root: root,
            wiki_root: inspected.wiki_root.clone(),
            name: inspected.manifest.name.clone(),
            manifest_fingerprint: inspected.manifest_fingerprint.clone(),
        };
        let collection = self
            .database
            .register_project_memory_attachment(&attachment)?;
        self.database
            .collection(collection.id)?
            .context("project memory collection disappeared")
    }

    pub fn open(&self, app_id: Uuid, project_root: &Path) -> Result<ProjectMemoryOpenResult> {
        let root = canonical_project_root(project_root)?;
        let manifest_path = root.join(PROJECT_DIRECTORY).join(MANIFEST_FILE);
        if !manifest_path.exists() {
            if let Some(attachment) = self.database.project_memory_attachment_for_root(&root)? {
                self.fail_closed(
                    &attachment,
                    ProjectMemoryAttachmentState::Missing,
                    "bundle_missing",
                )?;
            }
            return Ok(ProjectMemoryOpenResult::NotInitialized);
        }

        let attachment = self.database.project_memory_attachment_for_root(&root)?;
        if let Some(attachment) = attachment.as_ref() {
            let state = self.reconcile_attachment_record(attachment)?;
            if state != ProjectMemoryAttachmentState::Active {
                bail!("project memory is unavailable until its local files are repaired");
            }
            if self
                .database
                .application_wiki_role(app_id, attachment.collection_id)?
                .is_some()
            {
                return Ok(ProjectMemoryOpenResult::Ready {
                    collection_id: attachment.collection_id,
                    portable_wiki_id: attachment.portable_wiki_id,
                });
            }
        }

        let inspected = inspect_project_memory(&root)?;

        let request = self.database.request_project_memory_confirmation(
            app_id,
            ProjectMemoryRequestKind::Attach,
            &root,
            None,
            Some(&inspected.manifest_fingerprint),
            Utc::now() + Duration::minutes(PROJECT_MEMORY_REQUEST_TTL_MINUTES),
        )?;
        Ok(ProjectMemoryOpenResult::AwaitingConfirmation {
            request_id: request.id,
        })
    }

    pub fn approve(&self, request_id: Uuid) -> Result<ProjectMemoryApproval> {
        let request = self
            .database
            .project_memory_request(request_id)?
            .context("project memory request does not exist")?;
        if request.state != ProjectMemoryRequestState::AwaitingConfirmation
            || request.expires_at <= Utc::now()
        {
            bail!("project memory request is unavailable or expired");
        }
        let root = canonical_project_root(&request.project_root)?;
        if root != request.project_root {
            bail!("project memory root changed before confirmation");
        }
        let _guard = self.database.managed_bundle_guard()?;
        let created_files = match request.kind {
            ProjectMemoryRequestKind::Initialize => {
                self.materialize_project_memory(&request, &root)?;
                true
            }
            ProjectMemoryRequestKind::Attach => false,
        };
        let inspected = inspect_project_memory(&root)?;
        if request.kind == ProjectMemoryRequestKind::Attach
            && request.manifest_fingerprint.as_deref()
                != Some(inspected.manifest_fingerprint.as_str())
        {
            bail!("project memory changed before confirmation");
        }

        if let Some(existing) = self.database.project_memory_attachment_for_root(&root)? {
            if existing.project_id != inspected.manifest.project_id
                || existing.portable_wiki_id != inspected.manifest.wiki_id
            {
                self.fail_closed(
                    &existing,
                    ProjectMemoryAttachmentState::IdentityConflict,
                    "portable_identity_changed",
                )?;
                bail!("project memory portable identity conflicts with its local attachment");
            }
            self.activate_inspected(&existing, &inspected)?;
            self.database
                .confirm_existing_project_memory_request(request_id, existing.collection_id)?;
            let collection = self
                .database
                .collection(existing.collection_id)?
                .context("project memory collection disappeared")?;
            return Ok(ProjectMemoryApproval {
                collection,
                portable_wiki_id: existing.portable_wiki_id,
                created_files,
            });
        }

        let attachment = NewProjectMemoryAttachment {
            collection_id: Uuid::new_v4(),
            project_id: inspected.manifest.project_id,
            portable_wiki_id: inspected.manifest.wiki_id,
            project_root: root,
            wiki_root: inspected.wiki_root.clone(),
            name: inspected.manifest.name.clone(),
            manifest_fingerprint: inspected.manifest_fingerprint.clone(),
        };
        let collection = self
            .database
            .confirm_project_memory_request(request_id, &attachment)?;
        let collection = self
            .database
            .collection(collection.id)?
            .context("project memory collection disappeared")?;
        Ok(ProjectMemoryApproval {
            collection,
            portable_wiki_id: attachment.portable_wiki_id,
            created_files,
        })
    }

    pub fn reject(&self, request_id: Uuid) -> Result<()> {
        self.database.reject_project_memory_request(request_id)
    }

    pub fn pending_requests(&self) -> Result<Vec<ProjectMemoryRequestRecord>> {
        self.database.pending_project_memory_requests()
    }

    pub fn reconcile_all(&self) -> Result<ProjectMemoryReconciliationReport> {
        let mut report = ProjectMemoryReconciliationReport::default();
        for attachment in self.database.project_memory_attachments()? {
            match self.reconcile_attachment_record(&attachment) {
                Ok(ProjectMemoryAttachmentState::Active) => {
                    report.active = report.active.saturating_add(1);
                }
                Ok(ProjectMemoryAttachmentState::Missing) => {
                    report.missing = report.missing.saturating_add(1);
                }
                Ok(ProjectMemoryAttachmentState::IdentityConflict) => {
                    report.identity_conflicts = report.identity_conflicts.saturating_add(1);
                }
                Ok(ProjectMemoryAttachmentState::Invalid) | Err(_) => {
                    report.invalid = report.invalid.saturating_add(1);
                }
            }
        }
        Ok(report)
    }

    pub fn withhold_active_until_watchers_start(&self) -> Result<()> {
        let _guard = self.database.managed_bundle_guard()?;
        for attachment in self.database.project_memory_attachments()? {
            if attachment.state == ProjectMemoryAttachmentState::Active {
                self.database.withhold_project_memory_attachment(
                    attachment.collection_id,
                    ProjectMemoryAttachmentState::Invalid,
                    "watcher_start_pending",
                )?;
            }
        }
        Ok(())
    }

    pub fn reconcile(&self, collection_id: Uuid) -> Result<ProjectMemoryAttachmentState> {
        let attachment = self
            .database
            .project_memory_attachment(collection_id)?
            .context("project memory attachment does not exist")?;
        self.reconcile_attachment_record(&attachment)
    }

    pub fn detach(&self, collection_id: Uuid) -> Result<()> {
        self.database.detach_project_memory(collection_id)
    }

    pub fn withhold_watcher_unavailable(&self, collection_id: Uuid) -> Result<()> {
        let _guard = self.database.managed_bundle_guard()?;
        self.database.withhold_project_memory_attachment(
            collection_id,
            ProjectMemoryAttachmentState::Invalid,
            "watcher_unavailable",
        )
    }

    pub fn search(
        &self,
        app_id: Uuid,
        collection_id: Uuid,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemorySearchRecord>> {
        if query.is_empty()
            || query.len() > 2 * 1024
            || has_unsupported_text_control(query)
            || !(1..=20).contains(&limit)
        {
            bail!("project memory search input is invalid");
        }
        self.database
            .search_application_memory(app_id, collection_id, query, limit)
    }

    fn reconcile_attachment_record(
        &self,
        attachment: &ProjectMemoryAttachmentRecord,
    ) -> Result<ProjectMemoryAttachmentState> {
        let _guard = self.database.managed_bundle_guard()?;
        let root = match canonical_project_root(&attachment.project_root) {
            Ok(root) if root == attachment.project_root => root,
            Ok(_) => {
                self.fail_closed(
                    attachment,
                    ProjectMemoryAttachmentState::Missing,
                    "project_root_moved",
                )?;
                return Ok(ProjectMemoryAttachmentState::Missing);
            }
            Err(_) => {
                self.fail_closed(
                    attachment,
                    ProjectMemoryAttachmentState::Missing,
                    "project_root_missing",
                )?;
                return Ok(ProjectMemoryAttachmentState::Missing);
            }
        };
        self.database.withhold_project_memory_attachment(
            attachment.collection_id,
            ProjectMemoryAttachmentState::Invalid,
            "reconciling",
        )?;
        let inspected = match inspect_project_memory(&root) {
            Ok(inspected) => inspected,
            Err(_) => {
                let state = if root.join(PROJECT_DIRECTORY).exists() {
                    ProjectMemoryAttachmentState::Invalid
                } else {
                    ProjectMemoryAttachmentState::Missing
                };
                let code = if state == ProjectMemoryAttachmentState::Missing {
                    "bundle_missing"
                } else {
                    "bundle_invalid"
                };
                self.database.withhold_project_memory_attachment(
                    attachment.collection_id,
                    state,
                    code,
                )?;
                return Ok(state);
            }
        };
        if inspected.manifest.project_id != attachment.project_id
            || inspected.manifest.wiki_id != attachment.portable_wiki_id
        {
            self.fail_closed(
                attachment,
                ProjectMemoryAttachmentState::IdentityConflict,
                "portable_identity_changed",
            )?;
            return Ok(ProjectMemoryAttachmentState::IdentityConflict);
        }
        self.activate_inspected(attachment, &inspected)?;
        Ok(ProjectMemoryAttachmentState::Active)
    }

    fn activate_inspected(
        &self,
        attachment: &ProjectMemoryAttachmentRecord,
        inspected: &InspectedProjectMemory,
    ) -> Result<()> {
        if !matches!(
            inspected.import.compatibility,
            OkfCompatibility::DeclaredV02
        ) {
            self.fail_closed(
                attachment,
                ProjectMemoryAttachmentState::Invalid,
                "okf_profile_invalid",
            )?;
            bail!("project memory must declare OKF v0.2");
        }
        self.database.activate_project_memory_attachment(
            attachment.collection_id,
            &inspected.manifest_fingerprint,
            &inspected.import.concepts,
            inspected.import.declared_okf_version.as_deref(),
            &inspected.import.compatibility,
            inspected.import.uncompressed_bytes,
        )
    }

    fn fail_closed(
        &self,
        attachment: &ProjectMemoryAttachmentRecord,
        state: ProjectMemoryAttachmentState,
        error_code: &'static str,
    ) -> Result<()> {
        self.database.withhold_project_memory_attachment(
            attachment.collection_id,
            state,
            error_code,
        )
    }

    fn materialize_project_memory(
        &self,
        request: &ProjectMemoryRequestRecord,
        root: &Path,
    ) -> Result<()> {
        let name = request
            .requested_name
            .as_deref()
            .context("project memory initialization name is missing")?;
        self.materialize_project_memory_named(name, root)
    }

    fn materialize_project_memory_named(&self, name: &str, root: &Path) -> Result<()> {
        validate_project_name(name)?;
        let destination = root.join(PROJECT_DIRECTORY);
        if destination.exists() {
            bail!("project memory was initialized before confirmation");
        }
        let staging = root.join(format!(".airwiki-staging-{}", Uuid::new_v4()));
        let wiki_root = staging.join(WIKI_DIRECTORY);
        let concepts = wiki_root.join("concepts");
        std::fs::create_dir(&staging).context("could not create project memory staging")?;
        let result = (|| {
            std::fs::create_dir(&wiki_root).context("could not create project memory wiki")?;
            std::fs::create_dir(&concepts)
                .context("could not create project memory concepts directory")?;
            let manifest = ProjectMemoryManifest {
                schema_version: 1,
                project_id: Uuid::new_v4(),
                wiki_id: Uuid::new_v4(),
                name: name.to_owned(),
            };
            let yaml = serde_yaml::to_string(&manifest)
                .context("could not serialize project memory manifest")?;
            atomic_write(&staging.join(MANIFEST_FILE), yaml.as_bytes())?;
            let index = render_project_index(name);
            atomic_write(&wiki_root.join("index.md"), index.as_bytes())?;
            let staged = inspect_project_memory_at(&staging, root)?;
            if staged.manifest != manifest {
                bail!("project memory staging identity changed");
            }
            std::fs::rename(&staging, &destination)
                .context("could not atomically initialize project memory")?;
            Ok(())
        })();
        if result.is_err() && staging.exists() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        result
    }
}

fn inspect_project_memory(root: &Path) -> Result<InspectedProjectMemory> {
    inspect_project_memory_at(&root.join(PROJECT_DIRECTORY), root)
}

fn inspect_project_memory_at(
    directory: &Path,
    project_root: &Path,
) -> Result<InspectedProjectMemory> {
    let directory_metadata =
        std::fs::symlink_metadata(directory).context("project memory directory is unavailable")?;
    if !directory_metadata.is_dir() || metadata_is_link_or_reparse(&directory_metadata) {
        bail!("project memory directory is unsafe");
    }
    let manifest_path = directory.join(MANIFEST_FILE);
    let manifest_metadata = std::fs::symlink_metadata(&manifest_path)
        .context("project memory manifest is unavailable")?;
    if !manifest_metadata.is_file()
        || metadata_is_link_or_reparse(&manifest_metadata)
        || manifest_metadata.len() > PROJECT_MEMORY_MANIFEST_MAX_BYTES as u64
    {
        bail!("project memory manifest is unsafe or too large");
    }
    let bytes = std::fs::read(&manifest_path).context("could not read project memory manifest")?;
    if contains_source_control_conflict(&bytes) {
        bail!("project memory manifest contains unresolved conflict markers");
    }
    let manifest: ProjectMemoryManifest =
        serde_yaml::from_slice(&bytes).context("project memory manifest is invalid")?;
    if manifest.schema_version != 1 {
        bail!("project memory schema version is unsupported");
    }
    validate_project_name(&manifest.name)?;
    let wiki_root = directory.join(WIKI_DIRECTORY);
    let import = OkfImportValidator::validate_directory(&wiki_root)
        .context("project memory OKF bundle is invalid")?;
    reject_wiki_conflicts(&wiki_root)?;
    if !matches!(import.compatibility, OkfCompatibility::DeclaredV02) {
        bail!("project memory must declare OKF v0.2");
    }
    let expected_parent = directory
        .parent()
        .context("project memory directory has no parent")?;
    if expected_parent != project_root {
        bail!("project memory directory escaped its project root");
    }
    Ok(InspectedProjectMemory {
        wiki_root,
        manifest,
        manifest_fingerprint: hex::encode(Sha256::digest(&bytes)),
        import,
    })
}

fn canonical_project_root(root: &Path) -> Result<PathBuf> {
    if !root.is_absolute() || has_unsupported_text_control(&root.to_string_lossy()) {
        bail!("project root must be an absolute canonical path");
    }
    let metadata = std::fs::symlink_metadata(root).context("project root is unavailable")?;
    if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
        bail!("project root is not a safe directory");
    }
    let canonical = std::fs::canonicalize(root).context("could not canonicalize project root")?;
    if canonical != root {
        bail!("project root must already be canonical");
    }
    Ok(canonical)
}

fn validate_project_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > PROJECT_MEMORY_NAME_MAX_CHARS
        || has_unsupported_text_control(trimmed)
    {
        bail!("project memory name is invalid");
    }
    Ok(())
}

fn render_project_index(name: &str) -> String {
    let heading = name
        .trim()
        .replace(['\n', '\r'], " ")
        .replace('[', "\\[")
        .replace(']', "\\]");
    format!("---\nokf_version: \"0.2\"\n---\n\n# {heading}\n")
}

fn has_unsupported_text_control(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn reject_wiki_conflicts(wiki_root: &Path) -> Result<()> {
    for entry in WalkDir::new(wiki_root).follow_links(false) {
        let entry = entry.context("project memory conflict scan failed")?;
        if !entry.file_type().is_file()
            || entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| !extension.eq_ignore_ascii_case("md"))
        {
            continue;
        }
        let bytes = std::fs::read(entry.path()).context("could not inspect project memory page")?;
        if contains_source_control_conflict(&bytes) {
            bail!("project memory contains unresolved conflict markers");
        }
    }
    Ok(())
}

fn contains_source_control_conflict(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut started = false;
    let mut separated = false;
    for line in text.lines().map(str::trim_start) {
        if line.starts_with("<<<<<<< ") {
            started = true;
            separated = false;
        } else if started && line.trim_end() == "=======" {
            separated = true;
        } else if started && separated && line.starts_with(">>>>>>> ") {
            return true;
        }
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    fn copy_directory(source: &Path, destination: &Path) {
        std::fs::create_dir(destination).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_directory(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    fn application(database: &Database) -> Uuid {
        let app_id = Uuid::new_v4();
        let capability_prefix = format!("{:016x}", app_id.as_u128() as u64);
        let secret_hash = format!("{:032x}{:032x}", app_id.as_u128(), app_id.as_u128());
        database
            .create_application_capability(
                app_id,
                "Project memory test",
                "mcp",
                "codex/project-memory-test",
                &capability_prefix,
                &secret_hash,
            )
            .unwrap();
        app_id
    }

    fn initialize_and_approve(
        service: &ProjectMemoryService,
        app_id: Uuid,
        root: &Path,
    ) -> ProjectMemoryApproval {
        let request_id = match service.initialize(app_id, root, "Portable docs").unwrap() {
            ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected result: {other:?}"),
        };
        let approval = service.approve(request_id).unwrap();
        service.reconcile(approval.collection.id).unwrap();
        approval
    }

    #[test]
    fn initializes_only_after_confirmation_and_detach_preserves_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database.clone());
        let request_id = match service.initialize(app_id, &root, "Portable docs").unwrap() {
            ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected result: {other:?}"),
        };
        assert!(!root.join(PROJECT_DIRECTORY).exists());
        let approved = service.approve(request_id).unwrap();
        assert!(approved.created_files);
        assert!(root.join(PROJECT_DIRECTORY).join(MANIFEST_FILE).is_file());
        assert_eq!(
            database
                .project_memory_attachment(approved.collection.id)
                .unwrap()
                .unwrap()
                .state,
            ProjectMemoryAttachmentState::Invalid
        );
        service.reconcile(approved.collection.id).unwrap();
        service.detach(approved.collection.id).unwrap();
        assert!(root.join(PROJECT_DIRECTORY).join(MANIFEST_FILE).is_file());
    }

    #[test]
    fn rejects_future_or_extended_manifests() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let directory = root.join(PROJECT_DIRECTORY);
        std::fs::create_dir_all(directory.join(WIKI_DIRECTORY)).unwrap();
        std::fs::write(
            directory.join(MANIFEST_FILE),
            format!(
                "schema_version: 2\nproject_id: {}\nwiki_id: {}\nname: test\n",
                Uuid::new_v4(),
                Uuid::new_v4()
            ),
        )
        .unwrap();
        std::fs::write(
            directory.join(WIKI_DIRECTORY).join("index.md"),
            "---\nokf_version: \"0.2\"\n---\n\n# test\n",
        )
        .unwrap();
        assert!(inspect_project_memory(&root).is_err());

        std::fs::write(
            directory.join(MANIFEST_FILE),
            format!(
                "schema_version: 1\nproject_id: {}\nwiki_id: {}\nname: test\nextra: no\n",
                Uuid::new_v4(),
                Uuid::new_v4()
            ),
        )
        .unwrap();
        assert!(inspect_project_memory(&root).is_err());

        std::fs::write(
            directory.join(MANIFEST_FILE),
            format!(
                "schema_version: 1\nproject_id: {}\nwiki_id: {}\nname: test\n#{}",
                Uuid::new_v4(),
                Uuid::new_v4(),
                "x".repeat(PROJECT_MEMORY_MANIFEST_MAX_BYTES)
            ),
        )
        .unwrap();
        assert!(inspect_project_memory(&root).is_err());
    }

    #[test]
    fn deduplicates_and_rejects_initialization_without_writing_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database.clone());

        let first = service.initialize(app_id, &root, "Portable docs").unwrap();
        let second = service.initialize(app_id, &root, "Portable docs").unwrap();
        assert_eq!(first, second);
        let ProjectMemoryOpenResult::AwaitingConfirmation { request_id } = first else {
            panic!("initialization should await confirmation");
        };
        service.reject(request_id).unwrap();

        assert!(!root.join(PROJECT_DIRECTORY).exists());
        assert!(service.pending_requests().unwrap().is_empty());
        assert_eq!(
            database
                .project_memory_request(request_id)
                .unwrap()
                .unwrap()
                .state,
            ProjectMemoryRequestState::Rejected
        );
    }

    #[test]
    fn expired_requests_leave_no_pending_confirmation() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let request = database
            .request_project_memory_confirmation(
                app_id,
                ProjectMemoryRequestKind::Initialize,
                &root,
                Some("Portable docs"),
                None,
                Utc::now() + Duration::milliseconds(20),
            )
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));

        assert!(
            database
                .pending_project_memory_requests()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            database
                .project_memory_request(request.id)
                .unwrap()
                .unwrap()
                .state,
            ProjectMemoryRequestState::Expired
        );
    }

    #[test]
    fn limits_each_application_to_sixteen_pending_roots() {
        let temp = tempfile::tempdir().unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database);
        for index in 0..16 {
            let root = temp.path().join(format!("project-{index}"));
            std::fs::create_dir(&root).unwrap();
            let root = std::fs::canonicalize(root).unwrap();
            assert!(matches!(
                service.initialize(app_id, &root, "Portable docs").unwrap(),
                ProjectMemoryOpenResult::AwaitingConfirmation { .. }
            ));
        }
        let overflow = temp.path().join("overflow");
        std::fs::create_dir(&overflow).unwrap();
        let overflow = std::fs::canonicalize(overflow).unwrap();
        assert!(
            service
                .initialize(app_id, &overflow, "Portable docs")
                .is_err()
        );
        assert_eq!(service.pending_requests().unwrap().len(), 16);
    }

    #[test]
    fn attach_approval_revalidates_the_manifest_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let first = std::fs::canonicalize(first).unwrap();
        let second = std::fs::canonicalize(second).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database.clone());
        initialize_and_approve(&service, app_id, &first);
        copy_directory(
            &first.join(PROJECT_DIRECTORY),
            &second.join(PROJECT_DIRECTORY),
        );
        let request_id = match service.open(app_id, &second).unwrap() {
            ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected result: {other:?}"),
        };
        let manifest_path = second.join(PROJECT_DIRECTORY).join(MANIFEST_FILE);
        let mut manifest: ProjectMemoryManifest =
            serde_yaml::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest.name = "Changed before approval".to_owned();
        std::fs::write(&manifest_path, serde_yaml::to_string(&manifest).unwrap()).unwrap();

        assert!(service.approve(request_id).is_err());
        assert!(
            database
                .project_memory_attachment_for_root(&second)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn identity_changes_fail_closed_and_restore_after_external_resolution() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database.clone());
        let approved = initialize_and_approve(&service, app_id, &root);
        let manifest_path = root.join(PROJECT_DIRECTORY).join(MANIFEST_FILE);
        let original = std::fs::read(&manifest_path).unwrap();
        let mut changed: ProjectMemoryManifest = serde_yaml::from_slice(&original).unwrap();
        changed.wiki_id = Uuid::new_v4();
        std::fs::write(&manifest_path, serde_yaml::to_string(&changed).unwrap()).unwrap();

        assert_eq!(
            service.reconcile(approved.collection.id).unwrap(),
            ProjectMemoryAttachmentState::IdentityConflict
        );
        assert!(service.open(app_id, &root).is_err());
        assert!(
            service
                .search(app_id, approved.collection.id, "portable", 10)
                .unwrap()
                .is_empty()
        );

        std::fs::write(&manifest_path, original).unwrap();
        assert_eq!(
            service.reconcile(approved.collection.id).unwrap(),
            ProjectMemoryAttachmentState::Active
        );
        assert!(matches!(
            service.open(app_id, &root).unwrap(),
            ProjectMemoryOpenResult::Ready { .. }
        ));
    }

    #[test]
    fn missing_bundle_fails_closed_and_recovers_when_restored() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database);
        let approved = initialize_and_approve(&service, app_id, &root);
        let bundle = root.join(PROJECT_DIRECTORY);
        let saved = root.join(".airwiki-saved");
        std::fs::rename(&bundle, &saved).unwrap();

        assert_eq!(
            service.reconcile(approved.collection.id).unwrap(),
            ProjectMemoryAttachmentState::Missing
        );
        assert_eq!(
            service.open(app_id, &root).unwrap(),
            ProjectMemoryOpenResult::NotInitialized
        );

        std::fs::rename(&saved, &bundle).unwrap();
        assert_eq!(
            service.reconcile(approved.collection.id).unwrap(),
            ProjectMemoryAttachmentState::Active
        );
    }

    #[test]
    fn unresolved_markdown_conflicts_fail_closed_until_resolved() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database);
        let approved = initialize_and_approve(&service, app_id, &root);
        let concept = root
            .join(PROJECT_DIRECTORY)
            .join(WIKI_DIRECTORY)
            .join("concepts")
            .join(format!("{}.md", Uuid::new_v4()));
        let resolved = "---\ntype: Decision\ntitle: Cache policy\nstatus: stable\n---\n\nUse bounded caches.\n";
        std::fs::write(
            &concept,
            "---\ntype: Decision\ntitle: Cache policy\nstatus: stable\n---\n\n<<<<<<< HEAD\nUse LRU.\n=======\nUse LFU.\n>>>>>>> feature\n",
        )
        .unwrap();

        assert_eq!(
            service.reconcile(approved.collection.id).unwrap(),
            ProjectMemoryAttachmentState::Invalid
        );
        std::fs::write(&concept, resolved).unwrap();
        assert_eq!(
            service.reconcile(approved.collection.id).unwrap(),
            ProjectMemoryAttachmentState::Active
        );
    }

    #[test]
    fn search_is_scoped_to_active_grants_and_revocation_is_immediate() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database.clone());
        let approved = initialize_and_approve(&service, app_id, &root);
        let concept_id = Uuid::new_v4();
        std::fs::write(
            root.join(PROJECT_DIRECTORY)
                .join(WIKI_DIRECTORY)
                .join("concepts")
                .join(format!("{concept_id}.md")),
            format!(
                "---\nid: {concept_id}\ntype: knowledge\ntitle: Portable decision\nstatus: stable\n---\n\nUse the portable cache contract. {}\n",
                "x".repeat(1_500)
            ),
        )
        .unwrap();
        assert_eq!(
            service.reconcile(approved.collection.id).unwrap(),
            ProjectMemoryAttachmentState::Active
        );

        let matches = service
            .search(app_id, approved.collection.id, "portable", 10)
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].snippet.len(), 1024);

        database
            .set_application_capability_revoked(app_id, true)
            .unwrap();
        assert!(
            service
                .search(app_id, approved.collection.id, "portable", 10)
                .unwrap()
                .is_empty()
        );
        assert!(service.open(app_id, &root).is_err());
    }

    #[test]
    fn two_clones_share_portable_identity_but_use_distinct_local_collections() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let first = std::fs::canonicalize(first).unwrap();
        let second = std::fs::canonicalize(second).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database);

        let first_request = match service
            .initialize(app_id, &first, "Shared project")
            .unwrap()
        {
            ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected result: {other:?}"),
        };
        let first_approval = service.approve(first_request).unwrap();
        copy_directory(
            &first.join(PROJECT_DIRECTORY),
            &second.join(PROJECT_DIRECTORY),
        );
        let second_request = match service.open(app_id, &second).unwrap() {
            ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected result: {other:?}"),
        };
        let second_approval = service.approve(second_request).unwrap();

        assert_ne!(first_approval.collection.id, second_approval.collection.id);
        assert_eq!(
            first_approval.portable_wiki_id,
            second_approval.portable_wiki_id
        );
    }

    #[test]
    fn initialization_approval_rejects_a_bundle_that_appeared_after_the_request() {
        let temp = tempfile::tempdir().unwrap();
        let requested_root = temp.path().join("requested");
        let external_root = temp.path().join("external");
        std::fs::create_dir(&requested_root).unwrap();
        std::fs::create_dir(&external_root).unwrap();
        let requested_root = std::fs::canonicalize(requested_root).unwrap();
        let external_root = std::fs::canonicalize(external_root).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database.clone());
        let original_request = match service
            .initialize(app_id, &requested_root, "Portable docs")
            .unwrap()
        {
            ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected result: {other:?}"),
        };
        service
            .initialize_native(&external_root, "Portable docs")
            .unwrap();
        copy_directory(
            &external_root.join(PROJECT_DIRECTORY),
            &requested_root.join(PROJECT_DIRECTORY),
        );

        assert!(service.approve(original_request).is_err());
        let replacement_request = match service
            .initialize(app_id, &requested_root, "Portable docs")
            .unwrap()
        {
            ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected result: {other:?}"),
        };

        assert_ne!(original_request, replacement_request);
        assert_eq!(
            database
                .project_memory_request(original_request)
                .unwrap()
                .unwrap()
                .state,
            ProjectMemoryRequestState::Rejected
        );
        let replacement = database
            .project_memory_request(replacement_request)
            .unwrap()
            .unwrap();
        assert_eq!(replacement.kind, ProjectMemoryRequestKind::Attach);
        assert!(replacement.manifest_fingerprint.is_some());
    }

    #[test]
    fn revocation_and_detach_cancel_pending_attachment_requests() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let database = Database::in_memory().unwrap();
        let owner = application(&database);
        let first_reader = application(&database);
        let second_reader = application(&database);
        let service = ProjectMemoryService::new(database.clone());
        let approved = initialize_and_approve(&service, owner, &root);
        let revoked_request = match service.open(first_reader, &root).unwrap() {
            ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected result: {other:?}"),
        };
        database
            .set_application_capability_revoked(first_reader, true)
            .unwrap();
        assert!(service.approve(revoked_request).is_err());
        assert_eq!(
            database
                .project_memory_request(revoked_request)
                .unwrap()
                .unwrap()
                .state,
            ProjectMemoryRequestState::Rejected
        );

        let detached_request = match service.open(second_reader, &root).unwrap() {
            ProjectMemoryOpenResult::AwaitingConfirmation { request_id } => request_id,
            other => panic!("unexpected result: {other:?}"),
        };
        service.detach(approved.collection.id).unwrap();

        assert_eq!(
            database
                .project_memory_request(detached_request)
                .unwrap()
                .unwrap()
                .state,
            ProjectMemoryRequestState::Rejected
        );
        assert!(root.join(PROJECT_DIRECTORY).is_dir());
    }

    #[test]
    fn watcher_gate_withdraws_projection_until_reconciliation_succeeds() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database.clone());
        let approved = initialize_and_approve(&service, app_id, &root);
        let concept_id = Uuid::new_v4();
        std::fs::write(
            approved
                .collection
                .wiki_folder
                .join("concepts")
                .join(format!("{concept_id}.md")),
            format!(
                "---\nid: {concept_id}\ntype: Decision\ntitle: Watcher gate\nstatus: stable\n---\n\nKeep the projection closed until the watcher starts.\n"
            ),
        )
        .unwrap();
        service.reconcile(approved.collection.id).unwrap();
        assert_eq!(
            database
                .list_okf_concept_projection(approved.collection.id)
                .unwrap()
                .len(),
            1
        );

        service.withhold_active_until_watchers_start().unwrap();
        assert_eq!(
            database
                .project_memory_attachment(approved.collection.id)
                .unwrap()
                .unwrap()
                .state,
            ProjectMemoryAttachmentState::Invalid
        );
        assert!(
            database
                .list_okf_concept_projection(approved.collection.id)
                .unwrap()
                .is_empty()
        );

        service.reconcile(approved.collection.id).unwrap();
        assert_eq!(
            database
                .list_okf_concept_projection(approved.collection.id)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn network_eligibility_rejects_an_invalid_project_with_a_stale_projection() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let database = Database::in_memory().unwrap();
        let app_id = application(&database);
        let service = ProjectMemoryService::new(database.clone());
        let approved = initialize_and_approve(&service, app_id, &root);
        let concept_id = Uuid::new_v4();
        std::fs::write(
            approved
                .collection
                .wiki_folder
                .join("concepts")
                .join(format!("{concept_id}.md")),
            format!(
                "---\nid: {concept_id}\ntype: Decision\ntitle: Private while invalid\nstatus: stable\n---\n\nNever disclose a stale project projection.\n"
            ),
        )
        .unwrap();
        service.reconcile(approved.collection.id).unwrap();
        let mut public = airwiki_types::CollectionPolicy::local_only();
        public.internet_public = true;
        database
            .update_collection_policy(approved.collection.id, public)
            .unwrap();
        service
            .withhold_watcher_unavailable(approved.collection.id)
            .unwrap();
        let imported =
            OkfImportValidator::validate_directory(&approved.collection.wiki_folder).unwrap();
        database
            .replace_okf_concept_projection(approved.collection.id, &imported.concepts)
            .unwrap();
        assert_eq!(
            database
                .list_okf_concept_projection(approved.collection.id)
                .unwrap()
                .len(),
            1
        );

        assert!(
            database
                .publicly_searchable_collections(&[approved.collection.id])
                .unwrap()
                .is_empty()
        );
        assert!(
            database
                .public_concept_page("publisher", approved.collection.id, None, 10)
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_project_roots() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        let alias = temp.path().join("alias");
        std::fs::create_dir(&target).unwrap();
        symlink(&target, &alias).unwrap();
        assert!(canonical_project_root(&alias).is_err());
    }
}
