use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path},
};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

const MANIFEST_SCHEMA_VERSION: u32 = 1;
const SUPPORTED_LICENSES: [&str; 2] = ["CC-BY-SA-4.0", "CC-BY-4.0"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CorpusManifestSummary {
    pub(super) source_count: usize,
    pub(super) artifact_count: usize,
    pub(super) selection_count: usize,
    pub(super) answerable_count: usize,
    pub(super) unanswerable_count: usize,
    pub(super) group_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusManifest {
    schema_version: u32,
    corpus_id: String,
    sources: Vec<CorpusSource>,
    selections: Vec<CorpusSelection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusSource {
    id: String,
    repository: String,
    revision: String,
    license: String,
    artifacts: Vec<CorpusArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusArtifact {
    path: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusSelection {
    id: String,
    source_id: String,
    artifact_path: String,
    member_path: Option<String>,
    split: CorpusSplit,
    group_id: String,
    document_id: String,
    upstream_record_id: String,
    need_id: String,
    expectation: CorpusExpectation,
    supports: Vec<PassageRef>,
    hard_negatives: Vec<HardNegative>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PassageRef {
    artifact_path: String,
    member_path: Option<String>,
    document_id: String,
    segment_id: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum CorpusSplit {
    Training,
    Calibration,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CorpusExpectation {
    Answerable,
    Unanswerable,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HardNegative {
    artifact_path: String,
    member_path: Option<String>,
    document_id: String,
    segment_id: String,
    #[serde(rename = "kind")]
    _kind: HardNegativeKind,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
enum HardNegativeKind {
    WrongSubject,
    WrongRelation,
    WrongScope,
    WrongTime,
    Negated,
    RelatedButUnanswered,
    LexicalOverlap,
}

pub(super) fn validate_manifest(path: &Path) -> Result<CorpusManifestSummary> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read corpus manifest `{}`", path.display()))?;
    let manifest: CorpusManifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse corpus manifest `{}`", path.display()))?;

    validate_manifest_contents(&manifest)
}

fn validate_manifest_contents(manifest: &CorpusManifest) -> Result<CorpusManifestSummary> {
    ensure!(
        manifest.schema_version == MANIFEST_SCHEMA_VERSION,
        "corpus manifest schema_version must be {MANIFEST_SCHEMA_VERSION}"
    );
    validate_id(&manifest.corpus_id, "corpus")?;
    ensure!(
        !manifest.sources.is_empty(),
        "corpus manifest must contain at least one source"
    );
    ensure!(
        !manifest.selections.is_empty(),
        "corpus manifest must contain at least one selection"
    );

    let mut source_ids = HashSet::with_capacity(manifest.sources.len());
    let mut artifact_paths = HashSet::new();
    let mut artifact_count = 0_usize;
    for source in &manifest.sources {
        validate_source(source, &mut artifact_paths)?;
        ensure!(
            source_ids.insert(source.id.as_str()),
            "corpus manifest contains duplicate source id `{}`",
            source.id
        );
        artifact_count = artifact_count
            .checked_add(source.artifacts.len())
            .context("corpus manifest artifact count overflowed")?;
    }

    let mut selection_ids = HashSet::with_capacity(manifest.selections.len());
    let mut need_ids = HashSet::with_capacity(manifest.selections.len());
    let mut upstream_record_locators = HashSet::with_capacity(manifest.selections.len());
    let mut referenced_source_ids = HashSet::with_capacity(manifest.sources.len());
    let mut group_splits = HashMap::new();
    let mut splits = HashSet::new();
    let mut answerable_count = 0_usize;
    let mut unanswerable_count = 0_usize;

    for selection in &manifest.selections {
        validate_selection(
            selection,
            &manifest.sources,
            &mut selection_ids,
            &mut need_ids,
            &mut group_splits,
        )?;
        ensure!(
            upstream_record_locators.insert((
                selection.source_id.as_str(),
                selection.artifact_path.as_str(),
                selection.member_path.as_deref(),
                selection.document_id.as_str(),
                selection.upstream_record_id.as_str(),
            )),
            "corpus manifest contains a duplicate upstream record locator"
        );
        referenced_source_ids.insert(selection.source_id.as_str());
        splits.insert(selection.split);
        match selection.expectation {
            CorpusExpectation::Answerable => answerable_count += 1,
            CorpusExpectation::Unanswerable => unanswerable_count += 1,
        }
    }

    ensure!(
        source_ids == referenced_source_ids,
        "every corpus source must be referenced by at least one selection"
    );
    ensure!(
        splits.contains(&CorpusSplit::Training) && splits.contains(&CorpusSplit::Calibration),
        "corpus manifest must contain both training and calibration selections"
    );
    ensure!(
        answerable_count > 0 && unanswerable_count > 0,
        "corpus manifest must contain both answerable and unanswerable selections"
    );

    Ok(CorpusManifestSummary {
        source_count: manifest.sources.len(),
        artifact_count,
        selection_count: manifest.selections.len(),
        answerable_count,
        unanswerable_count,
        group_count: group_splits.len(),
    })
}

fn validate_source<'a>(
    source: &'a CorpusSource,
    artifact_paths: &mut HashSet<&'a str>,
) -> Result<()> {
    validate_id(&source.id, "source")?;
    ensure!(
        source.repository.starts_with("https://")
            && !source
                .repository
                .bytes()
                .any(|byte| byte.is_ascii_whitespace()),
        "corpus source repository must be an HTTPS URL without whitespace"
    );
    ensure!(
        is_lower_hex(&source.revision, 40),
        "corpus source revision must be a full 40-character lowercase Git commit"
    );
    ensure!(
        SUPPORTED_LICENSES.contains(&source.license.as_str()),
        "corpus source license must be CC-BY-SA-4.0 or CC-BY-4.0"
    );
    ensure!(
        !source.artifacts.is_empty(),
        "corpus source `{}` must contain at least one artifact",
        source.id
    );

    for artifact in &source.artifacts {
        validate_artifact(artifact)?;
        ensure!(
            artifact_paths.insert(artifact.path.as_str()),
            "corpus source artifact paths must be unique"
        );
    }
    Ok(())
}

fn validate_artifact(artifact: &CorpusArtifact) -> Result<()> {
    validate_artifact_path(&artifact.path)?;
    ensure!(
        is_lower_hex_sha256(&artifact.sha256),
        "corpus artifact SHA-256 must contain exactly 64 lowercase hexadecimal characters"
    );
    ensure!(
        artifact.size > 0,
        "corpus artifact size must be greater than zero"
    );
    Ok(())
}

fn validate_selection<'a>(
    selection: &'a CorpusSelection,
    sources: &[CorpusSource],
    selection_ids: &mut HashSet<&'a str>,
    need_ids: &mut HashSet<&'a str>,
    group_splits: &mut HashMap<&'a str, CorpusSplit>,
) -> Result<()> {
    validate_id(&selection.id, "selection")?;
    validate_id(&selection.source_id, "selection source")?;
    validate_id(&selection.group_id, "selection group")?;
    validate_id(&selection.document_id, "selection document")?;
    validate_id(&selection.upstream_record_id, "upstream record")?;
    validate_id(&selection.need_id, "selection need")?;
    ensure!(
        selection_ids.insert(selection.id.as_str()),
        "corpus manifest contains duplicate selection id `{}`",
        selection.id
    );
    ensure!(
        need_ids.insert(selection.need_id.as_str()),
        "corpus manifest contains duplicate need id `{}`",
        selection.need_id
    );
    let source = sources
        .iter()
        .find(|source| source.id == selection.source_id)
        .context("corpus selection references an unknown source id")?;
    validate_source_artifact_locator(
        source,
        &selection.artifact_path,
        selection.member_path.as_deref(),
        "selection",
    )?;

    if let Some(existing_split) = group_splits.insert(selection.group_id.as_str(), selection.split)
    {
        ensure!(
            existing_split == selection.split,
            "corpus group cannot cross training and calibration splits"
        );
    }

    let mut support_ids = HashSet::with_capacity(selection.supports.len());
    for support in &selection.supports {
        validate_passage_ref(source, support, "support")?;
        ensure!(
            support_ids.insert(passage_key(support)),
            "corpus selection contains duplicate support passages"
        );
    }

    ensure!(
        !selection.hard_negatives.is_empty(),
        "corpus selection must contain at least one hard negative"
    );
    validate_hard_negatives(source, &selection.hard_negatives)?;
    ensure!(
        selection
            .hard_negatives
            .iter()
            .all(|negative| !support_ids.contains(&hard_negative_key(negative))),
        "corpus selection cannot use the same passage as support and hard negative"
    );

    match selection.expectation {
        CorpusExpectation::Answerable => ensure!(
            !selection.supports.is_empty(),
            "answerable corpus selection must contain at least one support"
        ),
        CorpusExpectation::Unanswerable => ensure!(
            selection.supports.is_empty(),
            "unanswerable corpus selection cannot contain supports"
        ),
    }
    Ok(())
}

fn validate_hard_negatives(source: &CorpusSource, hard_negatives: &[HardNegative]) -> Result<()> {
    let mut ids = HashSet::with_capacity(hard_negatives.len());
    for hard_negative in hard_negatives {
        validate_source_artifact_locator(
            source,
            &hard_negative.artifact_path,
            hard_negative.member_path.as_deref(),
            "hard negative",
        )?;
        validate_id(&hard_negative.document_id, "hard-negative document")?;
        validate_id(&hard_negative.segment_id, "hard-negative segment")?;
        ensure!(
            ids.insert(hard_negative_key(hard_negative)),
            "corpus selection contains duplicate hard-negative passages"
        );
    }
    Ok(())
}

fn validate_passage_ref(source: &CorpusSource, passage: &PassageRef, kind: &str) -> Result<()> {
    validate_source_artifact_locator(
        source,
        &passage.artifact_path,
        passage.member_path.as_deref(),
        kind,
    )?;
    validate_id(&passage.document_id, &format!("{kind} document"))?;
    validate_id(&passage.segment_id, &format!("{kind} segment"))?;
    Ok(())
}

fn validate_source_artifact_locator(
    source: &CorpusSource,
    artifact_path: &str,
    member_path: Option<&str>,
    kind: &str,
) -> Result<()> {
    validate_artifact_path(artifact_path)?;
    ensure!(
        source
            .artifacts
            .iter()
            .any(|artifact| artifact.path == artifact_path),
        "corpus {kind} references an artifact outside its source"
    );
    if artifact_path.ends_with(".zip") {
        let member_path = member_path
            .with_context(|| format!("corpus {kind} ZIP locator requires a member path"))?;
        validate_artifact_path(member_path)?;
    } else {
        ensure!(
            member_path.is_none(),
            "corpus {kind} cannot set a member path for a non-archive artifact"
        );
    }
    Ok(())
}

fn passage_key(passage: &PassageRef) -> (&str, Option<&str>, &str, &str) {
    (
        passage.artifact_path.as_str(),
        passage.member_path.as_deref(),
        passage.document_id.as_str(),
        passage.segment_id.as_str(),
    )
}

fn hard_negative_key(negative: &HardNegative) -> (&str, Option<&str>, &str, &str) {
    (
        negative.artifact_path.as_str(),
        negative.member_path.as_deref(),
        negative.document_id.as_str(),
        negative.segment_id.as_str(),
    )
}

fn validate_id(value: &str, kind: &str) -> Result<()> {
    let Some(first) = value.bytes().next() else {
        anyhow::bail!("corpus {kind} id must not be empty");
    };
    ensure!(
        first.is_ascii_lowercase() || first.is_ascii_digit(),
        "corpus {kind} id must start with lowercase ASCII or a digit"
    );
    ensure!(
        value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        }),
        "corpus {kind} id must use lowercase ASCII, digits, underscores or hyphens"
    );
    Ok(())
}

fn is_lower_hex_sha256(value: &str) -> bool {
    is_lower_hex(value, 64)
}

fn is_lower_hex(value: &str, expected_length: usize) -> bool {
    value.len() == expected_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn validate_artifact_path(value: &str) -> Result<()> {
    ensure!(!value.is_empty(), "corpus artifact path must not be empty");
    ensure!(
        !value.contains('\\'),
        "corpus artifact path must use forward slashes"
    );
    ensure!(
        !value.starts_with('/'),
        "corpus artifact path must be relative"
    );
    let bytes = value.as_bytes();
    ensure!(
        !(bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'),
        "corpus artifact path must not use a Windows drive prefix"
    );
    ensure!(
        value
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | "..")),
        "corpus artifact path must be normalized and cannot traverse directories"
    );
    ensure!(
        Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_))),
        "corpus artifact path must contain only normal relative components"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde_json::{Value, json};
    use tempfile::NamedTempFile;

    use super::*;

    fn valid_manifest() -> Value {
        json!({
            "schema_version": 1,
            "corpus_id": "airwiki_dev_v1",
            "sources": [
                {
                    "id": "squad2",
                    "repository": "https://example.invalid/squad2",
                    "revision": "1".repeat(40),
                    "license": "CC-BY-SA-4.0",
                    "artifacts": [{
                        "path": "sources/squad2/train.json",
                        "sha256": "a".repeat(64),
                        "size": 42
                    }]
                },
                {
                    "id": "contractnli",
                    "repository": "https://example.invalid/contractnli",
                    "revision": "2".repeat(40),
                    "license": "CC-BY-4.0",
                    "artifacts": [{
                        "path": "sources/contractnli/train.jsonl",
                        "sha256": "b".repeat(64),
                        "size": 84
                    }]
                }
            ],
            "selections": [
                {
                    "id": "selection_answerable",
                    "source_id": "squad2",
                    "artifact_path": "sources/squad2/train.json",
                    "split": "training",
                    "group_id": "document_one",
                    "document_id": "document_one",
                    "upstream_record_id": "record_one",
                    "need_id": "need_one",
                    "expectation": "answerable",
                    "supports": [{
                        "artifact_path": "sources/squad2/train.json",
                        "document_id": "document_one",
                        "segment_id": "passage_one"
                    }],
                    "hard_negatives": [{
                        "artifact_path": "sources/squad2/train.json",
                        "document_id": "document_one",
                        "segment_id": "passage_wrong_subject",
                        "kind": "wrong_subject"
                    }]
                },
                {
                    "id": "selection_unanswerable",
                    "source_id": "contractnli",
                    "artifact_path": "sources/contractnli/train.jsonl",
                    "split": "calibration",
                    "group_id": "document_two",
                    "document_id": "document_two",
                    "upstream_record_id": "record_two",
                    "need_id": "need_two",
                    "expectation": "unanswerable",
                    "supports": [],
                    "hard_negatives": [{
                        "artifact_path": "sources/contractnli/train.jsonl",
                        "document_id": "document_two",
                        "segment_id": "passage_related",
                        "kind": "related_but_unanswered"
                    }]
                }
            ]
        })
    }

    fn validate_json(value: &Value) -> Result<CorpusManifestSummary> {
        let mut file = NamedTempFile::new().context("failed to create manifest fixture")?;
        serde_json::to_writer(file.as_file_mut(), value)
            .context("failed to write manifest fixture")?;
        file.flush().context("failed to flush manifest fixture")?;
        validate_manifest(file.path())
    }

    fn validation_error(value: &Value) -> String {
        format!("{:#}", validate_json(value).unwrap_err())
    }

    #[test]
    fn valid_manifest_returns_content_free_counts() {
        let summary = validate_json(&valid_manifest()).unwrap();

        assert_eq!(
            summary,
            CorpusManifestSummary {
                source_count: 2,
                artifact_count: 2,
                selection_count: 2,
                answerable_count: 1,
                unanswerable_count: 1,
                group_count: 2,
            }
        );
    }

    #[test]
    fn unknown_field_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["unexpected"] = json!(true);

        assert!(validation_error(&manifest).contains("unknown field"));
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["schema_version"] = json!(2);

        assert!(validation_error(&manifest).contains("schema_version must be 1"));
    }

    #[test]
    fn empty_sources_are_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"] = json!([]);

        assert!(validation_error(&manifest).contains("at least one source"));
    }

    #[test]
    fn empty_selections_are_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"] = json!([]);

        assert!(validation_error(&manifest).contains("at least one selection"));
    }

    #[test]
    fn uppercase_identifier_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["corpus_id"] = json!("AirWiki");

        assert!(validation_error(&manifest).contains("lowercase ASCII"));
    }

    #[test]
    fn duplicate_source_id_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"][1]["id"] = json!("squad2");

        assert!(validation_error(&manifest).contains("duplicate source id"));
    }

    #[test]
    fn duplicate_selection_id_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][1]["id"] = json!("selection_answerable");

        assert!(validation_error(&manifest).contains("duplicate selection id"));
    }

    #[test]
    fn duplicate_need_id_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][1]["need_id"] = json!("need_one");

        assert!(validation_error(&manifest).contains("duplicate need id"));
    }

    #[test]
    fn duplicate_upstream_record_locator_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][1]["source_id"] = json!("squad2");
        manifest["selections"][1]["artifact_path"] = json!("sources/squad2/train.json");
        manifest["selections"][1]["document_id"] = json!("document_one");
        manifest["selections"][1]["upstream_record_id"] = json!("record_one");
        manifest["selections"][1]["hard_negatives"][0]["artifact_path"] =
            json!("sources/squad2/train.json");

        assert!(validation_error(&manifest).contains("duplicate upstream record locator"));
    }

    #[test]
    fn unknown_source_reference_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][0]["source_id"] = json!("missing_source");

        assert!(validation_error(&manifest).contains("unknown source id"));
    }

    #[test]
    fn unreferenced_source_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][1]["source_id"] = json!("squad2");
        manifest["selections"][1]["artifact_path"] = json!("sources/squad2/train.json");
        manifest["selections"][1]["hard_negatives"][0]["artifact_path"] =
            json!("sources/squad2/train.json");

        assert!(validation_error(&manifest).contains("every corpus source"));
    }

    #[test]
    fn empty_source_metadata_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"][0]["repository"] = json!("  ");

        assert!(validation_error(&manifest).contains("HTTPS URL"));
    }

    #[test]
    fn abbreviated_source_revision_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"][0]["revision"] = json!("deadbeef");

        assert!(validation_error(&manifest).contains("40-character"));
    }

    #[test]
    fn unsupported_license_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"][0]["license"] = json!("MIT");

        assert!(validation_error(&manifest).contains("CC-BY-SA-4.0 or CC-BY-4.0"));
    }

    #[test]
    fn source_without_artifacts_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"][0]["artifacts"] = json!([]);

        assert!(validation_error(&manifest).contains("at least one artifact"));
    }

    #[test]
    fn uppercase_or_short_sha256_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"][0]["artifacts"][0]["sha256"] = json!("A".repeat(64));

        assert!(validation_error(&manifest).contains("lowercase hexadecimal"));
    }

    #[test]
    fn zero_size_artifact_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"][0]["artifacts"][0]["size"] = json!(0);

        assert!(validation_error(&manifest).contains("greater than zero"));
    }

    #[test]
    fn absolute_artifact_path_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"][0]["artifacts"][0]["path"] = json!("/tmp/train.json");

        assert!(validation_error(&manifest).contains("must be relative"));
    }

    #[test]
    fn traversing_artifact_path_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"][0]["artifacts"][0]["path"] = json!("sources/../train.json");

        assert!(validation_error(&manifest).contains("cannot traverse"));
    }

    #[test]
    fn windows_artifact_path_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["sources"][0]["artifacts"][0]["path"] = json!("C:\\data\\train.json");

        assert!(validation_error(&manifest).contains("forward slashes"));
    }

    #[test]
    fn duplicate_artifact_path_is_rejected_across_sources() {
        let mut manifest = valid_manifest();
        let duplicate_path = manifest["sources"][0]["artifacts"][0]["path"].clone();
        manifest["sources"][1]["artifacts"][0]["path"] = duplicate_path;

        assert!(validation_error(&manifest).contains("paths must be unique"));
    }

    #[test]
    fn answerable_selection_without_support_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][0]["supports"] = json!([]);

        assert!(validation_error(&manifest).contains("at least one support"));
    }

    #[test]
    fn unanswerable_selection_with_support_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][1]["supports"] = json!([{
            "artifact_path": "sources/contractnli/train.jsonl",
            "document_id": "document_two",
            "segment_id": "unexpected_support"
        }]);

        assert!(validation_error(&manifest).contains("cannot contain supports"));
    }

    #[test]
    fn selection_without_hard_negative_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][0]["hard_negatives"] = json!([]);

        assert!(validation_error(&manifest).contains("at least one hard negative"));
    }

    #[test]
    fn duplicate_hard_negative_passage_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][0]["hard_negatives"] = json!([
            {
                "artifact_path": "sources/squad2/train.json",
                "document_id": "document_one",
                "segment_id": "same_passage",
                "kind": "wrong_subject"
            },
            {
                "artifact_path": "sources/squad2/train.json",
                "document_id": "document_one",
                "segment_id": "same_passage",
                "kind": "wrong_relation"
            }
        ]);

        assert!(validation_error(&manifest).contains("duplicate hard-negative passages"));
    }

    #[test]
    fn repeated_hard_negative_kinds_are_allowed() {
        let mut manifest = valid_manifest();
        manifest["selections"][0]["hard_negatives"] = json!([
            {
                "artifact_path": "sources/squad2/train.json",
                "document_id": "document_one",
                "segment_id": "passage_wrong_one",
                "kind": "wrong_subject"
            },
            {
                "artifact_path": "sources/squad2/train.json",
                "document_id": "document_one",
                "segment_id": "passage_wrong_two",
                "kind": "wrong_subject"
            }
        ]);

        validate_json(&manifest).unwrap();
    }

    #[test]
    fn support_cannot_also_be_a_hard_negative() {
        let mut manifest = valid_manifest();
        manifest["selections"][0]["hard_negatives"][0]["segment_id"] = json!("passage_one");

        assert!(validation_error(&manifest).contains("support and hard negative"));
    }

    #[test]
    fn group_crossing_training_and_calibration_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][1]["group_id"] = json!("document_one");

        assert!(validation_error(&manifest).contains("cannot cross"));
    }

    #[test]
    fn unknown_hard_negative_kind_is_rejected() {
        let mut manifest = valid_manifest();
        manifest["selections"][0]["hard_negatives"][0]["kind"] = json!("plausible");

        assert!(validation_error(&manifest).contains("unknown variant"));
    }

    #[test]
    fn selection_artifact_must_belong_to_its_source() {
        let mut manifest = valid_manifest();
        manifest["selections"][0]["artifact_path"] = json!("sources/contractnli/train.jsonl");

        assert!(validation_error(&manifest).contains("outside its source"));
    }

    #[test]
    fn support_artifact_must_belong_to_its_source() {
        let mut manifest = valid_manifest();
        manifest["selections"][0]["supports"][0]["artifact_path"] =
            json!("sources/contractnli/train.jsonl");

        assert!(validation_error(&manifest).contains("outside its source"));
    }

    #[test]
    fn zip_selection_requires_a_member_path() {
        let mut manifest = valid_manifest();
        manifest["sources"][0]["artifacts"][0]["path"] = json!("sources/squad2/data.zip");
        manifest["selections"][0]["artifact_path"] = json!("sources/squad2/data.zip");
        manifest["selections"][0]["supports"][0]["artifact_path"] =
            json!("sources/squad2/data.zip");
        manifest["selections"][0]["hard_negatives"][0]["artifact_path"] =
            json!("sources/squad2/data.zip");

        assert!(validation_error(&manifest).contains("ZIP locator requires a member path"));
    }

    #[test]
    fn non_archive_selection_rejects_a_member_path() {
        let mut manifest = valid_manifest();
        manifest["selections"][0]["member_path"] = json!("inner/train.json");

        assert!(validation_error(&manifest).contains("non-archive artifact"));
    }

    #[test]
    fn both_development_splits_are_required() {
        let mut manifest = valid_manifest();
        manifest["selections"][1]["split"] = json!("training");

        assert!(validation_error(&manifest).contains("both training and calibration"));
    }

    #[test]
    fn both_answerability_classes_are_required() {
        let mut manifest = valid_manifest();
        manifest["selections"][1]["expectation"] = json!("answerable");
        manifest["selections"][1]["supports"] = json!([{
            "artifact_path": "sources/contractnli/train.jsonl",
            "document_id": "document_two",
            "segment_id": "passage_two"
        }]);

        assert!(validation_error(&manifest).contains("both answerable and unanswerable"));
    }
}
