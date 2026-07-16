use std::{
    ffi::OsStr,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{fs, io::AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    catalog::{GenerationSettings, INSTALL_HEADROOM_BYTES, ModelSelection},
    diagnostic::available_space_for,
};

pub const LLAMA_CPP_BUILD: &str = "b9946";
pub const E5_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";
pub const MMARCO_REVISION: &str = "1427fd652930e4ba29e8149678df786c240d8825";
const APACHE_2_LICENSE_URL: &str = "https://www.apache.org/licenses/LICENSE-2.0.txt";
pub const MACOS_LLAMA_SERVER_SHA256: &str =
    "12df97ffa9d48545e96cd3237a71f78efd1cc0222f971cbd65f7ab57e793b128";
pub const WINDOWS_LLAMA_SERVER_SHA256: &str =
    match option_env!("AIRWIKI_WINDOWS_LLAMA_SERVER_SHA256") {
        Some(value) => value,
        // Normal contributor builds do not distribute a Windows runtime. The
        // Windows packaging scripts replace this fail-closed sentinel with the
        // SHA-256 of the source-built runtime before compiling the desktop.
        None => "0000000000000000000000000000000000000000000000000000000000000000",
    };

/// Returns whether a failed asset installation is safe to retry without user
/// action. Integrity, permission, capacity and compatibility failures remain
/// fail-closed; only temporary network and I/O conditions are retried.
pub fn install_failure_is_transient(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        if let Some(error) = cause.downcast_ref::<reqwest::Error>() {
            return error.is_connect()
                || error.is_timeout()
                || error.status().is_some_and(|status| {
                    status.is_server_error()
                        || status == StatusCode::REQUEST_TIMEOUT
                        || status == StatusCode::TOO_MANY_REQUESTS
                });
        }
        cause.downcast_ref::<std::io::Error>().is_some_and(|error| {
            matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::Interrupted
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::WouldBlock
            )
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Model,
    MultimodalProjector,
    EmbeddingAsset,
    RelevanceAsset,
    RuntimeArchive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: &'static str,
    pub kind: ArtifactKind,
    pub filename: &'static str,
    pub revision: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub approximate_bytes: u64,
    pub license: &'static str,
    pub license_url: &'static str,
}

pub const QWEN_MODEL: Artifact = Artifact {
    id: "qwen3-1.7b-q8",
    kind: ArtifactKind::Model,
    filename: "Qwen3-1.7B-Q8_0.gguf",
    revision: "90862c4b9d2787eaed51d12237eafdfe7c5f6077",
    url: "https://huggingface.co/Qwen/Qwen3-1.7B-GGUF/resolve/90862c4b9d2787eaed51d12237eafdfe7c5f6077/Qwen3-1.7B-Q8_0.gguf?download=true",
    sha256: "061b54daade076b5d3362dac252678d17da8c68f07560be70818cace6590cb1a",
    approximate_bytes: 1_834_426_016,
    license: "Apache-2.0",
    license_url: "https://huggingface.co/Qwen/Qwen3-1.7B-GGUF/blob/90862c4b9d2787eaed51d12237eafdfe7c5f6077/LICENSE",
};

pub const E5_FILES: [Artifact; 5] = [
    Artifact {
        id: "multilingual-e5-small-onnx",
        kind: ArtifactKind::EmbeddingAsset,
        filename: "onnx/model.onnx",
        revision: E5_REVISION,
        url: "https://huggingface.co/intfloat/multilingual-e5-small/resolve/614241f622f53c4eeff9890bdc4f31cfecc418b3/onnx/model.onnx?download=true",
        sha256: "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665",
        approximate_bytes: 470_268_510,
        license: "MIT",
        license_url: "https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/LICENSE",
    },
    Artifact {
        id: "multilingual-e5-small-tokenizer",
        kind: ArtifactKind::EmbeddingAsset,
        filename: "tokenizer.json",
        revision: E5_REVISION,
        url: "https://huggingface.co/intfloat/multilingual-e5-small/resolve/614241f622f53c4eeff9890bdc4f31cfecc418b3/tokenizer.json?download=true",
        sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
        approximate_bytes: 17_082_730,
        license: "MIT",
        license_url: "https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/LICENSE",
    },
    Artifact {
        id: "multilingual-e5-small-config",
        kind: ArtifactKind::EmbeddingAsset,
        filename: "config.json",
        revision: E5_REVISION,
        url: "https://huggingface.co/intfloat/multilingual-e5-small/resolve/614241f622f53c4eeff9890bdc4f31cfecc418b3/config.json?download=true",
        sha256: "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959",
        approximate_bytes: 655,
        license: "MIT",
        license_url: "https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/LICENSE",
    },
    Artifact {
        id: "multilingual-e5-small-special-tokens",
        kind: ArtifactKind::EmbeddingAsset,
        filename: "special_tokens_map.json",
        revision: E5_REVISION,
        url: "https://huggingface.co/intfloat/multilingual-e5-small/resolve/614241f622f53c4eeff9890bdc4f31cfecc418b3/special_tokens_map.json?download=true",
        sha256: "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7",
        approximate_bytes: 167,
        license: "MIT",
        license_url: "https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/LICENSE",
    },
    Artifact {
        id: "multilingual-e5-small-tokenizer-config",
        kind: ArtifactKind::EmbeddingAsset,
        filename: "tokenizer_config.json",
        revision: E5_REVISION,
        url: "https://huggingface.co/intfloat/multilingual-e5-small/resolve/614241f622f53c4eeff9890bdc4f31cfecc418b3/tokenizer_config.json?download=true",
        sha256: "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
        approximate_bytes: 443,
        license: "MIT",
        license_url: "https://huggingface.co/intfloat/multilingual-e5-small/blob/614241f622f53c4eeff9890bdc4f31cfecc418b3/LICENSE",
    },
];

pub const MMARCO_COMMON_FILES: [Artifact; 4] = [
    Artifact {
        id: "mmarco-minilm-config",
        kind: ArtifactKind::RelevanceAsset,
        filename: "config.json",
        revision: MMARCO_REVISION,
        url: "https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/resolve/1427fd652930e4ba29e8149678df786c240d8825/config.json?download=true",
        sha256: "cc2cfe51aa3fd759d21d21acf5dfd6994aa67a3c9210636d22e143699d336c77",
        approximate_bytes: 891,
        license: "Apache-2.0",
        license_url: APACHE_2_LICENSE_URL,
    },
    Artifact {
        id: "mmarco-minilm-special-tokens",
        kind: ArtifactKind::RelevanceAsset,
        filename: "special_tokens_map.json",
        revision: MMARCO_REVISION,
        url: "https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/resolve/1427fd652930e4ba29e8149678df786c240d8825/special_tokens_map.json?download=true",
        sha256: "378eb3bf733eb16e65792d7e3fda5b8a4631387ca04d2015199c4d4f22ae554d",
        approximate_bytes: 239,
        license: "Apache-2.0",
        license_url: APACHE_2_LICENSE_URL,
    },
    Artifact {
        id: "mmarco-minilm-tokenizer",
        kind: ArtifactKind::RelevanceAsset,
        filename: "tokenizer.json",
        revision: MMARCO_REVISION,
        url: "https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/resolve/1427fd652930e4ba29e8149678df786c240d8825/tokenizer.json?download=true",
        sha256: "62c24cdc13d4c9952d63718d6c9fa4c287974249e16b7ade6d5a85e7bbb75626",
        approximate_bytes: 17_082_660,
        license: "Apache-2.0",
        license_url: APACHE_2_LICENSE_URL,
    },
    Artifact {
        id: "mmarco-minilm-tokenizer-config",
        kind: ArtifactKind::RelevanceAsset,
        filename: "tokenizer_config.json",
        revision: MMARCO_REVISION,
        url: "https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/resolve/1427fd652930e4ba29e8149678df786c240d8825/tokenizer_config.json?download=true",
        sha256: "e7fbfbfa6347b4e414c1cee50d142e2c2f9a895dad68b068ae83a8b564c3837e",
        approximate_bytes: 435,
        license: "Apache-2.0",
        license_url: APACHE_2_LICENSE_URL,
    },
];

pub const MACOS_MMARCO_MODEL: Artifact = Artifact {
    id: "mmarco-minilm-onnx-macos-arm64",
    kind: ArtifactKind::RelevanceAsset,
    filename: "onnx/model_qint8_arm64.onnx",
    revision: MMARCO_REVISION,
    url: "https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/resolve/1427fd652930e4ba29e8149678df786c240d8825/onnx/model_qint8_arm64.onnx?download=true",
    sha256: "1825907d6c1a9001ff78124780bbde20a614a8c3df3b63409cf3c72c6fe5c8b4",
    approximate_bytes: 118_620_017,
    license: "Apache-2.0",
    license_url: APACHE_2_LICENSE_URL,
};

pub const WINDOWS_MMARCO_MODEL: Artifact = Artifact {
    id: "mmarco-minilm-onnx-windows-avx2",
    kind: ArtifactKind::RelevanceAsset,
    filename: "onnx/model_quint8_avx2.onnx",
    revision: MMARCO_REVISION,
    url: "https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/resolve/1427fd652930e4ba29e8149678df786c240d8825/onnx/model_quint8_avx2.onnx?download=true",
    sha256: "6c2513767fb63d008a4377bef7a7a3555433d9436342bb53e35a3a72ffc52d4b",
    approximate_bytes: 118_620_016,
    license: "Apache-2.0",
    license_url: APACHE_2_LICENSE_URL,
};

fn relevance_model_for(target_os: &str, target_arch: &str) -> Option<&'static Artifact> {
    match (target_os, target_arch) {
        ("macos", "aarch64") => Some(&MACOS_MMARCO_MODEL),
        ("windows", "x86_64") => Some(&WINDOWS_MMARCO_MODEL),
        _ => None,
    }
}

pub fn platform_relevance_model() -> Option<&'static Artifact> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    if !std::is_x86_feature_detected!("avx2") {
        return None;
    }
    relevance_model_for(std::env::consts::OS, std::env::consts::ARCH)
}

