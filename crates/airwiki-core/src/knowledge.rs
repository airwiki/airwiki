//! Read-only inspection of the OKF artifacts published by AirWiki.
//!
//! This module deliberately does not reuse the strict producer validation in
//! [`crate::okf`]. A viewer must remain useful when a bundle is incomplete,
//! produced on Windows, or contains a newer OKF extension. Structural and
//! reconciliation problems are reported through [`BundleHealthReport`]
//! instead of making the entire bundle unreadable.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use airwiki_types::{CollectionPolicy, DocumentStatus};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveDate, Utc};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::storage::{CollectionRecord, ConceptRecord, Database, SourceDocumentRecord};

/// Hard upper bound for content returned by [`OkfBundleInspector::load_page`].
/// Fingerprints are still calculated over the complete file using streaming
/// I/O, so truncating a preview never weakens stale-view detection.
pub const MAX_KNOWLEDGE_PAGE_BYTES: usize = 1024 * 1024;
const RECONCILIATION_GRACE_SECONDS: i64 = 2;
const TRANSIENT_RECONCILIATION_CODES: &[&str] = &[
    "missing_bundle",
    "missing_index",
    "missing_log",
    "missing_concept",
    "metadata_mismatch",
    "index_missing_concept",
    "stale_index_metadata",
    "stale_log_revision",
    "log_missing_publication",
];
const AUTOMATIC_DERIVED_RECOVERY_CODES: &[&str] = &[
    "broken_index_link",
    "index_missing_concept",
    "invalid_index_structure",
    "missing_index",
    "stale_index_metadata",
];
const GUIDED_CONTENT_RECOVERY_CODES: &[&str] = &[
    "broken_link",
    "invalid_frontmatter",
    "invalid_utf8",
    "metadata_mismatch",
    "missing_airwiki_profile",
    "missing_concept",
    "missing_frontmatter",
    "missing_type",
    "unexpected_concept",
    "unsafe_link",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KnowledgePageId {
    Index,
    Log,
    Concept(Uuid),
}

impl KnowledgePageId {
    pub fn relative_path(self) -> String {
        match self {
            Self::Index => "index.md".to_owned(),
            Self::Log => "log.md".to_owned(),
            Self::Concept(id) => format!("concepts/{id}.md"),
        }
    }

    fn path_below(self, root: &Path) -> PathBuf {
        match self {
            Self::Index => root.join("index.md"),
            Self::Log => root.join("log.md"),
            Self::Concept(id) => root.join("concepts").join(format!("{id}.md")),
        }
    }
}

impl fmt::Display for KnowledgePageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.relative_path())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnowledgeBundleState {
    Empty,
    Ready,
    Updating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HealthSeverity {
    Info,
    Warning,
    Error,
}

/// Recovery boundary for one health finding.
///
/// Unknown or structurally unsafe findings default to [`Self::ManualIntervention`].
/// This keeps UI and repair planning fail-closed when a newer inspector code is
/// introduced without an explicitly verified recovery path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthRecovery {
    /// AirWiki may rebuild a deterministic artifact from a coherent snapshot.
    AutomaticDerived,
    /// AirWiki may prepare a preview that still requires explicit confirmation.
    GuidedContent,
    /// Append-only publication history requires a separate human decision.
    ManualHistory,
    /// Filesystem, storage, or otherwise ambiguous state needs manual recovery.
    ManualIntervention,
    /// The finding is diagnostic and does not require a recovery action.
    Informational,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeLinkDisposition {
    Internal(KnowledgePageId),
    External,
    Broken,
    Unsafe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeLinkView {
    pub source: KnowledgePageId,
    pub label: String,
    pub raw_target: String,
    pub disposition: KnowledgeLinkDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeConceptView {
    pub id: Uuid,
    pub relative_path: String,
    pub concept_type: String,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub resource: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub revision: Option<u32>,
    pub source_sha256: Option<String>,
    pub language: Option<String>,
    pub generator_model: Option<String>,
    pub reviewed_at: Option<DateTime<Utc>>,
    /// Flattened OKF/frontmatter fields outside the v0.1 and AirWiki
    /// profile understood by this viewer. They are preserved for display but
    /// never interpreted as permissions or publication state.
    pub extensions: BTreeMap<String, String>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeBundleView {
    pub collection_id: Uuid,
    pub collection_name: String,
    pub collection_policy: CollectionPolicy,
    pub fingerprint: String,
    pub state: KnowledgeBundleState,
    pub index_fingerprint: Option<String>,
    pub log_fingerprint: Option<String>,
    pub concepts: Vec<KnowledgeConceptView>,
    pub links: Vec<KnowledgeLinkView>,
    pub backlinks: BTreeMap<KnowledgePageId, Vec<KnowledgePageId>>,
    pub health: BundleHealthReport,
}

impl KnowledgeBundleView {
    /// Transient placeholder for the desktop while a background inspection is
    /// queued. It contains no filesystem-derived data and is never considered
    /// a valid bundle snapshot.
    pub fn updating(collection_id: Uuid, collection_name: impl Into<String>) -> Self {
        Self {
            collection_id,
            collection_name: collection_name.into(),
            collection_policy: CollectionPolicy::default(),
            fingerprint: String::new(),
            state: KnowledgeBundleState::Updating,
            index_fingerprint: None,
            log_fingerprint: None,
            concepts: Vec::new(),
            links: Vec::new(),
            backlinks: BTreeMap::new(),
            health: BundleHealthReport::empty(),
        }
    }

    pub fn page_fingerprint(&self, page_id: KnowledgePageId) -> Option<&str> {
        match page_id {
            KnowledgePageId::Index => self.index_fingerprint.as_deref(),
            KnowledgePageId::Log => self.log_fingerprint.as_deref(),
            KnowledgePageId::Concept(id) => self
                .concepts
                .iter()
                .find(|concept| concept.id == id)
                .map(|concept| concept.fingerprint.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgePageView {
    pub collection_id: Uuid,
    pub page_id: KnowledgePageId,
    pub title: String,
    pub fingerprint: String,
    pub body_markdown: String,
    pub metadata: Vec<(String, String)>,
    pub outgoing_links: Vec<KnowledgeLinkView>,
    pub backlinks: Vec<KnowledgePageId>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleHealthReport {
    pub checked_at: DateTime<Utc>,
    pub total_concepts: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub issues: Vec<HealthIssue>,
}

impl BundleHealthReport {
    fn empty() -> Self {
        Self {
            checked_at: Utc::now(),
            total_concepts: 0,
            error_count: 0,
            warning_count: 0,
            issues: Vec::new(),
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.error_count == 0
    }

    fn push(&mut self, issue: HealthIssue) {
        match issue.severity {
            HealthSeverity::Error => self.error_count += 1,
            HealthSeverity::Warning => self.warning_count += 1,
            HealthSeverity::Info => {}
        }
        self.issues.push(issue);
    }

    fn finalize(&mut self) {
        self.issues.sort_by(|left, right| {
            left.severity
                .cmp(&right.severity)
                .reverse()
                .then_with(|| left.page.cmp(&right.page))
                .then_with(|| left.code.cmp(&right.code))
                .then_with(|| left.message.cmp(&right.message))
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthIssue {
    pub severity: HealthSeverity,
    pub code: String,
    pub page: Option<KnowledgePageId>,
    pub message: String,
}

impl HealthIssue {
    fn new(
        severity: HealthSeverity,
        code: impl Into<String>,
        page: Option<KnowledgePageId>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            page,
            message: message.into(),
        }
    }

    /// Returns the only recovery path currently verified for this finding.
    pub fn recovery(&self) -> HealthRecovery {
        if self.severity == HealthSeverity::Info {
            return HealthRecovery::Informational;
        }
        if self.page == Some(KnowledgePageId::Index)
            && AUTOMATIC_DERIVED_RECOVERY_CODES.contains(&self.code.as_str())
        {
            return HealthRecovery::AutomaticDerived;
        }
        if self.page == Some(KnowledgePageId::Log) {
            return HealthRecovery::ManualHistory;
        }
        if matches!(self.page, Some(KnowledgePageId::Concept(_)))
            && GUIDED_CONTENT_RECOVERY_CODES.contains(&self.code.as_str())
        {
            return HealthRecovery::GuidedContent;
        }
        HealthRecovery::ManualIntervention
    }
}

/// Read-only facade over the database and its managed OKF bundle folders.
#[derive(Debug, Clone)]
pub struct OkfBundleInspector {
    database: Database,
}

#[derive(Debug)]
struct DatabaseBundleSnapshot {
    collection: CollectionRecord,
    published: Vec<ConceptRecord>,
    sources: BTreeMap<Uuid, Option<SourceDocumentRecord>>,
    publication_pending: bool,
    fingerprint: String,
}

impl OkfBundleInspector {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub fn inspect_bundle(&self, collection_id: Uuid) -> Result<KnowledgeBundleView> {
        let before = self.database_snapshot(collection_id)?;
        let files_before = bundle_tree_fingerprint(&before.collection.wiki_folder);
        let mut view = self.inspect_collection(&before)?;
        let files_after = bundle_tree_fingerprint(&before.collection.wiki_folder);
        let after = self.database_snapshot(collection_id)?;

        if before.publication_pending || after.publication_pending {
            mark_bundle_updating(
                &mut view,
                "publication_pending",
                "Una publicación aprobada se está materializando; la vista se volverá a cargar.",
            );
        }

        if before.fingerprint != after.fingerprint {
            mark_bundle_updating(
                &mut view,
                "database_changed_during_inspection",
                "SQLite cambió mientras se inspeccionaba el bundle; la vista se volverá a cargar.",
            );
        }
        if files_before != files_after {
            mark_bundle_updating(
                &mut view,
                "bundle_changed_during_inspection",
                "Los archivos OKF cambiaron mientras se inspeccionaba el bundle; la vista se volverá a cargar.",
            );
        }
        Ok(view)
    }

    pub fn load_page(
        &self,
        collection_id: Uuid,
        page_id: KnowledgePageId,
        expected_fingerprint: Option<&str>,
        max_bytes: usize,
    ) -> Result<KnowledgePageView> {
        if max_bytes == 0 {
            bail!("El límite de la página de conocimiento debe ser mayor que cero");
        }
        let database_before = self.database_snapshot(collection_id)?;
        Self::authorize_page(&database_before, page_id)?;

        // Inspect first so backlinks and internal-link resolution are derived
        // from the same fail-closed set of database-published concepts.
        let bundle = self.inspect_bundle(collection_id)?;
        if bundle.state == KnowledgeBundleState::Updating {
            bail!("El bundle OKF se está actualizando; vuelva a cargarlo antes de abrir la página");
        }
        let database_after_inspection = self.database_snapshot(collection_id)?;
        if database_before.fingerprint != database_after_inspection.fingerprint {
            bail!("La autorización o publicación cambió mientras se cargaba la página");
        }
        let inspected_fingerprint = bundle
            .page_fingerprint(page_id)
            .with_context(|| format!("La página de conocimiento {page_id} no está disponible"))?;
        if expected_fingerprint.is_some_and(|expected| expected != inspected_fingerprint) {
            bail!("La página de conocimiento cambió desde que se cargó el bundle");
        }

        let limit = max_bytes.min(MAX_KNOWLEDGE_PAGE_BYTES);
        let page_path = page_id.path_below(&database_before.collection.wiki_folder);
        let snapshot = read_page_snapshot(&page_path, limit)?;
        if snapshot.fingerprint != inspected_fingerprint {
            bail!("La página de conocimiento cambió mientras se estaba cargando");
        }
        let parsed = parse_markdown(&snapshot.markdown);
        let context = LinkContext::from_bundle(&bundle);
        let outgoing_links = resolve_page_links(page_id, &parsed.body, &context);
        let title = page_title(page_id, &parsed, &bundle);

        // Authorization and both backing snapshots are checked again after
        // parsing. This prevents a withdrawal/republication race from
        // returning bytes that were authorized only at the beginning.
        let database_after = self.database_snapshot(collection_id)?;
        Self::authorize_page(&database_after, page_id)?;
        if database_before.fingerprint != database_after.fingerprint {
            bail!("La autorización o publicación cambió mientras se cargaba la página");
        }
        let final_fingerprint = fingerprint_regular_file(&page_path)?;
        if final_fingerprint != snapshot.fingerprint {
            bail!("La página de conocimiento cambió mientras se estaba cargando");
        }

        Ok(KnowledgePageView {
            collection_id,
            page_id,
            title,
            fingerprint: snapshot.fingerprint,
            body_markdown: parsed.body,
            metadata: parsed.metadata,
            outgoing_links,
            backlinks: bundle.backlinks.get(&page_id).cloned().unwrap_or_default(),
            truncated: snapshot.truncated,
        })
    }

    fn authorize_page(snapshot: &DatabaseBundleSnapshot, page_id: KnowledgePageId) -> Result<()> {
        if let KnowledgePageId::Concept(concept_id) = page_id {
            let concept = snapshot
                .published
                .iter()
                .find(|concept| concept.id == concept_id)
                .with_context(|| format!("El concepto {concept_id} no existe"))?;
            if concept.collection_id != snapshot.collection.id
                || concept.status != DocumentStatus::Published
            {
                bail!("El concepto no está publicado en la colección solicitada");
            }
            let source = snapshot
                .sources
                .get(&concept.source_document_id)
                .and_then(Option::as_ref)
                .context("El concepto publicado perdió su documento fuente")?;
            if source.collection_id != snapshot.collection.id
                || source.status != DocumentStatus::Published
            {
                bail!("El documento fuente del concepto no está publicado");
            }
        }
        Ok(())
    }

    fn database_snapshot(&self, collection_id: Uuid) -> Result<DatabaseBundleSnapshot> {
        let collection = self
            .database
            .collection(collection_id)?
            .with_context(|| format!("La colección {collection_id} no existe"))?;
        let published = self.database.list_published_concepts(collection_id)?;
        let mut sources = BTreeMap::new();
        for concept in &published {
            sources
                .entry(concept.source_document_id)
                .or_insert(self.database.source_document(concept.source_document_id)?);
        }
        let publication_pending = self
            .database
            .collection_has_publication_claim(collection_id)?;
        let fingerprint =
            database_snapshot_fingerprint(&collection, &published, &sources, publication_pending)?;
        Ok(DatabaseBundleSnapshot {
            collection,
            published,
            sources,
            publication_pending,
            fingerprint,
        })
    }

    fn inspect_collection(&self, snapshot: &DatabaseBundleSnapshot) -> Result<KnowledgeBundleView> {
        let collection = &snapshot.collection;
        let published = snapshot.published.as_slice();
        let mut health = BundleHealthReport {
            checked_at: Utc::now(),
            total_concepts: published.len(),
            error_count: 0,
            warning_count: 0,
            issues: Vec::new(),
        };
        let mut inspected_pages = BTreeMap::<KnowledgePageId, InspectedPage>::new();
        let mut fingerprint_entries = BTreeMap::<String, String>::new();

        match fs::symlink_metadata(&collection.wiki_folder) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                health.push(HealthIssue::new(
                    HealthSeverity::Error,
                    "unsafe_bundle_root",
                    None,
                    "La raíz administrada del bundle OKF es un enlace simbólico.",
                ));
                return Ok(finalize_bundle(
                    collection,
                    published,
                    BundleParts::empty(fingerprint_entries, health),
                ));
            }
            Ok(metadata) if !metadata.is_dir() => {
                health.push(HealthIssue::new(
                    HealthSeverity::Error,
                    "invalid_bundle_root",
                    None,
                    "La raíz administrada del bundle OKF no es un directorio.",
                ));
                return Ok(finalize_bundle(
                    collection,
                    published,
                    BundleParts::empty(fingerprint_entries, health),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !published.is_empty() {
                    health.push(HealthIssue::new(
                        HealthSeverity::Error,
                        "missing_bundle",
                        None,
                        "SQLite contiene conceptos publicados, pero falta el bundle OKF.",
                    ));
                }
                return Ok(finalize_bundle(
                    collection,
                    published,
                    BundleParts::empty(fingerprint_entries, health),
                ));
            }
            Err(error) => {
                return Err(error).context("No se pudo inspeccionar la raíz del bundle OKF");
            }
        }

        for page_id in [KnowledgePageId::Index, KnowledgePageId::Log] {
            if let Some(page) = inspect_managed_page(
                &collection.wiki_folder,
                page_id,
                &mut health,
                !published.is_empty(),
            )? {
                fingerprint_entries
                    .insert(page_id.relative_path(), page.snapshot.fingerprint.clone());
                inspected_pages.insert(page_id, page);
            }
        }

        let expected_ids = published
            .iter()
            .map(|concept| concept.id)
            .collect::<BTreeSet<_>>();
        inspect_unexpected_markdown(
            &collection.wiki_folder,
            &expected_ids,
            &mut fingerprint_entries,
            &mut health,
        )?;

        for concept in published {
            let page_id = KnowledgePageId::Concept(concept.id);
            if let Some(page) =
                inspect_managed_page(&collection.wiki_folder, page_id, &mut health, true)?
            {
                fingerprint_entries
                    .insert(page_id.relative_path(), page.snapshot.fingerprint.clone());
                inspected_pages.insert(page_id, page);
            }
        }

        let available_ids = inspected_pages.keys().copied().collect::<BTreeSet<_>>();
        let context = LinkContext::from_pages(&inspected_pages, &available_ids);
        let mut links = Vec::new();
        for (page_id, page) in &inspected_pages {
            links.extend(resolve_page_links(*page_id, &page.parsed.body, &context));
        }
        inspect_link_health(&links, &mut health);
        inspect_index_coverage(published, &links, &inspected_pages, &mut health);
        inspect_reserved_page_shapes(&inspected_pages, &mut health);
        inspect_publication_coherence(
            published,
            &snapshot.sources,
            &links,
            &inspected_pages,
            &mut health,
        );

        let mut concepts = Vec::new();
        for concept in published {
            let page_id = KnowledgePageId::Concept(concept.id);
            let Some(page) = inspected_pages.get(&page_id) else {
                continue;
            };
            let source = snapshot
                .sources
                .get(&concept.source_document_id)
                .and_then(Option::as_ref);
            let view = reconcile_concept(concept, source, page, &mut health);
            concepts.push(view);
        }
        concepts.sort_by(|left, right| {
            left.title
                .to_lowercase()
                .cmp(&right.title.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });

        let backlinks = build_backlinks(&links);
        let index_fingerprint = inspected_pages
            .get(&KnowledgePageId::Index)
            .map(|page| page.snapshot.fingerprint.clone());
        let log_fingerprint = inspected_pages
            .get(&KnowledgePageId::Log)
            .map(|page| page.snapshot.fingerprint.clone());
        Ok(finalize_bundle(
            collection,
            published,
            BundleParts {
                concepts,
                links,
                backlinks,
                index_fingerprint,
                log_fingerprint,
                fingerprint_entries,
                health,
            },
        ))
    }
}

fn mark_bundle_updating(view: &mut KnowledgeBundleView, code: &str, message: &str) {
    view.state = KnowledgeBundleState::Updating;
    if !view.health.issues.iter().any(|issue| issue.code == code) {
        view.health
            .push(HealthIssue::new(HealthSeverity::Info, code, None, message));
        view.health.finalize();
    }
}

fn database_snapshot_fingerprint(
    collection: &CollectionRecord,
    published: &[ConceptRecord],
    sources: &BTreeMap<Uuid, Option<SourceDocumentRecord>>,
    publication_pending: bool,
) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_bytes(&mut hasher, b"airwiki-okf-database-snapshot-v1");
    hash_bytes(&mut hasher, collection.id.as_bytes());
    hash_bytes(&mut hasher, collection.name.as_bytes());
    hash_bytes(
        &mut hasher,
        collection.source_folder.as_os_str().as_encoded_bytes(),
    );
    hash_bytes(
        &mut hasher,
        collection.wiki_folder.as_os_str().as_encoded_bytes(),
    );
    hash_bool(&mut hasher, collection.policy.local_only);
    hash_bool(&mut hasher, collection.policy.peer_shareable);
    hash_bool(&mut hasher, collection.policy.allow_external_ai);
    hash_datetime(&mut hasher, collection.created_at);
    hash_datetime(&mut hasher, collection.updated_at);
    hash_bool(&mut hasher, publication_pending);

    let mut concepts = published.iter().collect::<Vec<_>>();
    concepts.sort_by_key(|concept| concept.id);
    for concept in concepts {
        hash_bytes(&mut hasher, concept.id.as_bytes());
        hash_bytes(&mut hasher, concept.source_document_id.as_bytes());
        hash_bytes(&mut hasher, concept.collection_id.as_bytes());
        hash_bytes(
            &mut hasher,
            &serde_json::to_vec(&concept.draft)
                .context("No se pudo capturar el borrador publicado")?,
        );
        hash_bytes(&mut hasher, concept.logical_resource_uri.as_bytes());
        hash_bytes(&mut hasher, concept.generator_model.as_bytes());
        hash_bytes(&mut hasher, concept.status.to_string().as_bytes());
        hash_optional_datetime(&mut hasher, concept.reviewed_at);
        hash_datetime(&mut hasher, concept.created_at);
        hash_datetime(&mut hasher, concept.updated_at);
    }

    for (source_id, source) in sources {
        hash_bytes(&mut hasher, source_id.as_bytes());
        let Some(source) = source else {
            hash_bytes(&mut hasher, b"missing");
            continue;
        };
        hash_bytes(&mut hasher, source.id.as_bytes());
        hash_bytes(&mut hasher, source.collection_id.as_bytes());
        hash_bytes(
            &mut hasher,
            source.source_path.as_os_str().as_encoded_bytes(),
        );
        hash_bytes(&mut hasher, source.source_sha256.as_bytes());
        hash_bytes(&mut hasher, source.source_format.as_bytes());
        hash_bytes(&mut hasher, &source.byte_size.to_be_bytes());
        hash_bytes(&mut hasher, &source.page_count.to_be_bytes());
        hash_bytes(&mut hasher, &source.character_count.to_be_bytes());
        hash_bytes(&mut hasher, source.status.to_string().as_bytes());
        hash_bytes(&mut hasher, &source.revision.to_be_bytes());
        hash_optional_uuid(&mut hasher, source.concept_id);
        hash_optional_string(&mut hasher, source.last_error.as_deref());
        hash_datetime(&mut hasher, source.discovered_at);
        hash_datetime(&mut hasher, source.updated_at);
        hash_optional_datetime(&mut hasher, source.deleted_at);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn hash_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value);
}

fn hash_bool(hasher: &mut Sha256, value: bool) {
    hash_bytes(hasher, &[u8::from(value)]);
}

fn hash_datetime(hasher: &mut Sha256, value: DateTime<Utc>) {
    hash_bytes(hasher, value.to_rfc3339().as_bytes());
}

fn hash_optional_datetime(hasher: &mut Sha256, value: Option<DateTime<Utc>>) {
    if let Some(value) = value {
        hash_bool(hasher, true);
        hash_datetime(hasher, value);
    } else {
        hash_bool(hasher, false);
    }
}

fn hash_optional_uuid(hasher: &mut Sha256, value: Option<Uuid>) {
    if let Some(value) = value {
        hash_bool(hasher, true);
        hash_bytes(hasher, value.as_bytes());
    } else {
        hash_bool(hasher, false);
    }
}

fn hash_optional_string(hasher: &mut Sha256, value: Option<&str>) {
    if let Some(value) = value {
        hash_bool(hasher, true);
        hash_bytes(hasher, value.as_bytes());
    } else {
        hash_bool(hasher, false);
    }
}

/// Captures every non-hidden Markdown/symlink entry the inspector can observe.
/// Errors become deterministic markers so a persistently damaged bundle still
/// yields a diagnostic view, while a changing tree produces a different hash.
fn bundle_tree_fingerprint(root: &Path) -> String {
    let mut hasher = Sha256::new();
    hash_bytes(&mut hasher, b"airwiki-okf-filesystem-snapshot-v1");
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            hash_bytes(&mut hasher, b"root-symlink");
            return hex::encode(hasher.finalize());
        }
        Ok(metadata) if !metadata.is_dir() => {
            hash_bytes(&mut hasher, b"root-not-directory");
            return hex::encode(hasher.finalize());
        }
        Ok(_) => {}
        Err(error) => {
            hash_bytes(&mut hasher, b"root-error");
            hash_bytes(&mut hasher, error.kind().to_string().as_bytes());
            return hex::encode(hasher.finalize());
        }
    }

    for entry in WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.')
        })
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                hash_bytes(&mut hasher, b"walk-error");
                hash_bytes(&mut hasher, error.to_string().as_bytes());
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map(relative_path_text)
            .unwrap_or_default();
        if entry.file_type().is_symlink() {
            hash_bytes(&mut hasher, b"symlink");
            hash_bytes(&mut hasher, relative.as_bytes());
            continue;
        }
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("md")
        {
            continue;
        }
        hash_bytes(&mut hasher, relative.as_bytes());
        match fingerprint_regular_file(entry.path()) {
            Ok(fingerprint) => hash_bytes(&mut hasher, fingerprint.as_bytes()),
            Err(error) => {
                hash_bytes(&mut hasher, b"file-error");
                hash_bytes(&mut hasher, error.to_string().as_bytes());
            }
        }
    }
    hex::encode(hasher.finalize())
}

struct BundleParts {
    concepts: Vec<KnowledgeConceptView>,
    links: Vec<KnowledgeLinkView>,
    backlinks: BTreeMap<KnowledgePageId, Vec<KnowledgePageId>>,
    index_fingerprint: Option<String>,
    log_fingerprint: Option<String>,
    fingerprint_entries: BTreeMap<String, String>,
    health: BundleHealthReport,
}

impl BundleParts {
    fn empty(fingerprint_entries: BTreeMap<String, String>, health: BundleHealthReport) -> Self {
        Self {
            concepts: Vec::new(),
            links: Vec::new(),
            backlinks: BTreeMap::new(),
            index_fingerprint: None,
            log_fingerprint: None,
            fingerprint_entries,
            health,
        }
    }
}

fn finalize_bundle(
    collection: &CollectionRecord,
    published: &[ConceptRecord],
    parts: BundleParts,
) -> KnowledgeBundleView {
    let BundleParts {
        concepts,
        links,
        backlinks,
        index_fingerprint,
        log_fingerprint,
        fingerprint_entries,
        mut health,
    } = parts;
    let mut hasher = Sha256::new();
    hasher.update(b"airwiki-okf-inspector-v1\0");
    hasher.update(collection.id.as_bytes());
    hasher.update(collection.name.as_bytes());
    hasher.update([
        u8::from(collection.policy.local_only),
        u8::from(collection.policy.peer_shareable),
        u8::from(collection.policy.allow_external_ai),
    ]);
    for concept in published {
        hasher.update(b"\0db\0");
        hasher.update(concept.id.as_bytes());
        hasher.update(concept.updated_at.to_rfc3339().as_bytes());
    }
    for (path, fingerprint) in fingerprint_entries {
        hasher.update(b"\0file\0");
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(fingerprint.as_bytes());
    }
    let state = if published.is_empty() {
        KnowledgeBundleState::Empty
    } else if reconciliation_is_transient(published, &health) {
        KnowledgeBundleState::Updating
    } else {
        KnowledgeBundleState::Ready
    };
    health.finalize();
    KnowledgeBundleView {
        collection_id: collection.id,
        collection_name: collection.name.clone(),
        collection_policy: collection.policy,
        fingerprint: hex::encode(hasher.finalize()),
        state,
        index_fingerprint,
        log_fingerprint,
        concepts,
        links,
        backlinks,
        health,
    }
}

fn reconciliation_is_transient(published: &[ConceptRecord], health: &BundleHealthReport) -> bool {
    let cutoff = health.checked_at - chrono::TimeDelta::seconds(RECONCILIATION_GRACE_SECONDS);
    health.issues.iter().any(|issue| {
        if !TRANSIENT_RECONCILIATION_CODES.contains(&issue.code.as_str()) {
            return false;
        }
        match issue.page {
            Some(KnowledgePageId::Concept(id)) => published
                .iter()
                .any(|concept| concept.id == id && concept.updated_at >= cutoff),
            Some(KnowledgePageId::Index | KnowledgePageId::Log) | None => {
                published.iter().any(|concept| concept.updated_at >= cutoff)
            }
        }
    })
}

#[derive(Debug)]
struct InspectedPage {
    snapshot: PageSnapshot,
    parsed: ParsedMarkdown,
}

#[derive(Debug)]
struct PageSnapshot {
    fingerprint: String,
    markdown: String,
    truncated: bool,
    valid_utf8: bool,
    byte_len: u64,
}

#[derive(Debug, Default)]
struct ParsedMarkdown {
    had_frontmatter: bool,
    frontmatter_error: Option<String>,
    yaml: Option<YamlValue>,
    metadata: Vec<(String, String)>,
    body: String,
}

fn inspect_managed_page(
    root: &Path,
    page_id: KnowledgePageId,
    health: &mut BundleHealthReport,
    required: bool,
) -> Result<Option<InspectedPage>> {
    let path = page_id.path_below(root);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if required {
                health.push(HealthIssue::new(
                    HealthSeverity::Error,
                    match page_id {
                        KnowledgePageId::Index => "missing_index",
                        KnowledgePageId::Log => "missing_log",
                        KnowledgePageId::Concept(_) => "missing_concept",
                    },
                    Some(page_id),
                    format!("Falta la página OKF requerida {page_id}."),
                ));
            }
            return Ok(None);
        }
        Err(error) => {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "page_metadata_error",
                Some(page_id),
                format!("No se pudo inspeccionar la metadata de la página OKF: {error}"),
            ));
            return Ok(None);
        }
    };
    if metadata.file_type().is_symlink() {
        health.push(HealthIssue::new(
            HealthSeverity::Error,
            "unsafe_page_symlink",
            Some(page_id),
            "Las páginas OKF administradas no pueden ser enlaces simbólicos.",
        ));
        return Ok(None);
    }
    if !metadata.is_file() {
        health.push(HealthIssue::new(
            HealthSeverity::Error,
            "invalid_page_type",
            Some(page_id),
            "La página OKF administrada no es un archivo regular.",
        ));
        return Ok(None);
    }

    let snapshot = match read_page_snapshot(&path, MAX_KNOWLEDGE_PAGE_BYTES) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "page_read_error",
                Some(page_id),
                format!("No se pudo leer la página OKF: {error:#}"),
            ));
            return Ok(None);
        }
    };
    if snapshot.truncated {
        health.push(HealthIssue::new(
            HealthSeverity::Error,
            "page_too_large",
            Some(page_id),
            format!(
                "La página OKF tiene {} bytes; la inspección limita el contenido a {} bytes.",
                snapshot.byte_len, MAX_KNOWLEDGE_PAGE_BYTES
            ),
        ));
    }
    if !snapshot.valid_utf8 {
        health.push(HealthIssue::new(
            HealthSeverity::Error,
            "invalid_utf8",
            Some(page_id),
            "Las páginas OKF deben contener UTF-8 válido.",
        ));
    }
    let parsed = parse_markdown(&snapshot.markdown);
    if let Some(error) = &parsed.frontmatter_error {
        health.push(HealthIssue::new(
            HealthSeverity::Error,
            "invalid_frontmatter",
            Some(page_id),
            error.clone(),
        ));
    }
    if matches!(page_id, KnowledgePageId::Concept(_)) && !parsed.had_frontmatter {
        health.push(HealthIssue::new(
            HealthSeverity::Error,
            "missing_frontmatter",
            Some(page_id),
            "El concepto OKF no tiene frontmatter YAML.",
        ));
    }
    Ok(Some(InspectedPage { snapshot, parsed }))
}

