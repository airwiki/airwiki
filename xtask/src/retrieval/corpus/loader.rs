use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::{self, File},
    io::{Cursor, Read},
    path::{Component, Path},
};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use super::{
    CorpusArtifact, CorpusExpectation, CorpusManifest, CorpusSelection, CorpusSplit,
    HardNegativeKind, PassageRef,
};

const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ZIP_MEMBER_BYTES: u64 = 16 * 1024 * 1024;
const MAX_NEED_CHARS: usize = 4_096;
const MAX_ANSWER_CHARS: usize = 1_024;
const MAX_PASSAGE_CHARS: usize = 16_000;

pub(in crate::retrieval) const CANDIDATE_ORDER_POLICY_VERSION: &str =
    "airwiki-answerability-candidate-order-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::retrieval) struct LoadedCorpusSummary {
    pub(in crate::retrieval) artifact_count: usize,
    pub(in crate::retrieval) selection_count: usize,
    pub(in crate::retrieval) answerable_count: usize,
    pub(in crate::retrieval) unanswerable_count: usize,
    pub(in crate::retrieval) candidate_count: usize,
}

pub(in crate::retrieval) struct LoadedCorpus {
    pub(in crate::retrieval) corpus_id: String,
    pub(in crate::retrieval) manifest_sha256: String,
    pub(in crate::retrieval) selections: Vec<LoadedSelection>,
    artifact_count: usize,
}

impl LoadedCorpus {
    pub(in crate::retrieval) fn summary(&self) -> LoadedCorpusSummary {
        let mut answerable_count = 0;
        let mut unanswerable_count = 0;
        let mut candidate_count = 0;
        for selection in &self.selections {
            match selection.expectation {
                CorpusExpectation::Answerable => answerable_count += 1,
                CorpusExpectation::Unanswerable => unanswerable_count += 1,
            }
            candidate_count += selection.candidates.len();

            // These content-bearing fields intentionally have no Debug or
            // serialization implementation. Reading them here also keeps the
            // complete internal contract covered until the model evaluator is
            // added.
            let _ = (
                &selection.id,
                selection.split,
                &selection.group_id,
                &selection.need_id,
                selection.need_kind,
                &selection.need,
                &selection.reference_answers,
            );
            for candidate in &selection.candidates {
                let _ = (&candidate.id, &candidate.passage, candidate.role);
            }
        }

        LoadedCorpusSummary {
            artifact_count: self.artifact_count,
            selection_count: self.selections.len(),
            answerable_count,
            unanswerable_count,
            candidate_count,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::retrieval) enum NeedKind {
    Question,
    Claim,
}

pub(in crate::retrieval) struct LoadedSelection {
    pub(in crate::retrieval) id: String,
    pub(in crate::retrieval) split: CorpusSplit,
    pub(in crate::retrieval) group_id: String,
    pub(in crate::retrieval) need_id: String,
    pub(in crate::retrieval) expectation: CorpusExpectation,
    pub(in crate::retrieval) need_kind: NeedKind,
    pub(in crate::retrieval) need: String,
    pub(in crate::retrieval) reference_answers: Vec<String>,
    pub(in crate::retrieval) candidates: Vec<LoadedCandidate>,
}

pub(in crate::retrieval) struct LoadedCandidate {
    pub(in crate::retrieval) id: String,
    pub(in crate::retrieval) passage: String,
    pub(in crate::retrieval) role: CandidateRole,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::retrieval) enum CandidateRole {
    Support,
    HardNegative(HardNegativeKind),
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DatasetKey {
    artifact_path: String,
    member_path: Option<String>,
}

enum ParsedDataset {
    Squad(SquadDataset),
    ContractNli(ContractNliDataset),
}

struct ResolvedNeed {
    kind: NeedKind,
    text: String,
    reference_answers: Vec<String>,
}

pub(super) fn load_verified(
    manifest: CorpusManifest,
    manifest_sha256: String,
    source_root: &Path,
) -> Result<LoadedCorpus> {
    validate_source_root(source_root)?;
    let required = required_datasets(&manifest)?;
    let parsed = load_required_datasets(&manifest, source_root, &required)?;
    let artifact_count = required
        .keys()
        .map(|key| key.artifact_path.as_str())
        .collect::<BTreeSet<_>>()
        .len();

    let selections = manifest
        .selections
        .iter()
        .map(|selection| load_selection(selection, &parsed))
        .collect::<Result<Vec<_>>>()?;

    Ok(LoadedCorpus {
        corpus_id: manifest.corpus_id,
        manifest_sha256,
        selections,
        artifact_count,
    })
}

fn validate_source_root(source_root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source_root)
        .context("answerability corpus source root is unavailable")?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "answerability corpus source root cannot be a symlink"
    );
    ensure!(
        metadata.is_dir(),
        "answerability corpus source root must be a directory"
    );
    Ok(())
}

fn required_datasets(manifest: &CorpusManifest) -> Result<BTreeMap<DatasetKey, String>> {
    let mut required = BTreeMap::new();
    for selection in &manifest.selections {
        insert_dataset_key(
            &mut required,
            dataset_key(&selection.artifact_path, selection.member_path.as_deref()),
            &selection.source_id,
        )?;
        for support in &selection.supports {
            insert_dataset_key(
                &mut required,
                dataset_key(&support.artifact_path, support.member_path.as_deref()),
                &selection.source_id,
            )?;
        }
        for negative in &selection.hard_negatives {
            insert_dataset_key(
                &mut required,
                dataset_key(&negative.artifact_path, negative.member_path.as_deref()),
                &selection.source_id,
            )?;
        }
    }
    Ok(required)
}

fn insert_dataset_key(
    required: &mut BTreeMap<DatasetKey, String>,
    key: DatasetKey,
    source_id: &str,
) -> Result<()> {
    if let Some(existing) = required.insert(key, source_id.to_owned()) {
        ensure!(
            existing == source_id,
            "one corpus artifact member cannot belong to multiple sources"
        );
    }
    Ok(())
}

fn load_required_datasets(
    manifest: &CorpusManifest,
    source_root: &Path,
    required: &BTreeMap<DatasetKey, String>,
) -> Result<BTreeMap<DatasetKey, ParsedDataset>> {
    let mut by_artifact = BTreeMap::<&str, Vec<(&DatasetKey, &str)>>::new();
    for (key, source_id) in required {
        by_artifact
            .entry(key.artifact_path.as_str())
            .or_default()
            .push((key, source_id));
    }

    let mut parsed = BTreeMap::new();
    for (artifact_path, datasets) in by_artifact {
        let artifact = find_artifact(manifest, artifact_path)?;
        let artifact_bytes = read_verified_artifact(source_root, artifact)?;
        for (key, source_id) in datasets {
            let member_bytes;
            let bytes = if let Some(member_path) = key.member_path.as_deref() {
                member_bytes = read_zip_member(&artifact_bytes, member_path)?;
                member_bytes.as_slice()
            } else {
                artifact_bytes.as_slice()
            };
            let dataset = parse_dataset(source_id, bytes)?;
            parsed.insert(key.clone(), dataset);
        }
    }
    Ok(parsed)
}

fn find_artifact<'a>(
    manifest: &'a CorpusManifest,
    artifact_path: &str,
) -> Result<&'a CorpusArtifact> {
    manifest
        .sources
        .iter()
        .flat_map(|source| source.artifacts.iter())
        .find(|artifact| artifact.path == artifact_path)
        .context("answerability selection references an unknown pinned artifact")
}

