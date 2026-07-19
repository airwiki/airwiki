#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::typed_evidence_v2::{
    Claim, Need, QuestionAnnotation, SourceAnnotation, validate_blind_question_annotation,
    validate_blind_source_annotation,
};

const TRANSPORT_LABEL: &str = "codex_exec_json_v1";
const INVOCATION_SCHEMA_VERSION: u32 = 1;
const MANIFEST_SCHEMA_VERSION: u32 = 1;
const DISPATCH_SCHEMA_LABEL: &str = "AIRWIKI_TYPED_EVIDENCE_DISPATCH_V1";
const CODEX_EXECUTABLE: &str = "codex";
const REQUESTED_MODEL: &str = "gpt-5.6-sol";
const REASONING_SETTING: &str = "high";
const EVIDENCE_OUTPUT_RELATIVE_PATH: &str = "experiments/typed-evidence-ceiling-v2/evidence";
const EXPECTED_CODEX_VERSION: &str = "codex-cli 0.144.4";
const EXPECTED_CODEX_BINARY_SHA256: &str =
    "3302acbda5f53de1a71ebdb0c0f2aae0d47f9324aa9fb6b4e78a47014fd51c7d";
const MAX_TEXT_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TRACE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ANNOTATION_BYTES: usize = 8 * 1024 * 1024;
const MAX_CODEX_BINARY_BYTES: usize = 256 * 1024 * 1024;
const RECORDED_ENVIRONMENT_NAMES: [&str; 3] = ["CODEX_EXEC_SERVER_URL", "HOME", "RUST_LOG"];
const EXPERIMENT_DIRECTORY: &str = "experiments/typed-evidence-ceiling-v2";
const FIELD_GUIDE_SHA256: &str = "1b9286680c1b9186a570c87d4660e8b0a3732fbecb62fc80374f1c5a9da4ea02";
const SOURCE_ANNOTATOR_PROMPT_SHA256: &str =
    "2181cfdecb26daac761fa22887afb6642697a2ff444798353e6237d1fed3776d";
const SOURCE_ADJUDICATOR_PROMPT_SHA256: &str =
    "c1bdf1e107f07841e62b427918e10594ee409981831d3b61ca378e857d3e4404";
const QUESTION_ANNOTATOR_PROMPT_SHA256: &str =
    "182e79754591e372cbd2447a7185d36cb46fe56c1e64199553f511d8e1540efc";
const QUESTION_ADJUDICATOR_PROMPT_SHA256: &str =
    "f3b3f36ab6abeb4b32f99b253039696df078bd61f2fd2f0c7e4ef1eda0e75c52";
const SOURCE_INPUT_SHA256: &str =
    "4303eba592c5174c5f37f3aaf35e56df3a25a9270e75a165d35bfebc7516400a";
const QUESTION_INPUT_SHA256: &str =
    "d71238bf3fa9072a226b995e956a99d0318136b74ac2b60c8e01d22571dff395";
const RUNNER_SOURCE_HASH_PREFIX: &str = "runner source SHA-256               ";
const RUNNER_SOURCE_PATHS: [&str; 5] = [
    "xtask/Cargo.toml",
    "xtask/src/main.rs",
    "xtask/src/retrieval.rs",
    "xtask/src/typed_evidence_trace.rs",
    "xtask/src/typed_evidence_v2.rs",
];

const FIELD_GUIDE_FILE: &str = "field-guide.md";
const SOURCE_ANNOTATOR_PROMPT_FILE: &str = "source-annotator-prompt.md";
const SOURCE_ADJUDICATOR_PROMPT_FILE: &str = "source-adjudicator-prompt.md";
const QUESTION_ANNOTATOR_PROMPT_FILE: &str = "question-annotator-prompt.md";
const QUESTION_ADJUDICATOR_PROMPT_FILE: &str = "question-adjudicator-prompt.md";
const SOURCE_INPUT_FILE: &str = "source-input.jsonl";
const QUESTION_INPUT_FILE: &str = "question-input.jsonl";
const MANIFEST_FILE: &str = "manifest.json";

const DISPATCH_FILE: &str = "dispatch.txt";
const INVOCATION_FILE: &str = "invocation.json";
const TRACE_FILE: &str = "trace.jsonl";
const ANNOTATION_FILE: &str = "annotation.jsonl";

