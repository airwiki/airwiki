//! Pinned, local evidence reranking for the hybrid search pipeline.
//!
//! The runtime is deliberately feature-gated: offline tests exercise the
//! score policy and output validation without loading or downloading a model.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use fastembed::{
    RerankInitOptionsUserDefined, RerankResult, TextRerank, TokenizerFiles,
    UserDefinedRerankingModel,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(feature = "retrieval-evaluation")]
use crate::search::RetrievalEvaluationRelevance;
use crate::{EvidenceDecision, EvidenceRelevanceError, EvidenceRelevanceProvider, RelevanceInput};

/// Hugging Face repository containing the selected multilingual cross-encoder.
pub const MMARCO_RERANKER_REPOSITORY: &str = "cross-encoder/mmarco-mMiniLMv2-L12-H384-v1";
/// Immutable model revision accepted by [`PinnedMmarcoRerankerSnapshot`].
pub const MMARCO_RERANKER_REVISION: &str = "1427fd652930e4ba29e8149678df786c240d8825";
/// Stable identity for both the model snapshot and its initial evidence policy.
pub const MMARCO_RERANKER_PROFILE_ID: &str =
    "mmarco-mMiniLMv2-L12-H384-v1@1427fd652930e4ba29e8149678df786c240d8825/evidence-v1";

const MAX_SEQUENCE_LENGTH: usize = 512;
const INFERENCE_BATCH_SIZE: usize = 8;
const MAX_DISTANCE_FROM_BEST: f32 = 3.6;
const RELEVANCE_DEADLINE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SnapshotAsset {
    relative_path: &'static str,
    sha256: &'static str,
}

// SECURITY: these hashes intentionally duplicate the download manifest in
// airwiki-inference. airwiki-core is the last trust boundary before ONNX Runtime,
// and cannot depend on the outer inference crate without reversing the
// workspace dependency direction. Both manifests are covered by exact-value
// tests so a model update must change them together.
const COMMON_SNAPSHOT_ASSETS: [SnapshotAsset; 4] = [
    SnapshotAsset {
        relative_path: "tokenizer.json",
        sha256: "62c24cdc13d4c9952d63718d6c9fa4c287974249e16b7ade6d5a85e7bbb75626",
    },
    SnapshotAsset {
        relative_path: "config.json",
        sha256: "cc2cfe51aa3fd759d21d21acf5dfd6994aa67a3c9210636d22e143699d336c77",
    },
    SnapshotAsset {
        relative_path: "special_tokens_map.json",
        sha256: "378eb3bf733eb16e65792d7e3fda5b8a4631387ca04d2015199c4d4f22ae554d",
    },
    SnapshotAsset {
        relative_path: "tokenizer_config.json",
        sha256: "e7fbfbfa6347b4e414c1cee50d142e2c2f9a895dad68b068ae83a8b564c3837e",
    },
];

/// Sanitized failures while loading the pinned mMARCO profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MmarcoRerankerLoadError {
    #[error("the pinned mMARCO reranker does not support this platform")]
    UnsupportedPlatform,
    #[error("pinned mMARCO reranker snapshot directory is unavailable")]
    SnapshotUnavailable,
    #[error("pinned mMARCO reranker revision marker is unavailable")]
    RevisionMarkerUnavailable,
    #[error("pinned mMARCO reranker revision does not match the required snapshot")]
    RevisionMismatch,
    #[error("pinned mMARCO reranker asset is unavailable: {asset}")]
    AssetUnavailable { asset: &'static str },
    #[error("pinned mMARCO reranker asset is not a regular file: {asset}")]
    AssetNotRegular { asset: &'static str },
    #[error("pinned mMARCO reranker asset failed integrity verification: {asset}")]
    IntegrityMismatch { asset: &'static str },
    #[error("mMARCO reranker intra_threads must be positive")]
    InvalidThreadBudget,
    #[error("could not initialize the pinned mMARCO reranker")]
    InitializationFailed,
}

/// A fully materialized mMARCO snapshot whose revision and platform model are
/// validated before ONNX Runtime can see it.
pub struct PinnedMmarcoRerankerSnapshot {
    root: PathBuf,
    tokenizer_file: Vec<u8>,
    config_file: Vec<u8>,
    special_tokens_map_file: Vec<u8>,
    tokenizer_config_file: Vec<u8>,
    onnx_file: Vec<u8>,
    onnx_relative_path: &'static str,
}

impl std::fmt::Debug for PinnedMmarcoRerankerSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PinnedMmarcoRerankerSnapshot")
            .field("revision", &MMARCO_RERANKER_REVISION)
            .field("onnx_relative_path", &self.onnx_relative_path)
            .finish_non_exhaustive()
    }
}

