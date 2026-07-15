use std::fs;

use airwiki_types::{DocumentStatus, EnrichmentDraft};
use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use uuid::Uuid;

use crate::ingest::sha256_file;
use crate::okf::OkfPublisher;
use crate::storage::{ConceptRecord, Database, PublicationClaim, SourceDocumentRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicationStep {
    ConceptWritten,
    IndexWritten,
    LogWritten,
    DatabaseCommitted,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PublicationRecoveryReport {
    pub completed: usize,
    pub cancelled: usize,
    pub pending: usize,
}

/// Drives the durable two-phase boundary between human approval in SQLite and
/// the three-file OKF representation. It never runs inference and is safe to
/// invoke at startup before search transports are enabled.
#[derive(Debug, Clone)]
pub struct OkfPublicationMaterializer {
    database: Database,
}

impl OkfPublicationMaterializer {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub fn approve(&self, concept_id: Uuid, draft: EnrichmentDraft) -> Result<ConceptRecord> {
        let _guard = self.database.publication_guard()?;
        self.approve_locked(concept_id, draft)
    }

    fn approve_locked(&self, concept_id: Uuid, draft: EnrichmentDraft) -> Result<ConceptRecord> {
        let recovery = self.recover_claims()?;
        if recovery.pending > 0 {
            bail!("pending OKF publication cleanup must finish before another approval");
        }
        let current = self
            .database
            .concept(concept_id)?
            .ok_or_else(|| anyhow!("concept {concept_id} does not exist"))?;
        if current.status != DocumentStatus::NeedsReview {
            bail!("concept is not awaiting human review");
        }
        let source = self
            .database
            .source_document(current.source_document_id)?
            .context("concept source document is missing")?;
        let collection = self
            .database
            .collection(current.collection_id)?
            .context("concept collection is missing")?;
        if !source_revision_matches_disk(&source)? {
            bail!("the source changed after enrichment; rescan it before publishing");
        }

        let mut candidate = current;
        candidate.draft = draft.clone();
        let reviewed_at = Utc::now();
        let publisher = OkfPublisher::new(&collection.wiki_folder);
        publisher.validate_candidate(&candidate, &source, reviewed_at)?;
        let action = if source.revision > 1 {
            "replaced"
        } else {
            "published"
        };
        let claim = self.database.begin_publication_if_current(
            concept_id,
            draft,
            &source.source_sha256,
            source.revision,
            action,
            reviewed_at,
        )?;

        match self.materialize_claim(&claim) {
            Ok(published) => Ok(published),
            Err(error) => {
                if let Err(cleanup_error) = self.cancel_claim(&claim, &format!("{error:#}")) {
                    return Err(anyhow!(
                        "OKF publication failed: {error}; cleanup also failed: {cleanup_error}"
                    ));
                }
                Err(error)
            }
        }
    }

    pub fn recover_pending(&self) -> Result<PublicationRecoveryReport> {
        let _guard = self.database.publication_guard()?;
        self.recover_claims()
    }

    pub fn withdraw_published_artifact(
        &self,
        collection_id: Uuid,
        concept_id: Uuid,
        source_sha256: &str,
    ) -> Result<()> {
        let _guard = self.database.publication_guard()?;
        self.withdraw_artifacts_locked(collection_id, &[(concept_id, source_sha256.to_owned())])
    }

    pub fn withdraw_published_artifacts(
        &self,
        collection_id: Uuid,
        artifacts: &[(Uuid, String)],
    ) -> Result<()> {
        let _guard = self.database.publication_guard()?;
        self.withdraw_artifacts_locked(collection_id, artifacts)
    }

    fn withdraw_artifacts_locked(
        &self,
        collection_id: Uuid,
        artifacts: &[(Uuid, String)],
    ) -> Result<()> {
        let collection = self
            .database
            .collection(collection_id)?
            .with_context(|| format!("collection {collection_id} does not exist"))?;
        let publisher = OkfPublisher::new(&collection.wiki_folder);
        for (concept_id, source_sha256) in artifacts {
            let remaining = self.database.list_published_concepts(collection_id)?;
            publisher.remove(*concept_id, source_sha256, &remaining)?;
        }
        Ok(())
    }

    fn recover_claims(&self) -> Result<PublicationRecoveryReport> {
        let mut report = PublicationRecoveryReport::default();
        for claim in self.database.publication_claims()? {
            self.database.note_publication_retry(&claim)?;
            let source_matches = match source_claim_matches_disk(&claim) {
                Ok(matches) => matches,
                Err(error) => {
                    self.database
                        .record_publication_error(&claim, &format!("{error:#}"))?;
                    report.pending += 1;
                    continue;
                }
            };
            let should_cancel = claim.job_state == "cancelling"
                || !self.database.publication_claim_is_current(&claim)?
                || !source_matches;
            if should_cancel {
                match self.cancel_claim(
                    &claim,
                    "pending publication no longer matches its approved source revision",
                ) {
                    Ok(()) => report.cancelled += 1,
                    Err(error) => {
                        self.database
                            .record_publication_error(&claim, &format!("{error:#}"))?;
                        report.pending += 1;
                    }
                }
                continue;
            }
            match self.materialize_claim(&claim) {
                Ok(_) => report.completed += 1,
                Err(error) => {
                    self.database
                        .record_publication_error(&claim, &format!("{error:#}"))?;
                    report.pending += 1;
                }
            }
        }
        Ok(report)
    }

    fn materialize_claim(&self, claim: &PublicationClaim) -> Result<ConceptRecord> {
        self.materialize_claim_with_observer(claim, |_| Ok(()))
    }

    fn materialize_claim_with_observer(
        &self,
        claim: &PublicationClaim,
        mut observe: impl FnMut(PublicationStep) -> Result<()>,
    ) -> Result<ConceptRecord> {
        if !self.database.publication_claim_is_current(claim)? {
            bail!("publication claim is no longer current");
        }
        if !source_claim_matches_disk(claim)? {
            bail!("source changed while OKF publication was pending");
        }
        let concept = self
            .database
            .concept(claim.concept_id)?
            .context("publication concept disappeared")?;
        let source = self
            .database
            .source_document(claim.source_document_id)?
            .context("publication source disappeared")?;
        let collection = self
            .database
            .collection(claim.collection_id)?
            .context("publication collection disappeared")?;
        let snapshot = self.database.publication_snapshot(claim)?;
        let publisher = OkfPublisher::new(&collection.wiki_folder);

        publisher.write_concept(&concept, &source)?;
        observe(PublicationStep::ConceptWritten)?;
        publisher.regenerate_index(&snapshot)?;
        observe(PublicationStep::IndexWritten)?;
        publisher.append_publication_log(&claim.action, &concept, &source)?;
        observe(PublicationStep::LogWritten)?;

        if !self.database.publication_claim_is_current(claim)? || !source_claim_matches_disk(claim)?
        {
            bail!("source or publication state changed while OKF files were being written");
        }
        if !self.database.complete_publication_if_current(claim)? {
            bail!("publication was superseded before its final database commit");
        }
        observe(PublicationStep::DatabaseCommitted)?;
        self.database
            .concept(claim.concept_id)?
            .context("published concept disappeared")
    }

    fn cancel_claim(&self, claim: &PublicationClaim, reason: &str) -> Result<()> {
        self.database.mark_publication_cancelling(claim, reason)?;
        let collection = self
            .database
            .collection(claim.collection_id)?
            .context("publication collection disappeared during cleanup")?;
        let remaining = self.database.list_published_concepts(claim.collection_id)?;
        // log.md is append-only. If a crash happened after its entry was written,
        // cleanup intentionally leaves that diagnostic history intact while the
        // inspector reports the DB/bundle disagreement.
        OkfPublisher::new(&collection.wiki_folder)
            .discard_failed_publication(claim.concept_id, &remaining)?;
        self.database.finish_publication_cancellation(claim, reason)
    }
}

fn source_revision_matches_disk(source: &SourceDocumentRecord) -> Result<bool> {
    match fs::symlink_metadata(&source.source_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Ok(false),
        Ok(_) => Ok(sha256_file(&source.source_path)? == source.source_sha256),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("could not inspect the approved source revision"),
    }
}

fn source_claim_matches_disk(claim: &PublicationClaim) -> Result<bool> {
    match fs::symlink_metadata(&claim.source_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Ok(false),
        Ok(_) => Ok(sha256_file(&claim.source_path)? == claim.source_sha256),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("could not inspect the pending publication source"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};

    use airwiki_types::{
        CollectionPolicy, ConceptType, EnrichmentDraft, SearchPurpose, SearchRequest,
    };
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::{
        DeterministicEmbeddingProvider, EMBEDDING_DIMENSIONS, HybridSearchEngine,
        KnowledgeBundleState, OkfBundleInspector, StoredChunk,
    };

    struct Fixture {
        _temp: tempfile::TempDir,
        database_path: PathBuf,
        database: Database,
        collection_id: Uuid,
        concept_id: Uuid,
        draft: EnrichmentDraft,
        source_sha256: String,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let database_path = temp.path().join("airwiki.sqlite");
        let source_folder = temp.path().join("source");
        let wiki_folder = temp.path().join("wiki");
        fs::create_dir_all(&source_folder).unwrap();
        fs::create_dir_all(&wiki_folder).unwrap();
        let source_path = source_folder.join("runbook.md");
        fs::write(&source_path, "# Recovery\nSynthetic recovery evidence").unwrap();
        let source_sha256 = hex::encode(Sha256::digest(fs::read(&source_path).unwrap()));
        let database = Database::open(&database_path).unwrap();
        let collection = database
            .create_collection(
                "Recovery",
                source_folder,
                wiki_folder,
                CollectionPolicy::local_only(),
            )
            .unwrap();
        let source_id = database
            .register_source(
                collection.id,
                &source_path,
                &source_sha256,
                "markdown",
                fs::metadata(&source_path).unwrap().len(),
            )
            .unwrap()
            .id();
        let draft = EnrichmentDraft {
            concept_type: ConceptType::Runbook,
            title: "Recovery runbook".into(),
            description: "Synthetic recovery procedure".into(),
            language: "en".into(),
            tags: vec!["recovery".into()],
            entities: Vec::new(),
            links: Vec::new(),
            summary: "Recover the synthetic service.".into(),
            classification_confidence: 0.9,
            classification_explanation: "Contains an operational procedure".into(),
        };
        let concept = database
            .save_enrichment(source_id, draft.clone(), "test-node", "fake-model")
            .unwrap();
        database
            .replace_chunks(
                concept.id,
                &[StoredChunk {
                    id: Uuid::new_v4(),
                    concept_id: concept.id,
                    source_document_id: source_id,
                    collection_id: collection.id,
                    ordinal: 0,
                    heading_or_page: "Recovery".into(),
                    text: "Synthetic recovery evidence".into(),
                    text_sha256: "b".repeat(64),
                    embedding: vec![0.1; EMBEDDING_DIMENSIONS],
                    source_revision: 1,
                }],
            )
            .unwrap();
        Fixture {
            _temp: temp,
            database_path,
            database,
            collection_id: collection.id,
            concept_id: concept.id,
            draft,
            source_sha256,
        }
    }

    fn begin_claim(fixture: &Fixture) -> PublicationClaim {
        fixture
            .database
            .begin_publication_if_current(
                fixture.concept_id,
                fixture.draft.clone(),
                &fixture.source_sha256,
                1,
                "published",
                Utc::now(),
            )
            .unwrap()
    }

    fn assert_recovery_after(fail_after: Option<PublicationStep>) {
        let fixture = fixture();
        let claim = begin_claim(&fixture);
        if let Some(fail_after) = fail_after {
            let materializer = OkfPublicationMaterializer::new(fixture.database.clone());
            let error = materializer
                .materialize_claim_with_observer(&claim, |step| {
                    if step == fail_after {
                        bail!("simulated abrupt shutdown after {step:?}");
                    }
                    Ok(())
                })
                .unwrap_err();
            assert!(error.to_string().contains("simulated abrupt shutdown"));
        }

        let database_path = fixture.database_path.clone();
        let collection_id = fixture.collection_id;
        let concept_id = fixture.concept_id;
        drop(fixture.database);
        let database = Database::open(database_path).unwrap();
        let materializer = OkfPublicationMaterializer::new(database.clone());
        let first = materializer.recover_pending().unwrap();
        if fail_after == Some(PublicationStep::DatabaseCommitted) {
            assert_eq!(first.completed, 0);
        } else {
            assert_eq!(first.completed, 1);
        }
        assert_eq!(first.pending, 0);
        assert_eq!(materializer.recover_pending().unwrap(), Default::default());
        assert_eq!(
            database.concept(concept_id).unwrap().unwrap().status,
            DocumentStatus::Published
        );
        assert!(database.publication_claims().unwrap().is_empty());
        assert_eq!(database.count("audit_events").unwrap(), 1);

        let published = database.concept(concept_id).unwrap().unwrap();
        let collection = database.collection(collection_id).unwrap().unwrap();
        let concept_markdown =
            fs::read_to_string(OkfPublisher::new(&collection.wiki_folder).concept_path(concept_id))
                .unwrap();
        let profile = crate::okf::OkfConcept::parse(&concept_markdown).unwrap();
        assert_eq!(profile.timestamp, published.reviewed_at.unwrap());
        let bundle = OkfBundleInspector::new(database.clone())
            .inspect_bundle(collection_id)
            .unwrap();
        assert!(bundle.health.is_healthy(), "{:#?}", bundle.health.issues);

        let index = fs::read_to_string(collection.wiki_folder.join("index.md")).unwrap();
        let log = fs::read_to_string(collection.wiki_folder.join("log.md")).unwrap();
        assert_eq!(index.matches(&concept_id.to_string()).count(), 1);
        assert_eq!(
            log.matches(&format!("airwiki:event:published:{concept_id}:"))
                .count(),
            1
        );
    }

    #[test]
    fn every_durable_publication_boundary_recovers_idempotently() {
        for fail_after in [
            None,
            Some(PublicationStep::ConceptWritten),
            Some(PublicationStep::IndexWritten),
            Some(PublicationStep::LogWritten),
            Some(PublicationStep::DatabaseCommitted),
        ] {
            assert_recovery_after(fail_after);
        }
    }

    #[tokio::test]
    async fn publishing_claim_is_fail_closed_for_search_and_visible_as_updating() {
        let fixture = fixture();
        let _claim = begin_claim(&fixture);
        let engine = HybridSearchEngine::new(
            fixture.database.clone(),
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(crate::DeterministicEvidenceRelevanceProvider),
            "test-node",
        );
        let response = engine
            .search_local(SearchRequest::new(
                "synthetic recovery evidence",
                SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap();
        assert!(response.hits.is_empty());
        let bundle = OkfBundleInspector::new(fixture.database.clone())
            .inspect_bundle(fixture.collection_id)
            .unwrap();
        assert_eq!(bundle.state, KnowledgeBundleState::Updating);
        assert!(
            bundle
                .health
                .issues
                .iter()
                .any(|issue| issue.code == "publication_pending")
        );
    }

    #[test]
    fn concurrent_recovery_calls_materialize_one_claim_once() {
        let fixture = fixture();
        let _claim = begin_claim(&fixture);
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let database = fixture.database.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                OkfPublicationMaterializer::new(database)
                    .recover_pending()
                    .unwrap()
            }));
        }
        barrier.wait();
        let reports = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            reports.iter().map(|report| report.completed).sum::<usize>(),
            1
        );
        assert!(reports.iter().all(|report| report.pending == 0));
        assert_eq!(fixture.database.count("audit_events").unwrap(), 1);
        assert!(fixture.database.publication_claims().unwrap().is_empty());
    }

    #[test]
    fn recovery_cancels_when_the_approved_source_changed_while_closed() {
        let fixture = fixture();
        let claim = begin_claim(&fixture);
        let materializer = OkfPublicationMaterializer::new(fixture.database.clone());
        materializer
            .materialize_claim_with_observer(&claim, |step| {
                if step == PublicationStep::ConceptWritten {
                    bail!("simulated shutdown");
                }
                Ok(())
            })
            .unwrap_err();
        fs::write(&claim.source_path, "# Changed\nA newer source revision").unwrap();

        let report = materializer.recover_pending().unwrap();
        assert_eq!(report.cancelled, 1);
        assert_eq!(report.pending, 0);
        assert_eq!(
            fixture
                .database
                .concept(fixture.concept_id)
                .unwrap()
                .unwrap()
                .status,
            DocumentStatus::NeedsReview
        );
        assert!(
            fixture
                .database
                .list_published_concepts(fixture.collection_id)
                .unwrap()
                .is_empty()
        );
        assert!(fixture.database.publication_claims().unwrap().is_empty());
        let collection = fixture
            .database
            .collection(fixture.collection_id)
            .unwrap()
            .unwrap();
        assert!(
            !OkfPublisher::new(collection.wiki_folder)
                .concept_path(fixture.concept_id)
                .exists()
        );
    }

    #[test]
    fn unavailable_bundle_keeps_recovery_pending_and_fail_closed() {
        let fixture = fixture();
        let _claim = begin_claim(&fixture);
        let collection = fixture
            .database
            .collection(fixture.collection_id)
            .unwrap()
            .unwrap();
        fs::remove_dir(&collection.wiki_folder).unwrap();
        fs::write(&collection.wiki_folder, "temporarily unavailable").unwrap();

        let report = OkfPublicationMaterializer::new(fixture.database.clone())
            .recover_pending()
            .unwrap();
        assert_eq!(report.pending, 1);
        assert_eq!(report.completed, 0);
        assert_eq!(
            fixture
                .database
                .concept(fixture.concept_id)
                .unwrap()
                .unwrap()
                .status,
            DocumentStatus::Publishing
        );
        assert_eq!(fixture.database.publication_claims().unwrap().len(), 1);
    }
}