fn platform_relevance_artifacts() -> Result<Vec<&'static Artifact>> {
    let model = platform_relevance_model().ok_or_else(|| {
        anyhow!(
            "no relevance model is defined for {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let mut artifacts = Vec::with_capacity(MMARCO_COMMON_FILES.len() + 1);
    artifacts.extend(MMARCO_COMMON_FILES.iter());
    artifacts.push(model);
    Ok(artifacts)
}

pub const MACOS_LLAMA: Artifact = Artifact {
    id: "llama.cpp-macos-arm64",
    kind: ArtifactKind::RuntimeArchive,
    filename: "llama-b9946-bin-macos-arm64.tar.gz",
    revision: LLAMA_CPP_BUILD,
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b9946/llama-b9946-bin-macos-arm64.tar.gz",
    sha256: "d51d0ab59f0f44282c532449bb1d0098367e3b9429d20b8d7e7ab270eaa2393f",
    approximate_bytes: 10_721_535,
    license: "MIT",
    license_url: "https://github.com/ggml-org/llama.cpp/blob/b9946/LICENSE",
};

pub fn platform_runtime() -> Option<&'static Artifact> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(&MACOS_LLAMA)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
struct RuntimeBinary {
    filename: &'static str,
    extracted_relative_path: &'static str,
    sha256: &'static str,
}

fn platform_runtime_binary() -> Option<RuntimeBinary> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(RuntimeBinary {
            filename: "llama-server",
            extracted_relative_path: "llama-b9946/llama-server",
            sha256: MACOS_LLAMA_SERVER_SHA256,
        })
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some(RuntimeBinary {
            filename: "llama-server.exe",
            extracted_relative_path: "llama-server.exe",
            sha256: WINDOWS_LLAMA_SERVER_SHA256,
        })
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub enum InstallEvent {
    Started {
        artifact: String,
        total_bytes: u64,
    },
    Progress {
        artifact: String,
        downloaded: u64,
        total_bytes: u64,
    },
    Verifying {
        artifact: String,
    },
    Extracting {
        artifact: String,
    },
    Complete {
        artifact: String,
    },
}

#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub selection: ModelSelection,
    pub model_path: PathBuf,
    pub generation_settings: GenerationSettings,
    pub embedding_snapshot_path: PathBuf,
    pub relevance_snapshot_path: PathBuf,
    pub llama_server_path: PathBuf,
}

impl InstallOutcome {
    /// Reuses the result of a completed integrity verification for presentation.
    ///
    /// The returned plan never authorizes loading an artifact: model activation
    /// still consumes the verified paths in this outcome, and every future
    /// installation recomputes its plan immediately before writing. Keeping this
    /// conversion filesystem-free prevents the desktop from hashing the same
    /// multi-gigabyte assets again merely to render their installed state.
    pub fn verified_install_plan(&self) -> InstallPlan {
        InstallPlan {
            selection: self.selection.clone(),
            artifact_ids: Vec::new(),
            download_bytes: 0,
            required_free_bytes: INSTALL_HEADROOM_BYTES,
            // No transfer is required for an already verified outcome. A later
            // installation always recomputes available capacity before writing.
            fits_available_disk: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub selection: ModelSelection,
    /// Artifacts that still need network transfer. Optional multimodal projectors are deliberately
    /// absent until their ingestion pipeline is enabled.
    pub artifact_ids: Vec<&'static str>,
    pub download_bytes: u64,
    pub required_free_bytes: u64,
    pub fits_available_disk: bool,
}

#[derive(Clone)]
pub struct AssetManager {
    root: PathBuf,
    client: Client,
    bundled_runtime: Option<PathBuf>,
}

impl AssetManager {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("airwiki/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            root: root.into(),
            client,
            bundled_runtime: None,
        })
    }

    /// Prefer a runtime shipped inside the installer. The archive is still pinned and verified by
    /// the packaging task; this path avoids downloading llama.cpp on first launch.
    pub fn with_bundled_runtime(mut self, server_binary: Option<PathBuf>) -> Self {
        // Keep an explicit path even when it is missing or malformed so installation and startup
        // verification fail closed instead of silently selecting a different executable.
        self.bundled_runtime = server_binary;
        self
    }

    pub fn required_artifacts(&self) -> Result<Vec<&'static Artifact>> {
        let mut artifacts = vec![&QWEN_MODEL];
        artifacts.extend(E5_FILES.iter());
        artifacts.extend(platform_relevance_artifacts()?);
        if self.bundled_runtime.is_none() {
            let runtime = platform_runtime().ok_or_else(|| {
                anyhow!(
                    "no downloadable llama.cpp runtime is defined for {} {}; Windows builds must bundle the reviewed source-built runtime",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )
            })?;
            artifacts.push(runtime);
        }
        Ok(artifacts)
    }

    pub fn model_path(&self, selection: &ModelSelection) -> PathBuf {
        if selection.model_id == QWEN_MODEL.id {
            // Preserve the path used by all installations prior to the adaptive catalog.
            self.root.join("models").join(QWEN_MODEL.filename)
        } else {
            self.root
                .join("models")
                .join(selection.model_id)
                .join(selection.manifest.artifact.revision)
                .join(selection.manifest.artifact.filename)
        }
    }

    /// Build an immutable, pinned text-model installation plan. Only missing or invalid files
    /// contribute to download size; one GiB of free-space headroom is always retained.
    pub fn build_install_plan(&self, selection: &ModelSelection) -> Result<InstallPlan> {
        let mut artifact_ids = Vec::new();
        let mut download_bytes = 0_u64;

        let model = selection.manifest.artifact;
        if verify_artifact_file(&self.model_path(selection), model).is_err() {
            artifact_ids.push(model.id);
            download_bytes = download_bytes.saturating_add(model.approximate_bytes);
        }

        let snapshot = self.embedding_snapshot_path();
        if !snapshot_is_verified(&snapshot)? {
            for artifact in &E5_FILES {
                artifact_ids.push(artifact.id);
                download_bytes = download_bytes.saturating_add(artifact.approximate_bytes);
            }
        }

        let relevance_artifacts = platform_relevance_artifacts()?;
        let relevance_snapshot = self.relevance_snapshot_path();
        if !relevance_snapshot_is_verified(&relevance_snapshot, &relevance_artifacts)? {
            for artifact in relevance_artifacts {
                artifact_ids.push(artifact.id);
                download_bytes = download_bytes.saturating_add(artifact.approximate_bytes);
            }
        }

        if let Some(path) = &self.bundled_runtime {
            verify_runtime_binary(path).with_context(|| {
                format!(
                    "bundled llama.cpp runtime failed verification at {}",
                    path.display()
                )
            })?;
        } else {
            let runtime = platform_runtime().context(
                "no downloadable llama.cpp runtime is available; use a distribution with the reviewed runtime bundled",
            )?;
            let runtime_dir = self
                .root
                .join("runtimes")
                .join(format!("llama-{LLAMA_CPP_BUILD}"));
            let installed = installed_runtime_path(&runtime_dir)?;
            if verify_runtime_binary(&installed).is_err() {
                let archive = self
                    .root
                    .join("cache")
                    .join("downloads")
                    .join(runtime.filename);
                if verify_artifact_file(&archive, runtime).is_err() {
                    artifact_ids.push(runtime.id);
                    download_bytes = download_bytes.saturating_add(runtime.approximate_bytes);
                }
            }
        }

        let required_free_bytes = download_bytes.saturating_add(INSTALL_HEADROOM_BYTES);
        let available_disk = available_space_for(&self.root)?;
        Ok(InstallPlan {
            selection: selection.clone(),
            artifact_ids,
            download_bytes,
            required_free_bytes,
            fits_available_disk: available_disk >= required_free_bytes,
        })
    }

    /// Async counterpart for UI/runtime workers; multi-gigabyte SHA-256 checks run on Tokio's
    /// blocking pool.
    pub async fn build_install_plan_async(
        &self,
        selection: &ModelSelection,
    ) -> Result<InstallPlan> {
        let manager = self.clone();
        let selection = selection.clone();
        tokio::task::spawn_blocking(move || manager.build_install_plan(&selection))
            .await
            .context("model install planning task failed")?
    }

    /// Verifies all required installed assets without downloading, deleting, renaming or writing
    /// any file.
    ///
    /// Hashing runs on Tokio's blocking pool so callers can safely invoke this during startup
    /// without blocking the UI or async network runtime. The text model, E5 embedding snapshot and
    /// platform-specific relevance snapshot are checked against pinned SHA-256 values and exact
    /// revision markers. `llama-server` is resolved from one deterministic b9946 path (or the
    /// explicit bundled path) and its executable hash is checked against the platform trust
    /// anchor. On Windows that hash is injected from the reviewed source-build manifest.
    pub async fn verify_installed(&self) -> Result<InstallOutcome> {
        self.verify_selection(&ModelSelection::legacy_qwen()).await
    }

    pub async fn verify_selection(&self, selection: &ModelSelection) -> Result<InstallOutcome> {
        let root = self.root.clone();
        let bundled_runtime = self.bundled_runtime.clone();
        let selection = selection.clone();
        tokio::task::spawn_blocking(move || {
            verify_installed_assets(&root, bundled_runtime.as_deref(), &selection)
        })
        .await
        .context("installed asset verification task failed")?
    }

    pub async fn install_required<F>(
        &self,
        cancel: CancellationToken,
        on_event: F,
    ) -> Result<InstallOutcome>
    where
        F: FnMut(InstallEvent) + Send,
    {
        self.install_selection_checked(&ModelSelection::legacy_qwen(), cancel, on_event)
            .await
    }

    pub async fn install_plan<F>(
        &self,
        plan: &InstallPlan,
        cancel: CancellationToken,
        on_event: F,
    ) -> Result<InstallOutcome>
    where
        F: FnMut(InstallEvent) + Send,
    {
        self.install_selection_checked(&plan.selection, cancel, on_event)
            .await
    }

    /// Recomputes the immutable plan immediately before any network transfer so a stale UI
    /// decision or newly consumed disk space cannot start an installation that no longer fits.
    pub async fn install_selection_checked<F>(
        &self,
        selection: &ModelSelection,
        cancel: CancellationToken,
        on_event: F,
    ) -> Result<InstallOutcome>
    where
        F: FnMut(InstallEvent) + Send,
    {
        let current = self.build_install_plan_async(selection).await?;
        if !current.fits_available_disk {
            bail!(
                "model installation requires {:.1} GiB free including headroom",
                current.required_free_bytes as f64 / (1024_f64.powi(3))
            );
        }
        self.install_selection(selection, cancel, on_event).await
    }

    pub async fn install_selection<F>(
        &self,
        selection: &ModelSelection,
        cancel: CancellationToken,
        mut on_event: F,
    ) -> Result<InstallOutcome>
    where
        F: FnMut(InstallEvent) + Send,
    {
        let model_dir = self.root.join("models");
        let cache_dir = self.root.join("cache").join("downloads");
        let runtime_dir = self
            .root
            .join("runtimes")
            .join(format!("llama-{LLAMA_CPP_BUILD}"));
        fs::create_dir_all(&model_dir).await?;
        fs::create_dir_all(&cache_dir).await?;
        fs::create_dir_all(self.root.join("runtimes")).await?;

        let model_path = self.model_path(selection);
        self.download_verified(
            selection.manifest.artifact,
            &model_path,
            &cancel,
            &mut on_event,
        )
        .await?;
        let embedding_snapshot_path = self
            .install_embedding_snapshot(&cancel, &mut on_event)
            .await?;
        let relevance_snapshot_path = self
            .install_relevance_snapshot(&cancel, &mut on_event)
            .await?;

        let llama_server_path = if let Some(path) = &self.bundled_runtime {
            verify_runtime_binary_async(path).await?;
            path.clone()
        } else {
            self.install_runtime_into(&cache_dir, &runtime_dir, &cancel, &mut on_event)
                .await?
        };

        Ok(InstallOutcome {
            selection: selection.clone(),
            model_path,
            generation_settings: selection.generation_settings(),
            embedding_snapshot_path,
            relevance_snapshot_path,
            llama_server_path,
        })
    }

    pub fn embedding_snapshot_path(&self) -> PathBuf {
        self.root
            .join("models")
            .join("multilingual-e5-small")
            .join("snapshots")
            .join(E5_REVISION)
    }

    pub fn relevance_snapshot_path(&self) -> PathBuf {
        self.root
            .join("models")
            .join("mmarco-mMiniLMv2-L12-H384-v1")
            .join("snapshots")
            .join(MMARCO_REVISION)
    }

    async fn install_embedding_snapshot<F>(
        &self,
        cancel: &CancellationToken,
        on_event: &mut F,
    ) -> Result<PathBuf>
    where
        F: FnMut(InstallEvent) + Send,
    {
        let final_path = self.embedding_snapshot_path();
        if snapshot_is_verified_async(&final_path).await? {
            return Ok(final_path);
        }
        let parent = final_path
            .parent()
            .context("embedding snapshot path has no parent")?;
        fs::create_dir_all(parent).await?;
        let staging = parent.join(format!("{E5_REVISION}.installing"));
        fs::create_dir_all(&staging).await?;
        for artifact in &E5_FILES {
            self.download_verified(artifact, &staging.join(artifact.filename), cancel, on_event)
                .await?;
        }
        write_revision_marker(&staging, E5_REVISION).await?;
        verify_embedding_snapshot_async(&staging).await?;
        if fs::try_exists(&final_path).await? {
            fs::remove_dir_all(&final_path).await?;
        }
        fs::rename(&staging, &final_path).await?;
        Ok(final_path)
    }

    async fn install_relevance_snapshot<F>(
        &self,
        cancel: &CancellationToken,
        on_event: &mut F,
    ) -> Result<PathBuf>
    where
        F: FnMut(InstallEvent) + Send,
    {
        let artifacts = platform_relevance_artifacts()?;
        let final_path = self.relevance_snapshot_path();
        if relevance_snapshot_is_verified_async(&final_path, &artifacts).await? {
            return Ok(final_path);
        }
        let parent = final_path
            .parent()
            .context("relevance snapshot path has no parent")?;
        fs::create_dir_all(parent).await?;
        let staging = parent.join(format!("{MMARCO_REVISION}.installing"));
        fs::create_dir_all(&staging).await?;
        for artifact in &artifacts {
            self.download_verified(artifact, &staging.join(artifact.filename), cancel, on_event)
                .await?;
        }
        write_revision_marker(&staging, MMARCO_REVISION).await?;
        verify_relevance_snapshot_async(&staging, &artifacts).await?;
        if fs::try_exists(&final_path).await? {
            fs::remove_dir_all(&final_path).await?;
        }
        fs::rename(&staging, &final_path).await?;
        Ok(final_path)
    }

    /// Download, verify and extract the upstream runtime on platforms where that archive is part
    /// of the reviewed distribution. Windows is intentionally bundled-only.
    pub async fn install_runtime_only<F>(
        &self,
        cancel: CancellationToken,
        mut on_event: F,
    ) -> Result<PathBuf>
    where
        F: FnMut(InstallEvent) + Send,
    {
        let cache_dir = self.root.join("cache").join("downloads");
        let runtime_dir = self
            .root
            .join("runtimes")
            .join(format!("llama-{LLAMA_CPP_BUILD}"));
        fs::create_dir_all(&cache_dir).await?;
        fs::create_dir_all(self.root.join("runtimes")).await?;
        self.install_runtime_into(&cache_dir, &runtime_dir, &cancel, &mut on_event)
            .await
    }

    async fn install_runtime_into<F>(
        &self,
        cache_dir: &Path,
        runtime_dir: &Path,
        cancel: &CancellationToken,
        on_event: &mut F,
    ) -> Result<PathBuf>
    where
        F: FnMut(InstallEvent) + Send,
    {
        let runtime = platform_runtime().context(
            "no downloadable llama.cpp runtime is available; Windows must use the reviewed source-built bundle",
        )?;
        let archive_path = cache_dir.join(runtime.filename);
        self.download_verified(runtime, &archive_path, cancel, on_event)
            .await?;
        let installed = installed_runtime_path(runtime_dir)?;
        if verify_runtime_binary_async(&installed).await.is_ok() {
            return Ok(installed);
        }
        on_event(InstallEvent::Extracting {
            artifact: runtime.id.to_owned(),
        });
        let root = self.root.clone();
        let archive = archive_path.clone();
        let filename = runtime.filename.to_owned();
        let destination = runtime_dir.to_path_buf();
        tokio::task::spawn_blocking(move || {
            extract_runtime(&root, &archive, &filename, &destination)
        })
        .await?
    }

    async fn download_verified<F>(
        &self,
        artifact: &Artifact,
        destination: &Path,
        cancel: &CancellationToken,
        on_event: &mut F,
    ) -> Result<()>
    where
        F: FnMut(InstallEvent) + Send,
    {
        let destination_exists = fs::try_exists(destination).await?;
        if destination_exists && verify_sha256_async(destination, artifact.sha256).await? {
            on_event(InstallEvent::Complete {
                artifact: artifact.id.to_owned(),
            });
            return Ok(());
        }
        if destination_exists {
            fs::remove_file(destination).await?;
        }
        let parent = destination
            .parent()
            .context("artifact destination has no parent")?;
        fs::create_dir_all(parent).await?;
        let part = destination.with_extension(format!(
            "{}part",
            destination
                .extension()
                .and_then(OsStr::to_str)
                .map(|v| format!("{v}."))
                .unwrap_or_default()
        ));
        let existing = fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);

        let mut request = self.client.get(artifact.url);
        if existing > 0 {
            request = request.header(header::RANGE, format!("bytes={existing}-"));
        }
        let response = request.send().await?;
        let (response, restarted_from_zero) =
            if existing > 0 && response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
                // A complete-length but corrupt partial file receives HTTP 416 from
                // range-aware hosts forever. Discard it and retry once from byte zero
                // so the repair action is self-healing.
                fs::remove_file(&part).await.ok();
                (
                    self.client
                        .get(artifact.url)
                        .send()
                        .await?
                        .error_for_status()?,
                    true,
                )
            } else {
                (response.error_for_status()?, false)
            };
        let resumed = !restarted_from_zero
            && existing > 0
            && response.status() == StatusCode::PARTIAL_CONTENT;
        let downloaded_start = if resumed { existing } else { 0 };
        let total = response
            .content_length()
            .map(|n| n + downloaded_start)
            .unwrap_or(artifact.approximate_bytes);
        on_event(InstallEvent::Started {
            artifact: artifact.id.to_owned(),
            total_bytes: total,
        });

        let mut options = fs::OpenOptions::new();
        options.create(true).write(true);
        if resumed {
            options.append(true);
        } else {
            options.truncate(true);
        }
        let mut file = options.open(&part).await?;
        let mut downloaded = downloaded_start;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = tokio::select! {
            _ = cancel.cancelled() => bail!("artifact installation was cancelled"),
            chunk = stream.next() => chunk,
        } {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            on_event(InstallEvent::Progress {
                artifact: artifact.id.to_owned(),
                downloaded,
                total_bytes: total,
            });
        }
        file.flush().await?;
        file.sync_all().await?;
        drop(file);

        on_event(InstallEvent::Verifying {
            artifact: artifact.id.to_owned(),
        });
        let valid = verify_sha256_async(&part, artifact.sha256).await?;
        if !valid {
            fs::remove_file(&part).await.ok();
            bail!("SHA-256 mismatch for {}", artifact.id);
        }
        fs::rename(&part, destination).await?;
        info!(
            artifact = artifact.id,
            "installed verified inference artifact"
        );
        on_event(InstallEvent::Complete {
            artifact: artifact.id.to_owned(),
        });
        Ok(())
    }
}

