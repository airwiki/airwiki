use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use airwiki_core::{
    Database, E5_MODEL_REVISION, EMBEDDING_DIMENSIONS, EmbeddingProvider, EvidenceDecision,
    EvidenceRelevanceError, EvidenceRelevanceProvider, FastEmbedE5Small, FastEmbedMmarcoReranker,
    HybridSearchEngine, MMARCO_RERANKER_REVISION, OkfPublicationMaterializer, PinnedE5Snapshot,
    PinnedMmarcoRerankerSnapshot, RelevanceInput, StoredChunk,
};
use airwiki_inference::{E5_REVISION, MMARCO_REVISION, platform_relevance_model};
use airwiki_types::{
    CollectionPolicy, ConceptType, EnrichmentDraft, MAX_HEADING_OR_PAGE_CHARS, MAX_SNIPPET_CHARS,
    MAX_TOP_K, SearchHit, SearchPurpose, SearchRequest,
};
use anyhow::{Context, Result, anyhow, ensure};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{replace_file, workspace_root};

const FIXTURE_PATH: &str = "fixtures/retrieval/search-quality-v3.json";
const REPORT_DIRECTORY: &str = "target/evals";
const REPORT_SCHEMA_VERSION: u32 = 4;
const FIXTURE_SCHEMA_VERSION: u32 = 3;
const TOP_K: u8 = 5;
const MIN_RECALL_AT_FIVE: f64 = 0.9;
const ORIGIN_NODE_ID: &str = "fixture-origin";
const PEER_NODE_ID: &str = "fixture-peer";
const ORIGIN_REQUESTER_ID: &str = "fixture-origin-requester";

const EXPECTED_CASE_IDS: [&str; 17] = [
    "calibration_aurora_owner",
    "calibration_cedar_injection",
    "calibration_lumen_peer_without_grant",
    "calibration_nebula_withdrawn_budget",
    "calibration_orion_external_private",
    "calibration_solstice_conflict",
    "holdout_harbor_compound_federated",
    "holdout_harbor_owner_cross_language",
    "holdout_harbor_paraphrase_recovery",
    "holdout_library_external_policy",
    "holdout_quasar_unrelated_injection",
    "holdout_sensor_conflict",
    "regression_atlas_compound_federated",
    "regression_atlas_date_cross_language",
    "regression_atlas_external_ai_policy",
    "regression_atlas_paraphrase_recovery",
    "regression_atlas_unrelated_injection",
];