fn read_verified_artifact(source_root: &Path, artifact: &CorpusArtifact) -> Result<Vec<u8>> {
    ensure!(
        artifact.size <= MAX_ARTIFACT_BYTES,
        "pinned corpus artifact exceeds the verifier size limit"
    );
    let path = checked_artifact_path(source_root, &artifact.path)?;
    let file = File::open(&path)
        .with_context(|| format!("failed to open pinned artifact `{}`", artifact.path))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect pinned artifact `{}`", artifact.path))?;
    ensure!(
        metadata.is_file(),
        "pinned artifact `{}` must be a regular file",
        artifact.path
    );
    ensure!(
        metadata.len() == artifact.size,
        "pinned artifact `{}` has an unexpected byte size",
        artifact.path
    );

    let capacity = usize::try_from(artifact.size)
        .context("pinned artifact size is unsupported on this platform")?;
    let limit = artifact
        .size
        .checked_add(1)
        .context("pinned artifact read limit overflowed")?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(limit)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read pinned artifact `{}`", artifact.path))?;
    ensure!(
        u64::try_from(bytes.len()).context("pinned artifact byte count overflowed")?
            == artifact.size,
        "pinned artifact `{}` changed while being read",
        artifact.path
    );
    let actual_sha256 = hex::encode(Sha256::digest(&bytes));
    ensure!(
        actual_sha256 == artifact.sha256,
        "pinned artifact `{}` failed SHA-256 verification",
        artifact.path
    );
    Ok(bytes)
}

fn checked_artifact_path(source_root: &Path, relative_path: &str) -> Result<std::path::PathBuf> {
    let mut current = source_root.to_path_buf();
    let mut components = Path::new(relative_path).components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(component) = component else {
            anyhow::bail!("pinned artifact path is not normalized");
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("pinned artifact `{relative_path}` is unavailable"))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "pinned artifact `{relative_path}` cannot traverse a symlink"
        );
        if components.peek().is_some() {
            ensure!(
                metadata.is_dir(),
                "pinned artifact `{relative_path}` has a non-directory parent"
            );
        }
    }
    Ok(current)
}

