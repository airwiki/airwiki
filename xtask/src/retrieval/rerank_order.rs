use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use airwiki_core::{
    EmbeddingProvider, EvidenceRelevanceError, EvidenceRelevanceProvider, FastEmbedE5Small,
    FastEmbedMmarcoReranker, PinnedE5Snapshot, PinnedMmarcoRerankerSnapshot,
    RetrievalEvaluationCandidate,
};
use airwiki_inference::platform_relevance_model;
use airwiki_types::{DEFAULT_TOP_K, SearchRequest};
use anyhow::{Context, Result, ensure};
use serde::Serialize;

use super::{
    EvaluationPhase, EvaluationProfile, EvaluationProviders, FactIdentity, FixtureCase,
    FixtureCorpus, FixtureNode, NormalizedHit, NormalizedRun, NormalizedSource, ProviderIdentity,
    RetrievalScope, build_corpus, candidate_fingerprint, load_fixture, percentile,
    validate_model_revisions,
};
use crate::{replace_file, workspace_root};

const REPORT_SCHEMA_VERSION: u32 = 1;
const MAX_SCORE_ORDER_P95_MICROS: u128 = 1_000;

type CandidateKey = (uuid::Uuid, uuid::Uuid, uuid::Uuid, u32);

#[derive(Debug)]
struct PairCaseReport {
    id: String,
    domain: String,
    no_answer: bool,
    candidate_count: usize,
    relevance_call_count: u32,
    candidate_preparation_micros: u128,
    shared_relevance_micros: u128,
    score_order_micros: u128,
    full_case_micros: u128,
    invalidated: bool,
    provider_failure: bool,
    improved_answerable: bool,
    lost_group_count: u32,
    filter_order: ArmCaseEvaluation,
    score_order: ArmCaseEvaluation,
}

#[derive(Debug)]
struct ArmCaseEvaluation {
    expected_group_count: u32,
    found_group_count: u32,
    reciprocal_rank_at_five: Option<f64>,
    returned_count: u32,
    false_evidence_count: u32,
    forbidden_evidence_count: u32,
    provenance_error_count: u32,
    duplicate_violation_count: u32,
    provider_failure_count: u32,
    found_group_indexes: BTreeSet<usize>,
}

#[derive(Debug, Serialize)]
struct PairCaseSummary {
    id: String,
    domain: String,
    no_answer: bool,
    candidate_count: usize,
    relevance_call_count: u32,
    candidate_preparation_micros: u128,
    shared_relevance_micros: u128,
    score_order_micros: u128,
    full_case_micros: u128,
    invalidated: bool,
    provider_failure: bool,
    improved_answerable: bool,
    lost_group_count: u32,
    filter_order: ArmCaseSummary,
    score_order: ArmCaseSummary,
}

impl From<&PairCaseReport> for PairCaseSummary {
    fn from(report: &PairCaseReport) -> Self {
        Self {
            id: report.id.clone(),
            domain: report.domain.clone(),
            no_answer: report.no_answer,
            candidate_count: report.candidate_count,
            relevance_call_count: report.relevance_call_count,
            candidate_preparation_micros: report.candidate_preparation_micros,
            shared_relevance_micros: report.shared_relevance_micros,
            score_order_micros: report.score_order_micros,
            full_case_micros: report.full_case_micros,
            invalidated: report.invalidated,
            provider_failure: report.provider_failure,
            improved_answerable: report.improved_answerable,
            lost_group_count: report.lost_group_count,
            filter_order: ArmCaseSummary::from(&report.filter_order),
            score_order: ArmCaseSummary::from(&report.score_order),
        }
    }
}

#[derive(Debug, Serialize)]
struct ArmCaseSummary {
    expected_group_count: u32,
    found_group_count: u32,
    reciprocal_rank_at_five: Option<f64>,
    returned_count: u32,
    false_evidence_count: u32,
    forbidden_evidence_count: u32,
    provenance_error_count: u32,
    duplicate_violation_count: u32,
    provider_failure_count: u32,
}

