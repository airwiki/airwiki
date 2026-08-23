use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub(crate) const MODEL_ACTIVATION_STATUS_FILE: &str = "model-activation-status.json";
const MODEL_ACTIVATION_STATUS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelActivationErrorKind {
    Configuration,
    OnnxInit,
    EmbeddingSmoke,
    RerankerSmoke,
    RuntimeSpawn,
    RuntimeExitBeforeHealth,
    RuntimeHealthTimeout,
    RuntimeState,
    GenerationTimeout,
    GenerationUnavailable,
    GenerationProtocol,
    GenerationInvalid,
    ActivationInternal,
    InstallNetwork,
    InstallIntegrity,
    InstallStorage,
    InstallPromotion,
    InstallRuntimeVerification,
    InstallCapacity,
    InstallConfiguration,
    InstallCancelled,
    InstallInternal,
}

impl ModelActivationErrorKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::OnnxInit => "onnx_init",
            Self::EmbeddingSmoke => "embedding_smoke",
            Self::RerankerSmoke => "reranker_smoke",
            Self::RuntimeSpawn => "runtime_spawn",
            Self::RuntimeExitBeforeHealth => "runtime_exit_before_health",
            Self::RuntimeHealthTimeout => "runtime_health_timeout",
            Self::RuntimeState => "runtime_state",
            Self::GenerationTimeout => "generation_timeout",
            Self::GenerationUnavailable => "generation_unavailable",
            Self::GenerationProtocol => "generation_protocol",
            Self::GenerationInvalid => "generation_invalid",
            Self::ActivationInternal => "activation_internal",
            Self::InstallNetwork => "install_network",
            Self::InstallIntegrity => "install_integrity",
            Self::InstallStorage => "install_storage",
            Self::InstallPromotion => "install_promotion",
            Self::InstallRuntimeVerification => "install_runtime_verification",
            Self::InstallCapacity => "install_capacity",
            Self::InstallConfiguration => "install_configuration",
            Self::InstallCancelled => "install_cancelled",
            Self::InstallInternal => "install_internal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ModelActivationElapsedBucket {
    #[serde(rename = "under_5s")]
    Under5s,
    #[serde(rename = "5s_to_30s")]
    From5sTo30s,
    #[serde(rename = "30s_to_120s")]
    From30sTo120s,
    #[serde(rename = "120s_to_300s")]
    From120sTo300s,
    #[serde(rename = "over_300s")]
    Over300s,
}

