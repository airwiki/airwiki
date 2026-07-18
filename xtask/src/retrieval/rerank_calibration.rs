//! Development-only calibration of the pinned mMARCO evidence cutoff.
//!
//! The evaluator consumes a hash-verified local corpus, keeps logits in memory
//! and writes only content-free aggregates. It does not alter production search.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use airwiki_core::{
    EvidenceDecision, EvidenceRelevanceProvider, FastEmbedMmarcoReranker,
    MMARCO_RERANKER_PROFILE_ID, MMARCO_RERANKER_REVISION, PinnedMmarcoRerankerSnapshot,
    RelevanceInput,
};
use airwiki_inference::platform_relevance_model;
use anyhow::{Context, Result, ensure};
use serde::Serialize;

use super::{
    corpus::{
        self, CandidateRole, CorpusExpectation, CorpusLanguage, CorpusSplit, LoadedSelection,
        NeedKind,
    },
    percentile, validate_model_revisions,
};
use crate::{replace_file, workspace_root};

const REPORT_SCHEMA_VERSION: u32 = 1;
const EXPECTED_CASE_COUNT: usize = 48;
const EXPECTED_CASES_PER_SPLIT: usize = 24;
const EXPECTED_CANDIDATES_PER_CASE: usize = 10;
const TOP_K: usize = 5;
const CURRENT_RELATIVE_WINDOW: f32 = 3.6;
const MIN_COMPLETE_COVERAGE_GAIN: f64 = 0.05;
const MIN_IMPROVED_STRATA: usize = 2;
const MAX_DECISION_P95_MICROS: u128 = 1_000;

#[derive(Clone)]
struct ScoredCase {
    id: String,
    split: CorpusSplit,
    source_id: String,
    query_language: CorpusLanguage,
    candidate_language: CorpusLanguage,
    expectation: CorpusExpectation,
    roles: Vec<CandidateRole>,
    production_decisions: Vec<EvidenceDecision>,
    score_order: Vec<usize>,
    scores: Vec<f32>,
    inference_micros: u128,
}