fn read_zip_member(archive_bytes: &[u8], member_path: &str) -> Result<Vec<u8>> {
    let reader = Cursor::new(archive_bytes);
    let mut archive = ZipArchive::new(reader).context("failed to open pinned corpus archive")?;
    let index = archive
        .index_for_name(member_path)
        .context("pinned corpus archive is missing the selected member")?;
    {
        let entry = archive
            .by_index_raw(index)
            .context("failed to inspect pinned corpus archive")?;
        validate_zip_entry(&entry, member_path)?;
    }
    let mut entry = archive
        .by_index(index)
        .context("failed to read the selected corpus archive member")?;
    validate_zip_entry(&entry, member_path)?;

    let capacity = usize::try_from(entry.size())
        .context("selected corpus archive member size is unsupported")?;
    let mut bytes = Vec::with_capacity(capacity);
    entry
        .by_ref()
        .take(MAX_ZIP_MEMBER_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("failed to read the selected corpus archive member")?;
    ensure!(
        u64::try_from(bytes.len()).context("selected corpus member byte count overflowed")?
            == entry.size(),
        "selected corpus archive member changed while being read"
    );
    Ok(bytes)
}

fn validate_zip_entry<R: Read>(entry: &zip::read::ZipFile<'_, R>, member_path: &str) -> Result<()> {
    validate_zip_entry_properties(
        entry.name_raw() == member_path.as_bytes()
            && entry.enclosed_name().as_deref() == Some(Path::new(member_path)),
        entry.is_file(),
        zip_entry_is_symlink(entry),
        entry.encrypted(),
        entry.size(),
    )
}

fn zip_entry_is_symlink<R: Read>(entry: &zip::read::ZipFile<'_, R>) -> bool {
    entry
        .unix_mode()
        .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
}

fn validate_zip_entry_properties(
    has_safe_exact_name: bool,
    is_file: bool,
    is_symlink: bool,
    is_encrypted: bool,
    size: u64,
) -> Result<()> {
    ensure!(
        has_safe_exact_name,
        "selected corpus archive member has an unsafe path"
    );
    ensure!(
        is_file && !is_symlink,
        "selected corpus archive member must be a regular file"
    );
    ensure!(
        !is_encrypted,
        "selected corpus archive member cannot be encrypted"
    );
    ensure!(
        size <= MAX_ZIP_MEMBER_BYTES,
        "selected corpus archive member exceeds the verifier size limit"
    );
    Ok(())
}

fn parse_dataset(source_id: &str, bytes: &[u8]) -> Result<ParsedDataset> {
    match source_id {
        "squad_v2" => parse_squad_dataset(bytes, "v2.0"),
        "xquad_en_es" => parse_squad_dataset(bytes, "1.1"),
        "contract_nli" => serde_json::from_slice(bytes)
            .map(ParsedDataset::ContractNli)
            .context("failed to parse the pinned ContractNLI artifact member"),
        _ => anyhow::bail!("answerability corpus source has no supported parser"),
    }
}

fn parse_squad_dataset(bytes: &[u8], expected_version: &str) -> Result<ParsedDataset> {
    let dataset: SquadDataset =
        serde_json::from_slice(bytes).context("failed to parse a pinned SQuAD-family artifact")?;
    ensure!(
        dataset.version == expected_version,
        "pinned SQuAD-family artifact has an unexpected source version"
    );
    Ok(ParsedDataset::Squad(dataset))
}

fn load_selection(
    selection: &CorpusSelection,
    datasets: &BTreeMap<DatasetKey, ParsedDataset>,
) -> Result<LoadedSelection> {
    let primary_key = dataset_key(&selection.artifact_path, selection.member_path.as_deref());
    let primary = datasets
        .get(&primary_key)
        .context("verified corpus is missing a primary dataset")?;
    let resolved = match primary {
        ParsedDataset::Squad(dataset) => resolve_squad_need(dataset, selection)?,
        ParsedDataset::ContractNli(dataset) => resolve_contract_need(dataset, selection)?,
    };

    let mut candidates =
        Vec::with_capacity(selection.supports.len() + selection.hard_negatives.len());
    for support in &selection.supports {
        candidates.push(UnnumberedCandidate {
            locator_key: canonical_locator_key(support),
            passage: resolve_passage(datasets, support)?,
            role: CandidateRole::Support,
        });
    }
    for negative in &selection.hard_negatives {
        let reference = PassageRef {
            artifact_path: negative.artifact_path.clone(),
            member_path: negative.member_path.clone(),
            document_id: negative.document_id.clone(),
            segment_id: negative.segment_id.clone(),
        };
        candidates.push(UnnumberedCandidate {
            locator_key: canonical_locator_key(&reference),
            passage: resolve_passage(datasets, &reference)?,
            role: CandidateRole::HardNegative(negative.kind),
        });
    }
    let candidates = number_candidates_blindly(&selection.id, candidates);

    Ok(LoadedSelection {
        id: selection.id.clone(),
        split: selection.split,
        group_id: selection.group_id.clone(),
        need_id: selection.need_id.clone(),
        expectation: selection.expectation,
        need_kind: resolved.kind,
        need: resolved.text,
        reference_answers: resolved.reference_answers,
        candidates,
    })
}

struct UnnumberedCandidate {
    locator_key: String,
    passage: String,
    role: CandidateRole,
}

fn canonical_locator_key(reference: &PassageRef) -> String {
    let member_path = reference.member_path.as_deref().unwrap_or_default();
    format!(
        "{}:{}{}:{}{}:{}{}:{}{}",
        reference.artifact_path.len(),
        reference.artifact_path,
        member_path.len(),
        member_path,
        reference.document_id.len(),
        reference.document_id,
        reference.segment_id.len(),
        reference.segment_id,
        usize::from(reference.member_path.is_some()),
    )
}

fn number_candidates_blindly(
    selection_id: &str,
    candidates: Vec<UnnumberedCandidate>,
) -> Vec<LoadedCandidate> {
    let mut candidates = candidates
        .into_iter()
        .map(|candidate| {
            let mut hasher = Sha256::new();
            hasher.update(CANDIDATE_ORDER_POLICY_VERSION.as_bytes());
            hasher.update([0]);
            hasher.update((selection_id.len() as u128).to_le_bytes());
            hasher.update(selection_id.as_bytes());
            hasher.update((candidate.locator_key.len() as u128).to_le_bytes());
            hasher.update(candidate.locator_key.as_bytes());
            (hasher.finalize(), candidate)
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by(|(left_hash, left), (right_hash, right)| {
        left_hash
            .cmp(right_hash)
            .then_with(|| left.locator_key.cmp(&right.locator_key))
    });
    candidates
        .into_iter()
        .enumerate()
        .map(|(index, (_, candidate))| LoadedCandidate {
            id: format!("c{index}"),
            passage: candidate.passage,
            role: candidate.role,
        })
        .collect()
}

fn resolve_squad_need(dataset: &SquadDataset, selection: &CorpusSelection) -> Result<ResolvedNeed> {
    let document_index = parse_prefixed_index(&selection.document_id, "data_")?;
    let document = dataset
        .data
        .get(document_index)
        .context("SQuAD selection document index is out of bounds")?;
    let mut matched = None;
    for (paragraph_index, paragraph) in document.paragraphs.iter().enumerate() {
        for qa in &paragraph.qas {
            if qa.id == selection.upstream_record_id {
                ensure!(matched.is_none(), "SQuAD question identifier is not unique");
                matched = Some((paragraph_index, paragraph, qa));
            }
        }
    }
    let (paragraph_index, paragraph, qa) =
        matched.context("SQuAD selection question identifier was not found")?;
    validate_bounded_text(&qa.question, MAX_NEED_CHARS, "SQuAD question")?;

    match selection.expectation {
        CorpusExpectation::Answerable => {
            ensure!(
                !qa.is_impossible && !qa.answers.is_empty(),
                "answerable SQuAD selection does not have source answers"
            );
            let expected_segment = format!("paragraph_{paragraph_index}");
            ensure!(
                selection.supports.iter().any(|support| {
                    support.artifact_path == selection.artifact_path
                        && support.member_path == selection.member_path
                        && support.document_id == selection.document_id
                        && support.segment_id == expected_segment
                }),
                "answerable SQuAD selection does not include its question paragraph as support"
            );
        }
        CorpusExpectation::Unanswerable => {
            ensure!(
                qa.is_impossible && qa.answers.is_empty(),
                "unanswerable SQuAD selection disagrees with the source label"
            );
            let expected_segment = format!("paragraph_{paragraph_index}");
            ensure!(
                selection.hard_negatives.iter().any(|negative| {
                    negative.artifact_path == selection.artifact_path
                        && negative.member_path == selection.member_path
                        && negative.document_id == selection.document_id
                        && negative.segment_id == expected_segment
                }),
                "unanswerable SQuAD selection does not include its adversarial paragraph"
            );
        }
    }

    let mut reference_answers = Vec::with_capacity(qa.answers.len());
    for answer in &qa.answers {
        validate_bounded_text(&answer.text, MAX_ANSWER_CHARS, "SQuAD reference answer")?;
        let end = answer
            .answer_start
            .checked_add(answer.text.chars().count())
            .context("SQuAD reference answer offset overflowed")?;
        let source_answer = slice_char_range(&paragraph.context, answer.answer_start, end)
            .context("SQuAD reference answer offset is out of bounds")?;
        ensure!(
            source_answer == answer.text,
            "SQuAD reference answer offset does not match the source context"
        );
        reference_answers.push(answer.text.clone());
    }

    Ok(ResolvedNeed {
        kind: NeedKind::Question,
        text: qa.question.clone(),
        reference_answers,
    })
}

fn resolve_contract_need(
    dataset: &ContractNliDataset,
    selection: &CorpusSelection,
) -> Result<ResolvedNeed> {
    let document = contract_document(dataset, &selection.document_id)?;
    let [annotation_set] = document.annotation_sets.as_slice() else {
        anyhow::bail!("selected ContractNLI document must have exactly one annotation set");
    };
    let annotation = annotation_set
        .annotations
        .get(&selection.upstream_record_id)
        .context("ContractNLI annotation identifier was not found")?;
    let label = dataset
        .labels
        .get(&selection.upstream_record_id)
        .context("ContractNLI hypothesis identifier was not found")?;
    validate_bounded_text(&label.hypothesis, MAX_NEED_CHARS, "ContractNLI hypothesis")?;

    match selection.expectation {
        CorpusExpectation::Answerable => {
            ensure!(
                annotation.choice == ContractChoice::Entailment,
                "answerable ContractNLI selection is not entailed upstream"
            );
            ensure!(
                selection.supports.iter().all(|support| {
                    support.artifact_path == selection.artifact_path
                        && support.member_path == selection.member_path
                        && support.document_id == selection.document_id
                }),
                "ContractNLI support locators must remain within the selected document"
            );
            let annotated = annotation.spans.iter().copied().collect::<BTreeSet<_>>();
            ensure!(
                annotated.len() == annotation.spans.len(),
                "ContractNLI annotation contains duplicate evidence spans"
            );
            let declared = selection
                .supports
                .iter()
                .map(|support| parse_prefixed_index(&support.segment_id, "span_"))
                .collect::<Result<BTreeSet<_>>>()?;
            ensure!(
                annotated == declared,
                "ContractNLI support locators disagree with the upstream evidence spans"
            );
        }
        CorpusExpectation::Unanswerable => ensure!(
            matches!(
                annotation.choice,
                ContractChoice::Contradiction | ContractChoice::NotMentioned
            ),
            "unanswerable ContractNLI selection is entailed upstream"
        ),
    }

    Ok(ResolvedNeed {
        kind: NeedKind::Claim,
        text: label.hypothesis.clone(),
        reference_answers: Vec::new(),
    })
}

fn resolve_passage(
    datasets: &BTreeMap<DatasetKey, ParsedDataset>,
    reference: &PassageRef,
) -> Result<String> {
    let key = dataset_key(&reference.artifact_path, reference.member_path.as_deref());
    let dataset = datasets
        .get(&key)
        .context("verified corpus is missing a referenced passage dataset")?;
    let passage = match dataset {
        ParsedDataset::Squad(dataset) => {
            let document_index = parse_prefixed_index(&reference.document_id, "data_")?;
            let paragraph_index = parse_prefixed_index(&reference.segment_id, "paragraph_")?;
            dataset
                .data
                .get(document_index)
                .and_then(|document| document.paragraphs.get(paragraph_index))
                .map(|paragraph| paragraph.context.clone())
                .context("SQuAD passage locator is out of bounds")?
        }
        ParsedDataset::ContractNli(dataset) => {
            let document = contract_document(dataset, &reference.document_id)?;
            let span_index = parse_prefixed_index(&reference.segment_id, "span_")?;
            let [start, end] = *document
                .spans
                .get(span_index)
                .context("ContractNLI span locator is out of bounds")?;
            slice_char_range(&document.text, start, end)
                .context("ContractNLI evidence span is out of bounds")?
                .to_owned()
        }
    };
    validate_bounded_text(&passage, MAX_PASSAGE_CHARS, "selected corpus passage")?;
    Ok(passage)
}

fn contract_document<'a>(
    dataset: &'a ContractNliDataset,
    document_id: &str,
) -> Result<&'a ContractDocument> {
    let numeric_id = document_id
        .parse::<u64>()
        .context("ContractNLI document identifier is not numeric")?;
    ensure!(
        numeric_id.to_string() == document_id,
        "ContractNLI document identifier is not canonical"
    );
    let mut matches = dataset
        .documents
        .iter()
        .filter(|document| document.id == numeric_id);
    let document = matches
        .next()
        .context("ContractNLI document identifier was not found")?;
    ensure!(
        matches.next().is_none(),
        "ContractNLI document identifier is not unique"
    );
    Ok(document)
}

fn parse_prefixed_index(value: &str, prefix: &str) -> Result<usize> {
    let digits = value
        .strip_prefix(prefix)
        .context("corpus locator has an unexpected prefix")?;
    let index = digits
        .parse::<usize>()
        .context("corpus locator index is not numeric")?;
    ensure!(
        format!("{prefix}{index}") == value,
        "corpus locator index is not canonical"
    );
    Ok(index)
}

fn dataset_key(artifact_path: &str, member_path: Option<&str>) -> DatasetKey {
    DatasetKey {
        artifact_path: artifact_path.to_owned(),
        member_path: member_path.map(str::to_owned),
    }
}

fn validate_bounded_text(value: &str, max_chars: usize, kind: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{kind} must not be empty");
    ensure!(
        value.chars().count() <= max_chars,
        "{kind} exceeds the verifier character limit"
    );
    Ok(())
}

fn slice_char_range(value: &str, start: usize, end: usize) -> Option<&str> {
    if start > end {
        return None;
    }
    let start_byte = char_boundary(value, start)?;
    let end_byte = char_boundary(value, end)?;
    value.get(start_byte..end_byte)
}

fn char_boundary(value: &str, index: usize) -> Option<usize> {
    if index == value.chars().count() {
        Some(value.len())
    } else {
        value.char_indices().nth(index).map(|(offset, _)| offset)
    }
}

#[derive(Deserialize)]
struct SquadDataset {
    version: String,
    data: Vec<SquadDocument>,
}

#[derive(Deserialize)]
struct SquadDocument {
    paragraphs: Vec<SquadParagraph>,
}

#[derive(Deserialize)]
struct SquadParagraph {
    context: String,
    qas: Vec<SquadQa>,
}

#[derive(Deserialize)]
struct SquadQa {
    id: String,
    question: String,
    answers: Vec<SquadAnswer>,
    #[serde(default)]
    is_impossible: bool,
}

#[derive(Deserialize)]
struct SquadAnswer {
    text: String,
    answer_start: usize,
}

#[derive(Deserialize)]
struct ContractNliDataset {
    documents: Vec<ContractDocument>,
    labels: HashMap<String, ContractLabel>,
}

#[derive(Deserialize)]
struct ContractDocument {
    id: u64,
    text: String,
    spans: Vec<[usize; 2]>,
    annotation_sets: Vec<ContractAnnotationSet>,
}

#[derive(Deserialize)]
struct ContractAnnotationSet {
    annotations: HashMap<String, ContractAnnotation>,
}

#[derive(Deserialize)]
struct ContractAnnotation {
    choice: ContractChoice,
    spans: Vec<usize>,
}

#[derive(Deserialize, PartialEq, Eq)]
enum ContractChoice {
    Entailment,
    Contradiction,
    NotMentioned,
}

#[derive(Deserialize)]
struct ContractLabel {
    hypothesis: String,
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde_json::json;
    use tempfile::tempdir;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    use super::*;
    use crate::retrieval::corpus::{CorpusSource, HardNegative};

    fn passage(document_id: &str, segment_id: &str) -> PassageRef {
        PassageRef {
            artifact_path: "source/data.json".to_owned(),
            member_path: None,
            document_id: document_id.to_owned(),
            segment_id: segment_id.to_owned(),
        }
    }

    fn squad_selection(expectation: CorpusExpectation) -> CorpusSelection {
        CorpusSelection {
            id: "selection_one".to_owned(),
            source_id: "squad_v2".to_owned(),
            artifact_path: "source/data.json".to_owned(),
            member_path: None,
            split: CorpusSplit::Training,
            group_id: "group_one".to_owned(),
            document_id: "data_0".to_owned(),
            upstream_record_id: "qa_one".to_owned(),
            need_id: "need_one".to_owned(),
            expectation,
            supports: if expectation == CorpusExpectation::Answerable {
                vec![passage("data_0", "paragraph_0")]
            } else {
                Vec::new()
            },
            hard_negatives: if expectation == CorpusExpectation::Unanswerable {
                vec![HardNegative {
                    artifact_path: "source/data.json".to_owned(),
                    member_path: None,
                    document_id: "data_0".to_owned(),
                    segment_id: "paragraph_0".to_owned(),
                    kind: HardNegativeKind::RelatedButUnanswered,
                }]
            } else {
                Vec::new()
            },
        }
    }

    fn squad_dataset(is_impossible: bool, answer_start: usize) -> SquadDataset {
        SquadDataset {
            version: "v2.0".to_owned(),
            data: vec![SquadDocument {
                paragraphs: vec![SquadParagraph {
                    context: "mañana café azul".to_owned(),
                    qas: vec![SquadQa {
                        id: "qa_one".to_owned(),
                        question: "¿Qué lugar se menciona?".to_owned(),
                        answers: if is_impossible {
                            Vec::new()
                        } else {
                            vec![SquadAnswer {
                                text: "café".to_owned(),
                                answer_start,
                            }]
                        },
                        is_impossible,
                    }],
                }],
            }],
        }
    }

    fn contract_dataset() -> ContractNliDataset {
        ContractNliDataset {
            documents: vec![ContractDocument {
                id: 34,
                text: "alpha beta gamma".to_owned(),
                spans: vec![[0, 5], [6, 10], [11, 16]],
                annotation_sets: vec![ContractAnnotationSet {
                    annotations: HashMap::from([
                        (
                            "nda-16".to_owned(),
                            ContractAnnotation {
                                choice: ContractChoice::Entailment,
                                spans: vec![0, 1],
                            },
                        ),
                        (
                            "nda-11".to_owned(),
                            ContractAnnotation {
                                choice: ContractChoice::NotMentioned,
                                spans: Vec::new(),
                            },
                        ),
                    ]),
                }],
            }],
            labels: HashMap::from([
                (
                    "nda-16".to_owned(),
                    ContractLabel {
                        hypothesis: "The agreement contains alpha and beta.".to_owned(),
                    },
                ),
                (
                    "nda-11".to_owned(),
                    ContractLabel {
                        hypothesis: "The agreement contains delta.".to_owned(),
                    },
                ),
            ]),
        }
    }

    fn contract_selection(record_id: &str, expectation: CorpusExpectation) -> CorpusSelection {
        CorpusSelection {
            id: format!("selection_{record_id}"),
            source_id: "contract_nli".to_owned(),
            artifact_path: "contract/data.zip".to_owned(),
            member_path: Some("contract/train.json".to_owned()),
            split: CorpusSplit::Calibration,
            group_id: "contract_34".to_owned(),
            document_id: "34".to_owned(),
            upstream_record_id: record_id.to_owned(),
            need_id: format!("need_{record_id}"),
            expectation,
            supports: if expectation == CorpusExpectation::Answerable {
                vec![
                    PassageRef {
                        artifact_path: "contract/data.zip".to_owned(),
                        member_path: Some("contract/train.json".to_owned()),
                        document_id: "34".to_owned(),
                        segment_id: "span_0".to_owned(),
                    },
                    PassageRef {
                        artifact_path: "contract/data.zip".to_owned(),
                        member_path: Some("contract/train.json".to_owned()),
                        document_id: "34".to_owned(),
                        segment_id: "span_1".to_owned(),
                    },
                ]
            } else {
                Vec::new()
            },
            hard_negatives: Vec::new(),
        }
    }

    fn artifact(path: &str, bytes: &[u8]) -> CorpusArtifact {
        CorpusArtifact {
            path: path.to_owned(),
            sha256: hex::encode(Sha256::digest(bytes)),
            size: u64::try_from(bytes.len()).unwrap(),
        }
    }

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (name, contents) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn corpus_source(id: &str, artifacts: Vec<CorpusArtifact>) -> CorpusSource {
        CorpusSource {
            id: id.to_owned(),
            repository: "https://example.invalid/corpus".to_owned(),
            revision: "1".repeat(40),
            license: "CC-BY-4.0".to_owned(),
            artifacts,
        }
    }

    fn corpus_manifest(source: CorpusSource, selections: Vec<CorpusSelection>) -> CorpusManifest {
        CorpusManifest {
            schema_version: 1,
            corpus_id: "synthetic_corpus".to_owned(),
            sources: vec![source],
            selections,
        }
    }

    fn write_artifact(root: &Path, relative_path: &str, bytes: &[u8]) {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn unicode_character_range_returns_the_expected_slice() {
        let value = "mañana café azul";

        let slice = slice_char_range(value, 7, 11).unwrap();

        assert_eq!(slice, "café");
    }

    #[test]
    fn prefixed_index_rejects_a_non_canonical_leading_zero() {
        let error = parse_prefixed_index("paragraph_01", "paragraph_").unwrap_err();

        assert!(error.to_string().contains("not canonical"));
    }

    #[test]
    fn squad_answerable_need_validates_unicode_reference_offset() {
        let resolved = resolve_squad_need(
            &squad_dataset(false, 7),
            &squad_selection(CorpusExpectation::Answerable),
        )
        .unwrap();

        assert_eq!(resolved.reference_answers, vec!["café"]);
    }

    #[test]
    fn squad_answerable_need_rejects_an_invalid_reference_offset() {
        let error = resolve_squad_need(
            &squad_dataset(false, 8),
            &squad_selection(CorpusExpectation::Answerable),
        )
        .err()
        .unwrap();

        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn squad_unanswerable_need_requires_the_upstream_impossible_label() {
        let resolved = resolve_squad_need(
            &squad_dataset(true, 0),
            &squad_selection(CorpusExpectation::Unanswerable),
        )
        .unwrap();

        assert!(resolved.reference_answers.is_empty());
    }

    #[test]
    fn squad_unanswerable_need_requires_its_question_paragraph_as_a_hard_negative() {
        let mut selection = squad_selection(CorpusExpectation::Unanswerable);
        selection.hard_negatives[0].segment_id = "paragraph_1".to_owned();

        let error = resolve_squad_need(&squad_dataset(true, 0), &selection)
            .err()
            .unwrap();

        assert!(error.to_string().contains("adversarial paragraph"));
    }

    #[test]
    fn squad_parser_rejects_an_unexpected_source_version() {
        let bytes = serde_json::to_vec(&json!({"version": "1.1", "data": []})).unwrap();

        let error = parse_dataset("squad_v2", &bytes).err().unwrap();

        assert!(error.to_string().contains("source version"));
    }

    #[test]
    fn contract_entailment_requires_the_exact_upstream_span_set() {
        let resolved = resolve_contract_need(
            &contract_dataset(),
            &contract_selection("nda-16", CorpusExpectation::Answerable),
        )
        .unwrap();

        assert!(resolved.reference_answers.is_empty());
    }

    #[test]
    fn contract_entailment_rejects_a_support_span_mismatch() {
        let mut selection = contract_selection("nda-16", CorpusExpectation::Answerable);
        selection.supports.pop();

        let error = resolve_contract_need(&contract_dataset(), &selection)
            .err()
            .unwrap();

        assert!(error.to_string().contains("evidence spans"));
    }

    #[test]
    fn contract_entailment_rejects_support_from_another_document() {
        let mut selection = contract_selection("nda-16", CorpusExpectation::Answerable);
        selection.supports[0].document_id = "35".to_owned();

        let error = resolve_contract_need(&contract_dataset(), &selection)
            .err()
            .unwrap();

        assert!(error.to_string().contains("selected document"));
    }

    #[test]
    fn contract_entailment_rejects_duplicate_upstream_evidence_spans() {
        let mut dataset = contract_dataset();
        dataset.documents[0].annotation_sets[0]
            .annotations
            .get_mut("nda-16")
            .unwrap()
            .spans = vec![0, 0];

        let error = resolve_contract_need(
            &dataset,
            &contract_selection("nda-16", CorpusExpectation::Answerable),
        )
        .err()
        .unwrap();

        assert!(error.to_string().contains("duplicate evidence spans"));
    }

    #[test]
    fn contract_not_mentioned_is_an_unanswerable_need() {
        let resolved = resolve_contract_need(
            &contract_dataset(),
            &contract_selection("nda-11", CorpusExpectation::Unanswerable),
        )
        .unwrap();

        assert_eq!(resolved.text, "The agreement contains delta.");
    }

    #[test]
    fn verified_artifact_rejects_a_wrong_size_before_parsing() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("source");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("data.json"), b"private-canary").unwrap();
        let mut metadata = artifact("source/data.json", b"private-canary");
        metadata.size += 1;

        let error = read_verified_artifact(directory.path(), &metadata).unwrap_err();

        assert!(error.to_string().contains("unexpected byte size"));
    }

    #[test]
    fn verified_artifact_hash_error_does_not_include_content_or_local_path() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("source");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("data.json"), b"private-canary").unwrap();
        let mut metadata = artifact("source/data.json", b"private-canary");
        metadata.sha256 = "0".repeat(64);

        let error = read_verified_artifact(directory.path(), &metadata)
            .unwrap_err()
            .to_string();

        assert!(
            !error.contains("private-canary")
                && !error.contains(&directory.path().display().to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn verified_artifact_rejects_a_symlink() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().unwrap();
        let path = directory.path().join("source");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("actual.json"), b"{}").unwrap();
        symlink(path.join("actual.json"), path.join("data.json")).unwrap();

        let error = read_verified_artifact(directory.path(), &artifact("source/data.json", b"{}"))
            .unwrap_err();

        assert!(error.to_string().contains("cannot traverse a symlink"));
    }

    #[test]
    fn zip_member_is_read_by_exact_safe_name() {
        let archive = zip_bytes(&[("contract/train.json", b"{}")]);

        let bytes = read_zip_member(&archive, "contract/train.json").unwrap();

        assert_eq!(bytes, b"{}");
    }

    #[test]
    fn zip_member_rejects_a_traversing_name() {
        let archive = zip_bytes(&[("../train.json", b"{}")]);

        let error = read_zip_member(&archive, "../train.json").unwrap_err();

        assert!(error.to_string().contains("unsafe path"));
    }

    #[test]
    fn zip_member_properties_reject_encryption() {
        let error = validate_zip_entry_properties(true, true, false, true, 1).unwrap_err();

        assert!(error.to_string().contains("encrypted"));
    }

    #[test]
    fn zip_member_properties_reject_a_symlink() {
        let error = validate_zip_entry_properties(true, true, true, false, 1).unwrap_err();

        assert!(error.to_string().contains("regular file"));
    }

    #[test]
    fn zip_member_properties_reject_an_oversized_member() {
        let error =
            validate_zip_entry_properties(true, true, false, false, MAX_ZIP_MEMBER_BYTES + 1)
                .unwrap_err();

        assert!(error.to_string().contains("size limit"));
    }

    #[test]
    fn candidate_ids_and_order_are_neutral_and_independent_of_role() {
        let candidates = vec![
            UnnumberedCandidate {
                locator_key: "locator_alpha".to_owned(),
                passage: "alpha".to_owned(),
                role: CandidateRole::Support,
            },
            UnnumberedCandidate {
                locator_key: "locator_beta".to_owned(),
                passage: "beta".to_owned(),
                role: CandidateRole::HardNegative(HardNegativeKind::WrongRelation),
            },
        ];
        let roles_swapped = vec![
            UnnumberedCandidate {
                locator_key: "locator_beta".to_owned(),
                passage: "beta".to_owned(),
                role: CandidateRole::Support,
            },
            UnnumberedCandidate {
                locator_key: "locator_alpha".to_owned(),
                passage: "alpha".to_owned(),
                role: CandidateRole::HardNegative(HardNegativeKind::WrongRelation),
            },
        ];

        let first = number_candidates_blindly("selection_one", candidates);
        let second = number_candidates_blindly("selection_one", roles_swapped);

        assert_eq!(
            first
                .iter()
                .map(|candidate| &candidate.id)
                .collect::<Vec<_>>(),
            vec!["c0", "c1"]
        );
        assert_eq!(
            first
                .iter()
                .map(|candidate| &candidate.passage)
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|candidate| &candidate.passage)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn load_verified_resolves_a_squad_selection_end_to_end() {
        let directory = tempdir().unwrap();
        let bytes = serde_json::to_vec(&json!({
            "version": "v2.0",
            "data": [{
                "title": "Synthetic",
                "paragraphs": [
                    {
                        "context": "mañana café azul",
                        "qas": [{
                            "id": "qa_one",
                            "question": "¿Qué lugar se menciona?",
                            "answers": [{"text": "café", "answer_start": 7}],
                            "is_impossible": false
                        }]
                    },
                    {
                        "context": "Este párrafo cercano no contiene la respuesta.",
                        "qas": []
                    }
                ]
            }]
        }))
        .unwrap();
        write_artifact(directory.path(), "source/data.json", &bytes);
        let mut selection = squad_selection(CorpusExpectation::Answerable);
        selection.hard_negatives.push(HardNegative {
            artifact_path: "source/data.json".to_owned(),
            member_path: None,
            document_id: "data_0".to_owned(),
            segment_id: "paragraph_1".to_owned(),
            kind: HardNegativeKind::RelatedButUnanswered,
        });
        let manifest = corpus_manifest(
            corpus_source(
                "squad_v2",
                vec![
                    artifact("source/data.json", &bytes),
                    CorpusArtifact {
                        path: "source/unused.json".to_owned(),
                        sha256: "0".repeat(64),
                        size: 1,
                    },
                ],
            ),
            vec![selection],
        );

        let loaded = load_verified(manifest, "a".repeat(64), directory.path()).unwrap();

        assert_eq!(
            loaded.summary(),
            LoadedCorpusSummary {
                artifact_count: 1,
                selection_count: 1,
                answerable_count: 1,
                unanswerable_count: 0,
                candidate_count: 2,
            }
        );
    }

    #[test]
    fn load_verified_resolves_a_contract_zip_selection_end_to_end() {
        let directory = tempdir().unwrap();
        let member_bytes = serde_json::to_vec(&json!({
            "documents": [{
                "id": 34,
                "text": "alpha beta gamma",
                "spans": [[0, 5], [6, 10], [11, 16]],
                "annotation_sets": [{
                    "annotations": {
                        "nda-16": {"choice": "Entailment", "spans": [0, 1]}
                    }
                }]
            }],
            "labels": {
                "nda-16": {"hypothesis": "The agreement contains alpha and beta."}
            }
        }))
        .unwrap();
        let archive = zip_bytes(&[("contract/train.json", &member_bytes)]);
        write_artifact(directory.path(), "contract/data.zip", &archive);
        let mut selection = contract_selection("nda-16", CorpusExpectation::Answerable);
        selection.hard_negatives.push(HardNegative {
            artifact_path: "contract/data.zip".to_owned(),
            member_path: Some("contract/train.json".to_owned()),
            document_id: "34".to_owned(),
            segment_id: "span_2".to_owned(),
            kind: HardNegativeKind::WrongRelation,
        });
        let manifest = corpus_manifest(
            corpus_source(
                "contract_nli",
                vec![artifact("contract/data.zip", &archive)],
            ),
            vec![selection],
        );

        let loaded = load_verified(manifest, "b".repeat(64), directory.path()).unwrap();

        assert_eq!(
            loaded.summary(),
            LoadedCorpusSummary {
                artifact_count: 1,
                selection_count: 1,
                answerable_count: 1,
                unanswerable_count: 0,
                candidate_count: 3,
            }
        );
    }
}
