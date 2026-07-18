//! Development-only reranking ablation over an exact semantic candidate pool.
//!
//! The graph may permute candidates already present in B32, but it cannot
//! nominate a concept, materialize evidence, or bypass the normal search scope.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use airwiki_core::{
    DeterministicEvidenceRelevanceProvider, EmbeddingProvider, FastEmbedE5Small,
    KnowledgeBundleState, OkfBundleInspector, PinnedE5Snapshot, RetrievalEvaluationCandidate,
};
use airwiki_types::SearchPurpose;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    CONTROL_LIMIT, FixtureProfile, LoadedFixture, ReplayCorpus, build_corpus, load_fixture_from,
};
use crate::retrieval::mini_graph::{GraphPurpose, MiniGraph, NodeId, QueryScope};
use crate::retrieval::sham_graph::{StructuralShamStats, build_weak_structural_sham};
use crate::workspace_root;

const FIXTURE_PATH: &str = "fixtures/retrieval/okf-diffusion-development-v1.json";
const EXPECTED_FIXTURE_SHA256: &str =
    "7855a88051a4f4117d9230a8acb350c12202e22e9aea460b081c3d81ab8a6468";
const EXPECTED_EXPERIMENT_ID: &str = "okf-diffusion-development-v1";
const REPORT_DIRECTORY: &str = "target/evals";
const REPORT_SCHEMA_VERSION: u32 = 1;
const NODE_ID: &str = "diffusion-rerank-development";
const OUTPUT_LIMIT: usize = 10;
const FROZEN_PREFIX_SIZE: usize = 8;
const RERANK_BLOCK_SIZE: usize = 4;
const MAX_MOVEMENT: usize = 3;
const EXPECTED_DOMAIN_COUNT: usize = 4;
const EXPECTED_CASE_COUNT: usize = 16;
const EXPECTED_DOCUMENT_COUNT: usize = 84;
const MIN_MACRO_RECALL_GAIN: f64 = 0.05;
const MIN_IMPROVED_DOMAIN_COUNT: usize = 3;
const MIN_CUTOFF_OPPORTUNITY_COUNT: u32 = 4;
const MIN_SHAM_REWIRE_RATIO: f64 = 0.80;
const MAX_GRAPH_PAIR_BYTES: usize = 1024 * 1024;
const MAX_PROJECTION_MICROS: u128 = 1_000_000;
const MAX_RERANK_P95_MICROS: u128 = 2_000;
const EPSILON: f64 = 1e-12;
const SHAM_TIEBREAK_SEED: &[u8] = b"airwiki-okf-diffusion-sham-order-v1";
const POLICY: &str = "airwiki-okf-diffusion-development-v1;ranking=bm25-e5-rrf-b32;candidate-membership=exact-b32-only;prior=1/(60+rank);edges=reviewed-internal-weak;diffusion=one-hop-symmetric-degree-normalized-tau-1;prefix=first-8-frozen;blocks=4;output=10;control=exact-weak-degree-preserving;sham-tiebreak=sha256(seed-v1,domain-ordinal,document-ordinal);production=false";

#[derive(Debug, Clone, Copy, Default)]
struct ArmAccumulator {
    found_group_count: u32,
    required_group_count: u32,
    reciprocal_rank_sum: f64,
    support_candidate_count: u32,
    candidate_count: u32,
}

impl ArmAccumulator {
    fn observe(&mut self, candidates: &[String], groups: &[Vec<String>]) {
        self.candidate_count = self
            .candidate_count
            .saturating_add(u32::try_from(candidates.len()).unwrap_or(u32::MAX));
        let support = groups
            .iter()
            .flatten()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        self.support_candidate_count = self.support_candidate_count.saturating_add(
            u32::try_from(
                candidates
                    .iter()
                    .filter(|candidate| support.contains(candidate.as_str()))
                    .count(),
            )
            .unwrap_or(u32::MAX),
        );
        self.required_group_count = self
            .required_group_count
            .saturating_add(u32::try_from(groups.len()).unwrap_or(u32::MAX));
        for group in groups {
            if let Some(rank) = candidates
                .iter()
                .position(|candidate| group.iter().any(|required| required == candidate))
            {
                self.found_group_count = self.found_group_count.saturating_add(1);
                self.reciprocal_rank_sum += 1.0 / (rank.saturating_add(1) as f64);
            }
        }
    }