fn read_page_snapshot(path: &Path, content_limit: usize) -> Result<PageSnapshot> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("No se pudo inspeccionar la página {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("La página no es un archivo regular o es un enlace simbólico");
    }
    let mut file = File::open(path)
        .with_context(|| format!("No se pudo abrir la página {}", path.display()))?;
    let opened_stamp = FileStamp::from_metadata(&file.metadata()?);
    let mut hasher = Sha256::new();
    let mut captured = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(usize::MAX)
            .min(content_limit),
    );
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        let remaining = content_limit.saturating_sub(captured.len());
        if remaining > 0 {
            captured.extend_from_slice(&buffer[..read.min(remaining)]);
        }
    }
    let handle_after = FileStamp::from_metadata(&file.metadata()?);
    let path_after = fs::symlink_metadata(path).with_context(|| {
        format!(
            "La página {} desapareció durante la lectura",
            path.display()
        )
    })?;
    if path_after.file_type().is_symlink()
        || !path_after.is_file()
        || opened_stamp != handle_after
        || opened_stamp != FileStamp::from_metadata(&path_after)
    {
        bail!("La página de conocimiento cambió mientras se estaba leyendo");
    }
    let fingerprint = hex::encode(hasher.finalize());
    if fingerprint_regular_file(path)? != fingerprint {
        bail!("La página de conocimiento cambió mientras se estaba leyendo");
    }
    let truncated = total > u64::try_from(content_limit).unwrap_or(u64::MAX);
    let (text, valid_utf8) = decode_utf8_prefix(&captured, truncated);
    Ok(PageSnapshot {
        fingerprint,
        markdown: normalize_markdown_text(&text),
        truncated,
        valid_utf8,
        byte_len: total,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl FileStamp {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
        }
    }
}

