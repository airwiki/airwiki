//! Real-ranking development replay for the compact OKF concept graph.
//!
//! Unlike the mechanistic fixture in `mini_graph`, this evaluator obtains its
//! candidate order from AirWiki's production BM25/E5/RRF implementation and
//! obtains every edge by inspecting materialized, healthy OKF bundles. It is
//! still a development experiment: it does not change production search.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use airwiki_core::{
    Database, DeterministicEvidenceRelevanceProvider, EMBEDDING_DIMENSIONS, EmbeddingProvider,
    EvidenceRelevanceProvider, FastEmbedE5Small, FastEmbedMmarcoReranker, HybridSearchEngine,
    KnowledgeBundleState, KnowledgeLinkDisposition, KnowledgePageId, OkfBundleInspector,
    OkfPublicationMaterializer, PinnedE5Snapshot, PinnedMmarcoRerankerSnapshot,
    RetrievalEvaluationCandidate, RetrievalEvaluationNominee, StoredChunk,
};
use airwiki_types::{
    CollectionPolicy, ConceptType, DocumentStatus, EnrichmentDraft, SearchPurpose, SearchRequest,
    SuggestedLink,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::EvaluationWorkspace;
use super::mini_graph::{
    BASELINE_LIMIT, CONTROL_LIMIT, ExpansionDirection, GraphLinkInput, GraphNode, GraphNodeInput,
    GraphPurpose, LinkDisposition, MiniGraph, NodeState, QueryScope,
};
use super::sham_graph::{StructuralShamStats, build_structural_sham};
use crate::{replace_file, workspace_root};

const FIXTURE_PATH: &str = "fixtures/retrieval/mini-graph-real-development-v1.json";
const FINAL_FIXTURE_PATH: &str = "fixtures/retrieval/mini-graph-final-holdout-v1.json";
const FINAL_FIXTURE_SHA256: &str =
    "96c0efbe5acdfbe77f4c3c7bece68b7991d0066a721b54bb90b055ba02e9383d";
const REPORT_DIRECTORY: &str = "target/evals";
const FIXTURE_SCHEMA_VERSION: u32 = 1;
const REPORT_SCHEMA_VERSION: u32 = 2;
const MIN_DOMAIN_COUNT: usize = 4;
const MIN_CASES_PER_DOMAIN: usize = 3;
const MAX_DISTRACTORS_PER_DOMAIN: usize = 64;
const MAX_DOCUMENTS: usize = 500;
const MAX_RETAINED_PAYLOAD_BYTES: usize = 1024 * 1024;
const MIN_GROUP_GAIN: u32 = 2;
const MIN_IMPROVED_DOMAIN_COUNT: u32 = 3;
const MIN_RESCUE_DOMAIN_COUNT: u32 = 2;
const NODE_ID: &str = "real-graph-development";
const FINAL_NODE_ID: &str = "final-graph-holdout";

const FINAL_DOMAIN_COUNT: usize = 8;
const FINAL_CASES_PER_DOMAIN: usize = 5;
const FINAL_DOCUMENTS_PER_DOMAIN: usize = 6;
const FINAL_SECTIONS_PER_DOCUMENT: usize = 3;
const FINAL_MIN_DISTRACTORS_PER_DOMAIN: usize = 24;
const FINAL_MAX_DISTRACTORS_PER_DOMAIN: usize = 40;
const FINAL_TOP_K: u8 = 5;
const FINAL_MIN_GROUP_RECALL: f64 = 0.90;
const FINAL_MIN_CITATION_PRECISION: f64 = 0.80;
const FINAL_MIN_MACRO_GAIN: f64 = 0.05;
const FINAL_MIN_IMPROVED_DOMAINS: usize = 5;
const FINAL_MAX_GRAPH_PAYLOAD_BYTES: usize = 1024 * 1024;
const FINAL_MAX_PROJECTION_MICROS: u128 = 1_000_000;
const FINAL_MAX_ASSEMBLY_P95_MICROS: u128 = 25_000;
const FINAL_MAX_FULL_QUERY_P95_MICROS: u128 = 3_000_000;
const FINAL_FULL_QUERY_ABSOLUTE_SLACK_MICROS: u128 = 10_000;
const FINAL_FULL_QUERY_RELATIVE_SLACK: f64 = 1.10;
const FINAL_BOOTSTRAP_RESAMPLES: usize = 10_000;
const FINAL_BOOTSTRAP_SEED: u64 = 0x41_49_52_57_49_4b_49;
const FINAL_EPSILON: f64 = 1e-12;
const SHAM_CONTRACT_VERSION: &str = "deterministic-min-cost-degree-preserving-v1.1";
const FINAL_POLICY: &str = "airwiki-final-graph-holdout-v1.1;warmup=one-unscored;arm-order=latin-square;ranking=bm25-e5-rrf-32;seeds=first-10-exact;expansion=one-hop-bidirectional-edge-neighbors;graph-nominee=concept-current-up-to-2-chunks;backfill=remaining-b32-exact;nominee-limit=32;assembly=expansion-plus-candidate-preparation;sham=deterministic-min-cost-degree-preserving-v1.1;sham-forced-original-edges=retained;final=mMARCOReranker-top5;bootstrap=paired-domain-10000-seed-0x41495257494b49";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplayFixture {
    schema_version: u32,
    experiment_id: String,
    domains: Vec<DomainFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DomainFixture {
    id: String,
    collection_name: String,
    language: String,
    distractor_count: usize,
    distractor_title: String,
    distractor_description: String,
    #[serde(default)]
    distractor_heading: Option<String>,
    #[serde(default)]
    distractor_text: Option<String>,
    #[serde(default)]
    distractor_sections: Vec<SectionFixture>,
    documents: Vec<DocumentFixture>,
    cases: Vec<CaseFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentFixture {
    id: String,
    title: String,
    description: String,
    #[serde(default)]
    heading: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    sections: Vec<SectionFixture>,
    links: Vec<DocumentLinkFixture>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SectionFixture {
    id: String,
    heading: String,
    text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentLinkFixture {
    label: String,
    target_document_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFixture {
    id: String,
    question: String,
    #[serde(default)]
    required_document_groups: Vec<Vec<String>>,
    #[serde(default)]
    expected_answerable: Option<bool>,
    #[serde(default)]
    required_evidence_groups: Vec<Vec<EvidenceId>>,
    #[serde(default)]
    forbidden_evidence: Vec<EvidenceId>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
struct EvidenceId {
    document_id: String,
    section_id: String,
}

#[derive(Debug)]
struct LoadedFixture {
    fixture: ReplayFixture,
    sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureProfile {
    Development,
    FinalHoldout,
}

#[derive(Debug, Clone)]
struct SeedDocument {
    logical_id: String,
    collection_id: Uuid,
    source_id: Uuid,
    concept_id: Uuid,
    draft: EnrichmentDraft,
    source_sha256: String,
    sections: Vec<SeedSection>,
    links: Vec<DocumentLinkFixture>,
}

#[derive(Debug, Clone)]
struct SeedSection {
    logical_id: String,
    heading: String,
    text: String,
}

#[derive(Debug)]
struct ExpectedCitation {
    evidence: EvidenceId,
    collection_id: Uuid,
    concept_id: Uuid,
    source_revision: u32,
    source_sha256: String,
    logical_resource_uri: String,
    title: String,
    heading_or_page: String,
}

struct ReplayCorpus {
    database: Database,
    engine: HybridSearchEngine,
    graph: MiniGraph,
    sham_graph: MiniGraph,
    sham_stats: StructuralShamStats,
    logical_by_concept: HashMap<Uuid, String>,
    concept_by_logical: HashMap<String, Uuid>,
    expected_by_chunk: HashMap<Uuid, ExpectedCitation>,
    revision_by_concept: HashMap<Uuid, u32>,
    collection_ids: BTreeSet<Uuid>,
    bundle_fingerprints: BTreeMap<Uuid, String>,
    projection_micros: u128,
    _workspace: EvaluationWorkspace,
}

#[derive(Debug, Clone, Serialize)]
struct ArmReport {
    candidate_count: u32,
    found_group_count: u32,
    required_group_count: u32,
    recall: Option<f64>,
    support_candidate_count: u32,
    support_density: Option<f64>,
    candidate_document_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CaseReport {
    id: String,
    domain: String,
    c10: ArmReport,
    c32: ArmReport,
    g1_out: ArmReport,
    g1_bidir: ArmReport,
    g1_sham: ArmReport,
    outgoing_unique_group_count_over_c32: u32,
    bidir_unique_group_count_over_outgoing: u32,
    sham_unique_group_count_over_c32: u32,
    lost_group_count_against_c32: u32,
}

#[derive(Debug, Default, Serialize)]
struct AggregateArm {
    candidate_count: u32,
    found_group_count: u32,
    required_group_count: u32,
    support_candidate_count: u32,
    recall: Option<f64>,
    support_density: Option<f64>,
}

#[derive(Debug, Serialize)]
struct ReplayReport {
    schema_version: u32,
    experiment_id: String,
    fixture_sha256: String,
    embedding_profile: String,
    target_os: String,
    target_arch: String,
    domain_count: u32,
    case_count: u32,
    concept_count: u32,
    edge_count: u32,
    graph_fingerprint: String,
    sham_graph_fingerprint: String,
    sham_contract_version: String,
    sham_linked_collection_count: u32,
    sham_retained_original_edge_count: u32,
    sham_rewired_edge_count: u32,
    sham_unchanged_collection_count: u32,
    retained_payload_bytes: usize,
    projection_micros: u128,
    ranking_and_expansion_micros: u128,
    c10: AggregateArm,
    c32: AggregateArm,
    g1_out: AggregateArm,
    g1_bidir: AggregateArm,
    g1_sham: AggregateArm,
    outgoing_unique_group_count_over_c32: u32,
    bidir_unique_group_count_over_outgoing: u32,
    sham_unique_group_count_over_c32: u32,
    improved_domain_count_over_controls: u32,
    outgoing_rescue_domain_count: u32,
    backlink_rescue_domain_count: u32,
    regressed_case_count_against_c32: u32,
    healthy_fingerprint_gate_passed: bool,
    development_gate_passed: bool,
    production_promotion_ready: bool,
    rejection_reasons: Vec<String>,
    cases: Vec<CaseReport>,
}

#[derive(Debug, Clone, Copy)]
struct DevelopmentGateMetrics {
    healthy_fingerprint: bool,
    c32_found_groups: u32,
    c32_support_density: Option<f64>,
    bidir_found_groups: u32,
    bidir_support_density: Option<f64>,
    sham_found_groups: u32,
    outgoing_unique_groups: u32,
    backlink_unique_groups: u32,
    sham_unique_groups: u32,
    improved_domains: u32,
    outgoing_rescue_domains: u32,
    backlink_rescue_domains: u32,
    regressed_cases: u32,
    retained_payload_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalArmKind {
    B32,
    Graph,
    Sham,
}

#[derive(Debug)]
struct FinalArmExecution {
    candidates: Vec<RetrievalEvaluationCandidate>,
    provider_failure: bool,
    candidate_preparation_micros: u128,
    full_query_micros: u128,
}

#[derive(Debug, Clone, Serialize)]
struct FinalArmReport {
    returned_citation_count: u32,
    unknown_citation_count: u32,
    found_group_count: u32,
    required_group_count: u32,
    group_recall_at_five: Option<f64>,
    reciprocal_rank_at_five: Option<f64>,
    relevant_citation_count: u32,
    citation_precision: Option<f64>,
    forbidden_citation_count: u32,
    no_answer_correct: Option<bool>,
    provenance_exact_and_current: bool,
    provider_failure: bool,
    full_query_micros: u128,
    evidence_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct FinalCaseReport {
    id: String,
    domain: String,
    expected_answerable: bool,
    arm_order: [String; 3],
    g1_candidate_assembly_micros: u128,
    b32: FinalArmReport,
    g1: FinalArmReport,
    g1_sham: FinalArmReport,
    g1_lost_b32_group_count: u32,
}

#[derive(Debug, Default, Serialize)]
struct FinalAggregateArm {
    returned_citation_count: u32,
    found_group_count: u32,
    required_group_count: u32,
    group_recall_at_five: Option<f64>,
    mean_reciprocal_rank_at_five: Option<f64>,
    relevant_citation_count: u32,
    citation_precision: Option<f64>,
    forbidden_citation_count: u32,
    no_answer_correct_count: u32,
    no_answer_case_count: u32,
    no_answer_accuracy: Option<f64>,
    provider_failure_count: u32,
    provenance_exact_and_current: bool,
    full_query_p95_micros: u128,
}

#[derive(Debug, Serialize)]
struct FinalDomainReport {
    id: String,
    b32_group_recall_at_five: f64,
    g1_group_recall_at_five: f64,
    sham_group_recall_at_five: f64,
    g1_improved_over_both: bool,
}

#[derive(Debug, Serialize)]
struct FinalReplayReport {
    schema_version: u32,
    experiment_id: String,
    fixture_sha256: String,
    evaluation_policy_fingerprint: String,
    embedding_profile: String,
    relevance_profile: String,
    target_os: String,
    target_arch: String,
    domain_count: u32,
    case_count: u32,
    concept_count: u32,
    edge_count: u32,
    graph_fingerprint: String,
    sham_graph_fingerprint: String,
    sham_contract_version: String,
    sham_linked_collection_count: u32,
    sham_retained_original_edge_count: u32,
    sham_rewired_edge_count: u32,
    sham_unchanged_collection_count: u32,
    bundle_set_fingerprint: String,
    retained_payload_bytes: usize,
    projection_micros: u128,
    g1_candidate_assembly_p95_micros: u128,
    b32: FinalAggregateArm,
    g1: FinalAggregateArm,
    g1_sham: FinalAggregateArm,
    b32_macro_domain_recall: f64,
    g1_macro_domain_recall: f64,
    sham_macro_domain_recall: f64,
    g1_macro_gain_over_b32: f64,
    g1_macro_gain_over_sham: f64,
    improved_domain_count_over_both: u32,
    paired_bootstrap_lower_bound_over_b32: f64,
    paired_bootstrap_lower_bound_over_sham: f64,
    g1_lost_b32_group_count: u32,
    healthy_fingerprint_gate_passed: bool,
    shadow_eligible: bool,
    production_promotion_ready: bool,
    rejection_reasons: Vec<String>,
    domains: Vec<FinalDomainReport>,
    cases: Vec<FinalCaseReport>,
}

pub(crate) async fn evaluate_real_mini_graph(embedding_snapshot: &Path) -> Result<()> {
    let loaded = load_fixture()?;
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 4);
    let embeddings: Arc<dyn EmbeddingProvider> = Arc::new(FastEmbedE5Small::from_snapshot(
        &PinnedE5Snapshot::open(embedding_snapshot)?,
        threads,
    )?);
    let embedding_profile = embeddings.model_id().to_owned();
    let relevance: Arc<dyn EvidenceRelevanceProvider> =
        Arc::new(DeterministicEvidenceRelevanceProvider);
    let corpus = build_corpus(&loaded.fixture, Arc::clone(&embeddings), relevance, NODE_ID).await?;
    let report = run_replay(&loaded, &corpus, embedding_profile).await?;
    let destination = write_report(&report)?;
    ensure!(
        report.development_gate_passed,
        "real-ranking mini-graph did not pass its development gate; report written to {}",
        destination.display()
    );
    println!(
        "real-ranking mini-graph passed its development gate; report written to {} (production promotion remains disabled)",
        destination.display()
    );
    Ok(())
}

pub(crate) async fn evaluate_final_mini_graph(
    embedding_snapshot: &Path,
    relevance_snapshot: &Path,
) -> Result<()> {
    let loaded = load_final_fixture()?;
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 4);
    let embeddings: Arc<dyn EmbeddingProvider> = Arc::new(FastEmbedE5Small::from_snapshot(
        &PinnedE5Snapshot::open(embedding_snapshot)?,
        threads,
    )?);
    let relevance: Arc<dyn EvidenceRelevanceProvider> =
        Arc::new(FastEmbedMmarcoReranker::from_snapshot(
            PinnedMmarcoRerankerSnapshot::open(relevance_snapshot)?,
            threads,
        )?);
    let embedding_profile = embeddings.model_id().to_owned();
    let relevance_profile = relevance.profile_id().to_owned();
    let corpus = build_corpus(
        &loaded.fixture,
        Arc::clone(&embeddings),
        relevance,
        FINAL_NODE_ID,
    )
    .await?;
    warm_final_evaluator(&corpus).await?;
    let report = run_final_replay(&loaded, &corpus, embedding_profile, relevance_profile).await?;
    let destination = write_final_report(&report)?;
    ensure!(
        report.shadow_eligible,
        "final mini-graph holdout did not pass its frozen shadow gate; report written to {}",
        destination.display()
    );
    println!(
        "final mini-graph holdout passed its frozen shadow gate; report written to {} (production promotion remains disabled)",
        destination.display()
    );
    Ok(())
}

fn load_fixture() -> Result<LoadedFixture> {
    load_fixture_from(FIXTURE_PATH, FixtureProfile::Development)
}

fn load_final_fixture() -> Result<LoadedFixture> {
    load_fixture_from(FINAL_FIXTURE_PATH, FixtureProfile::FinalHoldout)
}

fn load_fixture_from(relative_path: &str, profile: FixtureProfile) -> Result<LoadedFixture> {
    let path = workspace_root().join(relative_path);
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    validate_fixture_hash(profile, &sha256)?;
    let fixture = serde_json::from_slice::<ReplayFixture>(&bytes)
        .with_context(|| format!("parsing {}", path.display()))?;
    validate_fixture(&fixture, profile)?;
    Ok(LoadedFixture { fixture, sha256 })
}

fn validate_fixture_hash(profile: FixtureProfile, sha256: &str) -> Result<()> {
    if profile == FixtureProfile::FinalHoldout {
        ensure!(
            sha256 == FINAL_FIXTURE_SHA256,
            "sealed final holdout hash mismatch"
        );
    }
    Ok(())
}

fn validate_fixture(fixture: &ReplayFixture, profile: FixtureProfile) -> Result<()> {
    ensure!(
        fixture.schema_version == FIXTURE_SCHEMA_VERSION,
        "unsupported real-ranking mini-graph fixture schema"
    );
    validate_identifier(&fixture.experiment_id)?;
    match profile {
        FixtureProfile::Development => ensure!(
            fixture.domains.len() >= MIN_DOMAIN_COUNT,
            "real-ranking mini-graph fixture needs at least {MIN_DOMAIN_COUNT} domains"
        ),
        FixtureProfile::FinalHoldout => ensure!(
            fixture.domains.len() == FINAL_DOMAIN_COUNT,
            "final graph holdout must contain exactly {FINAL_DOMAIN_COUNT} domains"
        ),
    }
    let mut domain_ids = BTreeSet::new();
    let mut all_document_ids = BTreeSet::new();
    let mut all_case_ids = BTreeSet::new();
    let mut total_documents = 0_usize;
    let mut final_language_counts = BTreeMap::<&str, usize>::new();
    for domain in &fixture.domains {
        validate_identifier(&domain.id)?;
        ensure!(domain_ids.insert(domain.id.as_str()), "duplicate domain id");
        ensure!(
            !domain.collection_name.trim().is_empty(),
            "empty collection name"
        );
        ensure!(!domain.language.trim().is_empty(), "empty language");
        match profile {
            FixtureProfile::Development => {
                ensure!(
                    domain.distractor_count <= MAX_DISTRACTORS_PER_DOMAIN,
                    "domain exceeds distractor limit"
                );
                ensure!(
                    domain.cases.len() >= MIN_CASES_PER_DOMAIN,
                    "domain needs at least {MIN_CASES_PER_DOMAIN} cases"
                );
            }
            FixtureProfile::FinalHoldout => {
                ensure!(
                    (FINAL_MIN_DISTRACTORS_PER_DOMAIN..=FINAL_MAX_DISTRACTORS_PER_DOMAIN)
                        .contains(&domain.distractor_count),
                    "final holdout distractor count is outside its sealed range"
                );
                ensure!(
                    domain.documents.len() >= FINAL_DOCUMENTS_PER_DOMAIN,
                    "final holdout domain has too few curated documents"
                );
                ensure!(
                    domain.cases.len() == FINAL_CASES_PER_DOMAIN,
                    "final holdout domain must contain exactly {FINAL_CASES_PER_DOMAIN} cases"
                );
                ensure!(
                    matches!(domain.language.as_str(), "es" | "en"),
                    "final holdout language must be `es` or `en`"
                );
                *final_language_counts
                    .entry(domain.language.as_str())
                    .or_default() += 1;
            }
        }
        validate_section_shape(
            domain.distractor_heading.as_deref(),
            domain.distractor_text.as_deref(),
            &domain.distractor_sections,
            profile,
            "distractor",
        )?;
        total_documents = total_documents
            .saturating_add(domain.documents.len())
            .saturating_add(domain.distractor_count);
        let domain_documents = domain
            .documents
            .iter()
            .map(|document| document.id.as_str())
            .collect::<BTreeSet<_>>();
        ensure!(
            domain_documents.len() == domain.documents.len(),
            "duplicate document id within domain"
        );
        for document in &domain.documents {
            validate_identifier(&document.id)?;
            ensure!(
                all_document_ids.insert(document.id.as_str()),
                "duplicate document id across domains"
            );
            ensure!(!document.title.trim().is_empty(), "empty document title");
            validate_section_shape(
                document.heading.as_deref(),
                document.text.as_deref(),
                &document.sections,
                profile,
                "document",
            )?;
            let section_ids = resolved_sections(document)?
                .into_iter()
                .map(|section| section.id);
            ensure!(
                section_ids.clone().collect::<BTreeSet<_>>().len() == section_ids.count(),
                "duplicate section id within document"
            );
            for link in &document.links {
                ensure!(!link.label.trim().is_empty(), "empty link label");
                ensure!(
                    domain_documents.contains(link.target_document_id.as_str()),
                    "link target must exist in the same domain"
                );
                ensure!(
                    link.target_document_id != document.id,
                    "self links are not valid replay evidence"
                );
            }
        }
        let domain_evidence = domain
            .documents
            .iter()
            .flat_map(|document| {
                document.sections.iter().map(|section| EvidenceId {
                    document_id: document.id.clone(),
                    section_id: section.id.clone(),
                })
            })
            .collect::<BTreeSet<_>>();
        let mut answerable_count = 0_usize;
        let mut no_answer_count = 0_usize;
        let mut compound_count = 0_usize;
        for case in &domain.cases {
            validate_identifier(&case.id)?;
            ensure!(all_case_ids.insert(case.id.as_str()), "duplicate case id");
            ensure!(!case.question.trim().is_empty(), "empty replay question");
            match profile {
                FixtureProfile::Development => {
                    ensure!(
                        case.expected_answerable.is_none()
                            && case.required_evidence_groups.is_empty()
                            && case.forbidden_evidence.is_empty(),
                        "development replay cannot contain final-only case fields"
                    );
                    ensure!(
                        !case.required_document_groups.is_empty(),
                        "replay case has no required group"
                    );
                    for group in &case.required_document_groups {
                        ensure!(!group.is_empty(), "replay case has an empty group");
                        ensure!(
                            group
                                .iter()
                                .all(|id| domain_documents.contains(id.as_str())),
                            "replay gold must stay inside its domain"
                        );
                    }
                }
                FixtureProfile::FinalHoldout => {
                    ensure!(
                        case.required_document_groups.is_empty(),
                        "final holdout cannot contain development document groups"
                    );
                    let expected_answerable = case
                        .expected_answerable
                        .context("final holdout case is missing expected_answerable")?;
                    if expected_answerable {
                        answerable_count = answerable_count.saturating_add(1);
                        ensure!(
                            !case.required_evidence_groups.is_empty(),
                            "answerable final holdout case has no evidence group"
                        );
                        if case.required_evidence_groups.len() > 1 {
                            compound_count = compound_count.saturating_add(1);
                        }
                    } else {
                        no_answer_count = no_answer_count.saturating_add(1);
                        ensure!(
                            case.required_evidence_groups.is_empty(),
                            "no-answer final holdout case contains required evidence"
                        );
                    }
                    for group in &case.required_evidence_groups {
                        ensure!(
                            !group.is_empty(),
                            "final holdout has an empty evidence group"
                        );
                        ensure!(
                            group
                                .iter()
                                .all(|evidence| domain_evidence.contains(evidence)),
                            "final holdout gold must name an exact section in its domain"
                        );
                    }
                    ensure!(
                        case.forbidden_evidence
                            .iter()
                            .all(|evidence| domain_evidence.contains(evidence)),
                        "final holdout forbidden evidence must name an exact section in its domain"
                    );
                    let required = case
                        .required_evidence_groups
                        .iter()
                        .flatten()
                        .collect::<BTreeSet<_>>();
                    ensure!(
                        case.forbidden_evidence
                            .iter()
                            .all(|evidence| !required.contains(evidence)),
                        "final holdout evidence cannot be both required and forbidden"
                    );
                }
            }
        }
        if profile == FixtureProfile::FinalHoldout {
            ensure!(
                answerable_count == 4 && no_answer_count == 1,
                "final holdout domains require four answerable and one no-answer case"
            );
            ensure!(
                compound_count >= 1,
                "final holdout domain requires a compound evidence case"
            );
        }
    }
    ensure!(
        total_documents > CONTROL_LIMIT,
        "real-ranking replay must contain more than {CONTROL_LIMIT} candidates"
    );
    ensure!(
        total_documents <= MAX_DOCUMENTS,
        "real-ranking replay exceeds graph node budget"
    );
    if profile == FixtureProfile::FinalHoldout {
        ensure!(
            final_language_counts.get("es") == Some(&4)
                && final_language_counts.get("en") == Some(&4),
            "final holdout requires four Spanish and four English domains"
        );
    }
    Ok(())
}

fn validate_section_shape(
    legacy_heading: Option<&str>,
    legacy_text: Option<&str>,
    sections: &[SectionFixture],
    profile: FixtureProfile,
    label: &str,
) -> Result<()> {
    match profile {
        FixtureProfile::Development => ensure!(
            legacy_heading.is_some_and(|value| !value.trim().is_empty())
                && legacy_text.is_some_and(|value| !value.trim().is_empty())
                && sections.is_empty(),
            "development {label} must use exactly one legacy section"
        ),
        FixtureProfile::FinalHoldout => {
            ensure!(
                legacy_heading.is_none() && legacy_text.is_none(),
                "final holdout {label} cannot use legacy section fields"
            );
            ensure!(
                sections.len() >= FINAL_SECTIONS_PER_DOCUMENT,
                "final holdout {label} has too few sections"
            );
            for section in sections {
                validate_identifier(&section.id)?;
                ensure!(!section.heading.trim().is_empty(), "empty section heading");
                ensure!(!section.text.trim().is_empty(), "empty section text");
            }
        }
    }
    Ok(())
}

fn resolved_sections(document: &DocumentFixture) -> Result<Vec<SectionFixture>> {
    if !document.sections.is_empty() {
        return Ok(document.sections.clone());
    }
    let heading = document
        .heading
        .clone()
        .context("development document is missing its heading")?;
    let text = document
        .text
        .clone()
        .context("development document is missing its text")?;
    Ok(vec![SectionFixture {
        id: "section".to_owned(),
        heading,
        text,
    }])
}

fn validate_identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 96
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            }),
        "invalid real-ranking replay identifier"
    );
    Ok(())
}

async fn build_corpus(
    fixture: &ReplayFixture,
    embeddings: Arc<dyn EmbeddingProvider>,
    relevance: Arc<dyn EvidenceRelevanceProvider>,
    node_id: &str,
) -> Result<ReplayCorpus> {
    let workspace = EvaluationWorkspace::create()?;
    let database = Database::in_memory()?;
    let mut collection_ids = BTreeSet::new();
    let mut documents = Vec::new();

    for domain in &fixture.domains {
        let root = workspace.path().join(&domain.id);
        let source_folder = root.join("sources");
        let wiki_folder = root.join("wiki");
        std::fs::create_dir_all(&source_folder).context("creating replay source directory")?;
        std::fs::create_dir_all(&wiki_folder).context("creating replay wiki directory")?;
        let collection = database.create_collection(
            &domain.collection_name,
            &source_folder,
            &wiki_folder,
            CollectionPolicy::local_only(),
        )?;
        collection_ids.insert(collection.id);
        for document in &domain.documents {
            documents.push(seed_document_metadata(
                &database,
                collection.id,
                &source_folder,
                &domain.language,
                document.clone(),
                node_id,
            )?);
        }
        for index in 0..domain.distractor_count {
            let ordinal = index.saturating_add(1);
            let document = DocumentFixture {
                id: format!("{}_distractor_{ordinal:02}", domain.id),
                title: format!("{} {ordinal:02}", domain.distractor_title),
                description: domain.distractor_description.clone(),
                heading: domain.distractor_heading.clone(),
                text: domain.distractor_text.clone(),
                sections: domain.distractor_sections.clone(),
                links: Vec::new(),
            };
            documents.push(seed_document_metadata(
                &database,
                collection.id,
                &source_folder,
                &domain.language,
                document,
                node_id,
            )?);
        }
    }

    embed_and_store_chunks(&database, &documents, Arc::clone(&embeddings)).await?;
    let concepts = documents
        .iter()
        .map(|document| (document.logical_id.clone(), document.concept_id))
        .collect::<HashMap<_, _>>();
    let resources = documents
        .iter()
        .map(|document| {
            database
                .concept(document.concept_id)?
                .map(|concept| (document.logical_id.clone(), concept.logical_resource_uri))
                .context("seeded replay concept disappeared")
        })
        .collect::<Result<HashMap<_, _>>>()?;
    let materializer = OkfPublicationMaterializer::new(database.clone());
    for document in &mut documents {
        let mut draft = document.draft.clone();
        draft.links = document
            .links
            .iter()
            .map(|link| {
                Ok(SuggestedLink {
                    label: link.label.clone(),
                    target: resources
                        .get(&link.target_document_id)
                        .cloned()
                        .context("replay link target has no materialized resource")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let concept = database.save_enrichment(
            document.source_id,
            draft.clone(),
            node_id,
            "real-ranking-replay",
        )?;
        ensure!(
            concept.id == document.concept_id,
            "re-enrichment changed concept identity"
        );
        let evidence = database
            .review_evidence_page(concept.id, 1, None, None, 1)?
            .context("replay review evidence is missing")?;
        materializer.approve(concept.id, draft.clone(), &evidence.review_version)?;
        document.draft = draft;
    }
    let logical_by_concept = documents
        .iter()
        .map(|document| (document.concept_id, document.logical_id.clone()))
        .collect::<HashMap<_, _>>();

    let projection_started = Instant::now();
    let inspector = OkfBundleInspector::new(database.clone());
    let mut bundle_fingerprints = BTreeMap::new();
    let mut node_inputs = Vec::new();
    let mut link_inputs = Vec::new();
    for collection_id in &collection_ids {
        let bundle = inspector.inspect_bundle(*collection_id)?;
        ensure!(
            bundle.state == KnowledgeBundleState::Ready && bundle.health.is_healthy(),
            "real-ranking replay requires a healthy Ready bundle"
        );
        bundle_fingerprints.insert(*collection_id, bundle.fingerprint.clone());
        node_inputs.extend(bundle.concepts.iter().map(|concept| GraphNodeInput {
            node: GraphNode {
                concept_id: concept.id,
                collection_id: *collection_id,
            },
            state: NodeState::Current,
        }));
        for link in &bundle.links {
            if let (
                KnowledgePageId::Concept(source),
                KnowledgeLinkDisposition::Internal(KnowledgePageId::Concept(target)),
            ) = (link.source, &link.disposition)
            {
                link_inputs.push(GraphLinkInput {
                    source,
                    target: *target,
                    disposition: LinkDisposition::ReviewedInternal,
                });
            }
        }
    }
    let graph = MiniGraph::build(&node_inputs, &link_inputs)?;
    let structural_sham = build_structural_sham(&node_inputs, &link_inputs, &logical_by_concept)?;
    let sham_graph = MiniGraph::build(&node_inputs, &structural_sham.links)?;
    ensure!(
        graph.node_count() == sham_graph.node_count()
            && graph.edge_count() == sham_graph.edge_count()
            && directed_degrees(&link_inputs) == directed_degrees(&structural_sham.links),
        "sham graph must preserve graph size and per-node directed degrees"
    );
    let projection_micros = projection_started.elapsed().as_micros();
    let concept_by_logical = concepts.into_iter().collect::<HashMap<_, _>>();
    let revision_by_concept = documents
        .iter()
        .map(|document| (document.concept_id, 1))
        .collect::<HashMap<_, _>>();
    let expected_by_chunk = expected_citations(&database, &documents)?;
    let engine = HybridSearchEngine::new(database.clone(), embeddings, relevance, node_id);
    Ok(ReplayCorpus {
        database,
        engine,
        graph,
        sham_graph,
        sham_stats: structural_sham.stats,
        logical_by_concept,
        concept_by_logical,
        expected_by_chunk,
        revision_by_concept,
        collection_ids,
        bundle_fingerprints,
        projection_micros,
        _workspace: workspace,
    })
}

fn seed_document_metadata(
    database: &Database,
    collection_id: Uuid,
    source_folder: &Path,
    language: &str,
    document: DocumentFixture,
    node_id: &str,
) -> Result<SeedDocument> {
    let sections = resolved_sections(&document)?
        .into_iter()
        .map(|section| SeedSection {
            logical_id: section.id,
            heading: section.heading,
            text: section.text,
        })
        .collect::<Vec<_>>();
    let mut source_contents = format!("# {}\n", document.title);
    for section in &sections {
        source_contents.push_str(&format!("\n## {}\n\n{}\n", section.heading, section.text));
    }
    let source_sha256 = hex::encode(Sha256::digest(source_contents.as_bytes()));
    let source_path = source_folder.join(format!("{}.md", document.id));
    std::fs::write(&source_path, source_contents.as_bytes())
        .context("writing real-ranking replay source")?;
    let source = database.register_source(
        collection_id,
        &source_path,
        &source_sha256,
        "markdown",
        u64::try_from(source_contents.len()).context("replay source is too large")?,
    )?;
    database.mark_extracted(
        source.id(),
        0,
        u64::try_from(source_contents.chars().count())
            .context("replay source character count overflow")?,
    )?;
    let summary = sections
        .iter()
        .map(|section| section.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let draft = EnrichmentDraft {
        concept_type: ConceptType::Document,
        title: document.title,
        description: document.description,
        language: language.to_owned(),
        tags: vec!["synthetic-evaluation".to_owned()],
        entities: Vec::new(),
        links: Vec::new(),
        summary,
        classification_confidence: 1.0,
        classification_explanation: "synthetic real-ranking graph replay".to_owned(),
    };
    let concept =
        database.save_enrichment(source.id(), draft.clone(), node_id, "real-ranking-replay")?;
    Ok(SeedDocument {
        logical_id: document.id,
        collection_id,
        source_id: source.id(),
        concept_id: concept.id,
        draft,
        source_sha256,
        sections,
        links: document.links,
    })
}

async fn embed_and_store_chunks(
    database: &Database,
    documents: &[SeedDocument],
    embeddings: Arc<dyn EmbeddingProvider>,
) -> Result<()> {
    let pending = documents
        .iter()
        .flat_map(|document| {
            document
                .sections
                .iter()
                .enumerate()
                .map(move |(index, section)| (document, index, section))
        })
        .collect::<Vec<_>>();
    let mut chunks_by_concept = HashMap::<Uuid, Vec<StoredChunk>>::new();
    for batch in pending.chunks(32) {
        let inputs = batch
            .iter()
            .map(|(_, _, section)| format!("passage: {}", section.text))
            .collect::<Vec<_>>();
        let vectors = embeddings.embed(&inputs).await?;
        ensure!(
            vectors.len() == batch.len(),
            "embedding batch size mismatch"
        );
        for ((document, section_index, section), embedding) in batch.iter().zip(vectors) {
            ensure!(
                embedding.len() == EMBEDDING_DIMENSIONS,
                "embedding dimension mismatch"
            );
            let text_sha256 = hex::encode(Sha256::digest(section.text.as_bytes()));
            let ordinal = u32::try_from(*section_index).context("too many replay sections")?;
            let chunk = StoredChunk {
                id: Uuid::new_v5(
                    &Uuid::NAMESPACE_URL,
                    format!(
                        "airwiki-real-graph:{}:{}",
                        document.logical_id, section.logical_id
                    )
                    .as_bytes(),
                ),
                concept_id: document.concept_id,
                source_document_id: document.source_id,
                collection_id: document.collection_id,
                ordinal,
                heading_or_page: section.heading.clone(),
                text: section.text.clone(),
                text_sha256,
                embedding,
                source_revision: 1,
            };
            chunks_by_concept
                .entry(document.concept_id)
                .or_default()
                .push(chunk);
        }
    }
    for document in documents {
        let chunks = chunks_by_concept
            .remove(&document.concept_id)
            .context("embedded replay concept has no chunks")?;
        database.replace_chunks(document.concept_id, &chunks)?;
    }
    Ok(())
}

fn expected_citations(
    database: &Database,
    documents: &[SeedDocument],
) -> Result<HashMap<Uuid, ExpectedCitation>> {
    let mut expected = HashMap::new();
    for document in documents {
        let concept = database
            .concept(document.concept_id)?
            .context("seeded replay concept disappeared")?;
        let chunks = database.chunks_for_concept(document.concept_id)?;
        ensure!(
            chunks.len() == document.sections.len(),
            "seeded replay chunk count changed"
        );
        for (chunk, section) in chunks.iter().zip(&document.sections) {
            let public_id =
                replay_public_chunk_id(&document.source_sha256, chunk.ordinal, &chunk.text_sha256);
            ensure!(
                expected
                    .insert(
                        public_id,
                        ExpectedCitation {
                            evidence: EvidenceId {
                                document_id: document.logical_id.clone(),
                                section_id: section.logical_id.clone(),
                            },
                            collection_id: document.collection_id,
                            concept_id: document.concept_id,
                            source_revision: 1,
                            source_sha256: document.source_sha256.clone(),
                            logical_resource_uri: concept.logical_resource_uri.clone(),
                            title: document.draft.title.clone(),
                            heading_or_page: section.heading.clone(),
                        },
                    )
                    .is_none(),
                "duplicate public replay chunk identity"
            );
        }
    }
    Ok(expected)
}

fn replay_public_chunk_id(source_sha256: &str, ordinal: u32, text_sha256: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("urn:airwiki:chunk:{source_sha256}:{ordinal}:{text_sha256}").as_bytes(),
    )
}

fn directed_degrees(links: &[GraphLinkInput]) -> (BTreeMap<Uuid, usize>, BTreeMap<Uuid, usize>) {
    let mut outgoing = BTreeMap::new();
    let mut incoming = BTreeMap::new();
    for link in links {
        *outgoing.entry(link.source).or_insert(0) += 1;
        *incoming.entry(link.target).or_insert(0) += 1;
    }
    (outgoing, incoming)
}

async fn run_replay(
    loaded: &LoadedFixture,
    corpus: &ReplayCorpus,
    embedding_profile: String,
) -> Result<ReplayReport> {
    let started = Instant::now();
    let scope = QueryScope {
        purpose: GraphPurpose::LocalAssistant,
        authorized_collections: corpus.collection_ids.clone(),
        external_ai_collections: BTreeSet::new(),
    };
    let mut reports = Vec::new();
    for domain in &loaded.fixture.domains {
        for case in &domain.cases {
            let ranked = corpus
                .engine
                .rank_local_for_evaluation(
                    &case.question,
                    SearchPurpose::LocalAssistant,
                    CONTROL_LIMIT,
                )
                .await?;
            ensure!(
                ranked
                    .iter()
                    .all(|candidate| candidate.source_revision == 1),
                "replay ranking returned a stale revision"
            );
            let ranked_concepts = ranked
                .iter()
                .map(|candidate| candidate.concept_id)
                .collect::<Vec<_>>();
            let c10_nodes =
                corpus
                    .graph
                    .visible_candidates(&ranked_concepts, &scope, BASELINE_LIMIT);
            let c32_nodes =
                corpus
                    .graph
                    .visible_candidates(&ranked_concepts, &scope, CONTROL_LIMIT);
            let g1_out_nodes = corpus
                .graph
                .expand_one_hop_with_backfill(
                    &c10_nodes,
                    &ranked_concepts,
                    &scope,
                    ExpansionDirection::Outgoing,
                )
                .candidates;
            let g1_bidir_nodes = corpus
                .graph
                .expand_one_hop_with_backfill(
                    &c10_nodes,
                    &ranked_concepts,
                    &scope,
                    ExpansionDirection::Bidirectional,
                )
                .candidates;
            let sham_seeds =
                corpus
                    .sham_graph
                    .visible_candidates(&ranked_concepts, &scope, BASELINE_LIMIT);
            let g1_sham_nodes = corpus
                .sham_graph
                .expand_one_hop_with_backfill(
                    &sham_seeds,
                    &ranked_concepts,
                    &scope,
                    ExpansionDirection::Bidirectional,
                )
                .candidates;
            let c10 = score_arm(
                node_documents(&corpus.graph, &c10_nodes, &corpus.logical_by_concept)?,
                &case.required_document_groups,
            );
            let c32 = score_arm(
                node_documents(&corpus.graph, &c32_nodes, &corpus.logical_by_concept)?,
                &case.required_document_groups,
            );
            let g1_out = score_arm(
                node_documents(&corpus.graph, &g1_out_nodes, &corpus.logical_by_concept)?,
                &case.required_document_groups,
            );
            let g1_bidir = score_arm(
                node_documents(&corpus.graph, &g1_bidir_nodes, &corpus.logical_by_concept)?,
                &case.required_document_groups,
            );
            let g1_sham = score_arm(
                node_documents(
                    &corpus.sham_graph,
                    &g1_sham_nodes,
                    &corpus.logical_by_concept,
                )?,
                &case.required_document_groups,
            );
            reports.push(CaseReport {
                id: case.id.clone(),
                domain: domain.id.clone(),
                outgoing_unique_group_count_over_c32: unique_groups(
                    &g1_out,
                    &c32,
                    &case.required_document_groups,
                ),
                bidir_unique_group_count_over_outgoing: unique_groups(
                    &g1_bidir,
                    &g1_out,
                    &case.required_document_groups,
                ),
                sham_unique_group_count_over_c32: unique_groups(
                    &g1_sham,
                    &c32,
                    &case.required_document_groups,
                ),
                lost_group_count_against_c32: lost_groups(
                    &g1_bidir,
                    &c32,
                    &case.required_document_groups,
                ),
                c10,
                c32,
                g1_out,
                g1_bidir,
                g1_sham,
            });
        }
    }
    let ranking_and_expansion_micros = started.elapsed().as_micros();
    let inspector = OkfBundleInspector::new(corpus.database.clone());
    let healthy_fingerprint_gate_passed =
        corpus
            .bundle_fingerprints
            .iter()
            .all(|(collection_id, expected)| {
                inspector
                    .inspect_bundle(*collection_id)
                    .is_ok_and(|bundle| {
                        bundle.state == KnowledgeBundleState::Ready
                            && bundle.health.is_healthy()
                            && bundle.fingerprint == *expected
                    })
            });
    ensure!(
        corpus.concept_by_logical.values().all(|concept_id| corpus
            .database
            .chunks_for_concept(*concept_id)
            .is_ok_and(|chunks| { chunks.len() == 1 && chunks[0].source_revision == 1 })),
        "real-ranking replay requires one current chunk per concept"
    );

    let c10 = aggregate(reports.iter().map(|report| &report.c10));
    let c32 = aggregate(reports.iter().map(|report| &report.c32));
    let g1_out = aggregate(reports.iter().map(|report| &report.g1_out));
    let g1_bidir = aggregate(reports.iter().map(|report| &report.g1_bidir));
    let g1_sham = aggregate(reports.iter().map(|report| &report.g1_sham));
    let outgoing_unique_group_count_over_c32 = reports
        .iter()
        .map(|report| report.outgoing_unique_group_count_over_c32)
        .sum();
    let bidir_unique_group_count_over_outgoing = reports
        .iter()
        .map(|report| report.bidir_unique_group_count_over_outgoing)
        .sum();
    let sham_unique_group_count_over_c32 = reports
        .iter()
        .map(|report| report.sham_unique_group_count_over_c32)
        .sum();
    let improved_domain_count_over_controls = loaded
        .fixture
        .domains
        .iter()
        .filter(|domain| {
            let domain_reports = reports.iter().filter(|report| report.domain == domain.id);
            let (c32_found, sham_found, bidir_found) =
                domain_reports.fold((0_u32, 0_u32, 0_u32), |counts, report| {
                    (
                        counts.0.saturating_add(report.c32.found_group_count),
                        counts.1.saturating_add(report.g1_sham.found_group_count),
                        counts.2.saturating_add(report.g1_bidir.found_group_count),
                    )
                });
            bidir_found > c32_found && bidir_found > sham_found
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let outgoing_rescue_domain_count = loaded
        .fixture
        .domains
        .iter()
        .filter(|domain| {
            reports.iter().any(|report| {
                report.domain == domain.id && report.outgoing_unique_group_count_over_c32 > 0
            })
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let backlink_rescue_domain_count = loaded
        .fixture
        .domains
        .iter()
        .filter(|domain| {
            reports.iter().any(|report| {
                report.domain == domain.id && report.bidir_unique_group_count_over_outgoing > 0
            })
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let regressed_case_count_against_c32 = reports
        .iter()
        .filter(|report| report.lost_group_count_against_c32 > 0)
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let retained_payload_bytes = corpus
        .graph
        .retained_payload_bytes()
        .saturating_add(corpus.sham_graph.retained_payload_bytes());
    let rejection_reasons = development_rejection_reasons(DevelopmentGateMetrics {
        healthy_fingerprint: healthy_fingerprint_gate_passed,
        c32_found_groups: c32.found_group_count,
        c32_support_density: c32.support_density,
        bidir_found_groups: g1_bidir.found_group_count,
        bidir_support_density: g1_bidir.support_density,
        sham_found_groups: g1_sham.found_group_count,
        outgoing_unique_groups: outgoing_unique_group_count_over_c32,
        backlink_unique_groups: bidir_unique_group_count_over_outgoing,
        sham_unique_groups: sham_unique_group_count_over_c32,
        improved_domains: improved_domain_count_over_controls,
        outgoing_rescue_domains: outgoing_rescue_domain_count,
        backlink_rescue_domains: backlink_rescue_domain_count,
        regressed_cases: regressed_case_count_against_c32,
        retained_payload_bytes,
    });
    let development_gate_passed = rejection_reasons.is_empty();
    Ok(ReplayReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment_id: loaded.fixture.experiment_id.clone(),
        fixture_sha256: loaded.sha256.clone(),
        embedding_profile,
        target_os: std::env::consts::OS.to_owned(),
        target_arch: std::env::consts::ARCH.to_owned(),
        domain_count: u32::try_from(loaded.fixture.domains.len()).unwrap_or(u32::MAX),
        case_count: u32::try_from(reports.len()).unwrap_or(u32::MAX),
        concept_count: corpus.graph.node_count(),
        edge_count: corpus.graph.edge_count(),
        graph_fingerprint: corpus.graph.fingerprint().to_owned(),
        sham_graph_fingerprint: corpus.sham_graph.fingerprint().to_owned(),
        sham_contract_version: SHAM_CONTRACT_VERSION.to_owned(),
        sham_linked_collection_count: corpus.sham_stats.collection_count,
        sham_retained_original_edge_count: corpus.sham_stats.retained_original_edge_count,
        sham_rewired_edge_count: corpus.sham_stats.rewired_edge_count,
        sham_unchanged_collection_count: corpus.sham_stats.unchanged_collection_count,
        retained_payload_bytes,
        projection_micros: corpus.projection_micros,
        ranking_and_expansion_micros,
        c10,
        c32,
        g1_out,
        g1_bidir,
        g1_sham,
        outgoing_unique_group_count_over_c32,
        bidir_unique_group_count_over_outgoing,
        sham_unique_group_count_over_c32,
        improved_domain_count_over_controls,
        outgoing_rescue_domain_count,
        backlink_rescue_domain_count,
        regressed_case_count_against_c32,
        healthy_fingerprint_gate_passed,
        development_gate_passed,
        production_promotion_ready: false,
        rejection_reasons,
        cases: reports,
    })
}

async fn warm_final_evaluator(corpus: &ReplayCorpus) -> Result<()> {
    let query = "synthetic evaluator warmup probe";
    let ranked = corpus
        .engine
        .rank_local_for_evaluation(query, SearchPurpose::LocalAssistant, CONTROL_LIMIT)
        .await?;
    ensure!(
        !ranked.is_empty(),
        "final evaluator warmup found no candidates"
    );
    let nominees = ranked
        .into_iter()
        .map(RetrievalEvaluationNominee::Exact)
        .collect::<Vec<_>>();
    let selection = corpus
        .engine
        .search_local_arm_for_evaluation(
            SearchRequest::new(query, SearchPurpose::LocalAssistant, FINAL_TOP_K),
            &nominees,
        )
        .await?;
    ensure!(
        !selection.partial,
        "final evaluator warmup observed a publication change"
    );
    Ok(())
}

async fn run_final_replay(
    loaded: &LoadedFixture,
    corpus: &ReplayCorpus,
    embedding_profile: String,
    relevance_profile: String,
) -> Result<FinalReplayReport> {
    let scope = QueryScope {
        purpose: GraphPurpose::LocalAssistant,
        authorized_collections: corpus.collection_ids.clone(),
        external_ai_collections: BTreeSet::new(),
    };
    let mut reports = Vec::with_capacity(FINAL_DOMAIN_COUNT * FINAL_CASES_PER_DOMAIN);
    let mut case_index = 0_usize;
    for domain in &loaded.fixture.domains {
        for case in &domain.cases {
            let ranking_started = Instant::now();
            let ranked = corpus
                .engine
                .rank_local_for_evaluation(
                    &case.question,
                    SearchPurpose::LocalAssistant,
                    CONTROL_LIMIT,
                )
                .await?;
            let ranking_micros = ranking_started.elapsed().as_micros();
            ensure!(
                ranked.len() == CONTROL_LIMIT,
                "final replay ranking did not fill its frozen candidate budget"
            );
            ensure!(
                ranked.iter().all(|candidate| {
                    candidate.source_revision == 1
                        && corpus.expected_by_chunk.contains_key(&candidate.chunk_id)
                }),
                "final replay ranking returned unknown or stale evidence"
            );

            let b32_nominees = ranked
                .iter()
                .copied()
                .map(RetrievalEvaluationNominee::Exact)
                .collect::<Vec<_>>();
            let graph_assembly_started = Instant::now();
            let g1_nominees = assemble_graph_nominees(
                &corpus.graph,
                &ranked,
                &scope,
                &corpus.revision_by_concept,
                &corpus.logical_by_concept,
            )?;
            let graph_assembly_micros = graph_assembly_started.elapsed().as_micros();
            let sham_assembly_started = Instant::now();
            let sham_nominees = assemble_graph_nominees(
                &corpus.sham_graph,
                &ranked,
                &scope,
                &corpus.revision_by_concept,
                &corpus.logical_by_concept,
            )?;
            let sham_assembly_micros = sham_assembly_started.elapsed().as_micros();

            let order = final_arm_order(case_index);
            let mut b32_execution = None;
            let mut g1_execution = None;
            let mut sham_execution = None;
            for arm in order {
                let (nominees, assembly_micros, destination) = match arm {
                    FinalArmKind::B32 => (&b32_nominees, 0, &mut b32_execution),
                    FinalArmKind::Graph => (&g1_nominees, graph_assembly_micros, &mut g1_execution),
                    FinalArmKind::Sham => {
                        (&sham_nominees, sham_assembly_micros, &mut sham_execution)
                    }
                };
                *destination = Some(
                    execute_final_arm(
                        corpus,
                        &case.question,
                        nominees,
                        ranking_micros.saturating_add(assembly_micros),
                    )
                    .await,
                );
            }
            let b32_execution = b32_execution.context("B32 arm was not executed")?;
            let g1_execution = g1_execution.context("G1 arm was not executed")?;
            let sham_execution = sham_execution.context("sham arm was not executed")?;
            let g1_candidate_assembly_micros =
                graph_assembly_micros.saturating_add(g1_execution.candidate_preparation_micros);
            let b32 = score_final_arm(corpus, b32_execution, case)?;
            let g1 = score_final_arm(corpus, g1_execution, case)?;
            let g1_sham = score_final_arm(corpus, sham_execution, case)?;
            reports.push(FinalCaseReport {
                id: case.id.clone(),
                domain: domain.id.clone(),
                expected_answerable: case
                    .expected_answerable
                    .context("validated final case lost answerability")?,
                arm_order: order.map(|arm| final_arm_label(arm).to_owned()),
                g1_candidate_assembly_micros,
                g1_lost_b32_group_count: lost_exact_groups(
                    &g1.evidence_ids,
                    &b32.evidence_ids,
                    &case.required_evidence_groups,
                ),
                b32,
                g1,
                g1_sham,
            });
            case_index = case_index.saturating_add(1);
        }
    }

    let healthy_fingerprint_gate_passed = bundle_fingerprints_unchanged(corpus);
    let b32 = aggregate_final(reports.iter().map(|report| &report.b32));
    let g1 = aggregate_final(reports.iter().map(|report| &report.g1));
    let g1_sham = aggregate_final(reports.iter().map(|report| &report.g1_sham));
    let domains = final_domain_reports(&loaded.fixture, &reports);
    let b32_macro_domain_recall =
        mean(domains.iter().map(|domain| domain.b32_group_recall_at_five));
    let g1_macro_domain_recall = mean(domains.iter().map(|domain| domain.g1_group_recall_at_five));
    let sham_macro_domain_recall = mean(
        domains
            .iter()
            .map(|domain| domain.sham_group_recall_at_five),
    );
    let g1_macro_gain_over_b32 = g1_macro_domain_recall - b32_macro_domain_recall;
    let g1_macro_gain_over_sham = g1_macro_domain_recall - sham_macro_domain_recall;
    let improved_domain_count_over_both = domains
        .iter()
        .filter(|domain| domain.g1_improved_over_both)
        .count();
    let bootstrap_over_b32 = paired_bootstrap_lower_bound(
        &domains
            .iter()
            .map(|domain| domain.g1_group_recall_at_five - domain.b32_group_recall_at_five)
            .collect::<Vec<_>>(),
    );
    let bootstrap_over_sham = paired_bootstrap_lower_bound(
        &domains
            .iter()
            .map(|domain| domain.g1_group_recall_at_five - domain.sham_group_recall_at_five)
            .collect::<Vec<_>>(),
    );
    let g1_candidate_assembly_p95_micros = percentile_micros(
        reports
            .iter()
            .map(|report| report.g1_candidate_assembly_micros),
        95,
    );
    let g1_lost_b32_group_count = reports
        .iter()
        .map(|report| report.g1_lost_b32_group_count)
        .sum();
    let retained_payload_bytes = corpus
        .graph
        .retained_payload_bytes()
        .saturating_add(corpus.sham_graph.retained_payload_bytes());
    let rejection_reasons = final_rejection_reasons(FinalGateInput {
        b32: &b32,
        g1: &g1,
        sham: &g1_sham,
        macro_gain_over_b32: g1_macro_gain_over_b32,
        macro_gain_over_sham: g1_macro_gain_over_sham,
        improved_domain_count: improved_domain_count_over_both,
        bootstrap_over_b32,
        bootstrap_over_sham,
        lost_b32_groups: g1_lost_b32_group_count,
        retained_payload_bytes,
        projection_micros: corpus.projection_micros,
        g1_candidate_assembly_p95_micros,
        healthy_fingerprint: healthy_fingerprint_gate_passed,
    });
    Ok(FinalReplayReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment_id: loaded.fixture.experiment_id.clone(),
        fixture_sha256: loaded.sha256.clone(),
        evaluation_policy_fingerprint: hex::encode(Sha256::digest(FINAL_POLICY.as_bytes())),
        embedding_profile,
        relevance_profile,
        target_os: std::env::consts::OS.to_owned(),
        target_arch: std::env::consts::ARCH.to_owned(),
        domain_count: u32::try_from(loaded.fixture.domains.len()).unwrap_or(u32::MAX),
        case_count: u32::try_from(reports.len()).unwrap_or(u32::MAX),
        concept_count: corpus.graph.node_count(),
        edge_count: corpus.graph.edge_count(),
        graph_fingerprint: corpus.graph.fingerprint().to_owned(),
        sham_graph_fingerprint: corpus.sham_graph.fingerprint().to_owned(),
        sham_contract_version: SHAM_CONTRACT_VERSION.to_owned(),
        sham_linked_collection_count: corpus.sham_stats.collection_count,
        sham_retained_original_edge_count: corpus.sham_stats.retained_original_edge_count,
        sham_rewired_edge_count: corpus.sham_stats.rewired_edge_count,
        sham_unchanged_collection_count: corpus.sham_stats.unchanged_collection_count,
        bundle_set_fingerprint: bundle_set_fingerprint(&corpus.bundle_fingerprints),
        retained_payload_bytes,
        projection_micros: corpus.projection_micros,
        g1_candidate_assembly_p95_micros,
        b32,
        g1,
        g1_sham,
        b32_macro_domain_recall,
        g1_macro_domain_recall,
        sham_macro_domain_recall,
        g1_macro_gain_over_b32,
        g1_macro_gain_over_sham,
        improved_domain_count_over_both: u32::try_from(improved_domain_count_over_both)
            .unwrap_or(u32::MAX),
        paired_bootstrap_lower_bound_over_b32: bootstrap_over_b32,
        paired_bootstrap_lower_bound_over_sham: bootstrap_over_sham,
        g1_lost_b32_group_count,
        healthy_fingerprint_gate_passed,
        shadow_eligible: rejection_reasons.is_empty(),
        production_promotion_ready: false,
        rejection_reasons,
        domains,
        cases: reports,
    })
}

fn assemble_graph_nominees(
    graph: &MiniGraph,
    ranked: &[RetrievalEvaluationCandidate],
    scope: &QueryScope,
    revision_by_concept: &HashMap<Uuid, u32>,
    stable_label_by_concept: &HashMap<Uuid, String>,
) -> Result<Vec<RetrievalEvaluationNominee>> {
    ensure!(
        ranked.len() >= BASELINE_LIMIT,
        "graph arm has fewer than ten exact seeds"
    );
    let mut nominees = ranked
        .iter()
        .take(BASELINE_LIMIT)
        .copied()
        .map(RetrievalEvaluationNominee::Exact)
        .collect::<Vec<_>>();
    let seed_concepts = ranked
        .iter()
        .take(BASELINE_LIMIT)
        .map(|candidate| candidate.concept_id)
        .collect::<Vec<_>>();
    let seeds = graph.visible_candidates(&seed_concepts, scope, BASELINE_LIMIT);
    let seed_set = seeds.iter().copied().collect::<BTreeSet<_>>();
    let expansion = graph.expand_one_hop(&seeds, scope, ExpansionDirection::Bidirectional);
    ensure!(
        !expansion.edge_budget_exhausted && !expansion.candidate_budget_exhausted,
        "final graph expansion exhausted its deterministic budget"
    );
    let mut expanded = expansion
        .candidates
        .iter()
        .copied()
        .filter(|node_id| !seed_set.contains(node_id))
        .map(|node_id| {
            let node = graph
                .node(node_id)
                .context("expanded graph node disappeared")?;
            let stable_label = stable_label_by_concept
                .get(&node.concept_id)
                .cloned()
                .context("expanded concept has no stable evaluation label")?;
            Ok((stable_label, node_id))
        })
        .collect::<Result<Vec<_>>>()?;
    expanded.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    for (_, node_id) in expanded {
        let node = graph
            .node(node_id)
            .context("expanded graph node disappeared")?;
        let expected_revision = revision_by_concept
            .get(&node.concept_id)
            .copied()
            .context("expanded concept has no current bundle revision")?;
        nominees.push(RetrievalEvaluationNominee::Concept {
            collection_id: node.collection_id,
            concept_id: node.concept_id,
            expected_revision,
        });
        if nominees.len() == CONTROL_LIMIT {
            return Ok(nominees);
        }
    }
    nominees.extend(
        ranked
            .iter()
            .skip(BASELINE_LIMIT)
            .copied()
            .map(RetrievalEvaluationNominee::Exact)
            .take(CONTROL_LIMIT.saturating_sub(nominees.len())),
    );
    ensure!(
        nominees.len() == CONTROL_LIMIT,
        "graph arm did not fill its frozen nominee budget"
    );
    Ok(nominees)
}

async fn execute_final_arm(
    corpus: &ReplayCorpus,
    question: &str,
    nominees: &[RetrievalEvaluationNominee],
    prior_micros: u128,
) -> FinalArmExecution {
    let started = Instant::now();
    let selection = corpus
        .engine
        .search_local_arm_for_evaluation(
            SearchRequest::new(question, SearchPurpose::LocalAssistant, FINAL_TOP_K),
            nominees,
        )
        .await;
    let full_query_micros = prior_micros.saturating_add(started.elapsed().as_micros());
    match selection {
        Ok(selection) if !selection.partial => FinalArmExecution {
            candidate_preparation_micros: selection.candidate_preparation_micros,
            candidates: selection.candidates,
            provider_failure: false,
            full_query_micros,
        },
        Ok(_) | Err(_) => FinalArmExecution {
            candidates: Vec::new(),
            provider_failure: true,
            candidate_preparation_micros: 0,
            full_query_micros,
        },
    }
}

fn final_arm_order(case_index: usize) -> [FinalArmKind; 3] {
    match case_index % 3 {
        0 => [FinalArmKind::B32, FinalArmKind::Graph, FinalArmKind::Sham],
        1 => [FinalArmKind::Graph, FinalArmKind::Sham, FinalArmKind::B32],
        _ => [FinalArmKind::Sham, FinalArmKind::B32, FinalArmKind::Graph],
    }
}

const fn final_arm_label(arm: FinalArmKind) -> &'static str {
    match arm {
        FinalArmKind::B32 => "b32",
        FinalArmKind::Graph => "g1",
        FinalArmKind::Sham => "g1_sham",
    }
}

fn score_final_arm(
    corpus: &ReplayCorpus,
    execution: FinalArmExecution,
    case: &CaseFixture,
) -> Result<FinalArmReport> {
    let expected_answerable = case
        .expected_answerable
        .context("validated final case lost answerability")?;
    let required = case
        .required_evidence_groups
        .iter()
        .flatten()
        .collect::<BTreeSet<_>>();
    let forbidden = case.forbidden_evidence.iter().collect::<BTreeSet<_>>();
    let mut evidence = Vec::new();
    let mut unknown_count = 0_usize;
    let mut provenance_exact_and_current = true;
    for candidate in &execution.candidates {
        let Some(expected) = corpus.expected_by_chunk.get(&candidate.chunk_id) else {
            unknown_count = unknown_count.saturating_add(1);
            provenance_exact_and_current = false;
            continue;
        };
        if !candidate_matches_current(corpus, candidate, expected)? {
            provenance_exact_and_current = false;
        }
        evidence.push(expected.evidence.clone());
    }
    let evidence_set = evidence.iter().collect::<BTreeSet<_>>();
    let found_group_count = case
        .required_evidence_groups
        .iter()
        .filter(|group| group.iter().any(|item| evidence_set.contains(item)))
        .count();
    let first_relevant_rank = evidence
        .iter()
        .position(|item| required.contains(item))
        .map(|index| index.saturating_add(1));
    let relevant_citation_count = evidence
        .iter()
        .filter(|item| required.contains(item))
        .count();
    let forbidden_citation_count = evidence
        .iter()
        .filter(|item| forbidden.contains(item))
        .count();
    let returned_citation_count = execution.candidates.len();
    Ok(FinalArmReport {
        returned_citation_count: u32::try_from(returned_citation_count).unwrap_or(u32::MAX),
        unknown_citation_count: u32::try_from(unknown_count).unwrap_or(u32::MAX),
        found_group_count: u32::try_from(found_group_count).unwrap_or(u32::MAX),
        required_group_count: u32::try_from(case.required_evidence_groups.len())
            .unwrap_or(u32::MAX),
        group_recall_at_five: ratio(found_group_count, case.required_evidence_groups.len()),
        reciprocal_rank_at_five: if expected_answerable {
            Some(
                first_relevant_rank
                    .map(|rank| 1.0 / rank as f64)
                    .unwrap_or(0.0),
            )
        } else {
            None
        },
        relevant_citation_count: u32::try_from(relevant_citation_count).unwrap_or(u32::MAX),
        citation_precision: ratio(relevant_citation_count, returned_citation_count),
        forbidden_citation_count: u32::try_from(forbidden_citation_count).unwrap_or(u32::MAX),
        no_answer_correct: (!expected_answerable).then_some(returned_citation_count == 0),
        provenance_exact_and_current,
        provider_failure: execution.provider_failure,
        full_query_micros: execution.full_query_micros,
        evidence_ids: evidence.iter().map(evidence_label).collect(),
    })
}

fn candidate_matches_current(
    corpus: &ReplayCorpus,
    candidate: &RetrievalEvaluationCandidate,
    expected: &ExpectedCitation,
) -> Result<bool> {
    if candidate.collection_id != expected.collection_id
        || candidate.concept_id != expected.concept_id
        || candidate.source_revision != expected.source_revision
    {
        return Ok(false);
    }
    let Some(concept) = corpus.database.concept(candidate.concept_id)? else {
        return Ok(false);
    };
    let Some(source) = corpus
        .database
        .source_document(concept.source_document_id)?
    else {
        return Ok(false);
    };
    let chunks = corpus.database.chunks_for_concept(candidate.concept_id)?;
    let chunk_matches = chunks.iter().any(|chunk| {
        replay_public_chunk_id(&source.source_sha256, chunk.ordinal, &chunk.text_sha256)
            == candidate.chunk_id
            && chunk.heading_or_page == expected.heading_or_page
            && chunk.source_revision == expected.source_revision
    });
    Ok(concept.status == DocumentStatus::Published
        && source.status == DocumentStatus::Published
        && source.revision == expected.source_revision
        && source.source_sha256 == expected.source_sha256
        && concept.logical_resource_uri == expected.logical_resource_uri
        && concept.draft.title == expected.title
        && chunk_matches)
}

fn evidence_label(evidence: &EvidenceId) -> String {
    format!("{}#{}", evidence.document_id, evidence.section_id)
}

fn lost_exact_groups(
    candidate_labels: &[String],
    control_labels: &[String],
    groups: &[Vec<EvidenceId>],
) -> u32 {
    let candidate = candidate_labels
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let control = control_labels
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let count = groups
        .iter()
        .filter(|group| {
            let candidate_found = group
                .iter()
                .map(evidence_label)
                .any(|label| candidate.contains(label.as_str()));
            let control_found = group
                .iter()
                .map(evidence_label)
                .any(|label| control.contains(label.as_str()));
            control_found && !candidate_found
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn aggregate_final<'a>(arms: impl Iterator<Item = &'a FinalArmReport>) -> FinalAggregateArm {
    let arms = arms.collect::<Vec<_>>();
    let returned_citation_count = arms.iter().map(|arm| arm.returned_citation_count).sum();
    let found_group_count = arms.iter().map(|arm| arm.found_group_count).sum();
    let required_group_count = arms.iter().map(|arm| arm.required_group_count).sum();
    let relevant_citation_count = arms.iter().map(|arm| arm.relevant_citation_count).sum();
    let forbidden_citation_count = arms.iter().map(|arm| arm.forbidden_citation_count).sum();
    let no_answer_case_count = arms
        .iter()
        .filter(|arm| arm.no_answer_correct.is_some())
        .count();
    let no_answer_correct_count = arms
        .iter()
        .filter(|arm| arm.no_answer_correct == Some(true))
        .count();
    let reciprocal_ranks = arms
        .iter()
        .filter_map(|arm| arm.reciprocal_rank_at_five)
        .collect::<Vec<_>>();
    FinalAggregateArm {
        returned_citation_count,
        found_group_count,
        required_group_count,
        group_recall_at_five: ratio(
            usize::try_from(found_group_count).unwrap_or(usize::MAX),
            usize::try_from(required_group_count).unwrap_or(usize::MAX),
        ),
        mean_reciprocal_rank_at_five: (!reciprocal_ranks.is_empty())
            .then(|| mean(reciprocal_ranks.into_iter())),
        relevant_citation_count,
        citation_precision: ratio(
            usize::try_from(relevant_citation_count).unwrap_or(usize::MAX),
            usize::try_from(returned_citation_count).unwrap_or(usize::MAX),
        ),
        forbidden_citation_count,
        no_answer_correct_count: u32::try_from(no_answer_correct_count).unwrap_or(u32::MAX),
        no_answer_case_count: u32::try_from(no_answer_case_count).unwrap_or(u32::MAX),
        no_answer_accuracy: ratio(no_answer_correct_count, no_answer_case_count),
        provider_failure_count: u32::try_from(
            arms.iter().filter(|arm| arm.provider_failure).count(),
        )
        .unwrap_or(u32::MAX),
        provenance_exact_and_current: arms.iter().all(|arm| arm.provenance_exact_and_current),
        full_query_p95_micros: percentile_micros(arms.iter().map(|arm| arm.full_query_micros), 95),
    }
}

fn final_domain_reports(
    fixture: &ReplayFixture,
    cases: &[FinalCaseReport],
) -> Vec<FinalDomainReport> {
    fixture
        .domains
        .iter()
        .map(|domain| {
            let domain_cases = cases
                .iter()
                .filter(|case| case.domain == domain.id)
                .collect::<Vec<_>>();
            let b32 = aggregate_final(domain_cases.iter().map(|case| &case.b32));
            let g1 = aggregate_final(domain_cases.iter().map(|case| &case.g1));
            let sham = aggregate_final(domain_cases.iter().map(|case| &case.g1_sham));
            let b32_recall = b32.group_recall_at_five.unwrap_or(0.0);
            let g1_recall = g1.group_recall_at_five.unwrap_or(0.0);
            let sham_recall = sham.group_recall_at_five.unwrap_or(0.0);
            FinalDomainReport {
                id: domain.id.clone(),
                b32_group_recall_at_five: b32_recall,
                g1_group_recall_at_five: g1_recall,
                sham_group_recall_at_five: sham_recall,
                g1_improved_over_both: g1_recall > b32_recall && g1_recall > sham_recall,
            }
        })
        .collect()
}

fn bundle_fingerprints_unchanged(corpus: &ReplayCorpus) -> bool {
    let inspector = OkfBundleInspector::new(corpus.database.clone());
    corpus
        .bundle_fingerprints
        .iter()
        .all(|(collection_id, expected)| {
            inspector
                .inspect_bundle(*collection_id)
                .is_ok_and(|bundle| {
                    bundle.state == KnowledgeBundleState::Ready
                        && bundle.health.is_healthy()
                        && bundle.fingerprint == *expected
                })
        })
}

fn bundle_set_fingerprint(fingerprints: &BTreeMap<Uuid, String>) -> String {
    let mut values = fingerprints.values().collect::<Vec<_>>();
    values.sort_unstable();
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.as_bytes());
        hasher.update([0xff]);
    }
    hex::encode(hasher.finalize())
}

fn percentile_micros(values: impl Iterator<Item = u128>, percentile: usize) -> u128 {
    let mut values = values.collect::<Vec<_>>();
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let numerator = percentile
        .saturating_mul(values.len().saturating_sub(1))
        .saturating_add(99);
    let index = (numerator / 100).min(values.len().saturating_sub(1));
    values[index]
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, count) = values.fold((0.0, 0_usize), |(sum, count), value| {
        (sum + value, count.saturating_add(1))
    });
    if count == 0 { 0.0 } else { sum / count as f64 }
}

fn paired_bootstrap_lower_bound(domain_differences: &[f64]) -> f64 {
    if domain_differences.is_empty() {
        return 0.0;
    }
    let mut state = FINAL_BOOTSTRAP_SEED;
    let mut samples = Vec::with_capacity(FINAL_BOOTSTRAP_RESAMPLES);
    for _ in 0..FINAL_BOOTSTRAP_RESAMPLES {
        let mut sum = 0.0;
        for _ in 0..domain_differences.len() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let index = usize::try_from(state % domain_differences.len() as u64).unwrap_or(0);
            sum += domain_differences[index];
        }
        samples.push(sum / domain_differences.len() as f64);
    }
    samples.sort_by(f64::total_cmp);
    let index = ((FINAL_BOOTSTRAP_RESAMPLES - 1) * 25) / 1000;
    samples[index]
}

struct FinalGateInput<'a> {
    b32: &'a FinalAggregateArm,
    g1: &'a FinalAggregateArm,
    sham: &'a FinalAggregateArm,
    macro_gain_over_b32: f64,
    macro_gain_over_sham: f64,
    improved_domain_count: usize,
    bootstrap_over_b32: f64,
    bootstrap_over_sham: f64,
    lost_b32_groups: u32,
    retained_payload_bytes: usize,
    projection_micros: u128,
    g1_candidate_assembly_p95_micros: u128,
    healthy_fingerprint: bool,
}

fn final_rejection_reasons(input: FinalGateInput<'_>) -> Vec<String> {
    let mut reasons = Vec::new();
    if input.g1.group_recall_at_five.unwrap_or(0.0) + FINAL_EPSILON < FINAL_MIN_GROUP_RECALL {
        reasons.push("G1 group Recall@5 is below 0.90".to_owned());
    }
    if input.g1.citation_precision.unwrap_or(0.0) + FINAL_EPSILON < FINAL_MIN_CITATION_PRECISION {
        reasons.push("G1 citation precision is below 0.80".to_owned());
    }
    if [input.b32, input.g1, input.sham]
        .iter()
        .any(|arm| arm.no_answer_accuracy != Some(1.0))
    {
        reasons.push("at least one arm returned evidence for a no-answer case".to_owned());
    }
    if [input.b32, input.g1, input.sham]
        .iter()
        .any(|arm| arm.forbidden_citation_count > 0)
    {
        reasons.push("an arm returned forbidden evidence".to_owned());
    }
    if input.lost_b32_groups > 0 {
        reasons.push("G1 lost evidence groups covered by B32".to_owned());
    }
    if input.macro_gain_over_b32 + FINAL_EPSILON < FINAL_MIN_MACRO_GAIN {
        reasons.push("G1 macro recall gain over B32 is below 0.05".to_owned());
    }
    if input.macro_gain_over_sham + FINAL_EPSILON < FINAL_MIN_MACRO_GAIN {
        reasons.push("G1 macro recall gain over sham is below 0.05".to_owned());
    }
    if input.improved_domain_count < FINAL_MIN_IMPROVED_DOMAINS {
        reasons.push("G1 improved over both controls in fewer than five domains".to_owned());
    }
    if input.bootstrap_over_b32 <= 0.0 || input.bootstrap_over_sham <= 0.0 {
        reasons.push("paired domain bootstrap lower bound is not positive".to_owned());
    }
    let g1_mrr = input.g1.mean_reciprocal_rank_at_five.unwrap_or(0.0);
    if g1_mrr + FINAL_EPSILON < input.b32.mean_reciprocal_rank_at_five.unwrap_or(0.0)
        || g1_mrr + FINAL_EPSILON < input.sham.mean_reciprocal_rank_at_five.unwrap_or(0.0)
    {
        reasons.push("G1 MRR@5 regressed against a control".to_owned());
    }
    if input.retained_payload_bytes >= FINAL_MAX_GRAPH_PAYLOAD_BYTES {
        reasons.push("combined graph payload is not below one MiB".to_owned());
    }
    if input.projection_micros >= FINAL_MAX_PROJECTION_MICROS {
        reasons.push("graph projection did not finish below one second".to_owned());
    }
    if input.g1_candidate_assembly_p95_micros >= FINAL_MAX_ASSEMBLY_P95_MICROS {
        reasons.push("G1 expansion and candidate assembly p95 is not below 25 ms".to_owned());
    }
    let b32_relative =
        (input.b32.full_query_p95_micros as f64 * FINAL_FULL_QUERY_RELATIVE_SLACK).ceil() as u128;
    let b32_absolute = input
        .b32
        .full_query_p95_micros
        .saturating_add(FINAL_FULL_QUERY_ABSOLUTE_SLACK_MICROS);
    if input.g1.full_query_p95_micros >= FINAL_MAX_FULL_QUERY_P95_MICROS
        || input.g1.full_query_p95_micros > b32_relative.max(b32_absolute)
    {
        reasons.push("G1 full-query p95 exceeded its frozen latency budget".to_owned());
    }
    if [input.b32, input.g1, input.sham]
        .iter()
        .any(|arm| arm.provider_failure_count > 0)
    {
        reasons.push("an evaluation arm failed or returned a partial result".to_owned());
    }
    if [input.b32, input.g1, input.sham]
        .iter()
        .any(|arm| !arm.provenance_exact_and_current)
    {
        reasons.push("citation provenance was not exact and current".to_owned());
    }
    if !input.healthy_fingerprint {
        reasons.push("bundle health or fingerprint changed during the holdout".to_owned());
    }
    reasons
}

fn development_rejection_reasons(metrics: DevelopmentGateMetrics) -> Vec<String> {
    let mut reasons = Vec::new();
    if !metrics.healthy_fingerprint {
        reasons.push("bundle health or fingerprint changed during replay".to_owned());
    }
    if metrics.bidir_found_groups < metrics.c32_found_groups.saturating_add(MIN_GROUP_GAIN) {
        reasons.push("bidirectional graph gained fewer than two groups over C32".to_owned());
    }
    if metrics.bidir_found_groups < metrics.sham_found_groups.saturating_add(MIN_GROUP_GAIN) {
        reasons
            .push("real graph gained fewer than two groups over degree-preserving sham".to_owned());
    }
    if metrics.improved_domains < MIN_IMPROVED_DOMAIN_COUNT {
        reasons.push("graph improvement was concentrated in too few domains".to_owned());
    }
    if metrics.outgoing_unique_groups == 0 {
        reasons.push("outgoing links produced no unique rescue over C32".to_owned());
    }
    if metrics.backlink_unique_groups == 0 {
        reasons.push("backlinks produced no unique rescue over outgoing links".to_owned());
    }
    if metrics.outgoing_rescue_domains < MIN_RESCUE_DOMAIN_COUNT {
        reasons.push("outgoing-link rescues covered fewer than two domains".to_owned());
    }
    if metrics.backlink_rescue_domains < MIN_RESCUE_DOMAIN_COUNT {
        reasons.push("backlink rescues covered fewer than two domains".to_owned());
    }
    if metrics.regressed_cases > 0 {
        reasons.push("graph expansion regressed at least one C32 case".to_owned());
    }
    if !matches!(
        (metrics.bidir_support_density, metrics.c32_support_density),
        (Some(bidir), Some(c32)) if bidir >= c32
    ) {
        reasons.push("graph support density regressed against C32".to_owned());
    }
    if metrics.retained_payload_bytes > MAX_RETAINED_PAYLOAD_BYTES {
        reasons.push("graph projections exceeded the one MiB payload budget".to_owned());
    }
    if metrics.sham_unique_groups > 0 {
        reasons.push("sham graph produced a unique gold rescue".to_owned());
    }
    reasons
}

fn node_documents(
    graph: &MiniGraph,
    nodes: &[super::mini_graph::NodeId],
    logical_by_concept: &HashMap<Uuid, String>,
) -> Result<Vec<String>> {
    nodes
        .iter()
        .map(|node_id| {
            let node = graph.node(*node_id).context("graph node disappeared")?;
            logical_by_concept
                .get(&node.concept_id)
                .cloned()
                .context("graph concept has no replay identity")
        })
        .collect()
}

fn score_arm(candidates: Vec<String>, required_groups: &[Vec<String>]) -> ArmReport {
    let candidate_set = candidates
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let support_ids = required_groups
        .iter()
        .flatten()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let found_group_count = required_groups
        .iter()
        .filter(|group| group.iter().any(|id| candidate_set.contains(id.as_str())))
        .count();
    let support_candidate_count = candidate_set.intersection(&support_ids).count();
    let candidate_count = candidates.len();
    ArmReport {
        candidate_count: u32::try_from(candidate_count).unwrap_or(u32::MAX),
        found_group_count: u32::try_from(found_group_count).unwrap_or(u32::MAX),
        required_group_count: u32::try_from(required_groups.len()).unwrap_or(u32::MAX),
        recall: ratio(found_group_count, required_groups.len()),
        support_candidate_count: u32::try_from(support_candidate_count).unwrap_or(u32::MAX),
        support_density: ratio(support_candidate_count, candidate_count),
        candidate_document_ids: candidates,
    }
}

fn unique_groups(candidate: &ArmReport, control: &ArmReport, groups: &[Vec<String>]) -> u32 {
    let candidate_ids = candidate
        .candidate_document_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let control_ids = control
        .candidate_document_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let count = groups
        .iter()
        .filter(|group| {
            group.iter().any(|id| candidate_ids.contains(id.as_str()))
                && !group.iter().any(|id| control_ids.contains(id.as_str()))
        })
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn lost_groups(candidate: &ArmReport, control: &ArmReport, groups: &[Vec<String>]) -> u32 {
    unique_groups(control, candidate, groups)
}

fn aggregate<'a>(arms: impl Iterator<Item = &'a ArmReport>) -> AggregateArm {
    let mut aggregate = AggregateArm::default();
    for arm in arms {
        aggregate.candidate_count = aggregate
            .candidate_count
            .saturating_add(arm.candidate_count);
        aggregate.found_group_count = aggregate
            .found_group_count
            .saturating_add(arm.found_group_count);
        aggregate.required_group_count = aggregate
            .required_group_count
            .saturating_add(arm.required_group_count);
        aggregate.support_candidate_count = aggregate
            .support_candidate_count
            .saturating_add(arm.support_candidate_count);
    }
    aggregate.recall = ratio(
        usize::try_from(aggregate.found_group_count).unwrap_or(usize::MAX),
        usize::try_from(aggregate.required_group_count).unwrap_or(usize::MAX),
    );
    aggregate.support_density = ratio(
        usize::try_from(aggregate.support_candidate_count).unwrap_or(usize::MAX),
        usize::try_from(aggregate.candidate_count).unwrap_or(usize::MAX),
    );
    aggregate
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

fn write_report(report: &ReplayReport) -> Result<std::path::PathBuf> {
    let directory = workspace_root().join(REPORT_DIRECTORY);
    std::fs::create_dir_all(&directory).context("creating replay report directory")?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates Unix epoch")?
        .as_secs();
    let destination = directory.join(format!(
        "retrieval-mini-graph-real-development-{}-{epoch}.json",
        &report.fixture_sha256[..12]
    ));
    let temporary = destination.with_extension("json.tmp");
    let mut contents = serde_json::to_string_pretty(report).context("serializing replay report")?;
    contents.push('\n');
    std::fs::write(&temporary, contents)
        .with_context(|| format!("writing {}", temporary.display()))?;
    replace_file(&temporary, &destination)?;
    Ok(destination)
}

fn write_final_report(report: &FinalReplayReport) -> Result<std::path::PathBuf> {
    let directory = workspace_root().join(REPORT_DIRECTORY);
    std::fs::create_dir_all(&directory).context("creating final holdout report directory")?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates Unix epoch")?
        .as_secs();
    let fixture_prefix = report
        .fixture_sha256
        .get(..12)
        .context("final fixture hash is unexpectedly short")?;
    let destination = directory.join(format!(
        "retrieval-mini-graph-final-holdout-{fixture_prefix}-{epoch}.json"
    ));
    let temporary = destination.with_extension("json.tmp");
    let mut contents =
        serde_json::to_string_pretty(report).context("serializing final holdout report")?;
    contents.push('\n');
    std::fs::write(&temporary, contents)
        .with_context(|| format!("writing {}", temporary.display()))?;
    replace_file(&temporary, &destination)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioned_real_ranking_fixture_is_valid() {
        let loaded = load_fixture().unwrap();
        assert_eq!(loaded.fixture.domains.len(), 4);
        assert_eq!(
            loaded
                .fixture
                .domains
                .iter()
                .map(|domain| domain.cases.len())
                .sum::<usize>(),
            12
        );
    }

    #[test]
    fn development_gate_accepts_the_minimum_distributed_gain() {
        let metrics = DevelopmentGateMetrics {
            healthy_fingerprint: true,
            c32_found_groups: 10,
            c32_support_density: Some(0.1),
            bidir_found_groups: 12,
            bidir_support_density: Some(0.1),
            sham_found_groups: 10,
            outgoing_unique_groups: 1,
            backlink_unique_groups: 1,
            sham_unique_groups: 0,
            improved_domains: 3,
            outgoing_rescue_domains: 2,
            backlink_rescue_domains: 2,
            regressed_cases: 0,
            retained_payload_bytes: MAX_RETAINED_PAYLOAD_BYTES,
        };

        assert!(development_rejection_reasons(metrics).is_empty());
    }

    #[test]
    fn development_gate_rejects_concentrated_gain_and_case_regression() {
        let metrics = DevelopmentGateMetrics {
            healthy_fingerprint: true,
            c32_found_groups: 10,
            c32_support_density: Some(0.1),
            bidir_found_groups: 11,
            bidir_support_density: Some(0.1),
            sham_found_groups: 10,
            outgoing_unique_groups: 1,
            backlink_unique_groups: 1,
            sham_unique_groups: 0,
            improved_domains: 1,
            outgoing_rescue_domains: 1,
            backlink_rescue_domains: 1,
            regressed_cases: 1,
            retained_payload_bytes: MAX_RETAINED_PAYLOAD_BYTES,
        };

        let reasons = development_rejection_reasons(metrics);

        assert!(reasons.iter().any(|reason| reason.contains("over C32")));
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("too few domains"))
        );
        assert!(reasons.iter().any(|reason| reason.contains("regressed")));
    }

    #[test]
    fn lost_groups_detects_equal_count_evidence_substitution() {
        let groups = vec![vec!["a".to_owned()], vec!["b".to_owned()]];
        let c32 = score_arm(vec!["a".to_owned()], &groups);
        let graph = score_arm(vec!["b".to_owned()], &groups);

        assert_eq!(c32.found_group_count, graph.found_group_count);
        assert_eq!(lost_groups(&graph, &c32, &groups), 1);
    }

    #[test]
    fn constructed_final_holdout_contract_is_valid_without_reading_the_sealed_fixture() {
        let fixture = constructed_final_fixture();

        assert!(validate_fixture(&fixture, FixtureProfile::FinalHoldout).is_ok());
    }

    #[test]
    fn final_holdout_accepts_only_its_frozen_hash() {
        assert!(validate_fixture_hash(FixtureProfile::FinalHoldout, FINAL_FIXTURE_SHA256).is_ok());
        assert!(validate_fixture_hash(FixtureProfile::FinalHoldout, &"0".repeat(64)).is_err());
        assert!(validate_fixture_hash(FixtureProfile::Development, &"0".repeat(64)).is_ok());
    }

    #[test]
    fn final_holdout_rejects_required_evidence_on_a_no_answer_case() {
        let mut fixture = constructed_final_fixture();
        fixture.domains[0].cases[4].required_evidence_groups = vec![vec![EvidenceId {
            document_id: "domain_0_doc_0".to_owned(),
            section_id: "section_0".to_owned(),
        }]];

        let error = validate_fixture(&fixture, FixtureProfile::FinalHoldout).unwrap_err();

        assert!(error.to_string().contains("no-answer"));
    }

    #[test]
    fn graph_arm_keeps_exact_seeds_then_neighbor_then_exact_backfill() {
        let collection_id = Uuid::new_v4();
        let concept_ids = (0..32).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        let nodes = concept_ids
            .iter()
            .map(|concept_id| GraphNodeInput {
                node: GraphNode {
                    concept_id: *concept_id,
                    collection_id,
                },
                state: NodeState::Current,
            })
            .collect::<Vec<_>>();
        let graph = MiniGraph::build(
            &nodes,
            &[GraphLinkInput {
                source: concept_ids[0],
                target: concept_ids[15],
                disposition: LinkDisposition::ReviewedInternal,
            }],
        )
        .unwrap();
        let ranked = concept_ids
            .iter()
            .map(|concept_id| RetrievalEvaluationCandidate {
                collection_id,
                concept_id: *concept_id,
                chunk_id: Uuid::new_v4(),
                source_revision: 1,
            })
            .collect::<Vec<_>>();
        let scope = QueryScope {
            purpose: GraphPurpose::LocalAssistant,
            authorized_collections: [collection_id].into_iter().collect(),
            external_ai_collections: BTreeSet::new(),
        };
        let revisions = concept_ids
            .iter()
            .map(|concept_id| (*concept_id, 1))
            .collect::<HashMap<_, _>>();

        let nominees = assemble_graph_nominees(
            &graph,
            &ranked,
            &scope,
            &revisions,
            &stable_labels(&concept_ids),
        )
        .unwrap();

        assert!(matches!(
            nominees[10],
            RetrievalEvaluationNominee::Concept { concept_id, .. }
                if concept_id == concept_ids[15]
        ));
        assert!(matches!(
            nominees[11],
            RetrievalEvaluationNominee::Exact(candidate)
                if candidate.chunk_id == ranked[10].chunk_id
        ));
    }

    #[test]
    fn graph_arm_never_expands_from_a_concept_below_the_first_ten_chunks() {
        let collection_id = Uuid::new_v4();
        let concept_ids = (0..32).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
        let nodes = concept_ids
            .iter()
            .map(|concept_id| GraphNodeInput {
                node: GraphNode {
                    concept_id: *concept_id,
                    collection_id,
                },
                state: NodeState::Current,
            })
            .collect::<Vec<_>>();
        let graph = MiniGraph::build(
            &nodes,
            &[GraphLinkInput {
                source: concept_ids[9],
                target: concept_ids[20],
                disposition: LinkDisposition::ReviewedInternal,
            }],
        )
        .unwrap();
        let ranked_concepts = std::iter::once(concept_ids[0])
            .chain(std::iter::once(concept_ids[0]))
            .chain(concept_ids.iter().copied().skip(1))
            .take(CONTROL_LIMIT)
            .collect::<Vec<_>>();
        let ranked = ranked_concepts
            .iter()
            .map(|concept_id| RetrievalEvaluationCandidate {
                collection_id,
                concept_id: *concept_id,
                chunk_id: Uuid::new_v4(),
                source_revision: 1,
            })
            .collect::<Vec<_>>();
        let scope = QueryScope {
            purpose: GraphPurpose::LocalAssistant,
            authorized_collections: [collection_id].into_iter().collect(),
            external_ai_collections: BTreeSet::new(),
        };
        let revisions = concept_ids
            .iter()
            .map(|concept_id| (*concept_id, 1))
            .collect::<HashMap<_, _>>();

        let nominees = assemble_graph_nominees(
            &graph,
            &ranked,
            &scope,
            &revisions,
            &stable_labels(&concept_ids),
        )
        .unwrap();

        assert!(matches!(
            nominees[10],
            RetrievalEvaluationNominee::Exact(candidate)
                if candidate.chunk_id == ranked[10].chunk_id
        ));
        assert!(!nominees.iter().any(|nominee| matches!(
            nominee,
            RetrievalEvaluationNominee::Concept { concept_id, .. }
                if *concept_id == concept_ids[20]
        )));
    }

    #[test]
    fn final_arm_order_rotates_as_a_latin_square() {
        assert_eq!(
            [final_arm_order(0), final_arm_order(1), final_arm_order(2)],
            [
                [FinalArmKind::B32, FinalArmKind::Graph, FinalArmKind::Sham],
                [FinalArmKind::Graph, FinalArmKind::Sham, FinalArmKind::B32],
                [FinalArmKind::Sham, FinalArmKind::B32, FinalArmKind::Graph],
            ]
        );
    }

    #[test]
    fn paired_bootstrap_lower_bound_is_positive_for_uniform_positive_differences() {
        let lower_bound = paired_bootstrap_lower_bound(&[0.1; FINAL_DOMAIN_COUNT]);

        assert!(lower_bound > 0.0);
    }

    #[test]
    fn final_shadow_gate_accepts_the_exact_frozen_boundaries() {
        let b32 = passing_final_aggregate(0.85, 0.70, 100_000);
        let g1 = passing_final_aggregate(0.90, 0.70, 105_000);
        let sham = passing_final_aggregate(0.85, 0.70, 100_000);

        let reasons = final_rejection_reasons(passing_final_gate_input(&b32, &g1, &sham));

        assert!(reasons.is_empty());
    }

    #[test]
    fn final_shadow_gate_rejects_evidence_and_integrity_failures() {
        let mut b32 = passing_final_aggregate(0.85, 0.70, 100_000);
        let mut g1 = passing_final_aggregate(0.90, 0.70, 105_000);
        let mut sham = passing_final_aggregate(0.85, 0.70, 100_000);
        b32.no_answer_accuracy = Some(0.0);
        g1.forbidden_citation_count = 1;
        g1.provider_failure_count = 1;
        sham.provenance_exact_and_current = false;
        let mut input = passing_final_gate_input(&b32, &g1, &sham);
        input.lost_b32_groups = 1;
        input.healthy_fingerprint = false;

        let reasons = final_rejection_reasons(input);

        for expected in [
            "no-answer",
            "forbidden",
            "lost evidence",
            "failed or returned a partial",
            "provenance",
            "fingerprint",
        ] {
            assert!(reasons.iter().any(|reason| reason.contains(expected)));
        }
    }

    #[test]
    fn final_shadow_gate_rejects_quality_and_latency_failures() {
        let b32 = passing_final_aggregate(0.85, 0.70, 100_000);
        let mut g1 = passing_final_aggregate(0.89, 0.60, FINAL_MAX_FULL_QUERY_P95_MICROS);
        let sham = passing_final_aggregate(0.85, 0.70, 100_000);
        g1.citation_precision = Some(0.79);
        let mut input = passing_final_gate_input(&b32, &g1, &sham);
        input.macro_gain_over_b32 = 0.04;
        input.macro_gain_over_sham = 0.04;
        input.improved_domain_count = FINAL_MIN_IMPROVED_DOMAINS - 1;
        input.bootstrap_over_b32 = 0.0;
        input.retained_payload_bytes = FINAL_MAX_GRAPH_PAYLOAD_BYTES;
        input.projection_micros = FINAL_MAX_PROJECTION_MICROS;
        input.g1_candidate_assembly_p95_micros = FINAL_MAX_ASSEMBLY_P95_MICROS;

        let reasons = final_rejection_reasons(input);

        for expected in [
            "Recall@5",
            "citation precision",
            "macro recall gain",
            "fewer than five domains",
            "bootstrap",
            "MRR@5",
            "one MiB",
            "one second",
            "25 ms",
            "full-query p95",
        ] {
            assert!(reasons.iter().any(|reason| reason.contains(expected)));
        }
    }

    fn constructed_final_fixture() -> ReplayFixture {
        ReplayFixture {
            schema_version: FIXTURE_SCHEMA_VERSION,
            experiment_id: "constructed-final-holdout".to_owned(),
            domains: (0..FINAL_DOMAIN_COUNT)
                .map(|domain_index| {
                    let domain_id = format!("domain_{domain_index}");
                    let documents = (0..FINAL_DOCUMENTS_PER_DOMAIN)
                        .map(|document_index| {
                            let id = format!("{domain_id}_doc_{document_index}");
                            let links = match document_index {
                                0 => vec![DocumentLinkFixture {
                                    label: "linked record".to_owned(),
                                    target_document_id: format!("{domain_id}_doc_1"),
                                }],
                                2 => vec![DocumentLinkFixture {
                                    label: "linked guide".to_owned(),
                                    target_document_id: format!("{domain_id}_doc_3"),
                                }],
                                _ => Vec::new(),
                            };
                            DocumentFixture {
                                id,
                                title: format!("Document {document_index}"),
                                description: "Synthetic final holdout document".to_owned(),
                                heading: None,
                                text: None,
                                sections: constructed_sections(),
                                links,
                            }
                        })
                        .collect::<Vec<_>>();
                    let evidence = |document_index, section_index| EvidenceId {
                        document_id: format!("{domain_id}_doc_{document_index}"),
                        section_id: format!("section_{section_index}"),
                    };
                    DomainFixture {
                        id: domain_id.clone(),
                        collection_name: format!("Domain {domain_index}"),
                        language: if domain_index < 4 { "es" } else { "en" }.to_owned(),
                        distractor_count: FINAL_MIN_DISTRACTORS_PER_DOMAIN,
                        distractor_title: "Synthetic distractor".to_owned(),
                        distractor_description: "Unrelated synthetic note".to_owned(),
                        distractor_heading: None,
                        distractor_text: None,
                        distractor_sections: constructed_sections(),
                        documents,
                        cases: vec![
                            constructed_answerable_case(&domain_id, 0, vec![vec![evidence(0, 0)]]),
                            constructed_answerable_case(&domain_id, 1, vec![vec![evidence(1, 0)]]),
                            constructed_answerable_case(&domain_id, 2, vec![vec![evidence(2, 0)]]),
                            constructed_answerable_case(
                                &domain_id,
                                3,
                                vec![vec![evidence(3, 0)], vec![evidence(4, 0)]],
                            ),
                            CaseFixture {
                                id: format!("{domain_id}_case_4"),
                                question: "Plausible question with no evidence".to_owned(),
                                required_document_groups: Vec::new(),
                                expected_answerable: Some(false),
                                required_evidence_groups: Vec::new(),
                                forbidden_evidence: vec![evidence(5, 0)],
                            },
                        ],
                    }
                })
                .collect(),
        }
    }

    fn constructed_sections() -> Vec<SectionFixture> {
        (0..FINAL_SECTIONS_PER_DOCUMENT)
            .map(|section_index| SectionFixture {
                id: format!("section_{section_index}"),
                heading: format!("Section {section_index}"),
                text: format!("Synthetic evidence paragraph {section_index}."),
            })
            .collect()
    }

    fn constructed_answerable_case(
        domain_id: &str,
        case_index: usize,
        required_evidence_groups: Vec<Vec<EvidenceId>>,
    ) -> CaseFixture {
        CaseFixture {
            id: format!("{domain_id}_case_{case_index}"),
            question: format!("Synthetic answerable question {case_index}"),
            required_document_groups: Vec::new(),
            expected_answerable: Some(true),
            required_evidence_groups,
            forbidden_evidence: vec![EvidenceId {
                document_id: format!("{domain_id}_doc_5"),
                section_id: "section_0".to_owned(),
            }],
        }
    }

    fn stable_labels(ids: &[Uuid]) -> HashMap<Uuid, String> {
        ids.iter()
            .enumerate()
            .map(|(index, id)| (*id, format!("concept_{index:02}")))
            .collect()
    }

    fn passing_final_aggregate(
        group_recall_at_five: f64,
        mean_reciprocal_rank_at_five: f64,
        full_query_p95_micros: u128,
    ) -> FinalAggregateArm {
        FinalAggregateArm {
            returned_citation_count: 10,
            found_group_count: 9,
            required_group_count: 10,
            group_recall_at_five: Some(group_recall_at_five),
            mean_reciprocal_rank_at_five: Some(mean_reciprocal_rank_at_five),
            relevant_citation_count: 8,
            citation_precision: Some(0.80),
            forbidden_citation_count: 0,
            no_answer_correct_count: 8,
            no_answer_case_count: 8,
            no_answer_accuracy: Some(1.0),
            provider_failure_count: 0,
            provenance_exact_and_current: true,
            full_query_p95_micros,
        }
    }

    fn passing_final_gate_input<'a>(
        b32: &'a FinalAggregateArm,
        g1: &'a FinalAggregateArm,
        sham: &'a FinalAggregateArm,
    ) -> FinalGateInput<'a> {
        FinalGateInput {
            b32,
            g1,
            sham,
            macro_gain_over_b32: FINAL_MIN_MACRO_GAIN,
            macro_gain_over_sham: FINAL_MIN_MACRO_GAIN,
            improved_domain_count: FINAL_MIN_IMPROVED_DOMAINS,
            bootstrap_over_b32: 0.01,
            bootstrap_over_sham: 0.01,
            lost_b32_groups: 0,
            retained_payload_bytes: FINAL_MAX_GRAPH_PAYLOAD_BYTES - 1,
            projection_micros: FINAL_MAX_PROJECTION_MICROS - 1,
            g1_candidate_assembly_p95_micros: FINAL_MAX_ASSEMBLY_P95_MICROS - 1,
            healthy_fingerprint: true,
        }
    }
}
