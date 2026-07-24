use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use airwiki_types::{
    FederatedSearch, MAX_HEADING_OR_PAGE_CHARS, MAX_SNIPPET_CHARS, SearchContractError, SearchHit,
    SearchPurpose, SearchRequest, SearchResponse,
};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use thiserror::Error;
use uuid::Uuid;

use crate::chunk_identity::public_chunk_id;
use crate::inference::EmbeddingProvider;
use crate::storage::{Database, RankedChunk};

const RRF_K: f64 = 60.0;
const VECTOR_SQL_BATCH_SIZE: usize = 512;
/// Fixed hybrid-retrieval pool classified before `top_k` truncates evidence.
pub const RELEVANCE_CANDIDATE_LIMIT: usize = 10;
/// Fixed per-channel over-retrieval bound before RRF and content deduplication.
const PRE_DEDUPLICATION_CANDIDATE_LIMIT: usize = RELEVANCE_CANDIDATE_LIMIT * 4;

/// Candidate evidence presented to the local relevance classifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelevanceInput {
    pub title: String,
    pub heading: String,
    pub text: String,
}

/// Fail-closed decision for one candidate, in the same order as the input batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceDecision {
    Relevant,
    Irrelevant,
}

/// Sanitized failures from a local evidence relevance provider.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EvidenceRelevanceError {
    #[error("evidence relevance provider is unavailable")]
    Unavailable,
    #[error("evidence relevance inference failed")]
    InferenceFailed,
    #[error("evidence relevance inference timed out")]
    TimedOut,
    #[error("evidence relevance provider returned invalid output")]
    InvalidOutput,
    #[error("evidence relevance provider returned {actual} decisions for {expected} candidates")]
    DecisionCountMismatch { expected: usize, actual: usize },
}

/// Classifies whether retrieved passages contain evidence for the question.
#[async_trait]
pub trait EvidenceRelevanceProvider: Send + Sync {
    fn profile_id(&self) -> &str;

    async fn classify(
        &self,
        question: &str,
        candidates: &[RelevanceInput],
    ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError>;
}

/// Repeatable lexical test double for offline tests. It is not a production
/// answerability model.
#[derive(Debug, Default, Clone)]
pub struct DeterministicEvidenceRelevanceProvider;

#[async_trait]
impl EvidenceRelevanceProvider for DeterministicEvidenceRelevanceProvider {
    fn profile_id(&self) -> &str {
        "deterministic-token-overlap-test-double"
    }

    async fn classify(
        &self,
        question: &str,
        candidates: &[RelevanceInput],
    ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
        let question_terms = normalized_terms(question);
        Ok(candidates
            .iter()
            .map(|candidate| {
                let mut candidate_terms = normalized_terms(&candidate.title);
                candidate_terms.extend(normalized_terms(&candidate.heading));
                candidate_terms.extend(normalized_terms(&candidate.text));
                if question_terms
                    .iter()
                    .any(|term| candidate_terms.contains(term))
                {
                    EvidenceDecision::Relevant
                } else {
                    EvidenceDecision::Irrelevant
                }
            })
            .collect())
    }
}

#[derive(Clone)]
pub struct HybridSearchEngine {
    database: Database,
    embeddings: Arc<dyn EmbeddingProvider>,
    relevance: Arc<dyn EvidenceRelevanceProvider>,
    node_id: String,
}

impl std::fmt::Debug for HybridSearchEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HybridSearchEngine")
            .field("database", &self.database)
            .field("embedding_model", &self.embeddings.model_id())
            .field("relevance_profile", &self.relevance.profile_id())
            .field("node_id", &self.node_id)
            .finish()
    }
}

impl HybridSearchEngine {
    pub fn new(
        database: Database,
        embeddings: Arc<dyn EmbeddingProvider>,
        relevance: Arc<dyn EvidenceRelevanceProvider>,
        node_id: impl Into<String>,
    ) -> Self {
        Self {
            database,
            embeddings,
            relevance,
            node_id: node_id.into(),
        }
    }

    /// Local desktop search includes local-only collections. `ExternalAi` is still
    /// filtered to collections explicitly opted into cloud disclosure.
    pub async fn search_local(&self, request: SearchRequest) -> Result<SearchResponse> {
        request.validate()?;
        let database = self.database.clone();
        let collections = run_search_blocking("local search scope worker task failed", move || {
            Ok(database
                .list_collections()?
                .into_iter()
                .map(|collection| collection.id)
                .collect::<Vec<_>>())
        })
        .await?;
        self.search_collections(request, &collections).await
    }

    /// Remote callers never provide their own collection list; it comes from the
    /// local trust store and collection policy intersection.
    pub async fn search_for_peer(
        &self,
        request: SearchRequest,
        peer_id: &str,
    ) -> Result<SearchResponse> {
        request.validate()?;
        let purpose = request.purpose;
        let database = self.database.clone();
        let peer_id = peer_id.to_owned();
        let scope_peer_id = peer_id.clone();
        let collections = run_search_blocking("peer search scope worker task failed", move || {
            let peer = database
                .peer(&scope_peer_id)?
                .ok_or_else(|| anyhow::anyhow!("unknown peer"))?;
            if !peer.trusted || peer.blocked {
                bail!("peer is not authorized");
            }
            database.granted_collections_for_search(&scope_peer_id, purpose)
        })
        .await?;
        let mut response = self.search_collections(request, &collections).await?;
        let database = self.database.clone();
        let hits = std::mem::take(&mut response.hits);
        let authorized_candidates = std::mem::take(&mut response.authorized_candidates);
        let hits_peer_id = peer_id.clone();
        response.hits =
            run_search_blocking("peer search revalidation worker task failed", move || {
                revalidate_peer_hits(database, hits, hits_peer_id, purpose)
            })
            .await?;
        let database = self.database.clone();
        response.authorized_candidates = run_search_blocking(
            "peer candidate revalidation worker task failed",
            move || revalidate_peer_hits(database, authorized_candidates, peer_id, purpose),
        )
        .await?;
        Ok(response)
    }

