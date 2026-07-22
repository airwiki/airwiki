use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};

use airwiki_types::{
    CollectionPolicy, ConceptType, DisclosureGate, DisclosureLease, DisclosureMutationGuard,
    DocumentStatus, EnrichmentDraft, PublicConceptSummary, SearchHit, SearchPurpose,
};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::EMBEDDING_DIMENSIONS;
use crate::chunk_identity::public_chunk_id;

const MIGRATION_1: &str = include_str!("../migrations/0001_initial.sql");
const MIGRATION_2: &str = include_str!("../migrations/0002_publication_claims.sql");
const MIGRATION_3: &str = include_str!("../migrations/0003_collection_maintenance.sql");
const MIGRATION_4: &str = include_str!("../migrations/0004_public_federation.sql");
const MIGRATION_5: &str = include_str!("../migrations/0005_public_federation_hardening.sql");

#[derive(Debug, Clone)]
pub struct Database {
    inner: Arc<Mutex<Connection>>,
    publication_lock: Arc<Mutex<()>>,
    disclosure_gate: DisclosureGate,
    path: Option<PathBuf>,
}

struct DatabaseConnectionGuard<'a> {
    connection: MutexGuard<'a, Connection>,
    _mutation: DisclosureMutationGuard,
}

impl Deref for DatabaseConnectionGuard<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

impl DerefMut for DatabaseConnectionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connection
    }
}

#[derive(Debug, Clone)]
pub struct CollectionRecord {
    pub id: Uuid,
    pub name: String,
    pub source_folder: PathBuf,
    pub wiki_folder: PathBuf,
    pub policy: CollectionPolicy,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicCollectionProfileRecord {
    pub collection_id: Uuid,
    pub description: String,
    pub languages: Vec<String>,
    pub manifest_sequence: u64,
    pub enabled_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicManifestMaterial {
    pub concept_count: u32,
    pub routing_terms: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FederationIndexRecord {
    pub peer_id: String,
    pub multiaddr: String,
    pub enabled: bool,
    pub source: String,
    pub registry_version: u32,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SourceDocumentRecord {
    pub id: Uuid,
    pub collection_id: Uuid,
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub source_format: String,
    pub byte_size: u64,
    pub page_count: u32,
    pub character_count: u64,
    pub status: DocumentStatus,
    pub revision: u32,
    pub concept_id: Option<Uuid>,
    pub last_error: Option<String>,
    pub discovered_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ConceptRecord {
    pub id: Uuid,
    pub source_document_id: Uuid,
    pub collection_id: Uuid,
    pub draft: EnrichmentDraft,
    pub logical_resource_uri: String,
    pub generator_model: String,
    pub status: DocumentStatus,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct StoredChunk {
    pub id: Uuid,
    pub concept_id: Uuid,
    pub source_document_id: Uuid,
    pub collection_id: Uuid,
    pub ordinal: u32,
    pub heading_or_page: String,
    pub text: String,
    pub text_sha256: String,
    pub embedding: Vec<f32>,
    pub source_revision: u32,
}

/// Opaque identity of the exact pending review state shown to a human.
///
/// The value is not an authorization secret, but its representation stays
/// private so it cannot accidentally become a logging or persistence contract.
#[derive(Clone, PartialEq, Eq)]
pub struct ReviewVersionToken([u8; 32]);

impl ReviewVersionToken {
    /// Constructs a token for boundary tests and non-persistent view fixtures.
    #[doc(hidden)]
    pub const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }
}

impl std::fmt::Debug for ReviewVersionToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ReviewVersionToken([REDACTED])")
    }
}

/// Source evidence displayed while a concept is awaiting human review.
///
/// Its custom `Debug` implementation intentionally excludes source-derived
/// strings so diagnostic logs cannot accidentally disclose document content.
#[derive(Clone, PartialEq, Eq)]
pub struct ReviewEvidenceChunkRecord {
    pub ordinal: u32,
    pub heading_or_page: String,
    pub text: String,
}

impl std::fmt::Debug for ReviewEvidenceChunkRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReviewEvidenceChunkRecord")
            .field("ordinal", &self.ordinal)
            .field("heading_or_page_len", &self.heading_or_page.len())
            .field("text_len", &self.text.len())
            .finish()
    }
}

/// One stable page of source evidence for the exact revision under review.
///
/// `next_ordinal`, when present, is an exclusive cursor to pass back as
/// `after_ordinal` when requesting the following page.
#[derive(Clone, PartialEq, Eq)]
pub struct ReviewEvidencePageRecord {
    pub concept_id: Uuid,
    pub source_revision: u32,
    pub review_version: ReviewVersionToken,
    pub total_chunks: usize,
    pub chunks: Vec<ReviewEvidenceChunkRecord>,
    pub next_ordinal: Option<u32>,
}

impl std::fmt::Debug for ReviewEvidencePageRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReviewEvidencePageRecord")
            .field("source_revision", &self.source_revision)
            .field("total_chunks", &self.total_chunks)
            .field("page_chunk_count", &self.chunks.len())
            .field("next_ordinal", &self.next_ordinal)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct JobRecord {
    pub id: Uuid,
    pub source_document_id: Option<Uuid>,
    pub kind: String,
    pub state: String,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Immutable identity captured while a pending review is exclusively claimed
/// for another enrichment attempt. The claim keeps the existing draft and
/// chunks intact, but makes the concept temporarily non-approvable.
#[derive(Debug, Clone)]
pub struct ReviewReanalysisClaim {
    pub job_id: Uuid,
    pub concept_id: Uuid,
    pub source_document_id: Uuid,
    pub collection_id: Uuid,
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub source_format: String,
    pub byte_size: u64,
    pub revision: u32,
}

/// Durable human-approval intent retained until its OKF files and SQLite state
/// have both reached the same publication boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublicationClaim {
    pub job_id: Uuid,
    pub concept_id: Uuid,
    pub source_document_id: Uuid,
    pub collection_id: Uuid,
    pub source_path: PathBuf,
    pub source_sha256: String,
    pub source_revision: u32,
    pub action: String,
    pub reviewed_at: DateTime<Utc>,
    pub job_state: String,
}

pub(crate) struct ExpectedReview<'a> {
    pub source_sha256: &'a str,
    pub source_revision: u32,
    pub review_version: &'a ReviewVersionToken,
}

#[derive(Debug, Clone)]
pub struct PeerRecord {
    pub peer_id: String,
    pub display_name: Option<String>,
    pub trusted: bool,
    pub blocked: bool,
    pub paired_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct GrantRecord {
    pub peer_id: String,
    pub collection_id: Uuid,
    pub granted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CollectionStats {
    pub sources: u64,
    pub needs_review: u64,
    pub published: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CollectionMaintenanceStatus {
    #[default]
    Never,
    Success,
    Partial,
    Failed,
    Quarantined,
}

impl CollectionMaintenanceStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Quarantined => "quarantined",
        }
    }
}