fn fingerprint_regular_file(path: &Path) -> Result<String> {
    let path_before = fs::symlink_metadata(path)
        .with_context(|| format!("No se pudo inspeccionar la página {}", path.display()))?;
    if path_before.file_type().is_symlink() || !path_before.is_file() {
        bail!("La página no es un archivo regular o es un enlace simbólico");
    }
    let mut file = File::open(path)
        .with_context(|| format!("No se pudo abrir la página {}", path.display()))?;
    let opened_stamp = FileStamp::from_metadata(&file.metadata()?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let handle_after = FileStamp::from_metadata(&file.metadata()?);
    let path_after = fs::symlink_metadata(path).with_context(|| {
        format!(
            "La página {} desapareció durante la lectura",
            path.display()
        )
    })?;
    if path_after.file_type().is_symlink()
        || !path_after.is_file()
        || opened_stamp != handle_after
        || opened_stamp != FileStamp::from_metadata(&path_after)
    {
        bail!("La página de conocimiento cambió mientras se estaba leyendo");
    }
    Ok(hex::encode(hasher.finalize()))
}

fn decode_utf8_prefix(bytes: &[u8], truncated: bool) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(value) => (value.to_owned(), true),
        Err(error) if truncated && error.error_len().is_none() => {
            let valid = &bytes[..error.valid_up_to()];
            // SAFETY: `Utf8Error::valid_up_to` guarantees this prefix is UTF-8.
            (String::from_utf8_lossy(valid).into_owned(), true)
        }
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), false),
    }
}