    fn report(self) -> ArmReport {
        ArmReport {
            found_group_count: self.found_group_count,
            required_group_count: self.required_group_count,
            recall_at_ten: ratio(self.found_group_count, self.required_group_count),
            mean_reciprocal_rank_at_ten: (self.required_group_count != 0)
                .then(|| self.reciprocal_rank_sum / f64::from(self.required_group_count)),
            support_density: ratio(self.support_candidate_count, self.candidate_count),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
struct ArmReport {
    found_group_count: u32,
    required_group_count: u32,
    recall_at_ten: Option<f64>,
    mean_reciprocal_rank_at_ten: Option<f64>,
    support_density: Option<f64>,
}

#[derive(Debug)]
struct CaseObservation {
    domain_index: usize,
    language: String,
    baseline_found: Vec<bool>,
    real_found: Vec<bool>,
    sham_found: Vec<bool>,
    baseline_candidates: Vec<String>,
    real_candidates: Vec<String>,
    sham_candidates: Vec<String>,
    cutoff_opportunity_count: u32,
}

#[derive(Debug, Serialize)]
struct LanguageReport {
    language: String,
    baseline_recall_at_ten: Option<f64>,
    real_recall_at_ten: Option<f64>,
    sham_recall_at_ten: Option<f64>,
}

#[derive(Debug, Serialize)]
struct DiffusionReport {
    schema_version: u32,
    experiment_id: String,
    fixture_sha256: String,
    evaluation_policy_fingerprint: String,
    embedding_profile: String,
    target_os: String,
    target_arch: String,
    domain_count: u32,
    case_count: u32,
    concept_count: u32,
    edge_count: u32,
    graph_fingerprint: String,
    sham_graph_fingerprint: String,
    sham_linked_collection_count: u32,
    sham_retained_original_edge_count: u32,
    sham_rewired_edge_count: u32,
    sham_unchanged_collection_count: u32,
    sham_rewire_ratio: Option<f64>,
    retained_graph_pair_bytes: usize,
    projection_micros: u128,
    real_rerank_p95_micros: u128,
    sham_rerank_p95_micros: u128,
    baseline: ArmReport,
    real: ArmReport,
    sham: ArmReport,
    baseline_macro_domain_recall: f64,
    real_macro_domain_recall: f64,
    sham_macro_domain_recall: f64,
    real_macro_gain_over_baseline: f64,
    real_macro_gain_over_sham: f64,
    improved_domain_count_over_both: u32,
    lost_baseline_group_count: u32,
    cutoff_opportunity_count: u32,
    language_reports: Vec<LanguageReport>,
    exact_pool_invariants_passed: bool,
    healthy_fingerprint_gate_passed: bool,
    development_gate_passed: bool,
    production_promotion_ready: bool,
    rejection_reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct GateInput {
    baseline: ArmReport,
    real: ArmReport,
    sham: ArmReport,
    baseline_macro: f64,
    real_macro: f64,
    sham_macro: f64,
    improved_domains: usize,
    lost_baseline_groups: u32,
    cutoff_opportunities: u32,
    language_regression: bool,
    exact_pool_invariants: bool,
    healthy_fingerprint: bool,
    sham_stats: StructuralShamStats,
    retained_graph_pair_bytes: usize,
    projection_micros: u128,
    real_rerank_p95_micros: u128,
    sham_rerank_p95_micros: u128,
}

pub(super) async fn evaluate(embedding_snapshot: &Path) -> Result<()> {
    ensure!(
        !cfg!(debug_assertions),
        "graph-conditioned diffusion must run in a release build"
    );
    let loaded = load_fixture_from(FIXTURE_PATH, FixtureProfile::Development)?;
    validate_experiment_shape(&loaded)?;
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 4);
    let embeddings: Arc<dyn EmbeddingProvider> = Arc::new(FastEmbedE5Small::from_snapshot(
        &PinnedE5Snapshot::open(embedding_snapshot)?,
        threads,
    )?);
    let embedding_profile = embeddings.model_id().to_owned();
    let relevance = Arc::new(DeterministicEvidenceRelevanceProvider);
    let corpus = build_corpus(&loaded.fixture, Arc::clone(&embeddings), relevance, NODE_ID).await?;
    let weak_projection_started = Instant::now();
    let sham_labels = opaque_sham_labels(&loaded.fixture, &corpus.concept_by_logical)?;
    let weak_sham =
        build_weak_structural_sham(&corpus.node_inputs, &corpus.link_inputs, &sham_labels)?;
    let sham_graph = MiniGraph::build(&corpus.node_inputs, &weak_sham.links)?;
    let projection_micros = corpus
        .projection_micros
        .saturating_add(weak_projection_started.elapsed().as_micros());
    let report = run(
        &loaded,
        &corpus,
        &sham_graph,
        weak_sham.stats,
        projection_micros,
        embedding_profile,
    )
    .await?;
    let destination = write_report(&report)?;
    ensure!(
        report.development_gate_passed,
        "graph-conditioned diffusion did not pass its frozen development gate; report written to {}",
        destination.display()
    );
    println!(
        "graph-conditioned diffusion passed its development gate; report written to {} (production promotion remains disabled)",
        destination.display()
    );
    Ok(())
}

fn validate_experiment_shape(loaded: &LoadedFixture) -> Result<()> {
    ensure!(
        loaded.sha256 == EXPECTED_FIXTURE_SHA256,
        "frozen diffusion development fixture hash mismatch"
    );
    ensure!(
        loaded.fixture.experiment_id == EXPECTED_EXPERIMENT_ID,
        "unexpected diffusion experiment identity"
    );
    ensure!(
        loaded.fixture.domains.len() == EXPECTED_DOMAIN_COUNT,
        "diffusion development fixture must contain exactly {EXPECTED_DOMAIN_COUNT} domains"
    );
    let case_count = loaded
        .fixture
        .domains
        .iter()
        .map(|domain| domain.cases.len())
        .sum::<usize>();
    ensure!(
        case_count == EXPECTED_CASE_COUNT,
        "diffusion development fixture must contain exactly {EXPECTED_CASE_COUNT} cases"
    );
    let document_count = loaded
        .fixture
        .domains
        .iter()
        .map(|domain| {
            domain
                .documents
                .len()
                .saturating_add(domain.distractor_count)
        })
        .sum::<usize>();
    ensure!(
        document_count == EXPECTED_DOCUMENT_COUNT,
        "diffusion development fixture must contain exactly {EXPECTED_DOCUMENT_COUNT} documents"
    );
    let language_counts =
        loaded
            .fixture
            .domains
            .iter()
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, domain| {
                *counts.entry(domain.language.as_str()).or_default() += 1;
                counts
            });
    ensure!(
        language_counts.get("es") == Some(&2) && language_counts.get("en") == Some(&2),
        "diffusion development fixture requires two Spanish and two English domains"
    );
    Ok(())
}

async fn run(
    loaded: &LoadedFixture,
    corpus: &ReplayCorpus,
    sham_graph: &MiniGraph,
    sham_stats: StructuralShamStats,
    projection_micros: u128,
    embedding_profile: String,
) -> Result<DiffusionReport> {
    ensure!(
        corpus.concept_by_logical.values().all(|concept_id| corpus
            .database
            .chunks_for_concept(*concept_id)
            .is_ok_and(|chunks| chunks.len() == 1 && chunks[0].source_revision == 1)),
        "diffusion development corpus requires one current chunk per concept"
    );
    let scope = QueryScope {
        purpose: GraphPurpose::LocalAssistant,
        authorized_collections: corpus.collection_ids.clone(),
        external_ai_collections: BTreeSet::new(),
    };
    let mut observations = Vec::new();
    let mut real_rerank_micros = Vec::new();
    let mut sham_rerank_micros = Vec::new();
    let mut exact_pool_invariants_passed = true;

    for (domain_index, domain) in loaded.fixture.domains.iter().enumerate() {
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
                ranked.len() == CONTROL_LIMIT,
                "diffusion development case did not produce an exact B32 pool"
            );
            ensure!(
                ranked
                    .iter()
                    .all(|candidate| candidate.source_revision == 1),
                "diffusion development ranking returned a stale revision"
            );
            let pool = candidate_nodes(&ranked, &corpus.graph)?;

            let real_started = Instant::now();
            let real_order = corpus.graph.rerank_one_hop_diffusion(&pool, &scope)?;
            real_rerank_micros.push(real_started.elapsed().as_micros());
            let sham_started = Instant::now();
            let sham_order = sham_graph.rerank_one_hop_diffusion(&pool, &scope)?;
            sham_rerank_micros.push(sham_started.elapsed().as_micros());

            exact_pool_invariants_passed &= validate_permutation(&real_order.permutation).is_ok()
                && validate_permutation(&sham_order.permutation).is_ok();
            let baseline_documents = candidate_documents(&ranked, corpus)?;
            let real_candidates = ordered_prefix(&ranked, &real_order.permutation)?;
            let sham_candidates = ordered_prefix(&ranked, &sham_order.permutation)?;
            let real_documents = candidate_documents(&real_candidates, corpus)?;
            let sham_documents = candidate_documents(&sham_candidates, corpus)?;
            let baseline_top_ten = baseline_documents[..OUTPUT_LIMIT].to_vec();
            let cutoff_opportunity_count =
                cutoff_opportunities(&baseline_documents, &case.required_document_groups);
            observations.push(CaseObservation {
                domain_index,
                language: domain.language.clone(),
                baseline_found: found_groups(&baseline_top_ten, &case.required_document_groups),
                real_found: found_groups(&real_documents, &case.required_document_groups),
                sham_found: found_groups(&sham_documents, &case.required_document_groups),
                baseline_candidates: baseline_top_ten,
                real_candidates: real_documents,
                sham_candidates: sham_documents,
                cutoff_opportunity_count,
            });
        }
    }

