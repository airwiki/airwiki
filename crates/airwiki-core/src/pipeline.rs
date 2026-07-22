use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use airwiki_types::{ConceptType, DocumentStatus, EnrichmentDraft};
use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::chunk_identity::stored_chunk_id;
use crate::inference::{EmbeddingProvider, GenerationProvider, MAX_GENERATION_INPUT_TOKENS};
use crate::ingest::{
    ChunkDraft, Chunker, FileCandidate, FileDiscovery, IngestLimits, SourceFormat, SourceIssueCode,
    Tokenizer, WhitespaceTokenizer, discover_files_with_issues, extract_file, sha256_file,
};
use crate::publication::OkfPublicationMaterializer;
use crate::storage::{
    AuditEvent, CollectionMaintenanceCounts, CollectionMaintenanceResult,
    CollectionMaintenanceStatus, CollectionRecord, ConceptRecord, Database, ReviewVersionToken,
    SourceDocumentRecord, SourceRegistration, StoredChunk,
};

const EMBEDDING_BATCH_SIZE: usize = 32;

#[derive(Debug)]
struct RegisteredCandidate {
    candidate: FileCandidate,
    registration: SourceRegistration,
    source: SourceDocumentRecord,
}

#[derive(Debug)]
struct PreparedDocument {
    generation_text: String,
    chunks: Vec<ChunkDraft>,
    page_count: u32,
    character_count: u64,
}

#[derive(Debug, thiserror::Error)]
#[error("source registration was superseded by a newer filesystem scan")]
struct SupersededProcessing;

/// Result of the synchronous, inference-free collection reconciliation phase.
/// Callers may discard it after using preflight only for immediate revocation;
/// the next full scan will safely reclaim the detected revisions without
/// incrementing their revision numbers.
#[derive(Debug)]
pub struct CollectionPreflight {
    outcomes: Vec<IngestOutcome>,
    pending: Vec<RegisteredCandidate>,
    quarantined: bool,
}

impl CollectionPreflight {
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn outcomes(&self) -> &[IngestOutcome] {
        &self.outcomes
    }
}

#[derive(Debug, Clone)]
pub struct ReviewEdits {
    pub draft: EnrichmentDraft,
}

#[derive(Debug, Clone)]
pub enum IngestOutcome {
    Unchanged {
        source_document_id: Uuid,
    },
    Renamed {
        source_document_id: Uuid,
    },
    NeedsReview {
        source_document_id: Uuid,
        concept_id: Uuid,
        used_fallback_metadata: bool,
    },
    Deleted {
        source_document_id: Uuid,
    },
    Failed {
        source_document_id: Option<Uuid>,
        path: PathBuf,
        code: SourceIssueCode,
        error: String,
    },
}

fn maintenance_result(
    outcomes: &[IngestOutcome],
    quarantined: bool,
) -> Result<CollectionMaintenanceResult> {
    let mut counts = CollectionMaintenanceCounts::default();
    for outcome in outcomes {
        match outcome {
            IngestOutcome::Unchanged { .. } => counts.unchanged += 1,
            IngestOutcome::Renamed { .. } => counts.renamed += 1,
            IngestOutcome::NeedsReview { .. } => counts.analyzed += 1,
            IngestOutcome::Deleted { .. } => counts.deleted += 1,
            IngestOutcome::Failed { .. } => counts.failed += 1,
        }
    }

    if quarantined {
        CollectionMaintenanceResult::issue(
            CollectionMaintenanceStatus::Quarantined,
            counts,
            "collection_quarantined",
            "The collection could not be monitored or reconciled safely.",
        )
    } else if counts.failed > 0 {
        CollectionMaintenanceResult::issue(
            CollectionMaintenanceStatus::Partial,
            counts,
            "collection_scan_partial",
            "One or more files could not be processed.",
        )
    } else {
        Ok(CollectionMaintenanceResult::success(counts))
    }
}

#[derive(Clone)]
pub struct IngestPipeline {
    database: Database,
    generation: Arc<dyn GenerationProvider>,
    embeddings: Arc<dyn EmbeddingProvider>,
    tokenizer: Arc<dyn Tokenizer>,
    chunker: Chunker,
    limits: IngestLimits,
    node_id: String,
}

impl std::fmt::Debug for IngestPipeline {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IngestPipeline")
            .field("generation_model", &self.generation.model_id())
            .field("embedding_model", &self.embeddings.model_id())
            .field("chunker", &self.chunker)
            .field("limits", &self.limits)
            .field("node_id", &self.node_id)
            .finish()
    }
}

impl IngestPipeline {
    pub fn new(
        database: Database,
        generation: Arc<dyn GenerationProvider>,
        embeddings: Arc<dyn EmbeddingProvider>,
        node_id: impl Into<String>,
    ) -> Self {
        Self {
            database,
            generation,
            embeddings,
            tokenizer: Arc::new(WhitespaceTokenizer),
            chunker: Chunker::default(),
            limits: IngestLimits::default(),
            node_id: node_id.into(),
        }
    }

    pub fn with_tokenizer(mut self, tokenizer: Arc<dyn Tokenizer>) -> Self {
        self.tokenizer = tokenizer;
        self
    }

    pub fn with_chunker(mut self, chunker: Chunker) -> Self {
        self.chunker = chunker;
        self
    }