#[derive(Clone, Copy)]
struct AnnotationInputs<'a> {
    field_guide: &'a [u8],
    source_annotator_prompt: &'a [u8],
    source_adjudicator_prompt: &'a [u8],
    question_annotator_prompt: &'a [u8],
    question_adjudicator_prompt: &'a [u8],
    source_input: &'a [u8],
    question_input: &'a [u8],
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifiedEvidence {
    pub(crate) source_input: Vec<u8>,
    pub(crate) question_input: Vec<u8>,
    pub(crate) source_adjudication: Vec<u8>,
    pub(crate) question_adjudication: Vec<u8>,
    pub(crate) manifest_sha256: String,
    pub(crate) source_adjudication_sha256: String,
    pub(crate) question_adjudication_sha256: String,
    pub(crate) repository_commit: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Role {
    SourceDraftA,
    SourceDraftB,
    SourceAdjudication,
    QuestionDraftA,
    QuestionDraftB,
    QuestionAdjudication,
}

impl Role {
    const ALL: [Self; 6] = [
        Self::SourceDraftA,
        Self::SourceDraftB,
        Self::SourceAdjudication,
        Self::QuestionDraftA,
        Self::QuestionDraftB,
        Self::QuestionAdjudication,
    ];

    fn directory(self) -> &'static str {
        match self {
            Self::SourceDraftA => "01-source-draft-a",
            Self::SourceDraftB => "02-source-draft-b",
            Self::SourceAdjudication => "03-source-adjudication",
            Self::QuestionDraftA => "04-question-draft-a",
            Self::QuestionDraftB => "05-question-draft-b",
            Self::QuestionAdjudication => "06-question-adjudication",
        }
    }

    fn dispatch_role(self) -> &'static str {
        match self {
            Self::SourceDraftA | Self::SourceDraftB => "source_annotator",
            Self::SourceAdjudication => "source_adjudicator",
            Self::QuestionDraftA | Self::QuestionDraftB => "question_annotator",
            Self::QuestionAdjudication => "question_adjudicator",
        }
    }

    fn is_source(self) -> bool {
        matches!(
            self,
            Self::SourceDraftA | Self::SourceDraftB | Self::SourceAdjudication
        )
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InvocationRecord {
    schema_version: u32,
    transport_label: String,
    role: Role,
    argv: Vec<String>,
    environment_names: Vec<String>,
    codex_cli_version: String,
    codex_binary_sha256: String,
    repository_commit: String,
    working_directory_token: String,
    working_directory_empty_before: bool,
    working_directory_empty_after: bool,
    dispatch_sha256: String,
    process_success: bool,
    process_exit_code: Option<i32>,
    stdout_sha256: String,
    stderr_size_bytes: u64,
    stderr_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EvidenceManifest {
    schema_version: u32,
    transport_label: String,
    requested_model: String,
    reasoning_setting: String,
    codex_cli_version: String,
    codex_binary_sha256: String,
    repository_commit: String,
    inputs: InputHashes,
    roles: Vec<RoleEvidence>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InputHashes {
    field_guide_sha256: String,
    source_annotator_prompt_sha256: String,
    source_adjudicator_prompt_sha256: String,
    question_annotator_prompt_sha256: String,
    question_adjudicator_prompt_sha256: String,
    source_input_sha256: String,
    question_input_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RoleEvidence {
    role: Role,
    directory: String,
    dispatch_sha256: String,
    invocation_sha256: String,
    trace_sha256: String,
    stderr_size_bytes: u64,
    stderr_sha256: String,
    annotation_sha256: String,
    thread_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceInputRecord {
    source_id: String,
    source_record_sha256: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    heading: Option<String>,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionInputRecord {
    question_id: String,
    question_record_sha256: String,
    question: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "status")]
enum SourceAnnotationRecord {
    Resolved {
        source_id: String,
        claims: Vec<Claim>,
    },
    Unresolved {
        source_id: String,
        reason_code: UnresolvedReason,
    },
}

impl SourceAnnotationRecord {
    fn id(&self) -> &str {
        match self {
            Self::Resolved { source_id, .. } | Self::Unresolved { source_id, .. } => source_id,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "status")]
enum QuestionAnnotationRecord {
    Resolved {
        question_id: String,
        needs: Vec<Need>,
    },
    Unresolved {
        question_id: String,
        reason_code: UnresolvedReason,
    },
}

impl QuestionAnnotationRecord {
    fn id(&self) -> &str {
        match self {
            Self::Resolved { question_id, .. } | Self::Unresolved { question_id, .. } => {
                question_id
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UnresolvedReason {
    MissingSubject,
    AmbiguousSubject,
    AmbiguousRelation,
    AmbiguousState,
    UnsupportedStructure,
}

struct TraceEvidence {
    thread_id: String,
    annotation: Vec<u8>,
}

struct RoleArtifacts {
    evidence: RoleEvidence,
    annotation: Vec<u8>,
}

struct EvidenceAssets {
    field_guide: Vec<u8>,
    source_annotator_prompt: Vec<u8>,
    source_adjudicator_prompt: Vec<u8>,
    question_annotator_prompt: Vec<u8>,
    question_adjudicator_prompt: Vec<u8>,
    source_input: Vec<u8>,
    question_input: Vec<u8>,
}

impl EvidenceAssets {
    fn as_inputs(&self) -> AnnotationInputs<'_> {
        AnnotationInputs {
            field_guide: &self.field_guide,
            source_annotator_prompt: &self.source_annotator_prompt,
            source_adjudicator_prompt: &self.source_adjudicator_prompt,
            question_annotator_prompt: &self.question_annotator_prompt,
            question_adjudicator_prompt: &self.question_adjudicator_prompt,
            source_input: &self.source_input,
            question_input: &self.question_input,
        }
    }
}

struct CodexIdentity {
    executable: PathBuf,
    version: String,
    binary_sha256: String,
}

struct EvidenceIdentity<'a> {
    codex_binary_sha256: &'a str,
    repository_commit: &'a str,
}

struct FreshWorkingDirectory {
    path: PathBuf,
    token: String,
}

impl FreshWorkingDirectory {
    fn create() -> Result<Self> {
        let token = format!("workdir-{}", Uuid::new_v4().simple());
        let path = env::temp_dir().join(format!("airwiki-typed-evidence-{token}"));
        fs::create_dir(&path).context("could not create fresh annotation working directory")?;
        ensure!(
            directory_is_empty(&path)?,
            "fresh working directory is not empty"
        );
        Ok(Self { path, token })
    }
}

impl Drop for FreshWorkingDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) fn annotate_from_workspace() -> Result<VerifiedEvidence> {
    ensure!(
        cfg!(all(target_os = "macos", target_arch = "aarch64")),
        "typed-evidence annotation is frozen to macOS arm64"
    );
    let workspace = workspace_root()?;
    let repository_commit = ensure_reviewed_main(&workspace)?;
    ensure_frozen_runner(&workspace)?;
    let assets = read_workspace_assets()?;
    annotate(
        &workspace.join(EVIDENCE_OUTPUT_RELATIVE_PATH),
        assets.as_inputs(),
        &repository_commit,
    )
}

fn annotate(
    output_directory: &Path,
    inputs: AnnotationInputs<'_>,
    repository_commit: &str,
) -> Result<VerifiedEvidence> {
    validate_annotation_inputs(inputs)?;
    let codex = codex_identity()?;
    fs::create_dir(output_directory).with_context(|| {
        format!(
            "evidence output must be a new directory: {}",
            output_directory.display()
        )
    })?;

    write_input_assets(output_directory, inputs)?;

    let mut annotations = BTreeMap::new();
    let mut role_evidence = Vec::with_capacity(Role::ALL.len());
    for role in Role::ALL {
        let dispatch = build_dispatch(role, inputs, &annotations)?;
        let artifacts = execute_role(
            output_directory,
            role,
            &dispatch,
            inputs,
            &codex,
            repository_commit,
        )?;
        annotations.insert(role, artifacts.annotation);
        role_evidence.push(artifacts.evidence);
    }

    let manifest = EvidenceManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        transport_label: TRANSPORT_LABEL.to_owned(),
        requested_model: REQUESTED_MODEL.to_owned(),
        reasoning_setting: REASONING_SETTING.to_owned(),
        codex_cli_version: codex.version,
        codex_binary_sha256: codex.binary_sha256,
        repository_commit: repository_commit.to_owned(),
        inputs: input_hashes(inputs),
        roles: role_evidence,
    };
    write_canonical_json_new(&output_directory.join(MANIFEST_FILE), &manifest)?;

    verify_evidence(output_directory)
}

pub(crate) fn verify_evidence(directory: &Path) -> Result<VerifiedEvidence> {
    ensure_directory_entries(
        directory,
        root_entry_names().into_iter().map(str::to_owned).collect(),
    )?;

    let assets = read_evidence_assets(directory)?;
    let source_inputs = parse_source_inputs(&assets.source_input)?;
    let question_inputs = parse_question_inputs(&assets.question_input)?;
    let manifest_bytes =
        read_regular_file(&directory.join(MANIFEST_FILE), MAX_TEXT_ARTIFACT_BYTES)?;
    let manifest: EvidenceManifest = parse_canonical_json(&manifest_bytes, "evidence manifest")?;
    validate_manifest_header(&manifest, &assets)?;

    let input_view = AnnotationInputs {
        field_guide: &assets.field_guide,
        source_annotator_prompt: &assets.source_annotator_prompt,
        source_adjudicator_prompt: &assets.source_adjudicator_prompt,
        question_annotator_prompt: &assets.question_annotator_prompt,
        question_adjudicator_prompt: &assets.question_adjudicator_prompt,
        source_input: &assets.source_input,
        question_input: &assets.question_input,
    };
    let mut annotations = BTreeMap::new();
    let mut thread_ids = BTreeSet::new();
    let identity = EvidenceIdentity {
        codex_binary_sha256: &manifest.codex_binary_sha256,
        repository_commit: &manifest.repository_commit,
    };

    ensure!(
        manifest.roles.len() == Role::ALL.len(),
        "evidence manifest must contain exactly six roles"
    );
    for (index, role) in Role::ALL.iter().copied().enumerate() {
        let role_manifest = manifest
            .roles
            .get(index)
            .context("evidence manifest role is missing")?;
        ensure!(
            role_manifest.role == role,
            "evidence roles are out of order"
        );
        ensure!(
            role_manifest.directory == role.directory(),
            "evidence role directory does not match the frozen layout"
        );

        let expected_dispatch = build_dispatch(role, input_view, &annotations)?;
        let role_artifacts = verify_role(
            directory,
            role,
            role_manifest,
            &expected_dispatch,
            &source_inputs,
            &question_inputs,
            &identity,
        )?;
        ensure!(
            thread_ids.insert(role_artifacts.evidence.thread_id.clone()),
            "annotation executions must use six distinct thread identifiers"
        );
        annotations.insert(role, role_artifacts.annotation);
    }

    let source_adjudication = annotations
        .remove(&Role::SourceAdjudication)
        .context("source adjudication artifact is missing")?;
    let question_adjudication = annotations
        .remove(&Role::QuestionAdjudication)
        .context("question adjudication artifact is missing")?;

    Ok(VerifiedEvidence {
        source_input: assets.source_input,
        question_input: assets.question_input,
        source_adjudication_sha256: sha256_hex(&source_adjudication),
        question_adjudication_sha256: sha256_hex(&question_adjudication),
        manifest_sha256: sha256_hex(&manifest_bytes),
        repository_commit: manifest.repository_commit,
        source_adjudication,
        question_adjudication,
    })
}

fn validate_annotation_inputs(inputs: AnnotationInputs<'_>) -> Result<()> {
    for (label, bytes) in [
        ("field guide", inputs.field_guide),
        ("source annotator prompt", inputs.source_annotator_prompt),
        (
            "source adjudicator prompt",
            inputs.source_adjudicator_prompt,
        ),
        (
            "question annotator prompt",
            inputs.question_annotator_prompt,
        ),
        (
            "question adjudicator prompt",
            inputs.question_adjudicator_prompt,
        ),
    ] {
        validate_canonical_text(bytes, label)?;
    }
    parse_source_inputs(inputs.source_input)?;
    parse_question_inputs(inputs.question_input)?;
    Ok(())
}

fn execute_role(
    output_directory: &Path,
    role: Role,
    dispatch: &[u8],
    inputs: AnnotationInputs<'_>,
    codex: &CodexIdentity,
    repository_commit: &str,
) -> Result<RoleArtifacts> {
    let role_directory = output_directory.join(role.directory());
    fs::create_dir(&role_directory).context("could not create evidence role directory")?;
    write_new(&role_directory.join(DISPATCH_FILE), dispatch)?;

    let working_directory = FreshWorkingDirectory::create()?;
    let empty_before = directory_is_empty(&working_directory.path)?;
    let argv = frozen_argv();

    let mut command = Command::new(&codex.executable);
    #[cfg(unix)]
    command.arg0(CODEX_EXECUTABLE);
    command
        .args(&argv[1..])
        .current_dir(&working_directory.path)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let environment_names = apply_isolated_environment(&mut command)?;

    let mut child = command
        .spawn()
        .context("could not start the frozen Codex annotation command")?;
    let write_result = child
        .stdin
        .as_mut()
        .context("Codex annotation stdin was unavailable")?
        .write_all(dispatch);
    drop(child.stdin.take());
    if let Err(error) = write_result {
        let _ = child.wait();
        return Err(error).context("could not send the frozen annotation dispatch");
    }
    let output = child
        .wait_with_output()
        .context("could not collect Codex annotation evidence")?;
    let empty_after = directory_is_empty(&working_directory.path)?;

    ensure!(
        output.stdout.len() <= MAX_TRACE_BYTES,
        "annotation trace exceeds the frozen size limit"
    );
    ensure!(
        output.stderr.len() <= MAX_TRACE_BYTES,
        "annotation stderr exceeds the frozen size limit"
    );
    write_new(&role_directory.join(TRACE_FILE), &output.stdout)?;

    let invocation = InvocationRecord {
        schema_version: INVOCATION_SCHEMA_VERSION,
        transport_label: TRANSPORT_LABEL.to_owned(),
        role,
        argv,
        environment_names,
        codex_cli_version: codex.version.clone(),
        codex_binary_sha256: codex.binary_sha256.clone(),
        repository_commit: repository_commit.to_owned(),
        working_directory_token: working_directory.token.clone(),
        working_directory_empty_before: empty_before,
        working_directory_empty_after: empty_after,
        dispatch_sha256: sha256_hex(dispatch),
        process_success: output.status.success(),
        process_exit_code: output.status.code(),
        stdout_sha256: sha256_hex(&output.stdout),
        stderr_size_bytes: output.stderr.len() as u64,
        stderr_sha256: sha256_hex(&output.stderr),
    };
    let invocation_bytes = canonical_json_bytes(&invocation)?;
    write_new(&role_directory.join(INVOCATION_FILE), &invocation_bytes)?;

    ensure!(
        empty_before,
        "annotation working directory was not empty before execution"
    );
    ensure!(
        empty_after,
        "annotation working directory changed during execution"
    );
    ensure!(
        output.status.success(),
        "Codex annotation process did not succeed"
    );
    ensure!(
        output.stderr.is_empty(),
        "Codex annotation process wrote to stderr"
    );

    let trace = verify_trace(&output.stdout)?;
    validate_annotation_for_role(
        role,
        &trace.annotation,
        &parse_source_inputs(inputs.source_input)?,
        &parse_question_inputs(inputs.question_input)?,
    )?;
    write_new(&role_directory.join(ANNOTATION_FILE), &trace.annotation)?;

    Ok(RoleArtifacts {
        evidence: RoleEvidence {
            role,
            directory: role.directory().to_owned(),
            dispatch_sha256: sha256_hex(dispatch),
            invocation_sha256: sha256_hex(&invocation_bytes),
            trace_sha256: sha256_hex(&output.stdout),
            stderr_size_bytes: output.stderr.len() as u64,
            stderr_sha256: sha256_hex(&output.stderr),
            annotation_sha256: sha256_hex(&trace.annotation),
            thread_id: trace.thread_id,
        },
        annotation: trace.annotation,
    })
}

fn verify_role(
    root: &Path,
    role: Role,
    manifest: &RoleEvidence,
    expected_dispatch: &[u8],
    source_inputs: &[SourceInputRecord],
    question_inputs: &[QuestionInputRecord],
    identity: &EvidenceIdentity<'_>,
) -> Result<RoleArtifacts> {
    let directory = root.join(role.directory());
    ensure_directory_entries(
        &directory,
        [DISPATCH_FILE, INVOCATION_FILE, TRACE_FILE, ANNOTATION_FILE]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    )?;

    let dispatch = read_regular_file(&directory.join(DISPATCH_FILE), MAX_TEXT_ARTIFACT_BYTES)?;
    let invocation_bytes =
        read_regular_file(&directory.join(INVOCATION_FILE), MAX_TEXT_ARTIFACT_BYTES)?;
    let trace_bytes = read_regular_file(&directory.join(TRACE_FILE), MAX_TRACE_BYTES)?;
    let annotation = read_regular_file(&directory.join(ANNOTATION_FILE), MAX_ANNOTATION_BYTES)?;

    ensure!(
        dispatch == expected_dispatch,
        "annotation dispatch is not the frozen dispatch"
    );
    ensure_hash(&dispatch, &manifest.dispatch_sha256, "dispatch")?;
    ensure_hash(&invocation_bytes, &manifest.invocation_sha256, "invocation")?;
    ensure_hash(&trace_bytes, &manifest.trace_sha256, "trace")?;
    ensure_hash(&annotation, &manifest.annotation_sha256, "annotation")?;
    ensure!(
        manifest.stderr_size_bytes == 0 && manifest.stderr_sha256 == sha256_hex(&[]),
        "annotation manifest must record empty stderr"
    );

    let invocation: InvocationRecord =
        parse_canonical_json(&invocation_bytes, "invocation record")?;
    validate_invocation(
        &invocation,
        role,
        &dispatch,
        &trace_bytes,
        &manifest.stderr_sha256,
        identity.codex_binary_sha256,
        identity.repository_commit,
    )?;

    let trace = verify_trace(&trace_bytes)?;
    ensure!(
        trace.thread_id == manifest.thread_id,
        "trace thread identifier does not match the manifest"
    );
    ensure!(
        trace.annotation == annotation,
        "annotation bytes do not match the completed agent message"
    );
    validate_annotation_for_role(role, &annotation, source_inputs, question_inputs)?;

    Ok(RoleArtifacts {
        evidence: RoleEvidence {
            role,
            directory: role.directory().to_owned(),
            dispatch_sha256: manifest.dispatch_sha256.clone(),
            invocation_sha256: manifest.invocation_sha256.clone(),
            trace_sha256: manifest.trace_sha256.clone(),
            stderr_size_bytes: manifest.stderr_size_bytes,
            stderr_sha256: manifest.stderr_sha256.clone(),
            annotation_sha256: manifest.annotation_sha256.clone(),
            thread_id: trace.thread_id,
        },
        annotation,
    })
}

fn validate_manifest_header(manifest: &EvidenceManifest, assets: &EvidenceAssets) -> Result<()> {
    ensure!(
        manifest.schema_version == MANIFEST_SCHEMA_VERSION,
        "unsupported evidence manifest schema"
    );
    ensure!(
        manifest.transport_label == TRANSPORT_LABEL,
        "unexpected annotation transport"
    );
    ensure!(
        manifest.requested_model == REQUESTED_MODEL,
        "unexpected annotation model"
    );
    ensure!(
        manifest.reasoning_setting == REASONING_SETTING,
        "unexpected annotation reasoning setting"
    );
    ensure!(
        manifest.codex_cli_version == EXPECTED_CODEX_VERSION,
        "unexpected Codex CLI version"
    );
    ensure!(
        manifest.codex_binary_sha256 == EXPECTED_CODEX_BINARY_SHA256,
        "unexpected Codex binary SHA-256"
    );
    ensure!(
        is_git_commit(&manifest.repository_commit),
        "invalid preregistration commit"
    );
    validate_frozen_asset_hashes(assets)?;
    let expected = InputHashes {
        field_guide_sha256: sha256_hex(&assets.field_guide),
        source_annotator_prompt_sha256: sha256_hex(&assets.source_annotator_prompt),
        source_adjudicator_prompt_sha256: sha256_hex(&assets.source_adjudicator_prompt),
        question_annotator_prompt_sha256: sha256_hex(&assets.question_annotator_prompt),
        question_adjudicator_prompt_sha256: sha256_hex(&assets.question_adjudicator_prompt),
        source_input_sha256: sha256_hex(&assets.source_input),
        question_input_sha256: sha256_hex(&assets.question_input),
    };
    ensure!(
        manifest.inputs.field_guide_sha256 == expected.field_guide_sha256
            && manifest.inputs.source_annotator_prompt_sha256
                == expected.source_annotator_prompt_sha256
            && manifest.inputs.source_adjudicator_prompt_sha256
                == expected.source_adjudicator_prompt_sha256
            && manifest.inputs.question_annotator_prompt_sha256
                == expected.question_annotator_prompt_sha256
            && manifest.inputs.question_adjudicator_prompt_sha256
                == expected.question_adjudicator_prompt_sha256
            && manifest.inputs.source_input_sha256 == expected.source_input_sha256
            && manifest.inputs.question_input_sha256 == expected.question_input_sha256,
        "evidence input hashes do not match the retained inputs"
    );
    Ok(())
}

fn validate_invocation(
    invocation: &InvocationRecord,
    role: Role,
    dispatch: &[u8],
    trace: &[u8],
    expected_stderr_sha256: &str,
    expected_codex_binary_sha256: &str,
    expected_repository_commit: &str,
) -> Result<()> {
    ensure!(
        invocation.schema_version == INVOCATION_SCHEMA_VERSION,
        "unsupported invocation schema"
    );
    ensure!(
        invocation.transport_label == TRANSPORT_LABEL,
        "unexpected invocation transport"
    );
    ensure!(
        invocation.role == role,
        "invocation role does not match its directory"
    );
    ensure!(
        invocation.argv == frozen_argv(),
        "invocation argv is not frozen"
    );
    ensure!(
        invocation.codex_cli_version == EXPECTED_CODEX_VERSION,
        "invocation Codex CLI version is not frozen"
    );
    ensure!(
        invocation.codex_binary_sha256 == expected_codex_binary_sha256,
        "invocation Codex binary SHA-256 differs from the manifest"
    );
    ensure!(
        invocation.repository_commit == expected_repository_commit,
        "invocation preregistration commit differs from the manifest"
    );
    ensure!(
        invocation.environment_names
            == RECORDED_ENVIRONMENT_NAMES
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        "invocation environment is not the frozen minimal allowlist"
    );
    ensure!(
        invocation.working_directory_token.starts_with("workdir-")
            && invocation.working_directory_token.len() == 40,
        "invocation working-directory token is invalid"
    );
    ensure!(
        invocation.working_directory_empty_before && invocation.working_directory_empty_after,
        "invocation working directory was not empty before and after execution"
    );
    ensure!(
        invocation.process_success && invocation.process_exit_code == Some(0),
        "annotation process did not record a successful exit"
    );
    ensure_hash(dispatch, &invocation.dispatch_sha256, "invocation dispatch")?;
    ensure_hash(trace, &invocation.stdout_sha256, "invocation stdout")?;
    ensure!(
        invocation.stderr_size_bytes == 0
            && invocation.stderr_sha256 == expected_stderr_sha256
            && invocation.stderr_sha256 == sha256_hex(&[]),
        "invocation must record empty stderr"
    );
    Ok(())
}

fn verify_trace(bytes: &[u8]) -> Result<TraceEvidence> {
    ensure!(
        bytes.len() <= MAX_TRACE_BYTES,
        "annotation trace exceeds size limit"
    );
    validate_jsonl_bytes(bytes, "annotation trace")?;
    let text = std::str::from_utf8(bytes).context("annotation trace is not UTF-8")?;
    let mut stage = 0_u8;
    let mut thread_id = None;
    let mut agent_message = None;

    for line in text.split_terminator('\n') {
        let event: Value =
            serde_json::from_str(line).context("annotation trace line is not JSON")?;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .context("annotation trace event has no string type")?;
        match (stage, event_type) {
            (0, "thread.started") => {
                let value = event
                    .get("thread_id")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .context("thread.started has no thread identifier")?;
                thread_id = Some(value.to_owned());
                stage = 1;
            }
            (1, "turn.started") => stage = 2,
            (2, "item.completed") => {
                let item = event
                    .get("item")
                    .and_then(Value::as_object)
                    .context("item.completed has no item object")?;
                match item.get("type").and_then(Value::as_str) {
                    Some("reasoning") if agent_message.is_none() => {}
                    Some("agent_message") if agent_message.is_none() => {
                        let text = item
                            .get("text")
                            .and_then(Value::as_str)
                            .context("completed agent message has no text")?;
                        agent_message = Some(canonical_annotation_bytes(text)?);
                        stage = 3;
                    }
                    Some("agent_message") => bail!("annotation trace has an extra agent message"),
                    Some(_) => bail!("annotation trace contains a forbidden item type"),
                    None => bail!("completed trace item has no string type"),
                }
            }
            (3, "turn.completed") => stage = 4,
            (_, "error") | (_, "turn.failed") => {
                bail!("annotation trace contains an error event")
            }
            (4, _) => bail!("annotation trace contains an event after turn.completed"),
            _ => bail!("annotation trace event sequence is invalid"),
        }
    }

    ensure!(stage == 4, "annotation trace has no valid terminal event");
    Ok(TraceEvidence {
        thread_id: thread_id.context("annotation trace has no thread identifier")?,
        annotation: agent_message.context("annotation trace has no completed agent message")?,
    })
}

fn canonical_annotation_bytes(text: &str) -> Result<Vec<u8>> {
    ensure!(!text.is_empty(), "annotation message is empty");
    ensure!(
        text.len() <= MAX_ANNOTATION_BYTES,
        "annotation message exceeds size limit"
    );
    ensure!(
        !text.as_bytes().contains(&b'\r'),
        "annotation message contains a carriage return"
    );
    ensure!(
        !text.as_bytes().contains(&0),
        "annotation message contains a NUL byte"
    );
    ensure!(
        !text.ends_with("\n\n"),
        "annotation message has more than one terminal LF"
    );

    let mut bytes = text.as_bytes().to_vec();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    validate_annotation_jsonl(&bytes)?;
    Ok(bytes)
}

fn validate_annotation_for_role(
    role: Role,
    bytes: &[u8],
    source_inputs: &[SourceInputRecord],
    question_inputs: &[QuestionInputRecord],
) -> Result<()> {
    if role.is_source() {
        validate_source_annotations(bytes, source_inputs)
    } else {
        validate_question_annotations(bytes, question_inputs)
    }
}

fn validate_source_annotations(bytes: &[u8], inputs: &[SourceInputRecord]) -> Result<()> {
    let lines = annotation_lines(bytes)?;
    ensure!(
        lines.len() == inputs.len(),
        "source annotation record count does not match input"
    );
    for (line, input) in lines.into_iter().zip(inputs) {
        let record: SourceAnnotationRecord =
            serde_json::from_str(line).context("source annotation has invalid schema")?;
        ensure!(
            record.id() == input.source_id,
            "source annotation IDs are incomplete, reordered or changed"
        );
        match record {
            SourceAnnotationRecord::Resolved { source_id, claims } => {
                validate_blind_source_annotation(&SourceAnnotation {
                    schema_version: 2,
                    fact_id: source_id,
                    claims: claims.clone(),
                })
                .map_err(anyhow::Error::new)?;
                validate_claims(&claims, &input.text)?;
            }
            SourceAnnotationRecord::Unresolved { reason_code, .. } => {
                let _ = reason_code;
            }
        }
    }
    Ok(())
}

fn validate_claims(claims: &[Claim], text: &str) -> Result<()> {
    let mut previous: Option<(usize, &str)> = None;
    for claim in claims {
        let position = text
            .find(&claim.support_quote)
            .context("source support quote is not an exact substring")?;
        if let Some((previous_position, previous_relation)) = previous {
            ensure!(
                position > previous_position
                    || (position == previous_position
                        && claim.relation.as_str() >= previous_relation),
                "source claims are not in required text/relation order"
            );
        }
        previous = Some((position, &claim.relation));
    }
    Ok(())
}

fn validate_question_annotations(bytes: &[u8], inputs: &[QuestionInputRecord]) -> Result<()> {
    let lines = annotation_lines(bytes)?;
    ensure!(
        lines.len() == inputs.len(),
        "question annotation record count does not match input"
    );
    for (line, input) in lines.into_iter().zip(inputs) {
        let record: QuestionAnnotationRecord =
            serde_json::from_str(line).context("question annotation has invalid schema")?;
        ensure!(
            record.id() == input.question_id,
            "question annotation IDs are incomplete, reordered or changed"
        );
        match record {
            QuestionAnnotationRecord::Resolved { question_id, needs } => {
                validate_blind_question_annotation(&QuestionAnnotation {
                    schema_version: 2,
                    case_id: question_id,
                    needs: needs.clone(),
                })
                .map_err(anyhow::Error::new)?;
                validate_needs(&needs, &input.question)?;
            }
            QuestionAnnotationRecord::Unresolved { reason_code, .. } => {
                let _ = reason_code;
            }
        }
    }
    Ok(())
}

fn validate_needs(needs: &[Need], question: &str) -> Result<()> {
    let mut previous_position = None;
    for need in needs {
        let position = question
            .find(&need.question_quote)
            .context("question quote is not an exact substring")?;
        if let Some(previous) = previous_position {
            ensure!(
                position >= previous,
                "question needs do not preserve question order"
            );
        }
        previous_position = Some(position);
    }
    Ok(())
}

fn parse_source_inputs(bytes: &[u8]) -> Result<Vec<SourceInputRecord>> {
    validate_canonical_jsonl(bytes, "source input")?;
    let text = std::str::from_utf8(bytes).context("source input is not UTF-8")?;
    let mut records = Vec::new();
    let mut ids = BTreeSet::new();
    for line in text.split_terminator('\n') {
        let record: SourceInputRecord =
            serde_json::from_str(line).context("source input has invalid schema")?;
        ensure_nonempty_fields([
            record.source_id.as_str(),
            record.source_record_sha256.as_str(),
            record.text.as_str(),
        ])?;
        ensure!(
            ids.insert(record.source_id.clone()),
            "source input contains a duplicate ID"
        );
        let _ = (&record.title, &record.heading);
        records.push(record);
    }
    ensure!(!records.is_empty(), "source input is empty");
    Ok(records)
}

fn parse_question_inputs(bytes: &[u8]) -> Result<Vec<QuestionInputRecord>> {
    validate_canonical_jsonl(bytes, "question input")?;
    let text = std::str::from_utf8(bytes).context("question input is not UTF-8")?;
    let mut records = Vec::new();
    let mut ids = BTreeSet::new();
    for line in text.split_terminator('\n') {
        let record: QuestionInputRecord =
            serde_json::from_str(line).context("question input has invalid schema")?;
        ensure_nonempty_fields([
            record.question_id.as_str(),
            record.question_record_sha256.as_str(),
            record.question.as_str(),
        ])?;
        ensure!(
            ids.insert(record.question_id.clone()),
            "question input contains a duplicate ID"
        );
        records.push(record);
    }
    ensure!(!records.is_empty(), "question input is empty");
    Ok(records)
}

fn validate_annotation_jsonl(bytes: &[u8]) -> Result<()> {
    validate_jsonl_bytes(bytes, "annotation")?;
    for line in annotation_lines(bytes)? {
        ensure!(
            no_json_whitespace_outside_strings(line),
            "annotation JSONL records must be compact"
        );
        let _: Value = serde_json::from_str(line).context("annotation line is not JSON")?;
    }
    Ok(())
}

fn annotation_lines(bytes: &[u8]) -> Result<Vec<&str>> {
    validate_annotation_jsonl_shape(bytes)?;
    let text = std::str::from_utf8(bytes).context("annotation is not UTF-8")?;
    Ok(text.split_terminator('\n').collect())
}

fn validate_annotation_jsonl_shape(bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() <= MAX_ANNOTATION_BYTES,
        "annotation exceeds size limit"
    );
    validate_jsonl_bytes(bytes, "annotation")
}

fn validate_canonical_jsonl(bytes: &[u8], label: &str) -> Result<()> {
    validate_canonical_text(bytes, label)?;
    let text = std::str::from_utf8(bytes).with_context(|| format!("{label} is not UTF-8"))?;
    for line in text.split_terminator('\n') {
        ensure!(!line.is_empty(), "{label} contains an empty record");
        let _: Value =
            serde_json::from_str(line).with_context(|| format!("{label} line is not JSON"))?;
    }
    Ok(())
}

fn validate_jsonl_bytes(bytes: &[u8], label: &str) -> Result<()> {
    ensure!(!bytes.is_empty(), "{label} is empty");
    ensure!(
        !bytes.contains(&b'\r'),
        "{label} contains a carriage return"
    );
    ensure!(!bytes.contains(&0), "{label} contains a NUL byte");
    ensure!(bytes.ends_with(b"\n"), "{label} has no terminal LF");
    ensure!(
        !bytes.ends_with(b"\n\n"),
        "{label} has more than one terminal LF"
    );
    let text = std::str::from_utf8(bytes).with_context(|| format!("{label} is not UTF-8"))?;
    ensure!(
        text.split_terminator('\n').all(|line| !line.is_empty()),
        "{label} contains an empty line"
    );
    Ok(())
}

fn validate_canonical_text(bytes: &[u8], label: &str) -> Result<()> {
    ensure!(
        bytes.len() <= MAX_TEXT_ARTIFACT_BYTES,
        "{label} exceeds size limit"
    );
    ensure!(!bytes.is_empty(), "{label} is empty");
    ensure!(
        !bytes.contains(&b'\r'),
        "{label} contains a carriage return"
    );
    ensure!(!bytes.contains(&0), "{label} contains a NUL byte");
    ensure!(bytes.ends_with(b"\n"), "{label} has no terminal LF");
    ensure!(
        !bytes.ends_with(b"\n\n"),
        "{label} has more than one terminal LF"
    );
    std::str::from_utf8(bytes).with_context(|| format!("{label} is not UTF-8"))?;
    Ok(())
}

fn no_json_whitespace_outside_strings(line: &str) -> bool {
    let mut in_string = false;
    let mut escaped = false;
    for byte in line.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte.is_ascii_whitespace() {
            return false;
        }
    }
    !in_string && !escaped
}

fn ensure_nonempty_fields<'a>(fields: impl IntoIterator<Item = &'a str>) -> Result<()> {
    ensure!(
        fields.into_iter().all(|value| !value.is_empty()),
        "annotation or input contains an empty required string"
    );
    Ok(())
}

fn build_dispatch(
    role: Role,
    inputs: AnnotationInputs<'_>,
    annotations: &BTreeMap<Role, Vec<u8>>,
) -> Result<Vec<u8>> {
    let mut dispatch = Vec::new();
    append_header(&mut dispatch, DISPATCH_SCHEMA_LABEL);
    append_header(&mut dispatch, &format!("ROLE {}", role.dispatch_role()));

    match role {
        Role::SourceDraftA | Role::SourceDraftB => {
            append_section(&mut dispatch, "ROLE_PROMPT", inputs.source_annotator_prompt)?;
            append_section(&mut dispatch, "FIELD_GUIDE", inputs.field_guide)?;
            append_section(&mut dispatch, "SOURCE_INPUT_JSONL", inputs.source_input)?;
        }
        Role::SourceAdjudication => {
            append_section(
                &mut dispatch,
                "ROLE_PROMPT",
                inputs.source_adjudicator_prompt,
            )?;
            append_section(&mut dispatch, "FIELD_GUIDE", inputs.field_guide)?;
            append_section(&mut dispatch, "SOURCE_INPUT_JSONL", inputs.source_input)?;
            append_section(
                &mut dispatch,
                "SOURCE_DRAFT_A_JSONL",
                annotation_for(annotations, Role::SourceDraftA)?,
            )?;
            append_section(
                &mut dispatch,
                "SOURCE_DRAFT_B_JSONL",
                annotation_for(annotations, Role::SourceDraftB)?,
            )?;
        }
        Role::QuestionDraftA | Role::QuestionDraftB => {
            append_section(
                &mut dispatch,
                "ROLE_PROMPT",
                inputs.question_annotator_prompt,
            )?;
            append_section(&mut dispatch, "FIELD_GUIDE", inputs.field_guide)?;
            append_section(&mut dispatch, "QUESTION_INPUT_JSONL", inputs.question_input)?;
        }
        Role::QuestionAdjudication => {
            append_section(
                &mut dispatch,
                "ROLE_PROMPT",
                inputs.question_adjudicator_prompt,
            )?;
            append_section(&mut dispatch, "FIELD_GUIDE", inputs.field_guide)?;
            append_section(&mut dispatch, "QUESTION_INPUT_JSONL", inputs.question_input)?;
            append_section(
                &mut dispatch,
                "QUESTION_DRAFT_A_JSONL",
                annotation_for(annotations, Role::QuestionDraftA)?,
            )?;
            append_section(
                &mut dispatch,
                "QUESTION_DRAFT_B_JSONL",
                annotation_for(annotations, Role::QuestionDraftB)?,
            )?;
        }
    }
    ensure!(
        dispatch.len() <= MAX_TEXT_ARTIFACT_BYTES,
        "annotation dispatch exceeds size limit"
    );
    Ok(dispatch)
}

fn annotation_for(annotations: &BTreeMap<Role, Vec<u8>>, role: Role) -> Result<&[u8]> {
    annotations
        .get(&role)
        .map(Vec::as_slice)
        .with_context(|| format!("required draft for {} is missing", role.directory()))
}

fn append_header(target: &mut Vec<u8>, value: &str) {
    target.extend_from_slice(value.as_bytes());
    target.push(b'\n');
}

fn append_section(target: &mut Vec<u8>, name: &str, bytes: &[u8]) -> Result<()> {
    ensure!(
        std::str::from_utf8(bytes).is_ok(),
        "dispatch section is not UTF-8"
    );
    append_header(target, &format!("{name}_BYTES {}", bytes.len()));
    target.extend_from_slice(bytes);
    Ok(())
}

fn frozen_argv() -> Vec<String> {
    [
        CODEX_EXECUTABLE,
        "exec",
        "--ephemeral",
        "--ignore-user-config",
        "--ignore-rules",
        "--strict-config",
        "--skip-git-repo-check",
        "--json",
        "--color",
        "never",
        "--model",
        REQUESTED_MODEL,
        "--config",
        "model_reasoning_effort=\"high\"",
        "--config",
        "model_verbosity=\"low\"",
        "--config",
        "project_doc_max_bytes=0",
        "--config",
        "project_root_markers=[]",
        "--config",
        "default_permissions=\"airwiki_annotator\"",
        "--config",
        "permissions.airwiki_annotator.filesystem={\":root\"=\"deny\"}",
        "--config",
        "permissions.airwiki_annotator.network.enabled=false",
        "--config",
        "approval_policy=\"never\"",
        "--config",
        "shell_environment_policy.inherit=\"none\"",
        "--config",
        "features.shell_tool=false",
        "--config",
        "features.shell_snapshot=false",
        "--config",
        "features.multi_agent=false",
        "--config",
        "features.apps=false",
        "--config",
        "features.plugins=false",
        "--config",
        "features.remote_plugin=false",
        "--config",
        "features.hooks=false",
        "--config",
        "skills.bundled.enabled=false",
        "--config",
        "skills.include_instructions=false",
        "--config",
        "web_search=\"disabled\"",
        "-",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn apply_isolated_environment(command: &mut Command) -> Result<Vec<String>> {
    let home = env::var_os("HOME").context("HOME is required for Codex authentication")?;
    command.env("CODEX_EXEC_SERVER_URL", "none");
    command.env("HOME", home);
    command.env("RUST_LOG", "off");
    Ok(RECORDED_ENVIRONMENT_NAMES
        .into_iter()
        .map(str::to_owned)
        .collect())
}

fn codex_identity() -> Result<CodexIdentity> {
    let executable = env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(CODEX_EXECUTABLE))
        .find(|candidate| candidate.is_file())
        .context("could not find the frozen Codex CLI on PATH")?
        .canonicalize()
        .context("could not resolve the Codex CLI executable")?;
    let binary = read_regular_file(&executable, MAX_CODEX_BINARY_BYTES)?;
    let binary_sha256 = sha256_hex(&binary);
    ensure!(
        binary_sha256 == EXPECTED_CODEX_BINARY_SHA256,
        "Codex CLI binary differs from the frozen executable"
    );

    let mut command = Command::new(&executable);
    command
        .arg("--version")
        .env_clear()
        .env("RUST_LOG", "off")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command
        .output()
        .context("could not inspect the Codex CLI version")?;
    ensure!(
        output.status.success() && output.stderr.is_empty(),
        "Codex CLI version preflight failed"
    );
    let version = std::str::from_utf8(&output.stdout)
        .context("Codex CLI version is not UTF-8")?
        .trim_end_matches('\n');
    ensure!(
        version == EXPECTED_CODEX_VERSION,
        "Codex CLI version differs from the frozen version"
    );
    Ok(CodexIdentity {
        executable,
        version: version.to_owned(),
        binary_sha256,
    })
}

fn read_workspace_assets() -> Result<EvidenceAssets> {
    let root = workspace_root()?.join(EXPERIMENT_DIRECTORY);
    let assets = EvidenceAssets {
        field_guide: read_frozen(&root.join("field-guide.md"), FIELD_GUIDE_SHA256)?,
        source_annotator_prompt: read_frozen(
            &root.join("prompts/source-annotator.md"),
            SOURCE_ANNOTATOR_PROMPT_SHA256,
        )?,
        source_adjudicator_prompt: read_frozen(
            &root.join("prompts/source-adjudicator.md"),
            SOURCE_ADJUDICATOR_PROMPT_SHA256,
        )?,
        question_annotator_prompt: read_frozen(
            &root.join("prompts/question-annotator.md"),
            QUESTION_ANNOTATOR_PROMPT_SHA256,
        )?,
        question_adjudicator_prompt: read_frozen(
            &root.join("prompts/question-adjudicator.md"),
            QUESTION_ADJUDICATOR_PROMPT_SHA256,
        )?,
        source_input: read_frozen(
            &root.join("prepared/source-input.jsonl"),
            SOURCE_INPUT_SHA256,
        )?,
        question_input: read_frozen(
            &root.join("prepared/question-input.jsonl"),
            QUESTION_INPUT_SHA256,
        )?,
    };
    validate_annotation_inputs(assets.as_inputs())?;
    Ok(assets)
}

fn workspace_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .context("xtask has no workspace root")
}

pub(crate) fn ensure_reviewed_main(workspace: &Path) -> Result<String> {
    let branch = git_stdout(workspace, &["branch", "--show-current"])?;
    ensure!(
        branch == "main",
        "typed-evidence execution is allowed only from the reviewed main branch"
    );
    let head = git_stdout(workspace, &["rev-parse", "HEAD"])?;
    let remote_main = git_stdout(workspace, &["rev-parse", "origin/main"])?;
    ensure!(
        is_git_commit(&head) && head == remote_main,
        "typed-evidence execution requires HEAD to equal the reviewed origin/main"
    );
    for arguments in [
        ["diff", "--quiet", "--exit-code", "--"].as_slice(),
        ["diff", "--cached", "--quiet", "--exit-code", "--"].as_slice(),
    ] {
        let status = Command::new("git")
            .args(arguments)
            .current_dir(workspace)
            .env_clear()
            .env("PATH", env::var_os("PATH").unwrap_or_default())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("could not inspect the typed-evidence worktree")?;
        ensure!(
            status.success(),
            "typed-evidence execution requires a clean tracked worktree"
        );
    }
    Ok(head)
}

pub(crate) fn ensure_frozen_runner(workspace: &Path) -> Result<()> {
    let protocol = read_regular_file(
        &workspace.join("docs/typed-evidence-ceiling-v2.md"),
        MAX_TEXT_ARTIFACT_BYTES,
    )?;
    ensure!(
        !protocol
            .windows(b"PENDING_FREEZE".len())
            .any(|window| window == b"PENDING_FREEZE"),
        "typed-evidence preregistration is not frozen"
    );
    validate_runner_digest(workspace, &protocol)
}

fn git_stdout(workspace: &Path, arguments: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(workspace)
        .env_clear()
        .env("PATH", env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("could not inspect the typed-evidence repository")?;
    ensure!(
        output.status.success() && output.stdout.len() <= 128,
        "typed-evidence repository preflight failed"
    );
    let value = std::str::from_utf8(&output.stdout)
        .context("typed-evidence repository metadata is not UTF-8")?
        .trim_end_matches(['\r', '\n']);
    Ok(value.to_owned())
}

fn read_frozen(path: &Path, expected_sha256: &str) -> Result<Vec<u8>> {
    let bytes = read_regular_file(path, MAX_TEXT_ARTIFACT_BYTES)?;
    ensure_hash(&bytes, expected_sha256, "frozen annotation input")?;
    Ok(bytes)
}

fn validate_runner_digest(workspace: &Path, protocol: &[u8]) -> Result<()> {
    let protocol = std::str::from_utf8(protocol)
        .context("typed-evidence preregistration protocol is not UTF-8")?;
    let mut matches = protocol
        .lines()
        .filter_map(|line| line.strip_prefix(RUNNER_SOURCE_HASH_PREFIX));
    let expected = matches
        .next()
        .context("typed-evidence runner digest is absent from the freeze block")?;
    ensure!(
        matches.next().is_none() && is_lower_sha256(expected),
        "typed-evidence runner digest is not one canonical SHA-256"
    );

    let actual = runner_digest(workspace)?;
    ensure!(
        actual == expected,
        "typed-evidence runner source differs from the frozen digest"
    );
    Ok(())
}

fn runner_digest(workspace: &Path) -> Result<String> {
    let mut digest = Sha256::new();
    for relative_path in RUNNER_SOURCE_PATHS {
        digest.update(relative_path.as_bytes());
        digest.update([0]);
        digest.update(read_regular_file(
            &workspace.join(relative_path),
            MAX_TEXT_ARTIFACT_BYTES,
        )?);
        digest.update([0]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn write_input_assets(directory: &Path, inputs: AnnotationInputs<'_>) -> Result<()> {
    for (name, bytes) in [
        (FIELD_GUIDE_FILE, inputs.field_guide),
        (SOURCE_ANNOTATOR_PROMPT_FILE, inputs.source_annotator_prompt),
        (
            SOURCE_ADJUDICATOR_PROMPT_FILE,
            inputs.source_adjudicator_prompt,
        ),
        (
            QUESTION_ANNOTATOR_PROMPT_FILE,
            inputs.question_annotator_prompt,
        ),
        (
            QUESTION_ADJUDICATOR_PROMPT_FILE,
            inputs.question_adjudicator_prompt,
        ),
        (SOURCE_INPUT_FILE, inputs.source_input),
        (QUESTION_INPUT_FILE, inputs.question_input),
    ] {
        write_new(&directory.join(name), bytes)?;
    }
    Ok(())
}

fn read_evidence_assets(directory: &Path) -> Result<EvidenceAssets> {
    let assets = EvidenceAssets {
        field_guide: read_regular_file(&directory.join(FIELD_GUIDE_FILE), MAX_TEXT_ARTIFACT_BYTES)?,
        source_annotator_prompt: read_regular_file(
            &directory.join(SOURCE_ANNOTATOR_PROMPT_FILE),
            MAX_TEXT_ARTIFACT_BYTES,
        )?,
        source_adjudicator_prompt: read_regular_file(
            &directory.join(SOURCE_ADJUDICATOR_PROMPT_FILE),
            MAX_TEXT_ARTIFACT_BYTES,
        )?,
        question_annotator_prompt: read_regular_file(
            &directory.join(QUESTION_ANNOTATOR_PROMPT_FILE),
            MAX_TEXT_ARTIFACT_BYTES,
        )?,
        question_adjudicator_prompt: read_regular_file(
            &directory.join(QUESTION_ADJUDICATOR_PROMPT_FILE),
            MAX_TEXT_ARTIFACT_BYTES,
        )?,
        source_input: read_regular_file(
            &directory.join(SOURCE_INPUT_FILE),
            MAX_TEXT_ARTIFACT_BYTES,
        )?,
        question_input: read_regular_file(
            &directory.join(QUESTION_INPUT_FILE),
            MAX_TEXT_ARTIFACT_BYTES,
        )?,
    };
    validate_annotation_inputs(AnnotationInputs {
        field_guide: &assets.field_guide,
        source_annotator_prompt: &assets.source_annotator_prompt,
        source_adjudicator_prompt: &assets.source_adjudicator_prompt,
        question_annotator_prompt: &assets.question_annotator_prompt,
        question_adjudicator_prompt: &assets.question_adjudicator_prompt,
        source_input: &assets.source_input,
        question_input: &assets.question_input,
    })?;
    Ok(assets)
}

fn input_hashes(inputs: AnnotationInputs<'_>) -> InputHashes {
    InputHashes {
        field_guide_sha256: sha256_hex(inputs.field_guide),
        source_annotator_prompt_sha256: sha256_hex(inputs.source_annotator_prompt),
        source_adjudicator_prompt_sha256: sha256_hex(inputs.source_adjudicator_prompt),
        question_annotator_prompt_sha256: sha256_hex(inputs.question_annotator_prompt),
        question_adjudicator_prompt_sha256: sha256_hex(inputs.question_adjudicator_prompt),
        source_input_sha256: sha256_hex(inputs.source_input),
        question_input_sha256: sha256_hex(inputs.question_input),
    }
}

fn validate_frozen_asset_hashes(assets: &EvidenceAssets) -> Result<()> {
    for (bytes, expected, label) in [
        (&assets.field_guide[..], FIELD_GUIDE_SHA256, "field guide"),
        (
            &assets.source_annotator_prompt,
            SOURCE_ANNOTATOR_PROMPT_SHA256,
            "source annotator prompt",
        ),
        (
            &assets.source_adjudicator_prompt,
            SOURCE_ADJUDICATOR_PROMPT_SHA256,
            "source adjudicator prompt",
        ),
        (
            &assets.question_annotator_prompt,
            QUESTION_ANNOTATOR_PROMPT_SHA256,
            "question annotator prompt",
        ),
        (
            &assets.question_adjudicator_prompt,
            QUESTION_ADJUDICATOR_PROMPT_SHA256,
            "question adjudicator prompt",
        ),
        (&assets.source_input, SOURCE_INPUT_SHA256, "source input"),
        (
            &assets.question_input,
            QUESTION_INPUT_SHA256,
            "question input",
        ),
    ] {
        ensure_hash(bytes, expected, label)?;
    }
    Ok(())
}

fn root_entry_names() -> Vec<&'static str> {
    let mut names = vec![
        FIELD_GUIDE_FILE,
        SOURCE_ANNOTATOR_PROMPT_FILE,
        SOURCE_ADJUDICATOR_PROMPT_FILE,
        QUESTION_ANNOTATOR_PROMPT_FILE,
        QUESTION_ADJUDICATOR_PROMPT_FILE,
        SOURCE_INPUT_FILE,
        QUESTION_INPUT_FILE,
        MANIFEST_FILE,
    ];
    names.extend(Role::ALL.map(Role::directory));
    names
}

fn ensure_directory_entries(directory: &Path, expected: BTreeSet<String>) -> Result<()> {
    let metadata =
        fs::symlink_metadata(directory).context("could not inspect evidence directory")?;
    ensure!(
        metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
        "evidence path is not a regular directory"
    );
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(directory).context("could not read evidence directory")? {
        let entry = entry.context("could not inspect evidence directory entry")?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("evidence entry name is not UTF-8"))?;
        actual.insert(name);
    }
    ensure!(actual == expected, "evidence directory layout is not exact");
    Ok(())
}

fn directory_is_empty(directory: &Path) -> Result<bool> {
    Ok(fs::read_dir(directory)
        .context("could not inspect annotation working directory")?
        .next()
        .transpose()
        .context("could not inspect annotation working-directory entry")?
        .is_none())
}

fn read_regular_file(path: &Path, maximum_bytes: usize) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).context("could not inspect evidence file")?;
    ensure!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "evidence artifact is not a regular file"
    );
    ensure!(
        metadata.len() <= maximum_bytes as u64,
        "evidence artifact exceeds size limit"
    );
    fs::read(path).context("could not read evidence artifact")
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .context("could not create evidence artifact")?;
    file.write_all(bytes)
        .context("could not write evidence artifact")?;
    file.sync_all()
        .context("could not sync evidence artifact")?;
    Ok(())
}

fn write_canonical_json_new(path: &Path, value: &impl Serialize) -> Result<()> {
    write_new(path, &canonical_json_bytes(value)?)
}

fn canonical_json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec(value).context("could not serialize canonical evidence JSON")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn parse_canonical_json<T>(bytes: &[u8], label: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    validate_jsonl_bytes(bytes, label)?;
    let value: T = serde_json::from_slice(bytes).with_context(|| format!("{label} is invalid"))?;
    ensure!(
        canonical_json_bytes(&value)? == bytes,
        "{label} is not canonical compact JSON with one terminal LF"
    );
    Ok(value)
}

fn ensure_hash(bytes: &[u8], expected: &str, label: &str) -> Result<()> {
    ensure!(
        sha256_hex(bytes) == expected,
        "{label} SHA-256 does not match"
    );
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_git_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE_INPUT: &[u8] = b"{\"source_id\":\"source_001\",\"source_record_sha256\":\"00\",\"title\":\"Atlas\",\"heading\":\"Estado\",\"text\":\"Atlas esta verde.\"}\n";
    const QUESTION_INPUT: &[u8] = b"{\"question_id\":\"question_001\",\"question_record_sha256\":\"11\",\"question\":\"Cual es el estado de Atlas?\"}\n";
    const SOURCE_ANNOTATION: &[u8] = b"{\"source_id\":\"source_001\",\"status\":\"resolved\",\"claims\":[{\"subject\":\"atlas\",\"relation\":\"has_status\",\"object_type\":\"status\",\"object_value\":\"verde\",\"qualifiers\":[],\"polarity\":\"positive\",\"lifecycles\":[\"current\"],\"provenance\":\"direct\",\"support_quote\":\"Atlas esta verde.\"}]}\n";
    const QUESTION_ANNOTATION: &[u8] = b"{\"question_id\":\"question_001\",\"status\":\"resolved\",\"needs\":[{\"subject\":\"atlas\",\"relation\":\"has_status\",\"requested_object_types\":[\"status\"],\"required_qualifiers\":[],\"allowed_polarities\":[\"positive\"],\"required_lifecycles\":[\"current\"],\"allowed_provenances\":[\"attributed\",\"direct\"],\"question_quote\":\"estado de Atlas\"}]}\n";

    fn git(directory: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .args(arguments)
            .current_dir(directory)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn reviewed_repository() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        git(
            directory.path(),
            &["config", "user.email", "tests@airwiki.local"],
        );
        git(directory.path(), &["config", "user.name", "AirWiki Tests"]);
        fs::write(directory.path().join("tracked.txt"), "frozen\n").unwrap();
        git(directory.path(), &["add", "tracked.txt"]);
        git(directory.path(), &["commit", "-m", "test fixture"]);
        git(
            directory.path(),
            &["update-ref", "refs/remotes/origin/main", "HEAD"],
        );
        directory
    }

    fn trace(thread_id: &str, annotation: &[u8]) -> Vec<u8> {
        let annotation = std::str::from_utf8(annotation).unwrap();
        let events = [
            serde_json::json!({"type":"thread.started","thread_id":thread_id}),
            serde_json::json!({"type":"turn.started"}),
            serde_json::json!({"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":""}}),
            serde_json::json!({"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":annotation}}),
            serde_json::json!({"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}),
        ];
        let mut bytes = Vec::new();
        for event in events {
            serde_json::to_writer(&mut bytes, &event).unwrap();
            bytes.push(b'\n');
        }
        bytes
    }

    fn unresolved_records(input: &[u8], id_field: &str) -> Vec<u8> {
        let mut output = Vec::new();
        for line in std::str::from_utf8(input).unwrap().split_terminator('\n') {
            let value: Value = serde_json::from_str(line).unwrap();
            let id = value.get(id_field).and_then(Value::as_str).unwrap();
            output.extend_from_slice(
                format!(
                    "{{\"{id_field}\":{},\"status\":\"unresolved\",\"reason_code\":\"unsupported_structure\"}}\n",
                    serde_json::to_string(id).unwrap()
                )
                .as_bytes(),
            );
        }
        output
    }

    fn create_fake_bundle(directory: &Path, duplicate_threads: bool) -> (Vec<u8>, Vec<u8>) {
        fs::create_dir(directory).unwrap();
        let assets = read_workspace_assets().unwrap();
        let inputs = assets.as_inputs();
        let source_annotation = unresolved_records(inputs.source_input, "source_id");
        let question_annotation = unresolved_records(inputs.question_input, "question_id");
        write_input_assets(directory, inputs).unwrap();
        let mut annotations = BTreeMap::new();
        let mut roles = Vec::new();

        for (index, role) in Role::ALL.into_iter().enumerate() {
            let role_directory = directory.join(role.directory());
            fs::create_dir(&role_directory).unwrap();
            let dispatch = build_dispatch(role, inputs, &annotations).unwrap();
            let annotation = if role.is_source() {
                source_annotation.clone()
            } else {
                question_annotation.clone()
            };
            let thread_id = if duplicate_threads {
                "thread-shared".to_owned()
            } else {
                format!("thread-{index}")
            };
            let trace = trace(&thread_id, &annotation);
            let stderr = Vec::new();
            let invocation = InvocationRecord {
                schema_version: INVOCATION_SCHEMA_VERSION,
                transport_label: TRANSPORT_LABEL.to_owned(),
                role,
                argv: frozen_argv(),
                environment_names: RECORDED_ENVIRONMENT_NAMES
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                codex_cli_version: EXPECTED_CODEX_VERSION.to_owned(),
                codex_binary_sha256: EXPECTED_CODEX_BINARY_SHA256.to_owned(),
                repository_commit: "0".repeat(40),
                working_directory_token: format!("workdir-{:032x}", index + 1),
                working_directory_empty_before: true,
                working_directory_empty_after: true,
                dispatch_sha256: sha256_hex(&dispatch),
                process_success: true,
                process_exit_code: Some(0),
                stdout_sha256: sha256_hex(&trace),
                stderr_size_bytes: 0,
                stderr_sha256: sha256_hex(&stderr),
            };
            let invocation = canonical_json_bytes(&invocation).unwrap();

            write_new(&role_directory.join(DISPATCH_FILE), &dispatch).unwrap();
            write_new(&role_directory.join(INVOCATION_FILE), &invocation).unwrap();
            write_new(&role_directory.join(TRACE_FILE), &trace).unwrap();
            write_new(&role_directory.join(ANNOTATION_FILE), &annotation).unwrap();

            roles.push(RoleEvidence {
                role,
                directory: role.directory().to_owned(),
                dispatch_sha256: sha256_hex(&dispatch),
                invocation_sha256: sha256_hex(&invocation),
                trace_sha256: sha256_hex(&trace),
                stderr_size_bytes: 0,
                stderr_sha256: sha256_hex(&stderr),
                annotation_sha256: sha256_hex(&annotation),
                thread_id,
            });
            annotations.insert(role, annotation);
        }

        let manifest = EvidenceManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            transport_label: TRANSPORT_LABEL.to_owned(),
            requested_model: REQUESTED_MODEL.to_owned(),
            reasoning_setting: REASONING_SETTING.to_owned(),
            codex_cli_version: EXPECTED_CODEX_VERSION.to_owned(),
            codex_binary_sha256: EXPECTED_CODEX_BINARY_SHA256.to_owned(),
            repository_commit: "0".repeat(40),
            inputs: input_hashes(inputs),
            roles,
        };
        write_canonical_json_new(&directory.join(MANIFEST_FILE), &manifest).unwrap();
        (source_annotation, question_annotation)
    }

    #[test]
    fn trace_should_extract_one_completed_agent_message() {
        let evidence = verify_trace(&trace("thread-1", SOURCE_ANNOTATION)).unwrap();

        assert_eq!(evidence.annotation, SOURCE_ANNOTATION);
        assert_eq!(evidence.thread_id, "thread-1");
    }

    #[test]
    fn repository_commit_requires_lowercase_full_sha() {
        assert!(is_git_commit(&"a1".repeat(20)));
        assert!(!is_git_commit(&"A1".repeat(20)));
        assert!(!is_git_commit(&"a1".repeat(19)));
    }

    #[test]
    fn reviewed_main_requires_matching_remote_and_clean_tracked_tree() {
        let directory = reviewed_repository();
        assert!(ensure_reviewed_main(directory.path()).is_ok());

        fs::write(directory.path().join("tracked.txt"), "changed\n").unwrap();
        assert!(ensure_reviewed_main(directory.path()).is_err());
    }

    #[test]
    fn reviewed_main_rejects_an_unmerged_branch() {
        let directory = reviewed_repository();
        git(directory.path(), &["switch", "-c", "experiment"]);

        assert!(ensure_reviewed_main(directory.path()).is_err());
    }

    #[test]
    fn frozen_invocation_removes_local_tools_and_denies_root() {
        let argv = frozen_argv();

        assert!(!argv.iter().any(|value| value == "--sandbox"));
        for required in [
            "permissions.airwiki_annotator.filesystem={\":root\"=\"deny\"}",
            "permissions.airwiki_annotator.network.enabled=false",
            "features.shell_tool=false",
            "features.shell_snapshot=false",
            "features.multi_agent=false",
            "features.apps=false",
            "features.plugins=false",
            "features.remote_plugin=false",
            "web_search=\"disabled\"",
        ] {
            assert!(argv.iter().any(|value| value == required));
        }
        assert_eq!(
            RECORDED_ENVIRONMENT_NAMES,
            ["CODEX_EXEC_SERVER_URL", "HOME", "RUST_LOG"]
        );
    }

    #[test]
    fn runner_digest_matches_the_freeze_block() {
        let workspace = workspace_root().unwrap();
        let protocol = fs::read(workspace.join("docs/typed-evidence-ceiling-v2.md")).unwrap();

        validate_runner_digest(&workspace, &protocol).unwrap();
    }

    #[test]
    fn runner_digest_rejects_a_source_mutation() {
        let directory = tempfile::tempdir().unwrap();
        for relative_path in RUNNER_SOURCE_PATHS {
            let path = directory.path().join(relative_path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("{relative_path}\n")).unwrap();
        }
        let digest = runner_digest(directory.path()).unwrap();
        let protocol = format!("# Freeze\n{RUNNER_SOURCE_HASH_PREFIX}{digest}\n");
        validate_runner_digest(directory.path(), protocol.as_bytes()).unwrap();

        fs::write(directory.path().join(RUNNER_SOURCE_PATHS[0]), "mutated\n").unwrap();
        assert!(validate_runner_digest(directory.path(), protocol.as_bytes()).is_err());
    }

    #[test]
    fn trace_should_reject_tools_errors_and_extra_messages() {
        let tool = b"{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n{\"type\":\"turn.started\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\"}}\n{\"type\":\"turn.completed\"}\n";
        let error = b"{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n{\"type\":\"turn.started\"}\n{\"type\":\"error\",\"message\":\"no\"}\n";
        let mut extra = trace("t", SOURCE_ANNOTATION);
        let terminal =
            b"{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n";
        extra.truncate(extra.len() - terminal.len());
        extra.extend_from_slice(
            format!(
                "{{\"type\":\"item.completed\",\"item\":{{\"type\":\"agent_message\",\"text\":{}}}}}\n",
                serde_json::to_string(std::str::from_utf8(SOURCE_ANNOTATION).unwrap()).unwrap()
            )
            .as_bytes(),
        );
        extra.extend_from_slice(terminal);

        assert!(verify_trace(tool).is_err());
        assert!(verify_trace(error).is_err());
        assert!(verify_trace(&extra).is_err());
    }

    #[test]
    fn trace_should_reject_missing_or_invalid_terminal_event() {
        let mut missing = trace("thread-1", SOURCE_ANNOTATION);
        let terminal =
            b"{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}\n";
        missing.truncate(missing.len() - terminal.len());
        let failed = b"{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n{\"type\":\"turn.started\"}\n{\"type\":\"turn.failed\"}\n";

        assert!(verify_trace(&missing).is_err());
        assert!(verify_trace(failed).is_err());
    }

    #[test]
    fn annotation_should_reject_cr_comment_and_extra_terminal_lf() {
        assert!(canonical_annotation_bytes("{}\r\n").is_err());
        assert!(canonical_annotation_bytes("comment\n").is_err());
        assert!(canonical_annotation_bytes("{}\n\n").is_err());
    }

    #[test]
    fn annotation_should_bind_ids_and_quotes_exactly() {
        let sources = parse_source_inputs(SOURCE_INPUT).unwrap();
        let questions = parse_question_inputs(QUESTION_INPUT).unwrap();
        validate_source_annotations(SOURCE_ANNOTATION, &sources).unwrap();
        validate_question_annotations(QUESTION_ANNOTATION, &questions).unwrap();
        let wrong_id = SOURCE_ANNOTATION
            .windows(b"source_001".len())
            .position(|window| window == b"source_001")
            .map(|position| {
                let mut bytes = SOURCE_ANNOTATION.to_vec();
                bytes[position..position + b"source_001".len()].copy_from_slice(b"source_999");
                bytes
            })
            .unwrap();
        let wrong_quote = SOURCE_ANNOTATION
            .windows(b"verde".len())
            .rposition(|window| window == b"verde")
            .map(|position| {
                let mut bytes = SOURCE_ANNOTATION.to_vec();
                bytes[position..position + b"verde".len()].copy_from_slice(b"ambar");
                bytes
            })
            .unwrap();

        assert!(validate_source_annotations(&wrong_id, &sources).is_err());
        assert!(validate_source_annotations(&wrong_quote, &sources).is_err());
    }

    #[test]
    fn evidence_verifier_should_accept_complete_bundle() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("evidence");
        let (source, question) = create_fake_bundle(&directory, false);

        let evidence = verify_evidence(&directory).unwrap();

        assert_eq!(evidence.source_adjudication, source);
        assert_eq!(evidence.question_adjudication, question);
    }

    #[test]
    fn evidence_verifier_should_reject_byte_mutation_and_reused_threads() {
        let temporary = tempfile::tempdir().unwrap();
        let mutated = temporary.path().join("mutated");
        let _ = create_fake_bundle(&mutated, false);
        let trace_path = mutated
            .join(Role::SourceDraftA.directory())
            .join(TRACE_FILE);
        let mut bytes = fs::read(&trace_path).unwrap();
        bytes[0] = b'[';
        fs::write(trace_path, bytes).unwrap();

        let duplicate = temporary.path().join("duplicate");
        let _ = create_fake_bundle(&duplicate, true);

        assert!(verify_evidence(&mutated).is_err());
        assert!(verify_evidence(&duplicate).is_err());
    }
}