    let (baseline, real, sham) = aggregate_arms(&observations, &loaded.fixture);
    let baseline_macro = macro_domain_recall(&observations, ArmKind::Baseline);
    let real_macro = macro_domain_recall(&observations, ArmKind::Real);
    let sham_macro = macro_domain_recall(&observations, ArmKind::Sham);
    let improved_domains = improved_domain_count(&observations, loaded.fixture.domains.len());
    let lost_baseline_groups = observations
        .iter()
        .map(|observation| {
            observation
                .baseline_found
                .iter()
                .zip(&observation.real_found)
                .filter(|(baseline_found, real_found)| **baseline_found && !**real_found)
                .count()
        })
        .sum::<usize>();
    let cutoff_opportunities = observations
        .iter()
        .map(|observation| observation.cutoff_opportunity_count)
        .sum();
    let language_reports = language_reports(&observations);
    let language_regression = language_reports.iter().any(|language| {
        language.real_recall_at_ten.unwrap_or(0.0) + EPSILON
            < language.baseline_recall_at_ten.unwrap_or(0.0)
    });
    let healthy_fingerprint = healthy_fingerprints(corpus);
    let retained_graph_pair_bytes = corpus
        .graph
        .retained_payload_bytes()
        .saturating_add(sham_graph.retained_payload_bytes());
    let real_rerank_p95_micros = percentile(&mut real_rerank_micros, 95);
    let sham_rerank_p95_micros = percentile(&mut sham_rerank_micros, 95);
    let gate = GateInput {
        baseline,
        real,
        sham,
        baseline_macro,
        real_macro,
        sham_macro,
        improved_domains,
        lost_baseline_groups: u32::try_from(lost_baseline_groups).unwrap_or(u32::MAX),
        cutoff_opportunities,
        language_regression,
        exact_pool_invariants: exact_pool_invariants_passed,
        healthy_fingerprint,
        sham_stats,
        retained_graph_pair_bytes,
        projection_micros,
        real_rerank_p95_micros,
        sham_rerank_p95_micros,
    };
    let rejection_reasons = rejection_reasons(&gate);
    Ok(DiffusionReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment_id: loaded.fixture.experiment_id.clone(),
        fixture_sha256: loaded.sha256.clone(),
        evaluation_policy_fingerprint: hex::encode(Sha256::digest(POLICY.as_bytes())),
        embedding_profile,
        target_os: std::env::consts::OS.to_owned(),
        target_arch: std::env::consts::ARCH.to_owned(),
        domain_count: u32::try_from(loaded.fixture.domains.len()).unwrap_or(u32::MAX),
        case_count: u32::try_from(observations.len()).unwrap_or(u32::MAX),
        concept_count: corpus.graph.node_count(),
        edge_count: corpus.graph.edge_count(),
        graph_fingerprint: corpus.graph.fingerprint().to_owned(),
        sham_graph_fingerprint: sham_graph.fingerprint().to_owned(),
        sham_linked_collection_count: sham_stats.collection_count,
        sham_retained_original_edge_count: sham_stats.retained_original_edge_count,
        sham_rewired_edge_count: sham_stats.rewired_edge_count,
        sham_unchanged_collection_count: sham_stats.unchanged_collection_count,
        sham_rewire_ratio: ratio(
            sham_stats.rewired_edge_count,
            sham_stats
                .rewired_edge_count
                .saturating_add(sham_stats.retained_original_edge_count),
        ),
        retained_graph_pair_bytes,
        projection_micros,
        real_rerank_p95_micros,
        sham_rerank_p95_micros,
        baseline,
        real,
        sham,
        baseline_macro_domain_recall: baseline_macro,
        real_macro_domain_recall: real_macro,
        sham_macro_domain_recall: sham_macro,
        real_macro_gain_over_baseline: real_macro - baseline_macro,
        real_macro_gain_over_sham: real_macro - sham_macro,
        improved_domain_count_over_both: u32::try_from(improved_domains).unwrap_or(u32::MAX),
        lost_baseline_group_count: gate.lost_baseline_groups,
        cutoff_opportunity_count: cutoff_opportunities,
        language_reports,
        exact_pool_invariants_passed,
        healthy_fingerprint_gate_passed: healthy_fingerprint,
        development_gate_passed: rejection_reasons.is_empty(),
        production_promotion_ready: false,
        rejection_reasons,
    })
}