    pub fn with_limits(mut self, limits: IngestLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn database(&self) -> &Database {
        &self.database
    }

    pub async fn scan_collection(&self, collection_id: Uuid) -> Result<Vec<IngestOutcome>> {
        self.database.start_collection_maintenance(collection_id)?;
        let pipeline = self.clone();
        let scan = async move {
            let preflight = run_blocking(
                "collection preflight worker stopped unexpectedly",
                move || pipeline.preflight_collection(collection_id),
            )
            .await?;
            let quarantined = preflight.quarantined;
            self.process_preflight(preflight)
                .await
                .map(|outcomes| (outcomes, quarantined))
        }
        .await;

        match scan {
            Ok((outcomes, quarantined)) => {
                let maintenance = maintenance_result(&outcomes, quarantined)?;
                self.database
                    .finish_collection_maintenance(collection_id, &maintenance)?;
                Ok(outcomes)
            }
            Err(scan_error) => {
                let maintenance = CollectionMaintenanceResult::issue(
                    CollectionMaintenanceStatus::Failed,
                    CollectionMaintenanceCounts::default(),
                    "collection_scan_failed",
                    "The collection could not be reconciled.",
                )?;
                self.database
                    .finish_collection_maintenance(collection_id, &maintenance)
                    .context("the scan failed and its maintenance state could not be recorded")?;
                Err(scan_error)
            }
        }
    }

    /// Hashes and registers every candidate, withdraws changed publications and
    /// tombstones missing sources without invoking parsing, generation or
    /// embeddings. Watchers can call this immediately while an older async scan
    /// is still in inference; guarded state transitions make that older worker
    /// observe that it has been superseded.
    pub fn preflight_collection(&self, collection_id: Uuid) -> Result<CollectionPreflight> {
        let collection = self
            .database
            .collection(collection_id)?
            .ok_or_else(|| anyhow!("collection {collection_id} does not exist"))?;
        let discovery = discover_files_with_issues(&collection.source_folder, self.limits)?;
        self.preflight_discovery(collection, discovery)
    }

    /// Runs the startup-only filesystem discovery behind a deadline before any
    /// SQLite or OKF mutation is allowed. A blocking filesystem syscall cannot
    /// be cancelled by Tokio, so the timed operation deliberately owns only a
    /// path and limits; a late result is detached and discarded.
    pub async fn preflight_collection_with_discovery_timeout(
        &self,
        collection_id: Uuid,
        deadline: Duration,
    ) -> Result<CollectionPreflight> {
        let collection = self
            .database
            .collection(collection_id)?
            .ok_or_else(|| anyhow!("collection {collection_id} does not exist"))?;
        let source_folder = collection.source_folder.clone();
        let limits = self.limits;
        let discovery = await_startup_discovery(
            tokio::task::spawn_blocking(move || discover_files_with_issues(source_folder, limits)),
            deadline,
        )
        .await?;
        let pipeline = self.clone();
        run_blocking(
            "collection startup preflight worker stopped unexpectedly",
            move || pipeline.preflight_discovery(collection, discovery),
        )
        .await
    }

    fn preflight_discovery(
        &self,
        collection: CollectionRecord,
        discovery: FileDiscovery,
    ) -> Result<CollectionPreflight> {
        let collection_id = collection.id;
        let is_complete = discovery.is_complete();
        let unavailable_paths = discovery
            .issues
            .iter()
            .map(|issue| (normalized_path(&issue.path), issue.error.clone()))
            .collect::<HashMap<_, _>>();
        let candidates = discovery.candidates;
        let present = candidates
            .iter()
            .map(|candidate| normalized_path(&candidate.path))
            .chain(unavailable_paths.keys().cloned())
            .collect::<HashSet<_>>();
        let mut outcomes = discovery
            .issues
            .into_iter()
            .map(|issue| IngestOutcome::Failed {
                source_document_id: None,
                path: issue.path,
                code: issue.code,
                error: issue.error,
            })
            .collect::<Vec<_>>();

        if !is_complete {
            self.quarantine_collection(
                collection_id,
                "source traversal incomplete; retry after filesystem access is restored",
            )?;
            return Ok(CollectionPreflight {
                outcomes,
                pending: Vec::new(),
                quarantined: true,
            });
        }

        let mut pending = Vec::new();
        for candidate in candidates {
            match self.register_candidate(collection_id, candidate.clone()) {
                Ok(registered) => match &registered.registration {
                    SourceRegistration::Unchanged(source_document_id) => {
                        outcomes.push(IngestOutcome::Unchanged {
                            source_document_id: *source_document_id,
                        });
                    }
                    SourceRegistration::Renamed(source_document_id) => {
                        outcomes.push(IngestOutcome::Renamed {
                            source_document_id: *source_document_id,
                        });
                    }
                    SourceRegistration::New(_)
                    | SourceRegistration::Changed(_)
                    | SourceRegistration::Replaced { .. } => {
                        pending.push(registered);
                    }
                },
                Err(error) => {
                    let error = error.to_string();
                    outcomes.push(IngestOutcome::Failed {
                        source_document_id: None,
                        path: candidate.path,
                        code: SourceIssueCode::from_error(&error),
                        error,
                    });
                }
            }
        }

        for source in self.database.list_sources(collection_id)? {
            if let Some(reason) = unavailable_paths.get(&normalized_path(&source.source_path)) {
                let already_unavailable = source.status == DocumentStatus::Failed
                    && source.last_error.as_deref() == Some(reason.as_str());
                if source.status != DocumentStatus::Deleted && !already_unavailable {
                    let artifact = self.database.quarantine_source(source.id, reason)?;
                    if let Some((concept_id, source_sha256)) = artifact {
                        OkfPublicationMaterializer::new(self.database.clone())
                            .withdraw_published_artifact(
                                collection_id,
                                concept_id,
                                &source_sha256,
                            )?;
                    }
                    self.audit(
                        "system",
                        "source_unavailable",
                        "source_document",
                        Some(source.id.to_string()),
                        serde_json::json!({
                            "collection_id": collection_id,
                            "reason": "source_discovery_issue"
                        }),
                    )?;
                }
                continue;
            }
            if source.status == DocumentStatus::Deleted {
                if let Some(concept_id) = source.concept_id {
                    OkfPublicationMaterializer::new(self.database.clone())
                        .withdraw_published_artifact(
                            collection_id,
                            concept_id,
                            &source.source_sha256,
                        )?;
                }
                continue;
            }
            if !present.contains(&normalized_path(&source.source_path)) {
                let concept_id = source.concept_id;
                self.database.mark_deleted(source.id)?;
                if let Some(concept_id) = concept_id {
                    OkfPublicationMaterializer::new(self.database.clone())
                        .withdraw_published_artifact(
                            collection_id,
                            concept_id,
                            &source.source_sha256,
                        )?;
                }
                self.audit(
                    "system",
                    "source_deleted",
                    "source_document",
                    Some(source.id.to_string()),
                    serde_json::json!({"collection_id": collection_id}),
                )?;
                outcomes.push(IngestOutcome::Deleted {
                    source_document_id: source.id,
                });
            }
        }

        Ok(CollectionPreflight {
            outcomes,
            pending,
            quarantined: false,
        })
    }

    /// Fails a collection closed when its root or watcher becomes unavailable.
    /// SQLite/FTS is withdrawn atomically first, then published OKF artifacts
    /// are removed. Recovery requires a successful scan and human review.
    pub fn quarantine_collection(
        &self,
        collection_id: Uuid,
        reason: impl AsRef<str>,
    ) -> Result<()> {
        self.database
            .collection(collection_id)?
            .ok_or_else(|| anyhow!("collection {collection_id} does not exist"))?;
        let artifacts = self
            .database
            .quarantine_collection(collection_id, reason.as_ref())?;
        OkfPublicationMaterializer::new(self.database.clone())
            .withdraw_published_artifacts(collection_id, &artifacts)?;
        self.audit(
            "system",
            "collection_quarantined",
            "collection",
            Some(collection_id.to_string()),
            serde_json::json!({"reason": reason.as_ref()}),
        )?;
        let maintenance = CollectionMaintenanceResult::issue(
            CollectionMaintenanceStatus::Quarantined,
            CollectionMaintenanceCounts::default(),
            "collection_quarantined",
            "The collection could not be monitored or reconciled safely.",
        )?;
        self.database
            .finish_collection_maintenance(collection_id, &maintenance)
    }

    async fn process_preflight(
        &self,
        mut preflight: CollectionPreflight,
    ) -> Result<Vec<IngestOutcome>> {
        for registered in preflight.pending {
            preflight
                .outcomes
                .push(self.process_candidate(registered).await?);
        }
        Ok(preflight.outcomes)
    }

    pub async fn ingest_path(
        &self,
        collection_id: Uuid,
        path: impl AsRef<Path>,
    ) -> Result<IngestOutcome> {
        let path = path.as_ref().to_path_buf();
        let pipeline = self.clone();
        let registered = run_blocking(
            "source registration worker stopped unexpectedly",
            move || {
                let format =
                    SourceFormat::from_path(&path).context("unsupported source file extension")?;
                let metadata = std::fs::symlink_metadata(&path)?;
                if metadata.file_type().is_symlink() {
                    bail!("symbolic links are not accepted");
                }
                if metadata.len() > pipeline.limits.max_bytes {
                    bail!("source exceeds the configured size limit");
                }
                pipeline.register_candidate(
                    collection_id,
                    FileCandidate {
                        path,
                        format,
                        byte_size: metadata.len(),
                    },
                )
            },
        )
        .await?;
        self.process_candidate(registered).await
    }

    fn register_candidate(
        &self,
        collection_id: Uuid,
        candidate: FileCandidate,
    ) -> Result<RegisteredCandidate> {
        let sha256 = sha256_file(&candidate.path)?;
        let registration = self.database.register_source(
            collection_id,
            &candidate.path,
            &sha256,
            candidate.format.as_str(),
            candidate.byte_size,
        )?;
        let source = self
            .database
            .source_document(registration.id())?
            .context("registered source document disappeared")?;
        if source.source_sha256 != sha256 {
            return Err(SupersededProcessing.into());
        }
        if registration.needs_processing()
            && let Some(concept_id) = source.concept_id
        {
            // SQLite registration already removed the searchable revision. Keep
            // the generated OKF bundle equally fail closed during preflight.
            OkfPublicationMaterializer::new(self.database.clone()).withdraw_published_artifact(
                collection_id,
                concept_id,
                registration
                    .previous_source_sha256()
                    .unwrap_or(&source.source_sha256),
            )?;
        }

        Ok(RegisteredCandidate {
            candidate,
            registration,
            source,
        })
    }

    async fn process_candidate(&self, registered: RegisteredCandidate) -> Result<IngestOutcome> {
        let source_document_id = registered.registration.id();
        match registered.registration {
            SourceRegistration::Unchanged(_) => {
                return Ok(IngestOutcome::Unchanged { source_document_id });
            }
            SourceRegistration::Renamed(_) => {
                return Ok(IngestOutcome::Renamed { source_document_id });
            }
            SourceRegistration::New(_)
            | SourceRegistration::Changed(_)
            | SourceRegistration::Replaced { .. } => {}
        }

        let job = self
            .database
            .create_job(Some(source_document_id), "ingest")?;
        self.database.set_job_state(job.id, "running", None)?;
        let result = self
            .process_registered_source(&registered.source, &registered.candidate)
            .await;
        match result {
            Ok(outcome) => {
                self.database.set_job_state(job.id, "completed", None)?;
                Ok(outcome)
            }
            Err(error) => {
                let message = error.to_string();
                let code = if error.downcast_ref::<SupersededProcessing>().is_some() {
                    SourceIssueCode::Superseded
                } else {
                    SourceIssueCode::from_error(&message)
                };
                if error.downcast_ref::<SupersededProcessing>().is_none() {
                    self.database.mark_source_failed_if_current(
                        source_document_id,
                        &registered.source.source_sha256,
                        registered.source.revision,
                        &message,
                    )?;
                }
                self.database
                    .set_job_state(job.id, "failed", Some(&message))?;
                Ok(IngestOutcome::Failed {
                    source_document_id: Some(source_document_id),
                    path: registered.candidate.path,
                    code,
                    error: message,
                })
            }
        }
    }

    async fn process_registered_source(
        &self,
        registered: &SourceDocumentRecord,
        candidate: &FileCandidate,
    ) -> Result<IngestOutcome> {
        let source_document_id = registered.id;
        let path = candidate.path.clone();
        let format = candidate.format;
        let expected_hash = registered.source_sha256.clone();
        let limits = self.limits;
        let chunker = self.chunker;
        let tokenizer = Arc::clone(&self.tokenizer);
        let prepared = run_blocking(
            "document extraction worker stopped unexpectedly",
            move || {
                prepare_document(
                    &path,
                    format,
                    limits,
                    &expected_hash,
                    chunker,
                    tokenizer.as_ref(),
                )
            },
        )
        .await?;
        if !self.database.mark_extracted_if_current(
            source_document_id,
            &registered.source_sha256,
            registered.revision,
            prepared.page_count,
            prepared.character_count,
        )? {
            return Err(SupersededProcessing.into());
        }

        let text = prepared.generation_text;
        let (draft, used_fallback_metadata) = match self.generation.enrich(&text).await {
            Ok(draft) => (draft, false),
            Err(first_error) => match self.generation.enrich(&text).await {
                Ok(draft) => (draft, false),
                Err(second_error) => (
                    fallback_draft(&candidate.path, &text, &first_error, &second_error),
                    true,
                ),
            },
        };
        if !self.database.mark_enriched_if_current(
            source_document_id,
            &registered.source_sha256,
            registered.revision,
        )? {
            return Err(SupersededProcessing.into());
        }
        let Some(concept) = self.database.save_enrichment_if_current(
            source_document_id,
            &registered.source_sha256,
            registered.revision,
            draft,
            &self.node_id,
            self.generation.model_id(),
        )?
        else {
            return Err(SupersededProcessing.into());
        };

        let embeddings = self.embed_chunk_drafts(&prepared.chunks).await?;
        let chunks = prepared.chunks;
        let database = self.database.clone();
        let source_sha256 = registered.source_sha256.clone();
        let source_revision = registered.revision;
        let concept_id = concept.id;
        let collection_id = concept.collection_id;
        let replaced = run_blocking("search index writer stopped unexpectedly", move || {
            let stored = stored_chunks_from_embeddings(
                chunks,
                embeddings,
                concept_id,
                source_document_id,
                collection_id,
                &source_sha256,
                source_revision,
            )?;
            database.replace_chunks_if_current(concept_id, &source_sha256, source_revision, &stored)
        })
        .await?;
        if !replaced {
            return Err(SupersededProcessing.into());
        }
        self.audit(
            "system",
            "source_ingested",
            "source_document",
            Some(source_document_id.to_string()),
            serde_json::json!({
                "collection_id": concept.collection_id,
                "source_sha256": registered.source_sha256,
                "revision": registered.revision,
                "fallback_metadata": used_fallback_metadata,
            }),
        )?;
        Ok(IngestOutcome::NeedsReview {
            source_document_id,
            concept_id: concept.id,
            used_fallback_metadata,
        })
    }

    /// Runs local enrichment again for an existing pending review. The prior
    /// draft and chunks remain intact until the new generation and embeddings
    /// have both succeeded, and the result always returns to `NeedsReview`.
    pub async fn reanalyze_review(&self, concept_id: Uuid) -> Result<ConceptRecord> {
        let claim = self.database.begin_review_reanalysis(concept_id)?;
        let result = self.reanalyze_claimed_review(&claim).await;
        match result {
            Ok(concept) => {
                self.audit(
                    "human",
                    "source_reanalyzed",
                    "concept",
                    Some(concept_id.to_string()),
                    serde_json::json!({
                        "collection_id": claim.collection_id,
                        "source_revision": claim.revision,
                        "source_sha256": claim.source_sha256,
                        "generator_model": self.generation.model_id(),
                    }),
                )?;
                Ok(concept)
            }
            Err(error) => {
                self.database
                    .fail_review_reanalysis(&claim, error.to_string())?;
                Err(error)
            }
        }
    }

    async fn reanalyze_claimed_review(
        &self,
        claim: &crate::storage::ReviewReanalysisClaim,
    ) -> Result<ConceptRecord> {
        let claimed_source = claim.clone();
        let limits = self.limits;
        let chunker = self.chunker;
        let tokenizer = Arc::clone(&self.tokenizer);
        let prepared = run_blocking("review extraction worker stopped unexpectedly", move || {
            prepare_review_document(&claimed_source, limits, chunker, tokenizer.as_ref())
        })
        .await?;
        let text = prepared.generation_text;
        let draft = match self.generation.enrich(&text).await {
            Ok(draft) => draft,
            Err(first_error) => match self.generation.enrich(&text).await {
                Ok(draft) => draft,
                Err(second_error) => {
                    bail!("automatic enrichment failed twice: {first_error:#}; {second_error:#}")
                }
            },
        };
        let embeddings = self.embed_chunk_drafts(&prepared.chunks).await?;
        let chunks = prepared.chunks;
        // Inference can take minutes on a small Windows machine. Re-hash at the
        // final boundary and let the atomic database predicate guard the same
        // revision against watcher/preflight races.
        let claimed_source = claim.clone();
        let database = self.database.clone();
        let generator_model = self.generation.model_id().to_owned();
        run_blocking("review index writer stopped unexpectedly", move || {
            if sha256_file(&claimed_source.source_path)? != claimed_source.source_sha256 {
                bail!("source changed during reanalysis; rescan it before reviewing");
            }
            let stored = stored_chunks_from_embeddings(
                chunks,
                embeddings,
                claimed_source.concept_id,
                claimed_source.source_document_id,
                claimed_source.collection_id,
                &claimed_source.source_sha256,
                claimed_source.revision,
            )?;
            if !database.complete_review_reanalysis_if_current(
                &claimed_source,
                draft,
                &generator_model,
                &stored,
            )? {
                bail!("source revision was superseded while it was being reanalyzed");
            }
            database
                .concept(claimed_source.concept_id)?
                .context("reanalyzed concept disappeared")
        })
        .await
    }

    async fn embed_chunk_drafts(&self, chunks: &[ChunkDraft]) -> Result<Vec<Vec<f32>>> {
        let texts = chunks
            .iter()
            .map(|chunk| format!("passage: {}", chunk.text))
            .collect::<Vec<_>>();
        let mut embeddings = Vec::with_capacity(texts.len());
        for batch in texts.chunks(EMBEDDING_BATCH_SIZE) {
            let values = self.embeddings.embed(batch).await?;
            if values.len() != batch.len() {
                bail!("embedding provider returned the wrong batch size");
            }
            embeddings.extend(values);
        }
        Ok(embeddings)
    }

    /// Human approval is the only API that changes a concept to `Published`.
    pub fn approve(
        &self,
        concept_id: Uuid,
        edits: ReviewEdits,
        review_version: &ReviewVersionToken,
    ) -> Result<ConceptRecord> {
        OkfPublicationMaterializer::new(self.database.clone()).approve(
            concept_id,
            edits.draft,
            review_version,
        )
    }

    fn audit(
        &self,
        actor: &str,
        action: &str,
        target_type: &str,
        target_id: Option<String>,
        details: serde_json::Value,
    ) -> Result<()> {
        self.database.record_audit(&AuditEvent {
            id: Uuid::new_v4(),
            actor: actor.into(),
            action: action.into(),
            target_type: target_type.into(),
            target_id,
            details,
            created_at: Utc::now(),
        })
    }
}

async fn run_blocking<T>(
    failure_context: &'static str,
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .context(failure_context)?
}

async fn await_startup_discovery(
    mut task: tokio::task::JoinHandle<Result<FileDiscovery>>,
    deadline: Duration,
) -> Result<FileDiscovery> {
    match tokio::time::timeout(deadline, &mut task).await {
        Ok(result) => result.context("collection startup discovery worker stopped unexpectedly")?,
        Err(_) => {
            // `spawn_blocking` may already be inside an uninterruptible syscall.
            // Abort is best-effort; dropping its eventual read-only result is
            // the safety property that matters here.
            task.abort();
            bail!("collection source discovery exceeded the startup deadline")
        }
    }
}

fn prepare_document(
    path: &Path,
    format: SourceFormat,
    limits: IngestLimits,
    expected_sha256: &str,
    chunker: Chunker,
    tokenizer: &dyn Tokenizer,
) -> Result<PreparedDocument> {
    let extracted = extract_file(path, format, limits)?;
    if sha256_file(path)? != expected_sha256 {
        bail!("source changed while it was being extracted; retrying requires a new scan");
    }
    let chunks = chunker.chunk(&extracted, tokenizer)?;
    if chunks.is_empty() {
        bail!("document produced no searchable chunks");
    }
    Ok(PreparedDocument {
        generation_text: extracted.representative_text(MAX_GENERATION_INPUT_TOKENS),
        chunks,
        page_count: u32::try_from(extracted.page_count).context("page count overflow")?,
        character_count: u64::try_from(extracted.character_count)
            .context("character count overflow")?,
    })
}

fn prepare_review_document(
    claim: &crate::storage::ReviewReanalysisClaim,
    limits: IngestLimits,
    chunker: Chunker,
    tokenizer: &dyn Tokenizer,
) -> Result<PreparedDocument> {
    let metadata = std::fs::symlink_metadata(&claim.source_path)
        .with_context(|| "could not inspect the source before reanalysis")?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("the source is no longer a regular file");
    }
    if metadata.len() > limits.max_bytes {
        bail!("source exceeds the configured size limit");
    }
    if metadata.len() != claim.byte_size {
        bail!("source size changed; rescan it before reanalysis");
    }
    let format = SourceFormat::from_path(&claim.source_path)
        .context("source format is no longer supported")?;
    if format.as_str() != claim.source_format {
        bail!("source format changed; rescan it before reanalysis");
    }
    if sha256_file(&claim.source_path)? != claim.source_sha256 {
        bail!("source changed; rescan it before reanalysis");
    }
    prepare_document(
        &claim.source_path,
        format,
        limits,
        &claim.source_sha256,
        chunker,
        tokenizer,
    )
}