const REQUIRED_TAGS: [RetrievalTag; 13] = [
    RetrievalTag::Direct,
    RetrievalTag::Paraphrase,
    RetrievalTag::CrossLanguage,
    RetrievalTag::Compound,
    RetrievalTag::Absence,
    RetrievalTag::Conflict,
    RetrievalTag::Privacy,
    RetrievalTag::Duplicate,
    RetrievalTag::Injection,
    RetrievalTag::Stale,
    RetrievalTag::EntityAmbiguity,
    RetrievalTag::Federated,
    RetrievalTag::Stability,
];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrievalFixture {
    schema_version: u32,
    collections: Vec<FixtureCollection>,
    documents: Vec<FixtureDocument>,
    cases: Vec<FixtureCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureCollection {
    id: String,
    node: FixtureNode,
    peer_shareable: bool,
    allow_external_ai: bool,
    granted_to_origin: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
enum FixtureNode {
    Origin,
    Peer,
}

impl FixtureNode {
    const fn runtime_id(self) -> &'static str {
        match self {
            Self::Origin => ORIGIN_NODE_ID,
            Self::Peer => PEER_NODE_ID,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Origin => "origin",
            Self::Peer => "peer",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureDocument {
    id: String,
    domain: String,
    collection_id: String,
    publication_state: FixturePublicationState,
    title: String,
    description: String,
    language: String,
    tags: Vec<String>,
    chunks: Vec<FixtureChunk>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FixturePublicationState {
    Published,
    Withdrawn,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureChunk {
    id: String,
    heading: String,
    text: String,
    semantic_keys: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureCase {
    id: String,
    domain: String,
    split: RetrievalSplit,
    tags: Vec<RetrievalTag>,
    scope: RetrievalScope,
    question: String,
    semantic_keys: Vec<String>,
    relevant_fact_ids: Vec<String>,
    expected_groups: Vec<Vec<String>>,
    allowed_support_fact_ids: Vec<String>,
    forbidden_fact_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum RetrievalSplit {
    Regression,
    Calibration,
    Holdout,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum RetrievalTag {
    Direct,
    Paraphrase,
    CrossLanguage,
    Compound,
    Absence,
    Conflict,
    Privacy,
    Duplicate,
    Injection,
    Stale,
    EntityAmbiguity,
    Federated,
    Stability,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RetrievalScope {
    Local,
    LocalExternalAi,
    TrustedPeer,
    TrustedPeerExternalAi,
    Federated,
}

impl RetrievalScope {
    const fn purpose(self) -> SearchPurpose {
        match self {
            Self::Local | Self::TrustedPeer | Self::Federated => SearchPurpose::LocalAssistant,
            Self::LocalExternalAi | Self::TrustedPeerExternalAi => SearchPurpose::ExternalAi,
        }
    }
}

#[derive(Debug)]
struct LoadedFixture {
    fixture: RetrievalFixture,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EvidenceLocator {
    title: String,
    heading: String,
}

fn evidence_locator(title: &str, heading: &str) -> EvidenceLocator {
    EvidenceLocator {
        title: title.to_owned(),
        heading: heading.chars().take(MAX_HEADING_OR_PAGE_CHARS).collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AuditedCandidate {
    fact_id: Option<String>,
    decision: EvidenceDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaskAuditCall {
    node: FixtureNode,
    candidates: Vec<AuditedCandidate>,
}

#[derive(Debug, Default)]
struct MaskAudit {
    calls: Mutex<HashMap<[u8; 32], Vec<MaskAuditCall>>>,
}

impl MaskAudit {
    fn take(&self, question: &str) -> Result<Vec<MaskAuditCall>> {
        let mut calls = self
            .calls
            .lock()
            .map_err(|_| anyhow!("retrieval mask audit state is unavailable"))?;
        let mut result = calls.remove(&audit_key(question)).unwrap_or_default();
        result.sort_by_key(|call| call.node);
        Ok(result)
    }
}

#[derive(Clone)]
struct AuditedRelevanceProvider {
    inner: Arc<dyn EvidenceRelevanceProvider>,
    facts: Arc<HashMap<EvidenceLocator, String>>,
    audit: Arc<MaskAudit>,
    node: FixtureNode,
}

#[async_trait]
impl EvidenceRelevanceProvider for AuditedRelevanceProvider {
    fn profile_id(&self) -> &str {
        self.inner.profile_id()
    }

    async fn classify(
        &self,
        question: &str,
        candidates: &[RelevanceInput],
    ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
        let decisions = self.inner.classify(question, candidates).await?;
        if decisions.len() != candidates.len() {
            return Err(EvidenceRelevanceError::DecisionCountMismatch {
                expected: candidates.len(),
                actual: decisions.len(),
            });
        }
        let audited = candidates
            .iter()
            .zip(&decisions)
            .map(|(candidate, decision)| AuditedCandidate {
                fact_id: self
                    .facts
                    .get(&evidence_locator(&candidate.title, &candidate.heading))
                    .cloned(),
                decision: *decision,
            })
            .collect();
        self.audit
            .calls
            .lock()
            .map_err(|_| EvidenceRelevanceError::Unavailable)?
            .entry(audit_key(question))
            .or_default()
            .push(MaskAuditCall {
                node: self.node,
                candidates: audited,
            });
        Ok(decisions)
    }
}

fn audit_key(question: &str) -> [u8; 32] {
    Sha256::digest(question.trim().as_bytes()).into()
}

#[derive(Debug, Clone)]
struct FactIdentity {
    id: String,
    title: String,
    heading: String,
    source_sha256: String,
    node: FixtureNode,
    concept_id: Uuid,
    collection_id: Uuid,
    chunk_id: Uuid,
    logical_resource_uri: String,
}

#[derive(Debug, Clone)]
struct FixtureEmbeddingProvider {
    vectors: Arc<HashMap<String, Vec<f32>>>,
}

#[async_trait]
impl EmbeddingProvider for FixtureEmbeddingProvider {
    fn model_id(&self) -> &str {
        "retrieval-fixture-embedding-test-double"
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts
            .iter()
            .map(|text| {
                self.vectors
                    .get(text)
                    .cloned()
                    .ok_or_else(|| anyhow!("retrieval fixture has no embedding for requested text"))
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
struct FixtureRelevanceProvider {
    facts: Arc<HashMap<EvidenceLocator, String>>,
    relevant_by_question: Arc<HashMap<String, HashSet<String>>>,
}

#[async_trait]
impl EvidenceRelevanceProvider for FixtureRelevanceProvider {
    fn profile_id(&self) -> &str {
        "retrieval-fixture-relevance-test-double"
    }

    async fn classify(
        &self,
        question: &str,
        candidates: &[RelevanceInput],
    ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
        let relevant = self
            .relevant_by_question
            .get(question)
            .ok_or(EvidenceRelevanceError::InvalidOutput)?;
        candidates
            .iter()
            .map(|candidate| {
                let key = evidence_locator(&candidate.title, &candidate.heading);
                let fact_id = self
                    .facts
                    .get(&key)
                    .ok_or(EvidenceRelevanceError::InvalidOutput)?;
                Ok(if relevant.contains(fact_id) {
                    EvidenceDecision::Relevant
                } else {
                    EvidenceDecision::Irrelevant
                })
            })
            .collect()
    }
}

#[derive(Clone)]
struct EvaluationProviders {
    embeddings: Arc<dyn EmbeddingProvider>,
    relevance: Arc<dyn EvidenceRelevanceProvider>,
    identity: ProviderIdentity,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderIdentity {
    embedding_profile: String,
    embedding_revision: String,
    relevance_profile: String,
    relevance_revision: String,
    relevance_artifact_filename: Option<String>,
    relevance_artifact_sha256: Option<String>,
    thread_count: usize,
}

#[derive(Debug)]
struct FixtureCorpus {
    origin: HybridSearchEngine,
    peer: HybridSearchEngine,
    facts_by_provenance: HashMap<(String, String), FactIdentity>,
    _workspace: EvaluationWorkspace,
}

#[derive(Debug)]
struct EvaluationWorkspace {
    path: PathBuf,
}

impl EvaluationWorkspace {
    fn create() -> Result<Self> {
        let path = workspace_root()
            .join("target")
            .join("evals")
            .join("retrieval-work")
            .join(Uuid::new_v4().to_string());
        std::fs::create_dir_all(&path).context("creating retrieval evaluation workspace")?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for EvaluationWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedHit {
    fact_id: String,
    rank: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedSource {
    node: FixtureNode,
    hits: Vec<NormalizedHit>,
    authorized_candidates: Vec<NormalizedHit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedRun {
    sources: Vec<NormalizedSource>,
    provenance_errors: u32,
}

#[derive(Debug)]
struct CaseEvaluationRuns {
    baseline: NormalizedRun,
    repeated: NormalizedRun,
    expanded: NormalizedRun,
    reversed: NormalizedRun,
    baseline_audit: Vec<MaskAuditCall>,
    expected_audit_nodes: BTreeSet<FixtureNode>,
    audit_stable: bool,
}

#[derive(Debug, Serialize)]
struct RetrievalCaseReport {
    id: String,
    split: RetrievalSplit,
    tags: Vec<RetrievalTag>,
    expected_group_count: u32,
    found_group_count: u32,
    effective_found_group_count: u32,
    reciprocal_rank_at_five: Option<f64>,
    returned_fact_ids: Vec<String>,
    returned_support_fact_ids: Vec<String>,
    returned_candidate_fact_ids: Vec<String>,
    missing_group_count: u32,
    unexpected_fact_ids: Vec<String>,
    candidate_only_group_count: u32,
    forbidden_fact_ids: Vec<String>,
    provenance_error_count: u32,
    duplicate_violation_count: u32,
    stage_attribution: StageAttribution,
    repeat_stable: bool,
    top_k_prefix_stable: bool,
    insertion_order_stable: bool,
    elapsed_ms: u128,
    passed: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
struct StageAttribution {
    source_candidate_group_count: u32,
    mask_surviving_group_count: u32,
    not_retrieved_group_count: u32,
    rejected_by_mask_group_count: u32,
    outside_top_k_group_count: u32,
    revalidation_loss_group_count: u32,
    unexpected_survivor_count: u32,
    hard_negative_source_candidate_count: u32,
    mapping_error_count: u32,
    audit_complete: bool,
    audit_stable: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
struct StageAttributionSummary {
    expected_group_count: u32,
    source_candidate_group_count: u32,
    mask_surviving_group_count: u32,
    source_candidate_recall_at_ten: Option<f64>,
    mask_surviving_recall_at_ten: Option<f64>,
    not_retrieved_group_count: u32,
    rejected_by_mask_group_count: u32,
    outside_top_k_group_count: u32,
    revalidation_loss_group_count: u32,
    unexpected_survivor_count: u32,
    hard_negative_source_candidate_count: u32,
    mapping_error_count: u32,
    audit_error_case_count: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
struct AggregateMetrics {
    case_count: u32,
    expected_group_count: u32,
    found_group_count: u32,
    effective_found_group_count: u32,
    candidate_only_group_count: u32,
    recall_at_five: Option<f64>,
    effective_recall_at_five: Option<f64>,
    mean_reciprocal_rank_at_five: Option<f64>,
    false_evidence_count: u32,
    forbidden_evidence_count: u32,
    provenance_error_count: u32,
    duplicate_violation_count: u32,
    unstable_case_count: u32,
    stage_audit_error_count: u32,
}

#[derive(Debug, Serialize)]
struct RetrievalEvaluationReport {
    schema_version: u32,
    fixture_sha256: String,
    target_os: String,
    target_arch: String,
    provider: ProviderIdentity,
    top_k: u8,
    elapsed_ms: u128,
    regression: AggregateMetrics,
    calibration: AggregateMetrics,
    holdout: AggregateMetrics,
    total: AggregateMetrics,
    stage_attribution: StageAttributionSummary,
    passed: bool,
    cases: Vec<RetrievalCaseReport>,
}

pub async fn validate() -> Result<()> {
    let loaded = load_fixture()?;
    let providers = fixture_providers(&loaded.fixture)?;
    let report = run_evaluation(&loaded, providers).await?;
    ensure!(
        report.passed,
        "deterministic retrieval pipeline did not meet the fixture contract"
    );
    println!(
        "validated {} retrieval cases through SQLite/FTS, vector RRF, relevance, deduplication, policy and revalidation (SHA-256 {})",
        loaded.fixture.cases.len(),
        loaded.sha256
    );
    Ok(())
}

pub async fn evaluate(embedding_snapshot: &Path, relevance_snapshot: &Path) -> Result<()> {
    validate_model_revisions()?;
    let loaded = load_fixture()?;
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 4);
    let embedding_snapshot = PinnedE5Snapshot::open(embedding_snapshot)?;
    let embeddings: Arc<dyn EmbeddingProvider> = Arc::new(FastEmbedE5Small::from_snapshot(
        &embedding_snapshot,
        threads,
    )?);
    let relevance_snapshot = PinnedMmarcoRerankerSnapshot::open(relevance_snapshot)?;
    let relevance: Arc<dyn EvidenceRelevanceProvider> = Arc::new(
        FastEmbedMmarcoReranker::from_snapshot(relevance_snapshot, threads)?,
    );
    let relevance_artifact =
        platform_relevance_model().context("unsupported retrieval evaluation target")?;
    let providers = EvaluationProviders {
        identity: ProviderIdentity {
            embedding_profile: embeddings.model_id().to_owned(),
            embedding_revision: E5_MODEL_REVISION.to_owned(),
            relevance_profile: relevance.profile_id().to_owned(),
            relevance_revision: MMARCO_RERANKER_REVISION.to_owned(),
            relevance_artifact_filename: Some(relevance_artifact.filename.to_owned()),
            relevance_artifact_sha256: Some(relevance_artifact.sha256.to_owned()),
            thread_count: threads,
        },
        embeddings,
        relevance,
    };
    let report = run_evaluation(&loaded, providers).await?;
    let destination = write_report(&report)?;
    ensure!(
        report.passed,
        "retrieval pipeline did not meet the acceptance thresholds; report written to {}",
        destination.display()
    );
    println!(
        "retrieval pipeline passed; report written to {}",
        destination.display()
    );
    Ok(())
}

fn load_fixture() -> Result<LoadedFixture> {
    let path = workspace_root().join(FIXTURE_PATH);
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let fixture: RetrievalFixture =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    validate_fixture_data(&fixture)?;
    Ok(LoadedFixture {
        fixture,
        sha256: hex::encode(Sha256::digest(&bytes)),
    })
}

fn validate_fixture_data(fixture: &RetrievalFixture) -> Result<()> {
    ensure!(
        fixture.schema_version == FIXTURE_SCHEMA_VERSION,
        "unsupported retrieval fixture schema"
    );
    let mut collection_ids = BTreeSet::new();
    let mut collection_nodes = HashMap::new();
    let mut collection_grants = HashMap::new();
    let mut collection_shareability = HashMap::new();
    for collection in &fixture.collections {
        validate_identifier(&collection.id, "collection")?;
        ensure!(
            collection_ids.insert(collection.id.as_str()),
            "duplicate retrieval collection id"
        );
        ensure!(
            !collection.granted_to_origin || collection.node == FixtureNode::Peer,
            "only peer collections may be granted to the origin fixture"
        );
        ensure!(
            !collection.granted_to_origin || collection.peer_shareable,
            "a granted retrieval collection must be peer-shareable"
        );
        collection_nodes.insert(collection.id.as_str(), collection.node);
        collection_grants.insert(collection.id.as_str(), collection.granted_to_origin);
        collection_shareability.insert(collection.id.as_str(), collection.peer_shareable);
    }
    ensure!(
        collection_nodes
            .values()
            .any(|node| *node == FixtureNode::Origin)
            && collection_nodes
                .values()
                .any(|node| *node == FixtureNode::Peer),
        "retrieval fixture requires origin and peer collections"
    );

    let mut document_ids = BTreeSet::new();
    let mut fact_ids = BTreeSet::new();
    let mut facts = HashMap::<&str, &FixtureChunk>::new();
    let mut fact_domains = HashMap::<&str, &str>::new();
    let mut fact_collections = HashMap::<&str, &str>::new();
    let mut evidence_locators = HashSet::new();
    let mut has_withdrawn_document = false;
    for document in &fixture.documents {
        validate_identifier(&document.id, "document")?;
        validate_identifier(&document.domain, "domain")?;
        ensure!(
            document_ids.insert(document.id.as_str()),
            "duplicate retrieval document id"
        );
        ensure!(
            collection_nodes.contains_key(document.collection_id.as_str()),
            "retrieval document references an unknown collection"
        );
        ensure!(!document.title.trim().is_empty(), "document title is empty");
        ensure!(
            !document.description.trim().is_empty(),
            "document description is empty"
        );
        ensure!(
            !document.language.trim().is_empty(),
            "document language is empty"
        );
        ensure!(!document.tags.is_empty(), "document tags are empty");
        ensure!(!document.chunks.is_empty(), "document has no chunks");
        has_withdrawn_document |= document.publication_state == FixturePublicationState::Withdrawn;
        let mut headings = BTreeSet::new();
        for chunk in &document.chunks {
            validate_identifier(&chunk.id, "fact")?;
            ensure!(
                fact_ids.insert(chunk.id.as_str()),
                "duplicate retrieval fact id"
            );
            ensure!(!chunk.heading.trim().is_empty(), "fact heading is empty");
            ensure!(
                headings.insert(chunk.heading.as_str()),
                "document has duplicate fact headings"
            );
            ensure!(!chunk.text.trim().is_empty(), "fact text is empty");
            ensure!(
                chunk.text.chars().count() < MAX_SNIPPET_CHARS,
                "retrieval fact must fit in one visible snippet"
            );
            validate_semantic_keys(&chunk.semantic_keys, "fact")?;
            ensure!(
                evidence_locators.insert(evidence_locator(&document.title, &chunk.heading)),
                "retrieval facts must have unique title and normalized heading pairs"
            );
            facts.insert(chunk.id.as_str(), chunk);
            fact_domains.insert(chunk.id.as_str(), document.domain.as_str());
            fact_collections.insert(chunk.id.as_str(), document.collection_id.as_str());
        }
    }
    ensure!(
        has_withdrawn_document,
        "retrieval fixture requires a withdrawn publication"
    );

    let expected_ids = EXPECTED_CASE_IDS.into_iter().collect::<BTreeSet<_>>();
    let mut case_ids = BTreeSet::new();
    let mut questions = BTreeSet::new();
    let mut splits = BTreeSet::new();
    let mut tags = BTreeSet::new();
    let mut regression_domains = BTreeSet::new();
    let mut calibration_domains = BTreeSet::new();
    let mut holdout_domains = BTreeSet::new();
    let mut has_peer_without_grant_case = false;
    for case in &fixture.cases {
        validate_identifier(&case.id, "case")?;
        validate_identifier(&case.domain, "domain")?;
        ensure!(
            case_ids.insert(case.id.as_str()),
            "duplicate retrieval case id"
        );
        ensure!(
            !case.question.trim().is_empty(),
            "retrieval question is empty"
        );
        ensure!(
            questions.insert(case.question.as_str()),
            "retrieval questions must be unique"
        );
        validate_semantic_keys(&case.semantic_keys, "case")?;
        let case_tags = case.tags.iter().copied().collect::<BTreeSet<_>>();
        ensure!(!case_tags.is_empty(), "retrieval case has no tags");
        ensure!(
            case_tags.len() == case.tags.len(),
            "retrieval case has duplicate tags"
        );
        tags.extend(case_tags);
        splits.insert(case.split);
        match case.split {
            RetrievalSplit::Regression => {
                regression_domains.insert(case.domain.as_str());
            }
            RetrievalSplit::Calibration => {
                calibration_domains.insert(case.domain.as_str());
            }
            RetrievalSplit::Holdout => {
                holdout_domains.insert(case.domain.as_str());
            }
        }

        let relevant = validate_fact_references(&case.relevant_fact_ids, &fact_ids, "relevant")?;
        let support =
            validate_fact_references(&case.allowed_support_fact_ids, &fact_ids, "allowed support")?;
        let forbidden = validate_fact_references(&case.forbidden_fact_ids, &fact_ids, "forbidden")?;
        ensure!(
            case.expected_groups.is_empty() || !relevant.is_empty(),
            "an answerable retrieval case needs relevant facts"
        );
        let mut expected = BTreeSet::new();
        for group in &case.expected_groups {
            ensure!(!group.is_empty(), "retrieval expected group is empty");
            let group_ids = validate_fact_references(group, &fact_ids, "expected group")?;
            for fact_id in &group_ids {
                ensure!(
                    expected.insert(*fact_id),
                    "retrieval expected groups must be pairwise disjoint"
                );
            }
            ensure!(
                group_ids.iter().all(|id| relevant.contains(id)),
                "expected retrieval facts must be relevant"
            );
            ensure!(
                group_ids.is_disjoint(&forbidden),
                "expected retrieval facts cannot be forbidden"
            );
            if group.len() > 1 {
                let first_text = facts
                    .get(group[0].as_str())
                    .context("expected retrieval fact is missing")?
                    .text
                    .as_str();
                ensure!(
                    group.iter().all(|id| {
                        facts
                            .get(id.as_str())
                            .is_some_and(|fact| fact.text == first_text)
                    }),
                    "multi-fact expected groups must contain equivalent text"
                );
            }
        }
        ensure!(
            support.is_disjoint(&expected),
            "allowed support facts cannot be expected evidence"
        );
        ensure!(
            support.is_disjoint(&relevant),
            "allowed support facts cannot be relevant evidence"
        );
        ensure!(
            support.is_disjoint(&forbidden),
            "allowed support facts cannot be forbidden"
        );
        if expected.is_empty() {
            ensure!(
                support.is_empty(),
                "non-answerable retrieval cases cannot allow support evidence"
            );
            ensure!(
                relevant.is_subset(&forbidden),
                "non-answerable relevant facts must be forbidden"
            );
        } else {
            ensure!(
                relevant == expected,
                "answerable relevant facts must exactly match expected evidence"
            );
        }
        has_peer_without_grant_case |= case.scope == RetrievalScope::TrustedPeer
            && expected.is_empty()
            && !relevant.is_empty()
            && relevant.iter().all(|fact_id| {
                fact_collections
                    .get(fact_id)
                    .and_then(|collection_id| collection_nodes.get(collection_id))
                    == Some(&FixtureNode::Peer)
                    && fact_collections
                        .get(fact_id)
                        .and_then(|collection_id| collection_grants.get(collection_id))
                        == Some(&false)
                    && fact_collections
                        .get(fact_id)
                        .and_then(|collection_id| collection_shareability.get(collection_id))
                        == Some(&true)
            });
        ensure!(
            relevant
                .iter()
                .chain(&support)
                .chain(&forbidden)
                .all(|fact_id| {
                    fact_domains.get(fact_id).copied() == Some(case.domain.as_str())
                }),
            "retrieval case references evidence from another domain"
        );
    }
    ensure!(case_ids == expected_ids, "retrieval case id set changed");
    ensure!(
        splits
            == BTreeSet::from([
                RetrievalSplit::Regression,
                RetrievalSplit::Calibration,
                RetrievalSplit::Holdout,
            ]),
        "retrieval fixture requires regression, calibration and holdout splits"
    );
    ensure!(
        regression_domains.is_disjoint(&calibration_domains)
            && regression_domains.is_disjoint(&holdout_domains)
            && calibration_domains.is_disjoint(&holdout_domains),
        "retrieval split domains must be pairwise disjoint"
    );
    ensure!(
        has_peer_without_grant_case,
        "retrieval fixture requires a peer-without-grant case"
    );
    for required in REQUIRED_TAGS {
        ensure!(
            tags.contains(&required),
            "retrieval fixture is missing a required tag"
        );
    }
    Ok(())
}

fn validate_identifier(value: &str, kind: &str) -> Result<()> {
    ensure!(!value.is_empty(), "retrieval {kind} id is empty");
    ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "retrieval {kind} id must use lowercase ASCII, digits and underscores"
    );
    Ok(())
}

fn validate_semantic_keys(keys: &[String], kind: &str) -> Result<()> {
    ensure!(!keys.is_empty(), "retrieval {kind} has no semantic keys");
    let unique = keys.iter().collect::<BTreeSet<_>>();
    ensure!(
        unique.len() == keys.len(),
        "retrieval {kind} has duplicate semantic keys"
    );
    ensure!(
        keys.iter().all(|key| {
            !key.is_empty()
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }),
        "retrieval semantic keys must use lowercase ASCII, digits and underscores"
    );
    Ok(())
}

fn validate_fact_references<'a>(
    ids: &'a [String],
    known: &BTreeSet<&str>,
    kind: &str,
) -> Result<BTreeSet<&'a str>> {
    let unique = ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
    ensure!(
        unique.len() == ids.len(),
        "retrieval {kind} fact list contains duplicates"
    );
    ensure!(
        unique.iter().all(|id| known.contains(id)),
        "retrieval {kind} fact list references an unknown fact"
    );
    Ok(unique)
}

fn fixture_providers(fixture: &RetrievalFixture) -> Result<EvaluationProviders> {
    let mut semantic_keys = BTreeSet::new();
    for document in &fixture.documents {
        for chunk in &document.chunks {
            semantic_keys.extend(chunk.semantic_keys.iter().cloned());
        }
    }
    for case in &fixture.cases {
        semantic_keys.extend(case.semantic_keys.iter().cloned());
    }
    ensure!(
        semantic_keys.len() <= EMBEDDING_DIMENSIONS,
        "retrieval fixture has more semantic keys than embedding dimensions"
    );
    let key_dimensions = semantic_keys
        .into_iter()
        .enumerate()
        .map(|(index, key)| (key, index))
        .collect::<HashMap<_, _>>();

    let mut vectors = HashMap::<String, Vec<f32>>::new();
    let mut facts = HashMap::new();
    for document in &fixture.documents {
        for chunk in &document.chunks {
            insert_fixture_vector(
                &mut vectors,
                format!("passage: {}", chunk.text),
                &chunk.semantic_keys,
                &key_dimensions,
            )?;
            facts.insert(
                evidence_locator(&document.title, &chunk.heading),
                chunk.id.clone(),
            );
        }
    }
    let mut relevant_by_question = HashMap::new();
    for case in &fixture.cases {
        insert_fixture_vector(
            &mut vectors,
            format!("query: {}", case.question.trim()),
            &case.semantic_keys,
            &key_dimensions,
        )?;
        relevant_by_question.insert(
            case.question.clone(),
            case.relevant_fact_ids.iter().cloned().collect(),
        );
    }

    let embeddings: Arc<dyn EmbeddingProvider> = Arc::new(FixtureEmbeddingProvider {
        vectors: Arc::new(vectors),
    });
    let relevance: Arc<dyn EvidenceRelevanceProvider> = Arc::new(FixtureRelevanceProvider {
        facts: Arc::new(facts),
        relevant_by_question: Arc::new(relevant_by_question),
    });
    Ok(EvaluationProviders {
        identity: ProviderIdentity {
            embedding_profile: embeddings.model_id().to_owned(),
            embedding_revision: "synthetic-v3".to_owned(),
            relevance_profile: relevance.profile_id().to_owned(),
            relevance_revision: "synthetic-v3".to_owned(),
            relevance_artifact_filename: None,
            relevance_artifact_sha256: None,
            thread_count: 1,
        },
        embeddings,
        relevance,
    })
}

fn fact_ids_by_locator(fixture: &RetrievalFixture) -> Result<HashMap<EvidenceLocator, String>> {
    let mut facts = HashMap::new();
    for document in &fixture.documents {
        for chunk in &document.chunks {
            let previous = facts.insert(
                evidence_locator(&document.title, &chunk.heading),
                chunk.id.clone(),
            );
            ensure!(
                previous.is_none(),
                "retrieval fixture title and normalized heading pairs must be unique"
            );
        }
    }
    Ok(facts)
}

fn expected_audit_nodes(fixture: &RetrievalFixture, case: &FixtureCase) -> BTreeSet<FixtureNode> {
    let purpose = case.scope.purpose();
    let scopes = match case.scope {
        RetrievalScope::Local | RetrievalScope::LocalExternalAi => {
            [Some(FixtureNode::Origin), None]
        }
        RetrievalScope::TrustedPeer | RetrievalScope::TrustedPeerExternalAi => {
            [Some(FixtureNode::Peer), None]
        }
        RetrievalScope::Federated => [Some(FixtureNode::Origin), Some(FixtureNode::Peer)],
    };
    scopes
        .into_iter()
        .flatten()
        .filter(|node| node_has_searchable_document(fixture, *node, purpose))
        .collect()
}

fn node_has_searchable_document(
    fixture: &RetrievalFixture,
    node: FixtureNode,
    purpose: SearchPurpose,
) -> bool {
    fixture.documents.iter().any(|document| {
        if document.publication_state != FixturePublicationState::Published {
            return false;
        }
        let Some(collection) = fixture
            .collections
            .iter()
            .find(|collection| collection.id == document.collection_id)
        else {
            return false;
        };
        if collection.node != node {
            return false;
        }
        let purpose_allowed = purpose != SearchPurpose::ExternalAi || collection.allow_external_ai;
        match node {
            FixtureNode::Origin => purpose_allowed,
            FixtureNode::Peer => {
                purpose_allowed && collection.peer_shareable && collection.granted_to_origin
            }
        }
    })
}

fn insert_fixture_vector(
    vectors: &mut HashMap<String, Vec<f32>>,
    input: String,
    semantic_keys: &[String],
    key_dimensions: &HashMap<String, usize>,
) -> Result<()> {
    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    for key in semantic_keys {
        let index = key_dimensions
            .get(key)
            .copied()
            .context("retrieval semantic key has no assigned dimension")?;
        vector[index] = 1.0;
    }
    normalize_vector(&mut vector);
    if let Some(previous) = vectors.insert(input, vector.clone()) {
        ensure!(
            previous == vector,
            "identical retrieval text cannot have different semantic keys"
        );
    }
    Ok(())
}

fn normalize_vector(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in vector {
            *value /= norm;
        }
    }
}

async fn run_evaluation(
    loaded: &LoadedFixture,
    providers: EvaluationProviders,
) -> Result<RetrievalEvaluationReport> {
    let started = Instant::now();
    let audit = Arc::new(MaskAudit::default());
    let facts = Arc::new(fact_ids_by_locator(&loaded.fixture)?);
    let forward = build_corpus(
        &loaded.fixture,
        &providers,
        false,
        Arc::clone(&audit),
        Arc::clone(&facts),
    )
    .await?;
    let reverse =
        build_corpus(&loaded.fixture, &providers, true, Arc::clone(&audit), facts).await?;
    let mut case_reports = Vec::with_capacity(loaded.fixture.cases.len());
    for case in &loaded.fixture.cases {
        let case_started = Instant::now();
        let baseline = run_case(&forward, case, TOP_K).await?;
        let baseline_audit = audit.take(&case.question)?;
        let repeated = run_case(&forward, case, TOP_K).await?;
        let repeated_audit = audit.take(&case.question)?;
        let expanded = run_case(&forward, case, MAX_TOP_K).await?;
        let expanded_audit = audit.take(&case.question)?;
        let reversed = run_case(&reverse, case, TOP_K).await?;
        let reversed_audit = audit.take(&case.question)?;
        let audit_stable = baseline_audit == repeated_audit
            && baseline_audit == expanded_audit
            && baseline_audit == reversed_audit;
        case_reports.push(score_case(
            case,
            CaseEvaluationRuns {
                baseline,
                repeated,
                expanded,
                reversed,
                baseline_audit,
                expected_audit_nodes: expected_audit_nodes(&loaded.fixture, case),
                audit_stable,
            },
            case_started.elapsed().as_millis(),
        ));
    }
    let regression = aggregate_metrics(
        case_reports
            .iter()
            .filter(|report| report.split == RetrievalSplit::Regression),
    );
    let calibration = aggregate_metrics(
        case_reports
            .iter()
            .filter(|report| report.split == RetrievalSplit::Calibration),
    );
    let holdout = aggregate_metrics(
        case_reports
            .iter()
            .filter(|report| report.split == RetrievalSplit::Holdout),
    );
    let total = aggregate_metrics(case_reports.iter());
    let stage_attribution = aggregate_stage_attribution(&case_reports);
    let passed = regression_cases_pass(&case_reports)
        && split_passes(&regression)
        && split_passes(&calibration)
        && split_passes(&holdout);
    Ok(RetrievalEvaluationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        fixture_sha256: loaded.sha256.clone(),
        target_os: std::env::consts::OS.to_owned(),
        target_arch: std::env::consts::ARCH.to_owned(),
        provider: providers.identity,
        top_k: TOP_K,
        elapsed_ms: started.elapsed().as_millis(),
        regression,
        calibration,
        holdout,
        total,
        stage_attribution,
        passed,
        cases: case_reports,
    })
}

async fn build_corpus(
    fixture: &RetrievalFixture,
    providers: &EvaluationProviders,
    reverse_documents: bool,
    audit: Arc<MaskAudit>,
    facts: Arc<HashMap<EvidenceLocator, String>>,
) -> Result<FixtureCorpus> {
    let origin_database = Database::in_memory()?;
    let peer_database = Database::in_memory()?;
    let workspace = EvaluationWorkspace::create()?;
    let mut origin_collections = HashMap::new();
    let mut peer_collections = HashMap::new();
    for collection in &fixture.collections {
        let database = match collection.node {
            FixtureNode::Origin => &origin_database,
            FixtureNode::Peer => &peer_database,
        };
        let base = workspace
            .path()
            .join(collection.node.label())
            .join(&collection.id);
        std::fs::create_dir_all(base.join("sources"))
            .context("creating retrieval fixture source directory")?;
        std::fs::create_dir_all(base.join("wiki"))
            .context("creating retrieval fixture wiki directory")?;
        let record = database.create_collection(
            &collection.id,
            base.join("sources"),
            base.join("wiki"),
            CollectionPolicy {
                local_only: !collection.peer_shareable && !collection.allow_external_ai,
                peer_shareable: collection.peer_shareable,
                allow_external_ai: collection.allow_external_ai,
                internet_public: false,
            },
        )?;
        match collection.node {
            FixtureNode::Origin => {
                origin_collections.insert(collection.id.as_str(), record.id);
            }
            FixtureNode::Peer => {
                peer_collections.insert(collection.id.as_str(), record.id);
            }
        }
    }

    peer_database.upsert_peer(&airwiki_core::PeerRecord {
        peer_id: ORIGIN_REQUESTER_ID.to_owned(),
        display_name: Some("fixture origin".to_owned()),
        trusted: true,
        blocked: false,
        paired_at: None,
        last_seen_at: None,
    })?;
    for collection in fixture
        .collections
        .iter()
        .filter(|collection| collection.node == FixtureNode::Peer && collection.granted_to_origin)
    {
        let collection_id = peer_collections
            .get(collection.id.as_str())
            .copied()
            .context("peer fixture collection was not created")?;
        peer_database.set_grant(ORIGIN_REQUESTER_ID, collection_id, true)?;
    }

    let mut facts_by_provenance = HashMap::new();
    let documents: Box<dyn Iterator<Item = &FixtureDocument>> = if reverse_documents {
        Box::new(fixture.documents.iter().rev())
    } else {
        Box::new(fixture.documents.iter())
    };
    for document in documents {
        let collection = fixture
            .collections
            .iter()
            .find(|collection| collection.id == document.collection_id)
            .context("fixture document collection is missing")?;
        let (database, collections) = match collection.node {
            FixtureNode::Origin => (&origin_database, &origin_collections),
            FixtureNode::Peer => (&peer_database, &peer_collections),
        };
        let collection_id = collections
            .get(document.collection_id.as_str())
            .copied()
            .context("fixture document collection was not created")?;
        seed_document(
            database,
            collection_id,
            collection.node,
            document,
            workspace.path(),
            Arc::clone(&providers.embeddings),
            &mut facts_by_provenance,
        )
        .await?;
    }

    Ok(FixtureCorpus {
        origin: HybridSearchEngine::new(
            origin_database,
            Arc::clone(&providers.embeddings),
            Arc::new(AuditedRelevanceProvider {
                inner: Arc::clone(&providers.relevance),
                facts: Arc::clone(&facts),
                audit: Arc::clone(&audit),
                node: FixtureNode::Origin,
            }),
            ORIGIN_NODE_ID,
        ),
        peer: HybridSearchEngine::new(
            peer_database,
            Arc::clone(&providers.embeddings),
            Arc::new(AuditedRelevanceProvider {
                inner: Arc::clone(&providers.relevance),
                facts,
                audit,
                node: FixtureNode::Peer,
            }),
            PEER_NODE_ID,
        ),
        facts_by_provenance,
        _workspace: workspace,
    })
}

async fn seed_document(
    database: &Database,
    collection_id: Uuid,
    node: FixtureNode,
    document: &FixtureDocument,
    workspace: &Path,
    embeddings: Arc<dyn EmbeddingProvider>,
    facts_by_provenance: &mut HashMap<(String, String), FactIdentity>,
) -> Result<()> {
    let source_contents = fixture_source_markdown(document);
    let source_sha256 = synthetic_sha256(&source_contents);
    let source_path = workspace
        .join(node.label())
        .join(&document.collection_id)
        .join("sources")
        .join(format!("{}.md", document.id));
    std::fs::write(&source_path, source_contents.as_bytes())
        .context("writing retrieval fixture source")?;
    let byte_size =
        u64::try_from(source_contents.len()).context("retrieval source is too large")?;
    let source = database.register_source(
        collection_id,
        &source_path,
        &source_sha256,
        "markdown",
        byte_size,
    )?;
    let character_count =
        u64::try_from(source_contents.chars().count()).context("retrieval source is too large")?;
    database.mark_extracted(source.id(), 0, character_count)?;
    let draft = EnrichmentDraft {
        concept_type: ConceptType::Document,
        title: document.title.clone(),
        description: document.description.clone(),
        language: document.language.clone(),
        tags: document.tags.clone(),
        entities: Vec::new(),
        links: Vec::new(),
        summary: document.description.clone(),
        classification_confidence: 1.0,
        classification_explanation: "synthetic retrieval fixture".to_owned(),
    };
    let concept = database.save_enrichment(
        source.id(),
        draft.clone(),
        node.runtime_id(),
        "retrieval-fixture",
    )?;
    let inputs = document
        .chunks
        .iter()
        .map(|chunk| format!("passage: {}", chunk.text))
        .collect::<Vec<_>>();
    let embedded = embeddings.embed(&inputs).await?;
    ensure!(
        embedded.len() == document.chunks.len(),
        "embedding provider returned the wrong retrieval fixture count"
    );
    let mut stored = Vec::with_capacity(document.chunks.len());
    for (ordinal, (chunk, embedding)) in document.chunks.iter().zip(embedded).enumerate() {
        ensure!(
            embedding.len() == EMBEDDING_DIMENSIONS,
            "embedding provider returned the wrong retrieval fixture dimensions"
        );
        let ordinal = u32::try_from(ordinal).context("retrieval chunk ordinal overflow")?;
        let text_sha256 = synthetic_sha256(&chunk.text);
        let public_chunk_id = expected_public_chunk_id(&source_sha256, ordinal, &text_sha256);
        stored.push(StoredChunk {
            id: Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("airwiki-retrieval:{}:{}", document.id, chunk.id).as_bytes(),
            ),
            concept_id: concept.id,
            source_document_id: source.id(),
            collection_id,
            ordinal,
            heading_or_page: chunk.heading.clone(),
            text: chunk.text.clone(),
            text_sha256,
            embedding,
            source_revision: 1,
        });
        let previous = facts_by_provenance.insert(
            (source_sha256.clone(), chunk.heading.clone()),
            FactIdentity {
                id: chunk.id.clone(),
                title: document.title.clone(),
                heading: chunk.heading.clone(),
                source_sha256: source_sha256.clone(),
                node,
                concept_id: concept.id,
                collection_id,
                chunk_id: public_chunk_id,
                logical_resource_uri: concept.logical_resource_uri.clone(),
            },
        );
        ensure!(
            previous.is_none(),
            "retrieval fixture provenance identity is not unique"
        );
    }
    database.replace_chunks(concept.id, &stored)?;
    let evidence = database
        .review_evidence_page(concept.id, 1, None, None, 1)?
        .context("retrieval fixture review evidence is missing")?;
    let materializer = OkfPublicationMaterializer::new(database.clone());
    materializer.approve(concept.id, draft, &evidence.review_version)?;
    if document.publication_state == FixturePublicationState::Withdrawn {
        database.mark_deleted(source.id())?;
        materializer.withdraw_published_artifact(collection_id, concept.id, &source_sha256)?;
    }
    Ok(())
}

fn fixture_source_markdown(document: &FixtureDocument) -> String {
    let mut source = format!("# {}\n\n{}\n", document.title, document.description);
    for chunk in &document.chunks {
        source.push_str(&format!("\n## {}\n\n{}\n", chunk.heading, chunk.text));
    }
    source
}

fn synthetic_sha256(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

/// Independently derives the documented wire identity so the evaluator catches
/// regressions in the production mapping instead of accepting any non-nil UUID.
fn expected_public_chunk_id(source_sha256: &str, ordinal: u32, text_sha256: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("urn:airwiki:chunk:{source_sha256}:{ordinal}:{text_sha256}").as_bytes(),
    )
}

async fn run_case(corpus: &FixtureCorpus, case: &FixtureCase, top_k: u8) -> Result<NormalizedRun> {
    let include_candidates = matches!(case.scope.purpose(), SearchPurpose::ExternalAi);
    let request = SearchRequest::new(&case.question, case.scope.purpose(), top_k);
    let responses = match case.scope {
        RetrievalScope::Local | RetrievalScope::LocalExternalAi => vec![(
            FixtureNode::Origin,
            corpus.origin.search_local(request).await?,
        )],
        RetrievalScope::TrustedPeer | RetrievalScope::TrustedPeerExternalAi => vec![(
            FixtureNode::Peer,
            corpus
                .peer
                .search_for_peer(request, ORIGIN_REQUESTER_ID)
                .await?,
        )],
        RetrievalScope::Federated => {
            // Preserve each node's top-k list so this evaluator measures source
            // coverage. The gateway's second RRF and cross-node deduplication
            // stay in airwiki-network's focused coordinator tests.
            let (local, peer) = tokio::join!(
                corpus.origin.search_local(request.clone()),
                corpus.peer.search_for_peer(request, ORIGIN_REQUESTER_ID)
            );
            vec![(FixtureNode::Origin, local?), (FixtureNode::Peer, peer?)]
        }
    };
    normalize_responses(responses, include_candidates, &corpus.facts_by_provenance)
}

fn normalize_responses(
    responses: Vec<(FixtureNode, airwiki_types::SearchResponse)>,
    include_candidates: bool,
    facts: &HashMap<(String, String), FactIdentity>,
) -> Result<NormalizedRun> {
    let mut sources = Vec::with_capacity(responses.len());
    let mut provenance_errors = 0_u32;
    for (node, response) in responses {
        let mut hits = Vec::with_capacity(response.hits.len());
        for (index, hit) in response.hits.into_iter().enumerate() {
            normalize_hit(index, hit, node, facts, &mut hits, &mut provenance_errors);
        }
        let mut authorized_candidates = Vec::with_capacity(response.authorized_candidates.len());
        if include_candidates {
            for (index, hit) in response.authorized_candidates.into_iter().enumerate() {
                normalize_hit(
                    index,
                    hit,
                    node,
                    facts,
                    &mut authorized_candidates,
                    &mut provenance_errors,
                );
            }
        }
        sources.push(NormalizedSource {
            node,
            hits,
            authorized_candidates,
        });
    }
    Ok(NormalizedRun {
        sources,
        provenance_errors,
    })
}

fn normalize_hit(
    index: usize,
    hit: SearchHit,
    node: FixtureNode,
    facts: &HashMap<(String, String), FactIdentity>,
    out: &mut Vec<NormalizedHit>,
    provenance_errors: &mut u32,
) {
    let expected_rank = u32::try_from(index + 1).unwrap_or(u32::MAX);
    let identity = facts.get(&(hit.source_sha256.clone(), hit.heading_or_page.clone()));
    let Some(identity) = identity else {
        *provenance_errors = provenance_errors.saturating_add(1);
        return;
    };
    if !hit_has_valid_provenance(&hit, identity, node, expected_rank) {
        *provenance_errors = provenance_errors.saturating_add(1);
    }
    out.push(NormalizedHit {
        fact_id: identity.id.clone(),
        rank: hit.rank,
    });
}

fn hit_has_valid_provenance(
    hit: &SearchHit,
    identity: &FactIdentity,
    node: FixtureNode,
    expected_rank: u32,
) -> bool {
    identity.node == node
        && hit.node_id == node.runtime_id()
        && hit.title == identity.title
        && hit.heading_or_page == identity.heading
        && hit.source_sha256 == identity.source_sha256
        && hit.source_revision == 1
        && hit.rank == expected_rank
        && hit.collection_id == identity.collection_id
        && hit.concept_id == identity.concept_id
        && hit.chunk_id == identity.chunk_id
        && !hit.snippet.trim().is_empty()
        && hit.logical_resource_uri == identity.logical_resource_uri
}

fn score_case(
    case: &FixtureCase,
    runs: CaseEvaluationRuns,
    elapsed_ms: u128,
) -> RetrievalCaseReport {
    let CaseEvaluationRuns {
        baseline,
        repeated,
        expanded,
        reversed,
        baseline_audit,
        expected_audit_nodes,
        audit_stable,
    } = runs;
    let repeat_stable = baseline == repeated;
    let top_k_prefix_stable = run_is_prefix(&baseline, &expanded);
    let insertion_order_stable = baseline == reversed;
    let returned = baseline
        .sources
        .iter()
        .flat_map(|source| source.hits.iter())
        .collect::<Vec<_>>();
    let returned_candidates = baseline
        .sources
        .iter()
        .flat_map(|source| source.authorized_candidates.iter())
        .collect::<Vec<_>>();
    let returned_ids = returned
        .iter()
        .map(|hit| hit.fact_id.as_str())
        .collect::<Vec<_>>();
    let returned_candidate_ids = returned_candidates
        .iter()
        .map(|hit| hit.fact_id.as_str())
        .collect::<Vec<_>>();
    let relevant = case
        .relevant_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let allowed_support = case
        .allowed_support_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let forbidden = case
        .forbidden_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let expected = case
        .expected_groups
        .iter()
        .flatten()
        .map(String::as_str)
        .collect::<HashSet<_>>();

    let returned_ids_set = returned_ids.iter().copied().collect::<HashSet<_>>();
    let returned_candidate_ids_set = returned_candidate_ids
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let effective_returned_ids = returned_ids_set
        .union(&returned_candidate_ids_set)
        .copied()
        .collect::<HashSet<_>>();
    let group_has = |group: &[String], ids: &HashSet<&str>| {
        group.iter().any(|fact_id| ids.contains(fact_id.as_str()))
    };
    let found_group_count = case
        .expected_groups
        .iter()
        .filter(|group| group_has(group, &returned_ids_set))
        .count();
    let effective_found_group_count = case
        .expected_groups
        .iter()
        .filter(|group| group_has(group, &effective_returned_ids))
        .count();
    let candidate_only_group_count = case
        .expected_groups
        .iter()
        .filter(|group| {
            group_has(group, &returned_candidate_ids_set) && !group_has(group, &returned_ids_set)
        })
        .count();
    let duplicate_violation_count = case
        .expected_groups
        .iter()
        .filter(|group| group.len() > 1)
        .filter(|group| {
            group
                .iter()
                .filter(|fact_id| returned_ids_set.contains(&fact_id.as_str()))
                .count()
                > 1
        })
        .count();
    let unexpected_fact_ids = returned_ids
        .iter()
        .copied()
        .filter(|fact_id| !relevant.contains(fact_id) && !allowed_support.contains(fact_id))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let returned_forbidden_fact_ids = returned_ids
        .iter()
        .copied()
        .filter(|fact_id| forbidden.contains(fact_id))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let returned_support_fact_ids = returned_ids
        .iter()
        .copied()
        .filter(|fact_id| allowed_support.contains(fact_id))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let returned_candidate_fact_ids = returned_candidate_ids
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let reciprocal_rank_at_five = returned
        .iter()
        .filter(|hit| expected.contains(hit.fact_id.as_str()))
        .map(|hit| hit.rank)
        .min()
        .map(|rank| 1.0 / f64::from(rank));
    let expected_group_count = case.expected_groups.len();
    let missing_group_count = expected_group_count.saturating_sub(found_group_count);
    let provenance_error_count = baseline.provenance_errors;
    let stage_attribution = attribute_retrieval_stages(
        case,
        &returned_ids,
        &baseline_audit,
        &expected_audit_nodes,
        audit_stable,
    );
    let passed = missing_group_count == 0
        && unexpected_fact_ids.is_empty()
        && returned_forbidden_fact_ids.is_empty()
        && provenance_error_count == 0
        && duplicate_violation_count == 0
        && stage_attribution_is_valid(&stage_attribution)
        && repeat_stable
        && top_k_prefix_stable
        && insertion_order_stable;
    RetrievalCaseReport {
        id: case.id.clone(),
        split: case.split,
        tags: case.tags.clone(),
        expected_group_count: u32::try_from(expected_group_count).unwrap_or(u32::MAX),
        found_group_count: u32::try_from(found_group_count).unwrap_or(u32::MAX),
        effective_found_group_count: u32::try_from(effective_found_group_count).unwrap_or(u32::MAX),
        reciprocal_rank_at_five,
        returned_fact_ids: returned_ids.into_iter().map(str::to_owned).collect(),
        returned_support_fact_ids,
        returned_candidate_fact_ids,
        missing_group_count: u32::try_from(missing_group_count).unwrap_or(u32::MAX),
        candidate_only_group_count: u32::try_from(candidate_only_group_count).unwrap_or(u32::MAX),
        unexpected_fact_ids,
        forbidden_fact_ids: returned_forbidden_fact_ids,
        provenance_error_count,
        duplicate_violation_count: u32::try_from(duplicate_violation_count).unwrap_or(u32::MAX),
        stage_attribution,
        repeat_stable,
        top_k_prefix_stable,
        insertion_order_stable,
        elapsed_ms,
        passed,
    }
}

fn stage_attribution_is_valid(stage: &StageAttribution) -> bool {
    stage.mapping_error_count == 0 && stage.audit_complete && stage.audit_stable
}

fn attribute_retrieval_stages(
    case: &FixtureCase,
    returned_ids: &[&str],
    calls: &[MaskAuditCall],
    expected_audit_nodes: &BTreeSet<FixtureNode>,
    audit_stable: bool,
) -> StageAttribution {
    let mut candidate_ids = HashSet::new();
    let mut surviving_ids = HashSet::new();
    let mut emitted_before_revalidation = HashSet::new();
    let mut mapping_error_count = 0_u32;
    let observed_nodes = calls.iter().map(|call| call.node).collect::<BTreeSet<_>>();
    let audit_complete =
        &observed_nodes == expected_audit_nodes && observed_nodes.len() == calls.len();
    for call in calls {
        let mut accepted_in_call = 0_usize;
        for candidate in &call.candidates {
            let emitted = candidate.decision == EvidenceDecision::Relevant
                && accepted_in_call < usize::from(TOP_K);
            if candidate.decision == EvidenceDecision::Relevant {
                accepted_in_call = accepted_in_call.saturating_add(1);
            }
            let Some(fact_id) = candidate.fact_id.as_deref() else {
                mapping_error_count = mapping_error_count.saturating_add(1);
                continue;
            };
            candidate_ids.insert(fact_id);
            if candidate.decision == EvidenceDecision::Relevant {
                surviving_ids.insert(fact_id);
                if emitted {
                    emitted_before_revalidation.insert(fact_id);
                }
            }
        }
    }

    let returned_ids = returned_ids.iter().copied().collect::<HashSet<_>>();
    let allowed_ids = case
        .expected_groups
        .iter()
        .flatten()
        .chain(&case.allowed_support_fact_ids)
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let forbidden_ids = case
        .forbidden_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let group_has = |group: &[String], facts: &HashSet<&str>| {
        group.iter().any(|fact_id| facts.contains(fact_id.as_str()))
    };

    let source_candidate_group_count = case
        .expected_groups
        .iter()
        .filter(|group| group_has(group, &candidate_ids))
        .count();
    let mask_surviving_group_count = case
        .expected_groups
        .iter()
        .filter(|group| group_has(group, &surviving_ids))
        .count();
    let mut not_retrieved_group_count = 0_usize;
    let mut rejected_by_mask_group_count = 0_usize;
    let mut outside_top_k_group_count = 0_usize;
    let mut revalidation_loss_group_count = 0_usize;
    for group in case
        .expected_groups
        .iter()
        .filter(|group| !group_has(group, &returned_ids))
    {
        if !group_has(group, &candidate_ids) {
            not_retrieved_group_count = not_retrieved_group_count.saturating_add(1);
        } else if !group_has(group, &surviving_ids) {
            rejected_by_mask_group_count = rejected_by_mask_group_count.saturating_add(1);
        } else if !group_has(group, &emitted_before_revalidation) {
            outside_top_k_group_count = outside_top_k_group_count.saturating_add(1);
        } else {
            revalidation_loss_group_count = revalidation_loss_group_count.saturating_add(1);
        }
    }

    StageAttribution {
        source_candidate_group_count: u32::try_from(source_candidate_group_count)
            .unwrap_or(u32::MAX),
        mask_surviving_group_count: u32::try_from(mask_surviving_group_count).unwrap_or(u32::MAX),
        not_retrieved_group_count: u32::try_from(not_retrieved_group_count).unwrap_or(u32::MAX),
        rejected_by_mask_group_count: u32::try_from(rejected_by_mask_group_count)
            .unwrap_or(u32::MAX),
        outside_top_k_group_count: u32::try_from(outside_top_k_group_count).unwrap_or(u32::MAX),
        revalidation_loss_group_count: u32::try_from(revalidation_loss_group_count)
            .unwrap_or(u32::MAX),
        unexpected_survivor_count: u32::try_from(surviving_ids.difference(&allowed_ids).count())
            .unwrap_or(u32::MAX),
        hard_negative_source_candidate_count: u32::try_from(
            candidate_ids.intersection(&forbidden_ids).count(),
        )
        .unwrap_or(u32::MAX),
        mapping_error_count,
        audit_complete,
        audit_stable,
    }
}

fn run_is_prefix(short: &NormalizedRun, long: &NormalizedRun) -> bool {
    short.provenance_errors == long.provenance_errors
        && short.sources.len() == long.sources.len()
        && short
            .sources
            .iter()
            .zip(&long.sources)
            .all(|(short_source, long_source)| {
                short_source.node == long_source.node
                    && long_source.hits.starts_with(&short_source.hits)
            })
}

fn aggregate_metrics<'a>(
    reports: impl Iterator<Item = &'a RetrievalCaseReport>,
) -> AggregateMetrics {
    let mut metrics = AggregateMetrics::default();
    let mut reciprocal_rank_sum = 0.0_f64;
    let mut reciprocal_rank_count = 0_u32;
    for report in reports {
        metrics.case_count = metrics.case_count.saturating_add(1);
        metrics.expected_group_count = metrics
            .expected_group_count
            .saturating_add(report.expected_group_count);
        metrics.found_group_count = metrics
            .found_group_count
            .saturating_add(report.found_group_count);
        metrics.effective_found_group_count = metrics
            .effective_found_group_count
            .saturating_add(report.effective_found_group_count);
        metrics.candidate_only_group_count = metrics
            .candidate_only_group_count
            .saturating_add(report.candidate_only_group_count);
        metrics.false_evidence_count = metrics
            .false_evidence_count
            .saturating_add(u32::try_from(report.unexpected_fact_ids.len()).unwrap_or(u32::MAX));
        metrics.forbidden_evidence_count = metrics
            .forbidden_evidence_count
            .saturating_add(u32::try_from(report.forbidden_fact_ids.len()).unwrap_or(u32::MAX));
        metrics.provenance_error_count = metrics
            .provenance_error_count
            .saturating_add(report.provenance_error_count);
        metrics.duplicate_violation_count = metrics
            .duplicate_violation_count
            .saturating_add(report.duplicate_violation_count);
        if !report.repeat_stable || !report.top_k_prefix_stable || !report.insertion_order_stable {
            metrics.unstable_case_count = metrics.unstable_case_count.saturating_add(1);
        }
        if !stage_attribution_is_valid(&report.stage_attribution) {
            metrics.stage_audit_error_count = metrics.stage_audit_error_count.saturating_add(1);
        }
        if report.expected_group_count > 0 {
            reciprocal_rank_sum += report.reciprocal_rank_at_five.unwrap_or(0.0);
            reciprocal_rank_count = reciprocal_rank_count.saturating_add(1);
        }
    }
    metrics.recall_at_five = (metrics.expected_group_count > 0)
        .then(|| f64::from(metrics.found_group_count) / f64::from(metrics.expected_group_count));
    metrics.effective_recall_at_five = (metrics.expected_group_count > 0).then(|| {
        f64::from(metrics.effective_found_group_count) / f64::from(metrics.expected_group_count)
    });
    metrics.mean_reciprocal_rank_at_five =
        (reciprocal_rank_count > 0).then(|| reciprocal_rank_sum / f64::from(reciprocal_rank_count));
    metrics
}

fn aggregate_stage_attribution(reports: &[RetrievalCaseReport]) -> StageAttributionSummary {
    let mut summary = StageAttributionSummary::default();
    for report in reports {
        let stage = &report.stage_attribution;
        summary.expected_group_count = summary
            .expected_group_count
            .saturating_add(report.expected_group_count);
        summary.source_candidate_group_count = summary
            .source_candidate_group_count
            .saturating_add(stage.source_candidate_group_count);
        summary.mask_surviving_group_count = summary
            .mask_surviving_group_count
            .saturating_add(stage.mask_surviving_group_count);
        summary.not_retrieved_group_count = summary
            .not_retrieved_group_count
            .saturating_add(stage.not_retrieved_group_count);
        summary.rejected_by_mask_group_count = summary
            .rejected_by_mask_group_count
            .saturating_add(stage.rejected_by_mask_group_count);
        summary.outside_top_k_group_count = summary
            .outside_top_k_group_count
            .saturating_add(stage.outside_top_k_group_count);
        summary.revalidation_loss_group_count = summary
            .revalidation_loss_group_count
            .saturating_add(stage.revalidation_loss_group_count);
        summary.unexpected_survivor_count = summary
            .unexpected_survivor_count
            .saturating_add(stage.unexpected_survivor_count);
        summary.hard_negative_source_candidate_count = summary
            .hard_negative_source_candidate_count
            .saturating_add(stage.hard_negative_source_candidate_count);
        summary.mapping_error_count = summary
            .mapping_error_count
            .saturating_add(stage.mapping_error_count);
        if !stage_attribution_is_valid(stage) {
            summary.audit_error_case_count = summary.audit_error_case_count.saturating_add(1);
        }
    }
    summary.source_candidate_recall_at_ten = (summary.expected_group_count > 0).then(|| {
        f64::from(summary.source_candidate_group_count) / f64::from(summary.expected_group_count)
    });
    summary.mask_surviving_recall_at_ten = (summary.expected_group_count > 0).then(|| {
        f64::from(summary.mask_surviving_group_count) / f64::from(summary.expected_group_count)
    });
    summary
}

fn split_passes(metrics: &AggregateMetrics) -> bool {
    metrics
        .recall_at_five
        .is_some_and(|recall| recall >= MIN_RECALL_AT_FIVE)
        && metrics.false_evidence_count == 0
        && metrics.forbidden_evidence_count == 0
        && metrics.provenance_error_count == 0
        && metrics.duplicate_violation_count == 0
        && metrics.unstable_case_count == 0
        && metrics.stage_audit_error_count == 0
}

fn regression_cases_pass(reports: &[RetrievalCaseReport]) -> bool {
    reports
        .iter()
        .filter(|report| report.split == RetrievalSplit::Regression)
        .all(|report| report.passed)
}

fn validate_model_revisions() -> Result<()> {
    ensure!(
        E5_REVISION == E5_MODEL_REVISION,
        "airwiki-core and airwiki-inference require different embedding revisions"
    );
    ensure!(
        MMARCO_REVISION == MMARCO_RERANKER_REVISION,
        "airwiki-core and airwiki-inference require different relevance revisions"
    );
    Ok(())
}

fn report_path() -> PathBuf {
    workspace_root().join(REPORT_DIRECTORY).join(format!(
        "retrieval-pipeline-{}-{}.json",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

fn write_report(report: &RetrievalEvaluationReport) -> Result<PathBuf> {
    let destination = report_path();
    let parent = destination
        .parent()
        .context("retrieval report has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let temporary = destination.with_extension("json.tmp");
    let mut contents = serde_json::to_string_pretty(report)?;
    contents.push('\n');
    std::fs::write(&temporary, contents)
        .with_context(|| format!("writing {}", temporary.display()))?;
    replace_file(&temporary, &destination)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FixedTestRelevanceProvider {
        decisions: Vec<EvidenceDecision>,
    }

    #[async_trait]
    impl EvidenceRelevanceProvider for FixedTestRelevanceProvider {
        fn profile_id(&self) -> &str {
            "fixed-test-relevance"
        }

        async fn classify(
            &self,
            _question: &str,
            _candidates: &[RelevanceInput],
        ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
            Ok(self.decisions.clone())
        }
    }

    #[tokio::test]
    async fn deterministic_fixture_exercises_the_complete_retrieval_pipeline() {
        let loaded = load_fixture().unwrap();
        let providers = fixture_providers(&loaded.fixture).unwrap();

        let report = run_evaluation(&loaded, providers).await.unwrap();

        assert!(report.passed, "deterministic retrieval report: {report:#?}");
        assert!(report.regression.case_count > 0);
        assert_eq!(
            report.stage_attribution.source_candidate_recall_at_ten,
            Some(1.0)
        );
        assert_eq!(
            report.stage_attribution.mask_surviving_recall_at_ten,
            Some(1.0)
        );
        assert_eq!(report.total.effective_recall_at_five, Some(1.0));
        assert!(report.total.effective_found_group_count >= report.total.found_group_count);
        assert_eq!(report.stage_attribution.mapping_error_count, 0);
    }

    #[tokio::test]
    async fn relevance_audit_preserves_decisions_and_records_no_document_text() {
        let audit = Arc::new(MaskAudit::default());
        let provider = AuditedRelevanceProvider {
            inner: Arc::new(FixedTestRelevanceProvider {
                decisions: vec![EvidenceDecision::Relevant, EvidenceDecision::Irrelevant],
            }),
            facts: Arc::new(HashMap::from([(
                EvidenceLocator {
                    title: "Synthetic title".to_owned(),
                    heading: "Synthetic heading".to_owned(),
                },
                "synthetic_fact".to_owned(),
            )])),
            audit: Arc::clone(&audit),
            node: FixtureNode::Origin,
        };
        let candidates = vec![
            RelevanceInput {
                title: "Synthetic title".to_owned(),
                heading: "Synthetic heading".to_owned(),
                text: "content must not enter audit state".to_owned(),
            },
            RelevanceInput {
                title: "Unmapped title".to_owned(),
                heading: "Unmapped heading".to_owned(),
                text: "other content must not enter audit state".to_owned(),
            },
        ];

        let decisions = provider
            .classify(" private synthetic question ", &candidates)
            .await
            .unwrap();
        let calls = audit.take("private synthetic question").unwrap();

        assert_eq!(
            decisions,
            vec![EvidenceDecision::Relevant, EvidenceDecision::Irrelevant]
        );
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].candidates.len(), 2);
        assert_eq!(
            calls[0].candidates[0].fact_id.as_deref(),
            Some("synthetic_fact")
        );
        assert!(calls[0].candidates[1].fact_id.is_none());
        assert!(audit.take("private synthetic question").unwrap().is_empty());
    }

    #[tokio::test]
    async fn allowed_support_is_not_fed_into_the_fixture_relevance_oracle() {
        let loaded = load_fixture().unwrap();
        let providers = fixture_providers(&loaded.fixture).unwrap();
        let case = loaded
            .fixture
            .cases
            .iter()
            .find(|case| case.id == "regression_atlas_paraphrase_recovery")
            .unwrap();
        let decisions = providers
            .relevance
            .classify(
                &case.question,
                &[RelevanceInput {
                    title: "Proyecto Atlas — recuperación operativa".to_owned(),
                    heading: "Reversión".to_owned(),
                    text: "Si la métrica no vuelve a verde, revierte la configuración y escala el incidente."
                        .to_owned(),
                }],
            )
            .await
            .unwrap();

        assert_eq!(decisions, [EvidenceDecision::Irrelevant]);
    }

    #[test]
    fn fixture_rejects_overlapping_split_domains() {
        let mut loaded = load_fixture().unwrap();
        let case = loaded
            .fixture
            .cases
            .iter_mut()
            .find(|case| case.id == "calibration_aurora_owner")
            .unwrap();
        case.domain = "atlas_acceptance".to_owned();
        let document = loaded
            .fixture
            .documents
            .iter_mut()
            .find(|document| document.id == "aurora_coordination")
            .unwrap();
        document.domain = "atlas_acceptance".to_owned();

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("pairwise disjoint"));
    }

    #[test]
    fn fixture_rejects_headings_that_collide_after_production_truncation() {
        let mut loaded = load_fixture().unwrap();
        let title = loaded
            .fixture
            .documents
            .iter()
            .find(|document| document.id == "atlas_recovery")
            .unwrap()
            .title
            .clone();
        let shared_prefix = "h".repeat(MAX_HEADING_OR_PAGE_CHARS);
        loaded
            .fixture
            .documents
            .iter_mut()
            .find(|document| document.id == "atlas_recovery")
            .unwrap()
            .chunks[0]
            .heading = format!("{shared_prefix}a");
        let target = loaded
            .fixture
            .documents
            .iter_mut()
            .find(|document| document.id == "aurora_coordination")
            .unwrap();
        target.title = title;
        target.chunks[0].heading = format!("{shared_prefix}b");

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("title and normalized heading pairs")
        );
    }

    #[test]
    fn fixture_rejects_cross_domain_evidence() {
        let mut loaded = load_fixture().unwrap();
        let case = loaded
            .fixture
            .cases
            .iter_mut()
            .find(|case| case.id == "holdout_harbor_owner_cross_language")
            .unwrap();
        case.domain = "quasar_security".to_owned();

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("another domain"));
    }

    #[test]
    fn fixture_rejects_related_but_non_answering_evidence() {
        let mut loaded = load_fixture().unwrap();
        let case = loaded
            .fixture
            .cases
            .iter_mut()
            .find(|case| case.id == "regression_atlas_unrelated_injection")
            .unwrap();
        case.relevant_fact_ids
            .push("atlas_recovery_rollback".to_owned());

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("cannot be relevant"));
    }

    #[test]
    fn fixture_rejects_an_unknown_allowed_support_fact() {
        let mut loaded = load_fixture().unwrap();
        loaded.fixture.cases[0]
            .allowed_support_fact_ids
            .push("unknown_fact".to_owned());

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("allowed support"));
        assert!(error.to_string().contains("unknown fact"));
    }

    #[test]
    fn fixture_rejects_expected_evidence_as_allowed_support() {
        let mut loaded = load_fixture().unwrap();
        loaded.fixture.cases[0]
            .allowed_support_fact_ids
            .push("atlas_recovery_procedure".to_owned());

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("cannot be expected"));
    }

    #[test]
    fn fixture_rejects_forbidden_evidence_as_allowed_support() {
        let mut loaded = load_fixture().unwrap();
        let case = loaded
            .fixture
            .cases
            .iter_mut()
            .find(|case| case.id == "regression_atlas_unrelated_injection")
            .unwrap();
        case.allowed_support_fact_ids
            .push("atlas_note_injection".to_owned());

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("cannot be forbidden"));
    }

    #[test]
    fn fixture_rejects_cross_domain_allowed_support() {
        let mut loaded = load_fixture().unwrap();
        let case = loaded
            .fixture
            .cases
            .iter_mut()
            .find(|case| case.id == "calibration_aurora_owner")
            .unwrap();
        case.allowed_support_fact_ids
            .push("cedar_note_authority".to_owned());

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("another domain"));
    }

    #[test]
    fn fixture_rejects_allowed_support_for_a_non_answerable_case() {
        let mut loaded = load_fixture().unwrap();
        let case = loaded
            .fixture
            .cases
            .iter_mut()
            .find(|case| case.id == "regression_atlas_external_ai_policy")
            .unwrap();
        case.allowed_support_fact_ids
            .push("atlas_target_date".to_owned());

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("non-answerable"));
    }

    #[test]
    fn fixture_rejects_a_fact_reused_across_expected_groups() {
        let mut loaded = load_fixture().unwrap();
        let case = loaded
            .fixture
            .cases
            .iter_mut()
            .find(|case| case.id == "regression_atlas_compound_federated")
            .unwrap();
        case.expected_groups.push(vec!["atlas_owner".to_owned()]);

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("pairwise disjoint"));
    }

    #[test]
    fn fixture_requires_non_answer_evidence_to_be_forbidden() {
        let mut loaded = load_fixture().unwrap();
        let case = loaded
            .fixture
            .cases
            .iter_mut()
            .find(|case| case.id == "regression_atlas_external_ai_policy")
            .unwrap();
        case.forbidden_fact_ids.clear();

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("must be forbidden"));
    }

    #[test]
    fn fixture_requires_a_peer_without_grant_case() {
        let mut loaded = load_fixture().unwrap();
        let collection = loaded
            .fixture
            .collections
            .iter_mut()
            .find(|collection| collection.id == "peer_ungranted")
            .unwrap();
        collection.granted_to_origin = true;

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("peer-without-grant"));
    }

    #[test]
    fn peer_without_grant_case_requires_a_shareable_collection() {
        let mut loaded = load_fixture().unwrap();
        let collection = loaded
            .fixture
            .collections
            .iter_mut()
            .find(|collection| collection.id == "peer_ungranted")
            .unwrap();
        collection.peer_shareable = false;

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("peer-without-grant"));
    }

    #[test]
    fn fixture_rejects_an_unknown_expected_fact() {
        let mut loaded = load_fixture().unwrap();
        loaded.fixture.cases[0].expected_groups[0][0] = "unknown_fact".to_owned();

        let error = validate_fixture_data(&loaded.fixture).unwrap_err();

        assert!(error.to_string().contains("unknown fact"));
    }

    #[test]
    fn serialized_report_omits_questions_snippets_paths_and_runtime_identities() {
        let loaded = load_fixture().unwrap();
        let report = RetrievalEvaluationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            fixture_sha256: loaded.sha256,
            target_os: "test".to_owned(),
            target_arch: "test".to_owned(),
            provider: ProviderIdentity {
                embedding_profile: "fixture".to_owned(),
                embedding_revision: "fixture".to_owned(),
                relevance_profile: "fixture".to_owned(),
                relevance_revision: "fixture".to_owned(),
                relevance_artifact_filename: None,
                relevance_artifact_sha256: None,
                thread_count: 1,
            },
            top_k: TOP_K,
            elapsed_ms: 0,
            regression: AggregateMetrics::default(),
            calibration: AggregateMetrics::default(),
            holdout: AggregateMetrics::default(),
            total: AggregateMetrics::default(),
            stage_attribution: StageAttributionSummary::default(),
            passed: true,
            cases: Vec::new(),
        };

        let serialized = serde_json::to_string(&report).unwrap();

        for forbidden in [
            "question",
            "snippet",
            "source_sha256",
            "logical_resource_uri",
            "node_id",
            "peer_id",
            "multiaddress",
            "source_path",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }

    fn score_support_case(returned_fact_ids: &[&str]) -> RetrievalCaseReport {
        let case = FixtureCase {
            id: "support_scoring".to_owned(),
            domain: "support_scoring".to_owned(),
            split: RetrievalSplit::Calibration,
            tags: vec![RetrievalTag::Direct],
            scope: RetrievalScope::Local,
            question: "synthetic support question".to_owned(),
            semantic_keys: vec!["support".to_owned()],
            relevant_fact_ids: vec!["answer".to_owned()],
            expected_groups: vec![vec!["answer".to_owned()]],
            allowed_support_fact_ids: vec!["support".to_owned()],
            forbidden_fact_ids: vec!["forbidden".to_owned()],
        };
        let baseline = NormalizedRun {
            sources: vec![NormalizedSource {
                node: FixtureNode::Origin,
                hits: returned_fact_ids
                    .iter()
                    .enumerate()
                    .map(|(index, fact_id)| NormalizedHit {
                        fact_id: (*fact_id).to_owned(),
                        rank: u32::try_from(index + 1).unwrap_or(u32::MAX),
                    })
                    .collect(),
                authorized_candidates: Vec::new(),
            }],
            provenance_errors: 0,
        };
        score_case(
            &case,
            CaseEvaluationRuns {
                baseline: baseline.clone(),
                repeated: baseline.clone(),
                expanded: baseline.clone(),
                reversed: baseline,
                baseline_audit: Vec::new(),
                expected_audit_nodes: BTreeSet::new(),
                audit_stable: true,
            },
            0,
        )
    }

    #[test]
    fn allowed_support_alone_does_not_satisfy_expected_evidence() {
        let report = score_support_case(&["support"]);

        assert_eq!(report.found_group_count, 0);
        assert_eq!(report.missing_group_count, 1);
        assert_eq!(report.reciprocal_rank_at_five, None);
        assert!(report.unexpected_fact_ids.is_empty());
        assert_eq!(report.returned_support_fact_ids, ["support"]);
        assert!(!report.passed);
    }

    #[test]
    fn allowed_support_before_expected_does_not_improve_expected_rank() {
        let report = score_support_case(&["support", "answer"]);

        assert_eq!(report.found_group_count, 1);
        assert_eq!(report.reciprocal_rank_at_five, Some(0.5));
        assert!(report.unexpected_fact_ids.is_empty());
        assert_eq!(report.returned_support_fact_ids, ["support"]);
    }

    #[test]
    fn unlisted_returned_fact_remains_unexpected() {
        let report = score_support_case(&["answer", "unlisted"]);

        assert_eq!(report.unexpected_fact_ids, ["unlisted"]);
        assert!(!report.passed);
    }

    #[test]
    fn forbidden_returned_fact_remains_forbidden() {
        let report = score_support_case(&["answer", "forbidden"]);

        assert_eq!(report.forbidden_fact_ids, ["forbidden"]);
        assert!(!report.passed);
    }

    #[test]
    fn mrr_includes_answerable_cases_without_a_hit_as_zero() {
        fn report(expected: u32, reciprocal_rank: Option<f64>) -> RetrievalCaseReport {
            RetrievalCaseReport {
                id: "synthetic_case".to_owned(),
                split: RetrievalSplit::Calibration,
                tags: vec![RetrievalTag::Direct],
                expected_group_count: expected,
                effective_found_group_count: expected,
                found_group_count: u32::from(reciprocal_rank.is_some()),
                reciprocal_rank_at_five: reciprocal_rank,
                returned_fact_ids: Vec::new(),
                returned_support_fact_ids: Vec::new(),
                returned_candidate_fact_ids: Vec::new(),
                candidate_only_group_count: 0,
                missing_group_count: u32::from(reciprocal_rank.is_none()),
                unexpected_fact_ids: Vec::new(),
                forbidden_fact_ids: Vec::new(),
                provenance_error_count: 0,
                duplicate_violation_count: 0,
                stage_attribution: StageAttribution::default(),
                repeat_stable: true,
                top_k_prefix_stable: true,
                insertion_order_stable: true,
                elapsed_ms: 0,
                passed: reciprocal_rank.is_some(),
            }
        }

        let reports = [report(1, Some(1.0)), report(1, None), report(0, None)];
        let metrics = aggregate_metrics(reports.iter());

        assert_eq!(metrics.mean_reciprocal_rank_at_five, Some(0.5));
    }

    #[test]
    fn split_rejects_an_invalid_stage_audit() {
        let metrics = AggregateMetrics {
            recall_at_five: Some(1.0),
            stage_audit_error_count: 1,
            ..AggregateMetrics::default()
        };

        assert!(!split_passes(&metrics));
    }

    #[test]
    fn every_regression_case_must_pass_individually() {
        fn report(id: &str, split: RetrievalSplit, passed: bool) -> RetrievalCaseReport {
            RetrievalCaseReport {
                id: id.to_owned(),
                split,
                tags: vec![RetrievalTag::Direct],
                expected_group_count: 1,
                effective_found_group_count: u32::from(passed),
                found_group_count: u32::from(passed),
                reciprocal_rank_at_five: passed.then_some(1.0),
                returned_fact_ids: Vec::new(),
                returned_support_fact_ids: Vec::new(),
                returned_candidate_fact_ids: Vec::new(),
                candidate_only_group_count: 0,
                missing_group_count: u32::from(!passed),
                unexpected_fact_ids: Vec::new(),
                forbidden_fact_ids: Vec::new(),
                provenance_error_count: 0,
                duplicate_violation_count: 0,
                stage_attribution: StageAttribution::default(),
                repeat_stable: true,
                top_k_prefix_stable: true,
                insertion_order_stable: true,
                elapsed_ms: 0,
                passed,
            }
        }

        let reports = [
            report("regression_pass", RetrievalSplit::Regression, true),
            report("regression_fail", RetrievalSplit::Regression, false),
            report("calibration_fail", RetrievalSplit::Calibration, false),
        ];

        assert!(!regression_cases_pass(&reports));
    }

    #[test]
    fn stage_attribution_partitions_each_missing_group_once() {
        let case = FixtureCase {
            id: "stage_partition".to_owned(),
            domain: "stage_partition".to_owned(),
            split: RetrievalSplit::Calibration,
            tags: vec![RetrievalTag::Compound],
            scope: RetrievalScope::Local,
            question: "synthetic stage question".to_owned(),
            semantic_keys: vec!["stage".to_owned()],
            relevant_fact_ids: [
                "not_retrieved",
                "masked",
                "outside",
                "revalidated",
                "returned",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            expected_groups: [
                "not_retrieved",
                "masked",
                "outside",
                "revalidated",
                "returned",
            ]
            .into_iter()
            .map(|fact| vec![fact.to_owned()])
            .collect(),
            allowed_support_fact_ids: Vec::new(),
            forbidden_fact_ids: Vec::new(),
        };
        let candidate = |fact_id: &str, decision| AuditedCandidate {
            fact_id: Some(fact_id.to_owned()),
            decision,
        };
        let calls = vec![MaskAuditCall {
            node: FixtureNode::Origin,
            candidates: vec![
                candidate("masked", EvidenceDecision::Irrelevant),
                candidate("revalidated", EvidenceDecision::Relevant),
                candidate("returned", EvidenceDecision::Relevant),
                candidate("filler_1", EvidenceDecision::Relevant),
                candidate("filler_2", EvidenceDecision::Relevant),
                candidate("filler_3", EvidenceDecision::Relevant),
                candidate("outside", EvidenceDecision::Relevant),
            ],
        }];

        let attribution = attribute_retrieval_stages(
            &case,
            &["returned"],
            &calls,
            &BTreeSet::from([FixtureNode::Origin]),
            true,
        );

        assert_eq!(attribution.source_candidate_group_count, 4);
        assert_eq!(attribution.mask_surviving_group_count, 3);
        assert_eq!(attribution.not_retrieved_group_count, 1);
        assert_eq!(attribution.rejected_by_mask_group_count, 1);
        assert_eq!(attribution.outside_top_k_group_count, 1);
        assert_eq!(attribution.revalidation_loss_group_count, 1);
        assert_eq!(
            attribution.not_retrieved_group_count
                + attribution.rejected_by_mask_group_count
                + attribution.outside_top_k_group_count
                + attribution.revalidation_loss_group_count,
            4
        );
        assert_eq!(attribution.mapping_error_count, 0);
        assert!(attribution.audit_complete);
        assert!(attribution.audit_stable);
    }

    #[test]
    fn allowed_support_does_not_count_as_an_unexpected_mask_survivor() {
        let case = FixtureCase {
            id: "support_stage".to_owned(),
            domain: "support_stage".to_owned(),
            split: RetrievalSplit::Calibration,
            tags: vec![RetrievalTag::Direct],
            scope: RetrievalScope::Local,
            question: "synthetic support stage question".to_owned(),
            semantic_keys: vec!["support".to_owned()],
            relevant_fact_ids: vec!["answer".to_owned()],
            expected_groups: vec![vec!["answer".to_owned()]],
            allowed_support_fact_ids: vec!["support".to_owned()],
            forbidden_fact_ids: Vec::new(),
        };
        let calls = vec![MaskAuditCall {
            node: FixtureNode::Origin,
            candidates: vec![
                AuditedCandidate {
                    fact_id: Some("support".to_owned()),
                    decision: EvidenceDecision::Relevant,
                },
                AuditedCandidate {
                    fact_id: Some("answer".to_owned()),
                    decision: EvidenceDecision::Relevant,
                },
            ],
        }];

        let attribution = attribute_retrieval_stages(
            &case,
            &["support", "answer"],
            &calls,
            &BTreeSet::from([FixtureNode::Origin]),
            true,
        );

        assert_eq!(attribution.source_candidate_group_count, 1);
        assert_eq!(attribution.mask_surviving_group_count, 1);
        assert_eq!(attribution.unexpected_survivor_count, 0);
    }

    #[test]
    fn forbidden_survivor_is_unexpected_for_a_non_answerable_case() {
        let case = FixtureCase {
            id: "forbidden_stage".to_owned(),
            domain: "forbidden_stage".to_owned(),
            split: RetrievalSplit::Calibration,
            tags: vec![RetrievalTag::Absence],
            scope: RetrievalScope::Local,
            question: "synthetic forbidden stage question".to_owned(),
            semantic_keys: vec!["forbidden".to_owned()],
            relevant_fact_ids: vec!["forbidden".to_owned()],
            expected_groups: Vec::new(),
            allowed_support_fact_ids: Vec::new(),
            forbidden_fact_ids: vec!["forbidden".to_owned()],
        };
        let calls = vec![MaskAuditCall {
            node: FixtureNode::Origin,
            candidates: vec![AuditedCandidate {
                fact_id: Some("forbidden".to_owned()),
                decision: EvidenceDecision::Relevant,
            }],
        }];

        let attribution = attribute_retrieval_stages(
            &case,
            &[],
            &calls,
            &BTreeSet::from([FixtureNode::Origin]),
            true,
        );

        assert_eq!(attribution.unexpected_survivor_count, 1);
        assert_eq!(attribution.hard_negative_source_candidate_count, 1);
    }

    #[test]
    fn evaluation_workspace_is_removed_when_its_guard_drops() {
        let workspace = EvaluationWorkspace::create().unwrap();
        let path = workspace.path().to_path_buf();
        assert!(path.is_dir());

        drop(workspace);

        assert!(!path.exists());
    }
}