#[derive(Debug, Clone, Default)]
struct ArmCaseEvaluation {
    returned_count: u32,
    found_support_count: u32,
    required_support_count: u32,
    hard_negative_count: u32,
    complete: bool,
    no_answer_correct: bool,
    reciprocal_rank_at_five: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct ArmCaseSummary {
    returned_count: u32,
    found_support_count: u32,
    required_support_count: u32,
    hard_negative_count: u32,
    complete: bool,
    no_answer_correct: bool,
    reciprocal_rank_at_five: Option<f64>,
}

impl From<&ArmCaseEvaluation> for ArmCaseSummary {
    fn from(value: &ArmCaseEvaluation) -> Self {
        Self {
            returned_count: value.returned_count,
            found_support_count: value.found_support_count,
            required_support_count: value.required_support_count,
            hard_negative_count: value.hard_negative_count,
            complete: value.complete,
            no_answer_correct: value.no_answer_correct,
            reciprocal_rank_at_five: value.reciprocal_rank_at_five,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct QualitySummary {
    case_count: u32,
    support_present_count: u32,
    support_absent_count: u32,
    complete_count: u32,
    complete_coverage: Option<f64>,
    support_recall_at_five: Option<f64>,
    mean_reciprocal_rank_at_five: Option<f64>,
    no_answer_accuracy: Option<f64>,
    returned_count: u32,
    hard_negative_count: u32,
    citation_precision: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct LatencySummary {
    count: usize,
    p50_micros: Option<u128>,
    p95_micros: Option<u128>,
    max_micros: Option<u128>,
}

impl LatencySummary {
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
struct CaseSummary {
    id: String,
    source: String,
    language_direction: String,
    support_present: bool,
    baseline: ArmCaseSummary,
    threshold_input_order: ArmCaseSummary,
    threshold_score_order: ArmCaseSummary,
}

#[derive(Debug, Serialize)]
struct CalibrationReport {
    schema_version: u32,
    evaluation_role: &'static str,
    corpus_id: String,
    manifest_sha256: String,
    target_os: String,
    target_arch: String,
    relevance_profile: &'static str,
    relevance_revision: &'static str,
    relevance_artifact_filename: String,
    relevance_artifact_sha256: String,
    relevance_call_count: u32,
    total_candidate_count: usize,
    selected_absolute_logit_cutoff: f32,
    relative_window: f32,
    training_baseline: QualitySummary,
    training_threshold_score_order: QualitySummary,
    calibration_baseline: QualitySummary,
    calibration_threshold_input_order: QualitySummary,
    calibration_threshold_score_order: QualitySummary,
    calibration_complete_coverage_gain: Option<f64>,
    calibration_lost_baseline_complete_count: u32,
    improved_strata: Vec<String>,
    inference_latency: LatencySummary,
    decision_latency: LatencySummary,
    rejection_reasons: Vec<String>,
    development_gate_passed: bool,
    sealed_holdout_authorized: bool,
    production_promotion_ready: bool,
    calibration_cases: Vec<CaseSummary>,
}

pub(crate) async fn evaluate_rerank_calibration(
    source_root: &Path,
    relevance_snapshot: &Path,
) -> Result<()> {
    validate_model_revisions()?;
    let manifest_path = workspace_root().join(super::RERANK_CALIBRATION_CORPUS_MANIFEST_PATH);
    let manifest_summary = corpus::validate_manifest(&manifest_path)?;
    ensure!(
        manifest_summary.selection_count == EXPECTED_CASE_COUNT,
        "rerank calibration corpus must contain exactly {EXPECTED_CASE_COUNT} cases"
    );
    ensure!(
        manifest_summary.group_count == 40,
        "rerank calibration corpus must contain exactly 40 document families"
    );
    let corpus = corpus::load_verified_corpus(&manifest_path, source_root)?;
    validate_loaded_corpus(&corpus.selections)?;

    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 4);
    let provider = FastEmbedMmarcoReranker::from_snapshot(
        PinnedMmarcoRerankerSnapshot::open(relevance_snapshot)?,
        threads,
    )?;
    let mut cases = Vec::with_capacity(corpus.selections.len());
    for selection in &corpus.selections {
        cases.push(score_case(&provider, selection).await?);
    }

    let training = cases
        .iter()
        .filter(|case| case.split == CorpusSplit::Training)
        .collect::<Vec<_>>();
    let calibration = cases
        .iter()
        .filter(|case| case.split == CorpusSplit::Calibration)
        .collect::<Vec<_>>();
    ensure!(
        training.len() == EXPECTED_CASES_PER_SPLIT && calibration.len() == EXPECTED_CASES_PER_SPLIT,
        "rerank calibration corpus must contain 24 training and 24 calibration cases"
    );

    let cutoff = select_cutoff(&training)?;
    let training_baseline = summarize(&training, baseline_evaluation);
    let training_threshold_score_order =
        summarize(&training, |case| threshold_evaluation(case, cutoff, true));

    let mut decision_micros = Vec::with_capacity(calibration.len());
    let calibration_rows = calibration
        .iter()
        .map(|case| {
            let case = *case;
            let baseline = baseline_evaluation(case);
            let started = Instant::now();
            let threshold_input_order = threshold_evaluation(case, cutoff, false);
            let threshold_score_order = threshold_evaluation(case, cutoff, true);
            decision_micros.push(started.elapsed().as_micros());
            (case, baseline, threshold_input_order, threshold_score_order)
        })
        .collect::<Vec<_>>();

    let calibration_baseline = summarize_rows(&calibration_rows, 1);
    let calibration_threshold_input_order = summarize_rows(&calibration_rows, 2);
    let calibration_threshold_score_order = summarize_rows(&calibration_rows, 3);
    let complete_coverage_gain = difference(
        calibration_threshold_score_order.complete_coverage,
        calibration_baseline.complete_coverage,
    );
    let lost_baseline_complete_count = calibration_rows
        .iter()
        .filter(|row| row.1.complete && !row.3.complete)
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    let improved_strata = improved_strata(&calibration_rows);
    let decision_latency = LatencySummary::from_values(decision_micros);
    let rejection_reasons = rejection_reasons(
        &calibration_baseline,
        &calibration_threshold_score_order,
        complete_coverage_gain,
        lost_baseline_complete_count,
        &improved_strata,
        decision_latency.p95_micros,
    );
    let development_gate_passed = rejection_reasons.is_empty();
    let artifact =
        platform_relevance_model().context("unsupported rerank calibration evaluation target")?;
    let report = CalibrationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        evaluation_role: "grouped-development-calibration",
        corpus_id: corpus.corpus_id,
        manifest_sha256: corpus.manifest_sha256,
        target_os: std::env::consts::OS.to_owned(),
        target_arch: std::env::consts::ARCH.to_owned(),
        relevance_profile: MMARCO_RERANKER_PROFILE_ID,
        relevance_revision: MMARCO_RERANKER_REVISION,
        relevance_artifact_filename: artifact.filename.to_owned(),
        relevance_artifact_sha256: artifact.sha256.to_owned(),
        relevance_call_count: u32::try_from(cases.len()).unwrap_or(u32::MAX),
        total_candidate_count: cases.iter().map(|case| case.scores.len()).sum(),
        selected_absolute_logit_cutoff: cutoff,
        relative_window: CURRENT_RELATIVE_WINDOW,
        training_baseline,
        training_threshold_score_order,
        calibration_baseline,
        calibration_threshold_input_order,
        calibration_threshold_score_order,
        calibration_complete_coverage_gain: complete_coverage_gain,
        calibration_lost_baseline_complete_count: lost_baseline_complete_count,
        improved_strata,
        inference_latency: LatencySummary::from_values(
            cases.iter().map(|case| case.inference_micros).collect(),
        ),
        decision_latency,
        rejection_reasons,
        development_gate_passed,
        sealed_holdout_authorized: development_gate_passed,
        production_promotion_ready: false,
        calibration_cases: calibration_rows
            .into_iter()
            .map(
                |(case, baseline, threshold_input_order, threshold_score_order)| CaseSummary {
                    id: case.id.clone(),
                    source: case.source_id.clone(),
                    language_direction: language_direction(case),
                    support_present: case.expectation == CorpusExpectation::Answerable,
                    baseline: ArmCaseSummary::from(&baseline),
                    threshold_input_order: ArmCaseSummary::from(&threshold_input_order),
                    threshold_score_order: ArmCaseSummary::from(&threshold_score_order),
                },
            )
            .collect(),
    };
    let destination = write_report(&report)?;
    ensure!(
        report.development_gate_passed,
        "mMARCO cutoff calibration did not pass its grouped development gate; report written to {}",
        destination.display()
    );
    println!(
        "mMARCO cutoff calibration passed its grouped development gate; report written to {} (production remains unchanged)",
        destination.display()
    );
    Ok(())
}

fn validate_loaded_corpus(selections: &[LoadedSelection]) -> Result<()> {
    ensure!(
        selections.len() == EXPECTED_CASE_COUNT,
        "rerank calibration corpus has an unexpected case count"
    );
    let mut ids = BTreeSet::new();
    for selection in selections {
        ensure!(
            ids.insert(selection.id.as_str()),
            "rerank calibration corpus contains duplicate case identities"
        );
        ensure!(
            selection.need_kind == NeedKind::Question,
            "rerank calibration corpus accepts question-passage cases only"
        );
        ensure!(
            selection.candidates.len() == EXPECTED_CANDIDATES_PER_CASE,
            "each rerank calibration case must contain exactly ten candidates"
        );
        ensure!(
            selection.query_language.is_some() && selection.candidate_language.is_some(),
            "rerank calibration cases require explicit query and candidate languages"
        );
        let support_count = selection
            .candidates
            .iter()
            .filter(|candidate| candidate.role == CandidateRole::Support)
            .count();
        match selection.expectation {
            CorpusExpectation::Answerable => ensure!(
                support_count > 0,
                "support-present rerank calibration case has no support"
            ),
            CorpusExpectation::Unanswerable => ensure!(
                support_count == 0,
                "support-absent rerank calibration case contains support"
            ),
        }
    }
    Ok(())
}

async fn score_case(
    provider: &FastEmbedMmarcoReranker,
    selection: &LoadedSelection,
) -> Result<ScoredCase> {
    let inputs = selection
        .candidates
        .iter()
        .map(|candidate| RelevanceInput {
            title: String::new(),
            heading: String::new(),
            text: candidate.passage.clone(),
        })
        .collect::<Vec<_>>();
    let started = Instant::now();
    let relevance = provider
        .classify_and_order_for_evaluation(selection.need.trim(), &inputs)
        .await?;
    let inference_micros = started.elapsed().as_micros();
    let scored = relevance.into_scored_parts()?;
    ensure!(
        scored.scores.len() == EXPECTED_CANDIDATES_PER_CASE,
        "rerank calibration provider returned an unexpected score count"
    );
    Ok(ScoredCase {
        id: selection.id.clone(),
        split: selection.split,
        source_id: selection.source_id.clone(),
        query_language: selection
            .query_language
            .context("rerank calibration query language is unavailable")?,
        candidate_language: selection
            .candidate_language
            .context("rerank calibration candidate language is unavailable")?,
        expectation: selection.expectation,
        roles: selection
            .candidates
            .iter()
            .map(|candidate| candidate.role)
            .collect(),
        production_decisions: scored.decisions,
        score_order: scored.score_order,
        scores: scored.scores,
        inference_micros,
    })
}

fn select_cutoff(training: &[&ScoredCase]) -> Result<f32> {
    ensure!(
        !training.is_empty(),
        "rerank calibration training split is empty"
    );
    let mut thresholds = training
        .iter()
        .flat_map(|case| case.scores.iter().copied())
        .collect::<Vec<_>>();
    ensure!(
        thresholds.iter().all(|score| score.is_finite()),
        "rerank calibration scores must be finite"
    );
    thresholds.push(f32::MAX);
    thresholds.sort_by(|left, right| right.total_cmp(left));
    thresholds.dedup_by(|left, right| left.total_cmp(right).is_eq());

    let mut selected = None;
    for threshold in thresholds {
        let evaluations = training
            .iter()
            .map(|case| threshold_evaluation(case, threshold, true))
            .collect::<Vec<_>>();
        let safe = evaluations
            .iter()
            .all(|evaluation| evaluation.hard_negative_count == 0 && evaluation.no_answer_correct);
        if !safe {
            continue;
        }
        let complete = evaluations
            .iter()
            .filter(|evaluation| evaluation.complete)
            .count();
        match selected {
            None => selected = Some((threshold, complete)),
            Some((best_threshold, best_complete))
                if complete > best_complete
                    || (complete == best_complete && threshold > best_threshold) =>
            {
                selected = Some((threshold, complete));
            }
            Some(_) => {}
        }
    }
    selected
        .map(|(threshold, _)| threshold)
        .context("rerank calibration could not select a fail-closed cutoff")
}

fn baseline_evaluation(case: &ScoredCase) -> ArmCaseEvaluation {
    let input_order = (0..case.roles.len()).collect::<Vec<_>>();
    evaluate_arm(case, &case.production_decisions, &input_order)
}

fn threshold_evaluation(
    case: &ScoredCase,
    cutoff: f32,
    use_score_order: bool,
) -> ArmCaseEvaluation {
    let decisions = threshold_decisions(&case.scores, cutoff);
    let input_order;
    let order = if use_score_order {
        case.score_order.as_slice()
    } else {
        input_order = (0..case.roles.len()).collect::<Vec<_>>();
        input_order.as_slice()
    };
    evaluate_arm(case, &decisions, order)
}

fn threshold_decisions(scores: &[f32], cutoff: f32) -> Vec<EvidenceDecision> {
    let Some(best) = scores.iter().copied().reduce(f32::max) else {
        return Vec::new();
    };
    if best < cutoff {
        return vec![EvidenceDecision::Irrelevant; scores.len()];
    }
    let relative_floor = best - CURRENT_RELATIVE_WINDOW;
    scores
        .iter()
        .map(|score| {
            if *score >= cutoff && *score >= relative_floor {
                EvidenceDecision::Relevant
            } else {
                EvidenceDecision::Irrelevant
            }
        })
        .collect()
}

fn evaluate_arm(
    case: &ScoredCase,
    decisions: &[EvidenceDecision],
    order: &[usize],
) -> ArmCaseEvaluation {
    let returned = order
        .iter()
        .copied()
        .filter(|index| decisions.get(*index) == Some(&EvidenceDecision::Relevant))
        .take(TOP_K)
        .collect::<Vec<_>>();
    let required_support_count = case
        .roles
        .iter()
        .filter(|role| **role == CandidateRole::Support)
        .count();
    let found_support_count = returned
        .iter()
        .filter(|index| case.roles.get(**index) == Some(&CandidateRole::Support))
        .count();
    let hard_negative_count = returned.len().saturating_sub(found_support_count);
    let complete = case.expectation == CorpusExpectation::Answerable
        && required_support_count > 0
        && found_support_count == required_support_count
        && hard_negative_count == 0;
    let no_answer_correct =
        case.expectation == CorpusExpectation::Answerable || returned.is_empty();
    let reciprocal_rank_at_five = returned
        .iter()
        .position(|index| case.roles.get(*index) == Some(&CandidateRole::Support))
        .map(|index| 1.0 / (index + 1) as f64);
    ArmCaseEvaluation {
        returned_count: u32::try_from(returned.len()).unwrap_or(u32::MAX),
        found_support_count: u32::try_from(found_support_count).unwrap_or(u32::MAX),
        required_support_count: u32::try_from(required_support_count).unwrap_or(u32::MAX),
        hard_negative_count: u32::try_from(hard_negative_count).unwrap_or(u32::MAX),
        complete,
        no_answer_correct,
        reciprocal_rank_at_five,
    }
}

fn summarize(
    cases: &[&ScoredCase],
    evaluate: impl Fn(&ScoredCase) -> ArmCaseEvaluation,
) -> QualitySummary {
    let rows = cases.iter().map(|case| evaluate(case)).collect::<Vec<_>>();
    summarize_evaluations(cases, &rows)
}

fn summarize_rows(
    rows: &[(
        &ScoredCase,
        ArmCaseEvaluation,
        ArmCaseEvaluation,
        ArmCaseEvaluation,
    )],
    arm_index: usize,
) -> QualitySummary {
    let cases = rows.iter().map(|row| row.0).collect::<Vec<_>>();
    let evaluations = rows
        .iter()
        .map(|row| match arm_index {
            1 => &row.1,
            2 => &row.2,
            3 => &row.3,
            _ => unreachable!("rerank calibration arm index is internal and fixed"),
        })
        .cloned()
        .collect::<Vec<_>>();
    summarize_evaluations(&cases, &evaluations)
}

fn summarize_evaluations(cases: &[&ScoredCase], rows: &[ArmCaseEvaluation]) -> QualitySummary {
    let support_present_count = cases
        .iter()
        .filter(|case| case.expectation == CorpusExpectation::Answerable)
        .count();
    let support_absent_count = cases.len().saturating_sub(support_present_count);
    let complete_count = rows.iter().filter(|row| row.complete).count();
    let required_support_count = rows
        .iter()
        .map(|row| row.required_support_count as usize)
        .sum::<usize>();
    let found_support_count = rows
        .iter()
        .map(|row| row.found_support_count as usize)
        .sum::<usize>();
    let no_answer_correct_count = cases
        .iter()
        .zip(rows)
        .filter(|(case, row)| {
            case.expectation == CorpusExpectation::Unanswerable && row.no_answer_correct
        })
        .count();
    let returned_count = rows
        .iter()
        .map(|row| row.returned_count as usize)
        .sum::<usize>();
    let hard_negative_count = rows
        .iter()
        .map(|row| row.hard_negative_count as usize)
        .sum::<usize>();
    let reciprocal_rank_sum = rows
        .iter()
        .map(|row| row.reciprocal_rank_at_five.unwrap_or(0.0))
        .sum::<f64>();
    QualitySummary {
        case_count: u32::try_from(cases.len()).unwrap_or(u32::MAX),
        support_present_count: u32::try_from(support_present_count).unwrap_or(u32::MAX),
        support_absent_count: u32::try_from(support_absent_count).unwrap_or(u32::MAX),
        complete_count: u32::try_from(complete_count).unwrap_or(u32::MAX),
        complete_coverage: ratio(complete_count, support_present_count),
        support_recall_at_five: ratio(found_support_count, required_support_count),
        mean_reciprocal_rank_at_five: (support_present_count != 0)
            .then(|| reciprocal_rank_sum / support_present_count as f64),
        no_answer_accuracy: ratio(no_answer_correct_count, support_absent_count),
        returned_count: u32::try_from(returned_count).unwrap_or(u32::MAX),
        hard_negative_count: u32::try_from(hard_negative_count).unwrap_or(u32::MAX),
        citation_precision: ratio(
            returned_count.saturating_sub(hard_negative_count),
            returned_count,
        ),
    }
}

fn improved_strata(
    rows: &[(
        &ScoredCase,
        ArmCaseEvaluation,
        ArmCaseEvaluation,
        ArmCaseEvaluation,
    )],
) -> Vec<String> {
    let mut counts = BTreeMap::<String, (u32, u32)>::new();
    for (case, baseline, _, threshold) in rows {
        let key = stratum(case);
        let entry = counts.entry(key).or_default();
        entry.0 = entry.0.saturating_add(u32::from(baseline.complete));
        entry.1 = entry.1.saturating_add(u32::from(threshold.complete));
    }
    counts
        .into_iter()
        .filter_map(|(key, (baseline, threshold))| (threshold > baseline).then_some(key))
        .collect()
}

fn stratum(case: &ScoredCase) -> String {
    if case.source_id == "xquad_en_es" {
        language_direction(case)
    } else {
        case.source_id.clone()
    }
}

fn language_direction(case: &ScoredCase) -> String {
    format!(
        "{}_to_{}",
        language_code(case.query_language),
        language_code(case.candidate_language)
    )
}

fn language_code(language: CorpusLanguage) -> &'static str {
    match language {
        CorpusLanguage::En => "en",
        CorpusLanguage::Es => "es",
    }
}

fn rejection_reasons(
    baseline: &QualitySummary,
    threshold: &QualitySummary,
    coverage_gain: Option<f64>,
    lost_baseline_complete_count: u32,
    improved_strata: &[String],
    decision_p95_micros: Option<u128>,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if threshold.hard_negative_count != 0 || threshold.citation_precision != Some(1.0) {
        reasons.push("calibrated arm returned hard-negative evidence".to_owned());
    }
    if threshold.no_answer_accuracy != Some(1.0) {
        reasons.push("calibrated arm did not abstain on every support-absent query".to_owned());
    }
    if lost_baseline_complete_count != 0 {
        reasons.push("calibrated arm lost a query completed by the baseline".to_owned());
    }
    if coverage_gain.is_none_or(|gain| gain < MIN_COMPLETE_COVERAGE_GAIN) {
        reasons.push("complete-query coverage gain was below 0.05".to_owned());
    }
    if improved_strata.len() < MIN_IMPROVED_STRATA {
        reasons.push("calibrated improvement reached fewer than two strata".to_owned());
    }
    if metric_regressed(
        threshold.mean_reciprocal_rank_at_five,
        baseline.mean_reciprocal_rank_at_five,
    ) {
        reasons.push("MRR@5 regressed against the baseline".to_owned());
    }
    if decision_p95_micros.is_none_or(|value| value >= MAX_DECISION_P95_MICROS) {
        reasons.push("calibrated decision overhead reached one millisecond p95".to_owned());
    }
    reasons
}

fn metric_regressed(candidate: Option<f64>, baseline: Option<f64>) -> bool {
    match (candidate, baseline) {
        (Some(candidate), Some(baseline)) => candidate + f64::EPSILON < baseline,
        (None, Some(_)) => true,
        _ => false,
    }
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator != 0).then(|| numerator as f64 / denominator as f64)
}

fn difference(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    Some(left? - right?)
}

fn write_report(report: &CalibrationReport) -> Result<std::path::PathBuf> {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    let manifest_prefix = report.manifest_sha256.get(..12).unwrap_or("unknown");
    let destination = workspace_root().join(super::REPORT_DIRECTORY).join(format!(
        "retrieval-rerank-calibration-development-{manifest_prefix}-{epoch}.json"
    ));
    let parent = destination
        .parent()
        .context("rerank calibration report has no parent")?;
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

    fn case(
        id: &str,
        expectation: CorpusExpectation,
        scores: Vec<f32>,
        support_index: Option<usize>,
    ) -> ScoredCase {
        let mut roles = vec![
            CandidateRole::HardNegative(
                super::super::corpus::HardNegativeKind::RelatedButUnanswered,
            );
            scores.len()
        ];
        if let Some(index) = support_index {
            roles[index] = CandidateRole::Support;
        }
        let mut score_order = (0..scores.len()).collect::<Vec<_>>();
        score_order.sort_by(|left, right| {
            scores[*right]
                .total_cmp(&scores[*left])
                .then_with(|| left.cmp(right))
        });
        ScoredCase {
            id: id.to_owned(),
            split: CorpusSplit::Training,
            source_id: "xquad_en_es".to_owned(),
            query_language: CorpusLanguage::En,
            candidate_language: CorpusLanguage::Es,
            expectation,
            roles,
            production_decisions: vec![EvidenceDecision::Irrelevant; scores.len()],
            score_order,
            scores,
            inference_micros: 1,
        }
    }

    #[test]
    fn cutoff_selection_prefers_the_highest_equally_safe_boundary() {
        let answerable = case(
            "answerable",
            CorpusExpectation::Answerable,
            vec![4.0, 1.0, -2.0],
            Some(0),
        );
        let absent = case(
            "absent",
            CorpusExpectation::Unanswerable,
            vec![3.0, 0.5, -3.0],
            None,
        );
        let cutoff = select_cutoff(&[&answerable, &absent]).unwrap();
        assert_eq!(cutoff, 4.0);
        assert!(threshold_evaluation(&answerable, cutoff, true).complete);
        assert!(threshold_evaluation(&absent, cutoff, true).no_answer_correct);
    }

    #[test]
    fn relative_window_remains_part_of_the_calibrated_rule() {
        let decisions = threshold_decisions(&[5.0, 1.5, 1.39], 1.0);
        assert_eq!(
            decisions,
            vec![
                EvidenceDecision::Relevant,
                EvidenceDecision::Relevant,
                EvidenceDecision::Irrelevant,
            ]
        );
    }

    #[test]
    fn report_schema_is_content_free_and_explicit() {
        let arm = ArmCaseSummary::from(&ArmCaseEvaluation::default());
        let case_summary = CaseSummary {
            id: "opaque_case".to_owned(),
            source: "source".to_owned(),
            language_direction: "en_to_es".to_owned(),
            support_present: true,
            baseline: arm.clone(),
            threshold_input_order: arm.clone(),
            threshold_score_order: arm,
        };
        let quality = QualitySummary {
            case_count: 1,
            support_present_count: 1,
            support_absent_count: 0,
            complete_count: 1,
            complete_coverage: Some(1.0),
            support_recall_at_five: Some(1.0),
            mean_reciprocal_rank_at_five: Some(1.0),
            no_answer_accuracy: None,
            returned_count: 1,
            hard_negative_count: 0,
            citation_precision: Some(1.0),
        };
        let report = CalibrationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            evaluation_role: "grouped-development-calibration",
            corpus_id: "corpus".to_owned(),
            manifest_sha256: "a".repeat(64),
            target_os: "test".to_owned(),
            target_arch: "test".to_owned(),
            relevance_profile: MMARCO_RERANKER_PROFILE_ID,
            relevance_revision: MMARCO_RERANKER_REVISION,
            relevance_artifact_filename: "model.onnx".to_owned(),
            relevance_artifact_sha256: "b".repeat(64),
            relevance_call_count: 1,
            total_candidate_count: 10,
            selected_absolute_logit_cutoff: 2.5,
            relative_window: CURRENT_RELATIVE_WINDOW,
            training_baseline: quality.clone(),
            training_threshold_score_order: quality.clone(),
            calibration_baseline: quality.clone(),
            calibration_threshold_input_order: quality.clone(),
            calibration_threshold_score_order: quality,
            calibration_complete_coverage_gain: Some(0.0),
            calibration_lost_baseline_complete_count: 0,
            improved_strata: Vec::new(),
            inference_latency: LatencySummary::from_values(vec![1]),
            decision_latency: LatencySummary::from_values(vec![1]),
            rejection_reasons: Vec::new(),
            development_gate_passed: false,
            sealed_holdout_authorized: false,
            production_promotion_ready: false,
            calibration_cases: vec![case_summary],
        };
        let value = serde_json::to_value(&report).unwrap();
        let actual_top_level = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let expected_top_level = [
            "schema_version",
            "evaluation_role",
            "corpus_id",
            "manifest_sha256",
            "target_os",
            "target_arch",
            "relevance_profile",
            "relevance_revision",
            "relevance_artifact_filename",
            "relevance_artifact_sha256",
            "relevance_call_count",
            "total_candidate_count",
            "selected_absolute_logit_cutoff",
            "relative_window",
            "training_baseline",
            "training_threshold_score_order",
            "calibration_baseline",
            "calibration_threshold_input_order",
            "calibration_threshold_score_order",
            "calibration_complete_coverage_gain",
            "calibration_lost_baseline_complete_count",
            "improved_strata",
            "inference_latency",
            "decision_latency",
            "rejection_reasons",
            "development_gate_passed",
            "sealed_holdout_authorized",
            "production_promotion_ready",
            "calibration_cases",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        assert_eq!(actual_top_level, expected_top_level);

        fn assert_content_free_keys(value: &serde_json::Value) {
            match value {
                serde_json::Value::Object(fields) => {
                    for (key, nested) in fields {
                        assert!(
                            !matches!(
                                key.as_str(),
                                "scores"
                                    | "raw_scores"
                                    | "logits"
                                    | "raw_logits"
                                    | "question"
                                    | "query"
                                    | "passage"
                                    | "text"
                                    | "source_path"
                                    | "source_paths"
                            ),
                            "report serialized forbidden field `{key}`"
                        );
                        assert_content_free_keys(nested);
                    }
                }
                serde_json::Value::Array(values) => {
                    for nested in values {
                        assert_content_free_keys(nested);
                    }
                }
                _ => {}
            }
        }
        assert_content_free_keys(&value);
    }

    #[test]
    fn gate_rejects_false_evidence_even_when_coverage_improves() {
        let baseline = QualitySummary {
            case_count: 2,
            support_present_count: 1,
            support_absent_count: 1,
            complete_count: 0,
            complete_coverage: Some(0.0),
            support_recall_at_five: Some(0.0),
            mean_reciprocal_rank_at_five: None,
            no_answer_accuracy: Some(1.0),
            returned_count: 0,
            hard_negative_count: 0,
            citation_precision: None,
        };
        let mut candidate = baseline.clone();
        candidate.complete_count = 1;
        candidate.complete_coverage = Some(1.0);
        candidate.mean_reciprocal_rank_at_five = Some(1.0);
        candidate.returned_count = 2;
        candidate.hard_negative_count = 1;
        candidate.citation_precision = Some(0.5);
        assert!(
            rejection_reasons(
                &baseline,
                &candidate,
                Some(1.0),
                0,
                &["en_to_es".to_owned(), "squad_v2".to_owned()],
                Some(1),
            )
            .iter()
            .any(|reason| reason.contains("hard-negative"))
        );
    }
}