fn stored_chunks_from_embeddings(
    chunks: Vec<ChunkDraft>,
    embeddings: Vec<Vec<f32>>,
    concept_id: Uuid,
    source_document_id: Uuid,
    collection_id: Uuid,
    source_sha256: &str,
    source_revision: u32,
) -> Result<Vec<StoredChunk>> {
    if chunks.len() != embeddings.len() {
        bail!("embedding provider returned the wrong total batch size");
    }
    Ok(chunks
        .into_iter()
        .zip(embeddings)
        .map(|(chunk, embedding)| {
            let text_sha256 = hex::encode(Sha256::digest(chunk.text.as_bytes()));
            StoredChunk {
                id: stored_chunk_id(concept_id, source_sha256, chunk.ordinal, &text_sha256),
                concept_id,
                source_document_id,
                collection_id,
                ordinal: chunk.ordinal,
                heading_or_page: chunk.heading_or_page,
                text: chunk.text,
                text_sha256,
                embedding,
                source_revision,
            }
        })
        .collect())
}

fn fallback_draft(
    path: &Path,
    text: &str,
    first_error: &anyhow::Error,
    second_error: &anyhow::Error,
) -> EnrichmentDraft {
    let title = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Documento")
        .to_owned();
    EnrichmentDraft {
        concept_type: ConceptType::Document,
        title,
        description: "El enriquecimiento automático falló; complete los metadatos manualmente."
            .into(),
        language: "und".into(),
        tags: Vec::new(),
        entities: Vec::new(),
        links: Vec::new(),
        summary: text.chars().take(1_000).collect(),
        classification_confidence: 0.0,
        classification_explanation: format!(
            "Dos intentos fallaron: {first_error:#}; {second_error:#}"
        )
        .chars()
        .take(1_000)
        .collect(),
    }
}