impl PinnedMmarcoRerankerSnapshot {
    /// Opens a local snapshot without fetching missing assets.
    ///
    /// # Errors
    ///
    /// Returns a sanitized error when the platform is unsupported, the
    /// revision marker differs, or a required asset is absent or fails its
    /// pinned SHA-256 verification.
    pub fn open(root: impl AsRef<Path>) -> std::result::Result<Self, MmarcoRerankerLoadError> {
        let onnx_asset = platform_onnx_asset()?;
        Self::open_with_manifest(root.as_ref(), &COMMON_SNAPSHOT_ASSETS, onnx_asset)
    }

    fn open_with_manifest(
        root: &Path,
        common_assets: &[SnapshotAsset; 4],
        onnx_asset: SnapshotAsset,
    ) -> std::result::Result<Self, MmarcoRerankerLoadError> {
        let root = root.to_path_buf();
        let root_metadata = std::fs::symlink_metadata(&root)
            .map_err(|_| MmarcoRerankerLoadError::SnapshotUnavailable)?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(MmarcoRerankerLoadError::SnapshotUnavailable);
        }

        let revision_path = root.join("revision.txt");
        let revision_metadata = std::fs::symlink_metadata(&revision_path)
            .map_err(|_| MmarcoRerankerLoadError::RevisionMarkerUnavailable)?;
        if revision_metadata.file_type().is_symlink() || !revision_metadata.is_file() {
            return Err(MmarcoRerankerLoadError::RevisionMarkerUnavailable);
        }
        let revision = std::fs::read(revision_path)
            .map_err(|_| MmarcoRerankerLoadError::RevisionMarkerUnavailable)?;
        let expected_revision = format!("{MMARCO_RERANKER_REVISION}\n");
        if revision != expected_revision.as_bytes() {
            return Err(MmarcoRerankerLoadError::RevisionMismatch);
        }

        let [
            tokenizer_asset,
            config_asset,
            special_tokens_asset,
            tokenizer_config_asset,
        ] = *common_assets;
        let tokenizer_file = read_verified_asset(&root, tokenizer_asset)?;
        let config_file = read_verified_asset(&root, config_asset)?;
        let special_tokens_map_file = read_verified_asset(&root, special_tokens_asset)?;
        let tokenizer_config_file = read_verified_asset(&root, tokenizer_config_asset)?;
        let onnx_file = read_verified_asset(&root, onnx_asset)?;

        Ok(Self {
            root,
            tokenizer_file,
            config_file,
            special_tokens_map_file,
            tokenizer_config_file,
            onnx_file,
            onnx_relative_path: onnx_asset.relative_path,
        })
    }

    /// Returns the validated snapshot root.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn read_verified_asset(
    root: &Path,
    asset: SnapshotAsset,
) -> std::result::Result<Vec<u8>, MmarcoRerankerLoadError> {
    let path = root.join(asset.relative_path);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| {
        MmarcoRerankerLoadError::AssetUnavailable {
            asset: asset.relative_path,
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(MmarcoRerankerLoadError::AssetNotRegular {
            asset: asset.relative_path,
        });
    }
    let bytes = std::fs::read(path).map_err(|_| MmarcoRerankerLoadError::AssetUnavailable {
        asset: asset.relative_path,
    })?;
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != asset.sha256 {
        return Err(MmarcoRerankerLoadError::IntegrityMismatch {
            asset: asset.relative_path,
        });
    }
    Ok(bytes)
}

/// FastEmbed-backed mMARCO relevance provider.
///
/// Calls are serialized because ONNX sessions require mutable access. The
/// mutex is acquired only inside Tokio's blocking pool and is never held over
/// an async suspension point.
#[derive(Clone)]
pub struct FastEmbedMmarcoReranker {
    model: Arc<Mutex<TextRerank>>,
    inference_permit: Arc<tokio::sync::Semaphore>,
}

impl std::fmt::Debug for FastEmbedMmarcoReranker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FastEmbedMmarcoReranker")
            .field("profile_id", &MMARCO_RERANKER_PROFILE_ID)
            .finish_non_exhaustive()
    }
}