fn candidate_nodes(
    candidates: &[RetrievalEvaluationCandidate],
    graph: &MiniGraph,
) -> Result<Vec<NodeId>> {
    candidates
        .iter()
        .map(|candidate| {
            graph
                .node_id(candidate.concept_id)
                .context("B32 candidate is unavailable in the inspected graph")
        })
        .collect()
}

fn opaque_sham_labels(
    fixture: &super::ReplayFixture,
    concept_by_logical: &HashMap<String, uuid::Uuid>,
) -> Result<HashMap<uuid::Uuid, String>> {
    let mut labels = HashMap::with_capacity(concept_by_logical.len());
    let mut unique_labels = BTreeSet::new();
    for (domain_ordinal, domain) in fixture.domains.iter().enumerate() {
        let logical_ids = domain
            .documents
            .iter()
            .map(|document| document.id.clone())
            .chain(
                (1..=domain.distractor_count)
                    .map(|ordinal| format!("{}_distractor_{ordinal:02}", domain.id)),
            );
        for (document_ordinal, logical_id) in logical_ids.enumerate() {
            let concept_id = concept_by_logical
                .get(&logical_id)
                .copied()
                .context("diffusion sham control order lost a concept")?;
            let label = sham_control_label(domain_ordinal, document_ordinal)?;
            ensure!(
                unique_labels.insert(label.clone()),
                "diffusion sham control order produced a duplicate key"
            );
            ensure!(
                labels.insert(concept_id, label).is_none(),
                "diffusion sham input contains a duplicate concept"
            );
        }
    }
    ensure!(
        labels.len() == concept_by_logical.len(),
        "diffusion sham control order did not cover every concept"
    );
    Ok(labels)
}