fn normalize_markdown_text(value: &str) -> String {
    value
        .strip_prefix('\u{feff}')
        .unwrap_or(value)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn parse_markdown(markdown: &str) -> ParsedMarkdown {
    let Some(first_end) = markdown.find('\n') else {
        return ParsedMarkdown {
            body: markdown.to_owned(),
            ..ParsedMarkdown::default()
        };
    };
    if markdown[..first_end].trim() != "---" {
        return ParsedMarkdown {
            body: markdown.to_owned(),
            ..ParsedMarkdown::default()
        };
    }

    let yaml_start = first_end + 1;
    let mut line_start = yaml_start;
    let mut closing = None;
    while line_start <= markdown.len() {
        let line_end = markdown[line_start..]
            .find('\n')
            .map_or(markdown.len(), |offset| line_start + offset);
        if markdown[line_start..line_end].trim() == "---" {
            closing = Some((line_start, line_end));
            break;
        }
        if line_end == markdown.len() {
            break;
        }
        line_start = line_end + 1;
    }

    let Some((yaml_end, closing_end)) = closing else {
        return ParsedMarkdown {
            had_frontmatter: true,
            frontmatter_error: Some(
                "El frontmatter YAML no tiene delimitador de cierre.".to_owned(),
            ),
            body: markdown.to_owned(),
            ..ParsedMarkdown::default()
        };
    };
    let body_start = if closing_end < markdown.len() {
        closing_end + 1
    } else {
        closing_end
    };
    let body = markdown[body_start..].to_owned();
    match serde_yaml::from_str::<YamlValue>(&markdown[yaml_start..yaml_end]) {
        Ok(yaml @ YamlValue::Mapping(_)) => ParsedMarkdown {
            had_frontmatter: true,
            frontmatter_error: None,
            metadata: flatten_yaml_metadata(&yaml),
            yaml: Some(yaml),
            body,
        },
        Ok(_) => ParsedMarkdown {
            had_frontmatter: true,
            frontmatter_error: Some("El frontmatter YAML debe ser un mapa.".to_owned()),
            yaml: None,
            metadata: Vec::new(),
            body,
        },
        Err(error) => ParsedMarkdown {
            had_frontmatter: true,
            frontmatter_error: Some(format!(
                "No se pudo interpretar el frontmatter YAML: {error}"
            )),
            yaml: None,
            metadata: Vec::new(),
            body,
        },
    }
}

fn flatten_yaml_metadata(yaml: &YamlValue) -> Vec<(String, String)> {
    let json = match serde_json::to_value(yaml) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let mut output = Vec::new();
    flatten_json_metadata("", &json, &mut output);
    output
}

fn flatten_json_metadata(prefix: &str, value: &JsonValue, output: &mut Vec<(String, String)>) {
    match value {
        JsonValue::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                let next = if prefix.is_empty() {
                    key.to_owned()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_json_metadata(&next, &object[key], output);
            }
        }
        JsonValue::Array(_) => output.push((
            prefix.to_owned(),
            serde_json::to_string(value).unwrap_or_else(|_| "[]".to_owned()),
        )),
        JsonValue::Null => output.push((prefix.to_owned(), "null".to_owned())),
        JsonValue::String(value) => output.push((prefix.to_owned(), value.clone())),
        JsonValue::Bool(value) => output.push((prefix.to_owned(), value.to_string())),
        JsonValue::Number(value) => output.push((prefix.to_owned(), value.to_string())),
    }
}

fn inspect_unexpected_markdown(
    root: &Path,
    expected_concepts: &BTreeSet<Uuid>,
    fingerprints: &mut BTreeMap<String, String>,
    health: &mut BundleHealthReport,
) -> Result<()> {
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.')
        })
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                health.push(HealthIssue::new(
                    HealthSeverity::Warning,
                    "bundle_walk_error",
                    None,
                    format!("No se pudo inspeccionar una entrada del bundle: {error}"),
                ));
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        let relative = match entry.path().strip_prefix(root) {
            Ok(relative) => relative_path_text(relative),
            Err(_) => continue,
        };
        if entry.file_type().is_symlink() {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "unsafe_bundle_symlink",
                None,
                format!("La entrada `{relative}` del bundle es un enlace simbólico."),
            ));
            continue;
        }
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("md")
        {
            continue;
        }
        let expected = relative == "index.md"
            || relative == "log.md"
            || relative
                .strip_prefix("concepts/")
                .and_then(|name| name.strip_suffix(".md"))
                .and_then(|id| Uuid::parse_str(id).ok())
                .is_some_and(|id| expected_concepts.contains(&id));
        if expected {
            continue;
        }
        if let Ok(snapshot) = read_page_snapshot(entry.path(), 1) {
            fingerprints.insert(relative.clone(), snapshot.fingerprint);
        }
        let unexpected_concept_id = relative
            .strip_prefix("concepts/")
            .and_then(|name| name.strip_suffix(".md"))
            .and_then(|id| Uuid::parse_str(id).ok());
        let is_concept = relative.starts_with("concepts/");
        health.push(HealthIssue::new(
            if is_concept {
                HealthSeverity::Error
            } else {
                HealthSeverity::Warning
            },
            if is_concept {
                "unexpected_concept"
            } else {
                "unmanaged_markdown"
            },
            unexpected_concept_id.map(KnowledgePageId::Concept),
            format!("El bundle contiene la página Markdown no administrada `{relative}`."),
        ));
    }
    Ok(())
}

fn relative_path_text(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[derive(Debug, Default)]
struct LinkContext {
    paths: BTreeMap<String, KnowledgePageId>,
    resources: HashMap<String, KnowledgePageId>,
}

impl LinkContext {
    fn from_pages(
        pages: &BTreeMap<KnowledgePageId, InspectedPage>,
        available: &BTreeSet<KnowledgePageId>,
    ) -> Self {
        let mut context = Self::default();
        for page_id in available {
            context.paths.insert(page_id.relative_path(), *page_id);
        }
        for (page_id, page) in pages {
            if matches!(page_id, KnowledgePageId::Concept(_))
                && let Some(resource) = page
                    .parsed
                    .yaml
                    .as_ref()
                    .and_then(|yaml| yaml_string_at(yaml, &["resource"]))
                    .filter(|resource| !resource.trim().is_empty())
            {
                context.resources.insert(resource, *page_id);
            }
        }
        context
    }

    fn from_bundle(bundle: &KnowledgeBundleView) -> Self {
        let mut context = Self::default();
        if bundle.index_fingerprint.is_some() {
            context
                .paths
                .insert("index.md".to_owned(), KnowledgePageId::Index);
        }
        if bundle.log_fingerprint.is_some() {
            context
                .paths
                .insert("log.md".to_owned(), KnowledgePageId::Log);
        }
        for concept in &bundle.concepts {
            let page_id = KnowledgePageId::Concept(concept.id);
            context.paths.insert(concept.relative_path.clone(), page_id);
            if let Some(resource) = &concept.resource {
                context.resources.insert(resource.clone(), page_id);
            }
        }
        context
    }
}

fn resolve_page_links(
    source: KnowledgePageId,
    markdown_body: &str,
    context: &LinkContext,
) -> Vec<KnowledgeLinkView> {
    extract_markdown_links(markdown_body)
        .into_iter()
        .map(|(label, raw_target)| KnowledgeLinkView {
            source,
            disposition: resolve_link_target(source, &raw_target, context),
            label,
            raw_target,
        })
        .collect()
}

fn extract_markdown_links(markdown: &str) -> Vec<(String, String)> {
    let mut output = Vec::new();
    let mut current: Option<(String, String)> = None;
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                current = Some((dest_url.into_string(), String::new()));
            }
            Event::End(TagEnd::Link) => {
                if let Some((target, label)) = current.take() {
                    output.push((label.trim().to_owned(), target));
                }
            }
            Event::Text(text) | Event::Code(text) if current.is_some() => {
                if let Some((_, label)) = &mut current {
                    label.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak if current.is_some() => {
                if let Some((_, label)) = &mut current {
                    label.push(' ');
                }
            }
            _ => {}
        }
    }
    output
}

fn resolve_link_target(
    source: KnowledgePageId,
    raw_target: &str,
    context: &LinkContext,
) -> KnowledgeLinkDisposition {
    let target = raw_target.trim();
    if target.is_empty() || target.starts_with('#') || target.starts_with('?') {
        return KnowledgeLinkDisposition::Internal(source);
    }
    if target.starts_with('/') || target.contains('\\') {
        return KnowledgeLinkDisposition::Unsafe;
    }
    if target.starts_with("urn:airwiki:") {
        let resource = target.split(['#', '?']).next().unwrap_or(target);
        return context
            .resources
            .get(resource)
            .copied()
            .map(KnowledgeLinkDisposition::Internal)
            .unwrap_or(KnowledgeLinkDisposition::Broken);
    }
    if let Some(scheme) = uri_scheme(target) {
        return if ["http", "https", "mailto", "urn"]
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
        {
            KnowledgeLinkDisposition::External
        } else {
            KnowledgeLinkDisposition::Unsafe
        };
    }

    let path_only = target.split(['#', '?']).next().unwrap_or(target);
    if path_only.is_empty() {
        return KnowledgeLinkDisposition::Internal(source);
    }
    let mut source_components = source.relative_path();
    source_components.truncate(source_components.rfind('/').map_or(0, |index| index + 1));
    let mut components = source_components
        .split('/')
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    for component in path_only.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return KnowledgeLinkDisposition::Unsafe;
                }
            }
            value => components.push(value.to_owned()),
        }
    }
    if path_only.ends_with('/') {
        components.push("index.md".to_owned());
    }
    let normalized = components.join("/");
    context
        .paths
        .get(&normalized)
        .copied()
        .map(KnowledgeLinkDisposition::Internal)
        .unwrap_or(KnowledgeLinkDisposition::Broken)
}

fn uri_scheme(target: &str) -> Option<&str> {
    let colon = target.find(':')?;
    let candidate = &target[..colon];
    if candidate.is_empty()
        || !candidate
            .bytes()
            .enumerate()
            .all(|(index, byte)| match (index, byte) {
                (0, byte) => byte.is_ascii_alphabetic(),
                (_, byte) => byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'),
            })
    {
        return None;
    }
    Some(candidate)
}

