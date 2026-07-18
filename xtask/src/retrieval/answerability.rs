use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use airwiki_inference::{
    AssetManager, GenerationConfig, LLAMA_CPP_BUILD, LlamaClient, LlamaSupervisor, ModelProfile,
    ServerReasoningMode, SupervisorConfig, selection_for_model,
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    ANSWERABILITY_CORPUS_MANIFEST_PATH, REPORT_DIRECTORY, corpus, percentile, qa_entailment,
};
use crate::{replace_file, workspace_root};

const REPORT_SCHEMA_VERSION: u32 = 5;
const ANSWER_MATCH_POLICY_VERSION: &str = "squad-v2-normalized-exact-match-v2";
const ANSWERABILITY_SCORING_POLICY_VERSION: &str = "answerability-scoring-gate-v1";

#[derive(Debug, Clone, Serialize)]
struct ProviderIdentity {
    model_catalog_id: String,
    model_revision: String,
    model_artifact_sha256: String,
    llama_cpp_build: String,
    qa_profile_id: String,
    qa_prompt_version: String,
    qa_validation_policy_version: String,
    qa_policy_fingerprint: String,
    answer_match_policy_version: String,
    scoring_policy_version: String,
    candidate_order_policy_version: String,
    thread_count: usize,
    context_tokens: u32,
    max_input_tokens: u32,
    max_output_tokens: u16,
    temperature: f32,
    reasoning: String,
}