fn sham_control_label(domain_ordinal: usize, document_ordinal: usize) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(SHAM_TIEBREAK_SEED);
    hasher.update(
        u64::try_from(domain_ordinal)
            .context("diffusion sham domain ordinal exceeds u64")?
            .to_be_bytes(),
    );
    hasher.update(
        u64::try_from(document_ordinal)
            .context("diffusion sham document ordinal exceeds u64")?
            .to_be_bytes(),
    );
    Ok(hex::encode(hasher.finalize()))
}

fn candidate_documents(
    candidates: &[RetrievalEvaluationCandidate],
    corpus: &ReplayCorpus,
) -> Result<Vec<String>> {
    candidates
        .iter()
        .map(|candidate| {
            corpus
                .logical_by_concept
                .get(&candidate.concept_id)
                .cloned()
                .context("B32 candidate has no synthetic document identity")
        })
        .collect()
}

fn ordered_prefix(
    ranked: &[RetrievalEvaluationCandidate],
    permutation: &[usize],
) -> Result<Vec<RetrievalEvaluationCandidate>> {
    validate_permutation(permutation)?;
    permutation
        .iter()
        .take(OUTPUT_LIMIT)
        .map(|index| {
            ranked
                .get(*index)
                .copied()
                .context("diffusion permutation index is outside B32")
        })
        .collect()
}

fn validate_permutation(permutation: &[usize]) -> Result<()> {
    ensure!(
        permutation.len() == CONTROL_LIMIT,
        "diffusion result is not a complete B32 permutation"
    );
    let unique = permutation.iter().copied().collect::<BTreeSet<_>>();
    ensure!(
        unique.len() == CONTROL_LIMIT && unique.iter().copied().eq(0..CONTROL_LIMIT),
        "diffusion result is not a B32 bijection"
    );
    ensure!(
        permutation[..FROZEN_PREFIX_SIZE]
            .iter()
            .copied()
            .eq(0..FROZEN_PREFIX_SIZE),
        "diffusion changed the frozen semantic prefix"
    );
    ensure!(
        permutation
            .iter()
            .enumerate()
            .skip(FROZEN_PREFIX_SIZE)
            .all(|(output, input)| {
                output.abs_diff(*input) <= MAX_MOVEMENT
                    && output.saturating_sub(FROZEN_PREFIX_SIZE) / RERANK_BLOCK_SIZE
                        == input.saturating_sub(FROZEN_PREFIX_SIZE) / RERANK_BLOCK_SIZE
            }),
        "diffusion moved a candidate across block boundaries"
    );
    Ok(())
}