fn normalized_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use airwiki_types::{CollectionPolicy, SearchPurpose, SearchRequest};
    use async_trait::async_trait;
    use tokio::sync::Notify;

    use super::*;
    use crate::inference::{
        DeterministicEmbeddingProvider, DeterministicGenerationProvider, GenerationProvider,
    };
    use crate::search::HybridSearchEngine;

    fn setup(
        generator: Arc<dyn GenerationProvider>,
    ) -> (tempfile::TempDir, Database, Uuid, IngestPipeline) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let wiki = temp.path().join("wiki");
        std::fs::create_dir_all(&source).unwrap();
        let db = Database::in_memory().unwrap();
        let collection = db
            .create_collection(
                "Piloto",
                &source,
                &wiki,
                CollectionPolicy {
                    local_only: false,
                    peer_shareable: true,
                    allow_external_ai: true,
                    internet_public: false,
                },
            )
            .unwrap();
        let pipeline = IngestPipeline::new(
            db.clone(),
            generator,
            Arc::new(DeterministicEmbeddingProvider),
            "mac",
        );
        (temp, db, collection.id, pipeline)
    }

    fn review_version(database: &Database, concept_id: Uuid) -> Result<ReviewVersionToken> {
        let concept = database
            .concept(concept_id)?
            .context("test concept is missing")?;
        let source = database
            .source_document(concept.source_document_id)?
            .context("test source is missing")?;
        database
            .review_evidence_page(concept_id, source.revision, None, None, 1)?
            .context("test review is no longer current")
            .map(|page| page.review_version)
    }

    fn approve_current(
        pipeline: &IngestPipeline,
        database: &Database,
        concept_id: Uuid,
        draft: EnrichmentDraft,
    ) -> Result<ConceptRecord> {
        let review_version = review_version(database, concept_id)?;
        pipeline.approve(concept_id, ReviewEdits { draft }, &review_version)
    }

    struct BlockNextGeneration {
        block_next: AtomicBool,
        calls: AtomicUsize,
        entered: Notify,
        release: Notify,
    }

    impl BlockNextGeneration {
        fn new() -> Self {
            Self {
                block_next: AtomicBool::new(false),
                calls: AtomicUsize::new(0),
                entered: Notify::new(),
                release: Notify::new(),
            }
        }

        fn arm(&self) {
            self.block_next.store(true, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl GenerationProvider for BlockNextGeneration {
        fn model_id(&self) -> &str {
            "blocking-test"
        }

        async fn enrich(&self, document_text: &str) -> Result<EnrichmentDraft> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.block_next.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            DeterministicGenerationProvider.enrich(document_text).await
        }
    }

    struct CapturesGenerationInput {
        input: Mutex<Option<String>>,
    }

    #[async_trait]
    impl GenerationProvider for CapturesGenerationInput {
        fn model_id(&self) -> &str {
            "capturing-test"
        }

        async fn enrich(&self, document_text: &str) -> Result<EnrichmentDraft> {
            *self.input.lock().unwrap() = Some(document_text.to_owned());
            DeterministicGenerationProvider.enrich(document_text).await
        }
    }

    struct RecordsTokenizerThread {
        thread: Mutex<Option<std::thread::ThreadId>>,
    }

    impl Tokenizer for RecordsTokenizerThread {
        fn encode(&self, text: &str) -> Result<Vec<String>> {
            *self.thread.lock().unwrap() = Some(std::thread::current().id());
            WhitespaceTokenizer.encode(text)
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn extraction_and_tokenization_leave_the_async_runtime_thread() {
        let tokenizer = Arc::new(RecordsTokenizerThread {
            thread: Mutex::new(None),
        });
        let (temp, _db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let pipeline = pipeline.with_tokenizer(tokenizer.clone());
        std::fs::write(
            temp.path().join("source/off-thread.md"),
            "# Worker\nSynthetic content for the blocking worker.",
        )
        .unwrap();
        let runtime_thread = std::thread::current().id();

        pipeline.scan_collection(collection_id).await.unwrap();

        let tokenizer_thread = tokenizer.thread.lock().unwrap().unwrap();
        assert_ne!(tokenizer_thread, runtime_thread);
    }

    #[tokio::test]
    async fn long_document_uses_one_bounded_sample_but_indexes_complete_text() {
        let generator = Arc::new(CapturesGenerationInput {
            input: Mutex::new(None),
        });
        let (temp, db, collection_id, pipeline) = setup(generator.clone());
        let path = temp.path().join("source/large.md");
        let text = (1..=100)
            .map(|section| {
                format!(
                    "# Sección {section}\nMARCADOR-{section} á {}",
                    "contenido ".repeat(8)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        std::fs::write(&path, text).unwrap();

        let outcome = pipeline.ingest_path(collection_id, &path).await.unwrap();
        let concept_id = match outcome {
            IngestOutcome::NeedsReview { concept_id, .. } => concept_id,
            other => panic!("unexpected outcome: {other:?}"),
        };
        let input = generator.input.lock().unwrap().clone().unwrap();
        assert!(input.len() <= MAX_GENERATION_INPUT_TOKENS);
        assert!(input.contains("MARCADOR-1"));
        assert!(input.contains("MARCADOR-100"));
        let chunks = db.chunks_for_concept(concept_id).unwrap();
        assert!(chunks.iter().any(|chunk| chunk.text.contains("MARCADOR-1")));
        assert!(
            chunks
                .iter()
                .any(|chunk| chunk.text.contains("MARCADOR-100"))
        );
    }

    #[tokio::test]
    async fn collection_scan_surfaces_an_oversized_supported_file() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/oversized.md");
        std::fs::write(&path, "12345678901").unwrap();
        let outcomes = pipeline
            .with_limits(IngestLimits {
                max_bytes: 10,
                max_pdf_pages: 500,
                max_characters: 2_000_000,
            })
            .scan_collection(collection_id)
            .await
            .unwrap();
        assert!(matches!(
            outcomes.as_slice(),
            [IngestOutcome::Failed {
                source_document_id: None,
                path: failed_path,
                code: SourceIssueCode::FileTooLarge,
                error,
            }] if failed_path == &path && error.contains("10 byte limit")
        ));
        assert_eq!(db.count("source_documents").unwrap(), 0);
        let maintenance = db.collection_maintenance(collection_id).unwrap().unwrap();
        assert_eq!(maintenance.status, CollectionMaintenanceStatus::Partial);
        assert_eq!(maintenance.counts.failed, 1);
        assert_eq!(
            maintenance.issue_code.as_deref(),
            Some("collection_scan_partial")
        );
        assert!(
            !maintenance
                .issue_summary
                .unwrap()
                .contains(path.to_str().unwrap())
        );
    }

    #[tokio::test]
    async fn successful_collection_scan_records_aggregate_counts() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        std::fs::write(
            temp.path().join("source/maintenance.md"),
            "# Maintenance\nSynthetic content.",
        )
        .unwrap();

        pipeline.scan_collection(collection_id).await.unwrap();
        let maintenance = db.collection_maintenance(collection_id).unwrap().unwrap();

        assert_eq!(maintenance.status, CollectionMaintenanceStatus::Success);
        assert_eq!(maintenance.counts.analyzed, 1);
        assert!(maintenance.last_success_at.is_some());
        assert!(maintenance.issue_code.is_none());
    }

    #[tokio::test]
    async fn failed_collection_scan_records_sanitized_terminal_state() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let source = temp.path().join("source");
        std::fs::remove_dir(source).unwrap();

        let error = pipeline.scan_collection(collection_id).await.unwrap_err();
        let maintenance = db.collection_maintenance(collection_id).unwrap().unwrap();

        assert!(!error.to_string().is_empty());
        assert_eq!(maintenance.status, CollectionMaintenanceStatus::Failed);
        assert_eq!(
            maintenance.issue_code.as_deref(),
            Some("collection_scan_failed")
        );
        assert_eq!(
            maintenance.issue_summary.as_deref(),
            Some("The collection could not be reconciled.")
        );
    }

    #[tokio::test]
    async fn preflight_is_inference_free_and_a_later_scan_reclaims_the_revision() {
        let generator = Arc::new(BlockNextGeneration::new());
        let (temp, db, collection_id, pipeline) = setup(generator.clone());
        let path = temp.path().join("source/preflight.md");
        std::fs::write(&path, "# Preflight\nContenido").unwrap();

        let preflight = pipeline.preflight_collection(collection_id).unwrap();
        assert_eq!(preflight.pending_count(), 1);
        assert_eq!(generator.calls.load(Ordering::SeqCst), 0);
        let detected = db.list_sources(collection_id).unwrap().pop().unwrap();
        assert_eq!(detected.status, DocumentStatus::Detected);
        assert_eq!(detected.revision, 1);
        drop(preflight);

        assert!(matches!(
            pipeline
                .scan_collection(collection_id)
                .await
                .unwrap()
                .as_slice(),
            [IngestOutcome::NeedsReview { .. }]
        ));
        let reviewed = db.list_sources(collection_id).unwrap().pop().unwrap();
        assert_eq!(reviewed.status, DocumentStatus::NeedsReview);
        assert_eq!(reviewed.revision, 1);
        assert_eq!(generator.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stalled_startup_discovery_times_out_without_late_database_mutation() {
        let (_temp, db, collection_id, _pipeline) =
            setup(Arc::new(DeterministicGenerationProvider));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let discovery = tokio::task::spawn_blocking(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            finished_tx.send(()).unwrap();
            Ok(FileDiscovery::default())
        });
        tokio::task::spawn_blocking(move || {
            entered_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap();
        })
        .await
        .unwrap();

        let error = await_startup_discovery(discovery, std::time::Duration::ZERO)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("startup deadline"));
        assert_eq!(db.count("source_documents").unwrap(), 0);
        release_tx.send(()).unwrap();
        tokio::task::spawn_blocking(move || {
            finished_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap();
        })
        .await
        .unwrap();
        assert_eq!(db.count("source_documents").unwrap(), 0);
        assert!(db.collection(collection_id).unwrap().is_some());
    }

    #[tokio::test]
    async fn bounded_startup_preflight_preserves_the_successful_path() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        std::fs::write(
            temp.path().join("source/startup.md"),
            "# Inicio\nEvidencia sintética.",
        )
        .unwrap();

        let preflight = pipeline
            .preflight_collection_with_discovery_timeout(
                collection_id,
                std::time::Duration::from_secs(1),
            )
            .await
            .unwrap();

        assert_eq!(preflight.pending_count(), 1);
        assert_eq!(db.count("source_documents").unwrap(), 1);
    }

    #[tokio::test]
    async fn full_pipeline_is_idempotent_and_requires_explicit_approval() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/runbook.md");
        std::fs::write(
            &path,
            "# Recuperación de pagos\nProcedimiento para reiniciar la cola de pagos.",
        )
        .unwrap();
        let first = pipeline.scan_collection(collection_id).await.unwrap();
        let concept_id = match first.as_slice() {
            [IngestOutcome::NeedsReview { concept_id, .. }] => *concept_id,
            value => panic!("unexpected outcomes: {value:?}"),
        };
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::NeedsReview
        );
        assert!(!temp.path().join("wiki/concepts").exists());

        let draft = db.concept(concept_id).unwrap().unwrap().draft;
        approve_current(&pipeline, &db, concept_id, draft).unwrap();
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::Published
        );
        assert!(
            temp.path()
                .join(format!("wiki/concepts/{concept_id}.md"))
                .is_file()
        );

        let second = pipeline.scan_collection(collection_id).await.unwrap();
        assert!(matches!(
            second.as_slice(),
            [IngestOutcome::Unchanged { .. }]
        ));
        assert_eq!(db.count("source_documents").unwrap(), 1);
        assert_eq!(db.count("concepts").unwrap(), 1);
    }

    #[tokio::test]
    async fn modification_withdraws_search_until_reapproved() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/a.md");
        std::fs::write(&path, "# Pagos\nProcedimiento pagos versión uno").unwrap();
        let published_hash = sha256_file(&path).unwrap();
        let outcome = pipeline.ingest_path(collection_id, &path).await.unwrap();
        let concept_id = match outcome {
            IngestOutcome::NeedsReview { concept_id, .. } => concept_id,
            other => panic!("unexpected {other:?}"),
        };
        let draft = db.concept(concept_id).unwrap().unwrap().draft;
        approve_current(&pipeline, &db, concept_id, draft).unwrap();
        let engine = HybridSearchEngine::new(
            db.clone(),
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(crate::DeterministicEvidenceRelevanceProvider),
            "mac",
        );
        let request = SearchRequest::new("pagos", SearchPurpose::LocalAssistant, 5);
        assert_eq!(
            engine
                .search_local(request.clone())
                .await
                .unwrap()
                .hits
                .len(),
            1
        );

        std::fs::write(&path, "# Pagos\nProcedimiento pagos versión dos").unwrap();
        let replacement_hash = sha256_file(&path).unwrap();
        pipeline.ingest_path(collection_id, &path).await.unwrap();
        assert!(engine.search_local(request).await.unwrap().hits.is_empty());
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::NeedsReview
        );
        let log = std::fs::read_to_string(temp.path().join("wiki/log.md")).unwrap();
        let deprecation = log
            .lines()
            .find(|line| line.contains("**Deprecation**"))
            .unwrap();
        assert!(deprecation.contains(&published_hash));
        assert!(!deprecation.contains(&replacement_hash));
    }

    #[tokio::test]
    async fn incomplete_discovery_quarantines_without_creating_tombstones() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/visible.md");
        std::fs::write(&path, "# Visible\nEvidencia sintética").unwrap();
        let outcome = pipeline.ingest_path(collection_id, &path).await.unwrap();
        let (source_document_id, concept_id) = match outcome {
            IngestOutcome::NeedsReview {
                source_document_id,
                concept_id,
                ..
            } => (source_document_id, concept_id),
            other => panic!("unexpected {other:?}"),
        };
        let draft = db.concept(concept_id).unwrap().unwrap().draft;
        approve_current(&pipeline, &db, concept_id, draft).unwrap();

        let collection = db.collection(collection_id).unwrap().unwrap();
        let discovery = FileDiscovery {
            candidates: Vec::new(),
            issues: vec![crate::ingest::FileDiscoveryIssue {
                path: collection.source_folder.join("unreadable"),
                code: SourceIssueCode::Unreadable,
                error: "source traversal is incomplete: permission denied".into(),
            }],
            status: crate::ingest::FileDiscoveryStatus::Incomplete,
        };
        let preflight = pipeline.preflight_discovery(collection, discovery).unwrap();

        assert!(matches!(
            preflight.outcomes(),
            [IngestOutcome::Failed { .. }]
        ));
        let source = db.source_document(source_document_id).unwrap().unwrap();
        assert_eq!(source.status, DocumentStatus::Failed);
        assert!(source.deleted_at.is_none());
        assert!(
            source
                .last_error
                .as_deref()
                .unwrap()
                .contains("traversal incomplete")
        );
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::Failed
        );
        assert!(
            !temp
                .path()
                .join(format!("wiki/concepts/{concept_id}.md"))
                .exists()
        );
    }

    #[tokio::test]
    async fn present_source_over_limit_becomes_unavailable_not_deleted() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/limited.md");
        std::fs::write(&path, "# Documento\ncontenido que luego excede el límite").unwrap();
        let outcome = pipeline.ingest_path(collection_id, &path).await.unwrap();
        let (source_document_id, concept_id) = match outcome {
            IngestOutcome::NeedsReview {
                source_document_id,
                concept_id,
                ..
            } => (source_document_id, concept_id),
            other => panic!("unexpected {other:?}"),
        };
        let draft = db.concept(concept_id).unwrap().unwrap().draft;
        approve_current(&pipeline, &db, concept_id, draft).unwrap();

        let limited_pipeline = pipeline.clone().with_limits(IngestLimits {
            max_bytes: 8,
            ..IngestLimits::default()
        });
        let outcomes = limited_pipeline
            .scan_collection(collection_id)
            .await
            .unwrap();

        assert!(matches!(
            outcomes.as_slice(),
            [IngestOutcome::Failed { .. }]
        ));
        let source = db.source_document(source_document_id).unwrap().unwrap();
        assert_eq!(source.status, DocumentStatus::Failed);
        assert!(source.deleted_at.is_none());
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::Failed
        );
        assert!(
            !temp
                .path()
                .join(format!("wiki/concepts/{concept_id}.md"))
                .exists()
        );
    }

    #[tokio::test]
    async fn unavailable_collection_is_quarantined_until_rescan_and_review() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/quarantine.md");
        std::fs::write(
            &path,
            "# Secreto sintético\nEvidencia que no debe quedar stale",
        )
        .unwrap();
        let outcome = pipeline.ingest_path(collection_id, &path).await.unwrap();
        let concept_id = match outcome {
            IngestOutcome::NeedsReview { concept_id, .. } => concept_id,
            other => panic!("unexpected {other:?}"),
        };
        let draft = db.concept(concept_id).unwrap().unwrap().draft;
        approve_current(&pipeline, &db, concept_id, draft).unwrap();
        let engine = HybridSearchEngine::new(
            db.clone(),
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(crate::DeterministicEvidenceRelevanceProvider),
            "mac",
        );
        let request = SearchRequest::new("secreto sintético", SearchPurpose::ExternalAi, 5);
        assert_eq!(
            engine
                .search_local(request.clone())
                .await
                .unwrap()
                .hits
                .len(),
            1
        );

        pipeline
            .quarantine_collection(collection_id, "watcher unavailable")
            .unwrap();

        assert!(engine.search_local(request).await.unwrap().hits.is_empty());
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::Failed
        );
        assert!(db.chunks_for_concept(concept_id).unwrap().is_empty());
        assert!(
            !temp
                .path()
                .join(format!("wiki/concepts/{concept_id}.md"))
                .exists()
        );

        let outcomes = pipeline.scan_collection(collection_id).await.unwrap();
        assert!(matches!(
            outcomes.as_slice(),
            [IngestOutcome::NeedsReview { .. }]
        ));
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::NeedsReview
        );
    }

    #[tokio::test]
    async fn scan_withdraws_every_change_before_first_inference_finishes() {
        let generator = Arc::new(BlockNextGeneration::new());
        let (temp, db, collection_id, pipeline) = setup(generator.clone());
        let published_path = temp.path().join("source/99-published.md");
        std::fs::write(
            &published_path,
            "# Política publicada\nClave de evidencia versión uno",
        )
        .unwrap();
        let published = pipeline
            .ingest_path(collection_id, &published_path)
            .await
            .unwrap();
        let (published_source_id, published_concept_id) = match published {
            IngestOutcome::NeedsReview {
                source_document_id,
                concept_id,
                ..
            } => (source_document_id, concept_id),
            other => panic!("unexpected {other:?}"),
        };
        let draft = db.concept(published_concept_id).unwrap().unwrap().draft;
        approve_current(&pipeline, &db, published_concept_id, draft).unwrap();

        let new_path = temp.path().join("source/00-slow-new.md");
        std::fs::write(&new_path, "# Documento lento\nContenido nuevo").unwrap();
        std::fs::write(
            &published_path,
            "# Política publicada\nClave de evidencia versión dos",
        )
        .unwrap();
        generator.arm();
        let scan_pipeline = pipeline.clone();
        let scan = tokio::spawn(async move { scan_pipeline.scan_collection(collection_id).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            generator.entered.notified(),
        )
        .await
        .expect("the first inference should start");

        let source = db.source_document(published_source_id).unwrap().unwrap();
        assert_eq!(source.revision, 2);
        assert_eq!(source.status, DocumentStatus::Detected);
        assert_eq!(
            db.concept(published_concept_id).unwrap().unwrap().status,
            DocumentStatus::NeedsReview
        );
        assert!(
            db.chunks_for_concept(published_concept_id)
                .unwrap()
                .is_empty()
        );
        assert!(
            !temp
                .path()
                .join(format!("wiki/concepts/{published_concept_id}.md"))
                .exists()
        );
        let engine = HybridSearchEngine::new(
            db.clone(),
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(crate::DeterministicEvidenceRelevanceProvider),
            "mac",
        );
        assert!(
            engine
                .search_local(SearchRequest::new(
                    "clave de evidencia",
                    SearchPurpose::LocalAssistant,
                    5,
                ))
                .await
                .unwrap()
                .hits
                .is_empty()
        );

        generator.release.notify_one();
        let outcomes = scan.await.unwrap().unwrap();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, IngestOutcome::NeedsReview { .. }))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn superseded_worker_cannot_overwrite_the_new_revision() {
        let generator = Arc::new(BlockNextGeneration::new());
        let (temp, db, collection_id, pipeline) = setup(generator.clone());
        let path = temp.path().join("source/concurrent.md");
        std::fs::write(&path, "# Revisión uno\nContenido anterior").unwrap();

        generator.arm();
        let old_pipeline = pipeline.clone();
        let old_path = path.clone();
        let old =
            tokio::spawn(async move { old_pipeline.ingest_path(collection_id, old_path).await });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            generator.entered.notified(),
        )
        .await
        .expect("the old revision should enter inference");

        std::fs::write(&path, "# Revisión dos\nContenido vigente").unwrap();
        let current_outcomes = pipeline.scan_collection(collection_id).await.unwrap();
        let current_concept_id = match current_outcomes.as_slice() {
            [IngestOutcome::NeedsReview { concept_id, .. }] => *concept_id,
            other => panic!("unexpected current outcomes: {other:?}"),
        };

        generator.release.notify_one();
        assert!(matches!(
            old.await.unwrap().unwrap(),
            IngestOutcome::Failed { .. }
        ));
        let source = db.list_sources(collection_id).unwrap().pop().unwrap();
        assert_eq!(source.revision, 2);
        assert_eq!(source.status, DocumentStatus::NeedsReview);
        let concept = db.concept(current_concept_id).unwrap().unwrap();
        assert_eq!(concept.status, DocumentStatus::NeedsReview);
        assert_eq!(concept.draft.title, "Revisión dos");
        let chunks = db.chunks_for_concept(current_concept_id).unwrap();
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|chunk| chunk.source_revision == 2));
    }

    #[tokio::test]
    async fn scan_reuses_source_registered_through_a_path_alias() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/aliased.md");
        let alias = path
            .parent()
            .unwrap()
            .join(".")
            .join(path.file_name().unwrap());
        std::fs::write(&path, "# Revisión uno\nContenido anterior").unwrap();
        let initial = pipeline.ingest_path(collection_id, &alias).await.unwrap();
        let (source_document_id, concept_id) = match initial {
            IngestOutcome::NeedsReview {
                source_document_id,
                concept_id,
                ..
            } => (source_document_id, concept_id),
            other => panic!("unexpected initial outcome: {other:?}"),
        };

        std::fs::write(&path, "# Revisión dos\nContenido vigente").unwrap();
        let outcomes = pipeline.scan_collection(collection_id).await.unwrap();
        let current_concept_id = match outcomes.as_slice() {
            [IngestOutcome::NeedsReview { concept_id, .. }] => *concept_id,
            other => panic!("unexpected scan outcomes: {other:?}"),
        };

        let sources = db.list_sources(collection_id).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, source_document_id);
        assert_eq!(sources[0].revision, 2);
        assert_eq!(current_concept_id, concept_id);
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().draft.title,
            "Revisión dos"
        );
    }

    #[tokio::test]
    async fn transient_okf_failure_returns_to_review_without_losing_chunks() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/retry-publication.md");
        std::fs::write(&path, "# Pagos\nProcedimiento de recuperación").unwrap();
        let outcome = pipeline.ingest_path(collection_id, &path).await.unwrap();
        let concept_id = match outcome {
            IngestOutcome::NeedsReview { concept_id, .. } => concept_id,
            other => panic!("unexpected {other:?}"),
        };
        let draft = db.concept(concept_id).unwrap().unwrap().draft;
        let wiki = temp.path().join("wiki");
        std::fs::write(&wiki, "blocks the wiki directory").unwrap();

        assert!(approve_current(&pipeline, &db, concept_id, draft.clone()).is_err());
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::NeedsReview
        );
        assert!(!db.chunks_for_concept(concept_id).unwrap().is_empty());

        std::fs::remove_file(wiki).unwrap();
        approve_current(&pipeline, &db, concept_id, draft).unwrap();
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::Published
        );
    }

    #[tokio::test]
    async fn approval_refuses_a_source_changed_after_enrichment() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/changed-before-review.md");
        std::fs::write(&path, "# Pagos\nProcedimiento versión uno").unwrap();
        let outcome = pipeline.ingest_path(collection_id, &path).await.unwrap();
        let concept_id = match outcome {
            IngestOutcome::NeedsReview { concept_id, .. } => concept_id,
            other => panic!("unexpected {other:?}"),
        };
        let draft = db.concept(concept_id).unwrap().unwrap().draft;
        std::fs::write(&path, "# Pagos\nProcedimiento versión dos").unwrap();

        let error = approve_current(&pipeline, &db, concept_id, draft).unwrap_err();
        assert!(error.to_string().contains("source changed"));
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::NeedsReview
        );
        assert!(!temp.path().join("wiki/concepts").exists());
    }

    struct AlwaysInvalid {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl GenerationProvider for AlwaysInvalid {
        fn model_id(&self) -> &str {
            "invalid-test"
        }

        async fn enrich(&self, _document_text: &str) -> Result<EnrichmentDraft> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(anyhow!("unexpected token at byte 7").context("invalid JSON"))
        }
    }

    #[tokio::test]
    async fn generation_retries_once_then_creates_manual_review_draft() {
        let generator = Arc::new(AlwaysInvalid {
            calls: AtomicUsize::new(0),
        });
        let (temp, _db, collection_id, pipeline) = setup(generator.clone());
        let path = temp.path().join("source/manual.md");
        std::fs::write(&path, "contenido importante para revisar").unwrap();
        let outcome = pipeline.ingest_path(collection_id, path).await.unwrap();
        assert!(matches!(
            outcome,
            IngestOutcome::NeedsReview {
                used_fallback_metadata: true,
                ..
            }
        ));
        assert_eq!(generator.calls.load(Ordering::SeqCst), 2);
    }

    struct FailsInitialAttempts {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl GenerationProvider for FailsInitialAttempts {
        fn model_id(&self) -> &str {
            "recovered-model"
        }

        async fn enrich(&self, document_text: &str) -> Result<EnrichmentDraft> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call < 2 {
                bail!("model was not ready")
            }
            DeterministicGenerationProvider.enrich(document_text).await
        }
    }

    #[tokio::test]
    async fn reanalysis_replaces_fallback_in_place_without_publishing() {
        let generator = Arc::new(FailsInitialAttempts {
            calls: AtomicUsize::new(0),
        });
        let (temp, db, collection_id, pipeline) = setup(generator.clone());
        let path = temp.path().join("source/reanalyze.md");
        std::fs::write(
            &path,
            "# Hitos\nFecha de cierre y responsables del proyecto",
        )
        .unwrap();
        let (source_id, concept_id) =
            match pipeline.ingest_path(collection_id, &path).await.unwrap() {
                IngestOutcome::NeedsReview {
                    source_document_id,
                    concept_id,
                    used_fallback_metadata: true,
                } => (source_document_id, concept_id),
                other => panic!("unexpected outcome: {other:?}"),
            };
        let before = db.concept(concept_id).unwrap().unwrap();
        assert!(before.draft.description.contains("automático falló"));
        let revision = db.source_document(source_id).unwrap().unwrap().revision;

        let updated = pipeline.reanalyze_review(concept_id).await.unwrap();

        assert_eq!(updated.id, concept_id);
        assert_eq!(updated.source_document_id, source_id);
        assert_eq!(updated.status, DocumentStatus::NeedsReview);
        assert_eq!(updated.generator_model, "recovered-model");
        assert!(!updated.draft.description.contains("automático falló"));
        let source = db.source_document(source_id).unwrap().unwrap();
        assert_eq!(source.revision, revision);
        assert_eq!(source.status, DocumentStatus::NeedsReview);
        assert!(source.last_error.is_none());
        assert!(!db.chunks_for_concept(concept_id).unwrap().is_empty());
        assert!(!temp.path().join("wiki/concepts").exists());
        assert_eq!(generator.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn failed_reanalysis_restores_the_previous_review_unchanged() {
        let generator = Arc::new(AlwaysInvalid {
            calls: AtomicUsize::new(0),
        });
        let (temp, db, collection_id, pipeline) = setup(generator.clone());
        let path = temp.path().join("source/reanalyze-failure.md");
        std::fs::write(&path, "contenido que debe permanecer pendiente").unwrap();
        let concept_id = match pipeline.ingest_path(collection_id, &path).await.unwrap() {
            IngestOutcome::NeedsReview { concept_id, .. } => concept_id,
            other => panic!("unexpected outcome: {other:?}"),
        };
        let before = db.concept(concept_id).unwrap().unwrap();
        let before_draft = serde_json::to_value(&before.draft).unwrap();
        let before_chunks = db.chunks_for_concept(concept_id).unwrap();

        let error = pipeline.reanalyze_review(concept_id).await.unwrap_err();

        assert!(error.to_string().contains("failed twice"));
        assert!(
            error.to_string().contains("unexpected token at byte 7"),
            "nested inference cause was discarded: {error:#}"
        );
        let after = db.concept(concept_id).unwrap().unwrap();
        assert_eq!(after.status, DocumentStatus::NeedsReview);
        assert_eq!(serde_json::to_value(&after.draft).unwrap(), before_draft);
        let after_chunks = db.chunks_for_concept(concept_id).unwrap();
        assert_eq!(after_chunks.len(), before_chunks.len());
        assert_eq!(after_chunks[0].id, before_chunks[0].id);
        let source = db
            .source_document(after.source_document_id)
            .unwrap()
            .unwrap();
        assert_eq!(source.status, DocumentStatus::NeedsReview);
        let last_error = source.last_error.unwrap();
        assert!(last_error.contains("failed twice"));
        assert!(last_error.contains("unexpected token at byte 7"));
        assert_eq!(generator.calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn claimed_reanalysis_is_not_approvable_and_failure_restores_review() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/reanalysis-claim.md");
        std::fs::write(&path, "# Claim\nNo publicar durante inferencia").unwrap();
        let concept_id = match pipeline.ingest_path(collection_id, &path).await.unwrap() {
            IngestOutcome::NeedsReview { concept_id, .. } => concept_id,
            other => panic!("unexpected outcome: {other:?}"),
        };
        let draft = db.concept(concept_id).unwrap().unwrap().draft;
        let review_version = review_version(&db, concept_id).unwrap();
        let claim = db.begin_review_reanalysis(concept_id).unwrap();

        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::Enriched
        );
        assert!(
            pipeline
                .approve(concept_id, ReviewEdits { draft }, &review_version)
                .is_err()
        );
        assert!(db.fail_review_reanalysis(&claim, "cancelled").unwrap());
        assert_eq!(
            db.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::NeedsReview
        );
        assert_eq!(
            db.source_document(claim.source_document_id)
                .unwrap()
                .unwrap()
                .status,
            DocumentStatus::NeedsReview
        );
    }

    #[tokio::test]
    async fn deletion_creates_tombstone_and_removes_okf_file() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let path = temp.path().join("source/delete.md");
        std::fs::write(&path, "# Borrar\ncontenido de pagos").unwrap();
        let outcome = pipeline.ingest_path(collection_id, &path).await.unwrap();
        let (source_id, concept_id) = match outcome {
            IngestOutcome::NeedsReview {
                source_document_id,
                concept_id,
                ..
            } => (source_document_id, concept_id),
            other => panic!("unexpected {other:?}"),
        };
        let draft = db.concept(concept_id).unwrap().unwrap().draft;
        approve_current(&pipeline, &db, concept_id, draft).unwrap();
        std::fs::remove_file(path).unwrap();
        let outcomes = pipeline.scan_collection(collection_id).await.unwrap();
        assert!(matches!(
            outcomes.as_slice(),
            [IngestOutcome::Deleted { .. }]
        ));
        assert_eq!(
            db.source_document(source_id).unwrap().unwrap().status,
            DocumentStatus::Deleted
        );
        assert!(
            !temp
                .path()
                .join(format!("wiki/concepts/{concept_id}.md"))
                .exists()
        );
    }

    #[tokio::test]
    async fn same_hash_rename_preserves_source_and_concept_identity() {
        let (temp, db, collection_id, pipeline) = setup(Arc::new(DeterministicGenerationProvider));
        let old_path = temp.path().join("source/old.md");
        let new_path = temp.path().join("source/new.md");
        std::fs::write(&old_path, "# Runbook\nProcedimiento para pagos").unwrap();
        let outcome = pipeline
            .ingest_path(collection_id, &old_path)
            .await
            .unwrap();
        let (source_id, concept_id) = match outcome {
            IngestOutcome::NeedsReview {
                source_document_id,
                concept_id,
                ..
            } => (source_document_id, concept_id),
            other => panic!("unexpected {other:?}"),
        };
        let draft = db.concept(concept_id).unwrap().unwrap().draft;
        approve_current(&pipeline, &db, concept_id, draft).unwrap();
        std::fs::rename(old_path, &new_path).unwrap();
        let outcomes = pipeline.scan_collection(collection_id).await.unwrap();
        assert!(matches!(
            outcomes.as_slice(),
            [IngestOutcome::Renamed {
                source_document_id
            }] if *source_document_id == source_id
        ));
        let source = db.source_document(source_id).unwrap().unwrap();
        assert_eq!(source.source_path, new_path);
        assert_eq!(source.concept_id, Some(concept_id));
        assert_eq!(source.status, DocumentStatus::Published);
    }

    #[tokio::test]
    async fn identical_sources_in_distinct_collections_are_both_indexed() {
        let (temp, db, first_collection_id, pipeline) =
            setup(Arc::new(DeterministicGenerationProvider));
        let second_source = temp.path().join("second-source");
        let second_wiki = temp.path().join("second-wiki");
        std::fs::create_dir_all(&second_source).unwrap();
        let second_collection = db
            .create_collection(
                "Segundo piloto",
                &second_source,
                &second_wiki,
                CollectionPolicy::local_only(),
            )
            .unwrap();
        let first_path = temp.path().join("source/shared.md");
        let second_path = second_source.join("shared.md");
        let contents =
            "# Evidencia sintética\nEl mismo contenido puede pertenecer a dos colecciones.";
        std::fs::write(&first_path, contents).unwrap();
        std::fs::write(&second_path, contents).unwrap();

        let first = pipeline
            .ingest_path(first_collection_id, &first_path)
            .await
            .unwrap();
        let second = pipeline
            .ingest_path(second_collection.id, &second_path)
            .await
            .unwrap();

        let (first_concept_id, second_concept_id) = match (first, second) {
            (
                IngestOutcome::NeedsReview {
                    concept_id: first_concept_id,
                    ..
                },
                IngestOutcome::NeedsReview {
                    concept_id: second_concept_id,
                    ..
                },
            ) => (first_concept_id, second_concept_id),
            other => panic!("unexpected outcomes: {other:?}"),
        };
        let first_chunk = db.chunks_for_concept(first_concept_id).unwrap().remove(0);
        let second_chunk = db.chunks_for_concept(second_concept_id).unwrap().remove(0);

        assert_ne!(first_chunk.id, second_chunk.id);
    }
}