impl FastEmbedMmarcoReranker {
    /// Initializes the cross-encoder from a validated local snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero thread budget, unreadable assets, or an
    /// invalid ONNX/tokenizer snapshot. Underlying paths and runtime details
    /// are deliberately not exposed.
    pub fn from_snapshot(
        snapshot: PinnedMmarcoRerankerSnapshot,
        intra_threads: usize,
    ) -> std::result::Result<Self, MmarcoRerankerLoadError> {
        if intra_threads == 0 {
            return Err(MmarcoRerankerLoadError::InvalidThreadBudget);
        }

        let tokenizer_files = TokenizerFiles {
            tokenizer_file: snapshot.tokenizer_file,
            config_file: snapshot.config_file,
            special_tokens_map_file: snapshot.special_tokens_map_file,
            tokenizer_config_file: snapshot.tokenizer_config_file,
        };
        let model = UserDefinedRerankingModel::new(snapshot.onnx_file, tokenizer_files);
        let options = RerankInitOptionsUserDefined::new()
            .with_max_length(MAX_SEQUENCE_LENGTH)
            .with_intra_threads(intra_threads);
        let model = TextRerank::try_new_from_user_defined(model, options)
            .map_err(|_| MmarcoRerankerLoadError::InitializationFailed)?;

        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            inference_permit: Arc::new(tokio::sync::Semaphore::new(1)),
        })
    }

    async fn score_candidates(
        &self,
        question: &str,
        candidates: &[RelevanceInput],
    ) -> std::result::Result<Vec<f32>, EvidenceRelevanceError> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let model = Arc::clone(&self.model);
        let inference_permit = Arc::clone(&self.inference_permit);
        let question = question.to_owned();
        let documents = candidates.iter().map(candidate_passage).collect::<Vec<_>>();
        let expected = documents.len();

        let reranking = async move {
            let permit = inference_permit
                .acquire_owned()
                .await
                .map_err(|_| EvidenceRelevanceError::Unavailable)?;
            run_blocking_rerank(move || {
                // A Tokio timeout cannot cancel a running blocking task. Keep
                // the owned permit in that task so a timed-out session still
                // serializes access until ONNX Runtime actually returns.
                let _permit = permit;
                let mut model = model
                    .lock()
                    .map_err(|_| EvidenceRelevanceError::Unavailable)?;
                model
                    .rerank(question, documents, false, Some(INFERENCE_BATCH_SIZE))
                    .map_err(|_| EvidenceRelevanceError::InferenceFailed)
            })
            .await
        };
        let results = run_with_relevance_deadline(RELEVANCE_DEADLINE, reranking).await?;

        reconstruct_scores(results, expected)
    }
}

#[async_trait]
impl EvidenceRelevanceProvider for FastEmbedMmarcoReranker {
    fn profile_id(&self) -> &str {
        MMARCO_RERANKER_PROFILE_ID
    }

    async fn classify(
        &self,
        question: &str,
        candidates: &[RelevanceInput],
    ) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
        let scores = self.score_candidates(question, candidates).await?;
        decisions_from_scores(&scores)
    }

    #[cfg(feature = "retrieval-evaluation")]
    async fn classify_and_order_for_evaluation(
        &self,
        question: &str,
        candidates: &[RelevanceInput],
    ) -> std::result::Result<RetrievalEvaluationRelevance, EvidenceRelevanceError> {
        let scores = self.score_candidates(question, candidates).await?;
        let decisions = decisions_from_scores(&scores)?;
        let ordering_started = std::time::Instant::now();
        let score_order = score_descending_order(&scores)?;
        RetrievalEvaluationRelevance::from_decisions_and_order(
            decisions,
            score_order,
            ordering_started.elapsed().as_micros(),
        )
    }
}