impl From<&ArmCaseEvaluation> for ArmCaseSummary {
    fn from(report: &ArmCaseEvaluation) -> Self {
        Self {
            expected_group_count: report.expected_group_count,
            found_group_count: report.found_group_count,
            reciprocal_rank_at_five: report.reciprocal_rank_at_five,
            returned_count: report.returned_count,
            false_evidence_count: report.false_evidence_count,
            forbidden_evidence_count: report.forbidden_evidence_count,
            provenance_error_count: report.provenance_error_count,
            duplicate_violation_count: report.duplicate_violation_count,
            provider_failure_count: report.provider_failure_count,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct QualitySummary {
    case_count: u32,
    expected_group_count: u32,
    found_group_count: u32,
    recall_at_five: Option<f64>,
    mean_reciprocal_rank_at_five: Option<f64>,
    macro_domain_recall_at_five: Option<f64>,
    citation_precision: Option<f64>,
    no_answer_accuracy: Option<f64>,
    false_evidence_count: u32,
    forbidden_evidence_count: u32,
    provenance_error_count: u32,
    duplicate_violation_count: u32,
    provider_failure_count: u32,
    partial_case_count: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
struct LatencyReport {
    count: usize,
    p50_micros: Option<u128>,
    p95_micros: Option<u128>,
    max_micros: Option<u128>,
}

impl LatencyReport {
    fn from_values(mut values: Vec<u128>) -> Self {
        values.sort_unstable();
        Self {
            count: values.len(),
            p50_micros: percentile(&values, 50),
            p95_micros: percentile(&values, 95),
            max_micros: values.last().copied(),
        }
    }
}

#[derive(Debug, Serialize)]
struct EvaluationReport {
    schema_version: u32,
    evaluation_role: &'static str,
    fixture_sha256: String,
    candidate_fingerprint: String,
    target_os: String,
    target_arch: String,
    provider: ProviderIdentity,
    included_case_count: usize,
    excluded_case_count: usize,
    excluded_scope_counts: BTreeMap<&'static str, u32>,
    total_candidate_count: usize,
    relevance_call_count: u32,
    provider_failure_count: u32,
    improved_answerable_case_count: u32,
    lost_group_count: u32,
    filter_order: QualitySummary,
    score_order: QualitySummary,
    candidate_preparation_latency: LatencyReport,
    shared_relevance_latency: LatencyReport,
    score_order_latency: LatencyReport,
    full_case_latency: LatencyReport,
    diagnostic_passed: bool,
    production_promotion_ready: bool,
    cases: Vec<PairCaseSummary>,
}

pub(crate) async fn evaluate_rerank_order(
    embedding_snapshot: &Path,
    relevance_snapshot: &Path,
) -> Result<()> {
    validate_model_revisions()?;
    let loaded = load_fixture()?;
    let included = loaded
        .fixture
        .cases
        .iter()
        .filter(|case| EvaluationPhase::Development.includes(case.split))
        .filter(|case| is_local_scope(case.scope))
        .collect::<Vec<_>>();
    ensure!(
        !included.is_empty(),
        "rerank-order diagnostic has no eligible local development cases"
    );
    let included_domains = included
        .iter()
        .filter_map(|case| case.domain.as_deref())
        .collect::<BTreeSet<_>>();
    let excluded_scope_counts = excluded_scope_counts(&loaded.fixture.cases);
    let excluded_case_count = excluded_scope_counts
        .values()
        .copied()
        .map(|count| count as usize)
        .sum();

    let providers = real_providers(embedding_snapshot, relevance_snapshot)?;
    let corpus = build_corpus(&loaded.fixture, &providers, false, Some(&included_domains)).await?;
    let facts = fact_index(&corpus);

    let mut reports = Vec::with_capacity(included.len());
    let mut candidate_preparation_micros = Vec::with_capacity(included.len());
    let mut shared_relevance_micros = Vec::with_capacity(included.len());
    let mut score_order_micros = Vec::with_capacity(included.len());
    let mut full_case_micros = Vec::with_capacity(included.len());
    let mut total_candidate_count = 0_usize;
    let mut relevance_call_count = 0_u32;
    let mut provider_failure_count = 0_u32;

    for case in included {
        let started = Instant::now();
        let request = SearchRequest::new(&case.question, case.scope.purpose(), DEFAULT_TOP_K);
        let comparison = match corpus
            .origin
            .compare_local_score_order_for_evaluation(request)
            .await
        {
            Ok(comparison) => comparison,
            Err(error) if error.downcast_ref::<EvidenceRelevanceError>().is_some() => {
                provider_failure_count = provider_failure_count.saturating_add(1);
                relevance_call_count = relevance_call_count.saturating_add(1);
                let elapsed = started.elapsed().as_micros();
                full_case_micros.push(elapsed);
                reports.push(failed_pair(case, elapsed));
                continue;
            }
            Err(error) => return Err(error),
        };
        let elapsed = started.elapsed().as_micros();
        let filter_run = normalize_selection(&comparison.filter_order.candidates, &facts);
        let score_run = normalize_selection(&comparison.score_order.candidates, &facts);
        let filter_report = score_normalized(case, filter_run);
        let score_report = score_normalized(case, score_run);
        let improved_answerable = !case.expected_groups.is_empty()
            && score_report.found_group_count > filter_report.found_group_count;
        let lost_group_count = filter_report
            .found_group_indexes
            .difference(&score_report.found_group_indexes)
            .count()
            .try_into()
            .unwrap_or(u32::MAX);

        total_candidate_count = total_candidate_count.saturating_add(comparison.candidate_count);
        relevance_call_count = relevance_call_count.saturating_add(comparison.relevance_call_count);
        candidate_preparation_micros.push(comparison.filter_order.candidate_preparation_micros);
        shared_relevance_micros.push(comparison.shared_relevance_micros);
        score_order_micros.push(comparison.score_order_micros);
        full_case_micros.push(elapsed);
        reports.push(PairCaseReport {
            id: case.id.clone(),
            domain: case.domain.clone().unwrap_or_else(|| "unknown".to_owned()),
            no_answer: case.expected_groups.is_empty(),
            candidate_count: comparison.candidate_count,
            relevance_call_count: comparison.relevance_call_count,
            candidate_preparation_micros: comparison.filter_order.candidate_preparation_micros,
            shared_relevance_micros: comparison.shared_relevance_micros,
            score_order_micros: comparison.score_order_micros,
            full_case_micros: elapsed,
            invalidated: comparison.invalidated,
            provider_failure: false,
            improved_answerable,
            lost_group_count,
            filter_order: filter_report,
            score_order: score_report,
        });
    }

    let filter_order = summarize(&reports, |report| &report.filter_order);
    let score_order = summarize(&reports, |report| &report.score_order);
    let improved_answerable_case_count = reports
        .iter()
        .filter(|report| report.improved_answerable)
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let lost_group_count = reports
        .iter()
        .map(|report| report.lost_group_count)
        .fold(0_u32, u32::saturating_add);
    let score_order_latency = LatencyReport::from_values(score_order_micros);
    let diagnostic_passed = diagnostic_passes(
        &filter_order,
        &score_order,
        improved_answerable_case_count,
        lost_group_count,
        provider_failure_count,
        score_order_latency.p95_micros,
    );
    let report = EvaluationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        evaluation_role: "visible-development-mechanism-diagnostic",
        fixture_sha256: loaded.sha256,
        candidate_fingerprint: candidate_fingerprint(&providers.identity),
        target_os: std::env::consts::OS.to_owned(),
        target_arch: std::env::consts::ARCH.to_owned(),
        provider: providers.identity,
        included_case_count: reports.len(),
        excluded_case_count,
        excluded_scope_counts,
        total_candidate_count,
        relevance_call_count,
        provider_failure_count,
        improved_answerable_case_count,
        lost_group_count,
        filter_order,
        score_order,
        candidate_preparation_latency: LatencyReport::from_values(candidate_preparation_micros),
        shared_relevance_latency: LatencyReport::from_values(shared_relevance_micros),
        score_order_latency,
        full_case_latency: LatencyReport::from_values(full_case_micros),
        diagnostic_passed,
        production_promotion_ready: false,
        cases: reports.iter().map(PairCaseSummary::from).collect(),
    };
    let destination = write_report(&report)?;
    ensure!(
        report.diagnostic_passed,
        "mMARCO score-order hypothesis did not pass the visible development diagnostic; report written to {}",
        destination.display()
    );
    println!(
        "mMARCO score-order hypothesis passed the visible development diagnostic; report written to {}",
        destination.display()
    );
    Ok(())
}

fn real_providers(
    embedding_snapshot: &Path,
    relevance_snapshot: &Path,
) -> Result<EvaluationProviders> {
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
    let artifact =
        platform_relevance_model().context("unsupported rerank-order evaluation target")?;
    Ok(EvaluationProviders {
        profile: EvaluationProfile::Current,
        identity: ProviderIdentity {
            embedding_profile: embeddings.model_id().to_owned(),
            embedding_revision: airwiki_core::E5_MODEL_REVISION.to_owned(),
            relevance_profile: relevance.profile_id().to_owned(),
            relevance_revision: airwiki_core::MMARCO_RERANKER_REVISION.to_owned(),
            relevance_artifact_filename: Some(artifact.filename.to_owned()),
            relevance_artifact_sha256: Some(artifact.sha256.to_owned()),
            thread_count: threads,
        },
        embeddings,
        relevance,
        telemetry: None,
        startup_ms: None,
    })
}

fn is_local_scope(scope: RetrievalScope) -> bool {
    matches!(
        scope,
        RetrievalScope::Local | RetrievalScope::LocalExternalAi
    )
}

fn excluded_scope_counts(cases: &[FixtureCase]) -> BTreeMap<&'static str, u32> {
    let mut counts = BTreeMap::new();
    for case in cases
        .iter()
        .filter(|case| EvaluationPhase::Development.includes(case.split))
        .filter(|case| !is_local_scope(case.scope))
    {
        let label = match case.scope {
            RetrievalScope::TrustedPeer => "trusted_peer",
            RetrievalScope::TrustedPeerExternalAi => "trusted_peer_external_ai",
            RetrievalScope::Federated => "federated",
            RetrievalScope::Local | RetrievalScope::LocalExternalAi => continue,
        };
        counts
            .entry(label)
            .and_modify(|count: &mut u32| *count = count.saturating_add(1))
            .or_insert(1);
    }
    counts
}

fn fact_index(corpus: &FixtureCorpus) -> HashMap<CandidateKey, &FactIdentity> {
    corpus
        .facts_by_provenance
        .values()
        .filter(|fact| fact.node == FixtureNode::Origin)
        .map(|fact| {
            (
                (fact.collection_id, fact.concept_id, fact.chunk_id, 1),
                fact,
            )
        })
        .collect()
}

fn normalize_selection(
    candidates: &[RetrievalEvaluationCandidate],
    facts: &HashMap<CandidateKey, &FactIdentity>,
) -> NormalizedRun {
    let mut hits = Vec::with_capacity(candidates.len());
    let mut provenance_errors = 0_u32;
    for (index, candidate) in candidates.iter().enumerate() {
        let key = (
            candidate.collection_id,
            candidate.concept_id,
            candidate.chunk_id,
            candidate.source_revision,
        );
        let Some(fact) = facts.get(&key) else {
            provenance_errors = provenance_errors.saturating_add(1);
            continue;
        };
        hits.push(NormalizedHit {
            fact_id: fact.id.clone(),
            rank: u32::try_from(index + 1).unwrap_or(u32::MAX),
        });
    }
    NormalizedRun {
        sources: vec![NormalizedSource {
            node: FixtureNode::Origin,
            hits,
        }],
        provenance_errors,
    }
}

fn score_normalized(case: &FixtureCase, run: NormalizedRun) -> ArmCaseEvaluation {
    let returned = run
        .sources
        .iter()
        .flat_map(|source| source.hits.iter())
        .collect::<Vec<_>>();
    let returned_ids = returned
        .iter()
        .map(|hit| hit.fact_id.as_str())
        .collect::<Vec<_>>();
    let relevant = case
        .relevant_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let forbidden = case
        .forbidden_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = case
        .expected_groups
        .iter()
        .flatten()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let found_group_indexes = case
        .expected_groups
        .iter()
        .enumerate()
        .filter(|(_, group)| {
            group
                .iter()
                .any(|fact_id| returned_ids.contains(&fact_id.as_str()))
        })
        .map(|(index, _)| index)
        .collect::<BTreeSet<_>>();
    let duplicate_violation_count = case
        .expected_groups
        .iter()
        .filter(|group| group.len() > 1)
        .filter(|group| {
            group
                .iter()
                .filter(|fact_id| returned_ids.contains(&fact_id.as_str()))
                .count()
                > 1
        })
        .count();
    let false_evidence_count = returned_ids
        .iter()
        .copied()
        .filter(|fact_id| !relevant.contains(fact_id))
        .collect::<BTreeSet<_>>()
        .len();
    let forbidden_evidence_count = returned_ids
        .iter()
        .copied()
        .filter(|fact_id| forbidden.contains(fact_id))
        .collect::<BTreeSet<_>>()
        .len();
    let reciprocal_rank_at_five = returned
        .iter()
        .filter(|hit| expected.contains(hit.fact_id.as_str()))
        .map(|hit| hit.rank)
        .min()
        .map(|rank| 1.0 / f64::from(rank));
    ArmCaseEvaluation {
        expected_group_count: u32::try_from(case.expected_groups.len()).unwrap_or(u32::MAX),
        found_group_count: u32::try_from(found_group_indexes.len()).unwrap_or(u32::MAX),
        reciprocal_rank_at_five,
        returned_count: u32::try_from(returned.len()).unwrap_or(u32::MAX),
        false_evidence_count: u32::try_from(false_evidence_count).unwrap_or(u32::MAX),
        forbidden_evidence_count: u32::try_from(forbidden_evidence_count).unwrap_or(u32::MAX),
        provenance_error_count: run.provenance_errors,
        duplicate_violation_count: u32::try_from(duplicate_violation_count).unwrap_or(u32::MAX),
        provider_failure_count: 0,
        found_group_indexes,
    }
}

fn failed_pair(case: &FixtureCase, elapsed: u128) -> PairCaseReport {
    let failed_arm = || ArmCaseEvaluation {
        expected_group_count: u32::try_from(case.expected_groups.len()).unwrap_or(u32::MAX),
        found_group_count: 0,
        reciprocal_rank_at_five: None,
        returned_count: 0,
        false_evidence_count: 0,
        forbidden_evidence_count: 0,
        provenance_error_count: 0,
        duplicate_violation_count: 0,
        provider_failure_count: 1,
        found_group_indexes: BTreeSet::new(),
    };
    PairCaseReport {
        id: case.id.clone(),
        domain: case.domain.clone().unwrap_or_else(|| "unknown".to_owned()),
        no_answer: case.expected_groups.is_empty(),
        candidate_count: 0,
        relevance_call_count: 1,
        candidate_preparation_micros: 0,
        shared_relevance_micros: 0,
        score_order_micros: 0,
        full_case_micros: elapsed,
        invalidated: false,
        provider_failure: true,
        improved_answerable: false,
        lost_group_count: 0,
        filter_order: failed_arm(),
        score_order: failed_arm(),
    }
}

fn summarize(
    reports: &[PairCaseReport],
    arm: impl Fn(&PairCaseReport) -> &ArmCaseEvaluation,
) -> QualitySummary {
    let mut case_count = 0_u32;
    let mut expected_group_count = 0_u32;
    let mut found_group_count = 0_u32;
    let mut reciprocal_rank_sum = 0.0_f64;
    let mut reciprocal_rank_count = 0_u32;
    let mut false_evidence_count = 0_u32;
    let mut forbidden_evidence_count = 0_u32;
    let mut provenance_error_count = 0_u32;
    let mut duplicate_violation_count = 0_u32;
    let mut provider_failure_count = 0_u32;
    for report in reports {
        let arm = arm(report);
        case_count = case_count.saturating_add(1);
        expected_group_count = expected_group_count.saturating_add(arm.expected_group_count);
        found_group_count = found_group_count.saturating_add(arm.found_group_count);
        false_evidence_count = false_evidence_count.saturating_add(arm.false_evidence_count);
        forbidden_evidence_count =
            forbidden_evidence_count.saturating_add(arm.forbidden_evidence_count);
        provenance_error_count = provenance_error_count.saturating_add(arm.provenance_error_count);
        duplicate_violation_count =
            duplicate_violation_count.saturating_add(arm.duplicate_violation_count);
        provider_failure_count = provider_failure_count.saturating_add(arm.provider_failure_count);
        if arm.expected_group_count > 0 {
            reciprocal_rank_sum += arm.reciprocal_rank_at_five.unwrap_or(0.0);
            reciprocal_rank_count = reciprocal_rank_count.saturating_add(1);
        }
    }
    let recall_at_five = (expected_group_count > 0)
        .then(|| f64::from(found_group_count) / f64::from(expected_group_count));
    let mean_reciprocal_rank_at_five =
        (reciprocal_rank_count > 0).then(|| reciprocal_rank_sum / f64::from(reciprocal_rank_count));
    let returned_count = reports
        .iter()
        .map(|report| arm(report).returned_count)
        .fold(0_u32, u32::saturating_add);
    let citation_precision = (returned_count > 0).then(|| {
        f64::from(returned_count.saturating_sub(false_evidence_count)) / f64::from(returned_count)
    });
    let no_answer_cases = reports.iter().filter(|report| report.no_answer).count();
    let correct_no_answer = reports
        .iter()
        .filter(|report| report.no_answer && arm(report).returned_count == 0)
        .count();
    let no_answer_accuracy =
        (no_answer_cases > 0).then(|| correct_no_answer as f64 / no_answer_cases as f64);
    let mut domains = BTreeMap::<&str, (u32, u32)>::new();
    for report in reports {
        let arm = arm(report);
        if arm.expected_group_count == 0 {
            continue;
        }
        let totals = domains.entry(report.domain.as_str()).or_default();
        totals.0 = totals.0.saturating_add(arm.found_group_count);
        totals.1 = totals.1.saturating_add(arm.expected_group_count);
    }
    let macro_domain_recall_at_five = (!domains.is_empty()).then(|| {
        domains
            .values()
            .map(|(found, expected)| f64::from(*found) / f64::from(*expected))
            .sum::<f64>()
            / domains.len() as f64
    });
    let partial_case_count = reports
        .iter()
        .filter(|report| report.invalidated)
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    QualitySummary {
        case_count,
        expected_group_count,
        found_group_count,
        recall_at_five,
        mean_reciprocal_rank_at_five,
        macro_domain_recall_at_five,
        citation_precision,
        no_answer_accuracy,
        false_evidence_count,
        forbidden_evidence_count,
        provenance_error_count,
        duplicate_violation_count,
        provider_failure_count,
        partial_case_count,
    }
}

fn diagnostic_passes(
    filter: &QualitySummary,
    score: &QualitySummary,
    improved_answerable_case_count: u32,
    lost_group_count: u32,
    provider_failure_count: u32,
    score_order_p95_micros: Option<u128>,
) -> bool {
    improved_answerable_case_count > 0
        && lost_group_count == 0
        && option_not_lower(
            score.mean_reciprocal_rank_at_five,
            filter.mean_reciprocal_rank_at_five,
        )
        && score.false_evidence_count <= filter.false_evidence_count
        && filter.forbidden_evidence_count == 0
        && score.forbidden_evidence_count == 0
        && filter.duplicate_violation_count == 0
        && score.duplicate_violation_count == 0
        && score.no_answer_accuracy == Some(1.0)
        && filter.provenance_error_count == 0
        && score.provenance_error_count == 0
        && filter.provider_failure_count == 0
        && score.provider_failure_count == 0
        && score.partial_case_count == 0
        && provider_failure_count == 0
        && score_order_p95_micros.is_some_and(|micros| micros < MAX_SCORE_ORDER_P95_MICROS)
}

fn option_not_lower(candidate: Option<f64>, baseline: Option<f64>) -> bool {
    match (candidate, baseline) {
        (Some(candidate), Some(baseline)) => candidate + f64::EPSILON >= baseline,
        (None, None) => true,
        _ => false,
    }
}

fn write_report(report: &EvaluationReport) -> Result<PathBuf> {
    let destination = workspace_root().join("target").join("evals").join(format!(
        "retrieval-rerank-order-development-{}-{}.json",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    let parent = destination
        .parent()
        .context("rerank-order report has no parent")?;
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

    fn safe_quality() -> QualitySummary {
        QualitySummary {
            case_count: 1,
            expected_group_count: 1,
            found_group_count: 1,
            recall_at_five: Some(1.0),
            mean_reciprocal_rank_at_five: Some(0.5),
            macro_domain_recall_at_five: Some(0.5),
            citation_precision: Some(1.0),
            no_answer_accuracy: Some(1.0),
            false_evidence_count: 0,
            forbidden_evidence_count: 0,
            provenance_error_count: 0,
            duplicate_violation_count: 0,
            provider_failure_count: 0,
            partial_case_count: 0,
        }
    }

    #[test]
    fn diagnostic_gate_accepts_a_strict_safe_improvement() {
        assert!(diagnostic_passes(
            &safe_quality(),
            &safe_quality(),
            1,
            0,
            0,
            Some(10),
        ));
    }

    #[test]
    fn diagnostic_gate_rejects_an_order_without_improvement() {
        assert!(!diagnostic_passes(
            &safe_quality(),
            &safe_quality(),
            0,
            0,
            0,
            Some(10),
        ));
    }

    #[test]
    fn diagnostic_gate_rejects_forbidden_or_duplicate_evidence_in_either_arm() {
        let mut forbidden_filter = safe_quality();
        forbidden_filter.forbidden_evidence_count = 1;
        assert!(!diagnostic_passes(
            &forbidden_filter,
            &safe_quality(),
            1,
            0,
            0,
            Some(10),
        ));

        let mut duplicate_score = safe_quality();
        duplicate_score.duplicate_violation_count = 1;
        assert!(!diagnostic_passes(
            &safe_quality(),
            &duplicate_score,
            1,
            0,
            0,
            Some(10),
        ));
    }

    #[test]
    fn serialized_case_summary_contains_metrics_not_fact_id_fields() {
        let report = PairCaseReport {
            id: "synthetic-case".to_owned(),
            domain: "synthetic-domain".to_owned(),
            no_answer: false,
            candidate_count: 1,
            relevance_call_count: 1,
            candidate_preparation_micros: 1,
            shared_relevance_micros: 2,
            score_order_micros: 3,
            full_case_micros: 4,
            invalidated: false,
            provider_failure: false,
            improved_answerable: true,
            lost_group_count: 0,
            filter_order: safe_arm(),
            score_order: safe_arm(),
        };

        let serialized = serde_json::to_string(&PairCaseSummary::from(&report)).unwrap();

        assert!(!serialized.contains("returned_fact_ids"));
        assert!(!serialized.contains("unexpected_fact_ids"));
        assert!(!serialized.contains("forbidden_fact_ids"));
        assert!(!serialized.contains("question"));
        assert!(!serialized.contains("snippet"));
    }

    #[test]
    fn failed_provider_attempt_counts_the_single_relevance_call() {
        let loaded = load_fixture().unwrap();
        let case = loaded.fixture.cases.first().unwrap();

        let report = failed_pair(case, 1);

        assert_eq!(report.relevance_call_count, 1);
        assert_eq!(report.filter_order.provider_failure_count, 1);
        assert_eq!(report.score_order.provider_failure_count, 1);
    }

    fn safe_arm() -> ArmCaseEvaluation {
        ArmCaseEvaluation {
            expected_group_count: 1,
            found_group_count: 1,
            reciprocal_rank_at_five: Some(1.0),
            returned_count: 1,
            false_evidence_count: 0,
            forbidden_evidence_count: 0,
            provenance_error_count: 0,
            duplicate_violation_count: 0,
            provider_failure_count: 0,
            found_group_indexes: BTreeSet::from([0]),
        }
    }
}