fn cutoff_opportunities(candidates: &[String], groups: &[Vec<String>]) -> u32 {
    let top_ten = &candidates[..OUTPUT_LIMIT];
    let promotable = &candidates[OUTPUT_LIMIT..12];
    u32::try_from(
        groups
            .iter()
            .filter(|group| {
                !group.iter().any(|required| top_ten.contains(required))
                    && group.iter().any(|required| promotable.contains(required))
            })
            .count(),
    )
    .unwrap_or(u32::MAX)
}

fn found_groups(candidates: &[String], groups: &[Vec<String>]) -> Vec<bool> {
    groups
        .iter()
        .map(|group| group.iter().any(|required| candidates.contains(required)))
        .collect()
}

fn aggregate_arms(
    observations: &[CaseObservation],
    fixture: &super::ReplayFixture,
) -> (ArmReport, ArmReport, ArmReport) {
    let mut baseline = ArmAccumulator::default();
    let mut real = ArmAccumulator::default();
    let mut sham = ArmAccumulator::default();
    let groups = fixture.domains.iter().flat_map(|domain| {
        domain
            .cases
            .iter()
            .map(|case| &case.required_document_groups)
    });
    for (observation, required_groups) in observations.iter().zip(groups) {
        baseline.observe(&observation.baseline_candidates, required_groups);
        real.observe(&observation.real_candidates, required_groups);
        sham.observe(&observation.sham_candidates, required_groups);
    }
    (baseline.report(), real.report(), sham.report())
}

#[derive(Debug, Clone, Copy)]
enum ArmKind {
    Baseline,
    Real,
    Sham,
}

fn macro_domain_recall(observations: &[CaseObservation], arm: ArmKind) -> f64 {
    let mut domains = BTreeMap::<usize, (u32, u32)>::new();
    for observation in observations {
        let found = match arm {
            ArmKind::Baseline => &observation.baseline_found,
            ArmKind::Real => &observation.real_found,
            ArmKind::Sham => &observation.sham_found,
        };
        let entry = domains.entry(observation.domain_index).or_default();
        entry.0 = entry.0.saturating_add(
            u32::try_from(found.iter().filter(|value| **value).count()).unwrap_or(u32::MAX),
        );
        entry.1 = entry
            .1
            .saturating_add(u32::try_from(found.len()).unwrap_or(u32::MAX));
    }
    if domains.is_empty() {
        return 0.0;
    }
    domains
        .values()
        .map(|(found, required)| ratio(*found, *required).unwrap_or(0.0))
        .sum::<f64>()
        / domains.len() as f64
}

fn improved_domain_count(observations: &[CaseObservation], domain_count: usize) -> usize {
    (0..domain_count)
        .filter(|domain| {
            let domain_cases = observations
                .iter()
                .filter(|observation| observation.domain_index == *domain);
            let (baseline, real, sham) =
                domain_cases.fold((0_u32, 0_u32, 0_u32), |counts, observation| {
                    (
                        counts
                            .0
                            .saturating_add(count_true(&observation.baseline_found)),
                        counts.1.saturating_add(count_true(&observation.real_found)),
                        counts.2.saturating_add(count_true(&observation.sham_found)),
                    )
                });
            real > baseline && real > sham
        })
        .count()
}

fn language_reports(observations: &[CaseObservation]) -> Vec<LanguageReport> {
    let mut languages = BTreeMap::<String, (u32, u32, u32, u32)>::new();
    for observation in observations {
        let entry = languages.entry(observation.language.clone()).or_default();
        entry.0 = entry
            .0
            .saturating_add(count_true(&observation.baseline_found));
        entry.1 = entry.1.saturating_add(count_true(&observation.real_found));
        entry.2 = entry.2.saturating_add(count_true(&observation.sham_found));
        entry.3 = entry
            .3
            .saturating_add(u32::try_from(observation.baseline_found.len()).unwrap_or(u32::MAX));
    }
    languages
        .into_iter()
        .map(
            |(language, (baseline, real, sham, required))| LanguageReport {
                language,
                baseline_recall_at_ten: ratio(baseline, required),
                real_recall_at_ten: ratio(real, required),
                sham_recall_at_ten: ratio(sham, required),
            },
        )
        .collect()
}