    /// Searches only explicitly Internet-public collections. Public search is
    /// evidence-only: making content public cannot enable the separately typed
    /// external-AI candidate lane by caller assertion.
    pub async fn search_public(
        &self,
        request: airwiki_types::PublicSearchRequest,
    ) -> Result<SearchResponse> {
        request.validate()?;
        let database = self.database.clone();
        let requested = request
            .collections
            .iter()
            .map(|collection| collection.collection_id)
            .collect::<Vec<_>>();
        let collections =
            run_search_blocking("public search scope worker task failed", move || {
                database.publicly_searchable_collections(&requested)
            })
            .await?;
        if collections.is_empty() {
            bail!("no requested collection is publicly accessible");
        }
        let local_request = SearchRequest {
            protocol_version: airwiki_types::SEARCH_PROTOCOL.to_owned(),
            request_id: request.request_id,
            query: request.query,
            purpose: SearchPurpose::LocalAssistant,
            top_k: request.top_k,
        };
        let mut response = self.search_collections(local_request, &collections).await?;
        let database = self.database.clone();
        let hits = std::mem::take(&mut response.hits);
        response.hits =
            run_search_blocking("public search revalidation worker task failed", move || {
                revalidate_public_hits(database, hits)
            })
            .await?;
        response.authorized_candidates.clear();
        Ok(response)
    }

    pub async fn search_collections(
        &self,
        request: SearchRequest,
        collections: &[Uuid],
    ) -> Result<SearchResponse> {
        request.validate()?;
        let query_embedding = self
            .embeddings
            .embed(&[format!("query: {}", request.query.trim())])
            .await
            .context("could not embed search query")?
            .into_iter()
            .next()
            .context("embedding provider returned no query vector")?;
        if query_embedding.len() != crate::EMBEDDING_DIMENSIONS {
            bail!(
                "embedding provider returned {} dimensions; expected {}",
                query_embedding.len(),
                crate::EMBEDDING_DIMENSIONS
            );
        }

        let database = self.database.clone();
        let query = request.query.clone();
        // Public callers may provide the same scope more than once. SQL `IN`
        // historically collapsed those duplicates; preserve that contract now
        // that vector scanning visits one collection at a time.
        let mut seen_collections = HashSet::new();
        let collections = collections
            .iter()
            .copied()
            .filter(|collection_id| seen_collections.insert(*collection_id))
            .collect::<Vec<_>>();
        let purpose = request.purpose;
        let prepared = run_search_blocking("hybrid retrieval worker task failed", move || {
            prepare_candidates(database, query, collections, purpose, query_embedding)
        })
        .await?;
        let PreparedCandidates {
            candidates: deduplicated_candidates,
            visible_snippets,
            relevance_inputs,
            candidate_snapshot_changed,
        } = prepared;
        let decisions = if relevance_inputs.is_empty() {
            Vec::new()
        } else {
            self.relevance
                .classify(request.query.trim(), &relevance_inputs)
                .await?
        };
        if decisions.len() != deduplicated_candidates.len() {
            return Err(EvidenceRelevanceError::DecisionCountMismatch {
                expected: deduplicated_candidates.len(),
                actual: decisions.len(),
            }
            .into());
        }

        let mut hits = Vec::new();
        let mut authorized_candidates = Vec::new();
        for ((candidate, snippet), decision) in deduplicated_candidates
            .into_iter()
            .zip(visible_snippets)
            .zip(decisions)
        {
            let chunk_id = public_chunk_id(
                &candidate.source_sha256,
                candidate.chunk.ordinal,
                &candidate.chunk.text_sha256,
            );
            let mut hit = SearchHit {
                concept_id: candidate.chunk.concept_id,
                collection_id: candidate.chunk.collection_id,
                chunk_id,
                title: candidate.title,
                snippet,
                heading_or_page: candidate.chunk.heading_or_page,
                logical_resource_uri: candidate.logical_resource_uri,
                source_revision: candidate.chunk.source_revision,
                source_sha256: candidate.source_sha256,
                updated_at: candidate.updated_at,
                rank: 0,
                node_id: self.node_id.clone(),
            };
            hit.sanitize_for_wire();
            let destination = match decision {
                EvidenceDecision::Relevant => &mut hits,
                EvidenceDecision::Irrelevant if purpose == SearchPurpose::ExternalAi => {
                    &mut authorized_candidates
                }
                EvidenceDecision::Irrelevant => continue,
            };
            if destination.len() < usize::from(request.top_k) {
                hit.rank = u32::try_from(destination.len() + 1).unwrap_or(u32::MAX);
                destination.push(hit);
            }
            let candidate_lane_complete = purpose != SearchPurpose::ExternalAi
                || authorized_candidates.len() == usize::from(request.top_k);
            if hits.len() == usize::from(request.top_k) && candidate_lane_complete {
                break;
            }
        }
        let before_revalidation = hits.len().saturating_add(authorized_candidates.len());
        let database = self.database.clone();
        let purpose = request.purpose;
        let hits = run_search_blocking("local search revalidation worker task failed", move || {
            revalidate_local_hits(database, hits, purpose)
        })
        .await?;
        let database = self.database.clone();
        let authorized_candidates = run_search_blocking(
            "local candidate revalidation worker task failed",
            move || revalidate_local_hits(database, authorized_candidates, purpose),
        )
        .await?;
        let removed_during_revalidation =
            before_revalidation > hits.len().saturating_add(authorized_candidates.len());
        let mut warnings = Vec::new();
        if candidate_snapshot_changed {
            warnings.push("results changed during candidate hydration".to_owned());
        }
        if removed_during_revalidation {
            warnings.push("results changed during final publication revalidation".to_owned());
        }
        Ok(SearchResponse {
            request_id: request.request_id,
            hits,
            authorized_candidates,
            offline_nodes: Vec::new(),
            partial: !warnings.is_empty(),
            warnings,
        })
    }
}

#[async_trait]
impl FederatedSearch for HybridSearchEngine {
    async fn search(
        &self,
        request: SearchRequest,
    ) -> std::result::Result<SearchResponse, SearchContractError> {
        self.search_local(request)
            .await
            .map_err(|error| SearchContractError::Backend(error.to_string()))
    }
}

#[derive(Debug)]
struct PreparedCandidates {
    candidates: Vec<RankedChunk>,
    visible_snippets: Vec<String>,
    relevance_inputs: Vec<RelevanceInput>,
    candidate_snapshot_changed: bool,
}