fn verify_sha256(path: &Path, expected: &str) -> Result<bool> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()).eq_ignore_ascii_case(expected))
}

async fn run_blocking_integrity<T, F>(operation: F, task: &'static str) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .with_context(|| task)?
}

async fn verify_sha256_async(path: &Path, expected: &str) -> Result<bool> {
    let path = path.to_path_buf();
    let expected = expected.to_owned();
    run_blocking_integrity(
        move || verify_sha256(&path, &expected),
        "artifact SHA-256 verification task failed",
    )
    .await
}

async fn snapshot_is_verified_async(path: &Path) -> Result<bool> {
    let path = path.to_path_buf();
    run_blocking_integrity(
        move || snapshot_is_verified(&path),
        "embedding snapshot verification task failed",
    )
    .await
}

async fn verify_embedding_snapshot_async(path: &Path) -> Result<()> {
    let path = path.to_path_buf();
    run_blocking_integrity(
        move || verify_embedding_snapshot(&path),
        "embedding snapshot verification task failed",
    )
    .await
}

async fn relevance_snapshot_is_verified_async(
    path: &Path,
    artifacts: &[&'static Artifact],
) -> Result<bool> {
    let path = path.to_path_buf();
    let artifacts = artifacts.to_vec();
    run_blocking_integrity(
        move || relevance_snapshot_is_verified(&path, &artifacts),
        "relevance snapshot verification task failed",
    )
    .await
}

async fn verify_relevance_snapshot_async(
    path: &Path,
    artifacts: &[&'static Artifact],
) -> Result<()> {
    let path = path.to_path_buf();
    let artifacts = artifacts.to_vec();
    run_blocking_integrity(
        move || verify_relevance_snapshot(&path, &artifacts),
        "relevance snapshot verification task failed",
    )
    .await
}

async fn verify_runtime_binary_async(path: &Path) -> Result<()> {
    let path = path.to_path_buf();
    run_blocking_integrity(
        move || verify_runtime_binary(&path),
        "llama.cpp runtime verification task failed",
    )
    .await
}

fn verify_installed_assets(
    root: &Path,
    bundled_runtime: Option<&Path>,
    selection: &ModelSelection,
) -> Result<InstallOutcome> {
    let model_path = if selection.model_id == QWEN_MODEL.id {
        root.join("models").join(QWEN_MODEL.filename)
    } else {
        root.join("models")
            .join(selection.model_id)
            .join(selection.manifest.artifact.revision)
            .join(selection.manifest.artifact.filename)
    };
    verify_artifact_file(&model_path, selection.manifest.artifact)?;

    let embedding_snapshot_path = root
        .join("models")
        .join("multilingual-e5-small")
        .join("snapshots")
        .join(E5_REVISION);
    verify_embedding_snapshot(&embedding_snapshot_path)?;

    let relevance_snapshot_path = root
        .join("models")
        .join("mmarco-mMiniLMv2-L12-H384-v1")
        .join("snapshots")
        .join(MMARCO_REVISION);
    let relevance_artifacts = platform_relevance_artifacts()?;
    verify_relevance_snapshot(&relevance_snapshot_path, &relevance_artifacts)?;

    let llama_server_path = match bundled_runtime {
        Some(path) => path.to_path_buf(),
        None => {
            let runtime_dir = root
                .join("runtimes")
                .join(format!("llama-{LLAMA_CPP_BUILD}"));
            installed_runtime_path(&runtime_dir)?
        }
    };
    verify_runtime_binary(&llama_server_path)?;

    Ok(InstallOutcome {
        selection: selection.clone(),
        model_path,
        generation_settings: selection.generation_settings(),
        embedding_snapshot_path,
        relevance_snapshot_path,
        llama_server_path,
    })
}

fn verify_artifact_file(path: &Path, artifact: &Artifact) -> Result<()> {
    verify_regular_file(path, artifact.sha256, artifact.id)
}

fn verify_regular_file(path: &Path, expected_sha256: &str, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("{label} is not installed at {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("{label} is not a regular file at {}", path.display());
    }
    if !verify_sha256(path, expected_sha256)
        .with_context(|| format!("failed to hash {label} at {}", path.display()))?
    {
        bail!("SHA-256 mismatch for {label} at {}", path.display());
    }
    Ok(())
}

fn verify_embedding_snapshot(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).with_context(|| {
        format!(
            "multilingual-e5-small snapshot is not installed at {}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "multilingual-e5-small snapshot is not a regular directory at {}",
            path.display()
        );
    }
    for artifact in &E5_FILES {
        verify_artifact_file(&path.join(artifact.filename), artifact)?;
    }
    verify_revision_marker(
        &path.join("revision.txt"),
        E5_REVISION,
        "embedding revision marker",
    )
}

fn verify_relevance_snapshot(path: &Path, artifacts: &[&Artifact]) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).with_context(|| {
        format!(
            "mmarco-mMiniLMv2-L12-H384-v1 snapshot is not installed at {}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "mmarco-mMiniLMv2-L12-H384-v1 snapshot is not a regular directory at {}",
            path.display()
        );
    }
    for artifact in artifacts {
        verify_artifact_file(&path.join(artifact.filename), artifact)?;
    }
    verify_revision_marker(
        &path.join("revision.txt"),
        MMARCO_REVISION,
        "relevance revision marker",
    )
}

fn verify_revision_marker(revision_path: &Path, expected: &str, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(revision_path)
        .with_context(|| format!("{label} is not installed at {}", revision_path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "{label} is not a regular file at {}",
            revision_path.display()
        );
    }
    let revision = std::fs::read(revision_path)
        .with_context(|| format!("failed to read {label} at {}", revision_path.display()))?;
    let expected = format!("{expected}\n");
    if revision != expected.as_bytes() {
        bail!("{label} mismatch at {}", revision_path.display());
    }
    Ok(())
}

async fn write_revision_marker(snapshot: &Path, revision: &str) -> Result<()> {
    let revision_part = snapshot.join("revision.txt.part");
    let revision_path = snapshot.join("revision.txt");
    let mut revision_file = fs::File::create(&revision_part).await?;
    revision_file.write_all(revision.as_bytes()).await?;
    revision_file.write_all(b"\n").await?;
    revision_file.sync_all().await?;
    drop(revision_file);
    if fs::try_exists(&revision_path).await? {
        fs::remove_file(&revision_path).await?;
    }
    fs::rename(revision_part, revision_path).await?;
    Ok(())
}

fn installed_runtime_path(runtime_dir: &Path) -> Result<PathBuf> {
    let manifest =
        platform_runtime_binary().context("unsupported platform for llama.cpp runtime")?;
    Ok(runtime_dir.join(manifest.extracted_relative_path))
}

fn verify_runtime_binary(path: &Path) -> Result<()> {
    let manifest =
        platform_runtime_binary().context("unsupported platform for llama.cpp runtime")?;
    if path.file_name() != Some(OsStr::new(manifest.filename)) {
        bail!(
            "expected {} b9946 runtime, got {}",
            manifest.filename,
            path.display()
        );
    }
    verify_regular_file(path, manifest.sha256, "llama.cpp b9946 runtime")
}

fn snapshot_is_verified(path: &Path) -> Result<bool> {
    Ok(verify_embedding_snapshot(path).is_ok())
}

fn relevance_snapshot_is_verified(path: &Path, artifacts: &[&Artifact]) -> Result<bool> {
    Ok(verify_relevance_snapshot(path, artifacts).is_ok())
}

fn extract_runtime(
    root: &Path,
    archive: &Path,
    filename: &str,
    destination: &Path,
) -> Result<PathBuf> {
    let temp = tempfile::Builder::new()
        .prefix("llama-extract-")
        .tempdir_in(root)?;
    let extracted = temp.path().join("content");
    std::fs::create_dir_all(&extracted)?;
    if filename.ends_with(".tar.gz") {
        let reader = GzDecoder::new(File::open(archive)?);
        let mut tar = tar::Archive::new(reader);
        tar.unpack(&extracted)?;
    } else if filename.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let Some(relative) = entry.enclosed_name() else {
                continue;
            };
            let output = extracted.join(relative);
            if entry.is_dir() {
                std::fs::create_dir_all(&output)?;
            } else {
                if let Some(parent) = output.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut output_file = File::create(&output)?;
                std::io::copy(&mut entry, &mut output_file)?;
                output_file.flush()?;
            }
        }
    } else {
        bail!("unsupported runtime archive: {filename}");
    }

    let runtime_manifest = platform_runtime_binary().context("unsupported platform for runtime")?;
    let binary = extracted.join(runtime_manifest.extracted_relative_path);
    verify_runtime_binary(&binary).with_context(|| {
        format!(
            "verified archive did not contain the expected {} b9946 executable",
            runtime_manifest.filename
        )
    })?;
    let relative = binary.strip_prefix(&extracted)?.to_path_buf();
    if destination.exists() {
        std::fs::remove_dir_all(destination)?;
    }
    std::fs::rename(&extracted, destination)?;
    let installed = destination.join(relative);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&installed)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&installed, permissions)?;
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[test]
    fn retry_classifier_accepts_only_temporary_io_failures() {
        let temporary = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "synthetic timeout",
        ));
        let action_required = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "synthetic permission failure",
        ));

        assert!(install_failure_is_transient(&temporary));
        assert!(!install_failure_is_transient(&action_required));
        assert!(!install_failure_is_transient(&anyhow!("hash mismatch")));
    }

    #[test]
    fn sha256_validation_is_exact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artifact");
        std::fs::write(&path, b"airwiki").unwrap();
        assert!(
            verify_sha256(
                &path,
                "807072ad6ebfe1c42c3749025060c030186b6e8670ce5c26e2c6f8e3e61be471"
            )
            .unwrap()
        );
        assert!(!verify_sha256(&path, &"0".repeat(64)).unwrap());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn integrity_verification_does_not_block_the_async_executor() {
        let started = std::time::Instant::now();
        let verification = run_blocking_integrity(
            || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                Ok(())
            },
            "test integrity task failed",
        );
        let heartbeat = async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            started.elapsed()
        };

        let (result, heartbeat_elapsed) = tokio::join!(verification, heartbeat);

        result.unwrap();
        assert!(
            heartbeat_elapsed < std::time::Duration::from_millis(75),
            "integrity verification stalled Tokio for {heartbeat_elapsed:?}"
        );
    }

    #[test]
    fn manifests_are_pinned() {
        assert_eq!(QWEN_MODEL.revision.len(), 40);
        assert_eq!(QWEN_MODEL.sha256.len(), 64);
        assert_eq!(MACOS_LLAMA.revision, LLAMA_CPP_BUILD);
        assert_eq!(E5_FILES.len(), 5);
        assert!(
            E5_FILES
                .iter()
                .all(|artifact| artifact.revision == E5_REVISION)
        );
        assert!(E5_FILES.iter().all(|artifact| artifact.sha256.len() == 64));
        assert_eq!(MMARCO_REVISION.len(), 40);
        assert_eq!(MMARCO_COMMON_FILES.len(), 4);
        assert!(
            MMARCO_COMMON_FILES
                .iter()
                .all(|artifact| artifact.revision == MMARCO_REVISION)
        );
        assert!(
            MMARCO_COMMON_FILES
                .iter()
                .all(|artifact| artifact.sha256.len() == 64)
        );
        assert_eq!(MACOS_MMARCO_MODEL.revision, MMARCO_REVISION);
        assert_eq!(WINDOWS_MMARCO_MODEL.revision, MMARCO_REVISION);
        assert_eq!(MACOS_MMARCO_MODEL.sha256.len(), 64);
        assert_eq!(WINDOWS_MMARCO_MODEL.sha256.len(), 64);
        assert!(
            MMARCO_COMMON_FILES
                .iter()
                .chain([&MACOS_MMARCO_MODEL, &WINDOWS_MMARCO_MODEL])
                .all(|artifact| artifact.license_url == APACHE_2_LICENSE_URL)
        );
        assert_eq!(MACOS_LLAMA_SERVER_SHA256.len(), 64);
        assert_eq!(WINDOWS_LLAMA_SERVER_SHA256.len(), 64);
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    #[test]
    fn windows_runtime_is_bundled_only() {
        let dir = tempfile::tempdir().unwrap();
        let manager = AssetManager::new(dir.path()).unwrap();

        assert!(platform_runtime().is_none());
        assert!(manager.required_artifacts().is_err());
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    #[test]
    fn windows_bundled_runtime_is_not_added_to_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let manager = AssetManager::new(dir.path())
            .unwrap()
            .with_bundled_runtime(Some(dir.path().join("llama-server.exe")));

        let artifacts = manager.required_artifacts().unwrap();

        assert!(
            artifacts
                .iter()
                .all(|artifact| artifact.kind != ArtifactKind::RuntimeArchive)
        );
    }

    #[test]
    fn relevance_model_selection_is_platform_specific_and_fail_closed() {
        assert_eq!(
            relevance_model_for("macos", "aarch64").map(|artifact| artifact.id),
            Some(MACOS_MMARCO_MODEL.id)
        );
        assert_eq!(
            relevance_model_for("windows", "x86_64").map(|artifact| artifact.id),
            Some(WINDOWS_MMARCO_MODEL.id)
        );
        assert!(relevance_model_for("linux", "x86_64").is_none());
        assert!(relevance_model_for("macos", "x86_64").is_none());
    }

    #[test]
    fn current_platform_relevance_manifest_contains_common_and_one_model_file() {
        let Some(platform_model) = platform_relevance_model() else {
            return;
        };
        let artifacts = platform_relevance_artifacts().unwrap();

        assert_eq!(artifacts.len(), MMARCO_COMMON_FILES.len() + 1);
        assert_eq!(
            artifacts.last().map(|artifact| artifact.id),
            Some(platform_model.id)
        );
        assert!(
            artifacts
                .iter()
                .all(|artifact| artifact.kind == ArtifactKind::RelevanceAsset)
        );
    }

    #[test]
    fn required_artifacts_include_every_relevance_snapshot_file() {
        let Some(runtime) = platform_runtime() else {
            return;
        };
        let Some(platform_model) = platform_relevance_model() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let manager = AssetManager::new(dir.path()).unwrap();
        let expected = std::iter::once(QWEN_MODEL.id)
            .chain(E5_FILES.iter().map(|artifact| artifact.id))
            .chain(MMARCO_COMMON_FILES.iter().map(|artifact| artifact.id))
            .chain(std::iter::once(platform_model.id))
            .chain(std::iter::once(runtime.id))
            .collect::<Vec<_>>();

        assert_eq!(
            manager
                .required_artifacts()
                .unwrap()
                .iter()
                .map(|artifact| artifact.id)
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn clean_install_plan_is_dynamic_and_excludes_multimodal_projectors() {
        let Some(runtime) = platform_runtime() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let manager = AssetManager::new(dir.path()).unwrap();
        let selection = crate::selection_for_model(
            crate::ModelProfile::Automatic,
            crate::GEMMA_4_E2B.id,
            "test",
        )
        .unwrap();

        let plan = manager.build_install_plan(&selection).unwrap();
        let embeddings_bytes: u64 = E5_FILES.iter().map(|item| item.approximate_bytes).sum();
        let relevance_bytes: u64 = platform_relevance_artifacts()
            .unwrap()
            .iter()
            .map(|item| item.approximate_bytes)
            .sum();
        assert_eq!(
            plan.download_bytes,
            crate::GEMMA_4_E2B_MODEL.approximate_bytes
                + embeddings_bytes
                + relevance_bytes
                + runtime.approximate_bytes
        );
        assert_eq!(
            plan.required_free_bytes,
            plan.download_bytes + INSTALL_HEADROOM_BYTES
        );
        assert!(!plan.artifact_ids.contains(&crate::GEMMA_4_E2B_MMPROJ.id));
    }

    #[test]
    fn qwen_keeps_its_legacy_model_path() {
        let dir = tempfile::tempdir().unwrap();
        let manager = AssetManager::new(dir.path()).unwrap();
        assert_eq!(
            manager.model_path(&ModelSelection::legacy_qwen()),
            dir.path().join("models").join(QWEN_MODEL.filename)
        );
    }

    #[test]
    fn revision_marker_must_match_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("revision.txt");
        std::fs::write(&marker, format!("{E5_REVISION}\n")).unwrap();
        verify_revision_marker(&marker, E5_REVISION, "embedding revision marker").unwrap();

        std::fs::write(&marker, E5_REVISION).unwrap();
        let error = verify_revision_marker(&marker, E5_REVISION, "embedding revision marker")
            .unwrap_err()
            .to_string();
        assert!(error.contains("revision marker mismatch"));
    }

    #[test]
    fn relevance_snapshot_path_is_revision_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let manager = AssetManager::new(dir.path()).unwrap();

        assert_eq!(
            manager.relevance_snapshot_path(),
            dir.path()
                .join("models")
                .join("mmarco-mMiniLMv2-L12-H384-v1")
                .join("snapshots")
                .join(MMARCO_REVISION)
        );
    }

    #[test]
    fn installed_runtime_resolution_is_a_single_exact_path() {
        let Some(manifest) = platform_runtime_binary() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let runtime_root = dir.path().join(format!("llama-{LLAMA_CPP_BUILD}"));
        let decoy = runtime_root.join("unexpected").join(manifest.filename);
        std::fs::create_dir_all(decoy.parent().unwrap()).unwrap();
        std::fs::write(&decoy, b"decoy").unwrap();

        assert_eq!(
            installed_runtime_path(&runtime_root).unwrap(),
            runtime_root.join(manifest.extracted_relative_path)
        );
        assert_ne!(installed_runtime_path(&runtime_root).unwrap(), decoy);
    }

    #[cfg(unix)]
    #[test]
    fn integrity_checks_reject_symbolic_link_files() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("artifact");
        std::fs::write(&target, b"airwiki").unwrap();
        symlink(&target, &link).unwrap();

        let error = verify_regular_file(
            &link,
            "b5b2d15c36585dd4ee969a0417a981a89ae1565736e55af8e8e2f8623406a1d6",
            "test artifact",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("not a regular file"));
    }

    #[tokio::test]
    async fn startup_verification_does_not_create_or_replace_assets() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join("models");
        std::fs::create_dir_all(&model_dir).unwrap();
        let model = model_dir.join(QWEN_MODEL.filename);
        std::fs::write(&model, b"invalid model").unwrap();
        let before = std::fs::read(&model).unwrap();

        let manager = AssetManager::new(dir.path()).unwrap();
        let error = manager.verify_installed().await.unwrap_err().to_string();

        assert!(error.contains("SHA-256 mismatch"));
        assert_eq!(std::fs::read(&model).unwrap(), before);
        assert!(!dir.path().join("cache").exists());
        assert!(!dir.path().join("runtimes").exists());
    }

    #[test]
    fn verified_outcome_builds_a_plan_without_reading_artifact_paths() {
        let selection = ModelSelection::legacy_qwen();
        let outcome = InstallOutcome {
            generation_settings: selection.generation_settings(),
            selection,
            model_path: PathBuf::from("missing/model.gguf"),
            embedding_snapshot_path: PathBuf::from("missing/embeddings"),
            relevance_snapshot_path: PathBuf::from("missing/relevance"),
            llama_server_path: PathBuf::from("missing/llama-server"),
        };

        let plan = outcome.verified_install_plan();

        assert!(plan.artifact_ids.is_empty());
        assert_eq!(plan.download_bytes, 0);
        assert_eq!(plan.required_free_bytes, INSTALL_HEADROOM_BYTES);
        assert!(plan.fits_available_disk);
        assert_eq!(plan.selection.model_id, outcome.selection.model_id);
    }

    #[tokio::test]
    async fn range_416_discards_the_partial_and_restarts_from_zero() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let body = b"verified replacement".to_vec();
        let served = body.clone();
        let server = tokio::spawn(async move {
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0_u8; 4096];
                let read = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..read]);
                if attempt == 0 {
                    assert!(request.to_ascii_lowercase().contains("range: bytes="));
                    socket
                        .write_all(
                            b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await
                        .unwrap();
                } else {
                    assert!(!request.to_ascii_lowercase().contains("range: bytes="));
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        served.len()
                    );
                    socket.write_all(headers.as_bytes()).await.unwrap();
                    socket.write_all(&served).await.unwrap();
                }
            }
        });

        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("artifact.bin");
        let partial = destination.with_extension("bin.part");
        std::fs::write(&partial, b"corrupt complete partial").unwrap();
        let url: &'static str =
            Box::leak(format!("http://{address}/artifact.bin").into_boxed_str());
        let sha256: &'static str = Box::leak(hex::encode(Sha256::digest(&body)).into_boxed_str());
        let artifact = Artifact {
            id: "range-recovery-test",
            kind: ArtifactKind::Model,
            filename: "artifact.bin",
            revision: "test",
            url,
            sha256,
            approximate_bytes: body.len() as u64,
            license: "test",
            license_url: "https://example.invalid",
        };
        let manager = AssetManager::new(temp.path()).unwrap();
        manager
            .download_verified(
                &artifact,
                &destination,
                &CancellationToken::new(),
                &mut |_| {},
            )
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(std::fs::read(destination).unwrap(), body);
        assert!(!partial.exists());
    }
}