fn healthy_fingerprints(corpus: &ReplayCorpus) -> bool {
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

fn rejection_reasons(input: &GateInput) -> Vec<String> {
    let mut reasons = Vec::new();
    if input.real_macro + EPSILON < input.baseline_macro + MIN_MACRO_RECALL_GAIN {
        reasons.push("real diffusion macro recall gain over B10 is below 0.05".to_owned());
    }
    if input.real_macro + EPSILON < input.sham_macro + MIN_MACRO_RECALL_GAIN {
        reasons.push("real diffusion macro recall gain over sham is below 0.05".to_owned());
    }
    if input.improved_domains < MIN_IMPROVED_DOMAIN_COUNT {
        reasons.push("real diffusion improved in fewer than three domains".to_owned());
    }
    if input.lost_baseline_groups > 0 {
        reasons.push("real diffusion lost a group present in B10".to_owned());
    }
    if input.real.mean_reciprocal_rank_at_ten.unwrap_or(0.0) + EPSILON
        < input.baseline.mean_reciprocal_rank_at_ten.unwrap_or(0.0)
        || input.real.mean_reciprocal_rank_at_ten.unwrap_or(0.0) + EPSILON
            < input.sham.mean_reciprocal_rank_at_ten.unwrap_or(0.0)
    {
        reasons.push("real diffusion MRR@10 regressed against a control".to_owned());
    }
    if input.real.support_density.unwrap_or(0.0) + EPSILON
        < input.baseline.support_density.unwrap_or(0.0)
        || input.real.support_density.unwrap_or(0.0) + EPSILON
            < input.sham.support_density.unwrap_or(0.0)
    {
        reasons.push("real diffusion support density regressed against a control".to_owned());
    }
    if input.cutoff_opportunities < MIN_CUTOFF_OPPORTUNITY_COUNT {
        reasons.push("the frozen corpus exposes fewer than four cutoff opportunities".to_owned());
    }
    if input.language_regression {
        reasons.push("real diffusion regressed in at least one language".to_owned());
    }
    if !input.exact_pool_invariants {
        reasons.push("an exact B32 permutation invariant failed".to_owned());
    }
    if !input.healthy_fingerprint {
        reasons.push("bundle health or fingerprint changed during replay".to_owned());
    }
    let sham_edge_count = input
        .sham_stats
        .rewired_edge_count
        .saturating_add(input.sham_stats.retained_original_edge_count);
    if ratio(input.sham_stats.rewired_edge_count, sham_edge_count).unwrap_or(0.0) + EPSILON
        < MIN_SHAM_REWIRE_RATIO
        || input.sham_stats.unchanged_collection_count > 0
    {
        reasons.push("weak structural sham did not meet its rewiring gate".to_owned());
    }
    if input.retained_graph_pair_bytes > MAX_GRAPH_PAIR_BYTES {
        reasons.push("graph pair exceeded one MiB".to_owned());
    }
    if input.projection_micros > MAX_PROJECTION_MICROS {
        reasons.push("graph projection exceeded one second".to_owned());
    }
    if input.real_rerank_p95_micros > MAX_RERANK_P95_MICROS
        || input.sham_rerank_p95_micros > MAX_RERANK_P95_MICROS
    {
        reasons.push("diffusion reranking p95 exceeded two milliseconds".to_owned());
    }
    reasons
}

fn count_true(values: &[bool]) -> u32 {
    u32::try_from(values.iter().filter(|value| **value).count()).unwrap_or(u32::MAX)
}

fn ratio(numerator: u32, denominator: u32) -> Option<f64> {
    (denominator != 0).then(|| f64::from(numerator) / f64::from(denominator))
}

fn percentile(values: &mut [u128], percentile: usize) -> u128 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let index = values.len().saturating_mul(percentile).saturating_add(99) / 100;
    values[index.saturating_sub(1).min(values.len().saturating_sub(1))]
}