async fn run_search_blocking<T>(
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

fn prepare_candidates(
    database: Database,
    query: String,
    collections: Vec<Uuid>,
    purpose: SearchPurpose,
    query_embedding: Vec<f32>,
) -> Result<PreparedCandidates> {
    let lexical = database.lexical_candidates(
        &query,
        &collections,
        purpose,
        PRE_DEDUPLICATION_CANDIDATE_LIMIT,
    )?;
    let mut vector_scores = Vec::with_capacity(PRE_DEDUPLICATION_CANDIDATE_LIMIT * 2);
    for collection_id in &collections {
        let mut after_rowid = None;
        loop {
            let batch = database.vector_embedding_candidates_batch(
                *collection_id,
                purpose,
                VECTOR_SQL_BATCH_SIZE,
                after_rowid,
            )?;
            let batch_len = batch.len();
            after_rowid = batch.last().map(|candidate| candidate.scan_cursor);
            vector_scores.extend(batch.into_iter().map(|candidate| {
                let similarity = cosine_similarity(&query_embedding, &candidate.embedding);
                (candidate.chunk_id, similarity)
            }));
            if vector_scores.len() >= PRE_DEDUPLICATION_CANDIDATE_LIMIT * 2 {
                sort_and_truncate_vector_scores(
                    &mut vector_scores,
                    PRE_DEDUPLICATION_CANDIDATE_LIMIT,
                );
            }
            if batch_len < VECTOR_SQL_BATCH_SIZE {
                break;
            }
        }
    }
    sort_and_truncate_vector_scores(&mut vector_scores, PRE_DEDUPLICATION_CANDIDATE_LIMIT);
    let vector_ids = vector_scores
        .iter()
        .map(|(chunk_id, _)| *chunk_id)
        .collect::<Vec<_>>();
    let mut hydrated_vector = database
        .vector_candidates_by_id(&vector_ids, &collections, purpose)?
        .into_iter()
        .map(|candidate| (candidate.chunk.id, candidate))
        .collect::<HashMap<_, _>>();
    let candidate_snapshot_changed = hydrated_vector.len() != vector_ids.len();
    let vector = vector_scores
        .into_iter()
        .filter_map(|(chunk_id, similarity)| {
            hydrated_vector
                .remove(&chunk_id)
                .map(|candidate| (candidate, similarity))
        })
        .collect::<Vec<_>>();

    let mut candidates = HashMap::<Uuid, RankedChunk>::new();
    let mut scores = HashMap::<Uuid, f64>::new();
    for (index, candidate) in lexical.into_iter().enumerate() {
        // bm25 is used to establish rank; magnitudes never leave this node.
        let _ = candidate.lexical_score;
        *scores.entry(candidate.chunk.id).or_default() += rrf(index);
        candidates.insert(candidate.chunk.id, candidate);
    }
    for (index, (candidate, _similarity)) in vector.into_iter().enumerate() {
        *scores.entry(candidate.chunk.id).or_default() += rrf(index);
        candidates.entry(candidate.chunk.id).or_insert(candidate);
    }
    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|(id_a, score_a), (id_b, score_b)| {
        score_b
            .partial_cmp(score_a)
            .unwrap_or(Ordering::Equal)
            .then_with(|| id_a.cmp(id_b))
    });

    let mut dedup = HashSet::<(String, String)>::new();
    let mut deduplicated_candidates = Vec::new();
    for (chunk_id, _score) in ranked {
        let Some(candidate) = candidates.remove(&chunk_id) else {
            continue;
        };
        if !dedup.insert((
            candidate.source_sha256.clone(),
            candidate.chunk.text_sha256.clone(),
        )) {
            continue;
        }
        deduplicated_candidates.push(candidate);
        if deduplicated_candidates.len() == RELEVANCE_CANDIDATE_LIMIT {
            break;
        }
    }

    let visible_snippets = deduplicated_candidates
        .iter()
        .map(|candidate| relevant_snippet(&candidate.chunk.text, &query))
        .collect::<Vec<_>>();
    let relevance_inputs = deduplicated_candidates
        .iter()
        .zip(&visible_snippets)
        .map(|(candidate, snippet)| RelevanceInput {
            title: candidate.title.clone(),
            // Bound legacy rows too: older databases may predate the
            // extractor-side heading invariant.
            heading: candidate
                .chunk
                .heading_or_page
                .chars()
                .take(MAX_HEADING_OR_PAGE_CHARS)
                .collect(),
            text: snippet.clone(),
        })
        .collect::<Vec<_>>();

    Ok(PreparedCandidates {
        candidates: deduplicated_candidates,
        visible_snippets,
        relevance_inputs,
        candidate_snapshot_changed,
    })
}

fn revalidate_local_hits(
    database: Database,
    hits: Vec<SearchHit>,
    purpose: SearchPurpose,
) -> Result<Vec<SearchHit>> {
    let mut current_hits = Vec::with_capacity(hits.len());
    for hit in hits {
        if database.hit_is_current(&hit, purpose)? {
            current_hits.push(hit);
        }
    }
    renumber_hits(&mut current_hits);
    Ok(current_hits)
}

fn revalidate_peer_hits(
    database: Database,
    hits: Vec<SearchHit>,
    peer_id: String,
    purpose: SearchPurpose,
) -> Result<Vec<SearchHit>> {
    let mut current_hits = Vec::with_capacity(hits.len());
    for hit in hits {
        if database.peer_hit_is_current(&hit, &peer_id, purpose)? {
            current_hits.push(hit);
        }
    }
    renumber_hits(&mut current_hits);
    Ok(current_hits)
}

fn revalidate_public_hits(database: Database, hits: Vec<SearchHit>) -> Result<Vec<SearchHit>> {
    let mut current_hits = Vec::with_capacity(hits.len());
    for hit in hits {
        if database.public_hit_is_current(&hit)? {
            current_hits.push(hit);
        }
    }
    renumber_hits(&mut current_hits);
    Ok(current_hits)
}

fn renumber_hits(hits: &mut [SearchHit]) {
    for (index, hit) in hits.iter_mut().enumerate() {
        hit.rank = u32::try_from(index + 1).unwrap_or(u32::MAX);
    }
}

fn rrf(zero_based_rank: usize) -> f64 {
    1.0 / (RRF_K + zero_based_rank as f64 + 1.0)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return -1.0;
    }
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn sort_and_truncate_vector_scores(candidates: &mut Vec<(Uuid, f32)>, limit: usize) {
    candidates.sort_by(|(left_id, left), (right_id, right)| {
        right
            .partial_cmp(left)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left_id.cmp(right_id))
    });
    candidates.truncate(limit);
}

fn normalized_terms(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3)
        .map(str::to_owned)
        .collect()
}