impl ModelActivationElapsedBucket {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Under5s => "under_5s",
            Self::From5sTo30s => "5s_to_30s",
            Self::From30sTo120s => "30s_to_120s",
            Self::From120sTo300s => "120s_to_300s",
            Self::Over300s => "over_300s",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelActivationExitClass {
    None,
    Success,
    Failure,
    Unknown,
}

impl ModelActivationExitClass {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModelActivationFailureView {
    pub(crate) error_kind: ModelActivationErrorKind,
    pub(crate) exit_class: ModelActivationExitClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModelActivationState {
    Starting,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ModelActivationStatus {
    schema_version: u32,
    state: ModelActivationState,
    error_kind: Option<ModelActivationErrorKind>,
    elapsed_bucket: Option<ModelActivationElapsedBucket>,
    exit_class: Option<ModelActivationExitClass>,
}

impl ModelActivationStatus {
    pub(crate) const fn starting() -> Self {
        Self {
            schema_version: MODEL_ACTIVATION_STATUS_SCHEMA_VERSION,
            state: ModelActivationState::Starting,
            error_kind: None,
            elapsed_bucket: None,
            exit_class: None,
        }
    }

    pub(crate) const fn ready() -> Self {
        Self {
            schema_version: MODEL_ACTIVATION_STATUS_SCHEMA_VERSION,
            state: ModelActivationState::Ready,
            error_kind: None,
            elapsed_bucket: None,
            exit_class: None,
        }
    }

    pub(crate) const fn failed(
        failure: ModelActivationFailureView,
        elapsed_bucket: ModelActivationElapsedBucket,
    ) -> Self {
        Self {
            schema_version: MODEL_ACTIVATION_STATUS_SCHEMA_VERSION,
            state: ModelActivationState::Failed,
            error_kind: Some(failure.error_kind),
            elapsed_bucket: Some(elapsed_bucket),
            exit_class: Some(failure.exit_class),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ModelActivationStatusWriteError {
    #[error("model activation status path is invalid")]
    InvalidPath,
    #[error("model activation status serialization failed")]
    Serialization,
    #[error("model activation status storage is unavailable")]
    Storage,
    #[error("model activation status storage is unsafe")]
    UnsafeStorage,
    #[error("model activation status writer failed")]
    Worker,
}

impl ModelActivationStatusWriteError {
    pub(crate) const fn error_kind(self) -> &'static str {
        match self {
            Self::InvalidPath => "invalid_path",
            Self::Serialization => "serialization",
            Self::Storage => "storage",
            Self::UnsafeStorage => "unsafe_storage",
            Self::Worker => "worker",
        }
    }
}

pub(crate) async fn persist_model_activation_status(
    path: PathBuf,
    status: ModelActivationStatus,
) -> Result<(), ModelActivationStatusWriteError> {
    tokio::task::spawn_blocking(move || persist_model_activation_status_sync(&path, status))
        .await
        .map_err(|_| ModelActivationStatusWriteError::Worker)?
}

fn persist_model_activation_status_sync(
    path: &Path,
    status: ModelActivationStatus,
) -> Result<(), ModelActivationStatusWriteError> {
    let parent = path
        .parent()
        .ok_or(ModelActivationStatusWriteError::InvalidPath)?;
    fs::create_dir_all(parent).map_err(|_| ModelActivationStatusWriteError::Storage)?;
    let bytes =
        serde_json::to_vec(&status).map_err(|_| ModelActivationStatusWriteError::Serialization)?;
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));

    if temporary.exists() {
        let metadata = fs::symlink_metadata(&temporary)
            .map_err(|_| ModelActivationStatusWriteError::Storage)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(ModelActivationStatusWriteError::UnsafeStorage);
        }
        fs::remove_file(&temporary).map_err(|_| ModelActivationStatusWriteError::Storage)?;
    }
    if path.exists() {
        let metadata =
            fs::symlink_metadata(path).map_err(|_| ModelActivationStatusWriteError::Storage)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(ModelActivationStatusWriteError::UnsafeStorage);
        }
    }

    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        if let Ok(directory) = File::open(parent) {
            directory.sync_all().ok();
        }
        Ok::<(), std::io::Error>(())
    })();
    if write_result.is_err() {
        fs::remove_file(&temporary).ok();
    }
    write_result.map_err(|_| ModelActivationStatusWriteError::Storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_records_have_a_closed_sanitized_schema() {
        let failed = ModelActivationStatus::failed(
            ModelActivationFailureView {
                error_kind: ModelActivationErrorKind::RuntimeHealthTimeout,
                exit_class: ModelActivationExitClass::None,
            },
            ModelActivationElapsedBucket::From120sTo300s,
        );

        assert_eq!(
            serde_json::to_value(ModelActivationStatus::starting()).unwrap(),
            serde_json::json!({
                "schema_version": 1,
                "state": "starting",
                "error_kind": null,
                "elapsed_bucket": null,
                "exit_class": null,
            })
        );
        assert_eq!(
            serde_json::to_value(ModelActivationStatus::ready()).unwrap(),
            serde_json::json!({
                "schema_version": 1,
                "state": "ready",
                "error_kind": null,
                "elapsed_bucket": null,
                "exit_class": null,
            })
        );
        assert_eq!(
            serde_json::to_value(failed).unwrap(),
            serde_json::json!({
                "schema_version": 1,
                "state": "failed",
                "error_kind": "runtime_health_timeout",
                "elapsed_bucket": "120s_to_300s",
                "exit_class": "none",
            })
        );
    }

    #[test]
    fn serialized_classes_match_the_logged_allowlist_values() {
        for error_kind in [
            ModelActivationErrorKind::Configuration,
            ModelActivationErrorKind::OnnxInit,
            ModelActivationErrorKind::EmbeddingSmoke,
            ModelActivationErrorKind::RerankerSmoke,
            ModelActivationErrorKind::RuntimeSpawn,
            ModelActivationErrorKind::RuntimeExitBeforeHealth,
            ModelActivationErrorKind::RuntimeHealthTimeout,
            ModelActivationErrorKind::RuntimeState,
            ModelActivationErrorKind::GenerationTimeout,
            ModelActivationErrorKind::GenerationUnavailable,
            ModelActivationErrorKind::GenerationProtocol,
            ModelActivationErrorKind::GenerationInvalid,
            ModelActivationErrorKind::ActivationInternal,
            ModelActivationErrorKind::InstallNetwork,
            ModelActivationErrorKind::InstallIntegrity,
            ModelActivationErrorKind::InstallStorage,
            ModelActivationErrorKind::InstallPromotion,
            ModelActivationErrorKind::InstallRuntimeVerification,
            ModelActivationErrorKind::InstallCapacity,
            ModelActivationErrorKind::InstallConfiguration,
            ModelActivationErrorKind::InstallCancelled,
            ModelActivationErrorKind::InstallInternal,
        ] {
            assert_eq!(
                serde_json::to_value(error_kind).unwrap(),
                serde_json::json!(error_kind.as_str())
            );
        }
        for elapsed_bucket in [
            ModelActivationElapsedBucket::Under5s,
            ModelActivationElapsedBucket::From5sTo30s,
            ModelActivationElapsedBucket::From30sTo120s,
            ModelActivationElapsedBucket::From120sTo300s,
            ModelActivationElapsedBucket::Over300s,
        ] {
            assert_eq!(
                serde_json::to_value(elapsed_bucket).unwrap(),
                serde_json::json!(elapsed_bucket.as_str())
            );
        }
        for exit_class in [
            ModelActivationExitClass::None,
            ModelActivationExitClass::Success,
            ModelActivationExitClass::Failure,
            ModelActivationExitClass::Unknown,
        ] {
            assert_eq!(
                serde_json::to_value(exit_class).unwrap(),
                serde_json::json!(exit_class.as_str())
            );
        }
    }

    #[test]
    fn status_replaces_the_previous_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(MODEL_ACTIVATION_STATUS_FILE);
        persist_model_activation_status_sync(&path, ModelActivationStatus::starting()).unwrap();
        persist_model_activation_status_sync(&path, ModelActivationStatus::ready()).unwrap();

        let status: ModelActivationStatus =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();

        assert_eq!(status, ModelActivationStatus::ready());
    }

    #[test]
    fn status_rejects_a_non_regular_destination() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(MODEL_ACTIVATION_STATUS_FILE);
        fs::create_dir(&path).unwrap();

        assert_eq!(
            persist_model_activation_status_sync(&path, ModelActivationStatus::starting())
                .unwrap_err(),
            ModelActivationStatusWriteError::UnsafeStorage
        );
    }
}