fn write_report(report: &DiffusionReport) -> Result<PathBuf> {
    let directory = workspace_root().join(REPORT_DIRECTORY);
    std::fs::create_dir_all(&directory).context("creating diffusion report directory")?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates Unix epoch")?
        .as_secs();
    let fixture_prefix = report
        .fixture_sha256
        .get(..12)
        .context("diffusion fixture hash is unexpectedly short")?;
    let destination = directory.join(format!(
        "retrieval-okf-diffusion-development-{fixture_prefix}-{}-{}-{epoch}.json",
        report.target_os, report.target_arch
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&destination)
        .with_context(|| format!("creating {}", destination.display()))?;
    let mut bytes = serde_json::to_vec_pretty(report).context("serializing diffusion report")?;
    bytes.push(b'\n');
    file.write_all(&bytes)
        .with_context(|| format!("writing {}", destination.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {}", destination.display()))?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(found: u32, required: u32, mrr: f64) -> ArmReport {
        ArmReport {
            found_group_count: found,
            required_group_count: required,
            recall_at_ten: ratio(found, required),
            mean_reciprocal_rank_at_ten: Some(mrr),
            support_density: Some(f64::from(found) / 100.0),
        }
    }

    fn passing_gate() -> GateInput {
        GateInput {
            baseline: arm(8, 12, 0.4),
            real: arm(10, 12, 0.5),
            sham: arm(8, 12, 0.4),
            baseline_macro: 0.65,
            real_macro: 0.80,
            sham_macro: 0.65,
            improved_domains: 3,
            lost_baseline_groups: 0,
            cutoff_opportunities: 4,
            language_regression: false,
            exact_pool_invariants: true,
            healthy_fingerprint: true,
            sham_stats: StructuralShamStats {
                collection_count: 4,
                retained_original_edge_count: 4,
                rewired_edge_count: 36,
                unchanged_collection_count: 0,
            },
            retained_graph_pair_bytes: MAX_GRAPH_PAIR_BYTES,
            projection_micros: MAX_PROJECTION_MICROS,
            real_rerank_p95_micros: MAX_RERANK_P95_MICROS,
            sham_rerank_p95_micros: MAX_RERANK_P95_MICROS,
        }
    }

    #[test]
    fn versioned_diffusion_fixture_is_valid_and_bilingual() {
        let loaded = load_fixture_from(FIXTURE_PATH, FixtureProfile::Development).unwrap();

        validate_experiment_shape(&loaded).unwrap();
        assert_eq!(loaded.fixture.domains.len(), 4);
        assert_eq!(
            loaded
                .fixture
                .domains
                .iter()
                .map(|domain| domain.cases.len())
                .sum::<usize>(),
            16
        );
    }

    #[test]
    fn versioned_diffusion_fixture_rejects_a_different_hash() {
        let mut loaded = load_fixture_from(FIXTURE_PATH, FixtureProfile::Development).unwrap();
        loaded.sha256 = "0".repeat(64);

        let error = validate_experiment_shape(&loaded).unwrap_err();

        assert!(error.to_string().contains("fixture hash mismatch"));
    }

    #[test]
    fn exact_pool_permutation_accepts_only_block_local_movement() {
        let mut permutation = (0..CONTROL_LIMIT).collect::<Vec<_>>();
        permutation[8..12].rotate_left(2);

        assert!(validate_permutation(&permutation).is_ok());
    }

    #[test]
    fn sham_tiebreak_has_a_frozen_opaque_known_vector() {
        assert_eq!(
            sham_control_label(0, 0).unwrap(),
            "2160a211573634d5a76bd0a678174e87c87b7e938ca58bd1e6dcd122a6c327a7"
        );
        assert_ne!(
            sham_control_label(0, 0).unwrap(),
            sham_control_label(0, 1).unwrap()
        );
        assert_ne!(
            sham_control_label(0, 0).unwrap(),
            sham_control_label(1, 0).unwrap()
        );
    }

    #[test]
    fn exact_pool_permutation_rejects_a_changed_frozen_prefix() {
        let mut permutation = (0..CONTROL_LIMIT).collect::<Vec<_>>();
        permutation.swap(0, 1);

        let error = validate_permutation(&permutation).unwrap_err();

        assert!(error.to_string().contains("frozen semantic prefix"));
    }

    #[test]
    fn exact_pool_permutation_rejects_cross_block_movement() {
        let mut permutation = (0..CONTROL_LIMIT).collect::<Vec<_>>();
        permutation.swap(11, 12);

        let error = validate_permutation(&permutation).unwrap_err();

        assert!(error.to_string().contains("across block boundaries"));
    }

    #[test]
    fn development_gate_accepts_distributed_safe_gain() {
        assert!(rejection_reasons(&passing_gate()).is_empty());
    }

    #[test]
    fn development_gate_rejects_a_baseline_loss() {
        let mut gate = passing_gate();
        gate.lost_baseline_groups = 1;

        assert!(
            rejection_reasons(&gate)
                .iter()
                .any(|reason| reason.contains("lost a group"))
        );
    }
}
