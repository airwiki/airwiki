use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use airwiki_core::{
    E5_MODEL_REVISION, EmbeddingProvider, EvidenceDecision, EvidenceRelevanceError,
    EvidenceRelevanceProvider, FastEmbedE5Small, PinnedE5Snapshot, RelevanceInput,
};
use airwiki_inference::{
    AssetManager, E5_REVISION, GenerationConfig, LLAMA_CPP_BUILD, LlamaClient, LlamaSupervisor,
    ModelProfile, ServerReasoningMode, SupervisorConfig, selection_for_model,
};
use airwiki_types::MAX_TOP_K;
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    EvaluationPhase, EvaluationProfile, EvaluationProviders, FixtureCase, FixtureChunk,
    LoadedFixture, NormalizedRun, ProviderIdentity, REPORT_DIRECTORY, RetrievalFixture,
    RetrievalSplit, build_corpus, load_fixture, percentile, qa_entailment,
    reviewed_anchor_selector, run_case, validate_fixture_data,
};
use crate::{replace_file, workspace_root};

const REVIEWED_ANCHOR_FIXTURE_PATH: &str = "fixtures/retrieval/reviewed-anchors-v1.json";
const REPORT_SCHEMA_VERSION: u32 = 1;
const FIXTURE_SCHEMA_VERSION: u32 = 1;
const ANCHOR_VALIDATION_POLICY_VERSION: &str = "reviewed-anchor-validation-v1";
const CONFLICT_POLICY_VERSION: &str = "reviewed-anchor-conflict-scan-v1";
const SCORING_POLICY_VERSION: &str = "reviewed-anchor-ablation-scoring-v1";
const CANDIDATE_ORDER_POLICY_VERSION: &str = "reviewed-anchor-candidate-order-v1";
const MAX_CLAIM_CHARS: usize = 1_200;
const MAX_ANCHOR_CHARS: usize = 1_200;
const MAX_NEEDS: usize = 8;
const MAX_SELECTOR_CANDIDATES: usize = 32;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewedAnchorFixture {
    schema_version: u32,
    profile_id: String,
    base_fixture_sha256: String,
    single_valued_relations: Vec<String>,
    claims: Vec<ReviewedClaim>,
    cases: Vec<ReviewedCase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewedClaim {
    fact_id: String,
    claim: String,
    anchor: String,
    key: ClaimKey,
    polarity: ClaimPolarity,
    value: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
struct ClaimKey {
    subject: String,
    relation: String,
    scope: String,
    temporal: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum ClaimPolarity {
    Affirmed,
    Denied,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewedCase {
    case_id: String,
    #[serde(default)]
    pre_model_excluded_fact_ids: Vec<String>,
    needs: Vec<ReviewedNeed>,
    expected_outcome: Decision,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewedNeed {
    id: String,
    text: String,
    expected_groups: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Decision {
    Accepted,
    Abstained,
    Conflict,
    Unavailable,
}

struct LoadedReviewedFixture {
    fixture: ReviewedAnchorFixture,
    sha256: String,
    anchored: LoadedFixture,
}

#[derive(Debug, Clone)]
struct RetrievedCandidate {
    fact_id: String,
    text: String,
    rank: u32,
}

#[derive(Debug, Clone)]
struct RetrievedNeed {
    need_id: String,
    candidates: Vec<RetrievedCandidate>,
}

#[derive(Debug, Clone)]
struct NeedSelection {
    need_id: String,
    fact_ids: Vec<String>,
}

#[derive(Debug, Clone)]
struct ArmObservation {
    decision: Decision,
    selections: Vec<NeedSelection>,
    model_calls: usize,
    elapsed_ms: u128,
    failed: bool,
}

#[derive(Debug, Clone, Serialize)]
struct CaseReport {
    id: String,
    split: RetrievalSplit,
    expected: Decision,
    observed: Decision,
    atomic_need_count: usize,
    covered_need_count: usize,
    selected_evidence_count: usize,
    supported_evidence_count: usize,
    forbidden_evidence_count: usize,
    candidate_required_group_count: usize,
    candidate_found_group_count: usize,
    selective_risk: bool,
    outcome_correct: bool,
    conflict_found: bool,
    model_calls: usize,
    elapsed_ms: u128,
    failed: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ArmReport {
    case_count: usize,
    accepted_count: usize,
    abstained_count: usize,
    conflict_count: usize,
    unavailable_count: usize,
    expected_positive_need_count: usize,
    covered_need_count: usize,
    atomic_need_recall: Option<f64>,
    candidate_required_group_count: usize,
    candidate_found_group_count: usize,
    effective_candidate_group_recall: Option<f64>,
    selected_evidence_count: usize,
    supported_evidence_count: usize,
    evidence_precision: Option<f64>,
    complete_coverage_count: usize,
    selective_risk_count: usize,
    forbidden_evidence_count: usize,
    expected_conflict_count: usize,
    conflict_found_count: usize,
    false_conflict_count: usize,
    outcome_correct_count: usize,
    failure_count: usize,
    model_call_count: usize,
    p50_ms: Option<u128>,
    p95_ms: Option<u128>,
    max_ms: Option<u128>,
    cases: Vec<CaseReport>,
}

#[derive(Debug, Serialize)]
struct EvaluationReport {
    schema_version: u32,
    evaluation_role: String,
    base_fixture_sha256: String,
    reviewed_anchor_fixture_sha256: String,
    candidate_fingerprint: String,
    target_os: String,
    target_arch: String,
    model_catalog_id: String,
    model_revision: String,
    model_artifact_sha256: String,
    llama_cpp_build: String,
    embedding_revision: String,
    anchor_profile_id: String,
    anchor_validation_policy_version: String,
    conflict_policy_version: String,
    scoring_policy_version: String,
    qa_policy_fingerprint: String,
    selector_policy_fingerprint: String,
    thread_count: usize,
    startup_ms: u128,
    elapsed_ms: u128,
    human_review_time_measured: bool,
    peak_memory_measured: bool,
    two_call_passages: ArmReport,
    reviewed_claims: ArmReport,
    reviewed_claims_with_conflicts: ArmReport,
    mechanism_promising: bool,
    production_promotion_ready: bool,
}

#[derive(Debug)]
struct AllRelevantProvider;

#[async_trait]
impl EvidenceRelevanceProvider for AllRelevantProvider {
    fn profile_id(&self) -> &str {
        "reviewed-anchor-candidate-pass-through-v1"
    }

    async fn classify(
        &self,
        _question: &str,
        candidates: &[RelevanceInput],
    ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
        Ok(vec![EvidenceDecision::Relevant; candidates.len()])
    }
}

#[derive(Debug)]
struct PrecomputedEmbeddingProvider {
    vectors: HashMap<String, Vec<f32>>,
}

#[async_trait]
impl EmbeddingProvider for PrecomputedEmbeddingProvider {
    fn model_id(&self) -> &str {
        "reviewed-anchor-precomputed-multilingual-e5-small"
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts
            .iter()
            .map(|text| {
                self.vectors
                    .get(text)
                    .cloned()
                    .context("reviewed anchor evaluation requested an unpinned embedding input")
            })
            .collect()
    }
}

pub(crate) async fn evaluate_reviewed_anchors(
    data_root: &Path,
    llama_server: &Path,
    model_id: &str,
) -> Result<()> {
    ensure!(
        E5_REVISION == E5_MODEL_REVISION,
        "airwiki-core and airwiki-inference require different embedding revisions"
    );
    let base = load_fixture()?;
    let reviewed = load_reviewed_fixture(&base)?;
    let selection = selection_for_model(
        ModelProfile::Automatic,
        model_id,
        "reviewed anchor development evaluation",
    )
    .context("reviewed anchor model is not in the pinned AirWiki catalog")?;
    let assets = AssetManager::new(data_root)?
        .with_bundled_runtime(Some(llama_server.to_path_buf()))
        .verify_selection(&selection)
        .await
        .context("reviewed anchor assets failed verification")?;
    eprintln!("reviewed anchors: verified local assets");
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 4);
    eprintln!("reviewed anchors: preparing one bounded embedding batch");
    let embeddings = precompute_embeddings(
        &base.fixture,
        &reviewed,
        &PinnedE5Snapshot::open(&assets.embedding_snapshot_path)?,
        threads,
    )
    .await?;
    eprintln!("reviewed anchors: prepared embeddings");
    let providers = EvaluationProviders {
        embeddings,
        relevance: Arc::new(AllRelevantProvider),
        profile: EvaluationProfile::Current,
        identity: ProviderIdentity {
            embedding_profile: format!("multilingual-e5-small@{E5_REVISION}"),
            embedding_revision: E5_REVISION.to_owned(),
            relevance_profile: "reviewed-anchor-candidate-pass-through-v1".to_owned(),
            relevance_revision: ANCHOR_VALIDATION_POLICY_VERSION.to_owned(),
            relevance_artifact_filename: None,
            relevance_artifact_sha256: None,
            thread_count: threads,
        },
        telemetry: None,
        startup_ms: None,
    };

    let development_domains = base
        .fixture
        .cases
        .iter()
        .filter(|case| EvaluationPhase::Development.includes(case.split))
        .filter_map(|case| case.domain.as_deref())
        .collect::<BTreeSet<_>>();
    let raw_corpus =
        build_corpus(&base.fixture, &providers, false, Some(&development_domains)).await?;
    let anchor_corpus = build_corpus(
        &reviewed.anchored.fixture,
        &providers,
        false,
        Some(&development_domains),
    )
    .await?;
    eprintln!("reviewed anchors: built isolated raw and claim corpora");

    let mut supervisor_config =
        SupervisorConfig::bundled(assets.llama_server_path, assets.model_path);
    supervisor_config.model_id = assets.generation_settings.model_api_id.to_owned();
    supervisor_config.context_tokens = assets.generation_settings.context_tokens;
    supervisor_config.threads = threads;
    supervisor_config.reasoning_mode = ServerReasoningMode::Off;
    supervisor_config.idle_timeout = Duration::from_secs(20 * 60);
    let supervisor = LlamaSupervisor::new(supervisor_config);
    let startup_started = Instant::now();
    let endpoint = supervisor
        .ensure_running()
        .await
        .context("reviewed anchor runtime did not become ready")?;
    let startup_ms = startup_started.elapsed().as_millis();
    eprintln!("reviewed anchors: local generation runtime ready");

    let mut generation = GenerationConfig::from_settings(assets.generation_settings);
    generation.temperature = 0.0;
    generation.timeout = Duration::from_secs(35);
    let client = LlamaClient::new(endpoint, generation)?;
    let qa = qa_entailment::QaEntailmentEvaluator::new(client.clone());
    let selector = reviewed_anchor_selector::ReviewedAnchorSelector::new(client);
    let started = Instant::now();
    let report_result = run_ablation(
        &base,
        &reviewed,
        &raw_corpus,
        &anchor_corpus,
        &qa,
        &selector,
        assets.selection.model_id,
        assets.selection.manifest.artifact.revision,
        assets.selection.manifest.artifact.sha256,
        threads,
        startup_ms,
        started,
    )
    .await;
    let stop_result = supervisor.stop().await;
    let report = report_result?;
    let destination = write_report(&report)?;
    ensure!(
        stop_result.is_ok(),
        "reviewed anchor runtime did not stop cleanly; report written to {}",
        destination.display()
    );
    ensure!(
        report.mechanism_promising,
        "reviewed anchor mechanism did not meet the development gate; report written to {}",
        destination.display()
    );
    println!(
        "reviewed anchor mechanism passed its development gate; report written to {}",
        destination.display()
    );
    Ok(())
}

async fn precompute_embeddings(
    base: &RetrievalFixture,
    reviewed: &LoadedReviewedFixture,
    snapshot: &PinnedE5Snapshot,
    threads: usize,
) -> Result<Arc<dyn EmbeddingProvider>> {
    let mut inputs = BTreeSet::new();
    inputs.extend(
        base.documents
            .iter()
            .flat_map(|document| &document.chunks)
            .map(|chunk| format!("passage: {}", chunk.text)),
    );
    inputs.extend(
        reviewed
            .fixture
            .claims
            .iter()
            .map(|claim| format!("passage: {}", claim.claim)),
    );
    inputs.extend(
        reviewed
            .fixture
            .cases
            .iter()
            .flat_map(|case| &case.needs)
            .map(|need| format!("query: {}", need.text.trim())),
    );
    let inputs = inputs.into_iter().collect::<Vec<_>>();
    let model = FastEmbedE5Small::from_snapshot(snapshot, threads)?;
    let vectors = model.embed(&inputs).await?;
    ensure!(
        vectors.len() == inputs.len(),
        "embedding model returned the wrong reviewed-anchor batch size"
    );
    Ok(Arc::new(PrecomputedEmbeddingProvider {
        vectors: inputs.into_iter().zip(vectors).collect(),
    }))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the ablation identity and both frozen candidate arms stay explicit"
)]
async fn run_ablation(
    base: &LoadedFixture,
    reviewed: &LoadedReviewedFixture,
    raw_corpus: &super::FixtureCorpus,
    anchor_corpus: &super::FixtureCorpus,
    qa: &qa_entailment::QaEntailmentEvaluator,
    selector: &reviewed_anchor_selector::ReviewedAnchorSelector,
    model_catalog_id: &str,
    model_revision: &str,
    model_artifact_sha256: &str,
    threads: usize,
    startup_ms: u128,
    started: Instant,
) -> Result<EvaluationReport> {
    let raw_texts = fact_texts(&base.fixture);
    let claim_texts = reviewed
        .fixture
        .claims
        .iter()
        .map(|claim| (claim.fact_id.as_str(), claim.claim.as_str()))
        .collect::<HashMap<_, _>>();
    let claims = reviewed
        .fixture
        .claims
        .iter()
        .map(|claim| (claim.fact_id.as_str(), claim))
        .collect::<HashMap<_, _>>();
    let base_cases = base
        .fixture
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect::<HashMap<_, _>>();

    let mut reports_a = Vec::new();
    let mut reports_b = Vec::new();
    let mut reports_c = Vec::new();
    let development_case_count = reviewed
        .fixture
        .cases
        .iter()
        .filter(|reviewed_case| {
            base_cases
                .get(reviewed_case.case_id.as_str())
                .is_some_and(|case| EvaluationPhase::Development.includes(case.split))
        })
        .count();
    for (case_index, reviewed_case) in reviewed
        .fixture
        .cases
        .iter()
        .filter(|reviewed_case| {
            base_cases
                .get(reviewed_case.case_id.as_str())
                .is_some_and(|case| EvaluationPhase::Development.includes(case.split))
        })
        .enumerate()
    {
        eprintln!(
            "reviewed anchors: evaluating synthetic case {}/{}",
            case_index + 1,
            development_case_count
        );
        let base_case = base_cases
            .get(reviewed_case.case_id.as_str())
            .copied()
            .context("reviewed anchor case has no base retrieval case")?;
        let raw_retrieved =
            retrieve_needs(raw_corpus, base_case, reviewed_case, &raw_texts).await?;
        let claim_retrieved =
            retrieve_needs(anchor_corpus, base_case, reviewed_case, &claim_texts).await?;
        let claim_pool = union_candidates(&claim_retrieved);

        let observed_a = observe_two_call(qa, reviewed_case, &raw_retrieved).await;
        let observed_b = observe_reviewed_claims(selector, reviewed_case, &claim_pool).await;
        let observed_c = add_conflict_scan(
            &observed_b,
            &claim_retrieved,
            &claims,
            &reviewed.fixture.single_valued_relations,
        );
        reports_a.push(score_case(
            base_case,
            reviewed_case,
            &raw_retrieved,
            None,
            observed_a,
        ));
        reports_b.push(score_case(
            base_case,
            reviewed_case,
            &claim_retrieved,
            Some(&claim_pool),
            observed_b,
        ));
        reports_c.push(score_case(
            base_case,
            reviewed_case,
            &claim_retrieved,
            Some(&claim_pool),
            observed_c,
        ));
    }

    let two_call_passages = aggregate(reports_a);
    let reviewed_claims = aggregate(reports_b);
    let reviewed_claims_with_conflicts = aggregate(reports_c);
    let mechanism_promising =
        mechanism_is_promising(&two_call_passages, &reviewed_claims_with_conflicts);
    let candidate_fingerprint = candidate_fingerprint(
        &base.sha256,
        &reviewed.sha256,
        model_catalog_id,
        model_revision,
        model_artifact_sha256,
        threads,
    );

    Ok(EvaluationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        evaluation_role: "development_mechanism_ablation".to_owned(),
        base_fixture_sha256: base.sha256.clone(),
        reviewed_anchor_fixture_sha256: reviewed.sha256.clone(),
        candidate_fingerprint,
        target_os: std::env::consts::OS.to_owned(),
        target_arch: std::env::consts::ARCH.to_owned(),
        model_catalog_id: model_catalog_id.to_owned(),
        model_revision: model_revision.to_owned(),
        model_artifact_sha256: model_artifact_sha256.to_owned(),
        llama_cpp_build: LLAMA_CPP_BUILD.to_owned(),
        embedding_revision: E5_REVISION.to_owned(),
        anchor_profile_id: reviewed.fixture.profile_id.clone(),
        anchor_validation_policy_version: ANCHOR_VALIDATION_POLICY_VERSION.to_owned(),
        conflict_policy_version: CONFLICT_POLICY_VERSION.to_owned(),
        scoring_policy_version: SCORING_POLICY_VERSION.to_owned(),
        qa_policy_fingerprint: qa_entailment::policy_fingerprint(),
        selector_policy_fingerprint: reviewed_anchor_selector::policy_fingerprint(),
        thread_count: threads,
        startup_ms,
        elapsed_ms: started.elapsed().as_millis(),
        human_review_time_measured: false,
        peak_memory_measured: false,
        two_call_passages,
        reviewed_claims,
        reviewed_claims_with_conflicts,
        mechanism_promising,
        production_promotion_ready: false,
    })
}

fn mechanism_is_promising(baseline: &ArmReport, candidate: &ArmReport) -> bool {
    candidate.case_count > 0
        && candidate.forbidden_evidence_count == 0
        && candidate.selective_risk_count == 0
        && candidate.failure_count == 0
        && candidate.false_conflict_count == 0
        && candidate.conflict_found_count == candidate.expected_conflict_count
        && candidate.outcome_correct_count == candidate.case_count
        && candidate.candidate_found_group_count == candidate.candidate_required_group_count
        && candidate.complete_coverage_count >= baseline.complete_coverage_count
        && candidate.model_call_count <= candidate.case_count
}

async fn retrieve_needs(
    corpus: &super::FixtureCorpus,
    base_case: &FixtureCase,
    reviewed_case: &ReviewedCase,
    texts: &HashMap<&str, &str>,
) -> Result<Vec<RetrievedNeed>> {
    let mut retrieved = Vec::with_capacity(reviewed_case.needs.len());
    for need in &reviewed_case.needs {
        let mut atomic_case = base_case.clone();
        atomic_case.question.clone_from(&need.text);
        let run = run_case(corpus, &atomic_case, MAX_TOP_K).await?;
        validate_pre_model_boundary(&run, reviewed_case)?;
        retrieved.push(RetrievedNeed {
            need_id: need.id.clone(),
            candidates: normalize_candidates(run, texts)?,
        });
    }
    Ok(retrieved)
}

fn validate_pre_model_boundary(run: &NormalizedRun, case: &ReviewedCase) -> Result<()> {
    ensure!(
        run.provenance_errors == 0,
        "retrieval returned invalid provenance before reviewed-anchor selection"
    );
    let forbidden = case
        .pre_model_excluded_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let returned_forbidden = run
        .sources
        .iter()
        .flat_map(|source| &source.hits)
        .any(|hit| forbidden.contains(hit.fact_id.as_str()));
    ensure!(
        !returned_forbidden,
        "retrieval exposed forbidden evidence before reviewed-anchor selection"
    );
    Ok(())
}

fn normalize_candidates(
    run: NormalizedRun,
    texts: &HashMap<&str, &str>,
) -> Result<Vec<RetrievedCandidate>> {
    let mut candidates = run
        .sources
        .into_iter()
        .flat_map(|source| {
            source
                .hits
                .into_iter()
                .map(move |hit| (hit.rank, source.node, hit.fact_id))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|(_, _, fact_id)| seen.insert(fact_id.clone()))
        .map(|(rank, _, fact_id)| {
            let text = texts
                .get(fact_id.as_str())
                .copied()
                .context("retrieval result has no fixture text")?;
            Ok(RetrievedCandidate {
                fact_id,
                text: text.to_owned(),
                rank,
            })
        })
        .collect()
}

async fn observe_two_call(
    evaluator: &qa_entailment::QaEntailmentEvaluator,
    case: &ReviewedCase,
    retrieved: &[RetrievedNeed],
) -> ArmObservation {
    let started = Instant::now();
    let mut selections = Vec::with_capacity(case.needs.len());
    let mut model_calls = 0;
    for (need, candidates) in case.needs.iter().zip(retrieved) {
        if candidates.candidates.is_empty() {
            return ArmObservation {
                decision: Decision::Abstained,
                selections,
                model_calls,
                elapsed_ms: started.elapsed().as_millis(),
                failed: false,
            };
        }
        let opaque = candidates
            .candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| (format!("c{index}"), candidate))
            .collect::<Vec<_>>();
        let input = qa_entailment::QaEntailmentInput::new(
            qa_entailment::QaNeedKind::Question,
            &need.text,
            opaque
                .iter()
                .map(|(id, candidate)| qa_entailment::QaCandidateInput::new(id, &candidate.text))
                .collect(),
        );
        match evaluator.evaluate(&input).await {
            Ok(outcome) => {
                model_calls += 1 + usize::from(outcome.verifier_called);
                if !outcome.accepted {
                    return ArmObservation {
                        decision: Decision::Abstained,
                        selections: Vec::new(),
                        model_calls,
                        elapsed_ms: started.elapsed().as_millis(),
                        failed: false,
                    };
                }
                let selected = outcome
                    .selected_candidate_ids
                    .iter()
                    .filter_map(|id| {
                        opaque
                            .iter()
                            .find(|(opaque_id, _)| opaque_id == id)
                            .map(|(_, candidate)| candidate.fact_id.clone())
                    })
                    .collect();
                selections.push(NeedSelection {
                    need_id: need.id.clone(),
                    fact_ids: selected,
                });
            }
            Err(error) => {
                model_calls += match error.stage() {
                    qa_entailment::QaEntailmentStage::Proposal => 1,
                    qa_entailment::QaEntailmentStage::Verification => 2,
                };
                return ArmObservation {
                    decision: Decision::Unavailable,
                    selections,
                    model_calls,
                    elapsed_ms: started.elapsed().as_millis(),
                    failed: true,
                };
            }
        }
    }
    ArmObservation {
        decision: Decision::Accepted,
        selections,
        model_calls,
        elapsed_ms: started.elapsed().as_millis(),
        failed: false,
    }
}

async fn observe_reviewed_claims(
    selector: &reviewed_anchor_selector::ReviewedAnchorSelector,
    case: &ReviewedCase,
    candidates: &[RetrievedCandidate],
) -> ArmObservation {
    let started = Instant::now();
    let opaque = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (format!("c{index}"), candidate))
        .collect::<Vec<_>>();
    let input = reviewed_anchor_selector::ReviewedAnchorSelectorInput::new(
        case.needs
            .iter()
            .map(|need| {
                reviewed_anchor_selector::ReviewedAnchorNeedInput::new(&need.id, &need.text)
            })
            .collect(),
        opaque
            .iter()
            .map(|(id, candidate)| {
                reviewed_anchor_selector::ReviewedAnchorClaimInput::new(id, &candidate.text)
            })
            .collect(),
    );
    match selector.select(&input).await {
        Ok(outcome) => {
            let selections = if outcome.accepted {
                outcome
                    .assignments
                    .into_iter()
                    .map(|assignment| NeedSelection {
                        need_id: assignment.need_id,
                        fact_ids: assignment
                            .claim_ids
                            .iter()
                            .filter_map(|id| {
                                opaque
                                    .iter()
                                    .find(|(opaque_id, _)| opaque_id == id)
                                    .map(|(_, candidate)| candidate.fact_id.clone())
                            })
                            .collect(),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            ArmObservation {
                decision: if outcome.accepted {
                    Decision::Accepted
                } else {
                    Decision::Abstained
                },
                selections,
                model_calls: usize::from(outcome.call_elapsed.is_some()),
                elapsed_ms: started.elapsed().as_millis(),
                failed: false,
            }
        }
        Err(error) => {
            let _sanitized_failure = (error.kind(), error.invalid_reason(), error.call_elapsed());
            ArmObservation {
                decision: Decision::Unavailable,
                selections: Vec::new(),
                model_calls: usize::from(error.call_elapsed().is_some()),
                elapsed_ms: started.elapsed().as_millis(),
                failed: true,
            }
        }
    }
}

fn union_candidates(retrieved: &[RetrievedNeed]) -> Vec<RetrievedCandidate> {
    let mut best = BTreeMap::<&str, &RetrievedCandidate>::new();
    for candidate in retrieved.iter().flat_map(|need| &need.candidates) {
        best.entry(candidate.fact_id.as_str())
            .and_modify(|existing| {
                if (candidate.rank, candidate.fact_id.as_str())
                    < (existing.rank, existing.fact_id.as_str())
                {
                    *existing = candidate;
                }
            })
            .or_insert(candidate);
    }
    let mut candidates = best.into_values().cloned().collect::<Vec<_>>();
    candidates.sort_unstable_by(|left, right| {
        (left.rank, left.fact_id.as_str()).cmp(&(right.rank, right.fact_id.as_str()))
    });
    candidates.truncate(MAX_SELECTOR_CANDIDATES);
    candidates
}

fn add_conflict_scan(
    baseline: &ArmObservation,
    retrieved: &[RetrievedNeed],
    claims: &HashMap<&str, &ReviewedClaim>,
    single_valued_relations: &[String],
) -> ArmObservation {
    let started = Instant::now();
    if baseline.decision != Decision::Accepted {
        return baseline.clone();
    }
    let mut result = baseline.clone();
    let candidates_by_need = retrieved
        .iter()
        .map(|need| (need.need_id.as_str(), &need.candidates))
        .collect::<HashMap<_, _>>();
    let mut found_conflict = false;
    for selection in &mut result.selections {
        let Some(candidates) = candidates_by_need.get(selection.need_id.as_str()) else {
            result.decision = Decision::Unavailable;
            result.failed = true;
            return result;
        };
        let selected_claims = selection
            .fact_ids
            .iter()
            .filter_map(|fact_id| claims.get(fact_id.as_str()).copied())
            .collect::<Vec<_>>();
        let mut conflicts = BTreeSet::new();
        for candidate in candidates.iter().filter_map(|candidate| {
            claims
                .get(candidate.fact_id.as_str())
                .map(|claim| (candidate, *claim))
        }) {
            if selected_claims
                .iter()
                .any(|selected| claims_conflict(selected, candidate.1, single_valued_relations))
            {
                conflicts.insert(candidate.0.fact_id.clone());
            }
        }
        if !conflicts.is_empty() {
            found_conflict = true;
            selection.fact_ids.extend(conflicts);
            selection.fact_ids.sort();
            selection.fact_ids.dedup();
        }
    }
    if found_conflict {
        result.decision = Decision::Conflict;
    }
    result.elapsed_ms = result
        .elapsed_ms
        .saturating_add(started.elapsed().as_millis());
    result
}

fn claims_conflict(
    left: &ReviewedClaim,
    right: &ReviewedClaim,
    single_valued_relations: &[String],
) -> bool {
    if left.key != right.key {
        return false;
    }
    match (left.polarity, right.polarity) {
        (ClaimPolarity::Affirmed, ClaimPolarity::Affirmed) => {
            left.value != right.value
                && single_valued_relations
                    .iter()
                    .any(|relation| relation == &left.key.relation)
        }
        (ClaimPolarity::Affirmed, ClaimPolarity::Denied)
        | (ClaimPolarity::Denied, ClaimPolarity::Affirmed) => left.value == right.value,
        (ClaimPolarity::Denied, ClaimPolarity::Denied) => false,
    }
}

fn score_case(
    base_case: &FixtureCase,
    reviewed_case: &ReviewedCase,
    retrieved: &[RetrievedNeed],
    effective_pool: Option<&[RetrievedCandidate]>,
    observed: ArmObservation,
) -> CaseReport {
    let selections = if matches!(observed.decision, Decision::Accepted | Decision::Conflict) {
        observed
            .selections
            .iter()
            .map(|selection| {
                (
                    selection.need_id.as_str(),
                    selection
                        .fact_ids
                        .iter()
                        .map(String::as_str)
                        .collect::<HashSet<_>>(),
                )
            })
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    };
    let forbidden = base_case
        .forbidden_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut covered_need_count = 0;
    let mut selected_evidence = HashSet::new();
    let mut supported_evidence = HashSet::new();
    let mut forbidden_evidence = HashSet::new();
    let mut candidate_required_group_count = 0;
    let mut candidate_found_group_count = 0;
    for (need, retrieved_need) in reviewed_case.needs.iter().zip(retrieved) {
        let selected = selections.get(need.id.as_str());
        let retrieved_ids = effective_pool
            .unwrap_or(&retrieved_need.candidates)
            .iter()
            .map(|candidate| candidate.fact_id.as_str())
            .collect::<HashSet<_>>();
        candidate_required_group_count += need.expected_groups.len();
        candidate_found_group_count += need
            .expected_groups
            .iter()
            .filter(|group| {
                group
                    .iter()
                    .any(|fact_id| retrieved_ids.contains(fact_id.as_str()))
            })
            .count();
        let expected = need
            .expected_groups
            .iter()
            .flatten()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let complete = !need.expected_groups.is_empty()
            && need.expected_groups.iter().all(|group| {
                selected.is_some_and(|selected| {
                    group
                        .iter()
                        .any(|fact_id| selected.contains(fact_id.as_str()))
                })
            });
        covered_need_count += usize::from(complete);
        if let Some(selected) = selected {
            for fact_id in selected {
                selected_evidence.insert(*fact_id);
                if expected.contains(fact_id) {
                    supported_evidence.insert(*fact_id);
                }
                if forbidden.contains(fact_id) {
                    forbidden_evidence.insert(*fact_id);
                }
            }
        }
    }
    let positive_need_count = reviewed_case
        .needs
        .iter()
        .filter(|need| !need.expected_groups.is_empty())
        .count();
    let complete_coverage = covered_need_count == positive_need_count && positive_need_count > 0;
    let unsupported = selected_evidence
        .len()
        .saturating_sub(supported_evidence.len());
    let selective_risk = observed.decision == Decision::Accepted
        && (reviewed_case.expected_outcome != Decision::Accepted
            || !complete_coverage
            || unsupported > 0
            || !forbidden_evidence.is_empty());
    let outcome_correct = observed.decision == reviewed_case.expected_outcome
        && match observed.decision {
            Decision::Accepted | Decision::Conflict => {
                complete_coverage && unsupported == 0 && forbidden_evidence.is_empty()
            }
            Decision::Abstained => selected_evidence.is_empty(),
            Decision::Unavailable => false,
        };
    CaseReport {
        id: reviewed_case.case_id.clone(),
        split: base_case.split,
        expected: reviewed_case.expected_outcome,
        observed: observed.decision,
        atomic_need_count: positive_need_count,
        covered_need_count,
        selected_evidence_count: selected_evidence.len(),
        supported_evidence_count: supported_evidence.len(),
        forbidden_evidence_count: forbidden_evidence.len(),
        candidate_required_group_count,
        candidate_found_group_count,
        selective_risk,
        outcome_correct,
        conflict_found: observed.decision == Decision::Conflict,
        model_calls: observed.model_calls,
        elapsed_ms: observed.elapsed_ms,
        failed: observed.failed,
    }
}

fn aggregate(cases: Vec<CaseReport>) -> ArmReport {
    let mut report = ArmReport::default();
    let mut elapsed = Vec::with_capacity(cases.len());
    for case in &cases {
        report.case_count += 1;
        match case.observed {
            Decision::Accepted => report.accepted_count += 1,
            Decision::Abstained => report.abstained_count += 1,
            Decision::Conflict => report.conflict_count += 1,
            Decision::Unavailable => report.unavailable_count += 1,
        }
        report.expected_positive_need_count += case.atomic_need_count;
        report.covered_need_count += case.covered_need_count;
        report.candidate_required_group_count += case.candidate_required_group_count;
        report.candidate_found_group_count += case.candidate_found_group_count;
        report.selected_evidence_count += case.selected_evidence_count;
        report.supported_evidence_count += case.supported_evidence_count;
        report.complete_coverage_count += usize::from(
            case.atomic_need_count > 0 && case.covered_need_count == case.atomic_need_count,
        );
        report.selective_risk_count += usize::from(case.selective_risk);
        report.forbidden_evidence_count += case.forbidden_evidence_count;
        report.expected_conflict_count += usize::from(case.expected == Decision::Conflict);
        report.conflict_found_count +=
            usize::from(case.expected == Decision::Conflict && case.conflict_found);
        report.false_conflict_count +=
            usize::from(case.expected != Decision::Conflict && case.conflict_found);
        report.outcome_correct_count += usize::from(case.outcome_correct);
        report.failure_count += usize::from(case.failed);
        report.model_call_count += case.model_calls;
        elapsed.push(case.elapsed_ms);
    }
    elapsed.sort_unstable();
    report.atomic_need_recall = (report.expected_positive_need_count > 0)
        .then(|| report.covered_need_count as f64 / report.expected_positive_need_count as f64);
    report.effective_candidate_group_recall =
        (report.candidate_required_group_count > 0).then(|| {
            report.candidate_found_group_count as f64 / report.candidate_required_group_count as f64
        });
    report.evidence_precision = (report.selected_evidence_count > 0)
        .then(|| report.supported_evidence_count as f64 / report.selected_evidence_count as f64);
    report.p50_ms = percentile(&elapsed, 50);
    report.p95_ms = percentile(&elapsed, 95);
    report.max_ms = elapsed.last().copied();
    report.cases = cases;
    report
}

fn load_reviewed_fixture(base: &LoadedFixture) -> Result<LoadedReviewedFixture> {
    let path = workspace_root().join(REVIEWED_ANCHOR_FIXTURE_PATH);
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let fixture: ReviewedAnchorFixture =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    validate_reviewed_fixture(&fixture, base)?;
    let mut anchored_fixture = base.fixture.clone();
    let claims = fixture
        .claims
        .iter()
        .map(|claim| (claim.fact_id.as_str(), claim.claim.as_str()))
        .collect::<HashMap<_, _>>();
    for chunk in anchored_fixture
        .documents
        .iter_mut()
        .flat_map(|document| &mut document.chunks)
    {
        chunk.text = claims
            .get(chunk.id.as_str())
            .copied()
            .context("reviewed anchor fixture is missing a claim")?
            .to_owned();
    }
    validate_fixture_data(&anchored_fixture)?;
    let anchored = LoadedFixture {
        fixture: anchored_fixture,
        sha256: synthetic_fixture_sha(&base.sha256, &bytes),
    };
    Ok(LoadedReviewedFixture {
        fixture,
        sha256: hex::encode(Sha256::digest(&bytes)),
        anchored,
    })
}

fn validate_reviewed_fixture(fixture: &ReviewedAnchorFixture, base: &LoadedFixture) -> Result<()> {
    ensure!(
        fixture.schema_version == FIXTURE_SCHEMA_VERSION,
        "unsupported reviewed anchor fixture schema"
    );
    validate_id(&fixture.profile_id, "profile")?;
    ensure!(
        fixture.base_fixture_sha256 == base.sha256,
        "reviewed anchor fixture pins a different base retrieval fixture"
    );
    let single_valued_relations = fixture
        .single_valued_relations
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    ensure!(
        single_valued_relations.len() == fixture.single_valued_relations.len(),
        "reviewed anchor fixture has duplicate single-valued relations"
    );
    for relation in &fixture.single_valued_relations {
        validate_id(relation, "single-valued relation")?;
    }
    let chunks = base
        .fixture
        .documents
        .iter()
        .flat_map(|document| &document.chunks)
        .map(|chunk| (chunk.id.as_str(), chunk))
        .collect::<HashMap<_, _>>();
    let mut claim_ids = BTreeSet::new();
    for claim in &fixture.claims {
        validate_id(&claim.fact_id, "claim fact")?;
        ensure!(
            claim_ids.insert(claim.fact_id.as_str()),
            "reviewed anchor fixture has duplicate claims"
        );
        let chunk = chunks
            .get(claim.fact_id.as_str())
            .context("reviewed claim references an unknown fact")?;
        validate_text(&claim.claim, MAX_CLAIM_CHARS, "reviewed claim")?;
        validate_text(&claim.anchor, MAX_ANCHOR_CHARS, "reviewed anchor")?;
        ensure!(
            chunk.text.contains(&claim.anchor),
            "reviewed anchor is not a literal substring of its source fact"
        );
        validate_key(&claim.key)?;
        validate_id(&claim.value, "claim value")?;
    }
    ensure!(
        claim_ids == chunks.keys().copied().collect(),
        "reviewed anchor fixture must bind every synthetic fact exactly once"
    );
    let claim_relations = fixture
        .claims
        .iter()
        .map(|claim| claim.key.relation.as_str())
        .collect::<BTreeSet<_>>();
    ensure!(
        single_valued_relations.is_subset(&claim_relations),
        "reviewed anchor fixture declares an unused single-valued relation"
    );
    let base_cases = base
        .fixture
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect::<HashMap<_, _>>();
    let mut case_ids = BTreeSet::new();
    for case in &fixture.cases {
        validate_id(&case.case_id, "reviewed case")?;
        ensure!(
            case_ids.insert(case.case_id.as_str()),
            "reviewed anchor fixture has duplicate cases"
        );
        let base_case = base_cases
            .get(case.case_id.as_str())
            .context("reviewed anchor case references an unknown base case")?;
        ensure!(
            !case.needs.is_empty() && case.needs.len() <= MAX_NEEDS,
            "reviewed anchor case has an invalid atomic need count"
        );
        let mut need_ids = BTreeSet::new();
        let mut expected = BTreeSet::new();
        for need in &case.needs {
            validate_id(&need.id, "atomic need")?;
            ensure!(
                need_ids.insert(need.id.as_str()),
                "reviewed anchor case has duplicate atomic needs"
            );
            validate_text(&need.text, 4_096, "atomic need")?;
            for group in &need.expected_groups {
                ensure!(!group.is_empty(), "reviewed anchor expected group is empty");
                let mut group_ids = BTreeSet::new();
                for fact_id in group {
                    ensure!(
                        chunks.contains_key(fact_id.as_str()),
                        "reviewed anchor expected group references an unknown fact"
                    );
                    ensure!(
                        group_ids.insert(fact_id.as_str()),
                        "reviewed anchor expected group contains duplicates"
                    );
                    expected.insert(fact_id.as_str());
                }
            }
        }
        let base_expected = base_case
            .expected_groups
            .iter()
            .flatten()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        ensure!(
            expected == base_expected,
            "reviewed atomic needs disagree with the base case evidence"
        );
        let pre_model_excluded = case
            .pre_model_excluded_fact_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        ensure!(
            pre_model_excluded.len() == case.pre_model_excluded_fact_ids.len(),
            "reviewed anchor case has duplicate pre-model exclusions"
        );
        let base_forbidden = base_case
            .forbidden_fact_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        ensure!(
            pre_model_excluded.is_subset(&base_forbidden),
            "reviewed anchor pre-model exclusion is not a forbidden base fact"
        );
        let expected_outcome = if base_case.expected_groups.is_empty() {
            Decision::Abstained
        } else if base_case.tags.contains(&super::RetrievalTag::Conflict) {
            Decision::Conflict
        } else {
            Decision::Accepted
        };
        ensure!(
            case.expected_outcome == expected_outcome,
            "reviewed case has an inconsistent expected outcome"
        );
    }
    ensure!(
        case_ids == base_cases.keys().copied().collect(),
        "reviewed anchor fixture must cover every base case exactly once"
    );
    Ok(())
}

fn validate_key(key: &ClaimKey) -> Result<()> {
    validate_id(&key.subject, "claim subject")?;
    validate_id(&key.relation, "claim relation")?;
    validate_id(&key.scope, "claim scope")?;
    validate_id(&key.temporal, "claim temporal scope")
}

fn validate_id(value: &str, kind: &str) -> Result<()> {
    ensure!(!value.is_empty(), "reviewed anchor {kind} id is empty");
    ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "reviewed anchor {kind} id must use lowercase ASCII, digits and underscores"
    );
    Ok(())
}

fn validate_text(value: &str, max_chars: usize, kind: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{kind} is empty");
    ensure!(value.chars().count() <= max_chars, "{kind} is too long");
    Ok(())
}

fn fact_texts(fixture: &RetrievalFixture) -> HashMap<&str, &str> {
    fixture
        .documents
        .iter()
        .flat_map(|document| &document.chunks)
        .map(|chunk: &FixtureChunk| (chunk.id.as_str(), chunk.text.as_str()))
        .collect()
}

fn synthetic_fixture_sha(base_sha: &str, overlay: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(base_sha.as_bytes());
    hasher.update([0]);
    hasher.update(overlay);
    hex::encode(hasher.finalize())
}

fn candidate_fingerprint(
    base_sha: &str,
    reviewed_sha: &str,
    model_id: &str,
    model_revision: &str,
    model_sha: &str,
    threads: usize,
) -> String {
    let mut hasher = Sha256::new();
    for value in [
        base_sha,
        reviewed_sha,
        model_id,
        model_revision,
        model_sha,
        LLAMA_CPP_BUILD,
        E5_REVISION,
        ANCHOR_VALIDATION_POLICY_VERSION,
        CONFLICT_POLICY_VERSION,
        SCORING_POLICY_VERSION,
        CANDIDATE_ORDER_POLICY_VERSION,
        &qa_entailment::policy_fingerprint(),
        &reviewed_anchor_selector::policy_fingerprint(),
    ] {
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    hasher.update(threads.to_le_bytes());
    hex::encode(hasher.finalize())
}

fn report_path() -> PathBuf {
    workspace_root().join(REPORT_DIRECTORY).join(format!(
        "retrieval-reviewed-anchors-development-{}-{}.json",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

fn write_report(report: &EvaluationReport) -> Result<PathBuf> {
    let destination = report_path();
    let parent = destination
        .parent()
        .context("reviewed anchor report has no parent")?;
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

    #[test]
    fn versioned_fixture_binds_literal_anchors_and_base_cases() {
        let base = load_fixture().unwrap();

        let reviewed = load_reviewed_fixture(&base).unwrap();

        assert_eq!(reviewed.fixture.claims.len(), 27);
        assert_eq!(reviewed.fixture.cases.len(), 17);
    }

    #[test]
    fn conflict_scan_finds_material_value_disagreement() {
        let claim_a = ReviewedClaim {
            fact_id: "fact_a".to_owned(),
            claim: "status blue".to_owned(),
            anchor: "status blue".to_owned(),
            key: key("current"),
            polarity: ClaimPolarity::Affirmed,
            value: "blue".to_owned(),
        };
        let claim_b = ReviewedClaim {
            fact_id: "fact_b".to_owned(),
            claim: "status amber".to_owned(),
            anchor: "status amber".to_owned(),
            key: key("current"),
            polarity: ClaimPolarity::Affirmed,
            value: "amber".to_owned(),
        };
        let claims = HashMap::from([("fact_a", &claim_a), ("fact_b", &claim_b)]);
        let baseline = accepted("fact_a");
        let retrieved = retrieved(&["fact_a", "fact_b"]);

        let result = add_conflict_scan(&baseline, &retrieved, &claims, &single_value_status());

        assert_eq!(result.decision, Decision::Conflict);
        assert_eq!(result.selections[0].fact_ids, ["fact_a", "fact_b"]);
    }

    #[test]
    fn conflict_scan_ignores_a_different_scope() {
        let claim_a = ReviewedClaim {
            fact_id: "fact_a".to_owned(),
            claim: "current blue".to_owned(),
            anchor: "current blue".to_owned(),
            key: key("current"),
            polarity: ClaimPolarity::Affirmed,
            value: "blue".to_owned(),
        };
        let claim_b = ReviewedClaim {
            fact_id: "fact_b".to_owned(),
            claim: "historic amber".to_owned(),
            anchor: "historic amber".to_owned(),
            key: key("historic"),
            polarity: ClaimPolarity::Affirmed,
            value: "amber".to_owned(),
        };
        let claims = HashMap::from([("fact_a", &claim_a), ("fact_b", &claim_b)]);

        let result = add_conflict_scan(
            &accepted("fact_a"),
            &retrieved(&["fact_a", "fact_b"]),
            &claims,
            &single_value_status(),
        );

        assert_eq!(result.decision, Decision::Accepted);
    }

    #[test]
    fn conflict_scan_ignores_compatible_negation_and_multivalued_relations() {
        let affirmed_blue = ReviewedClaim {
            fact_id: "fact_a".to_owned(),
            claim: "status blue".to_owned(),
            anchor: "status blue".to_owned(),
            key: key("current"),
            polarity: ClaimPolarity::Affirmed,
            value: "blue".to_owned(),
        };
        let denied_amber = ReviewedClaim {
            fact_id: "fact_b".to_owned(),
            claim: "status not amber".to_owned(),
            anchor: "status not amber".to_owned(),
            key: key("current"),
            polarity: ClaimPolarity::Denied,
            value: "amber".to_owned(),
        };
        let affirmed_amber = ReviewedClaim {
            fact_id: "fact_c".to_owned(),
            claim: "status amber".to_owned(),
            anchor: "status amber".to_owned(),
            key: key("current"),
            polarity: ClaimPolarity::Affirmed,
            value: "amber".to_owned(),
        };

        assert!(!claims_conflict(
            &affirmed_blue,
            &denied_amber,
            &single_value_status()
        ));
        assert!(!claims_conflict(&affirmed_blue, &affirmed_amber, &[]));
    }

    #[test]
    fn conflict_scan_finds_denial_of_the_same_value() {
        let affirmed = ReviewedClaim {
            fact_id: "fact_a".to_owned(),
            claim: "status blue".to_owned(),
            anchor: "status blue".to_owned(),
            key: key("current"),
            polarity: ClaimPolarity::Affirmed,
            value: "blue".to_owned(),
        };
        let denied = ReviewedClaim {
            fact_id: "fact_b".to_owned(),
            claim: "status not blue".to_owned(),
            anchor: "status not blue".to_owned(),
            key: key("current"),
            polarity: ClaimPolarity::Denied,
            value: "blue".to_owned(),
        };

        assert!(claims_conflict(&affirmed, &denied, &[]));
    }

    #[test]
    fn candidate_union_is_stable_and_bounded() {
        let mut needs = Vec::new();
        for need_index in 0..4 {
            needs.push(RetrievedNeed {
                need_id: format!("n{need_index}"),
                candidates: (0..20)
                    .map(|candidate_index| RetrievedCandidate {
                        fact_id: format!("fact_{candidate_index:02}"),
                        text: "synthetic claim".to_owned(),
                        rank: candidate_index + 1,
                    })
                    .collect(),
            });
        }

        let candidates = union_candidates(&needs);

        assert_eq!(candidates.len(), 20);
        assert_eq!(candidates[0].fact_id, "fact_00");
    }

    #[test]
    fn aggregate_counts_selective_risk_and_conflicts_separately() {
        let cases = vec![CaseReport {
            id: "case".to_owned(),
            split: RetrievalSplit::Calibration,
            expected: Decision::Conflict,
            observed: Decision::Accepted,
            atomic_need_count: 1,
            covered_need_count: 1,
            selected_evidence_count: 1,
            supported_evidence_count: 1,
            forbidden_evidence_count: 0,
            candidate_required_group_count: 2,
            candidate_found_group_count: 2,
            selective_risk: true,
            outcome_correct: false,
            conflict_found: false,
            model_calls: 1,
            elapsed_ms: 4,
            failed: false,
        }];

        let report = aggregate(cases);

        assert_eq!(report.selective_risk_count, 1);
        assert_eq!(report.expected_conflict_count, 1);
        assert_eq!(report.conflict_found_count, 0);
    }

    #[test]
    fn mechanism_gate_rejects_degenerate_full_abstention() {
        let baseline = ArmReport {
            case_count: 3,
            ..ArmReport::default()
        };
        let candidate = ArmReport {
            case_count: 3,
            abstained_count: 3,
            model_call_count: 3,
            ..ArmReport::default()
        };

        assert!(!mechanism_is_promising(&baseline, &candidate));
    }

    #[test]
    fn scorer_uses_effective_pool_and_discards_abstained_proposals() {
        let base_case = fixture_case(vec![vec!["fact_required".to_owned()]], Vec::new());
        let reviewed_case = ReviewedCase {
            case_id: base_case.id.clone(),
            pre_model_excluded_fact_ids: Vec::new(),
            needs: vec![ReviewedNeed {
                id: "need".to_owned(),
                text: "synthetic need".to_owned(),
                expected_groups: vec![vec!["fact_required".to_owned()]],
            }],
            expected_outcome: Decision::Accepted,
        };
        let retrieved = vec![RetrievedNeed {
            need_id: "need".to_owned(),
            candidates: vec![candidate("fact_required", 2)],
        }];
        let effective_pool = vec![candidate("fact_other", 1)];
        let observation = ArmObservation {
            decision: Decision::Abstained,
            selections: vec![NeedSelection {
                need_id: "need".to_owned(),
                fact_ids: vec!["fact_required".to_owned()],
            }],
            model_calls: 1,
            elapsed_ms: 1,
            failed: false,
        };

        let report = score_case(
            &base_case,
            &reviewed_case,
            &retrieved,
            Some(&effective_pool),
            observation,
        );

        assert_eq!(report.candidate_found_group_count, 0);
        assert_eq!(report.selected_evidence_count, 0);
        assert_eq!(report.covered_need_count, 0);
    }

    #[test]
    fn pre_model_boundary_rejects_forbidden_candidates_and_bad_provenance() {
        let case = ReviewedCase {
            case_id: "case".to_owned(),
            pre_model_excluded_fact_ids: vec!["fact_private".to_owned()],
            needs: vec![ReviewedNeed {
                id: "need".to_owned(),
                text: "synthetic need".to_owned(),
                expected_groups: Vec::new(),
            }],
            expected_outcome: Decision::Abstained,
        };
        let forbidden_run = super::super::NormalizedRun {
            sources: vec![super::super::NormalizedSource {
                node: super::super::FixtureNode::Origin,
                hits: vec![super::super::NormalizedHit {
                    fact_id: "fact_private".to_owned(),
                    rank: 1,
                }],
            }],
            provenance_errors: 0,
        };
        let provenance_run = super::super::NormalizedRun {
            sources: Vec::new(),
            provenance_errors: 1,
        };

        let forbidden = validate_pre_model_boundary(&forbidden_run, &case).unwrap_err();
        let provenance = validate_pre_model_boundary(&provenance_run, &case).unwrap_err();

        assert_eq!(
            forbidden.to_string(),
            "retrieval exposed forbidden evidence before reviewed-anchor selection"
        );
        assert_eq!(
            provenance.to_string(),
            "retrieval returned invalid provenance before reviewed-anchor selection"
        );
    }

    fn key(temporal: &str) -> ClaimKey {
        ClaimKey {
            subject: "subject".to_owned(),
            relation: "status".to_owned(),
            scope: "scope".to_owned(),
            temporal: temporal.to_owned(),
        }
    }

    fn single_value_status() -> Vec<String> {
        vec!["status".to_owned()]
    }

    fn fixture_case(
        expected_groups: Vec<Vec<String>>,
        forbidden_fact_ids: Vec<String>,
    ) -> FixtureCase {
        FixtureCase {
            id: "case".to_owned(),
            domain: Some("domain".to_owned()),
            split: RetrievalSplit::Calibration,
            tags: Vec::new(),
            scope: super::super::RetrievalScope::Local,
            question: "synthetic question".to_owned(),
            semantic_keys: vec!["key".to_owned()],
            relevant_fact_ids: expected_groups.iter().flatten().cloned().collect(),
            expected_groups,
            forbidden_fact_ids,
        }
    }

    fn candidate(fact_id: &str, rank: u32) -> RetrievedCandidate {
        RetrievedCandidate {
            fact_id: fact_id.to_owned(),
            text: "synthetic claim".to_owned(),
            rank,
        }
    }

    fn accepted(fact_id: &str) -> ArmObservation {
        ArmObservation {
            decision: Decision::Accepted,
            selections: vec![NeedSelection {
                need_id: "n0".to_owned(),
                fact_ids: vec![fact_id.to_owned()],
            }],
            model_calls: 1,
            elapsed_ms: 1,
            failed: false,
        }
    }

    fn retrieved(fact_ids: &[&str]) -> Vec<RetrievedNeed> {
        vec![RetrievedNeed {
            need_id: "n0".to_owned(),
            candidates: fact_ids
                .iter()
                .enumerate()
                .map(|(index, fact_id)| RetrievedCandidate {
                    fact_id: (*fact_id).to_owned(),
                    text: "synthetic claim".to_owned(),
                    rank: u32::try_from(index + 1).unwrap(),
                })
                .collect(),
        }]
    }
}
