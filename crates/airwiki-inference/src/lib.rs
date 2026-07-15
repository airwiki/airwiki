//! Included local inference runtime.
//!
//! This crate never sends document contents to the Internet. Network access is used only by
//! [`AssetManager`] to install pinned artifacts after the user accepts their licenses.

mod assets;
mod catalog;
mod client;
mod diagnostic;
mod supervisor;

pub use assets::{
    Artifact, ArtifactKind, AssetManager, E5_FILES, E5_REVISION, InstallEvent, InstallOutcome,
    InstallPlan, LLAMA_CPP_BUILD, MACOS_LLAMA_SERVER_SHA256, MACOS_MMARCO_MODEL,
    MMARCO_COMMON_FILES, MMARCO_REVISION, QWEN_MODEL, WINDOWS_LLAMA_SERVER_SHA256,
    WINDOWS_MMARCO_MODEL, install_failure_is_transient, platform_relevance_model,
};
pub use catalog::{
    GEMMA_4_E2B, GEMMA_4_E2B_MMPROJ, GEMMA_4_E2B_MODEL, GEMMA_4_E4B, GEMMA_4_E4B_MMPROJ,
    GEMMA_4_E4B_MODEL, GenerationSettings, INSTALL_HEADROOM_BYTES, MODEL_CATALOG, ModelBackend,
    ModelCapability, ModelDecision, ModelManifest, ModelProfile, ModelSelection, QWEN_3_1_7B,
    STRUCTURED_OUTPUT_TOKENS, ThinkingControl, manifest_by_id, model_by_id, select_model,
    selection_for_model,
};
pub use client::{GenerationConfig, LlamaClient};
pub use diagnostic::{HardwareReport, diagnose_hardware};
pub use supervisor::{LlamaEndpoint, LlamaSupervisor, ServerReasoningMode, SupervisorConfig};