fn relevant_snippet(text: &str, query: &str) -> String {
    let query_words = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.len() >= 3)
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    let lowercase = text.to_lowercase();
    let byte_start = query_words
        .iter()
        .filter_map(|word| lowercase.find(word))
        .min()
        .unwrap_or(0);
    let center_char = lowercase[..byte_start].chars().count();
    let total_chars = text.chars().count();
    let start_char = center_char.saturating_sub(MAX_SNIPPET_CHARS / 4);
    let end_char = (start_char + MAX_SNIPPET_CHARS).min(total_chars);
    let mut snippet = text
        .chars()
        .skip(start_char)
        .take(end_char - start_char)
        .collect::<String>();
    if start_char > 0 {
        snippet.insert(0, '…');
    }
    if end_char < total_chars {
        snippet.push('…');
    }
    snippet.chars().take(MAX_SNIPPET_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    use airwiki_types::{
        CollectionPolicy, ConceptType, DEFAULT_TOP_K, EnrichmentDraft, PUBLIC_SEARCH_PROTOCOL,
        PublicSearchRequest, SearchPurpose,
    };
    use chrono::Utc;

    use super::*;
    use crate::inference::{DeterministicEmbeddingProvider, EmbeddingProvider};
    use crate::storage::{PeerRecord, StoredChunk};

    #[derive(Debug, Clone)]
    struct FixedEvidenceRelevanceProvider {
        result: std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError>,
    }

    #[async_trait]
    impl EvidenceRelevanceProvider for FixedEvidenceRelevanceProvider {
        fn profile_id(&self) -> &str {
            "fixed-evidence-relevance-test-double"
        }

        async fn classify(
            &self,
            _question: &str,
            _candidates: &[RelevanceInput],
        ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
            self.result.clone()
        }
    }

    #[derive(Debug, Clone)]
    struct WithdrawsDuringRelevance {
        database: Database,
        source_document_id: Uuid,
        decision: EvidenceDecision,
    }

    #[derive(Debug, Clone, Default)]
    struct MarkerSensitiveEvidenceRelevanceProvider;

    #[async_trait]
    impl EvidenceRelevanceProvider for MarkerSensitiveEvidenceRelevanceProvider {
        fn profile_id(&self) -> &str {
            "marker-sensitive-evidence-relevance-test-double"
        }

        async fn classify(
            &self,
            _question: &str,
            candidates: &[RelevanceInput],
        ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
            Ok(candidates
                .iter()
                .map(|candidate| {
                    if candidate.text.contains("OUTSIDE_VISIBLE_SNIPPET") {
                        EvidenceDecision::Relevant
                    } else {
                        EvidenceDecision::Irrelevant
                    }
                })
                .collect())
        }
    }

    #[derive(Debug, Clone, Default)]
    struct CountingIrrelevantEvidenceRelevanceProvider {
        candidates_seen: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl EvidenceRelevanceProvider for CountingIrrelevantEvidenceRelevanceProvider {
        fn profile_id(&self) -> &str {
            "counting-irrelevant-evidence-relevance-test-double"
        }

        async fn classify(
            &self,
            _question: &str,
            candidates: &[RelevanceInput],
        ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
            self.candidates_seen
                .store(candidates.len(), AtomicOrdering::SeqCst);
            Ok(vec![EvidenceDecision::Irrelevant; candidates.len()])
        }
    }

    #[derive(Debug, Clone, Default)]
    struct AllRelevantEvidenceRelevanceProvider;

    #[async_trait]
    impl EvidenceRelevanceProvider for AllRelevantEvidenceRelevanceProvider {
        fn profile_id(&self) -> &str {
            "all-relevant-evidence-relevance-test-double"
        }

        async fn classify(
            &self,
            _question: &str,
            candidates: &[RelevanceInput],
        ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
            Ok(vec![EvidenceDecision::Relevant; candidates.len()])
        }
    }

    #[async_trait]
    impl EvidenceRelevanceProvider for WithdrawsDuringRelevance {
        fn profile_id(&self) -> &str {
            "withdraws-during-relevance-test-double"
        }

        async fn classify(
            &self,
            _question: &str,
            candidates: &[RelevanceInput],
        ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
            self.database
                .mark_deleted(self.source_document_id)
                .map_err(|_| EvidenceRelevanceError::InferenceFailed)?;
            Ok(vec![self.decision; candidates.len()])
        }
    }

    async fn indexed_database() -> (Database, Uuid, Uuid) {
        let db = Database::in_memory().unwrap();
        let collection = db
            .create_collection(
                "Runbooks",
                PathBuf::from("/tmp/source-search-test"),
                PathBuf::from("/tmp/wiki-search-test"),
                CollectionPolicy {
                    local_only: false,
                    peer_shareable: true,
                    allow_external_ai: false,
                    internet_public: false,
                },
            )
            .unwrap();
        let source = db
            .register_source(
                collection.id,
                "/tmp/source-search-test/payments.md",
                &"a".repeat(64),
                "markdown",
                100,
            )
            .unwrap();
        db.mark_extracted(source.id(), 0, 100).unwrap();
        let draft = EnrichmentDraft {
            concept_type: ConceptType::Runbook,
            title: "Recuperación de pagos".into(),
            description: "Restaurar el procesador".into(),
            language: "es".into(),
            tags: vec!["pagos".into()],
            entities: vec![],
            links: vec![],
            summary: "Reiniciar la cola".into(),
            classification_confidence: 1.0,
            classification_explanation: "fixture".into(),
        };
        let concept = db
            .save_enrichment(source.id(), draft.clone(), "mac", "fake")
            .unwrap();
        let embedding_provider = DeterministicEmbeddingProvider;
        let embedding = embedding_provider
            .embed(&["passage: reiniciar cola de pagos y validar API".into()])
            .await
            .unwrap()
            .remove(0);
        db.replace_chunks(
            concept.id,
            &[StoredChunk {
                id: Uuid::new_v4(),
                concept_id: concept.id,
                source_document_id: source.id(),
                collection_id: collection.id,
                ordinal: 0,
                heading_or_page: "Pasos".into(),
                text: "Reiniciar cola de pagos y validar API".into(),
                text_sha256: "text-hash".into(),
                embedding,
                source_revision: 1,
            }],
        )
        .unwrap();
        db.approve_concept(concept.id, draft).unwrap();
        (db, collection.id, concept.id)
    }

    fn allow_external_ai(database: &Database, collection_id: Uuid) {
        database
            .update_collection_policy(
                collection_id,
                CollectionPolicy {
                    local_only: false,
                    peer_shareable: true,
                    allow_external_ai: true,
                    internet_public: false,
                },
            )
            .unwrap();
    }

    async fn replace_with_ranked_fixture_chunks(database: &Database, concept_id: Uuid) {
        let template = database.chunks_for_concept(concept_id).unwrap().remove(0);
        let passages = [
            ("Primero", "Pagos: primera evidencia operativa"),
            ("Segundo", "Pagos: segunda evidencia operativa"),
            ("Tercero", "Pagos: tercera evidencia operativa"),
        ];
        let embedding_inputs = passages
            .iter()
            .map(|(_, text)| format!("passage: {text}"))
            .collect::<Vec<_>>();
        let embeddings = DeterministicEmbeddingProvider
            .embed(&embedding_inputs)
            .await
            .unwrap();
        let chunks = passages
            .into_iter()
            .zip(embeddings)
            .enumerate()
            .map(|(index, ((heading, text), embedding))| StoredChunk {
                id: Uuid::new_v4(),
                concept_id: template.concept_id,
                source_document_id: template.source_document_id,
                collection_id: template.collection_id,
                ordinal: u32::try_from(index).unwrap(),
                heading_or_page: heading.to_owned(),
                text: text.to_owned(),
                text_sha256: format!("text-hash-{index}"),
                embedding,
                source_revision: template.source_revision,
            })
            .collect::<Vec<_>>();
        database.replace_chunks(concept_id, &chunks).unwrap();
    }

    async fn replace_with_disjoint_lexical_and_vector_candidates(
        database: &Database,
        concept_id: Uuid,
    ) {
        let template = database.chunks_for_concept(concept_id).unwrap().remove(0);
        let query_embedding = DeterministicEmbeddingProvider
            .embed(&["query: lexicalneedle".to_owned()])
            .await
            .unwrap()
            .remove(0);
        let opposite_embedding = query_embedding
            .iter()
            .map(|value| -*value)
            .collect::<Vec<_>>();
        let mut chunks = Vec::new();
        for index in 0..8_u32 {
            chunks.push(StoredChunk {
                id: Uuid::new_v4(),
                concept_id: template.concept_id,
                source_document_id: template.source_document_id,
                collection_id: template.collection_id,
                ordinal: index,
                heading_or_page: format!("Lexical {index}"),
                text: format!("lexicalneedle evidencia {index}"),
                text_sha256: format!("lexical-{index}"),
                embedding: opposite_embedding.clone(),
                source_revision: template.source_revision,
            });
        }
        for index in 0..8_u32 {
            chunks.push(StoredChunk {
                id: Uuid::new_v4(),
                concept_id: template.concept_id,
                source_document_id: template.source_document_id,
                collection_id: template.collection_id,
                ordinal: index + 8,
                heading_or_page: format!("Vector {index}"),
                text: format!("tema neutral {index}"),
                text_sha256: format!("vector-{index}"),
                embedding: query_embedding.clone(),
                source_revision: template.source_revision,
            });
        }
        database.replace_chunks(concept_id, &chunks).unwrap();
    }

    async fn replace_with_multi_page_vector_candidates(database: &Database, concept_id: Uuid) {
        let template = database.chunks_for_concept(concept_id).unwrap().remove(0);
        let query_embedding = DeterministicEmbeddingProvider
            .embed(&["query: deepneedle".to_owned()])
            .await
            .unwrap()
            .remove(0);
        let opposite_embedding = query_embedding
            .iter()
            .map(|value| -*value)
            .collect::<Vec<_>>();
        let candidate_count = VECTOR_SQL_BATCH_SIZE * 2 + 1;
        let chunks = (0..candidate_count)
            .map(|index| {
                let is_target = index + 1 == candidate_count;
                StoredChunk {
                    id: Uuid::from_u128(u128::try_from(index + 1).unwrap()),
                    concept_id: template.concept_id,
                    source_document_id: template.source_document_id,
                    collection_id: template.collection_id,
                    ordinal: u32::try_from(index).unwrap(),
                    heading_or_page: if is_target {
                        "Deep page target".to_owned()
                    } else {
                        format!("Noise {index}")
                    },
                    text: if is_target {
                        "The unique vector target".to_owned()
                    } else {
                        format!("unrelated vector evidence {index}")
                    },
                    text_sha256: format!("multi-page-vector-{index}"),
                    embedding: if is_target {
                        query_embedding.clone()
                    } else {
                        opposite_embedding.clone()
                    },
                    source_revision: template.source_revision,
                }
            })
            .collect::<Vec<_>>();
        database.replace_chunks(concept_id, &chunks).unwrap();
    }

    async fn replace_with_duplicate_heavy_candidates(database: &Database, concept_id: Uuid) {
        let template = database.chunks_for_concept(concept_id).unwrap().remove(0);
        let query_embedding = DeterministicEmbeddingProvider
            .embed(&["query: lexicalneedle".to_owned()])
            .await
            .unwrap()
            .remove(0);
        let opposite_embedding = query_embedding
            .iter()
            .map(|value| -*value)
            .collect::<Vec<_>>();
        let duplicate_count = RELEVANCE_CANDIDATE_LIMIT + 2;
        let mut chunks = Vec::new();
        for index in 0..duplicate_count {
            chunks.push(StoredChunk {
                id: Uuid::new_v4(),
                concept_id: template.concept_id,
                source_document_id: template.source_document_id,
                collection_id: template.collection_id,
                ordinal: u32::try_from(index).unwrap(),
                heading_or_page: "Duplicate".to_owned(),
                text: "lexicalneedle".to_owned(),
                text_sha256: "duplicate-content".to_owned(),
                embedding: query_embedding.clone(),
                source_revision: template.source_revision,
            });
        }
        for index in 0..RELEVANCE_CANDIDATE_LIMIT {
            chunks.push(StoredChunk {
                id: Uuid::new_v4(),
                concept_id: template.concept_id,
                source_document_id: template.source_document_id,
                collection_id: template.collection_id,
                ordinal: u32::try_from(duplicate_count + index).unwrap(),
                heading_or_page: format!("Unique {index}"),
                text: format!(
                    "lexicalneedle unique evidence {index} {}",
                    "filler ".repeat(64)
                ),
                text_sha256: format!("unique-content-{index}"),
                embedding: opposite_embedding.clone(),
                source_revision: template.source_revision,
            });
        }
        database.replace_chunks(concept_id, &chunks).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_search_phase_yields_the_tokio_runtime() {
        let progress = Arc::new((Mutex::new(false), Condvar::new()));
        let blocking_progress = Arc::clone(&progress);
        let blocking_phase = run_search_blocking("blocking search test task failed", move || {
            let (lock, condition) = &*blocking_progress;
            let progressed = lock.lock().unwrap();
            let (progressed, _) = condition
                .wait_timeout_while(progressed, Duration::from_millis(250), |value| !*value)
                .unwrap();
            if !*progressed {
                bail!("Tokio runtime did not progress while search work was blocking");
            }
            Ok(())
        });
        let runtime_progress = async {
            tokio::task::yield_now().await;
            let (lock, condition) = &*progress;
            *lock.lock().unwrap() = true;
            condition.notify_one();
        };

        let (blocking_result, ()) = tokio::join!(blocking_phase, runtime_progress);

        blocking_result.unwrap();
    }

    #[tokio::test]
    async fn hybrid_search_returns_citable_evidence() {
        let (db, _collection_id, _concept_id) = indexed_database().await;
        let engine = HybridSearchEngine::new(
            db,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(DeterministicEvidenceRelevanceProvider),
            "mac-node",
        );
        let request = SearchRequest::new(
            "¿cómo recuperar pagos?",
            SearchPurpose::LocalAssistant,
            DEFAULT_TOP_K,
        );
        let response = engine.search_local(request).await.unwrap();
        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].node_id, "mac-node");
        assert_eq!(response.hits[0].heading_or_page, "Pasos");
        assert_eq!(response.hits[0].source_revision, 1);
    }

    #[tokio::test]
    async fn public_search_requires_live_public_policy_but_no_peer_grant() {
        let (db, collection_id, _concept_id) = indexed_database().await;
        let engine = HybridSearchEngine::new(
            db.clone(),
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(DeterministicEvidenceRelevanceProvider),
            "publisher",
        );
        let request = || PublicSearchRequest {
            protocol_version: PUBLIC_SEARCH_PROTOCOL.to_owned(),
            request_id: Uuid::new_v4(),
            query: "recuperar pagos".to_owned(),
            purpose: SearchPurpose::LocalAssistant,
            collections: vec![airwiki_types::PublicCollectionTarget {
                collection_id,
                manifest_sequence: 1,
                publication_fingerprint: "a".repeat(64),
            }],
            top_k: 5,
        };

        assert!(engine.search_public(request()).await.is_err());
        db.update_collection_policy(
            collection_id,
            CollectionPolicy {
                local_only: false,
                peer_shareable: false,
                allow_external_ai: false,
                internet_public: true,
            },
        )
        .unwrap();
        assert_eq!(engine.search_public(request()).await.unwrap().hits.len(), 1);

        db.update_collection_policy(collection_id, CollectionPolicy::local_only())
            .unwrap();
        assert!(engine.search_public(request()).await.is_err());
    }

    #[tokio::test]
    async fn search_exposes_content_stable_chunk_identity() {
        let (db, _collection_id, _concept_id) = indexed_database().await;
        let engine = HybridSearchEngine::new(
            db,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(DeterministicEvidenceRelevanceProvider),
            "mac-node",
        );
        let response = engine
            .search_local(SearchRequest::new(
                "¿cómo recuperar pagos?",
                SearchPurpose::LocalAssistant,
                1,
            ))
            .await
            .unwrap();

        assert_eq!(
            response.hits[0].chunk_id,
            public_chunk_id(&"a".repeat(64), 0, "text-hash")
        );
    }

    #[tokio::test]
    async fn peer_grant_and_external_ai_policy_are_both_enforced() {
        let (db, collection_id, _concept_id) = indexed_database().await;
        db.upsert_peer(&PeerRecord {
            peer_id: "windows".into(),
            display_name: None,
            trusted: true,
            blocked: false,
            paired_at: Some(Utc::now()),
            last_seen_at: None,
        })
        .unwrap();
        let engine = HybridSearchEngine::new(
            db.clone(),
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(DeterministicEvidenceRelevanceProvider),
            "mac",
        );
        let request = SearchRequest::new("pagos", SearchPurpose::LocalAssistant, 5);
        assert!(
            engine
                .search_for_peer(request.clone(), "windows")
                .await
                .unwrap()
                .hits
                .is_empty()
        );
        db.set_grant("windows", collection_id, true).unwrap();
        assert_eq!(
            engine
                .search_for_peer(request, "windows")
                .await
                .unwrap()
                .hits
                .len(),
            1
        );
        let external = SearchRequest::new("pagos", SearchPurpose::ExternalAi, 5);
        assert!(
            engine
                .search_for_peer(external, "windows")
                .await
                .unwrap()
                .hits
                .is_empty()
        );
        db.update_collection_policy(
            collection_id,
            CollectionPolicy {
                local_only: false,
                peer_shareable: true,
                allow_external_ai: true,
                internet_public: false,
            },
        )
        .unwrap();
        let external = SearchRequest::new("pagos", SearchPurpose::ExternalAi, 5);
        assert_eq!(
            engine
                .search_for_peer(external, "windows")
                .await
                .unwrap()
                .hits
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn local_external_ai_access_does_not_require_peer_sharing() {
        let (db, collection_id, _concept_id) = indexed_database().await;
        db.update_collection_policy(
            collection_id,
            CollectionPolicy {
                local_only: true,
                peer_shareable: false,
                allow_external_ai: true,
                internet_public: false,
            },
        )
        .unwrap();
        db.upsert_peer(&PeerRecord {
            peer_id: "windows".into(),
            display_name: None,
            trusted: true,
            blocked: false,
            paired_at: Some(Utc::now()),
            last_seen_at: None,
        })
        .unwrap();
        db.set_grant("windows", collection_id, true).unwrap();
        let engine = HybridSearchEngine::new(
            db,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(DeterministicEvidenceRelevanceProvider),
            "mac",
        );

        let local = engine
            .search_local(SearchRequest::new("pagos", SearchPurpose::ExternalAi, 5))
            .await
            .unwrap();
        assert_eq!(local.hits.len(), 1);

        let remote = engine
            .search_for_peer(
                SearchRequest::new("pagos", SearchPurpose::ExternalAi, 5),
                "windows",
            )
            .await
            .unwrap();
        assert!(remote.hits.is_empty());
        assert!(remote.authorized_candidates.is_empty());
    }

    #[tokio::test]
    async fn external_ai_policy_blocks_rejected_candidates_before_disclosure() {
        let (database, _collection_id, _concept_id) = indexed_database().await;
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(DeterministicEvidenceRelevanceProvider),
            "mac",
        );

        let response = engine
            .search_local(SearchRequest::new(
                "presupuesto anual",
                SearchPurpose::ExternalAi,
                5,
            ))
            .await
            .unwrap();

        assert!(response.hits.is_empty());
        assert!(response.authorized_candidates.is_empty());
    }

    #[tokio::test]
    async fn ranked_hit_is_rejected_after_policy_or_publication_changes() {
        let (db, _collection_id, _concept_id) = indexed_database().await;
        let engine = HybridSearchEngine::new(
            db.clone(),
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(DeterministicEvidenceRelevanceProvider),
            "mac",
        );
        let response = engine
            .search_local(SearchRequest::new(
                "pagos",
                SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap();
        let hit = response.hits.first().unwrap();
        assert!(
            db.hit_is_current(hit, SearchPurpose::LocalAssistant)
                .unwrap()
        );
        assert!(!db.hit_is_current(hit, SearchPurpose::ExternalAi).unwrap());

        let concept = db.concept(hit.concept_id).unwrap().unwrap();
        db.mark_deleted(concept.source_document_id).unwrap();
        assert!(
            !db.hit_is_current(hit, SearchPurpose::LocalAssistant)
                .unwrap()
        );
    }

    #[tokio::test]
    async fn irrelevant_candidates_remain_authorized_but_separate_from_evidence() {
        let (database, collection_id, _concept_id) = indexed_database().await;
        allow_external_ai(&database, collection_id);
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(DeterministicEvidenceRelevanceProvider),
            "mac",
        );

        let response = engine
            .search_local(SearchRequest::new(
                "presupuesto anual",
                SearchPurpose::ExternalAi,
                5,
            ))
            .await
            .unwrap();

        assert!(response.hits.is_empty());
        assert_eq!(response.authorized_candidates.len(), 1);
        assert_eq!(response.authorized_candidates[0].rank, 1);
        assert!(!response.partial);
        assert!(response.warnings.is_empty());
    }

    #[tokio::test]
    async fn relevance_gate_classifies_only_the_exact_visible_snippet() {
        let (database, collection_id, concept_id) = indexed_database().await;
        allow_external_ai(&database, collection_id);
        let template = database.chunks_for_concept(concept_id).unwrap().remove(0);
        let text = format!(
            "Pagos al inicio. {} OUTSIDE_VISIBLE_SNIPPET",
            "contenido de relleno ".repeat(100)
        );
        assert!(text.chars().count() > MAX_SNIPPET_CHARS);
        assert!(text.contains("OUTSIDE_VISIBLE_SNIPPET"));
        let embedding = DeterministicEmbeddingProvider
            .embed(&[format!("passage: {text}")])
            .await
            .unwrap()
            .remove(0);
        database
            .replace_chunks(
                concept_id,
                &[StoredChunk {
                    id: Uuid::new_v4(),
                    concept_id: template.concept_id,
                    source_document_id: template.source_document_id,
                    collection_id: template.collection_id,
                    ordinal: 0,
                    heading_or_page: "Pasos".into(),
                    text,
                    text_sha256: "long-visible-snippet-fixture".into(),
                    embedding,
                    source_revision: template.source_revision,
                }],
            )
            .unwrap();
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(MarkerSensitiveEvidenceRelevanceProvider),
            "mac",
        );

        let response = engine
            .search_local(SearchRequest::new("pagos", SearchPurpose::ExternalAi, 1))
            .await
            .unwrap();

        assert!(response.hits.is_empty());
        assert_eq!(response.authorized_candidates.len(), 1);
    }

    #[tokio::test]
    async fn local_assistant_does_not_receive_irrelevant_candidates() {
        let (database, _collection_id, _concept_id) = indexed_database().await;
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(DeterministicEvidenceRelevanceProvider),
            "mac",
        );

        let response = engine
            .search_local(SearchRequest::new(
                "presupuesto anual",
                SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap();

        assert!(response.hits.is_empty());
        assert!(response.authorized_candidates.is_empty());
    }

    #[tokio::test]
    async fn relevance_candidate_batch_is_limited_after_rrf_deduplication() {
        let (database, _collection_id, concept_id) = indexed_database().await;
        replace_with_disjoint_lexical_and_vector_candidates(&database, concept_id).await;
        let relevance = Arc::new(CountingIrrelevantEvidenceRelevanceProvider::default());
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            relevance.clone(),
            "mac",
        );

        for top_k in airwiki_types::MIN_TOP_K..=airwiki_types::MAX_TOP_K {
            let response = engine
                .search_local(SearchRequest::new(
                    "lexicalneedle",
                    SearchPurpose::LocalAssistant,
                    top_k,
                ))
                .await
                .unwrap();

            assert!(response.hits.is_empty());
            assert_eq!(
                relevance.candidates_seen.load(AtomicOrdering::SeqCst),
                RELEVANCE_CANDIDATE_LIMIT
            );
        }
    }

    #[tokio::test]
    async fn vector_scan_finds_a_candidate_after_multiple_sql_pages() {
        let (database, _collection_id, concept_id) = indexed_database().await;
        replace_with_multi_page_vector_candidates(&database, concept_id).await;
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(AllRelevantEvidenceRelevanceProvider),
            "mac",
        );

        let response = engine
            .search_local(SearchRequest::new(
                "deepneedle",
                SearchPurpose::LocalAssistant,
                1,
            ))
            .await
            .unwrap();

        assert_eq!(response.hits.len(), 1);
        assert_eq!(response.hits[0].heading_or_page, "Deep page target");
    }

    #[tokio::test]
    async fn duplicate_collection_scope_preserves_vector_ranking() {
        let (database, collection_id, concept_id) = indexed_database().await;
        replace_with_multi_page_vector_candidates(&database, concept_id).await;
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(AllRelevantEvidenceRelevanceProvider),
            "mac",
        );

        let unique = engine
            .search_collections(
                SearchRequest::new("deepneedle", SearchPurpose::LocalAssistant, 10),
                &[collection_id],
            )
            .await
            .unwrap();
        let duplicated = engine
            .search_collections(
                SearchRequest::new("deepneedle", SearchPurpose::LocalAssistant, 10),
                &[collection_id; 5],
            )
            .await
            .unwrap();

        let identity = |response: &SearchResponse| {
            response
                .hits
                .iter()
                .map(|hit| (hit.chunk_id, hit.rank, hit.heading_or_page.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(identity(&duplicated), identity(&unique));
        assert_eq!(duplicated.warnings, unique.warnings);
        assert_eq!(duplicated.partial, unique.partial);
    }

    #[tokio::test]
    async fn pre_deduplication_over_retrieval_fills_the_fixed_relevance_batch() {
        let (database, _collection_id, concept_id) = indexed_database().await;
        replace_with_duplicate_heavy_candidates(&database, concept_id).await;
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(AllRelevantEvidenceRelevanceProvider),
            "mac",
        );

        let response = engine
            .search_local(SearchRequest::new(
                "lexicalneedle",
                SearchPurpose::LocalAssistant,
                airwiki_types::MAX_TOP_K,
            ))
            .await
            .unwrap();

        assert_eq!(response.hits.len(), RELEVANCE_CANDIDATE_LIMIT);
        assert_eq!(
            response
                .hits
                .iter()
                .filter(|hit| hit.snippet.contains("unique evidence"))
                .count(),
            RELEVANCE_CANDIDATE_LIMIT - 1
        );
    }

    #[test]
    fn relevance_candidate_limit_can_still_fill_the_largest_response() {
        assert_eq!(
            RELEVANCE_CANDIDATE_LIMIT,
            usize::from(airwiki_types::MAX_TOP_K)
        );
    }

    #[tokio::test]
    async fn top_k_only_truncates_a_fixed_answerability_ranking() {
        let (database, _collection_id, concept_id) = indexed_database().await;
        replace_with_disjoint_lexical_and_vector_candidates(&database, concept_id).await;
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(AllRelevantEvidenceRelevanceProvider),
            "mac",
        );

        let top_one = engine
            .search_local(SearchRequest::new(
                "lexicalneedle",
                SearchPurpose::LocalAssistant,
                1,
            ))
            .await
            .unwrap();
        let top_ten = engine
            .search_local(SearchRequest::new(
                "lexicalneedle",
                SearchPurpose::LocalAssistant,
                10,
            ))
            .await
            .unwrap();

        assert_eq!(top_one.hits.len(), 1);
        assert_eq!(top_ten.hits.len(), RELEVANCE_CANDIDATE_LIMIT);
        assert_eq!(top_one.hits[0].chunk_id, top_ten.hits[0].chunk_id);
    }

    #[tokio::test]
    async fn relevance_provider_failure_is_not_reported_as_absence() {
        let (database, _collection_id, _concept_id) = indexed_database().await;
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(FixedEvidenceRelevanceProvider {
                result: Err(EvidenceRelevanceError::Unavailable),
            }),
            "mac",
        );

        let error = engine
            .search_local(SearchRequest::new(
                "pagos",
                SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap_err();

        assert_eq!(
            error.downcast_ref::<EvidenceRelevanceError>(),
            Some(&EvidenceRelevanceError::Unavailable)
        );
    }

    #[tokio::test]
    async fn wrong_relevance_decision_count_is_an_error() {
        let (database, _collection_id, _concept_id) = indexed_database().await;
        let engine = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(FixedEvidenceRelevanceProvider {
                result: Ok(Vec::new()),
            }),
            "mac",
        );

        let error = engine
            .search_local(SearchRequest::new(
                "pagos",
                SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap_err();

        assert_eq!(
            error.downcast_ref::<EvidenceRelevanceError>(),
            Some(&EvidenceRelevanceError::DecisionCountMismatch {
                expected: 1,
                actual: 0,
            })
        );
    }

    #[tokio::test]
    async fn relevance_filter_preserves_rrf_order_and_renumbers_hits() {
        let (database, collection_id, concept_id) = indexed_database().await;
        replace_with_ranked_fixture_chunks(&database, concept_id).await;
        let baseline = HybridSearchEngine::new(
            database.clone(),
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(DeterministicEvidenceRelevanceProvider),
            "mac",
        )
        .search_local(SearchRequest::new(
            "pagos",
            SearchPurpose::LocalAssistant,
            3,
        ))
        .await
        .unwrap();
        allow_external_ai(&database, collection_id);
        let filtered = HybridSearchEngine::new(
            database,
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(FixedEvidenceRelevanceProvider {
                result: Ok(vec![
                    EvidenceDecision::Irrelevant,
                    EvidenceDecision::Relevant,
                    EvidenceDecision::Relevant,
                ]),
            }),
            "mac",
        )
        .search_local(SearchRequest::new("pagos", SearchPurpose::ExternalAi, 3))
        .await
        .unwrap();

        assert_eq!(baseline.hits.len(), 3);
        assert_eq!(
            filtered
                .hits
                .iter()
                .map(|hit| hit.heading_or_page.as_str())
                .collect::<Vec<_>>(),
            baseline
                .hits
                .iter()
                .skip(1)
                .map(|hit| hit.heading_or_page.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            filtered.hits.iter().map(|hit| hit.rank).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(filtered.authorized_candidates.len(), 1);
        assert_eq!(filtered.authorized_candidates[0].rank, 1);
    }

    #[tokio::test]
    async fn publication_is_revalidated_after_relevance_classification() {
        let (database, _collection_id, concept_id) = indexed_database().await;
        let source_document_id = database
            .concept(concept_id)
            .unwrap()
            .unwrap()
            .source_document_id;
        let engine = HybridSearchEngine::new(
            database.clone(),
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(WithdrawsDuringRelevance {
                database,
                source_document_id,
                decision: EvidenceDecision::Relevant,
            }),
            "mac",
        );

        let response = engine
            .search_local(SearchRequest::new(
                "pagos",
                SearchPurpose::LocalAssistant,
                5,
            ))
            .await
            .unwrap();

        assert!(response.hits.is_empty());
        assert!(response.authorized_candidates.is_empty());
        assert!(response.partial);
        assert_eq!(response.warnings.len(), 1);
    }

    #[tokio::test]
    async fn candidate_publication_is_revalidated_after_relevance_classification() {
        let (database, collection_id, concept_id) = indexed_database().await;
        allow_external_ai(&database, collection_id);
        let source_document_id = database
            .concept(concept_id)
            .unwrap()
            .unwrap()
            .source_document_id;
        let engine = HybridSearchEngine::new(
            database.clone(),
            Arc::new(DeterministicEmbeddingProvider),
            Arc::new(WithdrawsDuringRelevance {
                database,
                source_document_id,
                decision: EvidenceDecision::Irrelevant,
            }),
            "mac",
        );

        let response = engine
            .search_local(SearchRequest::new("pagos", SearchPurpose::ExternalAi, 5))
            .await
            .unwrap();

        assert!(response.hits.is_empty());
        assert!(response.authorized_candidates.is_empty());
        assert!(response.partial);
        assert_eq!(response.warnings.len(), 1);
    }

    #[test]
    fn snippets_respect_unicode_character_limit() {
        let text = "á".repeat(MAX_SNIPPET_CHARS + 100);
        let snippet = relevant_snippet(&text, "nada");
        assert!(snippet.chars().count() <= MAX_SNIPPET_CHARS);
    }

    #[test]
    fn snippets_handle_unicode_lowercase_that_changes_utf8_byte_length() {
        let snippet = relevant_snippet("İ área de pagos", "área");

        assert!(snippet.contains("área"));
    }
}