#[derive(Debug, Clone, Default, Serialize)]
struct InvalidReasonReport {
    input_contract: usize,
    serialization: usize,
    output_schema: usize,
    empty_abstention_contract: usize,
    candidate_reference: usize,
    quote_provenance: usize,
    answer_span: usize,
    claim_consistency: usize,
    verification_schema: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
struct GateReport {
    selection_count: usize,
    expected_answerable_count: usize,
    expected_unanswerable_count: usize,
    expected_support_candidate_count: usize,
    proposed_support_candidate_count: usize,
    proposed_hard_negative_count: usize,
    accepted_support_candidate_count: usize,
    accepted_hard_negative_count: usize,
    false_positive_decision_count: usize,
    false_negative_decision_count: usize,
    answer_mismatch_count: usize,
    decision_invariant_failure_count: usize,
    invalid_output_count: usize,
    invalid_reasons: InvalidReasonReport,
    timeout_count: usize,
    provider_failure_count: usize,
    proposal_failure_count: usize,
    verification_failure_count: usize,
    verifier_call_count: usize,
    language_pair_count: usize,
    language_parity_failure_count: usize,
}

impl GateReport {
    fn passed(&self) -> bool {
        self.accepted_support_candidate_count == self.expected_support_candidate_count
            && self.accepted_hard_negative_count == 0
            && self.false_positive_decision_count == 0
            && self.false_negative_decision_count == 0
            && self.answer_mismatch_count == 0
            && self.decision_invariant_failure_count == 0
            && self.invalid_output_count == 0
            && self.timeout_count == 0
            && self.provider_failure_count == 0
            && self.language_parity_failure_count == 0
    }
}

#[derive(Debug, Clone, Default, Serialize)]
struct LatencyReport {
    call_count: usize,
    p50_ms: Option<u128>,
    p95_ms: Option<u128>,
    max_ms: Option<u128>,
}

impl LatencyReport {
    fn from_elapsed(mut elapsed_ms: Vec<u128>) -> Self {
        elapsed_ms.sort_unstable();
        Self {
            call_count: elapsed_ms.len(),
            p50_ms: percentile(&elapsed_ms, 50),
            p95_ms: percentile(&elapsed_ms, 95),
            max_ms: elapsed_ms.last().copied(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
struct LatencySummary {
    proposal: LatencyReport,
    verification: LatencyReport,
}

#[derive(Debug, Clone, Serialize)]
struct SplitReport {
    results: GateReport,
    latency: LatencySummary,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct EvaluationReport {
    schema_version: u32,
    evaluation_role: String,
    corpus_manifest_sha256: String,
    candidate_fingerprint: String,
    target_os: String,
    target_arch: String,
    provider: ProviderIdentity,
    startup_ms: u128,
    elapsed_ms: u128,
    training: SplitReport,
    calibration: SplitReport,
    total: SplitReport,
    passed: bool,
}

struct ObservedSuccess {
    selected_candidate_ids: Vec<String>,
    accepted: bool,
    verifier_called: bool,
    answers_need: bool,
    entailed: bool,
    answer_matches_reference: bool,
    proposal_elapsed_ms: u128,
    verification_elapsed_ms: Option<u128>,
}

enum ObservedResult {
    Success(ObservedSuccess),
    Failure(qa_entailment::QaEntailmentError),
}

struct ParityObservation<'a> {
    group_id: &'a str,
    expectation: corpus::CorpusExpectation,
    decision: Option<bool>,
}

struct ScoringCandidate<'a> {
    id: &'a str,
    is_support: bool,
}

struct ScoringInput<'a> {
    expectation: corpus::CorpusExpectation,
    requires_answer_match: bool,
    candidates: Vec<ScoringCandidate<'a>>,
}

#[derive(Default)]
struct ScoreAccumulator {
    results: GateReport,
    proposal_elapsed_ms: Vec<u128>,
    verification_elapsed_ms: Vec<u128>,
}

impl ScoreAccumulator {
    fn record(&mut self, input: &ScoringInput<'_>, observed: &ObservedResult) {
        self.results.selection_count += 1;
        match input.expectation {
            corpus::CorpusExpectation::Answerable => {
                self.results.expected_answerable_count += 1;
            }
            corpus::CorpusExpectation::Unanswerable => {
                self.results.expected_unanswerable_count += 1;
            }
        }
        self.results.expected_support_candidate_count += input
            .candidates
            .iter()
            .filter(|candidate| candidate.is_support)
            .count();

        let success = match observed {
            ObservedResult::Success(success) => success,
            ObservedResult::Failure(error) => {
                self.record_failure(input, error);
                return;
            }
        };

        self.proposal_elapsed_ms.push(success.proposal_elapsed_ms);
        if let Some(elapsed_ms) = success.verification_elapsed_ms {
            self.verification_elapsed_ms.push(elapsed_ms);
        }
        self.results.verifier_call_count += usize::from(success.verifier_called);

        self.record_selected_candidates(input, &success.selected_candidate_ids, success.accepted);

        let expected_decision = input.expectation == corpus::CorpusExpectation::Answerable;
        if success.accepted && !expected_decision {
            self.results.false_positive_decision_count += 1;
        } else if !success.accepted && expected_decision {
            self.results.false_negative_decision_count += 1;
        }
        if success.accepted && input.requires_answer_match && !success.answer_matches_reference {
            self.results.answer_mismatch_count += 1;
        }
        if success.accepted != (success.answers_need && success.entailed)
            || success.verifier_called != success.verification_elapsed_ms.is_some()
            || success.verifier_called == success.selected_candidate_ids.is_empty()
        {
            self.results.decision_invariant_failure_count += 1;
        }
    }

    fn record_selected_candidates(
        &mut self,
        input: &ScoringInput<'_>,
        selected_candidate_ids: &[String],
        accepted: bool,
    ) {
        let mut selected = HashSet::with_capacity(selected_candidate_ids.len());
        for candidate_id in selected_candidate_ids {
            if !selected.insert(candidate_id.as_str()) {
                self.results.invalid_output_count += 1;
                continue;
            }
            match input
                .candidates
                .iter()
                .find(|candidate| candidate.id == *candidate_id)
                .map(|candidate| candidate.is_support)
            {
                Some(true) => {
                    self.results.proposed_support_candidate_count += 1;
                    if accepted {
                        self.results.accepted_support_candidate_count += 1;
                    }
                }
                Some(false) => {
                    self.results.proposed_hard_negative_count += 1;
                    if accepted {
                        self.results.accepted_hard_negative_count += 1;
                    }
                }
                None => self.results.invalid_output_count += 1,
            }
        }
    }

    fn record_stage_failure(&mut self, stage: qa_entailment::QaEntailmentStage) {
        match stage {
            qa_entailment::QaEntailmentStage::Proposal => {
                self.results.proposal_failure_count += 1;
            }
            qa_entailment::QaEntailmentStage::Verification => {
                self.results.verification_failure_count += 1;
            }
        }
    }

    fn record_failure(
        &mut self,
        input: &ScoringInput<'_>,
        error: &qa_entailment::QaEntailmentError,
    ) {
        self.record_selected_candidates(input, error.selected_candidate_ids(), false);
        if let Some(elapsed) = error.proposal_elapsed() {
            self.proposal_elapsed_ms.push(elapsed.as_millis());
        }
        if let Some(elapsed) = error.verification_elapsed() {
            self.verification_elapsed_ms.push(elapsed.as_millis());
            self.results.verifier_call_count += 1;
        }
        match error.kind() {
            qa_entailment::QaEntailmentFailureKind::TimedOut => self.results.timeout_count += 1,
            qa_entailment::QaEntailmentFailureKind::InferenceFailed => {
                self.results.provider_failure_count += 1;
            }
            qa_entailment::QaEntailmentFailureKind::InvalidOutput => {
                self.results.invalid_output_count += 1;
                if let Some(reason) = error.invalid_reason() {
                    self.record_invalid_reason(reason);
                }
            }
        }
        self.record_stage_failure(error.stage());
        if input.expectation == corpus::CorpusExpectation::Answerable {
            self.results.false_negative_decision_count += 1;
        }
    }

    fn record_invalid_reason(&mut self, reason: qa_entailment::QaEntailmentInvalidReason) {
        let reasons = &mut self.results.invalid_reasons;
        match reason {
            qa_entailment::QaEntailmentInvalidReason::InputContract => {
                reasons.input_contract += 1;
            }
            qa_entailment::QaEntailmentInvalidReason::Serialization => {
                reasons.serialization += 1;
            }
            qa_entailment::QaEntailmentInvalidReason::OutputSchema => {
                reasons.output_schema += 1;
            }
            qa_entailment::QaEntailmentInvalidReason::EmptyAbstentionContract => {
                reasons.empty_abstention_contract += 1;
            }
            qa_entailment::QaEntailmentInvalidReason::CandidateReference => {
                reasons.candidate_reference += 1;
            }
            qa_entailment::QaEntailmentInvalidReason::QuoteProvenance => {
                reasons.quote_provenance += 1;
            }
            qa_entailment::QaEntailmentInvalidReason::AnswerSpan => {
                reasons.answer_span += 1;
            }
            qa_entailment::QaEntailmentInvalidReason::ClaimConsistency => {
                reasons.claim_consistency += 1;
            }
            qa_entailment::QaEntailmentInvalidReason::VerificationSchema => {
                reasons.verification_schema += 1;
            }
        }
    }

    fn finish(mut self, parity_observations: &[ParityObservation<'_>]) -> SplitReport {
        let (pair_count, failure_count) = language_parity_counts(parity_observations);
        self.results.language_pair_count = pair_count;
        self.results.language_parity_failure_count = failure_count;
        let passed = self.results.passed();
        SplitReport {
            results: self.results,
            latency: LatencySummary {
                proposal: LatencyReport::from_elapsed(self.proposal_elapsed_ms),
                verification: LatencyReport::from_elapsed(self.verification_elapsed_ms),
            },
            passed,
        }
    }
}

fn language_parity_counts(observations: &[ParityObservation<'_>]) -> (usize, usize) {
    let mut groups = HashMap::<&str, Vec<&ParityObservation<'_>>>::new();
    for observation in observations {
        groups
            .entry(observation.group_id)
            .or_default()
            .push(observation);
    }

    let mut pair_count = 0;
    let mut failure_count = 0;
    for group in groups.values() {
        if group.len() != 2 || group[0].expectation != group[1].expectation {
            continue;
        }
        pair_count += 1;
        if group[0].decision.is_none() || group[0].decision != group[1].decision {
            failure_count += 1;
        }
    }
    (pair_count, failure_count)
}

pub(crate) async fn evaluate_answerability(
    source_root: &Path,
    data_root: &Path,
    llama_server: &Path,
    model_id: &str,
) -> Result<()> {
    let manifest_path = workspace_root().join(ANSWERABILITY_CORPUS_MANIFEST_PATH);
    let corpus = corpus::load_verified_corpus(&manifest_path, source_root)?;
    let selection = selection_for_model(
        ModelProfile::Automatic,
        model_id,
        "answerability development evaluation",
    )
    .context("answerability model is not in the pinned AirWiki catalog")?;
    let outcome = AssetManager::new(data_root)?
        .with_bundled_runtime(Some(llama_server.to_path_buf()))
        .verify_selection(&selection)
        .await
        .context("answerability assets failed verification")?;
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 4);

    let provider = ProviderIdentity {
        model_catalog_id: outcome.selection.model_id.to_owned(),
        model_revision: outcome.selection.manifest.artifact.revision.to_owned(),
        model_artifact_sha256: outcome.selection.manifest.artifact.sha256.to_owned(),
        llama_cpp_build: LLAMA_CPP_BUILD.to_owned(),
        qa_profile_id: qa_entailment::QA_ENTAILMENT_PROFILE_ID.to_owned(),
        qa_prompt_version: qa_entailment::QA_ENTAILMENT_PROMPT_VERSION.to_owned(),
        qa_validation_policy_version: qa_entailment::QA_ENTAILMENT_VALIDATION_POLICY_VERSION
            .to_owned(),
        qa_policy_fingerprint: qa_entailment::policy_fingerprint(),
        answer_match_policy_version: ANSWER_MATCH_POLICY_VERSION.to_owned(),
        scoring_policy_version: ANSWERABILITY_SCORING_POLICY_VERSION.to_owned(),
        candidate_order_policy_version: corpus::CANDIDATE_ORDER_POLICY_VERSION.to_owned(),
        thread_count: threads,
        context_tokens: outcome.generation_settings.context_tokens,
        max_input_tokens: outcome.generation_settings.max_input_tokens,
        max_output_tokens: outcome.generation_settings.max_output_tokens,
        temperature: 0.0,
        reasoning: "off".to_owned(),
    };
    let candidate_fingerprint = candidate_fingerprint(&corpus.manifest_sha256, &provider);

    let mut supervisor_config =
        SupervisorConfig::bundled(outcome.llama_server_path, outcome.model_path);
    supervisor_config.model_id = outcome.generation_settings.model_api_id.to_owned();
    supervisor_config.context_tokens = outcome.generation_settings.context_tokens;
    supervisor_config.threads = threads;
    supervisor_config.reasoning_mode = ServerReasoningMode::Off;
    supervisor_config.idle_timeout = Duration::from_secs(15 * 60);
    let supervisor = LlamaSupervisor::new(supervisor_config);
    let startup_started = Instant::now();
    let endpoint = supervisor
        .ensure_running()
        .await
        .context("answerability runtime did not become ready")?;
    let startup_ms = startup_started.elapsed().as_millis();

    let evaluation_result = async {
        let mut generation_config = GenerationConfig::from_settings(outcome.generation_settings);
        generation_config.temperature = 0.0;
        generation_config.timeout =
            qa_entailment::QA_ENTAILMENT_CALL_TIMEOUT + Duration::from_secs(5);
        let evaluator = qa_entailment::QaEntailmentEvaluator::new(LlamaClient::new(
            endpoint,
            generation_config,
        )?);
        Ok::<_, anyhow::Error>(
            run_evaluation(
                &corpus,
                &evaluator,
                provider,
                candidate_fingerprint,
                startup_ms,
            )
            .await,
        )
    }
    .await;
    let stop_result = supervisor.stop().await;
    let report = evaluation_result?;
    let destination = write_report(&report)?;
    ensure!(
        stop_result.is_ok(),
        "answerability runtime did not stop cleanly; report written to {}",
        destination.display()
    );
    ensure!(
        report.passed,
        "answerability experiment did not meet the development gate; report written to {}",
        destination.display()
    );
    println!(
        "answerability experiment passed the development gate; report written to {}",
        destination.display()
    );
    Ok(())
}

async fn run_evaluation(
    corpus: &corpus::LoadedCorpus,
    evaluator: &qa_entailment::QaEntailmentEvaluator,
    provider: ProviderIdentity,
    candidate_fingerprint: String,
    startup_ms: u128,
) -> EvaluationReport {
    let started = Instant::now();
    let mut total_scores = ScoreAccumulator::default();
    let mut training_scores = ScoreAccumulator::default();
    let mut calibration_scores = ScoreAccumulator::default();
    let mut total_parity = Vec::with_capacity(corpus.selections.len());
    let mut training_parity = Vec::new();
    let mut calibration_parity = Vec::new();

    for selection in &corpus.selections {
        let need_kind = match selection.need_kind {
            corpus::NeedKind::Question => qa_entailment::QaNeedKind::Question,
            corpus::NeedKind::Claim => qa_entailment::QaNeedKind::Claim,
        };
        let candidates = selection
            .candidates
            .iter()
            .map(|candidate| {
                qa_entailment::QaCandidateInput::new(&candidate.id, &candidate.passage)
            })
            .collect();
        let input = qa_entailment::QaEntailmentInput::new(need_kind, &selection.need, candidates);
        let observed = match evaluator.evaluate(&input).await {
            Ok(outcome) => {
                let answer_matches_reference = match selection.need_kind {
                    corpus::NeedKind::Question => {
                        outcome.answer_text.as_deref().is_some_and(|answer| {
                            answer_matches_reference(answer, &selection.reference_answers)
                        })
                    }
                    corpus::NeedKind::Claim => outcome.answer_text.is_none(),
                };
                ObservedResult::Success(ObservedSuccess {
                    selected_candidate_ids: outcome.selected_candidate_ids,
                    accepted: outcome.accepted,
                    verifier_called: outcome.verifier_called,
                    answers_need: outcome.answers_need,
                    entailed: outcome.entailed,
                    answer_matches_reference,
                    proposal_elapsed_ms: outcome.proposal_elapsed.as_millis(),
                    verification_elapsed_ms: outcome
                        .verification_elapsed
                        .map(|elapsed| elapsed.as_millis()),
                })
            }
            Err(error) => ObservedResult::Failure(error),
        };
        let decision = match &observed {
            ObservedResult::Success(success) => Some(success.accepted),
            ObservedResult::Failure(_) => Some(false),
        };
        let scoring_input = ScoringInput {
            expectation: selection.expectation,
            requires_answer_match: selection.need_kind == corpus::NeedKind::Question,
            candidates: selection
                .candidates
                .iter()
                .map(|candidate| ScoringCandidate {
                    id: &candidate.id,
                    is_support: matches!(candidate.role, corpus::CandidateRole::Support),
                })
                .collect(),
        };
        total_scores.record(&scoring_input, &observed);
        total_parity.push(ParityObservation {
            group_id: &selection.group_id,
            expectation: selection.expectation,
            decision,
        });
        match selection.split {
            corpus::CorpusSplit::Training => {
                training_scores.record(&scoring_input, &observed);
                training_parity.push(ParityObservation {
                    group_id: &selection.group_id,
                    expectation: selection.expectation,
                    decision,
                });
            }
            corpus::CorpusSplit::Calibration => {
                calibration_scores.record(&scoring_input, &observed);
                calibration_parity.push(ParityObservation {
                    group_id: &selection.group_id,
                    expectation: selection.expectation,
                    decision,
                });
            }
        }
    }

    let training = training_scores.finish(&training_parity);
    let calibration = calibration_scores.finish(&calibration_parity);
    let total = total_scores.finish(&total_parity);
    let passed = training.passed
        && calibration.passed
        && total.passed
        && total.results.language_pair_count > 0;
    EvaluationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        evaluation_role: "seed_rejection_gate".to_owned(),
        corpus_manifest_sha256: corpus.manifest_sha256.clone(),
        candidate_fingerprint,
        target_os: std::env::consts::OS.to_owned(),
        target_arch: std::env::consts::ARCH.to_owned(),
        provider,
        startup_ms,
        elapsed_ms: started.elapsed().as_millis(),
        training,
        calibration,
        total,
        passed,
    }
}

fn candidate_fingerprint(corpus_manifest_sha256: &str, identity: &ProviderIdentity) -> String {
    let mut hasher = Sha256::new();
    for value in [
        corpus_manifest_sha256,
        identity.model_catalog_id.as_str(),
        identity.model_revision.as_str(),
        identity.model_artifact_sha256.as_str(),
        identity.llama_cpp_build.as_str(),
        identity.qa_profile_id.as_str(),
        identity.qa_prompt_version.as_str(),
        identity.qa_validation_policy_version.as_str(),
        identity.qa_policy_fingerprint.as_str(),
        identity.answer_match_policy_version.as_str(),
        identity.scoring_policy_version.as_str(),
        identity.candidate_order_policy_version.as_str(),
        identity.reasoning.as_str(),
    ] {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    hasher.update(identity.thread_count.to_le_bytes());
    hasher.update(identity.context_tokens.to_le_bytes());
    hasher.update(identity.max_input_tokens.to_le_bytes());
    hasher.update(identity.max_output_tokens.to_le_bytes());
    hasher.update(identity.temperature.to_le_bytes());
    hex::encode(hasher.finalize())
}

fn answer_matches_reference(answer: &str, references: &[String]) -> bool {
    let normalized_answer = normalize_squad_answer(answer);
    !normalized_answer.is_empty()
        && references
            .iter()
            .any(|reference| normalize_squad_answer(reference) == normalized_answer)
}

fn normalize_squad_answer(value: &str) -> String {
    const ARTICLES: [&[char]; 3] = [&['t', 'h', 'e'], &['a', 'n'], &['a']];

    let lowercase_without_punctuation = value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| !character.is_ascii_punctuation())
        .collect::<Vec<_>>();
    let mut without_articles = String::with_capacity(lowercase_without_punctuation.len());
    let mut cursor = 0;
    while cursor < lowercase_without_punctuation.len() {
        let article_length = ARTICLES.iter().find_map(|article| {
            let end = cursor + article.len();
            if lowercase_without_punctuation.get(cursor..end) != Some(*article) {
                return None;
            }
            let starts_at_boundary =
                cursor == 0 || !is_squad_word_character(lowercase_without_punctuation[cursor - 1]);
            let ends_at_boundary = end == lowercase_without_punctuation.len()
                || !is_squad_word_character(lowercase_without_punctuation[end]);
            (starts_at_boundary && ends_at_boundary).then_some(article.len())
        });
        if let Some(article_length) = article_length {
            without_articles.push(' ');
            cursor += article_length;
        } else {
            without_articles.push(lowercase_without_punctuation[cursor]);
            cursor += 1;
        }
    }
    without_articles
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_squad_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn report_path() -> PathBuf {
    workspace_root().join(REPORT_DIRECTORY).join(format!(
        "retrieval-answerability-development-{}-{}.json",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

fn write_report(report: &EvaluationReport) -> Result<PathBuf> {
    let destination = report_path();
    let parent = destination
        .parent()
        .context("answerability report has no parent")?;
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

    fn scoring_input(
        expectation: corpus::CorpusExpectation,
        candidates: Vec<(&'static str, bool)>,
    ) -> ScoringInput<'static> {
        ScoringInput {
            expectation,
            requires_answer_match: false,
            candidates: candidates
                .into_iter()
                .map(|(id, is_support)| ScoringCandidate { id, is_support })
                .collect(),
        }
    }

    fn observed_success(selected_candidate_ids: &[&str], accepted: bool) -> ObservedResult {
        ObservedResult::Success(ObservedSuccess {
            selected_candidate_ids: selected_candidate_ids
                .iter()
                .map(|id| (*id).to_owned())
                .collect(),
            accepted,
            verifier_called: true,
            answers_need: accepted,
            entailed: accepted,
            answer_matches_reference: true,
            proposal_elapsed_ms: 2,
            verification_elapsed_ms: Some(3),
        })
    }

    #[test]
    fn gate_accepts_every_support_and_rejects_negatives() {
        let input = scoring_input(
            corpus::CorpusExpectation::Answerable,
            vec![("opaque-a", true), ("opaque-b", false), ("opaque-c", true)],
        );
        let mut accumulator = ScoreAccumulator::default();

        accumulator.record(&input, &observed_success(&["opaque-a", "opaque-c"], true));
        let report = accumulator.finish(&[]);

        assert!(report.passed);
        assert_eq!(report.results.expected_support_candidate_count, 2);
        assert_eq!(report.results.proposed_support_candidate_count, 2);
        assert_eq!(report.results.accepted_support_candidate_count, 2);
        assert_eq!(report.results.accepted_hard_negative_count, 0);
        assert_eq!(report.latency.proposal.call_count, 1);
        assert_eq!(report.latency.verification.call_count, 1);
    }

    #[test]
    fn gate_allows_a_hard_negative_proposal_rejected_by_the_verifier() {
        let input = scoring_input(
            corpus::CorpusExpectation::Unanswerable,
            vec![("opaque-a", false)],
        );
        let mut accumulator = ScoreAccumulator::default();

        accumulator.record(&input, &observed_success(&["opaque-a"], false));
        let report = accumulator.finish(&[]);

        assert!(report.passed);
        assert_eq!(report.results.proposed_hard_negative_count, 1);
        assert_eq!(report.results.accepted_hard_negative_count, 0);
    }

    #[test]
    fn gate_reports_false_positive_and_false_negative_decisions() {
        let answerable = scoring_input(
            corpus::CorpusExpectation::Answerable,
            vec![("opaque-support", true)],
        );
        let unanswerable = scoring_input(
            corpus::CorpusExpectation::Unanswerable,
            vec![("opaque-negative", false)],
        );
        let mut accumulator = ScoreAccumulator::default();

        accumulator.record(&answerable, &observed_success(&["opaque-support"], false));
        accumulator.record(&unanswerable, &observed_success(&["opaque-negative"], true));
        let report = accumulator.finish(&[]);

        assert!(!report.passed);
        assert_eq!(report.results.false_positive_decision_count, 1);
        assert_eq!(report.results.false_negative_decision_count, 1);
        assert_eq!(report.results.accepted_hard_negative_count, 1);
    }

    #[test]
    fn gate_rejects_an_accepted_question_with_a_non_matching_answer() {
        let mut input = scoring_input(
            corpus::CorpusExpectation::Answerable,
            vec![("opaque-support", true)],
        );
        input.requires_answer_match = true;
        let mut observed = observed_success(&["opaque-support"], true);
        let ObservedResult::Success(success) = &mut observed else {
            unreachable!();
        };
        success.answer_matches_reference = false;
        let mut accumulator = ScoreAccumulator::default();

        accumulator.record(&input, &observed);
        let report = accumulator.finish(&[]);

        assert!(!report.passed);
        assert_eq!(report.results.answer_mismatch_count, 1);
    }

    #[test]
    fn answer_matching_uses_the_official_squad_style_normalization() {
        let references = ["The Eiffel Tower".to_owned()];
        let spanish_references = ["Café de París".to_owned()];
        let unicode_boundary_references = ["—answer".to_owned()];

        assert!(answer_matches_reference("Eiffel Tower!", &references));
        assert!(answer_matches_reference(
            "Café de París.",
            &spanish_references
        ));
        assert!(answer_matches_reference(
            "the—answer",
            &unicode_boundary_references
        ));
        assert!(!answer_matches_reference("Eiffel", &references));
    }

    #[test]
    fn gate_requires_all_support_candidates() {
        let input = scoring_input(
            corpus::CorpusExpectation::Answerable,
            vec![("opaque-a", true), ("opaque-b", true)],
        );
        let mut accumulator = ScoreAccumulator::default();

        accumulator.record(&input, &observed_success(&["opaque-a"], true));
        let report = accumulator.finish(&[]);

        assert!(!report.passed);
        assert_eq!(report.results.expected_support_candidate_count, 2);
        assert_eq!(report.results.proposed_support_candidate_count, 1);
        assert_eq!(report.results.accepted_support_candidate_count, 1);
    }

    #[test]
    fn gate_fails_closed_on_duplicate_and_unknown_ids() {
        let input = scoring_input(
            corpus::CorpusExpectation::Answerable,
            vec![("opaque-a", true)],
        );
        let mut accumulator = ScoreAccumulator::default();

        accumulator.record(
            &input,
            &observed_success(&["opaque-a", "opaque-a", "unknown"], true),
        );
        let report = accumulator.finish(&[]);

        assert!(!report.passed);
        assert_eq!(report.results.invalid_output_count, 2);
    }

    #[test]
    fn gate_counts_every_provider_failure_class() {
        let input = scoring_input(corpus::CorpusExpectation::Unanswerable, Vec::new());
        let mut accumulator = ScoreAccumulator::default();

        accumulator.record(
            &input,
            &ObservedResult::Failure(qa_entailment::QaEntailmentError::invalid(
                qa_entailment::QaEntailmentStage::Proposal,
                qa_entailment::QaEntailmentInvalidReason::OutputSchema,
            )),
        );
        accumulator.record(
            &input,
            &ObservedResult::Failure(qa_entailment::QaEntailmentError::new(
                qa_entailment::QaEntailmentStage::Verification,
                qa_entailment::QaEntailmentFailureKind::TimedOut,
            )),
        );
        accumulator.record(
            &input,
            &ObservedResult::Failure(qa_entailment::QaEntailmentError::new(
                qa_entailment::QaEntailmentStage::Proposal,
                qa_entailment::QaEntailmentFailureKind::InferenceFailed,
            )),
        );
        let report = accumulator.finish(&[]);

        assert!(!report.passed);
        assert_eq!(report.results.invalid_output_count, 1);
        assert_eq!(report.results.timeout_count, 1);
        assert_eq!(report.results.provider_failure_count, 1);
        assert_eq!(report.results.invalid_reasons.output_schema, 1);
        assert_eq!(report.results.proposal_failure_count, 2);
        assert_eq!(report.results.verification_failure_count, 1);
    }

    #[test]
    fn operational_failure_is_scored_as_abstention_and_keeps_latency() {
        let input = scoring_input(
            corpus::CorpusExpectation::Answerable,
            vec![("opaque-support", true)],
        );
        let error = qa_entailment::QaEntailmentError::invalid(
            qa_entailment::QaEntailmentStage::Proposal,
            qa_entailment::QaEntailmentInvalidReason::QuoteProvenance,
        )
        .with_proposal_elapsed(Duration::from_millis(7));
        let mut accumulator = ScoreAccumulator::default();

        accumulator.record(&input, &ObservedResult::Failure(error));
        let report = accumulator.finish(&[]);

        assert!(!report.passed);
        assert_eq!(report.results.false_negative_decision_count, 1);
        assert_eq!(report.results.invalid_reasons.quote_provenance, 1);
        assert_eq!(report.latency.proposal.call_count, 1);
        assert_eq!(report.latency.proposal.max_ms, Some(7));
    }

    #[test]
    fn verification_failure_preserves_proposed_evidence_without_accepting_it() {
        let input = scoring_input(
            corpus::CorpusExpectation::Answerable,
            vec![("opaque-support", true), ("opaque-negative", false)],
        );
        let error = qa_entailment::QaEntailmentError::new(
            qa_entailment::QaEntailmentStage::Verification,
            qa_entailment::QaEntailmentFailureKind::TimedOut,
        )
        .with_proposal_elapsed(Duration::from_millis(5))
        .with_verification_elapsed(Duration::from_millis(7))
        .with_selected_candidate_ids(vec![
            "opaque-support".to_owned(),
            "opaque-negative".to_owned(),
        ]);
        let mut accumulator = ScoreAccumulator::default();

        accumulator.record(&input, &ObservedResult::Failure(error));
        let report = accumulator.finish(&[]);

        assert_eq!(report.results.proposed_support_candidate_count, 1);
        assert_eq!(report.results.proposed_hard_negative_count, 1);
        assert_eq!(report.results.accepted_support_candidate_count, 0);
        assert_eq!(report.results.accepted_hard_negative_count, 0);
        assert_eq!(report.results.false_negative_decision_count, 1);
        assert_eq!(report.results.verifier_call_count, 1);
    }

    #[test]
    fn language_parity_compares_only_same_expectation_pairs() {
        let observations = [
            ParityObservation {
                group_id: "parallel-family",
                expectation: corpus::CorpusExpectation::Answerable,
                decision: Some(true),
            },
            ParityObservation {
                group_id: "parallel-family",
                expectation: corpus::CorpusExpectation::Answerable,
                decision: Some(false),
            },
            ParityObservation {
                group_id: "contract-document",
                expectation: corpus::CorpusExpectation::Answerable,
                decision: Some(true),
            },
            ParityObservation {
                group_id: "contract-document",
                expectation: corpus::CorpusExpectation::Unanswerable,
                decision: Some(false),
            },
        ];

        assert_eq!(language_parity_counts(&observations), (1, 1));
    }

    #[test]
    fn report_contains_only_aggregate_sanitized_results() {
        let split = SplitReport {
            results: GateReport::default(),
            latency: LatencySummary::default(),
            passed: true,
        };
        let report = EvaluationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            evaluation_role: "seed_rejection_gate".to_owned(),
            corpus_manifest_sha256: "manifest-fingerprint".to_owned(),
            candidate_fingerprint: "candidate-fingerprint".to_owned(),
            target_os: "test".to_owned(),
            target_arch: "test".to_owned(),
            provider: ProviderIdentity {
                model_catalog_id: "catalog-model".to_owned(),
                model_revision: "model-revision".to_owned(),
                model_artifact_sha256: "model-fingerprint".to_owned(),
                llama_cpp_build: "runtime-build".to_owned(),
                qa_profile_id: "qa-profile".to_owned(),
                qa_prompt_version: "prompt-version".to_owned(),
                qa_validation_policy_version: "validation-v2".to_owned(),
                qa_policy_fingerprint: "policy-fingerprint".to_owned(),
                answer_match_policy_version: "squad-normalized-exact-match-v1".to_owned(),
                scoring_policy_version: "answerability-scoring-gate-v1".to_owned(),
                candidate_order_policy_version: "blind-order-v1".to_owned(),
                thread_count: 1,
                context_tokens: 4_096,
                max_input_tokens: 2_800,
                max_output_tokens: 384,
                temperature: 0.0,
                reasoning: "off".to_owned(),
            },
            startup_ms: 1,
            elapsed_ms: 2,
            training: split.clone(),
            calibration: split.clone(),
            total: split,
            passed: true,
        };

        let serialized = serde_json::to_string(&report).unwrap();

        assert!(serialized.contains("seed_rejection_gate"));
        assert!(serialized.contains("training"));
        assert!(serialized.contains("calibration"));
        for forbidden in [
            "sentinel private question",
            "/private/source/root",
            "selection_id",
            "need_id",
            "group_id",
            "candidate_id",
            "candidate_role",
            "passage",
            "reference_answer",
            "source_root",
            "data_root",
            "llama_server",
            "endpoint",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[test]
    fn fingerprint_covers_corpus_model_and_policy() {
        let mut identity = ProviderIdentity {
            model_catalog_id: "catalog-model".to_owned(),
            model_revision: "model-revision".to_owned(),
            model_artifact_sha256: "model-fingerprint".to_owned(),
            llama_cpp_build: "runtime-build".to_owned(),
            qa_profile_id: "qa-profile".to_owned(),
            qa_prompt_version: "prompt-version".to_owned(),
            qa_validation_policy_version: "validation-v2".to_owned(),
            qa_policy_fingerprint: "policy-one".to_owned(),
            answer_match_policy_version: "squad-normalized-exact-match-v1".to_owned(),
            scoring_policy_version: "answerability-scoring-gate-v1".to_owned(),
            candidate_order_policy_version: "blind-order-v1".to_owned(),
            thread_count: 1,
            context_tokens: 4_096,
            max_input_tokens: 2_800,
            max_output_tokens: 384,
            temperature: 0.0,
            reasoning: "off".to_owned(),
        };
        let first = candidate_fingerprint("manifest-one", &identity);

        assert_ne!(first, candidate_fingerprint("manifest-two", &identity));
        identity.qa_policy_fingerprint = "policy-two".to_owned();
        assert_ne!(first, candidate_fingerprint("manifest-one", &identity));
        identity.qa_policy_fingerprint = "policy-one".to_owned();
        identity.context_tokens += 1;
        assert_ne!(first, candidate_fingerprint("manifest-one", &identity));
        identity.context_tokens -= 1;
        identity.answer_match_policy_version = "squad-normalized-exact-match-v2".to_owned();
        assert_ne!(first, candidate_fingerprint("manifest-one", &identity));
        identity.answer_match_policy_version = "squad-normalized-exact-match-v1".to_owned();
        identity.scoring_policy_version = "answerability-scoring-gate-v2".to_owned();
        assert_ne!(first, candidate_fingerprint("manifest-one", &identity));
    }
}