impl FromStr for CollectionMaintenanceStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "never" => Ok(Self::Never),
            "success" => Ok(Self::Success),
            "partial" => Ok(Self::Partial),
            "failed" => Ok(Self::Failed),
            "quarantined" => Ok(Self::Quarantined),
            _ => bail!("invalid collection maintenance status {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CollectionMaintenanceCounts {
    pub analyzed: u64,
    pub unchanged: u64,
    pub renamed: u64,
    pub deleted: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionMaintenanceRecord {
    pub collection_id: Uuid,
    pub last_started_at: Option<DateTime<Utc>>,
    pub last_finished_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub status: CollectionMaintenanceStatus,
    pub counts: CollectionMaintenanceCounts,
    pub issue_code: Option<String>,
    pub issue_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionMaintenanceResult {
    pub status: CollectionMaintenanceStatus,
    pub counts: CollectionMaintenanceCounts,
    pub issue_code: Option<String>,
    pub issue_summary: Option<String>,
}

impl CollectionMaintenanceResult {
    pub fn success(counts: CollectionMaintenanceCounts) -> Self {
        Self {
            status: CollectionMaintenanceStatus::Success,
            counts,
            issue_code: None,
            issue_summary: None,
        }
    }

    pub fn issue(
        status: CollectionMaintenanceStatus,
        counts: CollectionMaintenanceCounts,
        code: impl Into<String>,
        summary: impl Into<String>,
    ) -> Result<Self> {
        if matches!(
            status,
            CollectionMaintenanceStatus::Never | CollectionMaintenanceStatus::Success
        ) {
            bail!("maintenance issue result requires a non-success terminal status");
        }
        let result = Self {
            status,
            counts,
            issue_code: Some(code.into()),
            issue_summary: Some(summary.into()),
        };
        result.validate()?;
        Ok(result)
    }

    fn validate(&self) -> Result<()> {
        if self.status == CollectionMaintenanceStatus::Never {
            bail!("maintenance completion cannot use never status");
        }
        if self.issue_code.is_some() != self.issue_summary.is_some() {
            bail!("maintenance issue code and summary must be provided together");
        }
        if self.status == CollectionMaintenanceStatus::Success && self.issue_code.is_some() {
            bail!("successful maintenance cannot contain an issue");
        }
        if self.status != CollectionMaintenanceStatus::Success && self.issue_code.is_none() {
            bail!("non-successful maintenance requires a sanitized issue");
        }
        if self.issue_code.as_deref().is_some_and(|code| {
            code.is_empty()
                || code.len() > 64
                || !code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }) {
            bail!("maintenance issue code must be a stable snake_case identifier");
        }
        if self.issue_summary.as_deref().is_some_and(|summary| {
            summary.is_empty()
                || summary.chars().count() > 240
                || summary.chars().any(char::is_control)
        }) {
            bail!("maintenance issue summary exceeds the safe display limit");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub actor: String,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub details: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRegistration {
    New(Uuid),
    Changed(Uuid),
    Replaced {
        id: Uuid,
        previous_source_sha256: String,
    },
    Renamed(Uuid),
    Unchanged(Uuid),
}

impl SourceRegistration {
    pub fn id(&self) -> Uuid {
        match self {
            Self::New(id) | Self::Changed(id) | Self::Renamed(id) | Self::Unchanged(id) => *id,
            Self::Replaced { id, .. } => *id,
        }
    }

    pub fn needs_processing(&self) -> bool {
        matches!(
            self,
            Self::New(_) | Self::Changed(_) | Self::Replaced { .. }
        )
    }

    pub fn previous_source_sha256(&self) -> Option<&str> {
        match self {
            Self::Replaced {
                previous_source_sha256,
                ..
            } => Some(previous_source_sha256),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RankedChunk {
    pub chunk: StoredChunk,
    pub title: String,
    pub logical_resource_uri: String,
    pub source_sha256: String,
    pub updated_at: DateTime<Utc>,
    pub lexical_score: Option<f64>,
}

/// Minimal row used while scanning the vector index.
///
/// Search needs only an identity and embedding to maintain its bounded top-k
/// set. Loading source text and citation metadata for every published chunk
/// would make one query scale with the total text corpus as well as the vector
/// corpus, so those fields are hydrated only for the final candidate IDs.
pub(crate) struct VectorEmbeddingCandidate {
    pub scan_cursor: i64,
    pub chunk_id: Uuid,
    pub embedding: Vec<f32>,
}

impl std::fmt::Debug for VectorEmbeddingCandidate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VectorEmbeddingCandidate")
            .field("scan_cursor", &self.scan_cursor)
            .field("embedding_dimensions", &self.embedding.len())
            .finish()
    }
}

struct PendingReviewMetadata {
    source_document_id: String,
    collection_id: String,
    source_sha256: String,
    source_revision: u32,
    concept_type: String,
    title: String,
    description: String,
    language: String,
    tags_json: String,
    entities_json: String,
    links_json: String,
    summary: String,
    classification_confidence: f64,
    classification_explanation: String,
    logical_resource_uri: String,
    generator_model: String,
}

struct PendingReviewSnapshot {
    source_document_id: String,
    collection_id: String,
    source_sha256: String,
    source_revision: u32,
    review_version: ReviewVersionToken,
    total_chunks: usize,
}

fn pending_review_snapshot(
    tx: &Transaction<'_>,
    concept_id: Uuid,
    expected_revision: u32,
) -> Result<Option<PendingReviewSnapshot>> {
    let metadata = tx
        .query_row(
            "SELECT co.source_document_id,co.collection_id,sd.source_sha256,sd.revision,
                    co.concept_type,co.title,co.description,co.language,co.tags_json,
                    co.entities_json,co.links_json,co.summary,co.classification_confidence,
                    co.classification_explanation,co.logical_resource_uri,co.generator_model
             FROM concepts co
             JOIN source_documents sd ON sd.id=co.source_document_id
             WHERE co.id=?1 AND co.status='needs_review' AND sd.status='needs_review'
               AND sd.concept_id=co.id AND sd.collection_id=co.collection_id
               AND sd.revision=?2",
            params![concept_id.to_string(), expected_revision],
            |row| {
                Ok(PendingReviewMetadata {
                    source_document_id: row.get(0)?,
                    collection_id: row.get(1)?,
                    source_sha256: row.get(2)?,
                    source_revision: row.get(3)?,
                    concept_type: row.get(4)?,
                    title: row.get(5)?,
                    description: row.get(6)?,
                    language: row.get(7)?,
                    tags_json: row.get(8)?,
                    entities_json: row.get(9)?,
                    links_json: row.get(10)?,
                    summary: row.get(11)?,
                    classification_confidence: row.get(12)?,
                    classification_explanation: row.get(13)?,
                    logical_resource_uri: row.get(14)?,
                    generator_model: row.get(15)?,
                })
            },
        )
        .optional()?;
    let Some(metadata) = metadata else {
        return Ok(None);
    };

    let ownership_is_corrupt = tx.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM chunks ch
           WHERE (ch.concept_id=?1 OR ch.source_document_id=?2)
             AND NOT (
               ch.concept_id=?1 AND ch.source_document_id=?2
               AND ch.collection_id=?3 AND ch.source_revision=?4
             )
         )",
        params![
            concept_id.to_string(),
            metadata.source_document_id,
            metadata.collection_id,
            metadata.source_revision,
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if ownership_is_corrupt {
        bail!("review evidence chunks do not match current concept ownership and revision");
    }

    let mut hasher = Sha256::new();
    hasher.update(b"airwiki-review-version-v1");
    hash_review_component(&mut hasher, b"concept_id", concept_id.as_bytes())?;
    hash_review_component(
        &mut hasher,
        b"source_document_id",
        metadata.source_document_id.as_bytes(),
    )?;
    hash_review_component(
        &mut hasher,
        b"collection_id",
        metadata.collection_id.as_bytes(),
    )?;
    hash_review_component(
        &mut hasher,
        b"source_revision",
        &metadata.source_revision.to_be_bytes(),
    )?;
    hash_review_component(
        &mut hasher,
        b"source_sha256",
        metadata.source_sha256.as_bytes(),
    )?;
    for (label, value) in [
        (b"concept_type".as_slice(), metadata.concept_type.as_bytes()),
        (b"title".as_slice(), metadata.title.as_bytes()),
        (b"description".as_slice(), metadata.description.as_bytes()),
        (b"language".as_slice(), metadata.language.as_bytes()),
        (b"tags_json".as_slice(), metadata.tags_json.as_bytes()),
        (
            b"entities_json".as_slice(),
            metadata.entities_json.as_bytes(),
        ),
        (b"links_json".as_slice(), metadata.links_json.as_bytes()),
        (b"summary".as_slice(), metadata.summary.as_bytes()),
        (
            b"classification_explanation".as_slice(),
            metadata.classification_explanation.as_bytes(),
        ),
        (
            b"logical_resource_uri".as_slice(),
            metadata.logical_resource_uri.as_bytes(),
        ),
        (
            b"generator_model".as_slice(),
            metadata.generator_model.as_bytes(),
        ),
    ] {
        hash_review_component(&mut hasher, label, value)?;
    }
    hash_review_component(
        &mut hasher,
        b"classification_confidence",
        &metadata.classification_confidence.to_bits().to_be_bytes(),
    )?;

    let mut statement = tx.prepare(
        "SELECT ordinal,heading_or_page,text,text_sha256
         FROM chunks WHERE concept_id=?1 ORDER BY ordinal",
    )?;
    let mut rows = statement.query([concept_id.to_string()])?;
    let mut total_chunks = 0_usize;
    while let Some(row) = rows.next()? {
        let ordinal = row.get::<_, u32>(0)?;
        let heading_or_page = row.get::<_, String>(1)?;
        let text = row.get::<_, String>(2)?;
        let text_sha256 = row.get::<_, String>(3)?;
        hash_review_component(&mut hasher, b"chunk.ordinal", &ordinal.to_be_bytes())?;
        hash_review_component(
            &mut hasher,
            b"chunk.heading_or_page",
            heading_or_page.as_bytes(),
        )?;
        hash_review_component(&mut hasher, b"chunk.text", text.as_bytes())?;
        hash_review_component(&mut hasher, b"chunk.text_sha256", text_sha256.as_bytes())?;
        total_chunks = total_chunks
            .checked_add(1)
            .context("review evidence chunk count exceeds this platform's capacity")?;
    }
    drop(rows);
    drop(statement);

    Ok(Some(PendingReviewSnapshot {
        source_document_id: metadata.source_document_id,
        collection_id: metadata.collection_id,
        source_sha256: metadata.source_sha256,
        source_revision: metadata.source_revision,
        review_version: ReviewVersionToken::from_digest(hasher.finalize().into()),
        total_chunks,
    }))
}

fn hash_review_component(hasher: &mut Sha256, label: &[u8], value: &[u8]) -> Result<()> {
    let label_len = u64::try_from(label.len()).context("review hash label is too large")?;
    let value_len = u64::try_from(value.len()).context("review hash value is too large")?;
    hasher.update(label_len.to_be_bytes());
    hasher.update(label);
    hasher.update(value_len.to_be_bytes());
    hasher.update(value);
    Ok(())
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("could not open SQLite database {}", path.display()))?;
        Self::initialize(connection, Some(path.to_path_buf()))
    }

    pub fn in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?, None)
    }

    fn initialize(mut connection: Connection, path: Option<PathBuf>) -> Result<Self> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        if path.is_some() {
            connection.pragma_update(None, "journal_mode", "WAL")?;
            connection.pragma_update(None, "synchronous", "NORMAL")?;
        }
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version < 1 {
            let tx = connection.transaction()?;
            tx.execute_batch(MIGRATION_1)?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit()?;
        }
        if version < 2 {
            let tx = connection.transaction()?;
            tx.execute_batch(MIGRATION_2)?;
            tx.pragma_update(None, "user_version", 2)?;
            tx.commit()?;
        }
        if version < 3 {
            let tx = connection.transaction()?;
            tx.execute_batch(MIGRATION_3)?;
            tx.pragma_update(None, "user_version", 3)?;
            tx.commit()?;
        }
        if version < 4 {
            let tx = connection.transaction()?;
            tx.execute_batch(MIGRATION_4)?;
            tx.pragma_update(None, "user_version", 4)?;
            tx.commit()?;
        }
        if version < 5 {
            let tx = connection.transaction()?;
            tx.execute_batch(MIGRATION_5)?;
            tx.pragma_update(None, "user_version", 5)?;
            tx.commit()?;
        }
        if version > 5 {
            bail!("database schema {version} is newer than this application supports");
        }
        let database = Self {
            inner: Arc::new(Mutex::new(connection)),
            publication_lock: Arc::new(Mutex::new(())),
            disclosure_gate: DisclosureGate::default(),
            path,
        };
        database.recover_interrupted_jobs()?;
        Ok(database)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn connection(&self) -> Result<DatabaseConnectionGuard<'_>> {
        // Conservatively classify every SQLite operation as a potential
        // disclosure-state mutation. This keeps future write methods inside
        // the barrier by default; the final leased revalidation is the sole
        // explicit exception. Callers already keep this synchronous guard
        // scoped to one SQLite operation.
        let mutation = self.disclosure_gate.acquire_mutation();
        let connection = self
            .inner
            .lock()
            .map_err(|_| anyhow!("database lock is poisoned"))?;
        Ok(DatabaseConnectionGuard {
            connection,
            _mutation: mutation,
        })
    }

    fn connection_under_disclosure(
        &self,
        lease: &DisclosureLease,
    ) -> Result<MutexGuard<'_, Connection>> {
        if !self.disclosure_gate.owns(lease) {
            bail!("disclosure lease does not protect this database");
        }
        self.inner
            .lock()
            .map_err(|_| anyhow!("database lock is poisoned"))
    }

    /// Returns the barrier shared with the LAN authorization snapshot.
    pub fn disclosure_gate(&self) -> DisclosureGate {
        self.disclosure_gate.clone()
    }

    pub(crate) fn publication_guard(&self) -> Result<MutexGuard<'_, ()>> {
        self.publication_lock
            .lock()
            .map_err(|_| anyhow!("OKF publication lock is poisoned"))
    }

    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .connection()?
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    /// Records the start boundary of an idempotent full collection scan. This
    /// table contains operational counters only; source paths and content stay
    /// in their existing authoritative tables.
    pub fn start_collection_maintenance(&self, collection_id: Uuid) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.connection()?.execute(
            "INSERT INTO collection_maintenance(collection_id,last_started_at,status)
             VALUES (?1,?2,'never')
             ON CONFLICT(collection_id) DO UPDATE SET last_started_at=excluded.last_started_at",
            params![collection_id.to_string(), now],
        )?;
        Ok(())
    }

    pub fn finish_collection_maintenance(
        &self,
        collection_id: Uuid,
        result: &CollectionMaintenanceResult,
    ) -> Result<()> {
        result.validate()?;
        let now = Utc::now().to_rfc3339();
        let analyzed = sql_count(result.counts.analyzed)?;
        let unchanged = sql_count(result.counts.unchanged)?;
        let renamed = sql_count(result.counts.renamed)?;
        let deleted = sql_count(result.counts.deleted)?;
        let failed = sql_count(result.counts.failed)?;
        let successful_at =
            (result.status == CollectionMaintenanceStatus::Success).then_some(now.as_str());
        self.connection()?.execute(
            "INSERT INTO collection_maintenance(
               collection_id,last_started_at,last_finished_at,last_success_at,status,
               analyzed_count,unchanged_count,renamed_count,deleted_count,failed_count,
               issue_code,issue_summary)
             VALUES (?1,?2,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(collection_id) DO UPDATE SET
               last_finished_at=excluded.last_finished_at,
               last_success_at=coalesce(excluded.last_success_at,last_success_at),
               status=excluded.status,
               analyzed_count=excluded.analyzed_count,
               unchanged_count=excluded.unchanged_count,
               renamed_count=excluded.renamed_count,
               deleted_count=excluded.deleted_count,
               failed_count=excluded.failed_count,
               issue_code=excluded.issue_code,
               issue_summary=excluded.issue_summary",
            params![
                collection_id.to_string(),
                now,
                successful_at,
                result.status.as_str(),
                analyzed,
                unchanged,
                renamed,
                deleted,
                failed,
                result.issue_code,
                result.issue_summary,
            ],
        )?;
        Ok(())
    }

    pub fn collection_maintenance(
        &self,
        collection_id: Uuid,
    ) -> Result<Option<CollectionMaintenanceRecord>> {
        self.connection()?
            .query_row(
                "SELECT collection_id,last_started_at,last_finished_at,last_success_at,status,
                 analyzed_count,unchanged_count,renamed_count,deleted_count,failed_count,
                 issue_code,issue_summary
                 FROM collection_maintenance WHERE collection_id=?1",
                [collection_id.to_string()],
                collection_maintenance_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Marks work from a previous process as interrupted. Ingest sources become
    /// retryable on the next filesystem scan, while reanalysis restores the
    /// existing draft to human review instead of leaving it non-approvable.
    fn recover_interrupted_jobs(&self) -> Result<usize> {
        const INTERRUPTION: &str = "interrupted by a previous application shutdown";
        let now = Utc::now().to_rfc3339();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;

        tx.execute(
            "UPDATE concepts SET status='needs_review',reviewed_at=NULL,updated_at=?1
             WHERE status='enriched' AND source_document_id IN (
               SELECT j.source_document_id FROM jobs j
               WHERE j.kind='reanalyze' AND j.state IN ('queued','running')
                 AND j.source_document_id IS NOT NULL
             )",
            params![now],
        )?;
        tx.execute(
            "UPDATE source_documents SET status='needs_review',last_error=?1,updated_at=?2
             WHERE status='enriched' AND id IN (
               SELECT j.source_document_id FROM jobs j
               WHERE j.kind='reanalyze' AND j.state IN ('queued','running')
                 AND j.source_document_id IS NOT NULL
             ) AND concept_id IS NOT NULL",
            params![INTERRUPTION, now],
        )?;
        tx.execute(
            "UPDATE concepts SET status='failed',reviewed_at=NULL,updated_at=?1
             WHERE status='needs_review' AND source_document_id IN (
               SELECT j.source_document_id FROM jobs j
               WHERE j.kind='ingest' AND j.state IN ('queued','running')
                 AND j.source_document_id IS NOT NULL
             )",
            params![now],
        )?;
        tx.execute(
            "UPDATE source_documents SET status='failed',last_error=?1,updated_at=?2
             WHERE status IN ('detected','extracted','enriched','needs_review') AND id IN (
               SELECT j.source_document_id FROM jobs j
               WHERE j.kind='ingest' AND j.state IN ('queued','running')
                 AND j.source_document_id IS NOT NULL
             )",
            params![INTERRUPTION, now],
        )?;
        let recovered = tx.execute(
            "UPDATE jobs SET state='failed',last_error=?1,updated_at=?2
             WHERE state IN ('queued','running') AND kind!='publish'",
            params![INTERRUPTION, now],
        )?;
        tx.commit()?;
        Ok(recovered)
    }

    pub fn create_collection(
        &self,
        name: impl Into<String>,
        source_folder: impl AsRef<Path>,
        wiki_folder: impl AsRef<Path>,
        mut policy: CollectionPolicy,
    ) -> Result<CollectionRecord> {
        policy.normalize();
        let record = CollectionRecord {
            id: Uuid::new_v4(),
            name: name.into().trim().to_owned(),
            source_folder: absolute_path(source_folder.as_ref())?,
            wiki_folder: absolute_path(wiki_folder.as_ref())?,
            policy,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        if record.name.is_empty() {
            bail!("collection name must not be empty");
        }
        self.connection()?.execute(
            "INSERT INTO collections
             (id,name,source_folder,wiki_folder,local_only,peer_shareable,allow_external_ai,internet_public,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                record.id.to_string(),
                record.name,
                path_text(&record.source_folder),
                path_text(&record.wiki_folder),
                record.policy.local_only,
                record.policy.peer_shareable,
                record.policy.allow_external_ai,
                record.policy.internet_public,
                record.created_at.to_rfc3339(),
                record.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(record)
    }

    pub fn update_collection_policy(&self, id: Uuid, mut policy: CollectionPolicy) -> Result<()> {
        policy.normalize();
        let now = Utc::now();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let previous_public = tx
            .query_row(
                "SELECT internet_public FROM collections WHERE id=?1",
                [id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .optional()?;
        let count = tx.execute(
            "UPDATE collections SET local_only=?2, peer_shareable=?3, allow_external_ai=?4,
             internet_public=?5, updated_at=?6 WHERE id=?1",
            params![
                id.to_string(),
                policy.local_only,
                policy.peer_shareable,
                policy.allow_external_ai,
                policy.internet_public,
                now.to_rfc3339()
            ],
        )?;
        ensure_changed(count, "collection", id)?;
        if previous_public != Some(policy.internet_public) {
            tx.execute(
                "INSERT INTO public_collection_profiles
                 (collection_id,description,languages_json,manifest_sequence,enabled_at,updated_at)
                 VALUES (?1,'','[]',1,?2,?3)
                 ON CONFLICT(collection_id) DO UPDATE SET
                   manifest_sequence=manifest_sequence+1,
                   enabled_at=?2,
                   updated_at=?3",
                params![
                    id.to_string(),
                    policy.internet_public.then(|| now.to_rfc3339()),
                    now.to_rfc3339(),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn public_collection_profile(
        &self,
        collection_id: Uuid,
    ) -> Result<Option<PublicCollectionProfileRecord>> {
        self.connection()?
            .query_row(
                "SELECT collection_id,description,languages_json,manifest_sequence,enabled_at,updated_at
                 FROM public_collection_profiles WHERE collection_id=?1",
                [collection_id.to_string()],
                public_collection_profile_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn update_public_collection_profile(
        &self,
        collection_id: Uuid,
        description: &str,
        languages: &[String],
    ) -> Result<PublicCollectionProfileRecord> {
        let description = description.trim();
        if description.chars().count() > 1_000 || description.chars().any(char::is_control) {
            bail!("public collection description is invalid");
        }
        if languages.len() > 16
            || languages.iter().any(|language| {
                let language = language.trim();
                language.is_empty() || language.len() > 16 || language.chars().any(char::is_control)
            })
        {
            bail!("public collection languages are invalid");
        }
        let mut normalized = languages
            .iter()
            .map(|language| language.trim().to_ascii_lowercase())
            .collect::<Vec<_>>();
        normalized.sort();
        normalized.dedup();
        let now = Utc::now();
        let count = self.connection()?.execute(
            "UPDATE public_collection_profiles SET description=?2,languages_json=?3,
             manifest_sequence=manifest_sequence+1,updated_at=?4 WHERE collection_id=?1",
            params![
                collection_id.to_string(),
                description,
                serde_json::to_string(&normalized)?,
                now.to_rfc3339(),
            ],
        )?;
        ensure_changed(count, "public collection profile", collection_id)?;
        self.public_collection_profile(collection_id)?
            .context("public collection profile disappeared after update")
    }

    pub fn bump_public_manifest_sequence(&self, collection_id: Uuid) -> Result<Option<u64>> {
        let now = Utc::now();
        let connection = self.connection()?;
        let count = connection.execute(
            "UPDATE public_collection_profiles SET manifest_sequence=manifest_sequence+1,
             updated_at=?2 WHERE collection_id=?1 AND EXISTS(
               SELECT 1 FROM collections c WHERE c.id=?1 AND c.internet_public=1
             )",
            params![collection_id.to_string(), now.to_rfc3339()],
        )?;
        if count == 0 {
            return Ok(None);
        }
        connection
            .query_row(
                "SELECT manifest_sequence FROM public_collection_profiles WHERE collection_id=?1",
                [collection_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn update_collection_source_folder(
        &self,
        id: Uuid,
        source_folder: impl AsRef<Path>,
    ) -> Result<()> {
        let source_folder = absolute_path(source_folder.as_ref())?;
        let count = self.connection()?.execute(
            "UPDATE collections SET source_folder=?2, updated_at=?3 WHERE id=?1",
            params![
                id.to_string(),
                path_text(&source_folder),
                Utc::now().to_rfc3339()
            ],
        )?;
        ensure_changed(count, "collection", id)
    }

    pub fn collection(&self, id: Uuid) -> Result<Option<CollectionRecord>> {
        self.connection()?
            .query_row(
                "SELECT id,name,source_folder,wiki_folder,local_only,peer_shareable,
                 allow_external_ai,internet_public,created_at,updated_at FROM collections WHERE id=?1",
                [id.to_string()],
                collection_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_collections(&self) -> Result<Vec<CollectionRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,name,source_folder,wiki_folder,local_only,peer_shareable,
             allow_external_ai,internet_public,created_at,updated_at FROM collections ORDER BY name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], collection_from_row)?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    pub fn upsert_federation_index(
        &self,
        peer_id: &str,
        multiaddr: &str,
        enabled: bool,
        source: &str,
    ) -> Result<()> {
        if peer_id.is_empty()
            || peer_id.len() > 128
            || multiaddr.is_empty()
            || multiaddr.len() > 500
            || !matches!(source, "bootstrap" | "community")
        {
            bail!("federation index configuration is invalid");
        }
        let now = Utc::now().to_rfc3339();
        self.connection()?.execute(
            "INSERT INTO federation_indexes(peer_id,multiaddr,enabled,source,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?5)
             ON CONFLICT(peer_id) DO UPDATE SET multiaddr=?2,enabled=?3,source=?4,updated_at=?5",
            params![peer_id, multiaddr, enabled, source, now],
        )?;
        Ok(())
    }

    pub fn upsert_bootstrap_federation_index(
        &self,
        peer_id: &str,
        multiaddr: &str,
        registry_version: u32,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        if peer_id.is_empty()
            || peer_id.len() > 128
            || multiaddr.is_empty()
            || multiaddr.len() > 500
            || registry_version == 0
            || expires_at <= Utc::now()
        {
            bail!("bootstrap federation index metadata is invalid");
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let existing = transaction
            .query_row(
                "SELECT multiaddr,source,registry_version,expires_at
                 FROM federation_indexes WHERE peer_id=?1",
                [peer_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        let expiry = expires_at.to_rfc3339();
        if let Some((known_address, known_source, known_version, known_expiry)) = existing {
            if known_version > registry_version {
                bail!("bootstrap federation index registry downgrade rejected");
            }
            if known_source == "bootstrap" && known_version == registry_version {
                if known_address == multiaddr && known_expiry.as_deref() == Some(expiry.as_str()) {
                    return Ok(());
                }
                bail!("bootstrap federation index mutation requires a newer registry version");
            }
        }
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO federation_indexes
             (peer_id,multiaddr,enabled,source,registry_version,expires_at,created_at,updated_at)
             VALUES (?1,?2,1,'bootstrap',?3,?4,?5,?5)
             ON CONFLICT(peer_id) DO UPDATE SET multiaddr=?2,enabled=1,source='bootstrap',
               registry_version=?3,expires_at=?4,updated_at=?5",
            params![peer_id, multiaddr, registry_version, expiry, now],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn list_federation_indexes(&self) -> Result<Vec<FederationIndexRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT peer_id,multiaddr,enabled,source,registry_version,expires_at,created_at,updated_at
             FROM federation_indexes ORDER BY source,peer_id LIMIT 64",
        )?;
        statement
            .query_map([], federation_index_from_row)?
            .collect::<rusqlite::Result<_>>()
            .map_err(Into::into)
    }

    pub fn set_federation_index_enabled(&self, peer_id: &str, enabled: bool) -> Result<()> {
        let count = self.connection()?.execute(
            "UPDATE federation_indexes SET enabled=?2,updated_at=?3 WHERE peer_id=?1",
            params![peer_id, enabled, Utc::now().to_rfc3339()],
        )?;
        if count == 0 {
            bail!("federation index does not exist");
        }
        Ok(())
    }

    pub fn set_public_publisher_blocked(&self, publisher_id: &str, blocked: bool) -> Result<()> {
        let publisher_id = publisher_id.trim();
        if publisher_id.is_empty()
            || publisher_id.len() > 128
            || publisher_id.chars().any(char::is_control)
        {
            bail!("public publisher identity is invalid");
        }
        let connection = self.connection()?;
        if blocked {
            connection.execute(
                "INSERT INTO public_publisher_blocks(publisher_id,blocked_at) VALUES (?1,?2)
                 ON CONFLICT(publisher_id) DO UPDATE SET blocked_at=?2",
                params![publisher_id, Utc::now().to_rfc3339()],
            )?;
        } else {
            connection.execute(
                "DELETE FROM public_publisher_blocks WHERE publisher_id=?1",
                [publisher_id],
            )?;
        }
        Ok(())
    }

    pub fn public_publisher_is_blocked(&self, publisher_id: &str) -> Result<bool> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT 1 FROM public_publisher_blocks WHERE publisher_id=?1",
                [publisher_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn list_blocked_public_publishers(&self) -> Result<Vec<String>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT publisher_id FROM public_publisher_blocks ORDER BY publisher_id LIMIT 1024",
        )?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()
            .map_err(Into::into)
    }

    pub fn register_source(
        &self,
        collection_id: Uuid,
        path: impl AsRef<Path>,
        sha256: &str,
        source_format: &str,
        byte_size: u64,
    ) -> Result<SourceRegistration> {
        let source_path = absolute_path(path.as_ref())?;
        let source_path_text = path_text(&source_path);
        let now = Utc::now().to_rfc3339();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;

        if let Some((id, old_hash, status)) =
            source_registration_by_path(&tx, collection_id, &source_path, &source_path_text)?
        {
            let id = parse_uuid(&id)?;
            if old_hash == sha256 {
                if matches!(status.as_str(), "needs_review" | "publishing" | "published") {
                    tx.execute(
                        "UPDATE source_documents SET source_path=?2,updated_at=?3,deleted_at=NULL
                         WHERE id=?1",
                        params![id.to_string(), source_path_text, now],
                    )?;
                    tx.commit()?;
                    return Ok(SourceRegistration::Unchanged(id));
                }
                if status != "deleted" {
                    // A crash or processing failure must be retryable even when
                    // the file bytes did not change. Preserve the source revision
                    // while resetting any partial index state.
                    withdraw_source(&tx, id)?;
                    tx.execute(
                        "UPDATE source_documents SET source_path=?2,source_format=?3,byte_size=?4,
                         page_count=0,character_count=0,status='detected',last_error=NULL,
                         updated_at=?5,deleted_at=NULL WHERE id=?1",
                        params![
                            id.to_string(),
                            source_path_text,
                            source_format,
                            byte_size,
                            now
                        ],
                    )?;
                    tx.commit()?;
                    return Ok(SourceRegistration::Changed(id));
                }
            }
            let hash_changed = old_hash != sha256;
            withdraw_source(&tx, id)?;
            tx.execute(
                "UPDATE source_documents SET source_path=?2,source_sha256=?3,source_format=?4,
                 byte_size=?5,
                 page_count=0,character_count=0,status='detected',revision=revision+1,
                 last_error=NULL,updated_at=?6,deleted_at=NULL WHERE id=?1",
                params![
                    id.to_string(),
                    source_path_text,
                    sha256,
                    source_format,
                    byte_size,
                    now
                ],
            )?;
            tx.commit()?;
            return Ok(if hash_changed {
                SourceRegistration::Replaced {
                    id,
                    previous_source_sha256: old_hash,
                }
            } else {
                SourceRegistration::Changed(id)
            });
        }

        // A hash alone cannot distinguish a rename from two identical files.
        // Preserve identity only when there is exactly one prior record and its
        // old path is definitely gone. Existing or unreadable old paths are
        // treated as copies and receive their own source identity.
        let same_hash_sources = {
            let mut statement = tx.prepare(
                "SELECT id,status,source_path FROM source_documents
                 WHERE collection_id=?1 AND source_sha256=?2
                 ORDER BY updated_at DESC",
            )?;
            statement
                .query_map(params![collection_id.to_string(), sha256], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        PathBuf::from(row.get::<_, String>(2)?),
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if let [(id, status, previous_path)] = same_hash_sources.as_slice()
            && path_is_definitely_missing(previous_path)
        {
            let id = parse_uuid(id)?;
            let needs_processing =
                !matches!(status.as_str(), "needs_review" | "publishing" | "published");
            if needs_processing {
                withdraw_source(&tx, id)?;
                tx.execute(
                    "UPDATE source_documents SET source_path=?2,source_format=?3,byte_size=?4,
                     page_count=0,character_count=0,deleted_at=NULL,status='detected',
                     last_error=NULL,updated_at=?5 WHERE id=?1",
                    params![
                        id.to_string(),
                        source_path_text,
                        source_format,
                        byte_size,
                        now
                    ],
                )?;
            } else {
                tx.execute(
                    "UPDATE source_documents SET source_path=?2,source_format=?3,byte_size=?4,
                     deleted_at=NULL,updated_at=?5 WHERE id=?1",
                    params![
                        id.to_string(),
                        source_path_text,
                        source_format,
                        byte_size,
                        now
                    ],
                )?;
            }
            tx.commit()?;
            return Ok(if needs_processing {
                SourceRegistration::Changed(id)
            } else {
                SourceRegistration::Renamed(id)
            });
        }

        let id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO source_documents
             (id,collection_id,source_path,source_sha256,source_format,byte_size,status,
              revision,discovered_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,'detected',1,?7,?7)",
            params![
                id.to_string(),
                collection_id.to_string(),
                source_path_text,
                sha256,
                source_format,
                byte_size,
                now,
            ],
        )?;
        tx.commit()?;
        Ok(SourceRegistration::New(id))
    }

    pub fn source_document(&self, id: Uuid) -> Result<Option<SourceDocumentRecord>> {
        self.connection()?
            .query_row(
                "SELECT id,collection_id,source_path,source_sha256,source_format,byte_size,
                 page_count,character_count,status,revision,concept_id,last_error,discovered_at,
                 updated_at,deleted_at FROM source_documents WHERE id=?1",
                [id.to_string()],
                source_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_sources(&self, collection_id: Uuid) -> Result<Vec<SourceDocumentRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,collection_id,source_path,source_sha256,source_format,byte_size,
             page_count,character_count,status,revision,concept_id,last_error,discovered_at,
             updated_at,deleted_at FROM source_documents WHERE collection_id=?1 ORDER BY source_path",
        )?;
        let rows = statement.query_map([collection_id.to_string()], source_from_row)?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    pub fn mark_extracted(&self, id: Uuid, pages: u32, characters: u64) -> Result<()> {
        self.update_source_state(id, DocumentStatus::Extracted, pages, characters, None)
    }

    /// Claims the extraction transition only for the exact registered revision.
    /// The status predicate also prevents duplicate workers from moving a
    /// completed revision backwards through the pipeline.
    pub fn mark_extracted_if_current(
        &self,
        id: Uuid,
        source_sha256: &str,
        revision: u32,
        pages: u32,
        characters: u64,
    ) -> Result<bool> {
        let changed = self.connection()?.execute(
            "UPDATE source_documents SET status='extracted',page_count=?4,character_count=?5,
             last_error=NULL,updated_at=?6
             WHERE id=?1 AND source_sha256=?2 AND revision=?3 AND status='detected'",
            params![
                id.to_string(),
                source_sha256,
                revision,
                pages,
                characters,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn mark_enriched_if_current(
        &self,
        id: Uuid,
        source_sha256: &str,
        revision: u32,
    ) -> Result<bool> {
        let changed = self.connection()?.execute(
            "UPDATE source_documents SET status='enriched',last_error=NULL,updated_at=?4
             WHERE id=?1 AND source_sha256=?2 AND revision=?3 AND status='extracted'",
            params![
                id.to_string(),
                source_sha256,
                revision,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn mark_source_status(&self, id: Uuid, status: DocumentStatus) -> Result<()> {
        let current = self
            .source_document(id)?
            .ok_or_else(|| anyhow!("source document {id} does not exist"))?;
        self.update_source_state(
            id,
            status,
            current.page_count,
            current.character_count,
            None,
        )
    }

    pub fn mark_source_failed(&self, id: Uuid, error: impl AsRef<str>) -> Result<()> {
        let error: String = error.as_ref().chars().take(2_000).collect();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let count = tx.execute(
            "UPDATE source_documents SET status='failed',last_error=?2,updated_at=?3 WHERE id=?1",
            params![id.to_string(), error, Utc::now().to_rfc3339()],
        )?;
        ensure_changed(count, "source document", id)?;
        tx.execute(
            "UPDATE concepts SET status='failed',reviewed_at=NULL,updated_at=?2
             WHERE source_document_id=?1",
            params![id.to_string(), Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Marks a processing failure only if the source still points at the
    /// revision owned by that worker. A stale task therefore cannot overwrite a
    /// newer preflight's detected/review state.
    pub fn mark_source_failed_if_current(
        &self,
        id: Uuid,
        source_sha256: &str,
        revision: u32,
        error: impl AsRef<str>,
    ) -> Result<bool> {
        let error: String = error.as_ref().chars().take(2_000).collect();
        let now = Utc::now().to_rfc3339();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let changed = tx.execute(
            "UPDATE source_documents SET status='failed',last_error=?4,updated_at=?5
             WHERE id=?1 AND source_sha256=?2 AND revision=?3
             AND status NOT IN ('published','deleted')",
            params![id.to_string(), source_sha256, revision, error, now],
        )?;
        if changed == 0 {
            return Ok(false);
        }
        tx.execute(
            "UPDATE concepts SET status='failed',reviewed_at=NULL,updated_at=?2
             WHERE source_document_id=?1",
            params![id.to_string(), now],
        )?;
        tx.commit()?;
        Ok(true)
    }

    fn update_source_state(
        &self,
        id: Uuid,
        status: DocumentStatus,
        pages: u32,
        characters: u64,
        error: Option<&str>,
    ) -> Result<()> {
        let count = self.connection()?.execute(
            "UPDATE source_documents SET status=?2,page_count=?3,character_count=?4,last_error=?5,
             updated_at=?6 WHERE id=?1",
            params![
                id.to_string(),
                status.to_string(),
                pages,
                characters,
                error,
                Utc::now().to_rfc3339()
            ],
        )?;
        ensure_changed(count, "source document", id)
    }

    pub fn save_enrichment(
        &self,
        source_document_id: Uuid,
        mut draft: EnrichmentDraft,
        node_id: &str,
        generator_model: &str,
    ) -> Result<ConceptRecord> {
        draft.sanitize();
        validate_draft(&draft)?;
        let source = self
            .source_document(source_document_id)?
            .ok_or_else(|| anyhow!("source document {source_document_id} does not exist"))?;
        let id = source.concept_id.unwrap_or_else(Uuid::new_v4);
        let now = Utc::now();
        let logical_resource_uri = format!("urn:airwiki:{node_id}:{id}");
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO concepts
             (id,source_document_id,collection_id,concept_type,title,description,language,tags_json,
              entities_json,links_json,summary,classification_confidence,classification_explanation,
              logical_resource_uri,generator_model,status,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'needs_review',?16,?16)
             ON CONFLICT(source_document_id) DO UPDATE SET
              concept_type=excluded.concept_type,title=excluded.title,description=excluded.description,
              language=excluded.language,tags_json=excluded.tags_json,entities_json=excluded.entities_json,
              links_json=excluded.links_json,summary=excluded.summary,
              classification_confidence=excluded.classification_confidence,
              classification_explanation=excluded.classification_explanation,
              logical_resource_uri=excluded.logical_resource_uri,generator_model=excluded.generator_model,
              status='needs_review',reviewed_at=NULL,updated_at=excluded.updated_at",
            params![
                id.to_string(),
                source_document_id.to_string(),
                source.collection_id.to_string(),
                draft.concept_type.to_string(),
                draft.title,
                draft.description,
                draft.language,
                serde_json::to_string(&draft.tags)?,
                serde_json::to_string(&draft.entities)?,
                serde_json::to_string(&draft.links)?,
                draft.summary,
                draft.classification_confidence,
                draft.classification_explanation,
                logical_resource_uri,
                generator_model,
                now.to_rfc3339(),
            ],
        )?;
        connection.execute(
            "UPDATE source_documents SET concept_id=?2,status='needs_review',updated_at=?3 WHERE id=?1",
            params![source_document_id.to_string(), id.to_string(), now.to_rfc3339()],
        )?;
        drop(connection);
        self.concept(id)?
            .ok_or_else(|| anyhow!("saved concept disappeared"))
    }

    /// Atomically saves metadata only while the exact source revision remains
    /// in the `Enriched` stage claimed by the caller.
    pub fn save_enrichment_if_current(
        &self,
        source_document_id: Uuid,
        source_sha256: &str,
        revision: u32,
        mut draft: EnrichmentDraft,
        node_id: &str,
        generator_model: &str,
    ) -> Result<Option<ConceptRecord>> {
        draft.sanitize();
        validate_draft(&draft)?;
        let now = Utc::now();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let source = tx
            .query_row(
                "SELECT collection_id,concept_id FROM source_documents
                 WHERE id=?1 AND source_sha256=?2 AND revision=?3 AND status='enriched'",
                params![source_document_id.to_string(), source_sha256, revision],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let Some((collection_id, concept_id)) = source else {
            return Ok(None);
        };
        let collection_id = parse_uuid(&collection_id)?;
        let id = concept_id
            .map(|id| parse_uuid(&id))
            .transpose()?
            .unwrap_or_else(Uuid::new_v4);
        let logical_resource_uri = format!("urn:airwiki:{node_id}:{id}");
        tx.execute(
            "INSERT INTO concepts
             (id,source_document_id,collection_id,concept_type,title,description,language,tags_json,
              entities_json,links_json,summary,classification_confidence,classification_explanation,
              logical_resource_uri,generator_model,status,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'needs_review',?16,?16)
             ON CONFLICT(source_document_id) DO UPDATE SET
              concept_type=excluded.concept_type,title=excluded.title,description=excluded.description,
              language=excluded.language,tags_json=excluded.tags_json,entities_json=excluded.entities_json,
              links_json=excluded.links_json,summary=excluded.summary,
              classification_confidence=excluded.classification_confidence,
              classification_explanation=excluded.classification_explanation,
              logical_resource_uri=excluded.logical_resource_uri,generator_model=excluded.generator_model,
              status='needs_review',reviewed_at=NULL,updated_at=excluded.updated_at",
            params![
                id.to_string(),
                source_document_id.to_string(),
                collection_id.to_string(),
                draft.concept_type.to_string(),
                draft.title,
                draft.description,
                draft.language,
                serde_json::to_string(&draft.tags)?,
                serde_json::to_string(&draft.entities)?,
                serde_json::to_string(&draft.links)?,
                draft.summary,
                draft.classification_confidence,
                draft.classification_explanation,
                logical_resource_uri,
                generator_model,
                now.to_rfc3339(),
            ],
        )?;
        let changed = tx.execute(
            "UPDATE source_documents SET concept_id=?4,status='needs_review',updated_at=?5
             WHERE id=?1 AND source_sha256=?2 AND revision=?3 AND status='enriched'",
            params![
                source_document_id.to_string(),
                source_sha256,
                revision,
                id.to_string(),
                now.to_rfc3339()
            ],
        )?;
        if changed != 1 {
            return Ok(None);
        }
        tx.commit()?;
        drop(connection);
        self.concept(id)
    }

    pub fn concept(&self, id: Uuid) -> Result<Option<ConceptRecord>> {
        self.connection()?
            .query_row(
                "SELECT id,source_document_id,collection_id,concept_type,title,description,language,
                 tags_json,entities_json,links_json,summary,classification_confidence,
                 classification_explanation,logical_resource_uri,generator_model,status,reviewed_at,
                 created_at,updated_at FROM concepts WHERE id=?1",
                [id.to_string()],
                concept_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_concepts_for_review(&self) -> Result<Vec<ConceptRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,source_document_id,collection_id,concept_type,title,description,language,
             tags_json,entities_json,links_json,summary,classification_confidence,
             classification_explanation,logical_resource_uri,generator_model,status,reviewed_at,
             created_at,updated_at FROM concepts WHERE status='needs_review' ORDER BY updated_at",
        )?;
        let rows = statement.query_map([], concept_from_row)?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    /// Exclusively claims a pending concept for reanalysis without deleting
    /// its current human-review draft or chunks. `Enriched` is used as the
    /// transient, non-publishable state so the normal approval predicate
    /// cannot race the local model.
    pub fn begin_review_reanalysis(&self, concept_id: Uuid) -> Result<ReviewReanalysisClaim> {
        let now = Utc::now();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let source = tx
            .query_row(
                "SELECT sd.id,sd.collection_id,sd.source_path,sd.source_sha256,sd.source_format,
                        sd.byte_size,sd.revision
                 FROM concepts co
                 JOIN source_documents sd ON sd.id=co.source_document_id
                 WHERE co.id=?1 AND co.status='needs_review' AND sd.status='needs_review'
                   AND sd.concept_id=co.id",
                [concept_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, u64>(5)?,
                        row.get::<_, u32>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            source_document_id,
            collection_id,
            source_path,
            source_sha256,
            source_format,
            byte_size,
            revision,
        )) = source
        else {
            bail!("concept {concept_id} is no longer awaiting review");
        };
        let source_document_id = parse_uuid(&source_document_id)?;
        let collection_id = parse_uuid(&collection_id)?;
        let job_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO jobs(id,source_document_id,kind,state,attempts,created_at,updated_at)
             VALUES (?1,?2,'reanalyze','running',1,?3,?3)",
            params![
                job_id.to_string(),
                source_document_id.to_string(),
                now.to_rfc3339()
            ],
        )?;
        let concept_changed = tx.execute(
            "UPDATE concepts SET status='enriched',reviewed_at=NULL,updated_at=?2
             WHERE id=?1 AND status='needs_review'",
            params![concept_id.to_string(), now.to_rfc3339()],
        )?;
        let source_changed = tx.execute(
            "UPDATE source_documents SET status='enriched',last_error=NULL,updated_at=?2
             WHERE id=?1 AND status='needs_review'",
            params![source_document_id.to_string(), now.to_rfc3339()],
        )?;
        if concept_changed != 1 || source_changed != 1 {
            bail!("review state changed while claiming concept {concept_id} for reanalysis");
        }
        tx.commit()?;
        Ok(ReviewReanalysisClaim {
            job_id,
            concept_id,
            source_document_id,
            collection_id,
            source_path: PathBuf::from(source_path),
            source_sha256,
            source_format,
            byte_size,
            revision,
        })
    }

    /// Atomically replaces the machine-generated draft and searchable chunks
    /// only if the exact claimed source revision is still current. Completion
    /// always returns the concept to human review; it can never publish it.
    pub fn complete_review_reanalysis_if_current(
        &self,
        claim: &ReviewReanalysisClaim,
        mut draft: EnrichmentDraft,
        generator_model: &str,
        chunks: &[StoredChunk],
    ) -> Result<bool> {
        draft.sanitize();
        validate_draft(&draft)?;
        if generator_model.trim().is_empty() {
            bail!("generator model must not be empty");
        }
        if chunks.is_empty() {
            bail!("reanalysis must produce at least one searchable chunk");
        }
        if chunks
            .iter()
            .any(|chunk| chunk.embedding.len() != EMBEDDING_DIMENSIONS)
        {
            bail!("every embedding must have {EMBEDDING_DIMENSIONS} dimensions");
        }
        if chunks.iter().any(|chunk| {
            chunk.concept_id != claim.concept_id
                || chunk.source_document_id != claim.source_document_id
                || chunk.collection_id != claim.collection_id
                || chunk.source_revision != claim.revision
        }) {
            bail!("reanalysis chunk ownership does not match the claimed concept");
        }

        let now = Utc::now();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let current = tx.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM concepts co
               JOIN source_documents sd ON sd.id=co.source_document_id
               JOIN jobs j ON j.source_document_id=sd.id
               WHERE co.id=?1 AND co.status='enriched'
                 AND sd.id=?2 AND sd.status='enriched'
                 AND sd.collection_id=?3 AND sd.source_sha256=?4 AND sd.revision=?5
                 AND sd.concept_id=co.id
                 AND j.id=?6 AND j.kind='reanalyze' AND j.state='running'
             )",
            params![
                claim.concept_id.to_string(),
                claim.source_document_id.to_string(),
                claim.collection_id.to_string(),
                claim.source_sha256,
                claim.revision,
                claim.job_id.to_string(),
            ],
            |row| row.get::<_, bool>(0),
        )?;
        if !current {
            return Ok(false);
        }

        delete_chunks_for_concept(&tx, claim.concept_id)?;
        for chunk in chunks {
            tx.execute(
                "INSERT INTO chunks
                 (id,concept_id,source_document_id,collection_id,ordinal,heading_or_page,text,
                  text_sha256,embedding,source_revision,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    chunk.id.to_string(),
                    chunk.concept_id.to_string(),
                    chunk.source_document_id.to_string(),
                    chunk.collection_id.to_string(),
                    chunk.ordinal,
                    chunk.heading_or_page,
                    chunk.text,
                    chunk.text_sha256,
                    encode_embedding(&chunk.embedding),
                    chunk.source_revision,
                    now.to_rfc3339(),
                ],
            )?;
            tx.execute(
                "INSERT INTO chunk_fts(chunk_id,title,description,tags,heading,text)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    chunk.id.to_string(),
                    draft.title,
                    draft.description,
                    draft.tags.join(" "),
                    chunk.heading_or_page,
                    chunk.text,
                ],
            )?;
        }
        let concept_changed = tx.execute(
            "UPDATE concepts SET concept_type=?2,title=?3,description=?4,language=?5,tags_json=?6,
             entities_json=?7,links_json=?8,summary=?9,classification_confidence=?10,
             classification_explanation=?11,generator_model=?12,status='needs_review',
             reviewed_at=NULL,updated_at=?13 WHERE id=?1 AND status='enriched'",
            params![
                claim.concept_id.to_string(),
                draft.concept_type.to_string(),
                draft.title,
                draft.description,
                draft.language,
                serde_json::to_string(&draft.tags)?,
                serde_json::to_string(&draft.entities)?,
                serde_json::to_string(&draft.links)?,
                draft.summary,
                draft.classification_confidence,
                draft.classification_explanation,
                generator_model,
                now.to_rfc3339(),
            ],
        )?;
        let source_changed = tx.execute(
            "UPDATE source_documents SET status='needs_review',last_error=NULL,updated_at=?2
             WHERE id=?1 AND status='enriched' AND source_sha256=?3 AND revision=?4",
            params![
                claim.source_document_id.to_string(),
                now.to_rfc3339(),
                claim.source_sha256,
                claim.revision,
            ],
        )?;
        let job_changed = tx.execute(
            "UPDATE jobs SET state='completed',last_error=NULL,updated_at=?2
             WHERE id=?1 AND state='running'",
            params![claim.job_id.to_string(), now.to_rfc3339()],
        )?;
        if concept_changed != 1 || source_changed != 1 || job_changed != 1 {
            bail!("review state changed while completing reanalysis");
        }
        tx.commit()?;
        Ok(true)
    }

    /// Restores the unchanged prior draft after a failed reanalysis. If a
    /// watcher already superseded the claim, only the job is failed and the
    /// newer filesystem reconciliation remains authoritative.
    pub fn fail_review_reanalysis(
        &self,
        claim: &ReviewReanalysisClaim,
        error: impl AsRef<str>,
    ) -> Result<bool> {
        let error: String = error.as_ref().chars().take(2_000).collect();
        let now = Utc::now().to_rfc3339();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        tx.execute(
            "UPDATE jobs SET state='failed',last_error=?2,updated_at=?3
             WHERE id=?1 AND state='running'",
            params![claim.job_id.to_string(), error, now],
        )?;
        let source_changed = tx.execute(
            "UPDATE source_documents SET status='needs_review',last_error=?2,updated_at=?3
             WHERE id=?1 AND status='enriched' AND source_sha256=?4 AND revision=?5
               AND EXISTS (
                 SELECT 1 FROM concepts co WHERE co.id=?6 AND co.source_document_id=?1
                   AND co.status='enriched'
               )",
            params![
                claim.source_document_id.to_string(),
                error,
                now,
                claim.source_sha256,
                claim.revision,
                claim.concept_id.to_string(),
            ],
        )?;
        let concept_changed = if source_changed == 1 {
            tx.execute(
                "UPDATE concepts SET status='needs_review',reviewed_at=NULL,updated_at=?2
                 WHERE id=?1 AND status='enriched'",
                params![claim.concept_id.to_string(), now],
            )?
        } else {
            0
        };
        if source_changed != concept_changed {
            bail!("review state changed while restoring failed reanalysis");
        }
        tx.commit()?;
        Ok(source_changed == 1)
    }

    pub fn list_published_concepts(&self, collection_id: Uuid) -> Result<Vec<ConceptRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,source_document_id,collection_id,concept_type,title,description,language,
             tags_json,entities_json,links_json,summary,classification_confidence,
             classification_explanation,logical_resource_uri,generator_model,status,reviewed_at,
             created_at,updated_at FROM concepts WHERE collection_id=?1 AND status='published'
             ORDER BY title COLLATE NOCASE",
        )?;
        let rows = statement.query_map([collection_id.to_string()], concept_from_row)?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    pub fn public_concept_page(
        &self,
        publisher_id: &str,
        collection_id: Uuid,
        after_concept_id: Option<Uuid>,
        limit: u8,
    ) -> Result<Vec<PublicConceptSummary>> {
        if !(1..=airwiki_types::MAX_PUBLIC_PAGE_SIZE).contains(&limit) {
            bail!("public browse limit is invalid");
        }
        let connection = self.connection()?;
        Self::public_concept_page_on(
            &connection,
            publisher_id,
            collection_id,
            after_concept_id,
            limit,
        )
    }

    pub fn public_concept_page_under_disclosure(
        &self,
        lease: &DisclosureLease,
        publisher_id: &str,
        collection_id: Uuid,
        after_concept_id: Option<Uuid>,
        limit: u8,
    ) -> Result<Vec<PublicConceptSummary>> {
        let connection = self.connection_under_disclosure(lease)?;
        Self::public_concept_page_on(
            &connection,
            publisher_id,
            collection_id,
            after_concept_id,
            limit,
        )
    }

    fn public_concept_page_on(
        connection: &Connection,
        publisher_id: &str,
        collection_id: Uuid,
        after_concept_id: Option<Uuid>,
        limit: u8,
    ) -> Result<Vec<PublicConceptSummary>> {
        let is_public = connection
            .query_row(
                "SELECT internet_public FROM collections WHERE id=?1",
                [collection_id.to_string()],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false);
        if !is_public {
            bail!("collection is not publicly accessible");
        }
        let after = after_concept_id
            .map(|id| id.to_string())
            .unwrap_or_default();
        let mut statement = connection.prepare(
            "SELECT co.id,co.concept_type,co.title,co.description,co.language,co.tags_json,
                    co.summary,co.logical_resource_uri,sd.revision,co.updated_at
             FROM concepts co
             JOIN source_documents sd ON sd.id=co.source_document_id
             JOIN collections col ON col.id=co.collection_id
             WHERE co.collection_id=?1 AND co.status='published' AND sd.status='published'
               AND col.internet_public=1 AND co.id>?2
             ORDER BY co.id LIMIT ?3",
        )?;
        statement
            .query_map(
                params![collection_id.to_string(), after, i64::from(limit)],
                |row| {
                    Ok(PublicConceptSummary {
                        publisher_id: publisher_id.to_owned(),
                        collection_id,
                        concept_id: uuid_sql(row.get::<_, String>(0)?)?,
                        concept_type: concept_type_sql(row.get::<_, String>(1)?)?,
                        title: row.get(2)?,
                        description: row.get(3)?,
                        language: row.get(4)?,
                        tags: json_sql(row.get::<_, String>(5)?)?,
                        summary: row.get(6)?,
                        logical_resource_uri: row.get(7)?,
                        source_revision: row.get(8)?,
                        updated_at: datetime_sql(row.get::<_, String>(9)?)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()
            .map_err(Into::into)
    }

    pub fn public_manifest_sequence_under_disclosure(
        &self,
        lease: &DisclosureLease,
        collection_id: Uuid,
    ) -> Result<Option<u64>> {
        self.connection_under_disclosure(lease)?
            .query_row(
                "SELECT p.manifest_sequence FROM public_collection_profiles p
                 JOIN collections c ON c.id=p.collection_id
                 WHERE p.collection_id=?1 AND c.internet_public=1",
                [collection_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn public_collection_fingerprint(&self, collection_id: Uuid) -> Result<String> {
        let connection = self.connection()?;
        Self::public_collection_fingerprint_on(&connection, collection_id)
    }

    pub fn public_collection_fingerprint_under_disclosure(
        &self,
        lease: &DisclosureLease,
        collection_id: Uuid,
    ) -> Result<String> {
        let connection = self.connection_under_disclosure(lease)?;
        Self::public_collection_fingerprint_on(&connection, collection_id)
    }

    fn public_collection_fingerprint_on(
        connection: &Connection,
        collection_id: Uuid,
    ) -> Result<String> {
        let mut statement = connection.prepare(
            "SELECT co.id,sd.source_sha256,sd.revision,co.updated_at
             FROM concepts co
             JOIN source_documents sd ON sd.id=co.source_document_id
             JOIN collections col ON col.id=co.collection_id
             WHERE co.collection_id=?1 AND co.status='published' AND sd.status='published'
               AND col.internet_public=1 ORDER BY co.id",
        )?;
        let rows = statement.query_map([collection_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut hasher = Sha256::new();
        let mut count = 0_u64;
        for row in rows {
            let (concept_id, source_sha256, revision, updated_at) = row?;
            hasher.update(concept_id.as_bytes());
            hasher.update(source_sha256.as_bytes());
            hasher.update(revision.to_be_bytes());
            hasher.update(updated_at.as_bytes());
            count = count.saturating_add(1);
        }
        if count == 0 {
            bail!("public collection has no published concepts");
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub fn public_manifest_material(&self, collection_id: Uuid) -> Result<PublicManifestMaterial> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT co.title,co.description,co.language,co.tags_json,co.updated_at
             FROM concepts co
             JOIN source_documents sd ON sd.id=co.source_document_id
             JOIN collections col ON col.id=co.collection_id
             WHERE co.collection_id=?1 AND co.status='published' AND sd.status='published'
               AND col.internet_public=1 ORDER BY co.id",
        )?;
        let rows = statement.query_map([collection_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                datetime_sql(row.get::<_, String>(4)?)?,
            ))
        })?;
        let mut terms = std::collections::BTreeSet::new();
        let mut count = 0_u32;
        let mut updated_at = None;
        for row in rows {
            let (title, description, language, tags_json, concept_updated_at) = row?;
            count = count.saturating_add(1);
            updated_at = Some(
                updated_at.map_or(concept_updated_at, |known: DateTime<Utc>| {
                    known.max(concept_updated_at)
                }),
            );
            for value in [title, description, language, tags_json] {
                for term in value
                    .split(|character: char| !character.is_alphanumeric())
                    .filter(|term| term.chars().count() >= 2)
                {
                    if terms.len() >= airwiki_types::MAX_PUBLIC_ROUTING_TERMS {
                        break;
                    }
                    let normalized = term.to_lowercase();
                    if normalized.len() <= 64 {
                        terms.insert(normalized);
                    }
                }
            }
        }
        let updated_at = updated_at.unwrap_or_else(Utc::now);
        Ok(PublicManifestMaterial {
            concept_count: count,
            routing_terms: terms.into_iter().collect(),
            updated_at,
        })
    }

    pub fn return_to_review_if_current(
        &self,
        concept_id: Uuid,
        source_sha256: &str,
        revision: u32,
        reason: &str,
    ) -> Result<bool> {
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let source_id = tx
            .query_row(
                "SELECT co.source_document_id FROM concepts co
                 JOIN source_documents sd ON sd.id=co.source_document_id
                 WHERE co.id=?1 AND co.status='published' AND sd.status='published'
                   AND sd.source_sha256=?2 AND sd.revision=?3",
                params![concept_id.to_string(), source_sha256, revision],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(source_id) = source_id else {
            return Ok(false);
        };
        // Publication queries already require both source and concept status to
        // be `published`. Keep the computed chunks so a transient OKF filesystem
        // failure can be corrected and approved again without rerunning models.
        let concept_changed = tx.execute(
            "UPDATE concepts SET status='needs_review',reviewed_at=NULL,updated_at=?2
             WHERE id=?1 AND status='published'",
            params![concept_id.to_string(), Utc::now().to_rfc3339()],
        )?;
        let source_changed = tx.execute(
            "UPDATE source_documents SET status='needs_review',last_error=?2,updated_at=?3
             WHERE id=?1 AND status='published' AND source_sha256=?4 AND revision=?5",
            params![
                source_id,
                reason.chars().take(2_000).collect::<String>(),
                Utc::now().to_rfc3339(),
                source_sha256,
                revision,
            ],
        )?;
        if concept_changed != 1 || source_changed != 1 {
            bail!("publication state changed while returning concept to review");
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn replace_chunks(&self, concept_id: Uuid, chunks: &[StoredChunk]) -> Result<()> {
        self.replace_chunks_inner(concept_id, chunks, None)?;
        Ok(())
    }

    pub fn replace_chunks_if_current(
        &self,
        concept_id: Uuid,
        source_sha256: &str,
        revision: u32,
        chunks: &[StoredChunk],
    ) -> Result<bool> {
        self.replace_chunks_inner(concept_id, chunks, Some((source_sha256, revision)))
    }

    fn replace_chunks_inner(
        &self,
        concept_id: Uuid,
        chunks: &[StoredChunk],
        expected_source: Option<(&str, u32)>,
    ) -> Result<bool> {
        if chunks
            .iter()
            .any(|chunk| chunk.embedding.len() != EMBEDDING_DIMENSIONS)
        {
            bail!("every embedding must have {EMBEDDING_DIMENSIONS} dimensions");
        }
        let concept = self
            .concept(concept_id)?
            .ok_or_else(|| anyhow!("concept {concept_id} does not exist"))?;
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        if let Some((source_sha256, revision)) = expected_source {
            let current = tx.query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM source_documents
                   WHERE id=?1 AND source_sha256=?2 AND revision=?3 AND status='needs_review'
                 )",
                params![
                    concept.source_document_id.to_string(),
                    source_sha256,
                    revision
                ],
                |row| row.get::<_, bool>(0),
            )?;
            if !current {
                return Ok(false);
            }
        }
        delete_chunks_for_concept(&tx, concept_id)?;
        for chunk in chunks {
            if chunk.concept_id != concept_id
                || chunk.source_document_id != concept.source_document_id
                || chunk.collection_id != concept.collection_id
            {
                bail!("chunk ownership does not match concept {concept_id}");
            }
            tx.execute(
                "INSERT INTO chunks
                 (id,concept_id,source_document_id,collection_id,ordinal,heading_or_page,text,
                  text_sha256,embedding,source_revision,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    chunk.id.to_string(),
                    chunk.concept_id.to_string(),
                    chunk.source_document_id.to_string(),
                    chunk.collection_id.to_string(),
                    chunk.ordinal,
                    chunk.heading_or_page,
                    chunk.text,
                    chunk.text_sha256,
                    encode_embedding(&chunk.embedding),
                    chunk.source_revision,
                    Utc::now().to_rfc3339(),
                ],
            )?;
            tx.execute(
                "INSERT INTO chunk_fts(chunk_id,title,description,tags,heading,text)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    chunk.id.to_string(),
                    concept.draft.title,
                    concept.draft.description,
                    concept.draft.tags.join(" "),
                    chunk.heading_or_page,
                    chunk.text,
                ],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn chunks_for_concept(&self, concept_id: Uuid) -> Result<Vec<StoredChunk>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,concept_id,source_document_id,collection_id,ordinal,heading_or_page,text,
             text_sha256,embedding,source_revision FROM chunks WHERE concept_id=?1 ORDER BY ordinal",
        )?;
        let rows = statement.query_map([concept_id.to_string()], chunk_from_row)?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    /// Loads source evidence for the exact concept revision awaiting review.
    ///
    /// Returns `None` only when the requested review is stale. A current review
    /// with no chunks returns an empty page so callers can distinguish missing
    /// evidence from a concurrent state transition. Embeddings are deliberately
    /// excluded from the query.
    pub fn review_evidence_page(
        &self,
        concept_id: Uuid,
        expected_revision: u32,
        expected_review_version: Option<&ReviewVersionToken>,
        after_ordinal: Option<u32>,
        limit: usize,
    ) -> Result<Option<ReviewEvidencePageRecord>> {
        if !(1..=100).contains(&limit) {
            bail!("review evidence page limit must be between 1 and 100");
        }

        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let Some(snapshot) = pending_review_snapshot(&tx, concept_id, expected_revision)? else {
            tx.commit()?;
            return Ok(None);
        };
        if expected_review_version.is_some_and(|expected| expected != &snapshot.review_version) {
            tx.commit()?;
            return Ok(None);
        }
        let query_limit = i64::try_from(limit + 1)
            .context("review evidence page limit exceeds SQLite capacity")?;
        let mut statement = tx.prepare(
            "SELECT ordinal,heading_or_page,text
             FROM chunks
             WHERE concept_id=?1 AND source_document_id=?2 AND collection_id=?3
               AND source_revision=?4 AND ordinal>coalesce(?5,-1)
             ORDER BY ordinal
             LIMIT ?6",
        )?;
        let mut rows = statement.query(params![
            concept_id.to_string(),
            snapshot.source_document_id,
            snapshot.collection_id,
            snapshot.source_revision,
            after_ordinal,
            query_limit,
        ])?;
        let mut chunks = Vec::with_capacity(limit + 1);
        while let Some(row) = rows.next()? {
            chunks.push(ReviewEvidenceChunkRecord {
                ordinal: row.get(0)?,
                heading_or_page: row.get(1)?,
                text: row.get(2)?,
            });
        }
        drop(rows);
        drop(statement);

        let has_more = chunks.len() > limit;
        chunks.truncate(limit);
        let next_ordinal = if has_more {
            chunks.last().map(|chunk| chunk.ordinal)
        } else {
            None
        };
        tx.commit()?;

        Ok(Some(ReviewEvidencePageRecord {
            concept_id,
            source_revision: snapshot.source_revision,
            review_version: snapshot.review_version,
            total_chunks: snapshot.total_chunks,
            chunks,
            next_ordinal,
        }))
    }

    pub(crate) fn begin_publication_if_current(
        &self,
        concept_id: Uuid,
        mut draft: EnrichmentDraft,
        expected: ExpectedReview<'_>,
        action: &str,
        reviewed_at: DateTime<Utc>,
    ) -> Result<PublicationClaim> {
        draft.sanitize();
        validate_draft(&draft)?;
        if !matches!(action, "published" | "replaced") {
            bail!("unsupported OKF publication action {action}");
        }
        let now = Utc::now();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let snapshot = pending_review_snapshot(&tx, concept_id, expected.source_revision)?
            .filter(|snapshot| {
                snapshot.source_sha256 == expected.source_sha256
                    && snapshot.review_version == *expected.review_version
                    && snapshot.total_chunks > 0
            })
            .with_context(|| {
                format!("concept {concept_id} is no longer an approvable current review")
            })?;
        let source_id = parse_uuid(&snapshot.source_document_id)?;
        let collection_id = parse_uuid(&snapshot.collection_id)?;
        let job_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO jobs(id,source_document_id,kind,state,attempts,created_at,updated_at)
             VALUES (?1,?2,'publish','running',1,?3,?3)",
            params![job_id.to_string(), source_id.to_string(), now.to_rfc3339()],
        )?;
        tx.execute(
            "INSERT INTO publication_claims
             (job_id,concept_id,collection_id,source_sha256,source_revision,action,reviewed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                job_id.to_string(),
                concept_id.to_string(),
                collection_id.to_string(),
                expected.source_sha256,
                expected.source_revision,
                action,
                reviewed_at.to_rfc3339(),
            ],
        )?;
        let concept_changed = tx.execute(
            "UPDATE concepts SET concept_type=?2,title=?3,description=?4,language=?5,tags_json=?6,
             entities_json=?7,links_json=?8,summary=?9,classification_confidence=?10,
             classification_explanation=?11,status='publishing',reviewed_at=?12,updated_at=?13
             WHERE id=?1 AND status='needs_review'",
            params![
                concept_id.to_string(),
                draft.concept_type.to_string(),
                draft.title,
                draft.description,
                draft.language,
                serde_json::to_string(&draft.tags)?,
                serde_json::to_string(&draft.entities)?,
                serde_json::to_string(&draft.links)?,
                draft.summary,
                draft.classification_confidence,
                draft.classification_explanation,
                reviewed_at.to_rfc3339(),
                now.to_rfc3339(),
            ],
        )?;
        let source_changed = tx.execute(
            "UPDATE source_documents SET status='publishing',last_error=NULL,updated_at=?2
             WHERE id=?1 AND status='needs_review' AND source_sha256=?3 AND revision=?4",
            params![
                source_id.to_string(),
                now.to_rfc3339(),
                expected.source_sha256,
                expected.source_revision,
            ],
        )?;
        if concept_changed != 1 || source_changed != 1 {
            bail!("concept {concept_id} changed while publication was being claimed");
        }
        tx.execute(
            "UPDATE chunk_fts SET title=?2,description=?3,tags=?4
             WHERE chunk_id IN (SELECT id FROM chunks WHERE concept_id=?1)",
            params![
                concept_id.to_string(),
                draft.title,
                draft.description,
                draft.tags.join(" ")
            ],
        )?;
        tx.commit()?;
        drop(connection);
        self.publication_claim(job_id)?
            .context("durable publication claim disappeared")
    }

    pub(crate) fn publication_claim(&self, job_id: Uuid) -> Result<Option<PublicationClaim>> {
        self.connection()?
            .query_row(
                "SELECT pc.job_id,pc.concept_id,j.source_document_id,pc.collection_id,
                 sd.source_path,pc.source_sha256,pc.source_revision,pc.action,pc.reviewed_at,j.state
                 FROM publication_claims pc JOIN jobs j ON j.id=pc.job_id
                 JOIN source_documents sd ON sd.id=j.source_document_id
                 WHERE pc.job_id=?1 AND j.kind='publish'",
                [job_id.to_string()],
                publication_claim_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub(crate) fn publication_claims(&self) -> Result<Vec<PublicationClaim>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT pc.job_id,pc.concept_id,j.source_document_id,pc.collection_id,
             sd.source_path,pc.source_sha256,pc.source_revision,pc.action,pc.reviewed_at,j.state
             FROM publication_claims pc JOIN jobs j ON j.id=pc.job_id
             JOIN source_documents sd ON sd.id=j.source_document_id
             WHERE j.kind='publish' ORDER BY j.created_at,pc.job_id",
        )?;
        let rows = statement.query_map([], publication_claim_from_row)?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    pub(crate) fn collection_has_publication_claim(&self, collection_id: Uuid) -> Result<bool> {
        self.connection()?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM publication_claims WHERE collection_id=?1)",
                [collection_id.to_string()],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub(crate) fn publication_snapshot(
        &self,
        claim: &PublicationClaim,
    ) -> Result<Vec<ConceptRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id,source_document_id,collection_id,concept_type,title,description,language,
             tags_json,entities_json,links_json,summary,classification_confidence,
             classification_explanation,logical_resource_uri,generator_model,status,reviewed_at,
             created_at,updated_at FROM concepts WHERE collection_id=?1
             AND (status='published' OR (id=?2 AND status='publishing'))
             ORDER BY title COLLATE NOCASE",
        )?;
        let rows = statement.query_map(
            params![
                claim.collection_id.to_string(),
                claim.concept_id.to_string()
            ],
            concept_from_row,
        )?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    pub(crate) fn publication_claim_is_current(&self, claim: &PublicationClaim) -> Result<bool> {
        self.connection()?
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM publication_claims pc
                   JOIN jobs j ON j.id=pc.job_id
                   JOIN concepts co ON co.id=pc.concept_id
                   JOIN source_documents sd ON sd.id=j.source_document_id
                   WHERE pc.job_id=?1 AND pc.concept_id=?2 AND pc.collection_id=?3
                     AND pc.source_sha256=?4 AND pc.source_revision=?5
                     AND j.kind='publish' AND j.state='running'
                     AND co.status='publishing' AND sd.status='publishing'
                     AND co.source_document_id=sd.id AND sd.source_sha256=?4 AND sd.revision=?5
                     AND EXISTS (
                       SELECT 1 FROM chunks ch WHERE ch.concept_id=co.id
                         AND ch.source_document_id=sd.id AND ch.source_revision=sd.revision
                     )
                 )",
                params![
                    claim.job_id.to_string(),
                    claim.concept_id.to_string(),
                    claim.collection_id.to_string(),
                    claim.source_sha256,
                    claim.source_revision,
                ],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub(crate) fn complete_publication_if_current(&self, claim: &PublicationClaim) -> Result<bool> {
        let now = Utc::now();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let current: bool = tx.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM publication_claims pc
               JOIN jobs j ON j.id=pc.job_id
               JOIN concepts co ON co.id=pc.concept_id
               JOIN source_documents sd ON sd.id=j.source_document_id
               WHERE pc.job_id=?1 AND pc.concept_id=?2 AND pc.collection_id=?3
                 AND pc.source_sha256=?4 AND pc.source_revision=?5
                 AND j.kind='publish' AND j.state='running'
                 AND co.status='publishing' AND sd.status='publishing'
                 AND co.source_document_id=sd.id AND sd.source_sha256=?4 AND sd.revision=?5
             )",
            params![
                claim.job_id.to_string(),
                claim.concept_id.to_string(),
                claim.collection_id.to_string(),
                claim.source_sha256,
                claim.source_revision,
            ],
            |row| row.get(0),
        )?;
        if !current {
            return Ok(false);
        }
        let concept_changed = tx.execute(
            "UPDATE concepts SET status='published',updated_at=?2
             WHERE id=?1 AND status='publishing'",
            params![claim.concept_id.to_string(), now.to_rfc3339()],
        )?;
        let source_changed = tx.execute(
            "UPDATE source_documents SET status='published',last_error=NULL,updated_at=?2
             WHERE id=?1 AND status='publishing' AND source_sha256=?3 AND revision=?4",
            params![
                claim.source_document_id.to_string(),
                now.to_rfc3339(),
                claim.source_sha256,
                claim.source_revision,
            ],
        )?;
        let job_changed = tx.execute(
            "UPDATE jobs SET state='completed',last_error=NULL,updated_at=?2
             WHERE id=?1 AND kind='publish' AND state='running'",
            params![claim.job_id.to_string(), now.to_rfc3339()],
        )?;
        if concept_changed != 1 || source_changed != 1 || job_changed != 1 {
            bail!("publication state changed while its final commit was being written");
        }
        let details = serde_json::json!({
            "collection_id": claim.collection_id,
            "source_revision": claim.source_revision,
            "source_sha256": claim.source_sha256,
        });
        tx.execute(
            "INSERT INTO audit_events
             (id,actor,action,target_type,target_id,details_json,created_at)
             VALUES (?1,'human',?2,'concept',?3,?4,?5)",
            params![
                claim.job_id.to_string(),
                claim.action,
                claim.concept_id.to_string(),
                serde_json::to_string(&details)?,
                claim.reviewed_at.to_rfc3339(),
            ],
        )?;
        tx.execute(
            "DELETE FROM publication_claims WHERE job_id=?1",
            [claim.job_id.to_string()],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub(crate) fn mark_publication_cancelling(
        &self,
        claim: &PublicationClaim,
        error: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let error = error.chars().take(2_000).collect::<String>();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let changed = tx.execute(
            "UPDATE jobs SET state='cancelling',last_error=?2,updated_at=?3
             WHERE id=?1 AND kind='publish' AND state IN ('running','cancelling')",
            params![claim.job_id.to_string(), error, now,],
        )?;
        ensure_changed(changed, "publication job", claim.job_id)?;
        tx.execute(
            "UPDATE concepts SET status='needs_review',reviewed_at=NULL,updated_at=?2
             WHERE id=?1 AND status='publishing'",
            params![claim.concept_id.to_string(), now],
        )?;
        tx.execute(
            "UPDATE source_documents SET status='needs_review',last_error=?2,updated_at=?3
             WHERE id=?1 AND status='publishing' AND source_sha256=?4 AND revision=?5",
            params![
                claim.source_document_id.to_string(),
                error,
                now,
                claim.source_sha256,
                claim.source_revision,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn finish_publication_cancellation(
        &self,
        claim: &PublicationClaim,
        error: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let error = error.chars().take(2_000).collect::<String>();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM publication_claims WHERE job_id=?1)",
            [claim.job_id.to_string()],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(());
        }
        tx.execute(
            "UPDATE concepts SET status='needs_review',reviewed_at=NULL,updated_at=?2
             WHERE id=?1 AND status='publishing'",
            params![claim.concept_id.to_string(), now],
        )?;
        tx.execute(
            "UPDATE source_documents SET status='needs_review',last_error=?2,updated_at=?3
             WHERE id=?1 AND status='publishing' AND source_sha256=?4 AND revision=?5",
            params![
                claim.source_document_id.to_string(),
                error,
                now,
                claim.source_sha256,
                claim.source_revision,
            ],
        )?;
        tx.execute(
            "UPDATE jobs SET state='failed',last_error=?2,updated_at=?3 WHERE id=?1",
            params![claim.job_id.to_string(), error, now],
        )?;
        tx.execute(
            "DELETE FROM publication_claims WHERE job_id=?1",
            [claim.job_id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn note_publication_retry(&self, claim: &PublicationClaim) -> Result<()> {
        let changed = self.connection()?.execute(
            "UPDATE jobs SET attempts=attempts+1,last_error=?2,updated_at=?3
             WHERE id=?1 AND kind='publish' AND state IN ('running','cancelling')",
            params![
                claim.job_id.to_string(),
                Option::<String>::None,
                Utc::now().to_rfc3339(),
            ],
        )?;
        ensure_changed(changed, "publication job", claim.job_id)
    }

    pub(crate) fn record_publication_error(
        &self,
        claim: &PublicationClaim,
        error: &str,
    ) -> Result<()> {
        let changed = self.connection()?.execute(
            "UPDATE jobs SET last_error=?2,updated_at=?3
             WHERE id=?1 AND kind='publish' AND state IN ('running','cancelling')",
            params![
                claim.job_id.to_string(),
                error.chars().take(2_000).collect::<String>(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        ensure_changed(changed, "publication job", claim.job_id)
    }

    #[cfg(test)]
    pub fn approve_concept(
        &self,
        concept_id: Uuid,
        draft: EnrichmentDraft,
    ) -> Result<ConceptRecord> {
        let concept = self
            .concept(concept_id)?
            .ok_or_else(|| anyhow!("concept {concept_id} does not exist"))?;
        let source = self
            .source_document(concept.source_document_id)?
            .context("concept source document does not exist")?;
        self.approve_concept_if_current(concept_id, draft, &source.source_sha256, source.revision)
    }

    #[cfg(test)]
    pub fn approve_concept_if_current(
        &self,
        concept_id: Uuid,
        draft: EnrichmentDraft,
        expected_source_sha256: &str,
        expected_revision: u32,
    ) -> Result<ConceptRecord> {
        let mut draft = draft;
        draft.sanitize();
        validate_draft(&draft)?;
        let now = Utc::now();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let source_id = tx
            .query_row(
                "SELECT co.source_document_id FROM concepts co
                 JOIN source_documents sd ON sd.id=co.source_document_id
                 WHERE co.id=?1 AND co.status='needs_review' AND sd.status='needs_review'
                   AND sd.source_sha256=?2 AND sd.revision=?3
                   AND EXISTS (
                     SELECT 1 FROM chunks ch
                     WHERE ch.concept_id=co.id AND ch.source_document_id=sd.id
                       AND ch.source_revision=sd.revision
                   )",
                params![
                    concept_id.to_string(),
                    expected_source_sha256,
                    expected_revision
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(source_id) = source_id else {
            bail!("concept {concept_id} is no longer an approvable current revision");
        };
        let concept_changed = tx.execute(
            "UPDATE concepts SET concept_type=?2,title=?3,description=?4,language=?5,tags_json=?6,
             entities_json=?7,links_json=?8,summary=?9,classification_confidence=?10,
             classification_explanation=?11,status='published',reviewed_at=?12,updated_at=?12
             WHERE id=?1 AND status='needs_review'
               AND EXISTS (
                 SELECT 1 FROM source_documents sd
                 WHERE sd.id=concepts.source_document_id AND sd.status='needs_review'
                   AND sd.source_sha256=?13 AND sd.revision=?14
               )
               AND EXISTS (SELECT 1 FROM chunks ch WHERE ch.concept_id=concepts.id)",
            params![
                concept_id.to_string(),
                draft.concept_type.to_string(),
                draft.title,
                draft.description,
                draft.language,
                serde_json::to_string(&draft.tags)?,
                serde_json::to_string(&draft.entities)?,
                serde_json::to_string(&draft.links)?,
                draft.summary,
                draft.classification_confidence,
                draft.classification_explanation,
                now.to_rfc3339(),
                expected_source_sha256,
                expected_revision,
            ],
        )?;
        let source_changed = tx.execute(
            "UPDATE source_documents SET status='published',updated_at=?2
             WHERE id=?1 AND status='needs_review' AND source_sha256=?3 AND revision=?4",
            params![
                source_id,
                now.to_rfc3339(),
                expected_source_sha256,
                expected_revision
            ],
        )?;
        if concept_changed != 1 || source_changed != 1 {
            bail!("concept {concept_id} changed while approval was being committed");
        }
        // Refresh denormalized FTS metadata after human edits.
        tx.execute(
            "UPDATE chunk_fts SET title=?2,description=?3,tags=?4
             WHERE chunk_id IN (SELECT id FROM chunks WHERE concept_id=?1)",
            params![
                concept_id.to_string(),
                draft.title,
                draft.description,
                draft.tags.join(" ")
            ],
        )?;
        tx.commit()?;
        drop(connection);
        let published = self
            .concept(concept_id)?
            .ok_or_else(|| anyhow!("approved concept disappeared"))?;
        if published.status != DocumentStatus::Published {
            bail!("approved concept was superseded before publication could continue");
        }
        Ok(published)
    }

    pub fn publication_is_current(
        &self,
        concept_id: Uuid,
        source_sha256: &str,
        revision: u32,
    ) -> Result<bool> {
        self.connection()?
            .query_row(
                "SELECT EXISTS(
                   SELECT 1 FROM concepts co
                   JOIN source_documents sd ON sd.id=co.source_document_id
                   WHERE co.id=?1 AND co.status='published' AND sd.status='published'
                     AND sd.source_sha256=?2 AND sd.revision=?3
                     AND EXISTS (
                       SELECT 1 FROM chunks ch
                       WHERE ch.concept_id=co.id AND ch.source_document_id=sd.id
                         AND ch.source_revision=sd.revision
                     )
                 )",
                params![concept_id.to_string(), source_sha256, revision],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn mark_deleted(&self, source_document_id: Uuid) -> Result<()> {
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        withdraw_source(&tx, source_document_id)?;
        let now = Utc::now().to_rfc3339();
        let count = tx.execute(
            "UPDATE source_documents SET status='deleted',deleted_at=?2,updated_at=?2 WHERE id=?1",
            params![source_document_id.to_string(), now],
        )?;
        ensure_changed(count, "source document", source_document_id)?;
        tx.execute(
            "UPDATE concepts SET status='deleted',updated_at=?2 WHERE source_document_id=?1",
            params![source_document_id.to_string(), now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Makes one source non-searchable without claiming that it was deleted.
    /// This is used when discovery can still see the path but cannot safely
    /// process it, for example after a size-limit or metadata failure.
    pub fn quarantine_source(
        &self,
        source_document_id: Uuid,
        reason: impl AsRef<str>,
    ) -> Result<Option<(Uuid, String)>> {
        let reason = reason.as_ref().chars().take(2_000).collect::<String>();
        let now = Utc::now().to_rfc3339();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let (concept_id, source_sha256, status) = tx.query_row(
            "SELECT concept_id,source_sha256,status FROM source_documents WHERE id=?1",
            [source_document_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        if status == "deleted" {
            return Ok(None);
        }
        withdraw_source(&tx, source_document_id)?;
        tx.execute(
            "UPDATE source_documents SET status='failed',last_error=?2,updated_at=?3
             WHERE id=?1",
            params![source_document_id.to_string(), reason, now],
        )?;
        tx.execute(
            "UPDATE concepts SET status='failed',reviewed_at=NULL,updated_at=?2
             WHERE source_document_id=?1",
            params![source_document_id.to_string(), now],
        )?;
        tx.commit()?;

        if status == "published"
            && let Some(concept_id) = concept_id
        {
            return Ok(Some((parse_uuid(&concept_id)?, source_sha256)));
        }
        Ok(None)
    }

    /// Atomically makes every non-deleted source in a collection
    /// non-searchable when its filesystem can no longer be monitored. Chunks
    /// and FTS rows are removed in the same transaction; a later successful
    /// scan can reclaim unchanged hashes but must return them to human review.
    /// The returned pairs identify OKF files that were published before the
    /// quarantine and must be removed from the bundle.
    pub fn quarantine_collection(
        &self,
        collection_id: Uuid,
        reason: impl AsRef<str>,
    ) -> Result<Vec<(Uuid, String)>> {
        let reason = reason.as_ref().chars().take(2_000).collect::<String>();
        let now = Utc::now().to_rfc3339();
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let mut statement = tx.prepare(
            "SELECT id,concept_id,source_sha256,status FROM source_documents
             WHERE collection_id=?1 AND status!='deleted'",
        )?;
        let rows = statement
            .query_map([collection_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);

        let mut published_artifacts = Vec::new();
        for (source_id, concept_id, source_sha256, status) in rows {
            let source_id = parse_uuid(&source_id)?;
            withdraw_source(&tx, source_id)?;
            tx.execute(
                "UPDATE source_documents SET status='failed',last_error=?2,updated_at=?3
                 WHERE id=?1",
                params![source_id.to_string(), reason, now],
            )?;
            tx.execute(
                "UPDATE concepts SET status='failed',reviewed_at=NULL,updated_at=?2
                 WHERE source_document_id=?1",
                params![source_id.to_string(), now],
            )?;
            if status == "published"
                && let Some(concept_id) = concept_id
            {
                published_artifacts.push((parse_uuid(&concept_id)?, source_sha256));
            }
        }
        tx.commit()?;
        Ok(published_artifacts)
    }

    pub fn create_job(&self, source_document_id: Option<Uuid>, kind: &str) -> Result<JobRecord> {
        let now = Utc::now();
        let job = JobRecord {
            id: Uuid::new_v4(),
            source_document_id,
            kind: kind.to_owned(),
            state: "queued".into(),
            attempts: 0,
            last_error: None,
            created_at: now,
            updated_at: now,
        };
        self.connection()?.execute(
            "INSERT INTO jobs(id,source_document_id,kind,state,attempts,created_at,updated_at)
             VALUES (?1,?2,?3,?4,0,?5,?5)",
            params![
                job.id.to_string(),
                job.source_document_id.map(|id| id.to_string()),
                job.kind,
                job.state,
                now.to_rfc3339()
            ],
        )?;
        Ok(job)
    }

    pub fn set_job_state(&self, id: Uuid, state: &str, error: Option<&str>) -> Result<()> {
        let attempts_increment = u8::from(state == "running");
        let count = self.connection()?.execute(
            "UPDATE jobs SET state=?2,last_error=?3,attempts=attempts+?4,updated_at=?5 WHERE id=?1",
            params![
                id.to_string(),
                state,
                error,
                attempts_increment,
                Utc::now().to_rfc3339()
            ],
        )?;
        ensure_changed(count, "job", id)
    }

    pub fn upsert_peer(&self, peer: &PeerRecord) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.connection()?.execute(
            "INSERT INTO peers(peer_id,display_name,trusted,blocked,paired_at,last_seen_at,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?7)
             ON CONFLICT(peer_id) DO UPDATE SET display_name=excluded.display_name,
             trusted=excluded.trusted,blocked=excluded.blocked,paired_at=excluded.paired_at,
             last_seen_at=excluded.last_seen_at,updated_at=excluded.updated_at",
            params![
                peer.peer_id,
                peer.display_name,
                peer.trusted,
                peer.blocked,
                peer.paired_at.map(|v| v.to_rfc3339()),
                peer.last_seen_at.map(|v| v.to_rfc3339()),
                now,
            ],
        )?;
        Ok(())
    }

    pub fn peer(&self, peer_id: &str) -> Result<Option<PeerRecord>> {
        self.connection()?
            .query_row(
                "SELECT peer_id,display_name,trusted,blocked,paired_at,last_seen_at FROM peers WHERE peer_id=?1",
                [peer_id],
                |row| {
                    Ok(PeerRecord {
                        peer_id: row.get(0)?,
                        display_name: row.get(1)?,
                        trusted: row.get(2)?,
                        blocked: row.get(3)?,
                        paired_at: optional_datetime(row.get::<_, Option<String>>(4)?)?,
                        last_seen_at: optional_datetime(row.get::<_, Option<String>>(5)?)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_peers(&self) -> Result<Vec<PeerRecord>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT peer_id,display_name,trusted,blocked,paired_at,last_seen_at
             FROM peers ORDER BY COALESCE(display_name, peer_id) COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(PeerRecord {
                peer_id: row.get(0)?,
                display_name: row.get(1)?,
                trusted: row.get(2)?,
                blocked: row.get(3)?,
                paired_at: optional_datetime(row.get::<_, Option<String>>(4)?)?,
                last_seen_at: optional_datetime(row.get::<_, Option<String>>(5)?)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    pub fn set_grant(&self, peer_id: &str, collection_id: Uuid, granted: bool) -> Result<()> {
        let connection = self.connection()?;
        if granted {
            connection.execute(
                "INSERT INTO grants(peer_id,collection_id,granted_at) VALUES (?1,?2,?3)
                 ON CONFLICT(peer_id,collection_id) DO NOTHING",
                params![peer_id, collection_id.to_string(), Utc::now().to_rfc3339()],
            )?;
        } else {
            connection.execute(
                "DELETE FROM grants WHERE peer_id=?1 AND collection_id=?2",
                params![peer_id, collection_id.to_string()],
            )?;
        }
        Ok(())
    }

    pub fn list_grants(&self, peer_id: Option<&str>) -> Result<Vec<GrantRecord>> {
        let connection = self.connection()?;
        let mut grants = Vec::new();
        if let Some(peer_id) = peer_id {
            let mut statement = connection.prepare(
                "SELECT peer_id,collection_id,granted_at FROM grants
                 WHERE peer_id=?1 ORDER BY collection_id",
            )?;
            let rows = statement.query_map([peer_id], grant_from_row)?;
            for row in rows {
                grants.push(row?);
            }
        } else {
            let mut statement = connection.prepare(
                "SELECT peer_id,collection_id,granted_at FROM grants
                 ORDER BY peer_id,collection_id",
            )?;
            let rows = statement.query_map([], grant_from_row)?;
            for row in rows {
                grants.push(row?);
            }
        }
        Ok(grants)
    }

    pub fn revoke_peer(&self, peer_id: &str) -> Result<()> {
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        tx.execute("DELETE FROM grants WHERE peer_id=?1", [peer_id])?;
        tx.execute(
            "UPDATE peers SET trusted=0,blocked=1,updated_at=?2 WHERE peer_id=?1",
            params![peer_id, Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn granted_collections(&self, peer_id: &str) -> Result<Vec<Uuid>> {
        self.granted_collections_for_search(peer_id, SearchPurpose::LocalAssistant)
    }

    pub fn publicly_searchable_collections(&self, requested: &[Uuid]) -> Result<Vec<Uuid>> {
        if requested.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection()?;
        let placeholders = repeat_placeholders(requested.len(), 1);
        let sql = format!(
            "SELECT id FROM collections WHERE internet_public=1 AND id IN ({placeholders})"
        );
        let values = requested.iter().map(Uuid::to_string).collect::<Vec<_>>();
        let mut statement = connection.prepare(&sql)?;
        statement
            .query_map(params_from_iter(values), |row| row.get::<_, String>(0))?
            .map(|row| parse_uuid(&row?))
            .collect()
    }

    /// Returns the durable peer-grant and collection-policy intersection.
    /// External-AI searches require both independent collection opt-ins.
    pub fn granted_collections_for_search(
        &self,
        peer_id: &str,
        purpose: SearchPurpose,
    ) -> Result<Vec<Uuid>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT g.collection_id FROM grants g JOIN peers p ON p.peer_id=g.peer_id
             JOIN collections c ON c.id=g.collection_id
             WHERE g.peer_id=?1 AND p.trusted=1 AND p.blocked=0 AND c.peer_shareable=1
               AND (?2=0 OR c.allow_external_ai=1)",
        )?;
        let external_ai = purpose == SearchPurpose::ExternalAi;
        let rows =
            statement.query_map(params![peer_id, external_ai], |row| row.get::<_, String>(0))?;
        rows.map(|row| parse_uuid(&row?)).collect()
    }

    pub fn record_audit(&self, event: &AuditEvent) -> Result<()> {
        self.connection()?.execute(
            "INSERT INTO audit_events(id,actor,action,target_type,target_id,details_json,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                event.id.to_string(),
                event.actor,
                event.action,
                event.target_type,
                event.target_id,
                serde_json::to_string(&event.details)?,
                event.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn count(&self, table: &str) -> Result<u64> {
        if !matches!(
            table,
            "collections"
                | "source_documents"
                | "concepts"
                | "chunks"
                | "jobs"
                | "publication_claims"
                | "peers"
                | "grants"
                | "audit_events"
                | "collection_maintenance"
                | "public_collection_profiles"
                | "federation_indexes"
                | "public_publisher_blocks"
        ) {
            bail!("unsupported table name");
        }
        Ok(self
            .connection()?
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })?)
    }

    pub fn collection_stats(&self, collection_id: Uuid) -> Result<CollectionStats> {
        self.connection()?
            .query_row(
                "SELECT sum(CASE WHEN status!='deleted' THEN 1 ELSE 0 END),
                 sum(CASE WHEN status='needs_review' THEN 1 ELSE 0 END),
                 sum(CASE WHEN status='published' THEN 1 ELSE 0 END),
                 sum(CASE WHEN status='failed' THEN 1 ELSE 0 END)
                 FROM source_documents WHERE collection_id=?1",
                [collection_id.to_string()],
                |row| {
                    Ok(CollectionStats {
                        sources: row.get::<_, Option<u64>>(0)?.unwrap_or(0),
                        needs_review: row.get::<_, Option<u64>>(1)?.unwrap_or(0),
                        published: row.get::<_, Option<u64>>(2)?.unwrap_or(0),
                        failed: row.get::<_, Option<u64>>(3)?.unwrap_or(0),
                    })
                },
            )
            .map_err(Into::into)
    }

    pub(crate) fn lexical_candidates(
        &self,
        query: &str,
        collections: &[Uuid],
        purpose: SearchPurpose,
        limit: usize,
    ) -> Result<Vec<RankedChunk>> {
        if collections.is_empty() {
            return Ok(Vec::new());
        }
        let fts_query = fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = repeat_placeholders(collections.len(), 3);
        let external_clause = if purpose == SearchPurpose::ExternalAi {
            " AND col.allow_external_ai=1"
        } else {
            ""
        };
        let sql = format!(
            "SELECT ch.id,ch.concept_id,ch.source_document_id,ch.collection_id,ch.ordinal,
             ch.heading_or_page,ch.text,ch.text_sha256,ch.embedding,ch.source_revision,
             co.title,co.logical_resource_uri,sd.source_sha256,co.updated_at,bm25(chunk_fts)
             FROM chunk_fts JOIN chunks ch ON ch.id=chunk_fts.chunk_id
             JOIN concepts co ON co.id=ch.concept_id
             JOIN source_documents sd ON sd.id=ch.source_document_id
             JOIN collections col ON col.id=ch.collection_id
             WHERE chunk_fts MATCH ?1 AND co.status='published' AND sd.status='published'
             AND ch.collection_id IN ({placeholders}){external_clause}
             ORDER BY bm25(chunk_fts), ch.id LIMIT ?2"
        );
        let mut values: Vec<rusqlite::types::Value> = vec![
            fts_query.into(),
            i64::try_from(limit).unwrap_or(i64::MAX).into(),
        ];
        values.extend(collections.iter().map(|id| id.to_string().into()));
        let connection = self.connection()?;
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(values), ranked_chunk_from_row)?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    /// Scans one collection with a process-local row cursor.
    ///
    /// SQLite stores the table rowid in `chunks_collection`, so the
    /// `(collection_id, rowid)` predicate advances through that existing index
    /// without a growing OFFSET or a temporary sort. The cursor never leaves a
    /// single search and is not a durable chunk identity.
    pub(crate) fn vector_embedding_candidates_batch(
        &self,
        collection_id: Uuid,
        purpose: SearchPurpose,
        limit: usize,
        after_rowid: Option<i64>,
    ) -> Result<Vec<VectorEmbeddingCandidate>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let external_clause = if purpose == SearchPurpose::ExternalAi {
            " AND col.allow_external_ai=1"
        } else {
            ""
        };
        let cursor_clause = after_rowid
            .map(|_| " AND ch.rowid > ?2")
            .unwrap_or_default();
        let limit_parameter = 2 + usize::from(after_rowid.is_some());
        let sql = format!(
            "SELECT ch.rowid,ch.id,ch.embedding
             FROM chunks ch JOIN concepts co ON co.id=ch.concept_id
             JOIN source_documents sd ON sd.id=ch.source_document_id
             JOIN collections col ON col.id=ch.collection_id
             WHERE co.status='published' AND sd.status='published'
             AND ch.collection_id=?1{external_clause}{cursor_clause}
             ORDER BY ch.rowid LIMIT ?{limit_parameter}",
        );
        let mut values = vec![rusqlite::types::Value::from(collection_id.to_string())];
        if let Some(after_rowid) = after_rowid {
            values.push(after_rowid.into());
        }
        values.push(i64::try_from(limit).unwrap_or(i64::MAX).into());
        let connection = self.connection()?;
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(values), |row| {
            let bytes = row.get::<_, Vec<u8>>(2)?;
            Ok(VectorEmbeddingCandidate {
                scan_cursor: row.get(0)?,
                chunk_id: uuid_sql(row.get::<_, String>(1)?)?,
                embedding: decode_embedding(&bytes).map_err(to_sql_error)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    pub(crate) fn vector_candidates_by_id(
        &self,
        chunk_ids: &[Uuid],
        collections: &[Uuid],
        purpose: SearchPurpose,
    ) -> Result<Vec<RankedChunk>> {
        if chunk_ids.is_empty() || collections.is_empty() {
            return Ok(Vec::new());
        }
        let chunk_placeholders = repeat_placeholders(chunk_ids.len(), 1);
        let collection_placeholders = repeat_placeholders(collections.len(), chunk_ids.len() + 1);
        let external_clause = if purpose == SearchPurpose::ExternalAi {
            " AND col.allow_external_ai=1"
        } else {
            ""
        };
        let sql = format!(
            "SELECT ch.id,ch.concept_id,ch.source_document_id,ch.collection_id,ch.ordinal,
             ch.heading_or_page,ch.text,ch.text_sha256,ch.embedding,ch.source_revision,
             co.title,co.logical_resource_uri,sd.source_sha256,co.updated_at,NULL
             FROM chunks ch JOIN concepts co ON co.id=ch.concept_id
             JOIN source_documents sd ON sd.id=ch.source_document_id
             JOIN collections col ON col.id=ch.collection_id
             WHERE co.status='published' AND sd.status='published'
             AND ch.id IN ({chunk_placeholders})
             AND ch.collection_id IN ({collection_placeholders}){external_clause}"
        );
        let mut values = chunk_ids
            .iter()
            .map(|id| rusqlite::types::Value::from(id.to_string()))
            .collect::<Vec<_>>();
        values.extend(
            collections
                .iter()
                .map(|id| rusqlite::types::Value::from(id.to_string())),
        );
        let connection = self.connection()?;
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(values), ranked_chunk_from_row)?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    /// Revalidates an already ranked hit against the authoritative publication
    /// rows immediately before it leaves this node. This closes races with a
    /// concurrent modification, deletion, review withdrawal or cloud-policy
    /// restriction that occurred after candidates were loaded.
    pub fn hit_is_current(&self, hit: &SearchHit, purpose: SearchPurpose) -> Result<bool> {
        let external_clause = if purpose == SearchPurpose::ExternalAi {
            " AND col.allow_external_ai=1"
        } else {
            ""
        };
        let sql = format!(
            "SELECT ch.ordinal,ch.text_sha256 FROM chunks ch
                JOIN concepts co ON co.id=ch.concept_id
                JOIN source_documents sd ON sd.id=ch.source_document_id
                JOIN collections col ON col.id=ch.collection_id
                WHERE co.id=?1 AND ch.collection_id=?2
                  AND co.source_document_id=sd.id AND co.collection_id=ch.collection_id
                  AND sd.collection_id=ch.collection_id AND sd.concept_id=co.id
                  AND co.status='published' AND sd.status='published'
                  AND sd.source_sha256=?3 AND ch.source_revision=?4
                  AND ch.source_revision=sd.revision{external_clause}"
        );
        let connection = self.connection()?;
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query(params![
            hit.concept_id.to_string(),
            hit.collection_id.to_string(),
            hit.source_sha256,
            hit.source_revision,
        ])?;
        rows_contain_public_chunk(&mut rows, hit)
    }

    /// Atomically revalidates publication, trust, grant and collection egress
    /// policy immediately before evidence is returned to a LAN peer.
    pub fn peer_hit_is_current(
        &self,
        hit: &SearchHit,
        peer_id: &str,
        purpose: SearchPurpose,
    ) -> Result<bool> {
        let connection = self.connection()?;
        Self::peer_hit_is_current_on(&connection, hit, peer_id, purpose)
    }

    /// Revalidates publication and the collection's Internet opt-in without a
    /// pairing or per-reader grant.
    pub fn public_hit_is_current(&self, hit: &SearchHit) -> Result<bool> {
        let connection = self.connection()?;
        Self::public_hit_is_current_on(&connection, hit)
    }

    pub fn public_hit_is_current_under_disclosure(
        &self,
        lease: &DisclosureLease,
        hit: &SearchHit,
    ) -> Result<bool> {
        let connection = self.connection_under_disclosure(lease)?;
        Self::public_hit_is_current_on(&connection, hit)
    }

    fn public_hit_is_current_on(connection: &Connection, hit: &SearchHit) -> Result<bool> {
        let mut statement = connection.prepare(
            "SELECT ch.ordinal,ch.text_sha256 FROM chunks ch
                JOIN concepts co ON co.id=ch.concept_id
                JOIN source_documents sd ON sd.id=ch.source_document_id
                JOIN collections col ON col.id=ch.collection_id
                WHERE co.id=?1 AND ch.collection_id=?2
                  AND co.source_document_id=sd.id AND co.collection_id=ch.collection_id
                  AND sd.collection_id=ch.collection_id AND sd.concept_id=co.id
                  AND co.status='published' AND sd.status='published'
                  AND sd.source_sha256=?3 AND ch.source_revision=?4
                  AND ch.source_revision=sd.revision AND col.internet_public=1",
        )?;
        let mut rows = statement.query(params![
            hit.concept_id.to_string(),
            hit.collection_id.to_string(),
            hit.source_sha256,
            hit.source_revision,
        ])?;
        rows_contain_public_chunk(&mut rows, hit)
    }

    /// Revalidates a peer hit while retaining the disclosure lease through the
    /// transport handoff. The lease must originate from this database's gate.
    pub fn peer_hit_is_current_under_disclosure(
        &self,
        lease: &DisclosureLease,
        hit: &SearchHit,
        peer_id: &str,
        purpose: SearchPurpose,
    ) -> Result<bool> {
        let connection = self.connection_under_disclosure(lease)?;
        Self::peer_hit_is_current_on(&connection, hit, peer_id, purpose)
    }

    fn peer_hit_is_current_on(
        connection: &Connection,
        hit: &SearchHit,
        peer_id: &str,
        purpose: SearchPurpose,
    ) -> Result<bool> {
        let external_ai = purpose == SearchPurpose::ExternalAi;
        let mut statement = connection.prepare(
            "SELECT ch.ordinal,ch.text_sha256 FROM chunks ch
                JOIN concepts co ON co.id=ch.concept_id
                JOIN source_documents sd ON sd.id=ch.source_document_id
                JOIN collections col ON col.id=ch.collection_id
                JOIN grants g ON g.collection_id=ch.collection_id
                JOIN peers p ON p.peer_id=g.peer_id
                WHERE co.id=?1 AND ch.collection_id=?2
                  AND co.source_document_id=sd.id AND co.collection_id=ch.collection_id
                  AND sd.collection_id=ch.collection_id AND sd.concept_id=co.id
                  AND co.status='published' AND sd.status='published'
                  AND sd.source_sha256=?3 AND ch.source_revision=?4
                  AND ch.source_revision=sd.revision
                  AND g.peer_id=?5 AND p.trusted=1 AND p.blocked=0
                  AND col.peer_shareable=1 AND (?6=0 OR col.allow_external_ai=1)",
        )?;
        let mut rows = statement.query(params![
            hit.concept_id.to_string(),
            hit.collection_id.to_string(),
            hit.source_sha256,
            hit.source_revision,
            peer_id,
            external_ai,
        ])?;
        rows_contain_public_chunk(&mut rows, hit)
    }
}

fn rows_contain_public_chunk(rows: &mut rusqlite::Rows<'_>, hit: &SearchHit) -> Result<bool> {
    while let Some(row) = rows.next()? {
        let ordinal = row.get::<_, u32>(0)?;
        let text_sha256 = row.get::<_, String>(1)?;
        if public_chunk_id(&hit.source_sha256, ordinal, &text_sha256) == hit.chunk_id {
            return Ok(true);
        }
    }
    Ok(false)
}

fn sql_count(value: u64) -> Result<i64> {
    i64::try_from(value).context("collection maintenance count exceeds SQLite integer range")
}

fn withdraw_source(tx: &Transaction<'_>, source_document_id: Uuid) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    tx.execute(
        "UPDATE jobs SET state='cancelling',
         last_error='source revision changed while OKF publication was pending',updated_at=?2
         WHERE id IN (
           SELECT pc.job_id FROM publication_claims pc
           JOIN jobs j ON j.id=pc.job_id
           WHERE j.source_document_id=?1 AND j.kind='publish'
             AND j.state IN ('queued','running')
         )",
        params![source_document_id.to_string(), now],
    )?;
    let mut statement = tx.prepare("SELECT id FROM chunks WHERE source_document_id=?1")?;
    let chunk_ids = statement
        .query_map([source_document_id.to_string()], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for id in chunk_ids {
        tx.execute("DELETE FROM chunk_fts WHERE chunk_id=?1", [id])?;
    }
    tx.execute(
        "DELETE FROM chunks WHERE source_document_id=?1",
        [source_document_id.to_string()],
    )?;
    tx.execute(
        "UPDATE concepts SET status='needs_review',reviewed_at=NULL,updated_at=?2
         WHERE source_document_id=?1",
        params![source_document_id.to_string(), now],
    )?;
    Ok(())
}

fn delete_chunks_for_concept(tx: &Transaction<'_>, concept_id: Uuid) -> Result<()> {
    let mut statement = tx.prepare("SELECT id FROM chunks WHERE concept_id=?1")?;
    let ids = statement
        .query_map([concept_id.to_string()], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for id in ids {
        tx.execute("DELETE FROM chunk_fts WHERE chunk_id=?1", [id])?;
    }
    tx.execute(
        "DELETE FROM chunks WHERE concept_id=?1",
        [concept_id.to_string()],
    )?;
    Ok(())
}

fn collection_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CollectionRecord> {
    Ok(CollectionRecord {
        id: uuid_sql(row.get::<_, String>(0)?)?,
        name: row.get(1)?,
        source_folder: PathBuf::from(row.get::<_, String>(2)?),
        wiki_folder: PathBuf::from(row.get::<_, String>(3)?),
        policy: CollectionPolicy {
            local_only: row.get(4)?,
            peer_shareable: row.get(5)?,
            allow_external_ai: row.get(6)?,
            internet_public: row.get(7)?,
        },
        created_at: datetime_sql(row.get::<_, String>(8)?)?,
        updated_at: datetime_sql(row.get::<_, String>(9)?)?,
    })
}

fn public_collection_profile_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PublicCollectionProfileRecord> {
    Ok(PublicCollectionProfileRecord {
        collection_id: uuid_sql(row.get::<_, String>(0)?)?,
        description: row.get(1)?,
        languages: json_sql(row.get::<_, String>(2)?)?,
        manifest_sequence: row.get(3)?,
        enabled_at: row
            .get::<_, Option<String>>(4)?
            .map(datetime_sql)
            .transpose()?,
        updated_at: datetime_sql(row.get::<_, String>(5)?)?,
    })
}

fn federation_index_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FederationIndexRecord> {
    Ok(FederationIndexRecord {
        peer_id: row.get(0)?,
        multiaddr: row.get(1)?,
        enabled: row.get(2)?,
        source: row.get(3)?,
        registry_version: row.get(4)?,
        expires_at: row
            .get::<_, Option<String>>(5)?
            .map(datetime_sql)
            .transpose()?,
        created_at: datetime_sql(row.get::<_, String>(6)?)?,
        updated_at: datetime_sql(row.get::<_, String>(7)?)?,
    })
}

fn collection_maintenance_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CollectionMaintenanceRecord> {
    let status = row
        .get::<_, String>(4)?
        .parse::<CollectionMaintenanceStatus>()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    error.to_string(),
                )),
            )
        })?;
    Ok(CollectionMaintenanceRecord {
        collection_id: uuid_sql(row.get::<_, String>(0)?)?,
        last_started_at: row
            .get::<_, Option<String>>(1)?
            .map(datetime_sql)
            .transpose()?,
        last_finished_at: row
            .get::<_, Option<String>>(2)?
            .map(datetime_sql)
            .transpose()?,
        last_success_at: row
            .get::<_, Option<String>>(3)?
            .map(datetime_sql)
            .transpose()?,
        status,
        counts: CollectionMaintenanceCounts {
            analyzed: row.get(5)?,
            unchanged: row.get(6)?,
            renamed: row.get(7)?,
            deleted: row.get(8)?,
            failed: row.get(9)?,
        },
        issue_code: row.get(10)?,
        issue_summary: row.get(11)?,
    })
}

fn source_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceDocumentRecord> {
    Ok(SourceDocumentRecord {
        id: uuid_sql(row.get::<_, String>(0)?)?,
        collection_id: uuid_sql(row.get::<_, String>(1)?)?,
        source_path: PathBuf::from(row.get::<_, String>(2)?),
        source_sha256: row.get(3)?,
        source_format: row.get(4)?,
        byte_size: row.get(5)?,
        page_count: row.get(6)?,
        character_count: row.get(7)?,
        status: status_sql(row.get::<_, String>(8)?)?,
        revision: row.get(9)?,
        concept_id: row
            .get::<_, Option<String>>(10)?
            .map(uuid_sql)
            .transpose()?,
        last_error: row.get(11)?,
        discovered_at: datetime_sql(row.get::<_, String>(12)?)?,
        updated_at: datetime_sql(row.get::<_, String>(13)?)?,
        deleted_at: row
            .get::<_, Option<String>>(14)?
            .map(datetime_sql)
            .transpose()?,
    })
}

fn grant_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GrantRecord> {
    Ok(GrantRecord {
        peer_id: row.get(0)?,
        collection_id: uuid_sql(row.get::<_, String>(1)?)?,
        granted_at: datetime_sql(row.get::<_, String>(2)?)?,
    })
}

fn concept_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConceptRecord> {
    let concept_type = concept_type_sql(row.get::<_, String>(3)?)?;
    Ok(ConceptRecord {
        id: uuid_sql(row.get::<_, String>(0)?)?,
        source_document_id: uuid_sql(row.get::<_, String>(1)?)?,
        collection_id: uuid_sql(row.get::<_, String>(2)?)?,
        draft: EnrichmentDraft {
            concept_type,
            title: row.get(4)?,
            description: row.get(5)?,
            language: row.get(6)?,
            tags: json_sql(row.get::<_, String>(7)?)?,
            entities: json_sql(row.get::<_, String>(8)?)?,
            links: json_sql(row.get::<_, String>(9)?)?,
            summary: row.get(10)?,
            classification_confidence: row.get(11)?,
            classification_explanation: row.get(12)?,
        },
        logical_resource_uri: row.get(13)?,
        generator_model: row.get(14)?,
        status: status_sql(row.get::<_, String>(15)?)?,
        reviewed_at: row
            .get::<_, Option<String>>(16)?
            .map(datetime_sql)
            .transpose()?,
        created_at: datetime_sql(row.get::<_, String>(17)?)?,
        updated_at: datetime_sql(row.get::<_, String>(18)?)?,
    })
}

fn chunk_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredChunk> {
    let bytes: Vec<u8> = row.get(8)?;
    Ok(StoredChunk {
        id: uuid_sql(row.get::<_, String>(0)?)?,
        concept_id: uuid_sql(row.get::<_, String>(1)?)?,
        source_document_id: uuid_sql(row.get::<_, String>(2)?)?,
        collection_id: uuid_sql(row.get::<_, String>(3)?)?,
        ordinal: row.get(4)?,
        heading_or_page: row.get(5)?,
        text: row.get(6)?,
        text_sha256: row.get(7)?,
        embedding: decode_embedding(&bytes).map_err(to_sql_error)?,
        source_revision: row.get(9)?,
    })
}

fn ranked_chunk_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RankedChunk> {
    let chunk = chunk_from_row(row)?;
    Ok(RankedChunk {
        chunk,
        title: row.get(10)?,
        logical_resource_uri: row.get(11)?,
        source_sha256: row.get(12)?,
        updated_at: datetime_sql(row.get::<_, String>(13)?)?,
        lexical_score: row.get(14)?,
    })
}

fn publication_claim_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PublicationClaim> {
    Ok(PublicationClaim {
        job_id: uuid_sql(row.get::<_, String>(0)?)?,
        concept_id: uuid_sql(row.get::<_, String>(1)?)?,
        source_document_id: uuid_sql(row.get::<_, String>(2)?)?,
        collection_id: uuid_sql(row.get::<_, String>(3)?)?,
        source_path: PathBuf::from(row.get::<_, String>(4)?),
        source_sha256: row.get(5)?,
        source_revision: row.get(6)?,
        action: row.get(7)?,
        reviewed_at: datetime_sql(row.get::<_, String>(8)?)?,
        job_state: row.get(9)?,
    })
}

fn validate_draft(draft: &EnrichmentDraft) -> Result<()> {
    if draft.title.is_empty() || draft.description.is_empty() || draft.language.is_empty() {
        bail!("enrichment title, description and language are required");
    }
    if draft.tags.len() > 10 {
        bail!("enrichment may contain at most ten tags");
    }
    Ok(())
}

fn source_registration_by_path(
    tx: &Transaction<'_>,
    collection_id: Uuid,
    source_path: &Path,
    source_path_text: &str,
) -> Result<Option<(String, String, String)>> {
    let exact = tx
        .query_row(
            "SELECT id,source_sha256,status FROM source_documents
             WHERE collection_id=?1 AND source_path=?2",
            params![collection_id.to_string(), source_path_text],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    if exact.is_some() {
        return Ok(exact);
    }

    // Filesystem enumeration can return another textual alias for the same
    // existing path (notably drive-letter casing and verbatim prefixes on
    // Windows). Resolve aliases only after the indexed exact lookup misses so
    // ordinary registration remains cheap and hardlinks stay distinct.
    let Ok(candidate_identity) = std::fs::canonicalize(source_path) else {
        return Ok(None);
    };
    let sources = {
        let mut statement = tx.prepare(
            "SELECT id,source_sha256,status,source_path FROM source_documents
             WHERE collection_id=?1",
        )?;
        statement
            .query_map([collection_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    PathBuf::from(row.get::<_, String>(3)?),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut aliases = sources.into_iter().filter_map(|(id, hash, status, path)| {
        let identity = std::fs::canonicalize(path).ok()?;
        (identity == candidate_identity).then_some((id, hash, status))
    });
    let matched = aliases.next();
    if aliases.next().is_some() {
        bail!("multiple source records resolve to the same filesystem path");
    }
    Ok(matched)
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn path_is_definitely_missing(path: &Path) -> bool {
    matches!(
        std::fs::symlink_metadata(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    )
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn parse_uuid(value: &str) -> Result<Uuid> {
    Uuid::from_str(value).with_context(|| format!("invalid UUID in database: {value}"))
}

fn uuid_sql(value: String) -> rusqlite::Result<Uuid> {
    parse_uuid(&value).map_err(to_sql_error)
}

fn datetime_sql(value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(to_sql_error)
}

fn optional_datetime(value: Option<String>) -> rusqlite::Result<Option<DateTime<Utc>>> {
    value.map(datetime_sql).transpose()
}

fn status_sql(value: String) -> rusqlite::Result<DocumentStatus> {
    match value.as_str() {
        "detected" => Ok(DocumentStatus::Detected),
        "extracted" => Ok(DocumentStatus::Extracted),
        "enriched" => Ok(DocumentStatus::Enriched),
        "needs_review" => Ok(DocumentStatus::NeedsReview),
        "publishing" => Ok(DocumentStatus::Publishing),
        "published" => Ok(DocumentStatus::Published),
        "deleted" => Ok(DocumentStatus::Deleted),
        "failed" => Ok(DocumentStatus::Failed),
        _ => Err(to_sql_error(anyhow!("invalid document status {value}"))),
    }
}

fn concept_type_sql(value: String) -> rusqlite::Result<ConceptType> {
    match value.as_str() {
        "Document" => Ok(ConceptType::Document),
        "Policy" => Ok(ConceptType::Policy),
        "Procedure" => Ok(ConceptType::Procedure),
        "Runbook" => Ok(ConceptType::Runbook),
        "Reference" => Ok(ConceptType::Reference),
        "Report" => Ok(ConceptType::Report),
        _ => Err(to_sql_error(anyhow!("invalid concept type {value}"))),
    }
}

fn json_sql<T: for<'de> Deserialize<'de>>(value: String) -> rusqlite::Result<T> {
    serde_json::from_str(&value).map_err(to_sql_error)
}

fn to_sql_error(error: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::other(error.to_string())),
    )
}

fn ensure_changed(count: usize, kind: &str, id: Uuid) -> Result<()> {
    if count == 0 {
        bail!("{kind} {id} does not exist");
    }
    Ok(())
}

pub(crate) fn encode_embedding(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

pub(crate) fn decode_embedding(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(std::mem::size_of::<f32>()) {
        bail!("embedding BLOB has invalid byte length {}", bytes.len());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four-byte chunk")))
        .collect())
}

fn fts_query(query: &str) -> String {
    query
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|word| !word.is_empty())
        .take(32)
        .map(|word| format!("\"{}\"", word.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn repeat_placeholders(count: usize, start_index: usize) -> String {
    (0..count)
        .map(|index| format!("?{}", start_index + index))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> EnrichmentDraft {
        EnrichmentDraft {
            concept_type: ConceptType::Runbook,
            title: "Recuperación de pagos".into(),
            description: "Procedimiento probado".into(),
            language: "es".into(),
            tags: vec!["pagos".into()],
            entities: vec![],
            links: vec![],
            summary: "Restaurar el servicio.".into(),
            classification_confidence: 0.9,
            classification_explanation: "contiene pasos".into(),
        }
    }

    fn setup() -> (tempfile::TempDir, Database, CollectionRecord) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let wiki = temp.path().join("wiki");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&wiki).unwrap();
        let db = Database::in_memory().unwrap();
        let collection = db
            .create_collection(
                "Test",
                &source,
                &wiki,
                CollectionPolicy::shared_with_peers(),
            )
            .unwrap();
        (temp, db, collection)
    }

    fn setup_review_evidence(
        chunk_count: u32,
    ) -> (tempfile::TempDir, Database, CollectionRecord, Uuid, Uuid) {
        let (temp, db, collection) = setup();
        let path = temp.path().join("source/review.md");
        std::fs::write(&path, "review evidence").unwrap();
        let source_id = db
            .register_source(collection.id, &path, "source-hash", "markdown", 15)
            .unwrap()
            .id();
        db.mark_extracted(source_id, 0, 15).unwrap();
        let concept_id = db
            .save_enrichment(source_id, draft(), "peer-a", "fake")
            .unwrap()
            .id;
        let chunks = (0..chunk_count)
            .map(|ordinal| StoredChunk {
                id: Uuid::new_v4(),
                concept_id,
                source_document_id: source_id,
                collection_id: collection.id,
                ordinal,
                heading_or_page: format!("Section {ordinal}"),
                text: format!("Evidence {ordinal}"),
                text_sha256: format!("hash-{ordinal}"),
                embedding: vec![0.0; EMBEDDING_DIMENSIONS],
                source_revision: 1,
            })
            .collect::<Vec<_>>();
        db.replace_chunks(concept_id, &chunks).unwrap();
        (temp, db, collection, source_id, concept_id)
    }

    #[test]
    fn review_evidence_pages_twenty_chunks_and_continues_without_embeddings() {
        let (_temp, db, _collection, _source_id, concept_id) = setup_review_evidence(25);
        db.connection()
            .unwrap()
            .execute(
                "UPDATE chunks SET embedding=x'00' WHERE concept_id=?1",
                [concept_id.to_string()],
            )
            .unwrap();

        let first = db
            .review_evidence_page(concept_id, 1, None, None, 20)
            .unwrap()
            .unwrap();
        assert_eq!(first.total_chunks, 25);
        assert_eq!(first.chunks.len(), 20);
        assert_eq!(first.chunks.first().map(|chunk| chunk.ordinal), Some(0));
        assert_eq!(first.chunks.last().map(|chunk| chunk.ordinal), Some(19));
        assert_eq!(first.next_ordinal, Some(19));
        let review_version = first.review_version.clone();

        let second = db
            .review_evidence_page(concept_id, 1, Some(&review_version), first.next_ordinal, 20)
            .unwrap()
            .unwrap();
        assert_eq!(second.review_version, review_version);
        assert_eq!(second.total_chunks, 25);
        assert_eq!(second.chunks.len(), 5);
        assert_eq!(second.chunks.first().map(|chunk| chunk.ordinal), Some(20));
        assert_eq!(second.chunks.last().map(|chunk| chunk.ordinal), Some(24));
        assert_eq!(second.next_ordinal, None);
    }

    #[test]
    fn review_version_changes_when_chunks_are_replaced_for_the_same_source_revision() {
        let (_temp, db, _collection, _source_id, concept_id) = setup_review_evidence(1);
        let first = db
            .review_evidence_page(concept_id, 1, None, None, 20)
            .unwrap()
            .unwrap();
        let old_review_version = first.review_version;
        let mut chunks = db.chunks_for_concept(concept_id).unwrap();
        chunks[0].text = "Regenerated evidence".into();
        chunks[0].text_sha256 = "regenerated-hash".into();
        db.replace_chunks(concept_id, &chunks).unwrap();

        let refreshed = db
            .review_evidence_page(concept_id, 1, None, None, 20)
            .unwrap()
            .unwrap();
        assert_ne!(refreshed.review_version, old_review_version);
        assert!(
            db.review_evidence_page(concept_id, 1, Some(&old_review_version), None, 20,)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn review_version_changes_when_draft_or_model_is_reenriched() {
        let (_temp, db, _collection, source_id, concept_id) = setup_review_evidence(1);
        let old_review_version = db
            .review_evidence_page(concept_id, 1, None, None, 20)
            .unwrap()
            .unwrap()
            .review_version;
        let mut changed_draft = draft();
        changed_draft.summary = "A different generated summary".into();
        db.save_enrichment(
            source_id,
            changed_draft,
            "peer-a",
            "different-generator-model",
        )
        .unwrap();

        let current = db
            .review_evidence_page(concept_id, 1, None, None, 20)
            .unwrap()
            .unwrap();
        assert_ne!(current.review_version, old_review_version);
    }

    #[test]
    fn stale_review_version_cannot_create_a_publication_claim() {
        let (_temp, db, _collection, source_id, concept_id) = setup_review_evidence(1);
        let old_review_version = db
            .review_evidence_page(concept_id, 1, None, None, 20)
            .unwrap()
            .unwrap()
            .review_version;
        let mut chunks = db.chunks_for_concept(concept_id).unwrap();
        chunks[0].text = "Evidence changed after review".into();
        chunks[0].text_sha256 = "changed-after-review".into();
        db.replace_chunks(concept_id, &chunks).unwrap();

        let error = db
            .begin_publication_if_current(
                concept_id,
                draft(),
                ExpectedReview {
                    source_sha256: "source-hash",
                    source_revision: 1,
                    review_version: &old_review_version,
                },
                "published",
                Utc::now(),
            )
            .unwrap_err();

        assert!(error.to_string().contains("current review"));
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::NeedsReview
        );
        assert_eq!(
            db.source_document(source_id).unwrap().unwrap().status,
            DocumentStatus::NeedsReview
        );
        assert_eq!(db.count("publication_claims").unwrap(), 0);
        assert_eq!(db.count("jobs").unwrap(), 0);
    }

    #[test]
    fn review_evidence_returns_an_empty_page_for_a_current_review_without_chunks() {
        let (_temp, db, _collection, _source_id, concept_id) = setup_review_evidence(0);

        let page = db
            .review_evidence_page(concept_id, 1, None, None, 20)
            .unwrap()
            .unwrap();

        assert_eq!(page.total_chunks, 0);
        assert!(page.chunks.is_empty());
        assert_eq!(page.next_ordinal, None);
    }

    #[test]
    fn review_evidence_is_stale_after_publication() {
        let (_temp, db, _collection, _source_id, concept_id) = setup_review_evidence(1);
        db.approve_concept(concept_id, draft()).unwrap();

        assert!(
            db.review_evidence_page(concept_id, 1, None, None, 20)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn review_evidence_is_stale_after_source_status_changes() {
        let (_temp, db, _collection, source_id, concept_id) = setup_review_evidence(1);
        db.mark_source_status(source_id, DocumentStatus::Failed)
            .unwrap();

        assert!(
            db.review_evidence_page(concept_id, 1, None, None, 20)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn review_evidence_is_stale_after_source_revision_changes() {
        let (_temp, db, _collection, source_id, concept_id) = setup_review_evidence(1);
        db.connection()
            .unwrap()
            .execute(
                "UPDATE source_documents SET revision=2 WHERE id=?1",
                [source_id.to_string()],
            )
            .unwrap();

        assert!(
            db.review_evidence_page(concept_id, 1, None, None, 20)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn review_evidence_rejects_corrupt_ownership_outside_the_requested_page() {
        let (temp, db, _collection, _source_id, concept_id) = setup_review_evidence(25);
        let other_source = temp.path().join("other-source");
        let other_wiki = temp.path().join("other-wiki");
        std::fs::create_dir_all(&other_source).unwrap();
        std::fs::create_dir_all(&other_wiki).unwrap();
        let other_collection = db
            .create_collection(
                "Other",
                &other_source,
                &other_wiki,
                CollectionPolicy::local_only(),
            )
            .unwrap();
        db.connection()
            .unwrap()
            .execute(
                "UPDATE chunks SET collection_id=?1 WHERE concept_id=?2 AND ordinal=24",
                params![other_collection.id.to_string(), concept_id.to_string()],
            )
            .unwrap();

        let error = db
            .review_evidence_page(concept_id, 1, None, None, 20)
            .unwrap_err();
        assert!(error.to_string().contains("ownership and revision"));
    }

    #[test]
    fn review_evidence_debug_output_redacts_source_text() {
        let secret = "DO-NOT-LOG-THIS-EVIDENCE";
        let concept_id = Uuid::new_v4();
        let chunk = ReviewEvidenceChunkRecord {
            ordinal: 7,
            heading_or_page: secret.into(),
            text: secret.into(),
        };
        let page = ReviewEvidencePageRecord {
            concept_id,
            source_revision: 1,
            review_version: ReviewVersionToken::from_digest([0xab; 32]),
            total_chunks: 1,
            chunks: vec![chunk.clone()],
            next_ordinal: None,
        };

        let chunk_debug = format!("{chunk:?}");
        let page_debug = format!("{page:?}");
        let token_debug = format!("{:?}", page.review_version);
        assert!(!chunk_debug.contains(secret));
        assert!(!page_debug.contains(secret));
        assert!(!page_debug.contains(&concept_id.to_string()));
        assert!(!page_debug.contains("ReviewVersionToken"));
        assert_eq!(token_debug, "ReviewVersionToken([REDACTED])");
        assert!(chunk_debug.contains("text_len"));
        assert!(page_debug.contains("page_chunk_count"));
    }

    #[test]
    fn review_evidence_rejects_limits_outside_the_public_contract() {
        let (_temp, db, _collection, _source_id, concept_id) = setup_review_evidence(1);

        assert!(
            db.review_evidence_page(concept_id, 1, None, None, 0)
                .is_err()
        );
        assert!(
            db.review_evidence_page(concept_id, 1, None, None, 101)
                .is_err()
        );
    }

    #[test]
    fn disclosure_lease_blocks_a_concurrent_policy_write() {
        use std::sync::mpsc;

        let (_temp, database, collection) = setup();
        let lease = database.disclosure_gate().acquire_disclosure();
        let (finished_tx, finished_rx) = mpsc::channel();
        let updating_database = database.clone();
        let update = std::thread::spawn(move || {
            let result = updating_database
                .update_collection_policy(collection.id, CollectionPolicy::local_only());
            finished_tx.send(result).ok();
        });

        assert!(
            finished_rx
                .recv_timeout(std::time::Duration::from_millis(25))
                .is_err()
        );
        drop(lease);
        finished_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
            .unwrap();
        update.join().unwrap();
        assert_eq!(
            database.collection(collection.id).unwrap().unwrap().policy,
            CollectionPolicy::local_only()
        );
    }

    #[test]
    fn migration_builds_every_required_table_and_wal_database_reopens() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("db.sqlite");
        let db = Database::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), 5);
        for table in [
            "collections",
            "source_documents",
            "concepts",
            "chunks",
            "jobs",
            "publication_claims",
            "peers",
            "grants",
            "audit_events",
            "collection_maintenance",
            "public_collection_profiles",
            "federation_indexes",
            "public_publisher_blocks",
        ] {
            assert_eq!(db.count(table).unwrap(), 0);
        }
        drop(db);
        assert_eq!(Database::open(path).unwrap().schema_version().unwrap(), 5);
    }

    #[test]
    fn collection_policy_persists_independent_egress_opt_ins() {
        let (_temp, db, collection) = setup();
        db.update_collection_policy(
            collection.id,
            CollectionPolicy {
                local_only: true,
                peer_shareable: false,
                allow_external_ai: true,
                internet_public: false,
            },
        )
        .unwrap();

        assert_eq!(
            db.collection(collection.id).unwrap().unwrap().policy,
            CollectionPolicy {
                local_only: false,
                peer_shareable: false,
                allow_external_ai: true,
                internet_public: false,
            }
        );

        db.update_collection_policy(
            collection.id,
            CollectionPolicy {
                local_only: false,
                peer_shareable: false,
                allow_external_ai: false,
                internet_public: false,
            },
        )
        .unwrap();
        assert_eq!(
            db.collection(collection.id).unwrap().unwrap().policy,
            CollectionPolicy::local_only()
        );

        db.update_collection_policy(
            collection.id,
            CollectionPolicy {
                local_only: true,
                peer_shareable: false,
                allow_external_ai: false,
                internet_public: true,
            },
        )
        .unwrap();
        let public = db.collection(collection.id).unwrap().unwrap();
        assert!(!public.policy.local_only);
        assert!(public.policy.internet_public);
        let enabled = db
            .public_collection_profile(collection.id)
            .unwrap()
            .unwrap();
        assert_eq!(enabled.manifest_sequence, 1);
        assert!(enabled.enabled_at.is_some());

        db.update_collection_policy(collection.id, CollectionPolicy::local_only())
            .unwrap();
        let disabled = db
            .public_collection_profile(collection.id)
            .unwrap()
            .unwrap();
        assert_eq!(disabled.manifest_sequence, 2);
        assert!(disabled.enabled_at.is_none());
    }

    #[test]
    fn public_publisher_blocks_are_persistent_and_reversible() {
        let (_temp, db, _collection) = setup();
        let publisher = "12D3KooWSyntheticPublisher";

        assert!(!db.public_publisher_is_blocked(publisher).unwrap());
        db.set_public_publisher_blocked(publisher, true).unwrap();
        assert!(db.public_publisher_is_blocked(publisher).unwrap());
        assert_eq!(db.list_blocked_public_publishers().unwrap(), [publisher]);

        db.set_public_publisher_blocked(publisher, false).unwrap();
        assert!(!db.public_publisher_is_blocked(publisher).unwrap());
    }

    #[test]
    fn bootstrap_indexes_preserve_registry_version_and_expiry() {
        let (_temp, db, _collection) = setup();
        let expiry = Utc::now() + chrono::Duration::days(30);
        db.upsert_bootstrap_federation_index(
            "12D3KooWSyntheticBootstrap",
            "/ip4/203.0.113.10/tcp/42042",
            1,
            expiry,
        )
        .unwrap();

        let indexes = db.list_federation_indexes().unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].source, "bootstrap");
        assert_eq!(indexes[0].registry_version, 1);
        assert_eq!(indexes[0].expires_at, Some(expiry));
    }

    #[test]
    fn bootstrap_indexes_reject_downgrade_and_same_version_mutation() {
        let (_temp, db, _collection) = setup();
        let peer = "12D3KooWSyntheticBootstrap";
        let first_address = "/ip4/203.0.113.10/tcp/42042";
        let expiry = Utc::now() + chrono::Duration::days(30);
        db.upsert_bootstrap_federation_index(peer, first_address, 2, expiry)
            .unwrap();

        assert!(
            db.upsert_bootstrap_federation_index(peer, first_address, 1, expiry)
                .is_err()
        );
        assert!(
            db.upsert_bootstrap_federation_index(peer, "/ip4/203.0.113.11/tcp/42042", 2, expiry,)
                .is_err()
        );
        db.upsert_bootstrap_federation_index(peer, first_address, 2, expiry)
            .unwrap();

        let indexes = db.list_federation_indexes().unwrap();
        assert_eq!(indexes[0].multiaddr, first_address);
        assert_eq!(indexes[0].registry_version, 2);
    }

    #[test]
    fn collection_source_folder_can_be_relinked_without_changing_identity_or_policy() {
        let (temp, db, collection) = setup();
        let replacement = temp.path().join("replacement");
        std::fs::create_dir_all(&replacement).unwrap();

        db.update_collection_source_folder(collection.id, &replacement)
            .unwrap();

        let updated = db.collection(collection.id).unwrap().unwrap();
        assert_eq!(updated.id, collection.id);
        assert_eq!(updated.source_folder, replacement);
        assert_eq!(updated.wiki_folder, collection.wiki_folder);
        assert_eq!(updated.policy, collection.policy);
    }

    #[test]
    fn migration_two_preserves_version_one_collections_and_sources() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("version-one.sqlite");
        let mut connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        let tx = connection.transaction().unwrap();
        tx.execute_batch(MIGRATION_1).unwrap();
        let collection_id = Uuid::new_v4();
        let source_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO collections
             (id,name,source_folder,wiki_folder,created_at,updated_at)
             VALUES (?1,'Existing','/synthetic/source','/synthetic/wiki',?2,?2)",
            params![collection_id.to_string(), now],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO source_documents
             (id,collection_id,source_path,source_sha256,source_format,byte_size,status,
              revision,discovered_at,updated_at)
             VALUES (?1,?2,'/synthetic/source/a.md',?3,'markdown',1,'detected',1,?4,?4)",
            params![
                source_id.to_string(),
                collection_id.to_string(),
                "a".repeat(64),
                now,
            ],
        )
        .unwrap();
        tx.pragma_update(None, "user_version", 1).unwrap();
        tx.commit().unwrap();
        drop(connection);

        let database = Database::open(&path).unwrap();
        assert_eq!(database.schema_version().unwrap(), 5);
        assert_eq!(database.count("collections").unwrap(), 1);
        assert_eq!(database.count("source_documents").unwrap(), 1);
        assert_eq!(database.count("publication_claims").unwrap(), 0);
        assert_eq!(
            database
                .source_document(source_id)
                .unwrap()
                .unwrap()
                .source_sha256,
            "a".repeat(64)
        );
    }

    #[test]
    fn migration_three_preserves_version_two_collection_state() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("version-two.sqlite");
        let mut connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        let tx = connection.transaction().unwrap();
        tx.execute_batch(MIGRATION_1).unwrap();
        tx.execute_batch(MIGRATION_2).unwrap();
        let collection_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO collections
             (id,name,source_folder,wiki_folder,peer_shareable,created_at,updated_at)
             VALUES (?1,'Existing','/synthetic/source','/synthetic/wiki',1,?2,?2)",
            params![collection_id.to_string(), now],
        )
        .unwrap();
        tx.pragma_update(None, "user_version", 2).unwrap();
        tx.commit().unwrap();
        drop(connection);

        let database = Database::open(&path).unwrap();

        assert_eq!(database.schema_version().unwrap(), 5);
        assert!(
            database
                .collection(collection_id)
                .unwrap()
                .unwrap()
                .policy
                .peer_shareable
        );
        assert_eq!(database.count("collection_maintenance").unwrap(), 0);
    }

    #[test]
    fn migration_four_keeps_existing_collections_private() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("version-three.sqlite");
        let mut connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        let tx = connection.transaction().unwrap();
        tx.execute_batch(MIGRATION_1).unwrap();
        tx.execute_batch(MIGRATION_2).unwrap();
        tx.execute_batch(MIGRATION_3).unwrap();
        let collection_id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO collections
             (id,name,source_folder,wiki_folder,peer_shareable,allow_external_ai,created_at,updated_at)
             VALUES (?1,'Existing','/synthetic/source','/synthetic/wiki',1,1,?2,?2)",
            params![collection_id.to_string(), now],
        )
        .unwrap();
        tx.pragma_update(None, "user_version", 3).unwrap();
        tx.commit().unwrap();
        drop(connection);

        let database = Database::open(&path).unwrap();
        let collection = database.collection(collection_id).unwrap().unwrap();
        assert_eq!(database.schema_version().unwrap(), 5);
        assert!(collection.policy.peer_shareable);
        assert!(collection.policy.allow_external_ai);
        assert!(!collection.policy.internet_public);
        assert!(
            database
                .public_collection_profile(collection_id)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn migration_five_preserves_public_indexes_and_adds_private_blocks() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("version-four.sqlite");
        let mut connection = Connection::open(&path).unwrap();
        let tx = connection.transaction().unwrap();
        tx.execute_batch(MIGRATION_1).unwrap();
        tx.execute_batch(MIGRATION_2).unwrap();
        tx.execute_batch(MIGRATION_3).unwrap();
        tx.execute_batch(MIGRATION_4).unwrap();
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "INSERT INTO federation_indexes(peer_id,multiaddr,enabled,source,created_at,updated_at)
             VALUES ('synthetic','/ip4/127.0.0.1/tcp/42042',1,'community',?1,?1)",
            [&now],
        )
        .unwrap();
        tx.pragma_update(None, "user_version", 4).unwrap();
        tx.commit().unwrap();
        drop(connection);

        let database = Database::open(path).unwrap();
        assert_eq!(database.schema_version().unwrap(), 5);
        let indexes = database.list_federation_indexes().unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].registry_version, 0);
        assert!(indexes[0].expires_at.is_none());
        assert_eq!(database.count("public_publisher_blocks").unwrap(), 0);
    }

    #[test]
    fn collection_maintenance_tracks_start_and_successful_completion() {
        let (_temp, db, collection) = setup();
        db.start_collection_maintenance(collection.id).unwrap();
        let started = db.collection_maintenance(collection.id).unwrap().unwrap();
        assert_eq!(started.status, CollectionMaintenanceStatus::Never);
        assert!(started.last_started_at.is_some());
        assert!(started.last_finished_at.is_none());

        let counts = CollectionMaintenanceCounts {
            analyzed: 2,
            unchanged: 3,
            renamed: 1,
            deleted: 1,
            failed: 0,
        };
        db.finish_collection_maintenance(
            collection.id,
            &CollectionMaintenanceResult::success(counts),
        )
        .unwrap();
        let finished = db.collection_maintenance(collection.id).unwrap().unwrap();

        assert_eq!(finished.status, CollectionMaintenanceStatus::Success);
        assert_eq!(finished.counts, counts);
        assert!(finished.last_finished_at.is_some());
        assert!(finished.last_success_at.is_some());
        assert!(finished.issue_code.is_none());
    }

    #[test]
    fn collection_maintenance_failure_preserves_last_success() {
        let (_temp, db, collection) = setup();
        db.finish_collection_maintenance(
            collection.id,
            &CollectionMaintenanceResult::success(CollectionMaintenanceCounts::default()),
        )
        .unwrap();
        let successful_at = db
            .collection_maintenance(collection.id)
            .unwrap()
            .unwrap()
            .last_success_at;
        let failure = CollectionMaintenanceResult::issue(
            CollectionMaintenanceStatus::Failed,
            CollectionMaintenanceCounts {
                failed: 1,
                ..CollectionMaintenanceCounts::default()
            },
            "collection_scan_failed",
            "The collection could not be reconciled.",
        )
        .unwrap();
        db.finish_collection_maintenance(collection.id, &failure)
            .unwrap();

        assert_eq!(
            db.collection_maintenance(collection.id)
                .unwrap()
                .unwrap()
                .last_success_at,
            successful_at
        );
    }

    #[test]
    fn collection_maintenance_rejects_unstable_issue_codes() {
        let result = CollectionMaintenanceResult::issue(
            CollectionMaintenanceStatus::Partial,
            CollectionMaintenanceCounts::default(),
            "/private/source/document.md",
            "A file failed.",
        );

        assert!(result.is_err());
    }

    #[test]
    fn registration_is_idempotent_and_withdraws_changed_publication() {
        let (temp, db, collection) = setup();
        let path = temp.path().join("source/a.md");
        std::fs::write(&path, "hello").unwrap();
        let first = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();
        assert!(matches!(first, SourceRegistration::New(_)));
        db.mark_source_status(first.id(), DocumentStatus::NeedsReview)
            .unwrap();
        let same = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();
        assert!(matches!(same, SourceRegistration::Unchanged(_)));
        let changed = db
            .register_source(collection.id, &path, "bbb", "markdown", 6)
            .unwrap();
        assert!(matches!(
            changed,
            SourceRegistration::Replaced {
                ref previous_source_sha256,
                ..
            } if previous_source_sha256 == "aaa"
        ));
        assert_eq!(first.id(), changed.id());
        assert_eq!(db.count("source_documents").unwrap(), 1);
        assert_eq!(db.source_document(first.id()).unwrap().unwrap().revision, 2);
    }

    #[test]
    fn registration_reuses_an_existing_canonical_path_alias() {
        let (temp, db, collection) = setup();
        let path = temp.path().join("source/alias.md");
        std::fs::write(&path, "hello").unwrap();
        let alias = path
            .parent()
            .unwrap()
            .join(".")
            .join(path.file_name().unwrap());
        assert_ne!(path_text(&alias), path_text(&path));
        assert_eq!(
            std::fs::canonicalize(&alias).unwrap(),
            std::fs::canonicalize(&path).unwrap()
        );

        let first = db
            .register_source(collection.id, &alias, "aaa", "markdown", 5)
            .unwrap();
        db.mark_source_failed(first.id(), "synthetic interruption")
            .unwrap();
        let retry = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();

        assert!(matches!(retry, SourceRegistration::Changed(id) if id == first.id()));
        assert_eq!(db.count("source_documents").unwrap(), 1);
        assert_eq!(
            db.source_document(first.id()).unwrap().unwrap().source_path,
            path
        );
    }

    #[test]
    fn identical_files_at_distinct_existing_paths_keep_distinct_identities() {
        let (temp, db, collection) = setup();
        let first_path = temp.path().join("source/first.md");
        let second_path = temp.path().join("source/second.md");
        std::fs::write(&first_path, "same bytes").unwrap();
        std::fs::write(&second_path, "same bytes").unwrap();

        let first = db
            .register_source(collection.id, &first_path, "same", "markdown", 10)
            .unwrap();
        let second = db
            .register_source(collection.id, &second_path, "same", "markdown", 10)
            .unwrap();

        assert!(matches!(first, SourceRegistration::New(_)));
        assert!(matches!(second, SourceRegistration::New(_)));
        assert_ne!(first.id(), second.id());
        assert_eq!(db.count("source_documents").unwrap(), 2);
        assert_eq!(
            db.source_document(first.id()).unwrap().unwrap().source_path,
            first_path
        );
        assert_eq!(
            db.source_document(second.id())
                .unwrap()
                .unwrap()
                .source_path,
            second_path
        );
    }

    #[test]
    fn failed_or_interrupted_registration_retries_without_bumping_revision() {
        let (temp, db, collection) = setup();
        let path = temp.path().join("source/retry.md");
        std::fs::write(&path, "hello").unwrap();
        let first = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();
        db.mark_source_failed(first.id(), "embedding worker stopped")
            .unwrap();

        let retry = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();
        assert!(matches!(retry, SourceRegistration::Changed(id) if id == first.id()));
        let source = db.source_document(first.id()).unwrap().unwrap();
        assert_eq!(source.status, DocumentStatus::Detected);
        assert_eq!(source.revision, 1);
        assert!(source.last_error.is_none());
    }

    #[test]
    fn same_hash_rename_of_failed_work_is_reprocessed_without_bumping_revision() {
        let (temp, db, collection) = setup();
        let old_path = temp.path().join("source/old.md");
        let new_path = temp.path().join("source/new.md");
        std::fs::write(&old_path, "hello").unwrap();
        let first = db
            .register_source(collection.id, &old_path, "aaa", "markdown", 5)
            .unwrap();
        db.mark_source_failed(first.id(), "worker stopped").unwrap();
        std::fs::rename(&old_path, &new_path).unwrap();

        let retry = db
            .register_source(collection.id, &new_path, "aaa", "markdown", 5)
            .unwrap();
        assert!(matches!(retry, SourceRegistration::Changed(id) if id == first.id()));
        let source = db.source_document(first.id()).unwrap().unwrap();
        assert_eq!(source.source_path, new_path);
        assert_eq!(source.status, DocumentStatus::Detected);
        assert_eq!(source.revision, 1);
        assert!(source.last_error.is_none());
    }

    #[test]
    fn startup_recovery_closes_stale_ingest_jobs_and_keeps_source_retryable() {
        let (temp, db, collection) = setup();
        let path = temp.path().join("source/interrupted.md");
        std::fs::write(&path, "hello").unwrap();
        let source = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();
        db.mark_extracted(source.id(), 0, 5).unwrap();
        let job = db.create_job(Some(source.id()), "ingest").unwrap();
        db.set_job_state(job.id, "running", None).unwrap();

        assert_eq!(db.recover_interrupted_jobs().unwrap(), 1);
        let interrupted = db.source_document(source.id()).unwrap().unwrap();
        assert_eq!(interrupted.status, DocumentStatus::Failed);
        assert!(
            interrupted
                .last_error
                .as_deref()
                .unwrap()
                .contains("previous application shutdown")
        );
        let (state, last_error): (String, Option<String>) = db
            .connection()
            .unwrap()
            .query_row(
                "SELECT state,last_error FROM jobs WHERE id=?1",
                [job.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "failed");
        assert!(
            last_error
                .unwrap()
                .contains("previous application shutdown")
        );

        let retry = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();
        assert!(matches!(retry, SourceRegistration::Changed(id) if id == source.id()));
        assert_eq!(
            db.source_document(source.id()).unwrap().unwrap().revision,
            1
        );
    }

    #[test]
    fn startup_recovery_restores_an_interrupted_reanalysis_to_review() {
        let (temp, db, collection) = setup();
        let path = temp.path().join("source/reanalysis.md");
        std::fs::write(&path, "hello").unwrap();
        let source = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();
        db.mark_extracted(source.id(), 0, 5).unwrap();
        let concept = db
            .save_enrichment(source.id(), draft(), "peer-a", "fake")
            .unwrap();
        let claim = db.begin_review_reanalysis(concept.id).unwrap();
        assert_eq!(
            db.concept(concept.id).unwrap().unwrap().status,
            DocumentStatus::Enriched
        );

        assert_eq!(db.recover_interrupted_jobs().unwrap(), 1);
        assert_eq!(
            db.concept(concept.id).unwrap().unwrap().status,
            DocumentStatus::NeedsReview
        );
        let restored_source = db.source_document(source.id()).unwrap().unwrap();
        assert_eq!(restored_source.status, DocumentStatus::NeedsReview);
        let job_state: String = db
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM jobs WHERE id=?1",
                [claim.job_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(job_state, "failed");
    }

    #[test]
    fn startup_recovery_retries_ingest_interrupted_after_draft_before_chunks() {
        let (temp, db, collection) = setup();
        let path = temp.path().join("source/draft-without-chunks.md");
        std::fs::write(&path, "hello").unwrap();
        let source = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();
        db.mark_extracted(source.id(), 0, 5).unwrap();
        let concept = db
            .save_enrichment(source.id(), draft(), "peer-a", "fake")
            .unwrap();
        let job = db.create_job(Some(source.id()), "ingest").unwrap();
        db.set_job_state(job.id, "running", None).unwrap();

        assert_eq!(db.recover_interrupted_jobs().unwrap(), 1);
        assert_eq!(
            db.source_document(source.id()).unwrap().unwrap().status,
            DocumentStatus::Failed
        );
        assert_eq!(
            db.concept(concept.id).unwrap().unwrap().status,
            DocumentStatus::Failed
        );
        let retry = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();
        assert!(matches!(retry, SourceRegistration::Changed(id) if id == source.id()));
        assert_eq!(
            db.source_document(source.id()).unwrap().unwrap().revision,
            1
        );
    }

    #[test]
    fn publication_requires_review_and_chunks() {
        let (temp, db, collection) = setup();
        let path = temp.path().join("source/a.md");
        std::fs::write(&path, "hello").unwrap();
        let source = db
            .register_source(collection.id, &path, "aaa", "markdown", 5)
            .unwrap();
        db.mark_extracted(source.id(), 0, 5).unwrap();
        let concept = db
            .save_enrichment(source.id(), draft(), "peer-a", "fake")
            .unwrap();
        assert!(db.approve_concept(concept.id, draft()).is_err());
        let chunk = StoredChunk {
            id: Uuid::new_v4(),
            concept_id: concept.id,
            source_document_id: source.id(),
            collection_id: collection.id,
            ordinal: 0,
            heading_or_page: "Inicio".into(),
            text: "restaurar pagos".into(),
            text_sha256: "hash".into(),
            embedding: vec![0.0; EMBEDDING_DIMENSIONS],
            source_revision: 1,
        };
        db.replace_chunks(concept.id, &[chunk]).unwrap();
        let published = db.approve_concept(concept.id, draft()).unwrap();
        assert_eq!(published.status, DocumentStatus::Published);
        db.mark_deleted(source.id()).unwrap();
        assert!(db.chunks_for_concept(concept.id).unwrap().is_empty());
        assert_eq!(db.collection_stats(collection.id).unwrap().sources, 0);
    }

    #[test]
    fn concurrent_quarantine_cannot_be_resurrected_by_approval() {
        let (temp, db, collection) = setup();
        let path = temp.path().join("source/concurrent-approval.md");
        std::fs::write(&path, "hello").unwrap();
        let registration = db
            .register_source(collection.id, &path, "a", "markdown", 5)
            .unwrap();
        db.mark_extracted(registration.id(), 0, 5).unwrap();
        let concept = db
            .save_enrichment(registration.id(), draft(), "peer-a", "fake")
            .unwrap();
        db.replace_chunks(
            concept.id,
            &[StoredChunk {
                id: Uuid::new_v4(),
                concept_id: concept.id,
                source_document_id: registration.id(),
                collection_id: collection.id,
                ordinal: 0,
                heading_or_page: "Inicio".into(),
                text: "restaurar pagos".into(),
                text_sha256: "hash".into(),
                embedding: vec![0.0; EMBEDDING_DIMENSIONS],
                source_revision: 1,
            }],
        )
        .unwrap();

        let barrier = Arc::new(std::sync::Barrier::new(3));
        let approve_db = db.clone();
        let approve_barrier = Arc::clone(&barrier);
        let concept_id = concept.id;
        let approval = std::thread::spawn(move || {
            approve_barrier.wait();
            approve_db.approve_concept_if_current(concept_id, draft(), "a", 1)
        });
        let quarantine_db = db.clone();
        let quarantine_barrier = Arc::clone(&barrier);
        let quarantine = std::thread::spawn(move || {
            quarantine_barrier.wait();
            quarantine_db.quarantine_collection(collection.id, "watcher failed")
        });
        barrier.wait();
        let _ = approval.join().unwrap();
        quarantine.join().unwrap().unwrap();

        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::Failed
        );
        assert!(db.chunks_for_concept(concept_id).unwrap().is_empty());
        assert!(!db.publication_is_current(concept_id, "a", 1).unwrap());
        assert!(
            !db.return_to_review_if_current(concept_id, "a", 1, "late rollback")
                .unwrap()
        );
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::Failed
        );
    }

    #[test]
    fn grants_require_trusted_unblocked_peer_and_shareable_collection() {
        let (_temp, db, collection) = setup();
        db.upsert_peer(&PeerRecord {
            peer_id: "peer-b".into(),
            display_name: None,
            trusted: true,
            blocked: false,
            paired_at: Some(Utc::now()),
            last_seen_at: None,
        })
        .unwrap();
        db.set_grant("peer-b", collection.id, true).unwrap();
        assert_eq!(
            db.granted_collections("peer-b").unwrap(),
            vec![collection.id]
        );
        assert!(
            db.granted_collections_for_search("peer-b", SearchPurpose::ExternalAi)
                .unwrap()
                .is_empty()
        );
        assert_eq!(db.list_peers().unwrap().len(), 1);
        assert_eq!(db.list_grants(Some("peer-b")).unwrap().len(), 1);
        assert_eq!(db.collection_stats(collection.id).unwrap().sources, 0);

        db.update_collection_policy(
            collection.id,
            CollectionPolicy {
                local_only: false,
                peer_shareable: false,
                allow_external_ai: true,
                internet_public: false,
            },
        )
        .unwrap();
        assert!(db.granted_collections("peer-b").unwrap().is_empty());

        db.update_collection_policy(
            collection.id,
            CollectionPolicy {
                local_only: false,
                peer_shareable: true,
                allow_external_ai: true,
                internet_public: false,
            },
        )
        .unwrap();
        assert_eq!(
            db.granted_collections("peer-b").unwrap(),
            vec![collection.id]
        );
        assert_eq!(
            db.granted_collections_for_search("peer-b", SearchPurpose::ExternalAi)
                .unwrap(),
            vec![collection.id]
        );

        db.update_collection_policy(collection.id, CollectionPolicy::local_only())
            .unwrap();
        assert!(db.granted_collections("peer-b").unwrap().is_empty());

        db.revoke_peer("peer-b").unwrap();
        assert!(db.granted_collections("peer-b").unwrap().is_empty());
        assert!(db.list_grants(None).unwrap().is_empty());
    }

    #[test]
    fn embedding_round_trip_is_exact() {
        let values = vec![0.0, 1.0, -3.25, f32::MAX];
        assert_eq!(
            decode_embedding(&encode_embedding(&values)).unwrap(),
            values
        );
    }
}
