use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use crate::{replace_file, workspace_root};

const EXPERIMENT_DIRECTORY: &str = "experiments/typed-evidence-coverage-v3";
const CONTRACT_FILE: &str = "contract.json";
const FIELD_GUIDE_FILE: &str = "field-guide.md";
const EVALUATOR_FILE: &str = "xtask/src/typed_evidence_v3.rs";
const EXPERIMENT_ID: &str = "typed_evidence_coverage_v3";
const SCHEMA_VERSION: u32 = 1;
const REPORT_SCHEMA_VERSION: u32 = 1;
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const MAX_PACKAGE_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_RECORDS: usize = 10_000;
const CANDIDATE_TIMEOUT: Duration = Duration::from_secs(30);
const EXPECTED_PLATFORMS: [Platform; 2] = [Platform::MacosArm64, Platform::WindowsX64];
const REQUIRED_CASE_TAGS: [CaseTag; 9] = [
    CaseTag::Direct,
    CaseTag::Paraphrase,
    CaseTag::CrossLanguage,
    CaseTag::Compound,
    CaseTag::Absence,
    CaseTag::Conflict,
    CaseTag::Privacy,
    CaseTag::Injection,
    CaseTag::EntityAmbiguity,
];

const INPUT_SOURCE_FILE: &str = "inputs/sources.jsonl";
const INPUT_QUESTION_FILE: &str = "inputs/questions.jsonl";
const SOURCE_GOLD_FILE: &str = "review/source-gold.jsonl";
const QUESTION_GOLD_FILE: &str = "review/question-gold.jsonl";
const REVIEW_RECEIPT_FILE: &str = "review/receipt.json";
const SCORING_KEY_FILE: &str = "private/scoring-key.json";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExperimentContract {
    schema_version: u32,
    experiment_id: String,
    field_guide_sha256: String,
    evaluator_sha256: String,
    minimum_domains_per_split: usize,
    minimum_cases_per_split: usize,
    minimum_no_answer_cases_per_split: usize,
    minimum_compound_cases_per_split: usize,
    candidate_pool_limit: usize,
    top_k_per_source: usize,
    assignment_permutations: usize,
    minimum_recall_at_five: f64,
    minimum_exact_case_rate: f64,
    minimum_annotation_exact_rate: f64,
    minimum_control_delta: f64,
    required_platforms: Vec<Platform>,
    required_tags_per_split: Vec<CaseTag>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackageManifest {
    schema_version: u32,
    experiment_id: String,
    contract_sha256: String,
    candidate_revision_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RunReceipt {
    schema_version: u32,
    platform: Platform,
    contract_sha256: String,
    candidate_revision_sha256: String,
    binary_sha256: String,
    source_input_sha256: String,
    question_input_sha256: String,
    source_output_sha256: String,
    question_output_sha256: String,
    source_canonical_sha256: String,
    question_canonical_sha256: String,
    source_replay_sha256: String,
    question_replay_sha256: String,
    source_replay_canonical_sha256: String,
    question_replay_canonical_sha256: String,
    stderr_sha256: String,
    stderr_bytes: u64,
    source_exit_code: i32,
    source_replay_exit_code: i32,
    question_exit_code: i32,
    question_replay_exit_code: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReviewReceipt {
    schema_version: u32,
    contract_sha256: String,
    source_input_sha256: String,
    question_input_sha256: String,
    source_gold_sha256: String,
    question_gold_sha256: String,
    scoring_key_sha256: String,
    source_review_id: String,
    question_review_id: String,
    source_scope_was_isolated: bool,
    question_scope_was_isolated: bool,
    candidate_outputs_were_hidden: bool,
    scoring_key_was_hidden: bool,
    promotion_was_authored_after_freeze: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Platform {
    MacosArm64,
    WindowsX64,
}

impl Platform {
    const fn directory(self) -> &'static str {
        match self {
            Self::MacosArm64 => "macos_arm64",
            Self::WindowsX64 => "windows_x64",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceInput {
    source_id: String,
    title: String,
    heading: String,
    text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct QuestionInput {
    question_id: String,
    question: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SourceAnnotation {
    source_id: String,
    status: AnnotationStatus,
    claims: Vec<Claim>,
    reason_code: Option<UnresolvedReason>,
}

impl SourceAnnotation {
    fn normalize(&mut self) {
        for claim in &mut self.claims {
            claim.normalize();
        }
        self.claims.sort();
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct QuestionAnnotation {
    question_id: String,
    status: AnnotationStatus,
    needs: Vec<Need>,
    reason_code: Option<UnresolvedReason>,
}

impl QuestionAnnotation {
    fn normalize(&mut self) {
        for need in &mut self.needs {
            need.normalize();
        }
        self.needs.sort();
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum AnnotationStatus {
    Resolved,
    Unresolved,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum UnresolvedReason {
    MissingSubject,
    AmbiguousSubject,
    AmbiguousRelation,
    AmbiguousState,
    UnsupportedStructure,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
struct TextSpan {
    start: u32,
    end: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(deny_unknown_fields)]
struct SemanticField {
    normalized: String,
    spans: Vec<TextSpan>,
}

impl SemanticField {
    fn normalize(&mut self) {
        self.spans.sort();
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct Qualifier {
    name: SemanticField,
    value: SemanticField,
}

impl Qualifier {
    fn normalize(&mut self) {
        self.name.normalize();
        self.value.normalize();
    }

    fn identity(&self) -> (&str, &str) {
        (&self.name.normalized, &self.value.normalized)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct Claim {
    subject: SemanticField,
    relation: SemanticField,
    object_type: SemanticField,
    object_value: SemanticField,
    qualifiers: Vec<Qualifier>,
    polarity: Polarity,
    lifecycles: Vec<String>,
    provenance: Provenance,
    support_spans: Vec<TextSpan>,
}

impl Claim {
    fn normalize(&mut self) {
        self.subject.normalize();
        self.relation.normalize();
        self.object_type.normalize();
        self.object_value.normalize();
        for qualifier in &mut self.qualifiers {
            qualifier.normalize();
        }
        self.qualifiers.sort();
        self.lifecycles.sort();
        self.support_spans.sort();
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
struct Need {
    subject: SemanticField,
    relation: SemanticField,
    requested_object_types: Vec<SemanticField>,
    required_qualifiers: Vec<Qualifier>,
    allowed_polarities: Vec<Polarity>,
    required_lifecycles: Vec<String>,
    allowed_provenances: Vec<Provenance>,
    support_spans: Vec<TextSpan>,
}

impl Need {
    fn normalize(&mut self) {
        self.subject.normalize();
        self.relation.normalize();
        for object_type in &mut self.requested_object_types {
            object_type.normalize();
        }
        self.requested_object_types.sort();
        for qualifier in &mut self.required_qualifiers {
            qualifier.normalize();
        }
        self.required_qualifiers.sort();
        self.allowed_polarities.sort();
        self.required_lifecycles.sort();
        self.allowed_provenances.sort();
        self.support_spans.sort();
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
enum Polarity {
    Positive,
    Negative,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
enum Provenance {
    Direct,
    Attributed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScoringKey {
    schema_version: u32,
    cases: Vec<CaseKey>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CaseKey {
    case_id: String,
    domain_id: String,
    split: Split,
    question_id: String,
    tags: Vec<CaseTag>,
    source_pools: Vec<SourcePool>,
    relevant_source_ids: Vec<String>,
    expected_groups: Vec<Vec<String>>,
    allowed_support_source_ids: Vec<String>,
    forbidden_source_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourcePool {
    pool_id: String,
    candidate_source_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Split {
    Development,
    Promotion,
}

impl Split {
    const ALL: [Self; 2] = [Self::Development, Self::Promotion];
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
enum CaseTag {
    Direct,
    Paraphrase,
    CrossLanguage,
    Compound,
    Absence,
    Conflict,
    Privacy,
    Injection,
    EntityAmbiguity,
}

#[derive(Debug)]
struct LoadedPackage {
    manifest: PackageManifest,
    contract: ExperimentContract,
    contract_sha256: String,
    sources: Vec<SourceInput>,
    questions: Vec<QuestionInput>,
    source_gold: Vec<SourceAnnotation>,
    question_gold: Vec<QuestionAnnotation>,
    scoring_key: ScoringKey,
    runs: BTreeMap<Platform, ValidatedRun>,
}

#[derive(Debug)]
struct ValidatedRun {
    source_annotations: Vec<SourceAnnotation>,
    question_annotations: Vec<QuestionAnnotation>,
    source_canonical_sha256: String,
    question_canonical_sha256: String,
}

#[derive(Debug)]
struct CandidateExecution {
    output_path: PathBuf,
    output_sha256: String,
    exit_code: i32,
}

#[derive(Debug)]
struct TemporaryDirectory {
    path: PathBuf,
    armed: bool,
}

impl TemporaryDirectory {
    fn create(parent: &Path, prefix: &str) -> Result<Self> {
        let path = parent.join(format!(
            ".{prefix}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&path)
            .with_context(|| format!("creating temporary directory {}", path.display()))?;
        Ok(Self { path, armed: true })
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PackageSummary {
    pub cases: usize,
    pub sources: usize,
    pub questions: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SemanticReport {
    schema_version: u32,
    experiment_id: String,
    contract_sha256: String,
    candidate_revision_sha256: String,
    source_canonical_sha256: String,
    question_canonical_sha256: String,
    cross_platform_identical: bool,
    unresolved_source_records: usize,
    unresolved_question_records: usize,
    source_annotation_agreement: AnnotationAgreement,
    question_annotation_agreement: AnnotationAgreement,
    treatment: EvaluationMetrics,
    structure_only: EvaluationMetrics,
    best_assignment_permutation: EvaluationMetrics,
    control_delta: f64,
    passed: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct EvaluationMetrics {
    overall: SplitMetrics,
    development: SplitMetrics,
    promotion: SplitMetrics,
    unexpected_evidence: usize,
    forbidden_evidence: usize,
    authorization_errors: usize,
    provenance_errors: usize,
    duplicate_errors: usize,
    stability_errors: usize,
    compound_partial_errors: usize,
    conflict_coverage_errors: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SplitMetrics {
    found_groups: usize,
    expected_groups: usize,
    exact_cases: usize,
    cases: usize,
    recall_at_five: f64,
    exact_case_rate: f64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct AnnotationAgreement {
    overall: f64,
    development: f64,
    promotion: f64,
}

#[derive(Debug, Clone, Copy)]
enum EvaluationArm {
    Treatment,
    StructureOnly,
    AssignmentPermutation(usize),
}

pub fn validate_contract() -> Result<()> {
    let root = workspace_root();
    let contract_path = root.join(EXPERIMENT_DIRECTORY).join(CONTRACT_FILE);
    let field_guide_path = root.join(EXPERIMENT_DIRECTORY).join(FIELD_GUIDE_FILE);
    let contract = load_json::<ExperimentContract>(&contract_path)?;
    validate_contract_values(&contract, &field_guide_path)?;
    println!(
        "validated typed-evidence v3 contract (SHA-256 {})",
        hash_file(&contract_path)?
    );
    Ok(())
}

pub fn validate_package(path: &Path) -> Result<PackageSummary> {
    let package = load_package(path)?;
    let summary = PackageSummary {
        cases: package.scoring_key.cases.len(),
        sources: package.sources.len(),
        questions: package.questions.len(),
    };
    println!(
        "validated typed-evidence v3 package: {} cases, {} sources, {} questions",
        summary.cases, summary.sources, summary.questions
    );
    Ok(summary)
}

pub fn run_candidate(path: &Path, candidate: &Path, platform_label: &str) -> Result<()> {
    let platform = parse_current_platform(platform_label)?;
    let directory_metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("reading package metadata for {}", path.display()))?;
    ensure!(
        directory_metadata.is_dir() && !directory_metadata.file_type().is_symlink(),
        "typed-evidence package must be a real directory"
    );

    let contract_path = workspace_root()
        .join(EXPERIMENT_DIRECTORY)
        .join(CONTRACT_FILE);
    let field_guide_path = workspace_root()
        .join(EXPERIMENT_DIRECTORY)
        .join(FIELD_GUIDE_FILE);
    let contract = load_json::<ExperimentContract>(&contract_path)?;
    validate_contract_values(&contract, &field_guide_path)?;
    let contract_sha256 = hash_file(&contract_path)?;
    let manifest = load_json::<PackageManifest>(&package_file(path, "manifest.json")?)?;
    ensure!(
        manifest.schema_version == SCHEMA_VERSION
            && manifest.experiment_id == EXPERIMENT_ID
            && manifest.contract_sha256 == contract_sha256,
        "package manifest does not match the frozen contract"
    );
    validate_sha256(&manifest.candidate_revision_sha256, "candidate revision")?;

    ensure_regular_bounded_file(candidate)?;
    let package_root = path
        .canonicalize()
        .context("canonicalizing typed-evidence package")?;
    let candidate_path = candidate
        .canonicalize()
        .context("canonicalizing typed-evidence candidate")?;
    ensure!(
        !candidate_path.starts_with(&package_root),
        "candidate binary must remain outside the private package"
    );
    let binary_sha256 = hash_file(&candidate_path)?;
    let source_input_path = package_file(path, INPUT_SOURCE_FILE)?;
    let question_input_path = package_file(path, INPUT_QUESTION_FILE)?;
    let sources = load_jsonl::<SourceInput>(&source_input_path)?;
    let questions = load_jsonl::<QuestionInput>(&question_input_path)?;
    validate_inputs(&sources, &questions)?;
    let source_map = sources
        .iter()
        .map(|source| (source.source_id.as_str(), source))
        .collect::<HashMap<_, _>>();
    let question_map = questions
        .iter()
        .map(|question| (question.question_id.as_str(), question))
        .collect::<HashMap<_, _>>();

    let run_directory = path.join("runs").join(platform.directory());
    ensure!(
        !run_directory.exists(),
        "a run already exists for {}; preserve it instead of overwriting evidence",
        platform.directory()
    );
    let before = snapshot_directory(path)?;
    let system_temporary = std::env::temp_dir();
    let scratch = TemporaryDirectory::create(&system_temporary, "airwiki-typed-evidence-v3")?;
    let source_primary = execute_candidate_side(
        &candidate_path,
        "source",
        &source_input_path,
        &scratch.path,
        "source-primary",
    )?;
    let source_replay = execute_candidate_side(
        &candidate_path,
        "source",
        &source_input_path,
        &scratch.path,
        "source-replay",
    )?;
    let question_primary = execute_candidate_side(
        &candidate_path,
        "question",
        &question_input_path,
        &scratch.path,
        "question-primary",
    )?;
    let question_replay = execute_candidate_side(
        &candidate_path,
        "question",
        &question_input_path,
        &scratch.path,
        "question-replay",
    )?;
    ensure!(
        snapshot_directory(path)? == before,
        "candidate execution modified the private package"
    );

    let mut source_annotations = load_jsonl::<SourceAnnotation>(&source_primary.output_path)?;
    let mut source_replay_annotations = load_jsonl::<SourceAnnotation>(&source_replay.output_path)?;
    let mut question_annotations = load_jsonl::<QuestionAnnotation>(&question_primary.output_path)?;
    let mut question_replay_annotations =
        load_jsonl::<QuestionAnnotation>(&question_replay.output_path)?;
    normalize_source_annotations(&mut source_annotations);
    normalize_source_annotations(&mut source_replay_annotations);
    normalize_question_annotations(&mut question_annotations);
    normalize_question_annotations(&mut question_replay_annotations);
    validate_source_annotations(&source_annotations, &source_map)?;
    validate_source_annotations(&source_replay_annotations, &source_map)?;
    validate_question_annotations(&question_annotations, &question_map)?;
    validate_question_annotations(&question_replay_annotations, &question_map)?;
    let source_canonical = canonical_jsonl(&source_annotations)?;
    let source_replay_canonical = canonical_jsonl(&source_replay_annotations)?;
    let question_canonical = canonical_jsonl(&question_annotations)?;
    let question_replay_canonical = canonical_jsonl(&question_replay_annotations)?;
    ensure!(
        source_canonical == source_replay_canonical,
        "source extraction was not deterministic"
    );
    ensure!(
        question_canonical == question_replay_canonical,
        "question extraction was not deterministic"
    );

    let runs_parent = path.join("runs");
    let mut staging = TemporaryDirectory::create(path, platform.directory())?;
    copy_file(
        &source_primary.output_path,
        &staging.path.join("sources.jsonl"),
    )?;
    copy_file(
        &source_replay.output_path,
        &staging.path.join("sources-replay.jsonl"),
    )?;
    copy_file(
        &question_primary.output_path,
        &staging.path.join("questions.jsonl"),
    )?;
    copy_file(
        &question_replay.output_path,
        &staging.path.join("questions-replay.jsonl"),
    )?;
    let receipt = RunReceipt {
        schema_version: SCHEMA_VERSION,
        platform,
        contract_sha256,
        candidate_revision_sha256: manifest.candidate_revision_sha256,
        binary_sha256,
        source_input_sha256: hash_file(&source_input_path)?,
        question_input_sha256: hash_file(&question_input_path)?,
        source_output_sha256: source_primary.output_sha256,
        question_output_sha256: question_primary.output_sha256,
        source_canonical_sha256: hash_bytes(&source_canonical),
        question_canonical_sha256: hash_bytes(&question_canonical),
        source_replay_sha256: source_replay.output_sha256,
        question_replay_sha256: question_replay.output_sha256,
        source_replay_canonical_sha256: hash_bytes(&source_replay_canonical),
        question_replay_canonical_sha256: hash_bytes(&question_replay_canonical),
        stderr_sha256: EMPTY_SHA256.to_owned(),
        stderr_bytes: 0,
        source_exit_code: source_primary.exit_code,
        source_replay_exit_code: source_replay.exit_code,
        question_exit_code: question_primary.exit_code,
        question_replay_exit_code: question_replay.exit_code,
    };
    write_pretty_json(&staging.path.join("receipt.json"), &receipt)?;
    let runs_parent_existed = runs_parent.exists();
    if runs_parent_existed {
        let metadata = std::fs::symlink_metadata(&runs_parent)
            .context("reading candidate runs directory metadata")?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "candidate runs path is not a real directory"
        );
    } else {
        std::fs::create_dir(&runs_parent).context("creating candidate runs directory")?;
    }
    if let Err(error) = std::fs::rename(&staging.path, &run_directory) {
        if !runs_parent_existed {
            let _ = std::fs::remove_dir(&runs_parent);
        }
        return Err(error)
            .with_context(|| format!("installing candidate run at {}", run_directory.display()));
    }
    staging.disarm();
    println!(
        "recorded deterministic typed-evidence v3 run for {}",
        platform.directory()
    );
    Ok(())
}

pub fn score(path: &Path, report_path: &Path) -> Result<()> {
    let package = load_package(path)?;
    let report = build_report(&package)?;
    let replay = build_report(&package)?;
    let report_bytes = canonical_json(&report)?;
    let replay_bytes = canonical_json(&replay)?;
    ensure!(
        report_bytes == replay_bytes,
        "typed-evidence v3 scorer replay was not deterministic"
    );
    write_report(report_path, &report_bytes)?;
    ensure!(
        report.passed,
        "typed-evidence v3 candidate did not satisfy the frozen gates; report written to {}",
        report_path.display()
    );
    println!(
        "typed-evidence v3 candidate passed; report written to {}",
        report_path.display()
    );
    Ok(())
}

fn parse_current_platform(label: &str) -> Result<Platform> {
    let platform = match label {
        "macos_arm64" => Platform::MacosArm64,
        "windows_x64" => Platform::WindowsX64,
        other => bail!("unsupported typed-evidence platform `{other}`"),
    };
    let current = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Platform::MacosArm64,
        ("windows", "x86_64") => Platform::WindowsX64,
        (os, arch) => bail!("typed-evidence runs are unsupported on {os}/{arch}"),
    };
    ensure!(
        platform == current,
        "cannot record {} evidence on the current platform",
        platform.directory()
    );
    Ok(platform)
}

fn execute_candidate_side(
    candidate: &Path,
    mode: &str,
    input: &Path,
    scratch: &Path,
    execution_id: &str,
) -> Result<CandidateExecution> {
    ensure!(
        mode == "source" || mode == "question",
        "invalid candidate mode"
    );
    let execution_directory = scratch.join(execution_id);
    std::fs::create_dir(&execution_directory).with_context(|| {
        format!(
            "creating candidate execution directory {}",
            execution_directory.display()
        )
    })?;
    let input_copy = execution_directory.join("input.jsonl");
    let output_path = execution_directory.join("output.jsonl");
    let stderr_path = execution_directory.join("stderr.log");
    copy_file(input, &input_copy)?;
    let stdin = File::open(&input_copy).context("opening candidate stdin")?;
    let stdout = File::create(&output_path).context("creating candidate stdout")?;
    let stderr = File::create(&stderr_path).context("creating candidate stderr")?;
    let mut child = Command::new(candidate)
        .arg(mode)
        .current_dir(&execution_directory)
        .env_clear()
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| format!("starting typed-evidence `{mode}` candidate"))?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .context("polling typed-evidence candidate")?
        {
            break status;
        }
        if started.elapsed() >= CANDIDATE_TIMEOUT {
            child
                .kill()
                .context("terminating timed-out typed-evidence candidate")?;
            let _ = child.wait();
            bail!("typed-evidence `{mode}` candidate exceeded the timeout");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let exit_code = status
        .code()
        .context("typed-evidence candidate ended without an exit code")?;
    ensure!(exit_code == 0, "typed-evidence `{mode}` candidate failed");
    ensure_regular_bounded_file(&output_path)?;
    ensure_regular_bounded_file(&stderr_path)?;
    ensure!(
        std::fs::metadata(&stderr_path)
            .context("reading candidate stderr metadata")?
            .len()
            == 0,
        "typed-evidence `{mode}` candidate wrote stderr"
    );
    ensure!(
        hash_file(&input_copy)? == hash_file(input)?,
        "typed-evidence `{mode}` candidate modified its input"
    );
    let entries = std::fs::read_dir(&execution_directory)
        .context("listing candidate execution directory")?
        .map(|entry| {
            entry
                .context("reading candidate execution entry")?
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("candidate created a non-UTF-8 file name"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        entries
            == BTreeSet::from([
                "input.jsonl".to_owned(),
                "output.jsonl".to_owned(),
                "stderr.log".to_owned(),
            ]),
        "typed-evidence `{mode}` candidate created unexpected files"
    );
    Ok(CandidateExecution {
        output_sha256: hash_file(&output_path)?,
        output_path,
        exit_code,
    })
}

fn snapshot_directory(directory: &Path) -> Result<BTreeMap<String, String>> {
    let mut snapshot = BTreeMap::new();
    snapshot_directory_recursive(directory, directory, &mut snapshot)?;
    Ok(snapshot)
}

fn snapshot_directory_recursive(
    root: &Path,
    directory: &Path,
    snapshot: &mut BTreeMap<String, String>,
) -> Result<()> {
    let mut entries = std::fs::read_dir(directory)
        .with_context(|| format!("listing private package directory {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("reading private package entry {}", path.display()))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "private package contains a symlink"
        );
        let relative = path
            .strip_prefix(root)
            .context("private package entry escaped its root")?
            .to_str()
            .context("private package entry name is not UTF-8")?
            .replace('\\', "/");
        if metadata.is_dir() {
            ensure!(
                snapshot
                    .insert(format!("{relative}/"), "directory".to_owned())
                    .is_none(),
                "private package contains duplicate paths"
            );
            snapshot_directory_recursive(root, &path, snapshot)?;
        } else {
            ensure!(
                metadata.is_file(),
                "private package contains a special file"
            );
            ensure!(
                snapshot.insert(relative, hash_file(&path)?).is_none(),
                "private package contains duplicate paths"
            );
        }
    }
    Ok(())
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    let bytes = std::fs::read(source)
        .with_context(|| format!("reading candidate artifact {}", source.display()))?;
    ensure!(
        u64::try_from(bytes.len()).context("candidate artifact size overflow")?
            <= MAX_PACKAGE_FILE_BYTES,
        "candidate artifact exceeds the file limit"
    );
    std::fs::write(destination, bytes)
        .with_context(|| format!("writing candidate artifact {}", destination.display()))
}

fn write_pretty_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).context("serializing candidate receipt")?;
    bytes.push(b'\n');
    std::fs::write(path, bytes)
        .with_context(|| format!("writing candidate receipt {}", path.display()))
}

fn load_package(directory: &Path) -> Result<LoadedPackage> {
    ensure!(
        directory.is_dir(),
        "typed-evidence package is not a directory"
    );
    let contract_path = workspace_root()
        .join(EXPERIMENT_DIRECTORY)
        .join(CONTRACT_FILE);
    let field_guide_path = workspace_root()
        .join(EXPERIMENT_DIRECTORY)
        .join(FIELD_GUIDE_FILE);
    let contract = load_json::<ExperimentContract>(&contract_path)?;
    validate_contract_values(&contract, &field_guide_path)?;
    let contract_sha256 = hash_file(&contract_path)?;

    let manifest_path = package_file(directory, "manifest.json")?;
    let manifest = load_json::<PackageManifest>(&manifest_path)?;
    ensure!(
        manifest.schema_version == SCHEMA_VERSION,
        "unsupported package schema"
    );
    ensure!(
        manifest.experiment_id == EXPERIMENT_ID,
        "unexpected experiment id"
    );
    ensure!(
        manifest.contract_sha256 == contract_sha256,
        "package does not match the frozen contract"
    );
    validate_sha256(&manifest.candidate_revision_sha256, "candidate revision")?;

    let source_input_path = package_file(directory, INPUT_SOURCE_FILE)?;
    let question_input_path = package_file(directory, INPUT_QUESTION_FILE)?;
    let source_gold_path = package_file(directory, SOURCE_GOLD_FILE)?;
    let question_gold_path = package_file(directory, QUESTION_GOLD_FILE)?;
    let scoring_key_path = package_file(directory, SCORING_KEY_FILE)?;
    let review_receipt_path = package_file(directory, REVIEW_RECEIPT_FILE)?;

    let sources = load_jsonl::<SourceInput>(&source_input_path)?;
    let questions = load_jsonl::<QuestionInput>(&question_input_path)?;
    validate_inputs(&sources, &questions)?;
    let source_map = sources
        .iter()
        .map(|source| (source.source_id.as_str(), source))
        .collect::<HashMap<_, _>>();
    let question_map = questions
        .iter()
        .map(|question| (question.question_id.as_str(), question))
        .collect::<HashMap<_, _>>();

    let mut source_gold = load_jsonl::<SourceAnnotation>(&source_gold_path)?;
    let mut question_gold = load_jsonl::<QuestionAnnotation>(&question_gold_path)?;
    normalize_source_annotations(&mut source_gold);
    normalize_question_annotations(&mut question_gold);
    validate_source_annotations(&source_gold, &source_map)?;
    validate_question_annotations(&question_gold, &question_map)?;

    let scoring_key = load_json::<ScoringKey>(&scoring_key_path)?;
    validate_scoring_key(&scoring_key, &contract, &source_map, &question_map)?;

    let review_receipt = load_json::<ReviewReceipt>(&review_receipt_path)?;
    validate_review_receipt(
        &review_receipt,
        &contract_sha256,
        &source_input_path,
        &question_input_path,
        &source_gold_path,
        &question_gold_path,
        &scoring_key_path,
    )?;

    let mut runs = BTreeMap::new();
    for platform in EXPECTED_PLATFORMS {
        let run = load_run(
            directory,
            platform,
            &manifest,
            &source_input_path,
            &question_input_path,
            &source_map,
            &question_map,
        )?;
        ensure!(
            runs.insert(platform, run).is_none(),
            "duplicate platform run"
        );
    }
    validate_cross_platform_outputs(&runs)?;

    Ok(LoadedPackage {
        manifest,
        contract,
        contract_sha256,
        sources,
        questions,
        source_gold,
        question_gold,
        scoring_key,
        runs,
    })
}

fn validate_contract_values(contract: &ExperimentContract, field_guide: &Path) -> Result<()> {
    ensure!(
        contract.schema_version == SCHEMA_VERSION,
        "unsupported contract schema"
    );
    ensure!(
        contract.experiment_id == EXPERIMENT_ID,
        "unexpected contract id"
    );
    ensure!(
        contract.minimum_domains_per_split == 4,
        "domain gate changed"
    );
    ensure!(contract.minimum_cases_per_split == 12, "case gate changed");
    ensure!(
        contract.minimum_no_answer_cases_per_split == 3,
        "absence gate changed"
    );
    ensure!(
        contract.minimum_compound_cases_per_split == 2,
        "compound gate changed"
    );
    ensure!(
        contract.candidate_pool_limit == 10,
        "candidate pool limit changed"
    );
    ensure!(contract.top_k_per_source == 5, "top-k gate changed");
    ensure!(
        contract.assignment_permutations == 8,
        "control count changed"
    );
    ensure!(
        (contract.minimum_recall_at_five - 0.9).abs() < f64::EPSILON,
        "recall gate changed"
    );
    ensure!(
        (contract.minimum_exact_case_rate - 0.85).abs() < f64::EPSILON,
        "exact-case gate changed"
    );
    ensure!(
        (contract.minimum_annotation_exact_rate - 0.85).abs() < f64::EPSILON,
        "annotation gate changed"
    );
    ensure!(
        (contract.minimum_control_delta - 0.1).abs() < f64::EPSILON,
        "control gate changed"
    );
    ensure!(
        contract.required_platforms == EXPECTED_PLATFORMS,
        "platform gate changed"
    );
    ensure!(
        contract.required_tags_per_split == REQUIRED_CASE_TAGS,
        "taxonomy gate changed"
    );
    validate_sha256(&contract.field_guide_sha256, "field guide")?;
    ensure!(
        hash_file(field_guide)? == contract.field_guide_sha256,
        "field guide does not match the frozen contract"
    );
    validate_sha256(&contract.evaluator_sha256, "evaluator")?;
    ensure!(
        hash_file(&workspace_root().join(EVALUATOR_FILE))? == contract.evaluator_sha256,
        "evaluator source does not match the frozen contract"
    );
    Ok(())
}

fn validate_review_receipt(
    receipt: &ReviewReceipt,
    contract_sha256: &str,
    source_input: &Path,
    question_input: &Path,
    source_gold: &Path,
    question_gold: &Path,
    scoring_key: &Path,
) -> Result<()> {
    ensure!(
        receipt.schema_version == SCHEMA_VERSION,
        "unsupported review receipt"
    );
    ensure!(
        receipt.contract_sha256 == contract_sha256,
        "review used another contract"
    );
    for (actual, path, kind) in [
        (&receipt.source_input_sha256, source_input, "source input"),
        (
            &receipt.question_input_sha256,
            question_input,
            "question input",
        ),
        (&receipt.source_gold_sha256, source_gold, "source gold"),
        (
            &receipt.question_gold_sha256,
            question_gold,
            "question gold",
        ),
        (&receipt.scoring_key_sha256, scoring_key, "scoring key"),
    ] {
        validate_sha256(actual, kind)?;
        ensure!(
            *actual == hash_file(path)?,
            "review receipt {kind} hash differs"
        );
    }
    validate_identifier(&receipt.source_review_id, "source review")?;
    validate_identifier(&receipt.question_review_id, "question review")?;
    ensure!(
        receipt.source_review_id != receipt.question_review_id,
        "source and question review must be independent"
    );
    ensure!(
        receipt.source_scope_was_isolated,
        "source review was not isolated"
    );
    ensure!(
        receipt.question_scope_was_isolated,
        "question review was not isolated"
    );
    ensure!(
        receipt.candidate_outputs_were_hidden,
        "reviewers saw candidate output"
    );
    ensure!(
        receipt.scoring_key_was_hidden,
        "reviewers saw the scoring key"
    );
    ensure!(
        receipt.promotion_was_authored_after_freeze,
        "promotion was not authored after freeze"
    );
    Ok(())
}

fn load_run(
    directory: &Path,
    platform: Platform,
    manifest: &PackageManifest,
    source_input_path: &Path,
    question_input_path: &Path,
    source_map: &HashMap<&str, &SourceInput>,
    question_map: &HashMap<&str, &QuestionInput>,
) -> Result<ValidatedRun> {
    let prefix = format!("runs/{}", platform.directory());
    let source_path = package_file(directory, &format!("{prefix}/sources.jsonl"))?;
    let question_path = package_file(directory, &format!("{prefix}/questions.jsonl"))?;
    let source_replay_path = package_file(directory, &format!("{prefix}/sources-replay.jsonl"))?;
    let question_replay_path =
        package_file(directory, &format!("{prefix}/questions-replay.jsonl"))?;
    let receipt_path = package_file(directory, &format!("{prefix}/receipt.json"))?;
    let receipt = load_json::<RunReceipt>(&receipt_path)?;

    ensure!(
        receipt.schema_version == SCHEMA_VERSION,
        "unsupported run receipt"
    );
    ensure!(receipt.platform == platform, "run receipt platform differs");
    ensure!(
        receipt.contract_sha256 == manifest.contract_sha256,
        "run used another contract"
    );
    ensure!(
        receipt.candidate_revision_sha256 == manifest.candidate_revision_sha256,
        "run used another candidate revision"
    );
    validate_sha256(&receipt.binary_sha256, "candidate binary")?;
    ensure!(
        receipt.source_input_sha256 == hash_file(source_input_path)?,
        "source input hash differs"
    );
    ensure!(
        receipt.question_input_sha256 == hash_file(question_input_path)?,
        "question input hash differs"
    );
    ensure!(
        receipt.source_output_sha256 == hash_file(&source_path)?,
        "source output hash differs"
    );
    ensure!(
        receipt.question_output_sha256 == hash_file(&question_path)?,
        "question output hash differs"
    );
    ensure!(
        receipt.source_replay_sha256 == hash_file(&source_replay_path)?,
        "source replay hash differs"
    );
    ensure!(
        receipt.question_replay_sha256 == hash_file(&question_replay_path)?,
        "question replay hash differs"
    );
    ensure!(
        receipt.stderr_sha256 == EMPTY_SHA256 && receipt.stderr_bytes == 0,
        "candidate wrote stderr"
    );
    ensure!(
        receipt.source_exit_code == 0
            && receipt.source_replay_exit_code == 0
            && receipt.question_exit_code == 0
            && receipt.question_replay_exit_code == 0,
        "candidate process failed"
    );

    let mut source_annotations = load_jsonl::<SourceAnnotation>(&source_path)?;
    let mut question_annotations = load_jsonl::<QuestionAnnotation>(&question_path)?;
    let mut source_replay = load_jsonl::<SourceAnnotation>(&source_replay_path)?;
    let mut question_replay = load_jsonl::<QuestionAnnotation>(&question_replay_path)?;
    normalize_source_annotations(&mut source_annotations);
    normalize_question_annotations(&mut question_annotations);
    normalize_source_annotations(&mut source_replay);
    normalize_question_annotations(&mut question_replay);
    validate_source_annotations(&source_annotations, source_map)?;
    validate_question_annotations(&question_annotations, question_map)?;
    validate_source_annotations(&source_replay, source_map)?;
    validate_question_annotations(&question_replay, question_map)?;

    let source_canonical = canonical_jsonl(&source_annotations)?;
    let question_canonical = canonical_jsonl(&question_annotations)?;
    let source_replay_canonical = canonical_jsonl(&source_replay)?;
    let question_replay_canonical = canonical_jsonl(&question_replay)?;
    let source_canonical_sha256 = hash_bytes(&source_canonical);
    let question_canonical_sha256 = hash_bytes(&question_canonical);
    ensure!(
        receipt.source_canonical_sha256 == source_canonical_sha256,
        "source canonical hash differs"
    );
    ensure!(
        receipt.question_canonical_sha256 == question_canonical_sha256,
        "question canonical hash differs"
    );
    ensure!(
        receipt.source_replay_canonical_sha256 == hash_bytes(&source_replay_canonical),
        "source replay canonical hash differs"
    );
    ensure!(
        receipt.question_replay_canonical_sha256 == hash_bytes(&question_replay_canonical),
        "question replay canonical hash differs"
    );
    ensure!(
        source_canonical == source_replay_canonical,
        "source extraction is nondeterministic"
    );
    ensure!(
        question_canonical == question_replay_canonical,
        "question extraction is nondeterministic"
    );

    Ok(ValidatedRun {
        source_annotations,
        question_annotations,
        source_canonical_sha256,
        question_canonical_sha256,
    })
}

fn validate_cross_platform_outputs(runs: &BTreeMap<Platform, ValidatedRun>) -> Result<()> {
    let macos = runs
        .get(&Platform::MacosArm64)
        .context("macOS run is missing")?;
    let windows = runs
        .get(&Platform::WindowsX64)
        .context("Windows run is missing")?;
    ensure!(
        macos.source_annotations == windows.source_annotations,
        "source annotations differ across platforms"
    );
    ensure!(
        macos.question_annotations == windows.question_annotations,
        "question annotations differ across platforms"
    );
    Ok(())
}

fn validate_inputs(sources: &[SourceInput], questions: &[QuestionInput]) -> Result<()> {
    ensure!(
        !sources.is_empty() && sources.len() <= MAX_RECORDS,
        "invalid source input count"
    );
    ensure!(
        !questions.is_empty() && questions.len() <= MAX_RECORDS,
        "invalid question input count"
    );
    let mut source_ids = HashSet::new();
    for source in sources {
        validate_opaque_identifier(&source.source_id, "src_", "source")?;
        validate_text(&source.title, "source title")?;
        validate_text(&source.heading, "source heading")?;
        validate_text(&source.text, "source text")?;
        ensure!(
            source_ids.insert(source.source_id.as_str()),
            "duplicate source id"
        );
    }
    let mut question_ids = HashSet::new();
    for question in questions {
        validate_opaque_identifier(&question.question_id, "qry_", "question")?;
        validate_text(&question.question, "question text")?;
        ensure!(
            question_ids.insert(question.question_id.as_str()),
            "duplicate question id"
        );
    }
    Ok(())
}

fn validate_source_annotations(
    annotations: &[SourceAnnotation],
    sources: &HashMap<&str, &SourceInput>,
) -> Result<()> {
    ensure!(
        annotations.len() == sources.len(),
        "source annotation count differs from input"
    );
    let mut ids = HashSet::new();
    for annotation in annotations {
        ensure!(
            ids.insert(annotation.source_id.as_str()),
            "duplicate source annotation"
        );
        let source = sources
            .get(annotation.source_id.as_str())
            .with_context(|| {
                format!(
                    "annotation references unknown source `{}`",
                    annotation.source_id
                )
            })?;
        validate_annotation_state(
            annotation.status,
            annotation.claims.len(),
            annotation.reason_code,
        )?;
        ensure!(
            annotation
                .claims
                .windows(2)
                .all(|pair| matches!(pair, [left, right] if left != right)),
            "source annotation repeats an identical claim"
        );
        for claim in &annotation.claims {
            validate_claim(claim, &source.text)?;
        }
    }
    ensure!(
        ids.len() == sources.len(),
        "source annotations are incomplete"
    );
    Ok(())
}

fn validate_question_annotations(
    annotations: &[QuestionAnnotation],
    questions: &HashMap<&str, &QuestionInput>,
) -> Result<()> {
    ensure!(
        annotations.len() == questions.len(),
        "question annotation count differs from input"
    );
    let mut ids = HashSet::new();
    for annotation in annotations {
        ensure!(
            ids.insert(annotation.question_id.as_str()),
            "duplicate question annotation"
        );
        let question = questions
            .get(annotation.question_id.as_str())
            .with_context(|| {
                format!(
                    "annotation references unknown question `{}`",
                    annotation.question_id
                )
            })?;
        validate_annotation_state(
            annotation.status,
            annotation.needs.len(),
            annotation.reason_code,
        )?;
        let mut semantic_needs = HashSet::new();
        for need in &annotation.needs {
            validate_need(need, &question.question)?;
            ensure!(
                semantic_needs.insert(need_semantic_key(need)?),
                "question annotation repeats a semantic need"
            );
        }
    }
    ensure!(
        ids.len() == questions.len(),
        "question annotations are incomplete"
    );
    Ok(())
}

fn validate_annotation_state(
    status: AnnotationStatus,
    record_count: usize,
    reason: Option<UnresolvedReason>,
) -> Result<()> {
    match status {
        AnnotationStatus::Resolved => ensure!(
            record_count > 0 && reason.is_none(),
            "resolved annotation has invalid shape"
        ),
        AnnotationStatus::Unresolved => ensure!(
            record_count == 0 && reason.is_some(),
            "unresolved annotation has invalid shape"
        ),
    }
    Ok(())
}

fn validate_claim(claim: &Claim, text: &str) -> Result<()> {
    validate_span_list(&claim.support_spans, text, "claim support")?;
    validate_semantic_field(&claim.subject, text, &claim.support_spans, "claim subject")?;
    validate_semantic_field(
        &claim.relation,
        text,
        &claim.support_spans,
        "claim relation",
    )?;
    validate_semantic_field(
        &claim.object_type,
        text,
        &claim.support_spans,
        "claim object type",
    )?;
    validate_semantic_field(
        &claim.object_value,
        text,
        &claim.support_spans,
        "claim object",
    )?;
    validate_qualifiers(&claim.qualifiers, text, &claim.support_spans, "claim")?;
    validate_identifier_set(&claim.lifecycles, "claim lifecycle")?;
    Ok(())
}

fn validate_need(need: &Need, text: &str) -> Result<()> {
    validate_span_list(&need.support_spans, text, "need support")?;
    validate_semantic_field(&need.subject, text, &need.support_spans, "need subject")?;
    validate_semantic_field(&need.relation, text, &need.support_spans, "need relation")?;
    ensure!(
        !need.requested_object_types.is_empty(),
        "need has no requested object type"
    );
    for object_type in &need.requested_object_types {
        validate_semantic_field(
            object_type,
            text,
            &need.support_spans,
            "requested object type",
        )?;
    }
    let object_types = need
        .requested_object_types
        .iter()
        .map(|field| field.normalized.as_str())
        .collect::<HashSet<_>>();
    ensure!(
        object_types.len() == need.requested_object_types.len(),
        "requested object types contain duplicate normalized values"
    );
    validate_qualifiers(&need.required_qualifiers, text, &need.support_spans, "need")?;
    ensure_unique(&need.allowed_polarities, "allowed polarities")?;
    ensure!(
        !need.allowed_polarities.is_empty(),
        "need has no allowed polarity"
    );
    validate_identifier_set(&need.required_lifecycles, "required lifecycle")?;
    ensure_unique(&need.allowed_provenances, "allowed provenances")?;
    ensure!(
        !need.allowed_provenances.is_empty(),
        "need has no allowed provenance"
    );
    Ok(())
}

fn validate_qualifiers(
    qualifiers: &[Qualifier],
    text: &str,
    support_spans: &[TextSpan],
    kind: &str,
) -> Result<()> {
    let mut identities = HashSet::new();
    for qualifier in qualifiers {
        validate_semantic_field(
            &qualifier.name,
            text,
            support_spans,
            &format!("{kind} qualifier name"),
        )?;
        validate_semantic_field(
            &qualifier.value,
            text,
            support_spans,
            &format!("{kind} qualifier value"),
        )?;
        ensure!(
            identities.insert(qualifier.identity()),
            "duplicate {kind} qualifier"
        );
    }
    Ok(())
}

fn validate_semantic_field(
    field: &SemanticField,
    text: &str,
    support_spans: &[TextSpan],
    kind: &str,
) -> Result<()> {
    validate_identifier(&field.normalized, kind)?;
    validate_span_list(&field.spans, text, kind)?;
    for span in &field.spans {
        ensure!(
            support_spans
                .iter()
                .any(|support| support.start <= span.start && span.end <= support.end),
            "{kind} span is outside its support span"
        );
    }
    Ok(())
}

fn validate_span_list(spans: &[TextSpan], text: &str, kind: &str) -> Result<()> {
    ensure!(!spans.is_empty(), "{kind} has no spans");
    let mut previous_end = None;
    let mut unique = HashSet::new();
    for span in spans {
        let start = usize::try_from(span.start).context("span start is too large")?;
        let end = usize::try_from(span.end).context("span end is too large")?;
        ensure!(
            start < end && end <= text.len(),
            "{kind} span is out of bounds"
        );
        ensure!(
            text.is_char_boundary(start) && text.is_char_boundary(end),
            "{kind} span is not on UTF-8 boundaries"
        );
        let selected = text
            .get(start..end)
            .context("validated span is unexpectedly unavailable")?;
        ensure!(
            !selected.trim().is_empty(),
            "{kind} span selects only whitespace"
        );
        ensure!(unique.insert(*span), "{kind} repeats a span");
        if let Some(previous) = previous_end {
            ensure!(
                previous <= span.start,
                "{kind} spans overlap or are unsorted"
            );
        }
        previous_end = Some(span.end);
    }
    Ok(())
}

fn validate_scoring_key(
    key: &ScoringKey,
    contract: &ExperimentContract,
    sources: &HashMap<&str, &SourceInput>,
    questions: &HashMap<&str, &QuestionInput>,
) -> Result<()> {
    ensure!(
        key.schema_version == SCHEMA_VERSION,
        "unsupported scoring key schema"
    );
    let mut case_ids = HashSet::new();
    let mut question_ids = HashSet::new();
    let mut source_splits = HashMap::<&str, Split>::new();
    let mut referenced_sources = HashSet::<&str>::new();
    let mut domains = BTreeMap::<Split, BTreeSet<&str>>::new();
    let mut cases = BTreeMap::<Split, usize>::new();
    let mut no_answer = BTreeMap::<Split, usize>::new();
    let mut compound = BTreeMap::<Split, usize>::new();
    let mut tags = BTreeMap::<Split, BTreeSet<CaseTag>>::new();

    for case in &key.cases {
        validate_identifier(&case.case_id, "case")?;
        validate_identifier(&case.domain_id, "domain")?;
        validate_identifier(&case.question_id, "case question")?;
        ensure!(case_ids.insert(case.case_id.as_str()), "duplicate case id");
        ensure!(
            question_ids.insert(case.question_id.as_str()),
            "question belongs to multiple cases"
        );
        ensure!(
            questions.contains_key(case.question_id.as_str()),
            "case references unknown question"
        );
        ensure!(!case.source_pools.is_empty(), "case has no source pools");
        ensure_unique(&case.tags, "case tags")?;
        tags.entry(case.split)
            .or_default()
            .extend(case.tags.iter().copied());
        *cases.entry(case.split).or_default() += 1;
        domains
            .entry(case.split)
            .or_default()
            .insert(case.domain_id.as_str());
        if case.expected_groups.is_empty() {
            *no_answer.entry(case.split).or_default() += 1;
        }
        if case.tags.contains(&CaseTag::Compound) {
            ensure!(
                case.expected_groups.len() >= 2,
                "compound case must require multiple evidence groups"
            );
            *compound.entry(case.split).or_default() += 1;
        }
        if case.tags.contains(&CaseTag::Conflict) {
            ensure!(
                case.expected_groups.len() >= 2,
                "conflict case must preserve multiple evidence groups"
            );
        }

        let mut authorized = HashSet::new();
        let mut pool_ids = HashSet::new();
        for pool in &case.source_pools {
            validate_identifier(&pool.pool_id, "pool")?;
            ensure!(
                pool_ids.insert(pool.pool_id.as_str()),
                "duplicate pool id in case"
            );
            ensure!(
                !pool.candidate_source_ids.is_empty(),
                "source pool is empty"
            );
            ensure!(
                pool.candidate_source_ids.len() <= contract.candidate_pool_limit,
                "source pool exceeds frozen limit"
            );
            for source_id in &pool.candidate_source_ids {
                validate_identifier(source_id, "candidate source")?;
                ensure!(
                    sources.contains_key(source_id.as_str()),
                    "pool references unknown source"
                );
                ensure!(
                    authorized.insert(source_id.as_str()),
                    "candidate repeats across pools"
                );
                assign_source_split(
                    &mut source_splits,
                    &mut referenced_sources,
                    source_id,
                    case.split,
                )?;
            }
        }
        validate_case_labels(case, sources)?;
        for source_id in case
            .relevant_source_ids
            .iter()
            .chain(&case.allowed_support_source_ids)
            .chain(&case.forbidden_source_ids)
        {
            assign_source_split(
                &mut source_splits,
                &mut referenced_sources,
                source_id,
                case.split,
            )?;
        }
        let relevant = case
            .relevant_source_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        for group in &case.expected_groups {
            ensure!(!group.is_empty(), "expected group is empty");
            ensure!(
                group.iter().all(|id| relevant.contains(id.as_str())),
                "expected group contains non-relevant source"
            );
        }
        ensure!(
            case.relevant_source_ids
                .iter()
                .chain(&case.allowed_support_source_ids)
                .all(|source_id| authorized.contains(source_id.as_str())),
            "relevant or support evidence is outside the authorized pool"
        );
        ensure!(
            case.forbidden_source_ids
                .iter()
                .all(|source_id| !authorized.contains(source_id.as_str())),
            "forbidden evidence entered an authorized pool"
        );
    }

    ensure!(
        question_ids.len() == questions.len(),
        "every question must belong to one case"
    );
    ensure!(
        referenced_sources.len() == sources.len(),
        "every source must be referenced by a case"
    );
    for split in Split::ALL {
        ensure!(
            domains.get(&split).map_or(0, BTreeSet::len) >= contract.minimum_domains_per_split,
            "split has too few domains"
        );
        ensure!(
            *cases.get(&split).unwrap_or(&0) >= contract.minimum_cases_per_split,
            "split has too few cases"
        );
        ensure!(
            *no_answer.get(&split).unwrap_or(&0) >= contract.minimum_no_answer_cases_per_split,
            "split has too few no-answer cases"
        );
        ensure!(
            *compound.get(&split).unwrap_or(&0) >= contract.minimum_compound_cases_per_split,
            "split has too few compound cases"
        );
        ensure!(
            REQUIRED_CASE_TAGS.iter().all(|required| tags
                .get(&split)
                .is_some_and(|observed| observed.contains(required))),
            "split is missing required taxonomy coverage"
        );
    }
    let development_domains = domains
        .get(&Split::Development)
        .context("development domains missing")?;
    let promotion_domains = domains
        .get(&Split::Promotion)
        .context("promotion domains missing")?;
    ensure!(
        development_domains.is_disjoint(promotion_domains),
        "domains leak across splits"
    );
    Ok(())
}

fn assign_source_split<'a>(
    source_splits: &mut HashMap<&'a str, Split>,
    referenced_sources: &mut HashSet<&'a str>,
    source_id: &'a str,
    split: Split,
) -> Result<()> {
    referenced_sources.insert(source_id);
    if let Some(previous_split) = source_splits.insert(source_id, split) {
        ensure!(
            previous_split == split,
            "source leaks across development and promotion"
        );
    }
    Ok(())
}

fn validate_case_labels(case: &CaseKey, sources: &HashMap<&str, &SourceInput>) -> Result<()> {
    for ids in [
        case.relevant_source_ids.as_slice(),
        case.allowed_support_source_ids.as_slice(),
        case.forbidden_source_ids.as_slice(),
    ] {
        ensure_unique(ids, "case source labels")?;
        ensure!(
            ids.iter().all(|id| sources.contains_key(id.as_str())),
            "case label references unknown source"
        );
    }
    let relevant = case.relevant_source_ids.iter().collect::<HashSet<_>>();
    let support = case
        .allowed_support_source_ids
        .iter()
        .collect::<HashSet<_>>();
    let forbidden = case.forbidden_source_ids.iter().collect::<HashSet<_>>();
    ensure!(
        relevant.is_disjoint(&support)
            && relevant.is_disjoint(&forbidden)
            && support.is_disjoint(&forbidden),
        "case labels overlap"
    );
    let mut grouped = HashSet::new();
    for group in &case.expected_groups {
        for source_id in group {
            ensure!(grouped.insert(source_id), "expected groups overlap");
        }
    }
    ensure!(
        grouped == relevant,
        "expected groups must cover every relevant source exactly once"
    );
    if case.expected_groups.is_empty() {
        ensure!(
            case.relevant_source_ids.is_empty(),
            "no-answer case has relevant sources"
        );
    }
    Ok(())
}

fn build_report(package: &LoadedPackage) -> Result<SemanticReport> {
    let run = package
        .runs
        .get(&Platform::MacosArm64)
        .context("validated macOS run missing")?;
    let source_agreement = source_annotation_agreement(package, run)?;
    let question_agreement = question_annotation_agreement(package, run)?;
    let unresolved_source_records = run
        .source_annotations
        .iter()
        .filter(|record| record.status == AnnotationStatus::Unresolved)
        .count();
    let unresolved_question_records = run
        .question_annotations
        .iter()
        .filter(|record| record.status == AnnotationStatus::Unresolved)
        .count();
    let treatment = evaluate(package, run, EvaluationArm::Treatment)?;
    let structure_only = evaluate(package, run, EvaluationArm::StructureOnly)?;
    let mut best_assignment = None;
    for permutation in 0..package.contract.assignment_permutations {
        let metrics = evaluate(
            package,
            run,
            EvaluationArm::AssignmentPermutation(permutation),
        )?;
        if best_assignment
            .as_ref()
            .is_none_or(|current: &EvaluationMetrics| {
                metrics.overall.exact_case_rate > current.overall.exact_case_rate
            })
        {
            best_assignment = Some(metrics);
        }
    }
    let best_assignment = best_assignment.context("assignment controls are missing")?;
    let control_exact = structure_only
        .overall
        .exact_case_rate
        .max(best_assignment.overall.exact_case_rate);
    let control_delta = treatment.overall.exact_case_rate - control_exact;
    let cross_platform_identical = validate_cross_platform_outputs(&package.runs).is_ok();
    let passed = cross_platform_identical
        && unresolved_source_records == 0
        && unresolved_question_records == 0
        && agreement_passes(
            &source_agreement,
            package.contract.minimum_annotation_exact_rate,
        )
        && agreement_passes(
            &question_agreement,
            package.contract.minimum_annotation_exact_rate,
        )
        && metrics_pass(&treatment, &package.contract)
        && control_delta >= package.contract.minimum_control_delta;

    Ok(SemanticReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment_id: EXPERIMENT_ID.to_owned(),
        contract_sha256: package.contract_sha256.clone(),
        candidate_revision_sha256: package.manifest.candidate_revision_sha256.clone(),
        source_canonical_sha256: run.source_canonical_sha256.clone(),
        question_canonical_sha256: run.question_canonical_sha256.clone(),
        cross_platform_identical,
        unresolved_source_records,
        unresolved_question_records,
        source_annotation_agreement: source_agreement,
        question_annotation_agreement: question_agreement,
        treatment,
        structure_only,
        best_assignment_permutation: best_assignment,
        control_delta,
        passed,
    })
}

fn metrics_pass(metrics: &EvaluationMetrics, contract: &ExperimentContract) -> bool {
    [&metrics.overall, &metrics.development, &metrics.promotion]
        .iter()
        .all(|split| {
            split.recall_at_five >= contract.minimum_recall_at_five
                && split.exact_case_rate >= contract.minimum_exact_case_rate
        })
        && metrics.unexpected_evidence == 0
        && metrics.forbidden_evidence == 0
        && metrics.authorization_errors == 0
        && metrics.provenance_errors == 0
        && metrics.duplicate_errors == 0
        && metrics.stability_errors == 0
        && metrics.compound_partial_errors == 0
        && metrics.conflict_coverage_errors == 0
}

fn evaluate(
    package: &LoadedPackage,
    run: &ValidatedRun,
    arm: EvaluationArm,
) -> Result<EvaluationMetrics> {
    let sources = run
        .source_annotations
        .iter()
        .map(|record| (record.source_id.as_str(), record))
        .collect::<HashMap<_, _>>();
    let questions = run
        .question_annotations
        .iter()
        .map(|record| (record.question_id.as_str(), record))
        .collect::<HashMap<_, _>>();
    let mut metrics = EvaluationMetrics::default();
    for case in &package.scoring_key.cases {
        let question = questions
            .get(case.question_id.as_str())
            .context("case question annotation missing")?;
        let returned = match arm {
            EvaluationArm::Treatment => match_case(
                case,
                question,
                &sources,
                package.contract.top_k_per_source,
                None,
            )?,
            EvaluationArm::StructureOnly => {
                structure_only(case, question, &sources, package.contract.top_k_per_source)?
            }
            EvaluationArm::AssignmentPermutation(index) => match_case(
                case,
                question,
                &sources,
                package.contract.top_k_per_source,
                Some(index),
            )?,
        };
        score_case(&mut metrics, case, &returned);
    }
    finalize_metrics(&mut metrics);
    Ok(metrics)
}

fn match_case(
    case: &CaseKey,
    question: &QuestionAnnotation,
    sources: &HashMap<&str, &SourceAnnotation>,
    top_k: usize,
    permutation: Option<usize>,
) -> Result<Vec<String>> {
    if question.status == AnnotationStatus::Unresolved {
        return Ok(Vec::new());
    }
    let mut returned = Vec::new();
    let mut covered_needs = BTreeSet::new();
    for pool in &case.source_pools {
        let claims_by_candidate = permuted_claims(pool, sources, permutation)?;
        let mut seen_edges = BTreeSet::<(usize, String)>::new();
        let mut retained = 0;
        for source_id in &pool.candidate_source_ids {
            let claims = claims_by_candidate
                .get(source_id.as_str())
                .context("candidate claims missing")?;
            let mut new_edges = BTreeSet::new();
            for (need_index, need) in question.needs.iter().enumerate() {
                for claim in *claims {
                    if claim_matches_need(claim, need) {
                        let edge = (need_index, claim.object_value.normalized.clone());
                        if !seen_edges.contains(&edge) {
                            new_edges.insert(edge);
                        }
                    }
                }
            }
            if !new_edges.is_empty() {
                covered_needs.extend(new_edges.iter().map(|(need_index, _)| *need_index));
                seen_edges.extend(new_edges);
                returned.push(source_id.clone());
                retained += 1;
                if retained == top_k {
                    break;
                }
            }
        }
    }
    if covered_needs.len() == question.needs.len() {
        Ok(returned)
    } else {
        Ok(Vec::new())
    }
}

fn permuted_claims<'a>(
    pool: &SourcePool,
    sources: &'a HashMap<&str, &SourceAnnotation>,
    permutation: Option<usize>,
) -> Result<HashMap<&'a str, &'a [Claim]>> {
    let mut result = HashMap::new();
    let count = pool.candidate_source_ids.len();
    for (index, source_id) in pool.candidate_source_ids.iter().enumerate() {
        let source = sources
            .get(source_id.as_str())
            .context("source annotation missing")?;
        let claims: &[Claim] = if let Some(permutation_index) = permutation {
            if count <= 1 {
                &[]
            } else {
                let shift = 1 + permutation_index % (count - 1);
                let donor_index = (index + shift) % count;
                let donor_id = pool
                    .candidate_source_ids
                    .get(donor_index)
                    .context("permutation donor missing")?;
                &sources
                    .get(donor_id.as_str())
                    .context("permutation annotation missing")?
                    .claims
            }
        } else {
            &source.claims
        };
        result.insert(source.source_id.as_str(), claims);
    }
    Ok(result)
}

fn structure_only(
    case: &CaseKey,
    question: &QuestionAnnotation,
    sources: &HashMap<&str, &SourceAnnotation>,
    top_k: usize,
) -> Result<Vec<String>> {
    if question.status == AnnotationStatus::Unresolved {
        return Ok(Vec::new());
    }
    let mut returned = Vec::new();
    let mut claim_count = 0;
    for pool in &case.source_pools {
        let mut retained = 0;
        for source_id in &pool.candidate_source_ids {
            let source = sources
                .get(source_id.as_str())
                .context("source annotation missing")?;
            if !source.claims.is_empty() {
                returned.push(source_id.clone());
                claim_count += source.claims.len();
                retained += 1;
                if retained == top_k {
                    break;
                }
            }
        }
    }
    if claim_count >= question.needs.len() {
        Ok(returned)
    } else {
        Ok(Vec::new())
    }
}

fn claim_matches_need(claim: &Claim, need: &Need) -> bool {
    claim.subject.normalized == need.subject.normalized
        && claim.relation.normalized == need.relation.normalized
        && need
            .requested_object_types
            .iter()
            .any(|kind| kind.normalized == claim.object_type.normalized)
        && need.required_qualifiers.iter().all(|required| {
            claim
                .qualifiers
                .iter()
                .any(|candidate| candidate.identity() == required.identity())
        })
        && need
            .required_lifecycles
            .iter()
            .all(|required| claim.lifecycles.contains(required))
        && need.allowed_polarities.contains(&claim.polarity)
        && need.allowed_provenances.contains(&claim.provenance)
}

fn need_semantic_key(need: &Need) -> Result<String> {
    #[derive(Serialize)]
    struct Identity<'a> {
        subject: &'a str,
        relation: &'a str,
        requested_object_types: Vec<&'a str>,
        required_qualifiers: Vec<(&'a str, &'a str)>,
        allowed_polarities: &'a [Polarity],
        required_lifecycles: &'a [String],
        allowed_provenances: &'a [Provenance],
    }

    let identity = Identity {
        subject: &need.subject.normalized,
        relation: &need.relation.normalized,
        requested_object_types: need
            .requested_object_types
            .iter()
            .map(|field| field.normalized.as_str())
            .collect(),
        required_qualifiers: need
            .required_qualifiers
            .iter()
            .map(Qualifier::identity)
            .collect(),
        allowed_polarities: &need.allowed_polarities,
        required_lifecycles: &need.required_lifecycles,
        allowed_provenances: &need.allowed_provenances,
    };
    let bytes = serde_json::to_vec(&identity).context("serializing semantic need identity")?;
    Ok(hash_bytes(&bytes))
}

fn score_case(metrics: &mut EvaluationMetrics, case: &CaseKey, returned: &[String]) {
    let expected = case.expected_groups.len();
    let found = case
        .expected_groups
        .iter()
        .filter(|group| group.iter().any(|id| returned.contains(id)))
        .count();
    let authorized = case
        .source_pools
        .iter()
        .flat_map(|pool| pool.candidate_source_ids.iter())
        .collect::<HashSet<_>>();
    let relevant = case.relevant_source_ids.iter().collect::<HashSet<_>>();
    let support = case
        .allowed_support_source_ids
        .iter()
        .collect::<HashSet<_>>();
    let forbidden = case.forbidden_source_ids.iter().collect::<HashSet<_>>();
    let unexpected = returned
        .iter()
        .filter(|id| !relevant.contains(id) && !support.contains(id))
        .count();
    let forbidden_count = returned.iter().filter(|id| forbidden.contains(id)).count();
    let authorization = returned
        .iter()
        .filter(|id| !authorized.contains(id))
        .count();
    let duplicate_ids = returned.len() - returned.iter().collect::<HashSet<_>>().len();
    let duplicate_groups = case
        .expected_groups
        .iter()
        .filter(|group| group.iter().filter(|id| returned.contains(id)).count() > 1)
        .count();
    let exact = if expected == 0 {
        returned.is_empty()
    } else {
        found == expected
            && unexpected == 0
            && forbidden_count == 0
            && authorization == 0
            && duplicate_ids == 0
            && duplicate_groups == 0
    };

    metrics.unexpected_evidence += unexpected;
    metrics.forbidden_evidence += forbidden_count;
    metrics.authorization_errors += authorization;
    metrics.duplicate_errors += duplicate_ids + duplicate_groups;
    if case.tags.contains(&CaseTag::Compound) && found > 0 && found < expected {
        metrics.compound_partial_errors += 1;
    }
    if case.tags.contains(&CaseTag::Conflict) && found != expected {
        metrics.conflict_coverage_errors += 1;
    }
    add_case(&mut metrics.overall, found, expected, exact);
    match case.split {
        Split::Development => add_case(&mut metrics.development, found, expected, exact),
        Split::Promotion => add_case(&mut metrics.promotion, found, expected, exact),
    }
}

fn add_case(metrics: &mut SplitMetrics, found: usize, expected: usize, exact: bool) {
    metrics.found_groups += found;
    metrics.expected_groups += expected;
    metrics.cases += 1;
    metrics.exact_cases += usize::from(exact);
}

fn finalize_metrics(metrics: &mut EvaluationMetrics) {
    for split in [
        &mut metrics.overall,
        &mut metrics.development,
        &mut metrics.promotion,
    ] {
        split.recall_at_five = ratio(split.found_groups, split.expected_groups);
        split.exact_case_rate = ratio(split.exact_cases, split.cases);
    }
}

fn source_annotation_agreement(
    package: &LoadedPackage,
    run: &ValidatedRun,
) -> Result<AnnotationAgreement> {
    let candidate = run
        .source_annotations
        .iter()
        .map(|record| (record.source_id.as_str(), record))
        .collect::<HashMap<_, _>>();
    let gold = package
        .source_gold
        .iter()
        .map(|record| (record.source_id.as_str(), record))
        .collect::<HashMap<_, _>>();
    let mut by_split = BTreeMap::<Split, BTreeSet<&str>>::new();
    for case in &package.scoring_key.cases {
        let ids = by_split.entry(case.split).or_default();
        for source_id in case
            .source_pools
            .iter()
            .flat_map(|pool| pool.candidate_source_ids.iter())
            .chain(&case.forbidden_source_ids)
        {
            ids.insert(source_id.as_str());
        }
    }
    Ok(AnnotationAgreement {
        overall: exact_rate(&run.source_annotations, &package.source_gold),
        development: exact_rate_for_ids(
            &candidate,
            &gold,
            by_split
                .get(&Split::Development)
                .context("development source ids are missing")?,
        )?,
        promotion: exact_rate_for_ids(
            &candidate,
            &gold,
            by_split
                .get(&Split::Promotion)
                .context("promotion source ids are missing")?,
        )?,
    })
}

fn question_annotation_agreement(
    package: &LoadedPackage,
    run: &ValidatedRun,
) -> Result<AnnotationAgreement> {
    let candidate = run
        .question_annotations
        .iter()
        .map(|record| (record.question_id.as_str(), record))
        .collect::<HashMap<_, _>>();
    let gold = package
        .question_gold
        .iter()
        .map(|record| (record.question_id.as_str(), record))
        .collect::<HashMap<_, _>>();
    let mut by_split = BTreeMap::<Split, BTreeSet<&str>>::new();
    for case in &package.scoring_key.cases {
        by_split
            .entry(case.split)
            .or_default()
            .insert(case.question_id.as_str());
    }
    Ok(AnnotationAgreement {
        overall: exact_rate(&run.question_annotations, &package.question_gold),
        development: exact_rate_for_ids(
            &candidate,
            &gold,
            by_split
                .get(&Split::Development)
                .context("development question ids are missing")?,
        )?,
        promotion: exact_rate_for_ids(
            &candidate,
            &gold,
            by_split
                .get(&Split::Promotion)
                .context("promotion question ids are missing")?,
        )?,
    })
}

fn exact_rate_for_ids<T: PartialEq>(
    candidate: &HashMap<&str, &T>,
    gold: &HashMap<&str, &T>,
    ids: &BTreeSet<&str>,
) -> Result<f64> {
    let mut exact = 0;
    for id in ids {
        let candidate_record = candidate
            .get(id)
            .with_context(|| format!("candidate annotation `{id}` is missing"))?;
        let gold_record = gold
            .get(id)
            .with_context(|| format!("gold annotation `{id}` is missing"))?;
        exact += usize::from(candidate_record == gold_record);
    }
    Ok(ratio(exact, ids.len()))
}

fn agreement_passes(agreement: &AnnotationAgreement, minimum: f64) -> bool {
    agreement.overall >= minimum
        && agreement.development >= minimum
        && agreement.promotion >= minimum
}

fn exact_rate<T: PartialEq>(candidate: &[T], gold: &[T]) -> f64 {
    let exact = candidate
        .iter()
        .zip(gold)
        .filter(|(left, right)| left == right)
        .count();
    ratio(exact, gold.len())
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn normalize_source_annotations(records: &mut [SourceAnnotation]) {
    for record in records.iter_mut() {
        record.normalize();
    }
    records.sort_by(|left, right| left.source_id.cmp(&right.source_id));
}

fn normalize_question_annotations(records: &mut [QuestionAnnotation]) {
    for record in records.iter_mut() {
        record.normalize();
    }
    records.sort_by(|left, right| left.question_id.cmp(&right.question_id));
}

fn canonical_jsonl<T: Serialize>(records: &[T]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for record in records {
        serde_json::to_writer(&mut bytes, record).context("serializing canonical JSONL record")?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).context("serializing semantic report")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_report(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("report path has no parent")?;
    std::fs::create_dir_all(parent).context("creating report directory")?;
    let temporary = parent.join(format!(
        ".typed-evidence-v3-{}-{}.tmp",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&temporary, bytes).context("writing temporary semantic report")?;
    replace_file(&temporary, path).context("installing semantic report")
}

fn load_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    ensure_regular_bounded_file(path)?;
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

fn load_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    ensure_regular_bounded_file(path)?;
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut records = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line_number = index + 1;
        let line =
            line.with_context(|| format!("reading line {line_number} from {}", path.display()))?;
        ensure!(
            !line.trim().is_empty(),
            "{} contains a blank line at {line_number}",
            path.display()
        );
        let record = serde_json::from_str(&line)
            .with_context(|| format!("parsing line {line_number} from {}", path.display()))?;
        records.push(record);
        ensure!(
            records.len() <= MAX_RECORDS,
            "{} has too many records",
            path.display()
        );
    }
    ensure!(!records.is_empty(), "{} is empty", path.display());
    Ok(records)
}

fn package_file(directory: &Path, relative: &str) -> Result<PathBuf> {
    let path = directory.join(relative);
    ensure_regular_bounded_file(&path)?;
    Ok(path)
}

fn ensure_regular_bounded_file(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("reading metadata for {}", path.display()))?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "{} is a symlink",
        path.display()
    );
    ensure!(
        metadata.is_file(),
        "{} is not a regular file",
        path.display()
    );
    ensure!(
        metadata.len() <= MAX_PACKAGE_FILE_BYTES,
        "{} exceeds the file limit",
        path.display()
    );
    Ok(())
}

fn hash_file(path: &Path) -> Result<String> {
    ensure_regular_bounded_file(path)?;
    let bytes =
        std::fs::read(path).with_context(|| format!("reading {} for hashing", path.display()))?;
    Ok(hash_bytes(&bytes))
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn validate_sha256(value: &str, kind: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "{kind} is not a lowercase SHA-256"
    );
    Ok(())
}

fn validate_identifier(value: &str, kind: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= MAX_IDENTIFIER_BYTES,
        "{kind} identifier has an invalid length"
    );
    let mut bytes = value.bytes();
    let first = bytes.next().context("identifier is unexpectedly empty")?;
    ensure!(
        first.is_ascii_lowercase(),
        "{kind} identifier must start with lowercase ASCII"
    );
    ensure!(
        bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "{kind} identifier contains invalid characters"
    );
    ensure!(
        !value.ends_with('_') && !value.contains("__"),
        "{kind} identifier is not canonical"
    );
    Ok(())
}

fn validate_opaque_identifier(value: &str, prefix: &str, kind: &str) -> Result<()> {
    validate_identifier(value, kind)?;
    let suffix = value
        .strip_prefix(prefix)
        .with_context(|| format!("{kind} identifier lacks its opaque prefix"))?;
    ensure!(
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{kind} identifier is not an opaque 128-bit token"
    );
    Ok(())
}

fn validate_identifier_set(values: &[String], kind: &str) -> Result<()> {
    ensure!(!values.is_empty(), "{kind} set is empty");
    for value in values {
        validate_identifier(value, kind)?;
    }
    ensure_unique(values, kind)
}

fn validate_text(value: &str, kind: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{kind} is empty");
    ensure!(value.len() <= MAX_TEXT_BYTES, "{kind} is too large");
    ensure!(!value.contains('\0'), "{kind} contains a NUL byte");
    Ok(())
}

fn ensure_unique<T: Eq + std::hash::Hash>(values: &[T], kind: &str) -> Result<()> {
    let unique = values.iter().collect::<HashSet<_>>();
    ensure!(unique.len() == values.len(), "{kind} contains duplicates");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    fn span(start: u32, end: u32) -> TextSpan {
        TextSpan { start, end }
    }

    fn field(normalized: &str, start: u32, end: u32) -> SemanticField {
        SemanticField {
            normalized: normalized.to_owned(),
            spans: vec![span(start, end)],
        }
    }

    fn field_in(text: &str, normalized: &str, needle: &str) -> Result<SemanticField> {
        let start = text
            .find(needle)
            .with_context(|| format!("`{needle}` is missing from synthetic text"))?;
        let end = start + needle.len();
        Ok(field(
            normalized,
            u32::try_from(start).context("synthetic span start overflow")?,
            u32::try_from(end).context("synthetic span end overflow")?,
        ))
    }

    fn opaque_fragment(seed: &str, length: usize) -> Result<String> {
        let digest = hash_bytes(seed.as_bytes());
        digest
            .get(..length)
            .map(ToOwned::to_owned)
            .context("synthetic opaque fragment is unavailable")
    }

    fn opaque_id(prefix: &str, seed: &str) -> Result<String> {
        Ok(format!("{prefix}_{}", opaque_fragment(seed, 32)?))
    }

    fn whole_span(text: &str) -> Result<Vec<TextSpan>> {
        Ok(vec![span(
            0,
            u32::try_from(text.len()).context("synthetic text is too large")?,
        )])
    }

    fn claim(
        text: &str,
        subject: &str,
        relation: &str,
        relation_text: &str,
        object_type: &str,
        object_value: &str,
    ) -> Result<Claim> {
        Ok(Claim {
            subject: field_in(text, subject, subject)?,
            relation: field_in(text, relation, relation_text)?,
            object_type: field_in(text, object_type, object_value)?,
            object_value: field_in(text, object_value, object_value)?,
            qualifiers: Vec::new(),
            polarity: Polarity::Positive,
            lifecycles: vec!["current".to_owned()],
            provenance: Provenance::Direct,
            support_spans: whole_span(text)?,
        })
    }

    fn need(
        question: &str,
        subject: &str,
        relation: &str,
        relation_text: &str,
        object_type: &str,
        type_text: &str,
    ) -> Result<Need> {
        Ok(Need {
            subject: field_in(question, subject, subject)?,
            relation: field_in(question, relation, relation_text)?,
            requested_object_types: vec![field_in(question, object_type, type_text)?],
            required_qualifiers: Vec::new(),
            allowed_polarities: vec![Polarity::Positive],
            required_lifecycles: vec!["current".to_owned()],
            allowed_provenances: vec![Provenance::Direct, Provenance::Attributed],
            support_spans: whole_span(question)?,
        })
    }

    fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
        let parent = path.parent().context("synthetic JSON path has no parent")?;
        std::fs::create_dir_all(parent).context("creating synthetic JSON directory")?;
        let mut bytes = serde_json::to_vec_pretty(value).context("serializing synthetic JSON")?;
        bytes.push(b'\n');
        std::fs::write(path, bytes).context("writing synthetic JSON")
    }

    fn write_jsonl<T: Serialize>(path: &Path, values: &[T]) -> Result<()> {
        let parent = path
            .parent()
            .context("synthetic JSONL path has no parent")?;
        std::fs::create_dir_all(parent).context("creating synthetic JSONL directory")?;
        std::fs::write(path, canonical_jsonl(values)?).context("writing synthetic JSONL")
    }

    struct SyntheticCorpus {
        sources: Vec<SourceInput>,
        questions: Vec<QuestionInput>,
        source_annotations: Vec<SourceAnnotation>,
        question_annotations: Vec<QuestionAnnotation>,
        scoring_key: ScoringKey,
    }

    fn synthetic_corpus() -> Result<SyntheticCorpus> {
        let mut sources = Vec::new();
        let mut questions = Vec::new();
        let mut source_annotations = Vec::new();
        let mut question_annotations = Vec::new();
        let mut cases = Vec::new();

        for split in Split::ALL {
            let split_name = match split {
                Split::Development => "development",
                Split::Promotion => "promotion",
            };
            for ordinal in 0..12 {
                let seed = format!("{split_name}-{ordinal}");
                let entity = format!("entity_{}", opaque_fragment(&seed, 12)?);
                let case_id = format!("{split_name}_case_{ordinal}");
                let question_id = opaque_id("qry", &format!("{seed}-question"))?;
                let pool_id = format!("{split_name}_pool_{ordinal}");
                let domain_id = format!("{split_name}_domain_{}", ordinal % 4);
                let is_no_answer = ordinal < 3;
                let is_compound = ordinal == 3 || ordinal == 4;
                let question_text = if is_compound {
                    format!("{entity} owner who target when")
                } else {
                    format!("{entity} owner who")
                };
                let mut needs = vec![need(
                    &question_text,
                    &entity,
                    "responsible_party",
                    "owner",
                    "person",
                    "who",
                )?];
                if is_compound {
                    needs.push(need(
                        &question_text,
                        &entity,
                        "target_date",
                        "target",
                        "date",
                        "when",
                    )?);
                }
                questions.push(QuestionInput {
                    question_id: question_id.clone(),
                    question: question_text,
                });
                question_annotations.push(QuestionAnnotation {
                    question_id: question_id.clone(),
                    status: AnnotationStatus::Resolved,
                    needs,
                    reason_code: None,
                });

                let negative_id = opaque_id("src", &format!("{seed}-negative"))?;
                let negative_text = format!("{entity} status paused");
                sources.push(SourceInput {
                    source_id: negative_id.clone(),
                    title: "Synthetic evidence".to_owned(),
                    heading: "Current state".to_owned(),
                    text: negative_text.clone(),
                });
                source_annotations.push(SourceAnnotation {
                    source_id: negative_id.clone(),
                    status: AnnotationStatus::Resolved,
                    claims: vec![claim(
                        &negative_text,
                        &entity,
                        "current_status",
                        "status",
                        "status",
                        "paused",
                    )?],
                    reason_code: None,
                });

                let mut candidate_source_ids = vec![negative_id];
                let mut relevant_source_ids = Vec::new();
                let mut expected_groups = Vec::new();
                if !is_no_answer {
                    let owner_id = opaque_id("src", &format!("{seed}-owner"))?;
                    let owner_value =
                        format!("person_{}", opaque_fragment(&format!("{seed}-person"), 12)?);
                    let owner_text = format!("{entity} owner {owner_value}");
                    sources.push(SourceInput {
                        source_id: owner_id.clone(),
                        title: "Synthetic evidence".to_owned(),
                        heading: "Responsible party".to_owned(),
                        text: owner_text.clone(),
                    });
                    source_annotations.push(SourceAnnotation {
                        source_id: owner_id.clone(),
                        status: AnnotationStatus::Resolved,
                        claims: vec![claim(
                            &owner_text,
                            &entity,
                            "responsible_party",
                            "owner",
                            "person",
                            &owner_value,
                        )?],
                        reason_code: None,
                    });
                    candidate_source_ids.push(owner_id.clone());
                    relevant_source_ids.push(owner_id.clone());
                    expected_groups.push(vec![owner_id]);
                }
                if is_compound {
                    let date_id = opaque_id("src", &format!("{seed}-date"))?;
                    let date_value = format!("date_2026_09_{:02}", ordinal + 1);
                    let date_text = format!("{entity} target {date_value}");
                    sources.push(SourceInput {
                        source_id: date_id.clone(),
                        title: "Synthetic evidence".to_owned(),
                        heading: "Target date".to_owned(),
                        text: date_text.clone(),
                    });
                    source_annotations.push(SourceAnnotation {
                        source_id: date_id.clone(),
                        status: AnnotationStatus::Resolved,
                        claims: vec![claim(
                            &date_text,
                            &entity,
                            "target_date",
                            "target",
                            "date",
                            &date_value,
                        )?],
                        reason_code: None,
                    });
                    candidate_source_ids.push(date_id.clone());
                    relevant_source_ids.push(date_id.clone());
                    expected_groups.push(vec![date_id]);
                }

                cases.push(CaseKey {
                    case_id,
                    domain_id,
                    split,
                    question_id,
                    tags: match ordinal {
                        0..=2 => vec![CaseTag::Absence],
                        3 => vec![CaseTag::Compound, CaseTag::Conflict],
                        4 => vec![CaseTag::Compound],
                        5 => vec![CaseTag::Paraphrase],
                        6 => vec![CaseTag::CrossLanguage],
                        7 => vec![CaseTag::Privacy],
                        8 => vec![CaseTag::Injection],
                        9 => vec![CaseTag::EntityAmbiguity],
                        _ => vec![CaseTag::Direct],
                    },
                    source_pools: vec![SourcePool {
                        pool_id,
                        candidate_source_ids,
                    }],
                    relevant_source_ids,
                    expected_groups,
                    allowed_support_source_ids: Vec::new(),
                    forbidden_source_ids: Vec::new(),
                });
            }
        }

        Ok(SyntheticCorpus {
            sources,
            questions,
            source_annotations,
            question_annotations,
            scoring_key: ScoringKey {
                schema_version: SCHEMA_VERSION,
                cases,
            },
        })
    }

    fn canonical_source_hash(records: &[SourceAnnotation]) -> Result<String> {
        let mut normalized = records.to_vec();
        normalize_source_annotations(&mut normalized);
        Ok(hash_bytes(&canonical_jsonl(&normalized)?))
    }

    fn canonical_question_hash(records: &[QuestionAnnotation]) -> Result<String> {
        let mut normalized = records.to_vec();
        normalize_question_annotations(&mut normalized);
        Ok(hash_bytes(&canonical_jsonl(&normalized)?))
    }

    fn write_run(
        directory: &Path,
        platform: Platform,
        contract_sha256: &str,
        source_input_sha256: &str,
        question_input_sha256: &str,
        sources: &[SourceAnnotation],
        questions: &[QuestionAnnotation],
    ) -> Result<()> {
        let prefix = directory.join("runs").join(platform.directory());
        let source_path = prefix.join("sources.jsonl");
        let question_path = prefix.join("questions.jsonl");
        let source_replay_path = prefix.join("sources-replay.jsonl");
        let question_replay_path = prefix.join("questions-replay.jsonl");
        let mut reversed_sources = sources.to_vec();
        reversed_sources.reverse();
        let mut reversed_questions = questions.to_vec();
        reversed_questions.reverse();
        let (primary_sources, replay_sources, primary_questions, replay_questions) = match platform
        {
            Platform::MacosArm64 => (
                sources,
                reversed_sources.as_slice(),
                questions,
                reversed_questions.as_slice(),
            ),
            Platform::WindowsX64 => (
                reversed_sources.as_slice(),
                sources,
                reversed_questions.as_slice(),
                questions,
            ),
        };
        write_jsonl(&source_path, primary_sources)?;
        write_jsonl(&question_path, primary_questions)?;
        write_jsonl(&source_replay_path, replay_sources)?;
        write_jsonl(&question_replay_path, replay_questions)?;
        let receipt = RunReceipt {
            schema_version: SCHEMA_VERSION,
            platform,
            contract_sha256: contract_sha256.to_owned(),
            candidate_revision_sha256: "a".repeat(64),
            binary_sha256: match platform {
                Platform::MacosArm64 => "b".repeat(64),
                Platform::WindowsX64 => "c".repeat(64),
            },
            source_input_sha256: source_input_sha256.to_owned(),
            question_input_sha256: question_input_sha256.to_owned(),
            source_output_sha256: hash_file(&source_path)?,
            question_output_sha256: hash_file(&question_path)?,
            source_canonical_sha256: canonical_source_hash(primary_sources)?,
            question_canonical_sha256: canonical_question_hash(primary_questions)?,
            source_replay_sha256: hash_file(&source_replay_path)?,
            question_replay_sha256: hash_file(&question_replay_path)?,
            source_replay_canonical_sha256: canonical_source_hash(replay_sources)?,
            question_replay_canonical_sha256: canonical_question_hash(replay_questions)?,
            stderr_sha256: EMPTY_SHA256.to_owned(),
            stderr_bytes: 0,
            source_exit_code: 0,
            source_replay_exit_code: 0,
            question_exit_code: 0,
            question_replay_exit_code: 0,
        };
        write_json(&prefix.join("receipt.json"), &receipt)
    }

    fn complete_package() -> Result<TempDir> {
        let temporary = tempfile::tempdir().context("creating synthetic package")?;
        let directory = temporary.path();
        let corpus = synthetic_corpus()?;
        let contract_path = workspace_root()
            .join(EXPERIMENT_DIRECTORY)
            .join(CONTRACT_FILE);
        let contract_sha256 = hash_file(&contract_path)?;
        write_json(
            &directory.join("manifest.json"),
            &PackageManifest {
                schema_version: SCHEMA_VERSION,
                experiment_id: EXPERIMENT_ID.to_owned(),
                contract_sha256: contract_sha256.clone(),
                candidate_revision_sha256: "a".repeat(64),
            },
        )?;
        let source_input_path = directory.join(INPUT_SOURCE_FILE);
        let question_input_path = directory.join(INPUT_QUESTION_FILE);
        let source_gold_path = directory.join(SOURCE_GOLD_FILE);
        let question_gold_path = directory.join(QUESTION_GOLD_FILE);
        let scoring_key_path = directory.join(SCORING_KEY_FILE);
        write_jsonl(&source_input_path, &corpus.sources)?;
        write_jsonl(&question_input_path, &corpus.questions)?;
        write_jsonl(&source_gold_path, &corpus.source_annotations)?;
        write_jsonl(&question_gold_path, &corpus.question_annotations)?;
        write_json(&scoring_key_path, &corpus.scoring_key)?;
        let source_input_sha256 = hash_file(&source_input_path)?;
        let question_input_sha256 = hash_file(&question_input_path)?;
        for platform in EXPECTED_PLATFORMS {
            write_run(
                directory,
                platform,
                &contract_sha256,
                &source_input_sha256,
                &question_input_sha256,
                &corpus.source_annotations,
                &corpus.question_annotations,
            )?;
        }
        write_json(
            &directory.join(REVIEW_RECEIPT_FILE),
            &ReviewReceipt {
                schema_version: SCHEMA_VERSION,
                contract_sha256,
                source_input_sha256,
                question_input_sha256,
                source_gold_sha256: hash_file(&source_gold_path)?,
                question_gold_sha256: hash_file(&question_gold_path)?,
                scoring_key_sha256: hash_file(&scoring_key_path)?,
                source_review_id: "review_source".to_owned(),
                question_review_id: "review_question".to_owned(),
                source_scope_was_isolated: true,
                question_scope_was_isolated: true,
                candidate_outputs_were_hidden: true,
                scoring_key_was_hidden: true,
                promotion_was_authored_after_freeze: true,
            },
        )?;
        Ok(temporary)
    }

    #[test]
    fn committed_contract_is_self_consistent() -> Result<()> {
        validate_contract()
    }

    #[test]
    fn annotation_order_is_canonicalized_without_changing_semantics() -> Result<()> {
        let mut first = SourceAnnotation {
            source_id: "source_a".to_owned(),
            status: AnnotationStatus::Resolved,
            claims: vec![
                Claim {
                    subject: field("harbor", 0, 6),
                    relation: field("target_date", 7, 11),
                    object_type: field("date", 12, 22),
                    object_value: field("date_2026_09_10", 12, 22),
                    qualifiers: Vec::new(),
                    polarity: Polarity::Positive,
                    lifecycles: vec!["planned".to_owned(), "current".to_owned()],
                    provenance: Provenance::Direct,
                    support_spans: vec![span(0, 22)],
                },
                Claim {
                    subject: field("harbor", 0, 6),
                    relation: field("current_status", 23, 29),
                    object_type: field("status", 30, 35),
                    object_value: field("ready", 30, 35),
                    qualifiers: Vec::new(),
                    polarity: Polarity::Positive,
                    lifecycles: vec!["current".to_owned()],
                    provenance: Provenance::Direct,
                    support_spans: vec![span(23, 35)],
                },
            ],
            reason_code: None,
        };
        let mut second = first.clone();
        second.claims.reverse();
        second
            .claims
            .get_mut(1)
            .context("synthetic second claim is missing")?
            .lifecycles
            .reverse();
        first.normalize();
        second.normalize();
        ensure!(
            first == second,
            "canonicalization did not erase order-only differences"
        );
        ensure!(canonical_jsonl(&[first])? == canonical_jsonl(&[second])?);
        Ok(())
    }

    #[test]
    fn exact_match_requires_entity_relation_type_and_state() {
        let claim = Claim {
            subject: field("harbor", 0, 6),
            relation: field("target_date", 7, 11),
            object_type: field("date", 12, 22),
            object_value: field("date_2026_09_10", 12, 22),
            qualifiers: Vec::new(),
            polarity: Polarity::Positive,
            lifecycles: vec!["planned".to_owned()],
            provenance: Provenance::Direct,
            support_spans: vec![span(0, 22)],
        };
        let mut need = Need {
            subject: field("harbor", 0, 6),
            relation: field("target_date", 7, 11),
            requested_object_types: vec![field("date", 12, 16)],
            required_qualifiers: Vec::new(),
            allowed_polarities: vec![Polarity::Positive],
            required_lifecycles: vec!["planned".to_owned()],
            allowed_provenances: vec![Provenance::Direct],
            support_spans: vec![span(0, 16)],
        };
        assert!(claim_matches_need(&claim, &need));
        need.subject.normalized = "atlas".to_owned();
        assert!(!claim_matches_need(&claim, &need));
        need.subject.normalized = "harbor".to_owned();
        need.required_lifecycles = vec!["current".to_owned()];
        assert!(!claim_matches_need(&claim, &need));
    }

    #[test]
    fn utf8_spans_must_use_byte_boundaries() {
        let text = "Atlas está listo";
        assert!(validate_span_list(&[span(0, 5)], text, "test").is_ok());
        assert!(validate_span_list(&[span(9, 10)], text, "test").is_err());
        assert!(validate_span_list(&[span(6, 11)], text, "test").is_ok());
    }

    #[test]
    fn visible_input_ids_must_be_opaque() {
        assert!(
            validate_opaque_identifier("src_0123456789abcdef0123456789abcdef", "src_", "source")
                .is_ok()
        );
        assert!(validate_opaque_identifier("src_promotion_answer", "src_", "source").is_err());
    }

    #[test]
    fn scoring_key_rejects_expected_evidence_outside_authorized_pool() -> Result<()> {
        let mut corpus = synthetic_corpus()?;
        let case = corpus
            .scoring_key
            .cases
            .iter_mut()
            .find(|case| !case.relevant_source_ids.is_empty())
            .context("synthetic answerable case is missing")?;
        let relevant = case
            .relevant_source_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let pool = case
            .source_pools
            .first_mut()
            .context("synthetic source pool is missing")?;
        pool.candidate_source_ids
            .retain(|source_id| !relevant.contains(source_id));
        let source_map = corpus
            .sources
            .iter()
            .map(|source| (source.source_id.as_str(), source))
            .collect::<HashMap<_, _>>();
        let question_map = corpus
            .questions
            .iter()
            .map(|question| (question.question_id.as_str(), question))
            .collect::<HashMap<_, _>>();
        let contract = load_json::<ExperimentContract>(
            &workspace_root()
                .join(EXPERIMENT_DIRECTORY)
                .join(CONTRACT_FILE),
        )?;
        ensure!(
            validate_scoring_key(&corpus.scoring_key, &contract, &source_map, &question_map)
                .is_err(),
            "unauthorized expected evidence was accepted"
        );
        Ok(())
    }

    #[test]
    fn complete_cross_platform_package_passes_frozen_gates() -> Result<()> {
        let package = complete_package()?;
        let summary = validate_package(package.path())?;
        ensure!(summary.cases == 24);
        let report_path = package.path().join("result/report.json");
        score(package.path(), &report_path)?;
        let report: serde_json::Value = load_json(&report_path)?;
        ensure!(
            report.get("passed").and_then(serde_json::Value::as_bool) == Some(true),
            "synthetic package did not produce a passing report"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn candidate_execution_captures_stdout_and_rejects_side_effects() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = tempfile::tempdir().context("creating candidate fixture")?;
        let input = temporary.path().join("input.jsonl");
        std::fs::write(&input, b"{\"fixture\":true}\n").context("writing candidate input")?;
        let candidate = temporary.path().join("candidate.sh");
        std::fs::write(&candidate, b"#!/bin/sh\n/bin/cat\n")
            .context("writing candidate fixture")?;
        let mut permissions = std::fs::metadata(&candidate)
            .context("reading candidate fixture metadata")?
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&candidate, permissions)
            .context("making candidate fixture executable")?;
        let scratch = TemporaryDirectory::create(temporary.path(), "runner-test")?;
        let execution =
            execute_candidate_side(&candidate, "source", &input, &scratch.path, "clean")?;
        ensure!(
            std::fs::read(&execution.output_path).context("reading candidate output")?
                == b"{\"fixture\":true}\n"
        );

        std::fs::write(
            &candidate,
            b"#!/bin/sh\n/usr/bin/touch unexpected\n/bin/cat\n",
        )
        .context("writing side-effecting candidate fixture")?;
        ensure!(
            execute_candidate_side(&candidate, "source", &input, &scratch.path, "side-effect")
                .is_err(),
            "candidate side effect was not rejected"
        );
        Ok(())
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn macos_runner_installs_one_non_overwritable_receipt() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = tempfile::tempdir().context("creating runner package fixture")?;
        let package = temporary.path().join("package");
        std::fs::create_dir(&package).context("creating runner package")?;
        let contract_path = workspace_root()
            .join(EXPERIMENT_DIRECTORY)
            .join(CONTRACT_FILE);
        let source_id = "src_0123456789abcdef0123456789abcdef";
        let question_id = "qry_fedcba9876543210fedcba9876543210";
        let source_text = "atlas owner ana";
        let question_text = "atlas owner who";
        write_json(
            &package.join("manifest.json"),
            &PackageManifest {
                schema_version: SCHEMA_VERSION,
                experiment_id: EXPERIMENT_ID.to_owned(),
                contract_sha256: hash_file(&contract_path)?,
                candidate_revision_sha256: "a".repeat(64),
            },
        )?;
        write_jsonl(
            &package.join(INPUT_SOURCE_FILE),
            &[SourceInput {
                source_id: source_id.to_owned(),
                title: "Synthetic evidence".to_owned(),
                heading: "Responsible party".to_owned(),
                text: source_text.to_owned(),
            }],
        )?;
        write_jsonl(
            &package.join(INPUT_QUESTION_FILE),
            &[QuestionInput {
                question_id: question_id.to_owned(),
                question: question_text.to_owned(),
            }],
        )?;
        let source_output = canonical_jsonl(&[SourceAnnotation {
            source_id: source_id.to_owned(),
            status: AnnotationStatus::Resolved,
            claims: vec![claim(
                source_text,
                "atlas",
                "responsible_party",
                "owner",
                "person",
                "ana",
            )?],
            reason_code: None,
        }])?;
        let question_output = canonical_jsonl(&[QuestionAnnotation {
            question_id: question_id.to_owned(),
            status: AnnotationStatus::Resolved,
            needs: vec![need(
                question_text,
                "atlas",
                "responsible_party",
                "owner",
                "person",
                "who",
            )?],
            reason_code: None,
        }])?;
        let source_line = std::str::from_utf8(&source_output)
            .context("source fixture output is not UTF-8")?
            .trim_end();
        let question_line = std::str::from_utf8(&question_output)
            .context("question fixture output is not UTF-8")?
            .trim_end();
        let candidate = temporary.path().join("candidate.sh");
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\nsource) printf '%s\\n' '{source_line}' ;;\nquestion) printf '%s\\n' '{question_line}' ;;\n*) exit 2 ;;\nesac\n"
        );
        std::fs::write(&candidate, script).context("writing runner candidate fixture")?;
        let mut permissions = std::fs::metadata(&candidate)
            .context("reading runner candidate metadata")?
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&candidate, permissions)
            .context("making runner candidate executable")?;

        run_candidate(&package, &candidate, "macos_arm64")?;
        ensure!(
            package.join("runs/macos_arm64/receipt.json").is_file(),
            "runner receipt was not installed"
        );
        ensure!(
            run_candidate(&package, &candidate, "macos_arm64").is_err(),
            "runner overwrote existing evidence"
        );
        Ok(())
    }
}