fn build_backlinks(links: &[KnowledgeLinkView]) -> BTreeMap<KnowledgePageId, Vec<KnowledgePageId>> {
    let mut backlinks = BTreeMap::<KnowledgePageId, BTreeSet<KnowledgePageId>>::new();
    for link in links {
        if let KnowledgeLinkDisposition::Internal(target) = link.disposition {
            backlinks.entry(target).or_default().insert(link.source);
        }
    }
    backlinks
        .into_iter()
        .map(|(target, sources)| (target, sources.into_iter().collect()))
        .collect()
}

fn inspect_link_health(links: &[KnowledgeLinkView], health: &mut BundleHealthReport) {
    for link in links {
        match link.disposition {
            KnowledgeLinkDisposition::Broken => {
                let (severity, code) = match link.source {
                    KnowledgePageId::Index => (HealthSeverity::Error, "broken_index_link"),
                    KnowledgePageId::Log => (HealthSeverity::Info, "historical_broken_link"),
                    KnowledgePageId::Concept(_) => (HealthSeverity::Warning, "broken_link"),
                };
                health.push(HealthIssue::new(
                    severity,
                    code,
                    Some(link.source),
                    format!(
                        "No se puede resolver el destino `{}` del enlace Markdown.",
                        link.raw_target
                    ),
                ));
            }
            KnowledgeLinkDisposition::Unsafe => health.push(HealthIssue::new(
                HealthSeverity::Warning,
                "unsafe_link",
                Some(link.source),
                format!(
                    "No es seguro abrir el destino `{}` del enlace Markdown.",
                    link.raw_target
                ),
            )),
            KnowledgeLinkDisposition::Internal(_) | KnowledgeLinkDisposition::External => {}
        }
    }
}

fn inspect_index_coverage(
    published: &[ConceptRecord],
    links: &[KnowledgeLinkView],
    pages: &BTreeMap<KnowledgePageId, InspectedPage>,
    health: &mut BundleHealthReport,
) {
    if !pages.contains_key(&KnowledgePageId::Index) {
        return;
    }
    let indexed = links
        .iter()
        .filter(|link| link.source == KnowledgePageId::Index)
        .filter_map(|link| match link.disposition {
            KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(id)) => Some(id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for concept in published {
        if pages.contains_key(&KnowledgePageId::Concept(concept.id))
            && !indexed.contains(&concept.id)
        {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "index_missing_concept",
                Some(KnowledgePageId::Index),
                format!("index.md no enumera el concepto publicado {}.", concept.id),
            ));
        }
    }
}

fn inspect_publication_coherence(
    published: &[ConceptRecord],
    sources: &BTreeMap<Uuid, Option<SourceDocumentRecord>>,
    links: &[KnowledgeLinkView],
    pages: &BTreeMap<KnowledgePageId, InspectedPage>,
    health: &mut BundleHealthReport,
) {
    if pages.contains_key(&KnowledgePageId::Index) {
        for concept in published {
            let page_id = KnowledgePageId::Concept(concept.id);
            let Some(page) = pages.get(&page_id) else {
                continue;
            };
            let current_title = page
                .parsed
                .yaml
                .as_ref()
                .and_then(|yaml| yaml_string_at(yaml, &["title"]))
                .unwrap_or_else(|| concept.draft.title.clone());
            if let Some(index_link) = links.iter().find(|link| {
                link.source == KnowledgePageId::Index
                    && link.disposition == KnowledgeLinkDisposition::Internal(page_id)
            }) && index_link.label != current_title
            {
                health.push(HealthIssue::new(
                    HealthSeverity::Error,
                    "stale_index_metadata",
                    Some(KnowledgePageId::Index),
                    format!(
                        "index.md todavía muestra `{}` para el concepto {}, cuya revisión actual se titula `{current_title}`.",
                        index_link.label, concept.id
                    ),
                ));
            }
        }
    }

    let Some(log) = pages.get(&KnowledgePageId::Log) else {
        return;
    };
    let markers = latest_publication_markers(&log.parsed.body);
    for concept in published {
        let page_id = KnowledgePageId::Concept(concept.id);
        let Some(page) = pages.get(&page_id) else {
            continue;
        };
        let source = sources
            .get(&concept.source_document_id)
            .and_then(Option::as_ref);
        let expected_revision = page
            .parsed
            .yaml
            .as_ref()
            .and_then(|yaml| yaml_u32_at(yaml, &["airwiki", "revision"]))
            .or_else(|| source.map(|source| source.revision));
        let expected_hash = page
            .parsed
            .yaml
            .as_ref()
            .and_then(|yaml| yaml_string_at(yaml, &["airwiki", "source_sha256"]))
            .or_else(|| source.map(|source| source.source_sha256.clone()));
        let Some((revision, sha256)) = markers.get(&concept.id) else {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "log_missing_publication",
                Some(KnowledgePageId::Log),
                format!(
                    "log.md no contiene un evento de publicación para el concepto {}.",
                    concept.id
                ),
            ));
            continue;
        };
        if expected_revision.is_some_and(|expected| expected != *revision)
            || expected_hash
                .as_deref()
                .is_some_and(|expected| expected != sha256)
        {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "stale_log_revision",
                Some(KnowledgePageId::Log),
                format!(
                    "log.md registra una revisión anterior del concepto {} (revisión {revision}).",
                    concept.id
                ),
            ));
        }
    }
}

fn latest_publication_markers(markdown: &str) -> HashMap<Uuid, (u32, String)> {
    let mut markers = HashMap::new();
    for line in markdown.lines() {
        let marker_prefix = "<!-- airwiki:event:";
        let Some(start) = line.find(marker_prefix) else {
            continue;
        };
        let marker = &line[start + marker_prefix.len()..];
        let Some(end) = marker.find("-->") else {
            continue;
        };
        let marker = marker[..end].trim();
        let mut fields = marker.split(':');
        let (Some(_action), Some(id), Some(revision), Some(sha256), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            continue;
        };
        let (Ok(id), Ok(revision)) = (Uuid::parse_str(id), revision.parse::<u32>()) else {
            continue;
        };
        markers
            .entry(id)
            .or_insert_with(|| (revision, sha256.to_owned()));
    }
    markers
}

fn inspect_reserved_page_shapes(
    pages: &BTreeMap<KnowledgePageId, InspectedPage>,
    health: &mut BundleHealthReport,
) {
    if let Some(index) = pages.get(&KnowledgePageId::Index) {
        let first = index
            .parsed
            .body
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty());
        if !first.is_some_and(|line| line.starts_with("# ")) {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "invalid_index_structure",
                Some(KnowledgePageId::Index),
                "index.md debe comenzar con un encabezado de nivel 1.",
            ));
        }
    }

    if let Some(log) = pages.get(&KnowledgePageId::Log) {
        if log.parsed.had_frontmatter {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "log_frontmatter_not_allowed",
                Some(KnowledgePageId::Log),
                "log.md no puede contener frontmatter.",
            ));
        }
        let mut lines = log
            .parsed
            .body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());
        if lines.next() != Some("# Directory Update Log") {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "invalid_log_structure",
                Some(KnowledgePageId::Log),
                "log.md debe comenzar con `# Directory Update Log`.",
            ));
            return;
        }
        let mut previous_date = None;
        let mut current_date = None;
        for line in lines {
            if let Some(raw_date) = line.strip_prefix("## ") {
                let date = match NaiveDate::parse_from_str(raw_date, "%Y-%m-%d") {
                    Ok(date) => date,
                    Err(_) => {
                        health.push(HealthIssue::new(
                            HealthSeverity::Error,
                            "invalid_log_date",
                            Some(KnowledgePageId::Log),
                            "Los grupos de log.md deben usar fechas YYYY-MM-DD.",
                        ));
                        current_date = None;
                        continue;
                    }
                };
                if previous_date.is_some_and(|previous| date >= previous) {
                    health.push(HealthIssue::new(
                        HealthSeverity::Error,
                        "unordered_log_dates",
                        Some(KnowledgePageId::Log),
                        "Los grupos de log.md deben ser únicos y estar del más nuevo al más antiguo.",
                    ));
                }
                previous_date = Some(date);
                current_date = Some(date);
            } else if !line.starts_with("* ") || current_date.is_none() {
                health.push(HealthIssue::new(
                    HealthSeverity::Error,
                    "invalid_log_entry",
                    Some(KnowledgePageId::Log),
                    "Cada entrada de log.md debe ser un ítem dentro de un grupo de fecha.",
                ));
            }
        }
    }
}

fn reconcile_concept(
    concept: &ConceptRecord,
    source: Option<&SourceDocumentRecord>,
    page: &InspectedPage,
    health: &mut BundleHealthReport,
) -> KnowledgeConceptView {
    let page_id = KnowledgePageId::Concept(concept.id);
    let expected_type = concept.draft.concept_type.to_string();
    let yaml = page.parsed.yaml.as_ref();
    let concept_type = yaml.and_then(|value| yaml_string_at(value, &["type"]));
    let title = yaml.and_then(|value| yaml_string_at(value, &["title"]));
    let description = yaml.and_then(|value| yaml_string_at(value, &["description"]));
    let tags = yaml.and_then(|value| yaml_strings_at(value, &["tags"]));
    let resource = yaml.and_then(|value| yaml_string_at(value, &["resource"]));
    let timestamp = yaml.and_then(|value| yaml_datetime_at(value, &["timestamp"]));
    let profile_id = yaml.and_then(|value| yaml_uuid_at(value, &["airwiki", "id"]));
    let profile_collection =
        yaml.and_then(|value| yaml_uuid_at(value, &["airwiki", "collection_id"]));
    let profile_version =
        yaml.and_then(|value| yaml_u32_at(value, &["airwiki", "profile_version"]));
    let revision = yaml.and_then(|value| yaml_u32_at(value, &["airwiki", "revision"]));
    let source_sha256 = yaml.and_then(|value| yaml_string_at(value, &["airwiki", "source_sha256"]));
    let language = yaml.and_then(|value| yaml_string_at(value, &["airwiki", "language"]));
    let status = yaml.and_then(|value| yaml_string_at(value, &["airwiki", "status"]));
    let generator_model =
        yaml.and_then(|value| yaml_string_at(value, &["airwiki", "generator_model"]));
    let reviewed_at = yaml.and_then(|value| yaml_datetime_at(value, &["airwiki", "reviewed_at"]));
    let extensions = concept_extensions(&page.parsed.metadata);

    if let Some(yaml) = yaml {
        compare_field(
            health,
            page_id,
            "type",
            concept_type.as_deref(),
            Some(expected_type.as_str()),
        );
        compare_field(
            health,
            page_id,
            "title",
            title.as_deref(),
            Some(concept.draft.title.as_str()),
        );
        compare_field(
            health,
            page_id,
            "description",
            description.as_deref(),
            Some(concept.draft.description.as_str()),
        );
        compare_field(
            health,
            page_id,
            "resource",
            resource.as_deref(),
            Some(concept.logical_resource_uri.as_str()),
        );
        compare_field(
            health,
            page_id,
            "airwiki.id",
            profile_id.as_ref(),
            Some(&concept.id),
        );
        compare_field(
            health,
            page_id,
            "airwiki.collection_id",
            profile_collection.as_ref(),
            Some(&concept.collection_id),
        );
        compare_field(
            health,
            page_id,
            "airwiki.profile_version",
            profile_version.as_ref(),
            Some(&1_u32),
        );
        compare_field(
            health,
            page_id,
            "airwiki.language",
            language.as_deref(),
            Some(concept.draft.language.as_str()),
        );
        compare_field(
            health,
            page_id,
            "airwiki.status",
            status.as_deref(),
            Some("published"),
        );
        compare_field(
            health,
            page_id,
            "airwiki.generator_model",
            generator_model.as_deref(),
            Some(concept.generator_model.as_str()),
        );
        compare_concept_timestamp(health, page_id, timestamp.as_ref(), concept);
        compare_field(
            health,
            page_id,
            "airwiki.reviewed_at",
            reviewed_at.as_ref(),
            concept.reviewed_at.as_ref(),
        );
        compare_field(
            health,
            page_id,
            "tags",
            tags.as_ref(),
            Some(&concept.draft.tags),
        );
        if concept_type.as_deref().is_none_or(str::is_empty) {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "missing_type",
                Some(page_id),
                "El frontmatter del concepto no contiene un campo `type` no vacío.",
            ));
        }
        if yaml_at(yaml, &["airwiki"]).is_none() {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "missing_airwiki_profile",
                Some(page_id),
                "El concepto publicado no contiene el perfil `airwiki`.",
            ));
        }
    }

    if let Some(source) = source {
        compare_field(
            health,
            page_id,
            "airwiki.revision",
            revision.as_ref(),
            Some(&source.revision),
        );
        compare_field(
            health,
            page_id,
            "airwiki.source_sha256",
            source_sha256.as_deref(),
            Some(source.source_sha256.as_str()),
        );
        if source.status != DocumentStatus::Published {
            health.push(HealthIssue::new(
                HealthSeverity::Error,
                "source_not_published",
                Some(page_id),
                "SQLite marca el concepto como publicado pero no su documento fuente.",
            ));
        }
    } else {
        health.push(HealthIssue::new(
            HealthSeverity::Error,
            "missing_source_record",
            Some(page_id),
            "El concepto publicado perdió su documento fuente en SQLite.",
        ));
    }

    KnowledgeConceptView {
        id: concept.id,
        relative_path: page_id.relative_path(),
        concept_type: concept_type.unwrap_or(expected_type),
        title: title.unwrap_or_else(|| concept.draft.title.clone()),
        description: description.unwrap_or_default(),
        tags: tags.unwrap_or_default(),
        resource,
        timestamp,
        revision,
        source_sha256,
        language,
        generator_model,
        reviewed_at,
        extensions,
        fingerprint: page.snapshot.fingerprint.clone(),
    }
}