async fn run_with_relevance_deadline<T, F>(
    deadline: Duration,
    operation: F,
) -> std::result::Result<T, EvidenceRelevanceError>
where
    F: std::future::Future<Output = std::result::Result<T, EvidenceRelevanceError>>,
{
    match tokio::time::timeout(deadline, operation).await {
        Ok(result) => result,
        Err(_) => Err(EvidenceRelevanceError::TimedOut),
    }
}

async fn run_blocking_rerank<T, F>(operation: F) -> std::result::Result<T, EvidenceRelevanceError>
where
    T: Send + 'static,
    F: FnOnce() -> std::result::Result<T, EvidenceRelevanceError> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| EvidenceRelevanceError::InferenceFailed)?
}

fn candidate_passage(candidate: &RelevanceInput) -> String {
    [
        candidate.title.trim(),
        candidate.heading.trim(),
        candidate.text.trim(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn reconstruct_scores(
    results: Vec<RerankResult>,
    expected: usize,
) -> std::result::Result<Vec<f32>, EvidenceRelevanceError> {
    if results.len() != expected {
        return Err(EvidenceRelevanceError::InvalidOutput);
    }

    let mut scores = vec![None; expected];
    for result in results {
        if result.index >= expected || !result.score.is_finite() {
            return Err(EvidenceRelevanceError::InvalidOutput);
        }
        let slot = &mut scores[result.index];
        if slot.replace(result.score).is_some() {
            return Err(EvidenceRelevanceError::InvalidOutput);
        }
    }

    scores
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or(EvidenceRelevanceError::InvalidOutput)
}

/// Applies the initial profile policy without performing inference:
///
/// - a negative best score rejects the whole batch;
/// - otherwise, a passage must be non-negative and no more than 3.6 logits
///   below the best passage in the same candidate set.
fn decisions_from_scores(
    scores: &[f32],
) -> std::result::Result<Vec<EvidenceDecision>, EvidenceRelevanceError> {
    if scores.iter().any(|score| !score.is_finite()) {
        return Err(EvidenceRelevanceError::InvalidOutput);
    }
    let Some(best_score) = scores.iter().copied().reduce(f32::max) else {
        return Ok(Vec::new());
    };
    if best_score < 0.0 {
        return Ok(vec![EvidenceDecision::Irrelevant; scores.len()]);
    }

    let minimum_relative_score = best_score - MAX_DISTANCE_FROM_BEST;
    Ok(scores
        .iter()
        .map(|score| {
            if *score >= 0.0 && *score >= minimum_relative_score {
                EvidenceDecision::Relevant
            } else {
                EvidenceDecision::Irrelevant
            }
        })
        .collect())
}

#[cfg(feature = "retrieval-evaluation")]
fn score_descending_order(
    scores: &[f32],
) -> std::result::Result<Vec<usize>, EvidenceRelevanceError> {
    if scores.iter().any(|score| !score.is_finite()) {
        return Err(EvidenceRelevanceError::InvalidOutput);
    }
    let mut order = (0..scores.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        scores[*right]
            .total_cmp(&scores[*left])
            .then_with(|| left.cmp(right))
    });
    Ok(order)
}

fn platform_onnx_asset() -> std::result::Result<SnapshotAsset, MmarcoRerankerLoadError> {
    onnx_asset_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        avx2_available(),
    )
}

fn onnx_asset_for(
    target_os: &str,
    target_arch: &str,
    avx2: bool,
) -> std::result::Result<SnapshotAsset, MmarcoRerankerLoadError> {
    match (target_os, target_arch, avx2) {
        ("macos", "aarch64", _) => Ok(SnapshotAsset {
            relative_path: "onnx/model_qint8_arm64.onnx",
            sha256: "1825907d6c1a9001ff78124780bbde20a614a8c3df3b63409cf3c72c6fe5c8b4",
        }),
        ("windows", "x86_64", true) => Ok(SnapshotAsset {
            relative_path: "onnx/model_quint8_avx2.onnx",
            sha256: "6c2513767fb63d008a4377bef7a7a3555433d9436342bb53e35a3a72ffc52d4b",
        }),
        _ => Err(MmarcoRerankerLoadError::UnsupportedPlatform),
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn avx2_available() -> bool {
    std::arch::is_x86_feature_detected!("avx2")
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn avx2_available() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const FIXTURE_COMMON_ASSETS: [SnapshotAsset; 4] = [
        SnapshotAsset {
            relative_path: "tokenizer.json",
            sha256: EMPTY_SHA256,
        },
        SnapshotAsset {
            relative_path: "config.json",
            sha256: EMPTY_SHA256,
        },
        SnapshotAsset {
            relative_path: "special_tokens_map.json",
            sha256: EMPTY_SHA256,
        },
        SnapshotAsset {
            relative_path: "tokenizer_config.json",
            sha256: EMPTY_SHA256,
        },
    ];
    const FIXTURE_ONNX_ASSET: SnapshotAsset = SnapshotAsset {
        relative_path: "onnx/model.onnx",
        sha256: EMPTY_SHA256,
    };

    fn result(index: usize, score: f32) -> RerankResult {
        RerankResult {
            document: None,
            score,
            index,
        }
    }

    #[test]
    fn consumer_hashes_match_the_installer_manifest() {
        for asset in COMMON_SNAPSHOT_ASSETS {
            let installed = airwiki_inference::MMARCO_COMMON_FILES
                .iter()
                .find(|candidate| candidate.filename == asset.relative_path)
                .unwrap();
            assert_eq!(asset.sha256, installed.sha256);
            assert_eq!(installed.revision, MMARCO_RERANKER_REVISION);
        }

        match (
            platform_onnx_asset(),
            airwiki_inference::platform_relevance_model(),
        ) {
            (Ok(consumed), Some(installed)) => {
                assert_eq!(consumed.relative_path, installed.filename);
                assert_eq!(consumed.sha256, installed.sha256);
                assert_eq!(installed.revision, MMARCO_RERANKER_REVISION);
            }
            (Err(MmarcoRerankerLoadError::UnsupportedPlatform), None) => {}
            mismatch => panic!("installer and consumer platform manifests differ: {mismatch:?}"),
        }
    }

    #[test]
    fn platform_selector_rejects_windows_without_avx2() {
        assert_eq!(
            onnx_asset_for("windows", "x86_64", false),
            Err(MmarcoRerankerLoadError::UnsupportedPlatform)
        );
        assert_eq!(
            onnx_asset_for("windows", "x86_64", true)
                .unwrap()
                .relative_path,
            "onnx/model_quint8_avx2.onnx"
        );
        assert_eq!(
            onnx_asset_for("macos", "aarch64", false)
                .unwrap()
                .relative_path,
            "onnx/model_qint8_arm64.onnx"
        );
    }

    fn write_snapshot_fixture(root: &Path) {
        std::fs::create_dir(root.join("onnx")).unwrap();
        std::fs::write(
            root.join("revision.txt"),
            format!("{MMARCO_RERANKER_REVISION}\n"),
        )
        .unwrap();
        for asset in FIXTURE_COMMON_ASSETS {
            std::fs::write(root.join(asset.relative_path), []).unwrap();
        }
        std::fs::write(root.join(FIXTURE_ONNX_ASSET.relative_path), []).unwrap();
    }

    fn open_snapshot_fixture(
        root: &Path,
    ) -> std::result::Result<PinnedMmarcoRerankerSnapshot, MmarcoRerankerLoadError> {
        PinnedMmarcoRerankerSnapshot::open_with_manifest(
            root,
            &FIXTURE_COMMON_ASSETS,
            FIXTURE_ONNX_ASSET,
        )
    }

    #[test]
    fn score_policy_accepts_empty_input() {
        assert_eq!(decisions_from_scores(&[]).unwrap(), Vec::new());
    }

    #[test]
    fn score_policy_rejects_every_candidate_when_best_is_negative() {
        assert_eq!(
            decisions_from_scores(&[-0.01, -4.0, -1.5]).unwrap(),
            vec![EvidenceDecision::Irrelevant; 3]
        );
    }

    #[test]
    fn score_policy_accepts_zero_when_zero_is_best() {
        assert_eq!(
            decisions_from_scores(&[-0.1, -0.0]).unwrap(),
            vec![EvidenceDecision::Irrelevant, EvidenceDecision::Relevant]
        );
    }

    #[test]
    fn score_policy_applies_zero_floor_and_inclusive_relative_boundary() {
        let relative_boundary = 5.0_f32 - MAX_DISTANCE_FROM_BEST;
        assert_eq!(
            decisions_from_scores(&[5.0, relative_boundary, relative_boundary - 0.000_1, -0.1])
                .unwrap(),
            vec![
                EvidenceDecision::Relevant,
                EvidenceDecision::Relevant,
                EvidenceDecision::Irrelevant,
                EvidenceDecision::Irrelevant,
            ]
        );
    }

    #[test]
    fn score_policy_keeps_non_negative_candidates_when_best_is_below_delta() {
        assert_eq!(
            decisions_from_scores(&[2.0, 0.0, -0.001]).unwrap(),
            vec![
                EvidenceDecision::Relevant,
                EvidenceDecision::Relevant,
                EvidenceDecision::Irrelevant,
            ]
        );
    }

    #[test]
    fn score_policy_rejects_every_non_finite_value() {
        for scores in [
            vec![f32::NAN],
            vec![f32::INFINITY],
            vec![f32::NEG_INFINITY],
            vec![1.0, f32::NAN],
        ] {
            assert_eq!(
                decisions_from_scores(&scores),
                Err(EvidenceRelevanceError::InvalidOutput)
            );
        }
    }

    #[cfg(feature = "retrieval-evaluation")]
    #[test]
    fn score_order_sorts_descending_with_stable_input_ties() {
        assert_eq!(
            score_descending_order(&[0.5, 4.0, 4.0, -2.0]).unwrap(),
            vec![1, 2, 0, 3]
        );
    }

    #[test]
    fn rerank_results_are_restored_to_input_order() {
        assert_eq!(
            reconstruct_scores(vec![result(2, 7.0), result(0, 3.0), result(1, -1.0)], 3,).unwrap(),
            vec![3.0, -1.0, 7.0]
        );
    }

    #[test]
    fn rerank_results_reject_count_mismatch() {
        assert_eq!(
            reconstruct_scores(vec![result(0, 1.0)], 2),
            Err(EvidenceRelevanceError::InvalidOutput)
        );
    }

    #[test]
    fn rerank_results_reject_out_of_range_index() {
        assert_eq!(
            reconstruct_scores(vec![result(1, 1.0)], 1),
            Err(EvidenceRelevanceError::InvalidOutput)
        );
    }

    #[test]
    fn rerank_results_reject_duplicate_index() {
        assert_eq!(
            reconstruct_scores(vec![result(0, 1.0), result(0, 2.0)], 2),
            Err(EvidenceRelevanceError::InvalidOutput)
        );
    }

    #[test]
    fn rerank_results_reject_non_finite_scores() {
        assert_eq!(
            reconstruct_scores(vec![result(0, f32::NAN)], 1),
            Err(EvidenceRelevanceError::InvalidOutput)
        );
    }

    #[test]
    fn candidate_passage_omits_empty_metadata_lines() {
        let candidate = RelevanceInput {
            title: " Title ".to_owned(),
            heading: "  ".to_owned(),
            text: " Body ".to_owned(),
        };

        assert_eq!(candidate_passage(&candidate), "Title\nBody");
    }

    #[cfg(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64")
    ))]
    #[test]
    fn snapshot_requires_exact_revision_marker() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("revision.txt"), "wrong").unwrap();

        let error = PinnedMmarcoRerankerSnapshot::open(temp.path()).unwrap_err();

        assert_eq!(
            error.to_string(),
            "pinned mMARCO reranker revision does not match the required snapshot"
        );
    }

    #[cfg(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64")
    ))]
    #[test]
    fn snapshot_rejects_missing_assets_without_disclosing_root() {
        let temp = tempfile::tempdir().unwrap();
        write_snapshot_fixture(temp.path());
        std::fs::remove_file(temp.path().join("tokenizer.json")).unwrap();

        let error = PinnedMmarcoRerankerSnapshot::open(temp.path()).unwrap_err();

        assert_eq!(
            error,
            MmarcoRerankerLoadError::AssetUnavailable {
                asset: "tokenizer.json"
            }
        );
        assert!(
            !error
                .to_string()
                .contains(&temp.path().display().to_string())
        );
    }

    #[test]
    fn constructor_rejects_zero_threads_before_loading_assets() {
        let snapshot = PinnedMmarcoRerankerSnapshot {
            root: PathBuf::new(),
            tokenizer_file: Vec::new(),
            config_file: Vec::new(),
            special_tokens_map_file: Vec::new(),
            tokenizer_config_file: Vec::new(),
            onnx_file: Vec::new(),
            onnx_relative_path: "unused.onnx",
        };

        let error = FastEmbedMmarcoReranker::from_snapshot(snapshot, 0).unwrap_err();

        assert_eq!(
            error.to_string(),
            "mMARCO reranker intra_threads must be positive"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reranking_work_does_not_block_the_async_executor() {
        let started = std::time::Instant::now();
        let blocking = run_blocking_rerank(|| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            Ok(())
        });
        let heartbeat = async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            started.elapsed()
        };

        let (result, heartbeat_elapsed) = tokio::join!(blocking, heartbeat);

        result.unwrap();
        assert!(
            heartbeat_elapsed < std::time::Duration::from_millis(75),
            "blocking reranker work stalled Tokio for {heartbeat_elapsed:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deadline_fails_closed_and_blocking_work_retains_its_permit() {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
        let blocking = run_blocking_rerank(move || {
            let _permit = permit;
            std::thread::sleep(Duration::from_millis(75));
            Ok(())
        });

        let error = run_with_relevance_deadline(Duration::from_millis(5), blocking)
            .await
            .unwrap_err();

        assert_eq!(error, EvidenceRelevanceError::TimedOut);
        assert_eq!(semaphore.available_permits(), 0);
        let released = tokio::time::timeout(
            Duration::from_secs(2),
            Arc::clone(&semaphore).acquire_owned(),
        )
        .await
        .expect("blocking reranker did not release its permit")
        .unwrap();
        drop(released);
        assert_eq!(semaphore.available_permits(), 1);
    }

    #[test]
    fn snapshot_accepts_complete_platform_snapshot_without_loading_model() {
        let temp = tempfile::tempdir().unwrap();
        write_snapshot_fixture(temp.path());

        let snapshot = open_snapshot_fixture(temp.path()).unwrap();

        assert_eq!(snapshot.root(), temp.path());
        assert!(!format!("{snapshot:?}").contains(&temp.path().display().to_string()));
    }

    #[test]
    fn snapshot_rejects_tampered_bytes_after_revision_matches() {
        let temp = tempfile::tempdir().unwrap();
        write_snapshot_fixture(temp.path());
        std::fs::write(temp.path().join("tokenizer.json"), b"tampered").unwrap();

        assert_eq!(
            open_snapshot_fixture(temp.path()).unwrap_err(),
            MmarcoRerankerLoadError::IntegrityMismatch {
                asset: "tokenizer.json"
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_rejects_symlinked_assets() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        write_snapshot_fixture(temp.path());
        let tokenizer = temp.path().join("tokenizer.json");
        std::fs::remove_file(&tokenizer).unwrap();
        symlink(temp.path().join("config.json"), &tokenizer).unwrap();

        assert_eq!(
            open_snapshot_fixture(temp.path()).unwrap_err(),
            MmarcoRerankerLoadError::AssetNotRegular {
                asset: "tokenizer.json"
            }
        );
    }
}