fn concept_extensions(metadata: &[(String, String)]) -> BTreeMap<String, String> {
    const KNOWN_FIELDS: &[&str] = &[
        "type",
        "title",
        "description",
        "resource",
        "tags",
        "timestamp",
        "airwiki.profile_version",
        "airwiki.id",
        "airwiki.collection_id",
        "airwiki.source_sha256",
        "airwiki.revision",
        "airwiki.language",
        "airwiki.status",
        "airwiki.generator_model",
        "airwiki.reviewed_at",
    ];
    metadata
        .iter()
        .filter(|(key, _)| !KNOWN_FIELDS.contains(&key.as_str()))
        .cloned()
        .collect()
}

fn compare_field<T: PartialEq + ?Sized>(
    health: &mut BundleHealthReport,
    page_id: KnowledgePageId,
    field: &str,
    actual: Option<&T>,
    expected: Option<&T>,
) {
    if actual != expected {
        health.push(HealthIssue::new(
            HealthSeverity::Error,
            "metadata_mismatch",
            Some(page_id),
            format!("El campo `{field}` del bundle no coincide con SQLite."),
        ));
    }
}

fn compare_concept_timestamp(
    health: &mut BundleHealthReport,
    page_id: KnowledgePageId,
    actual: Option<&DateTime<Utc>>,
    concept: &ConceptRecord,
) {
    let matches_published_revision = actual.is_some_and(|actual| {
        concept.reviewed_at.is_some_and(|reviewed_at| {
            // Profile v1 used the publishing transition's operational
            // `updated_at` before timestamp became canonical. Those legacy
            // values are bounded by the durable review and final commit.
            *actual >= reviewed_at && *actual <= concept.updated_at
        })
    });
    if !matches_published_revision {
        health.push(HealthIssue::new(
            HealthSeverity::Error,
            "metadata_mismatch",
            Some(page_id),
            "El campo `timestamp` del bundle no coincide con la revisión publicada en SQLite.",
        ));
    }
}

fn yaml_at<'a>(root: &'a YamlValue, path: &[&str]) -> Option<&'a YamlValue> {
    let mut current = root;
    for component in path {
        let mapping = current.as_mapping()?;
        current = mapping.get(YamlValue::String((*component).to_owned()))?;
    }
    Some(current)
}

fn yaml_string_at(root: &YamlValue, path: &[&str]) -> Option<String> {
    yaml_at(root, path)?.as_str().map(ToOwned::to_owned)
}

fn yaml_strings_at(root: &YamlValue, path: &[&str]) -> Option<Vec<String>> {
    yaml_at(root, path)?
        .as_sequence()?
        .iter()
        .map(|value| value.as_str().map(ToOwned::to_owned))
        .collect()
}

fn yaml_u32_at(root: &YamlValue, path: &[&str]) -> Option<u32> {
    u32::try_from(yaml_at(root, path)?.as_u64()?).ok()
}

fn yaml_uuid_at(root: &YamlValue, path: &[&str]) -> Option<Uuid> {
    Uuid::parse_str(yaml_at(root, path)?.as_str()?).ok()
}

fn yaml_datetime_at(root: &YamlValue, path: &[&str]) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(yaml_at(root, path)?.as_str()?)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn page_title(
    page_id: KnowledgePageId,
    parsed: &ParsedMarkdown,
    bundle: &KnowledgeBundleView,
) -> String {
    if let Some(title) = parsed
        .yaml
        .as_ref()
        .and_then(|yaml| yaml_string_at(yaml, &["title"]))
        .filter(|title| !title.trim().is_empty())
    {
        return title;
    }
    if let Some(title) = parsed
        .body
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("# "))
        .filter(|title| !title.trim().is_empty())
    {
        return title.trim().to_owned();
    }
    match page_id {
        KnowledgePageId::Index => "Índice".to_owned(),
        KnowledgePageId::Log => "Historial".to_owned(),
        KnowledgePageId::Concept(id) => bundle
            .concepts
            .iter()
            .find(|concept| concept.id == id)
            .map(|concept| concept.title.clone())
            .unwrap_or_else(|| id.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use airwiki_types::{ConceptType, EnrichmentDraft, SuggestedEntity, SuggestedLink};
    use tempfile::TempDir;

    use crate::{EMBEDDING_DIMENSIONS, OkfConcept, OkfPublisher, StoredChunk};

    use super::*;

    struct Fixture {
        _temp: TempDir,
        database: Database,
        collection: CollectionRecord,
        source_root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let source_root = temp.path().join("source");
            fs::create_dir_all(&source_root).unwrap();
            let database = Database::in_memory().unwrap();
            let collection = database
                .create_collection(
                    "Conocimiento interno",
                    &source_root,
                    temp.path().join("vault-with-unrelated-id"),
                    Default::default(),
                )
                .unwrap();
            Self {
                _temp: temp,
                database,
                collection,
                source_root,
            }
        }

        fn inspector(&self) -> OkfBundleInspector {
            OkfBundleInspector::new(self.database.clone())
        }

        fn publish(&self, file_name: &str, title: &str) -> ConceptRecord {
            let source_text = format!("# {title}\n\nContenido fuente verificable.");
            let source_path = self.source_root.join(file_name);
            fs::write(&source_path, &source_text).unwrap();
            let source_hash = hex::encode(Sha256::digest(source_text.as_bytes()));
            let source_id = self
                .database
                .register_source(
                    self.collection.id,
                    &source_path,
                    &source_hash,
                    "markdown",
                    u64::try_from(source_text.len()).unwrap(),
                )
                .unwrap()
                .id();
            self.database
                .mark_extracted(source_id, 1, u64::try_from(source_text.len()).unwrap())
                .unwrap();
            let draft = draft(title);
            let concept = self
                .database
                .save_enrichment(source_id, draft.clone(), "peer-test", "model-test")
                .unwrap();
            let text_hash = hex::encode(Sha256::digest(source_text.as_bytes()));
            self.database
                .replace_chunks(
                    concept.id,
                    &[StoredChunk {
                        id: Uuid::new_v4(),
                        concept_id: concept.id,
                        source_document_id: source_id,
                        collection_id: self.collection.id,
                        ordinal: 0,
                        heading_or_page: title.to_owned(),
                        text: source_text,
                        text_sha256: text_hash,
                        embedding: vec![0.0; EMBEDDING_DIMENSIONS],
                        source_revision: 1,
                    }],
                )
                .unwrap();
            let published = self.database.approve_concept(concept.id, draft).unwrap();
            let source = self.database.source_document(source_id).unwrap().unwrap();
            let all = self
                .database
                .list_published_concepts(self.collection.id)
                .unwrap();
            OkfPublisher::new(&self.collection.wiki_folder)
                .publish(&published, &source, &all, "published")
                .unwrap();
            published
        }

        fn replace_database_and_concept_only(&self, file_name: &str, title: &str) -> ConceptRecord {
            let source_text = format!("# {title}\n\nContenido fuente revisado.");
            let source_path = self.source_root.join(file_name);
            fs::write(&source_path, &source_text).unwrap();
            let source_hash = hex::encode(Sha256::digest(source_text.as_bytes()));
            let source_id = self
                .database
                .register_source(
                    self.collection.id,
                    &source_path,
                    &source_hash,
                    "markdown",
                    u64::try_from(source_text.len()).unwrap(),
                )
                .unwrap()
                .id();
            self.database
                .mark_extracted(source_id, 1, u64::try_from(source_text.len()).unwrap())
                .unwrap();
            let draft = draft(title);
            let concept = self
                .database
                .save_enrichment(source_id, draft.clone(), "peer-test", "model-test")
                .unwrap();
            let source = self.database.source_document(source_id).unwrap().unwrap();
            self.database
                .replace_chunks(
                    concept.id,
                    &[StoredChunk {
                        id: Uuid::new_v4(),
                        concept_id: concept.id,
                        source_document_id: source_id,
                        collection_id: self.collection.id,
                        ordinal: 0,
                        heading_or_page: title.to_owned(),
                        text: source_text.clone(),
                        text_sha256: hex::encode(Sha256::digest(source_text.as_bytes())),
                        embedding: vec![0.0; EMBEDDING_DIMENSIONS],
                        source_revision: source.revision,
                    }],
                )
                .unwrap();
            let published = self.database.approve_concept(concept.id, draft).unwrap();
            let source = self.database.source_document(source_id).unwrap().unwrap();
            let rendered = OkfPublisher::new(&self.collection.wiki_folder)
                .validate_candidate(
                    &published,
                    &source,
                    published.reviewed_at.expect("approved concept is reviewed"),
                )
                .unwrap();
            fs::write(self.concept_path(published.id), rendered).unwrap();
            published
        }

        fn concept_path(&self, id: Uuid) -> PathBuf {
            KnowledgePageId::Concept(id).path_below(&self.collection.wiki_folder)
        }
    }

    fn draft(title: &str) -> EnrichmentDraft {
        EnrichmentDraft {
            concept_type: ConceptType::Document,
            title: title.to_owned(),
            description: format!("Descripción de {title}."),
            language: "es".to_owned(),
            tags: vec!["interno".to_owned(), "prueba".to_owned()],
            entities: vec![SuggestedEntity {
                name: "Empresa".to_owned(),
                kind: "organización".to_owned(),
            }],
            links: vec![SuggestedLink {
                label: "Sitio".to_owned(),
                target: "https://example.invalid".to_owned(),
            }],
            summary: format!("Resumen de {title}."),
            classification_confidence: 0.9,
            classification_explanation: "Documento de prueba".to_owned(),
        }
    }

    fn has_issue(bundle: &KnowledgeBundleView, code: &str) -> bool {
        bundle.health.issues.iter().any(|issue| issue.code == code)
    }

    fn recovery_for(
        severity: HealthSeverity,
        code: &str,
        page: Option<KnowledgePageId>,
    ) -> HealthRecovery {
        HealthIssue::new(severity, code, page, "synthetic finding").recovery()
    }

    #[test]
    fn missing_index_has_automatic_derived_recovery() {
        assert_eq!(
            recovery_for(
                HealthSeverity::Error,
                "missing_index",
                Some(KnowledgePageId::Index),
            ),
            HealthRecovery::AutomaticDerived
        );
    }

    #[test]
    fn concept_metadata_drift_has_guided_recovery() {
        assert_eq!(
            recovery_for(
                HealthSeverity::Error,
                "metadata_mismatch",
                Some(KnowledgePageId::Concept(Uuid::nil())),
            ),
            HealthRecovery::GuidedContent
        );
    }

    #[test]
    fn missing_bundle_requires_manual_intervention() {
        assert_eq!(
            recovery_for(HealthSeverity::Error, "missing_bundle", None),
            HealthRecovery::ManualIntervention
        );
    }

    #[test]
    fn unknown_recovery_code_fails_closed_to_manual_intervention() {
        assert_eq!(
            recovery_for(
                HealthSeverity::Error,
                "future_recovery_code",
                Some(KnowledgePageId::Concept(Uuid::nil())),
            ),
            HealthRecovery::ManualIntervention
        );
    }

    #[test]
    fn log_findings_require_manual_history_recovery() {
        assert_eq!(
            recovery_for(
                HealthSeverity::Error,
                "missing_log",
                Some(KnowledgePageId::Log),
            ),
            HealthRecovery::ManualHistory
        );
    }

    #[test]
    fn informational_findings_never_offer_recovery() {
        assert_eq!(
            recovery_for(
                HealthSeverity::Info,
                "historical_broken_link",
                Some(KnowledgePageId::Log),
            ),
            HealthRecovery::Informational
        );
    }

    #[test]
    fn empty_collection_without_directory_is_a_healthy_empty_bundle() {
        let fixture = Fixture::new();

        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();

        assert_eq!(bundle.state, KnowledgeBundleState::Empty);
        assert!(bundle.concepts.is_empty());
        assert!(bundle.health.is_healthy());
        assert_eq!(bundle.fingerprint.len(), 64);
    }

    #[test]
    fn orphan_is_reported_even_when_database_has_no_published_concepts() {
        let fixture = Fixture::new();
        let orphan_id = Uuid::new_v4();
        fs::create_dir_all(fixture.collection.wiki_folder.join("concepts")).unwrap();
        fs::write(
            fixture.concept_path(orphan_id),
            "---\ntype: Document\nresource: urn:airwiki:orphan\n---\n\n# Huérfano\n",
        )
        .unwrap();

        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();

        assert_eq!(bundle.state, KnowledgeBundleState::Empty);
        assert!(bundle.concepts.is_empty());
        assert!(has_issue(&bundle, "unexpected_concept"));
        assert_eq!(bundle.health.error_count, 1);
    }

    #[test]
    fn uuid_named_orphan_has_a_guided_recovery_target() {
        let fixture = Fixture::new();
        let orphan_id = Uuid::new_v4();
        fs::create_dir_all(fixture.collection.wiki_folder.join("concepts")).unwrap();
        fs::write(
            fixture.concept_path(orphan_id),
            "---\ntype: Document\nresource: urn:airwiki:orphan\n---\n\n# Huérfano\n",
        )
        .unwrap();

        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();
        let issue = bundle
            .health
            .issues
            .iter()
            .find(|issue| issue.code == "unexpected_concept")
            .unwrap();

        assert_eq!(
            (issue.page, issue.recovery()),
            (
                Some(KnowledgePageId::Concept(orphan_id)),
                HealthRecovery::GuidedContent,
            )
        );
    }

    #[test]
    fn valid_bundle_reconciles_and_loads_every_reserved_page() {
        let fixture = Fixture::new();
        let concept = fixture.publish("policy.md", "Política de pagos");
        let inspector = fixture.inspector();

        let bundle = inspector.inspect_bundle(fixture.collection.id).unwrap();

        assert_eq!(bundle.state, KnowledgeBundleState::Ready);
        assert!(bundle.health.is_healthy(), "{:#?}", bundle.health.issues);
        assert_eq!(bundle.health.total_concepts, 1);
        assert_eq!(bundle.concepts.len(), 1);
        assert_eq!(bundle.concepts[0].id, concept.id);
        assert_eq!(bundle.collection_policy, fixture.collection.policy);
        assert_eq!(bundle.concepts[0].fingerprint.len(), 64);
        assert!(bundle.index_fingerprint.is_some());
        assert!(bundle.log_fingerprint.is_some());
        assert!(bundle.links.iter().any(|link| {
            link.source == KnowledgePageId::Index
                && link.disposition
                    == KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(concept.id))
        }));
        let backlinks = bundle
            .backlinks
            .get(&KnowledgePageId::Concept(concept.id))
            .unwrap();
        assert!(backlinks.contains(&KnowledgePageId::Index));
        assert!(backlinks.contains(&KnowledgePageId::Log));

        for page_id in [
            KnowledgePageId::Index,
            KnowledgePageId::Log,
            KnowledgePageId::Concept(concept.id),
        ] {
            let page = inspector
                .load_page(
                    fixture.collection.id,
                    page_id,
                    bundle.page_fingerprint(page_id),
                    MAX_KNOWLEDGE_PAGE_BYTES,
                )
                .unwrap();
            assert!(!page.truncated);
            assert_eq!(page.fingerprint, bundle.page_fingerprint(page_id).unwrap());
            assert!(!page.body_markdown.starts_with("---"));
        }
    }

    #[test]
    fn timestamp_reconciliation_accepts_only_the_published_revision_window() {
        let fixture = Fixture::new();
        let mut concept = fixture.publish("legacy.md", "Concepto heredado");
        let reviewed_at = concept.reviewed_at.unwrap();
        let legacy_timestamp = reviewed_at + chrono::TimeDelta::microseconds(1);
        concept.updated_at = reviewed_at + chrono::TimeDelta::microseconds(2);
        let mut health = BundleHealthReport {
            checked_at: Utc::now(),
            total_concepts: 1,
            error_count: 0,
            warning_count: 0,
            issues: Vec::new(),
        };

        compare_concept_timestamp(
            &mut health,
            KnowledgePageId::Concept(concept.id),
            Some(&legacy_timestamp),
            &concept,
        );
        assert!(health.is_healthy(), "{:#?}", health.issues);

        let outside_revision = concept.updated_at + chrono::TimeDelta::microseconds(1);
        compare_concept_timestamp(
            &mut health,
            KnowledgePageId::Concept(concept.id),
            Some(&outside_revision),
            &concept,
        );
        assert_eq!(health.error_count, 1);
        assert_eq!(health.issues[0].code, "metadata_mismatch");
    }

    #[test]
    fn replacement_with_old_index_and_log_is_reported_as_updating() {
        let fixture = Fixture::new();
        let original = fixture.publish("replace.md", "Título versión uno");
        let index_before = fs::read(fixture.collection.wiki_folder.join("index.md")).unwrap();
        let log_before = fs::read(fixture.collection.wiki_folder.join("log.md")).unwrap();

        let replacement =
            fixture.replace_database_and_concept_only("replace.md", "Título versión dos");
        assert_eq!(replacement.id, original.id);
        assert_eq!(
            fs::read(fixture.collection.wiki_folder.join("index.md")).unwrap(),
            index_before
        );
        assert_eq!(
            fs::read(fixture.collection.wiki_folder.join("log.md")).unwrap(),
            log_before
        );

        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();

        assert_eq!(bundle.state, KnowledgeBundleState::Updating);
        assert!(has_issue(&bundle, "stale_index_metadata"));
        assert!(has_issue(&bundle, "stale_log_revision"));
        assert_eq!(bundle.concepts[0].revision, Some(2));
    }

    #[test]
    fn fresh_publication_drift_is_updating_but_old_drift_is_stable_error() {
        let fixture = Fixture::new();
        let concept = fixture.publish("race.md", "Publicación en curso");
        fs::remove_file(fixture.collection.wiki_folder.join("index.md")).unwrap();

        let updating = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();

        assert_eq!(updating.state, KnowledgeBundleState::Updating);
        assert!(has_issue(&updating, "missing_index"));
        assert!(updating.health.error_count > 0);

        let mut old_concept = concept;
        old_concept.updated_at = Utc::now() - chrono::TimeDelta::seconds(10);
        let mut health = BundleHealthReport {
            checked_at: Utc::now(),
            total_concepts: 1,
            error_count: 0,
            warning_count: 0,
            issues: Vec::new(),
        };
        health.push(HealthIssue::new(
            HealthSeverity::Error,
            "missing_index",
            Some(KnowledgePageId::Index),
            "Falta index.md.",
        ));
        let stable = finalize_bundle(
            &fixture.collection,
            &[old_concept],
            BundleParts::empty(BTreeMap::new(), health),
        );
        assert_eq!(stable.state, KnowledgeBundleState::Ready);
        assert!(stable.health.error_count > 0);
    }

    #[test]
    fn inspector_accepts_bom_crlf_unknown_type_extensions_and_index_version() {
        let fixture = Fixture::new();
        let concept = fixture.publish("future.md", "Concepto futuro");
        let concept_path = fixture.concept_path(concept.id);
        let concept_markdown = fs::read_to_string(&concept_path)
            .unwrap()
            .replace(
                "type: Document",
                "type: Future Knowledge\nx_extension:\n  enabled: true",
            )
            .replace('\n', "\r\n");
        fs::write(&concept_path, format!("\u{feff}{concept_markdown}")).unwrap();
        let index_path = fixture.collection.wiki_folder.join("index.md");
        let index = fs::read_to_string(&index_path).unwrap();
        fs::write(
            &index_path,
            format!(
                "\u{feff}---\r\nokf_version: '0.1'\r\n---\r\n\r\n{}",
                index.replace('\n', "\r\n")
            ),
        )
        .unwrap();

        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();

        assert_eq!(bundle.concepts[0].concept_type, "Future Knowledge");
        assert_eq!(
            bundle.concepts[0].extensions.get("x_extension.enabled"),
            Some(&"true".to_owned())
        );
        assert!(has_issue(&bundle, "metadata_mismatch"));
        assert!(!has_issue(&bundle, "invalid_frontmatter"));
        assert!(!has_issue(&bundle, "invalid_index_structure"));
        let raw = fs::read_to_string(&concept_path).unwrap();
        assert!(
            OkfConcept::parse(&raw).is_err(),
            "strict producer parser changed"
        );
        fs::write(
            &concept_path,
            raw.replace("type: Future Knowledge", "type: Document"),
        )
        .unwrap();
        let loadable = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();
        assert_eq!(loadable.state, KnowledgeBundleState::Ready);
        let page = fixture
            .inspector()
            .load_page(
                fixture.collection.id,
                KnowledgePageId::Concept(concept.id),
                loadable.page_fingerprint(KnowledgePageId::Concept(concept.id)),
                MAX_KNOWLEDGE_PAGE_BYTES,
            )
            .unwrap();
        assert!(!page.body_markdown.contains('\r'));
        assert!(
            page.metadata
                .iter()
                .any(|(key, value)| key == "x_extension.enabled" && value == "true")
        );
    }

    #[test]
    fn invalid_yaml_and_oversized_pages_are_health_issues_not_inspection_errors() {
        let fixture = Fixture::new();
        let concept = fixture.publish("large.md", "Documento grande");
        let path = fixture.concept_path(concept.id);
        let invalid = fs::read_to_string(&path)
            .unwrap()
            .replace("title: Documento grande", "title: [sin cierre");
        fs::write(&path, invalid).unwrap();

        let malformed = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();
        assert!(has_issue(&malformed, "invalid_frontmatter"));
        assert_eq!(malformed.concepts.len(), 1);

        let mut oversized = fs::read_to_string(&path)
            .unwrap()
            .replace("title: [sin cierre", "title: Documento grande");
        oversized.push_str(&"x".repeat(MAX_KNOWLEDGE_PAGE_BYTES + 1024));
        fs::write(&path, oversized).unwrap();
        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();
        assert!(has_issue(&bundle, "page_too_large"));
        let page = fixture
            .inspector()
            .load_page(
                fixture.collection.id,
                KnowledgePageId::Concept(concept.id),
                bundle.page_fingerprint(KnowledgePageId::Concept(concept.id)),
                usize::MAX,
            )
            .unwrap();
        assert!(page.truncated);
        assert!(page.body_markdown.len() <= MAX_KNOWLEDGE_PAGE_BYTES);
    }

    #[test]
    fn links_resolve_relative_and_airwiki_urn_and_classify_unsafe_targets() {
        let fixture = Fixture::new();
        let first = fixture.publish("first.md", "Primer concepto");
        let second = fixture.publish("second.md", "Segundo concepto");
        let second_resource = second.logical_resource_uri.clone();
        let path = fixture.concept_path(first.id);
        let mut markdown = fs::read_to_string(&path).unwrap();
        markdown.push_str(&format!(
            "\n[relativo]({}.md)\n[urn]({second_resource})\n[web](https://example.com)\n[correo](mailto:test@example.com)\n[absoluto](/concepts/{}.md)\n[traversal](../../secret.md)\n[script](javascript:alert(1))\n[roto](missing.md)\n[self](#seccion)\n",
            second.id, second.id
        ));
        fs::write(path, markdown).unwrap();

        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();
        let by_label = bundle
            .links
            .iter()
            .filter(|link| link.source == KnowledgePageId::Concept(first.id))
            .map(|link| (link.label.as_str(), &link.disposition))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            by_label["relativo"],
            &KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(second.id))
        );
        assert_eq!(
            by_label["urn"],
            &KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(second.id))
        );
        assert_eq!(by_label["web"], &KnowledgeLinkDisposition::External);
        assert_eq!(by_label["correo"], &KnowledgeLinkDisposition::External);
        assert_eq!(by_label["absoluto"], &KnowledgeLinkDisposition::Unsafe);
        assert_eq!(by_label["traversal"], &KnowledgeLinkDisposition::Unsafe);
        assert_eq!(by_label["script"], &KnowledgeLinkDisposition::Unsafe);
        assert_eq!(by_label["roto"], &KnowledgeLinkDisposition::Broken);
        assert_eq!(
            by_label["self"],
            &KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(first.id))
        );
        assert!(
            bundle
                .backlinks
                .get(&KnowledgePageId::Concept(second.id))
                .unwrap()
                .contains(&KnowledgePageId::Concept(first.id))
        );
        assert!(has_issue(&bundle, "unsafe_link"));
        assert!(has_issue(&bundle, "broken_link"));

        let page = fixture
            .inspector()
            .load_page(
                fixture.collection.id,
                KnowledgePageId::Concept(first.id),
                bundle.page_fingerprint(KnowledgePageId::Concept(first.id)),
                MAX_KNOWLEDGE_PAGE_BYTES,
            )
            .unwrap();
        assert!(page.outgoing_links.iter().any(|link| {
            link.label == "urn"
                && link.disposition
                    == KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(second.id))
        }));
    }

    #[test]
    fn frontmatter_resource_is_authoritative_for_link_resolution() {
        let fixture = Fixture::new();
        let source = fixture.publish("source.md", "Origen");
        let target = fixture.publish("target.md", "Destino");
        let database_resource = target.logical_resource_uri.clone();
        let okf_resource = format!("urn:airwiki:frontmatter:{}", target.id);
        let target_path = fixture.concept_path(target.id);
        let target_markdown = fs::read_to_string(&target_path)
            .unwrap()
            .replace(&database_resource, &okf_resource);
        fs::write(target_path, target_markdown).unwrap();
        let source_path = fixture.concept_path(source.id);
        let mut source_markdown = fs::read_to_string(&source_path).unwrap();
        source_markdown.push_str(&format!(
            "\n[recurso OKF]({okf_resource})\n[recurso SQLite]({database_resource})\n"
        ));
        fs::write(source_path, source_markdown).unwrap();

        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();
        let links = bundle
            .links
            .iter()
            .filter(|link| link.source == KnowledgePageId::Concept(source.id))
            .map(|link| (link.label.as_str(), &link.disposition))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            links["recurso OKF"],
            &KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(target.id))
        );
        assert_eq!(links["recurso SQLite"], &KnowledgeLinkDisposition::Broken);
        let concept = bundle
            .concepts
            .iter()
            .find(|concept| concept.id == target.id)
            .unwrap();
        assert_eq!(concept.resource.as_deref(), Some(okf_resource.as_str()));

        let context = LinkContext::from_bundle(&bundle);
        assert_eq!(
            resolve_link_target(KnowledgePageId::Concept(source.id), &okf_resource, &context),
            KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(target.id))
        );
        assert_eq!(
            resolve_link_target(
                KnowledgePageId::Concept(source.id),
                &database_resource,
                &context
            ),
            KnowledgeLinkDisposition::Broken
        );
    }

    #[test]
    fn missing_orphan_and_database_drift_are_reported_without_exposing_orphans() {
        let fixture = Fixture::new();
        let concept = fixture.publish("drift.md", "Título original");
        let path = fixture.concept_path(concept.id);
        let changed = fs::read_to_string(&path)
            .unwrap()
            .replace("title: Título original", "title: Título alterado");
        fs::write(&path, changed).unwrap();
        let orphan_id = Uuid::new_v4();
        fs::write(
            fixture.concept_path(orphan_id),
            "---\ntype: Document\n---\n\n# Huérfano\n",
        )
        .unwrap();

        let drift = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();
        assert!(has_issue(&drift, "metadata_mismatch"));
        assert!(has_issue(&drift, "unexpected_concept"));
        assert!(drift.concepts.iter().all(|item| item.id != orphan_id));

        fs::remove_file(path).unwrap();
        let missing = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();
        assert!(has_issue(&missing, "missing_concept"));
        assert!(missing.concepts.is_empty());
    }

    #[test]
    fn hidden_markdown_and_hidden_directories_are_ignored() {
        let fixture = Fixture::new();
        fixture.publish("visible.md", "Documento visible");
        fs::write(
            fixture.collection.wiki_folder.join(".private.md"),
            "# No administrar\n",
        )
        .unwrap();
        let hidden = fixture.collection.wiki_folder.join(".cache");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(hidden.join("secret.md"), "# No administrar\n").unwrap();

        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();

        assert!(!has_issue(&bundle, "unmanaged_markdown"));
        assert!(!has_issue(&bundle, "unexpected_concept"));
        assert_eq!(bundle.concepts.len(), 1);
    }

    #[test]
    fn fingerprints_detect_stale_pages_and_inspection_performs_no_writes() {
        let fixture = Fixture::new();
        let concept = fixture.publish("stable.md", "Documento estable");
        let inspector = fixture.inspector();
        let first = inspector.inspect_bundle(fixture.collection.id).unwrap();
        let second = inspector.inspect_bundle(fixture.collection.id).unwrap();
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(
            first.concepts[0].fingerprint,
            second.concepts[0].fingerprint
        );

        let paths = [
            fixture.collection.wiki_folder.join("index.md"),
            fixture.collection.wiki_folder.join("log.md"),
            fixture.concept_path(concept.id),
        ];
        let before = paths
            .iter()
            .map(fs::read)
            .collect::<std::io::Result<Vec<_>>>()
            .unwrap();
        let audit_count = fixture.database.count("audit_events").unwrap();
        let page = inspector
            .load_page(
                fixture.collection.id,
                KnowledgePageId::Concept(concept.id),
                Some(&first.concepts[0].fingerprint),
                4096,
            )
            .unwrap();
        assert!(!page.truncated);
        assert_eq!(
            before,
            paths
                .iter()
                .map(fs::read)
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        );
        assert_eq!(audit_count, fixture.database.count("audit_events").unwrap());

        let mut changed = fs::read_to_string(fixture.concept_path(concept.id)).unwrap();
        changed.push_str("\nCambio posterior.\n");
        fs::write(fixture.concept_path(concept.id), changed).unwrap();
        let error = inspector
            .load_page(
                fixture.collection.id,
                KnowledgePageId::Concept(concept.id),
                Some(&first.concepts[0].fingerprint),
                4096,
            )
            .unwrap_err();
        assert!(error.to_string().contains("cambió"));
        let third = inspector.inspect_bundle(fixture.collection.id).unwrap();
        assert_ne!(first.fingerprint, third.fingerprint);
        assert_ne!(first.concepts[0].fingerprint, third.concepts[0].fingerprint);
    }

    #[test]
    fn invalid_utf8_is_visible_as_health_issue_instead_of_aborting_bundle() {
        let fixture = Fixture::new();
        let concept = fixture.publish("utf8.md", "UTF-8");
        let path = fixture.concept_path(concept.id);
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&[0xff, 0xfe]);
        fs::write(path, bytes).unwrap();

        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();

        assert!(has_issue(&bundle, "invalid_utf8"));
        assert_eq!(bundle.concepts.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_concept_is_rejected_fail_closed() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let concept = fixture.publish("link.md", "Enlace");
        let path = fixture.concept_path(concept.id);
        let target = fixture.source_root.join("outside.md");
        fs::write(&target, fs::read(&path).unwrap()).unwrap();
        fs::remove_file(&path).unwrap();
        symlink(target, path).unwrap();

        let bundle = fixture
            .inspector()
            .inspect_bundle(fixture.collection.id)
            .unwrap();

        assert!(has_issue(&bundle, "unsafe_page_symlink"));
        assert!(bundle.concepts.is_empty());
    }
}
