use serde::{Deserialize, Serialize};

use crate::{
    assets::{Artifact, ArtifactKind, QWEN_MODEL},
    diagnostic::HardwareReport,
};

pub const GIB: u64 = 1024 * 1024 * 1024;
pub const INSTALL_HEADROOM_BYTES: u64 = GIB;
pub const STRUCTURED_OUTPUT_TOKENS: u16 = 384;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum ModelProfile {
    #[default]
    Automatic,
    Efficient,
    Quality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapability {
    StructuredText,
    VisionDocument,
    SpeechTranscription,
    ShortAudioUnderstanding,
    VideoUnderstanding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingControl {
    None,
    NoThinkDirective,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct GenerationSettings {
    pub model_api_id: &'static str,
    pub context_tokens: u32,
    pub max_input_tokens: u32,
    pub max_output_tokens: u16,
    pub temperature: f32,
    pub thinking_control: ThinkingControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelBackend {
    LlamaCpp,
}

#[derive(Debug, Clone)]
pub struct ModelManifest {
    /// Immutable installation identity. A different artifact revision or SHA-256 must receive a
    /// new ID so persisted active/pending selections can always resolve to their exact bytes.
    pub id: &'static str,
    pub display_name: &'static str,
    pub artifact: &'static Artifact,
    /// Pinned for the later multimodal pipeline, but not part of a text-only install plan.
    pub multimodal_projector: Option<&'static Artifact>,
    pub capabilities: &'static [ModelCapability],
    pub backend: ModelBackend,
    pub minimum_ram_bytes: u64,
    pub windows_minimum_ram_bytes: u64,
    pub recommended_ram_bytes: u64,
    pub supports_macos_arm64: bool,
    pub supports_windows_x64: bool,
    pub windows_requires_avx2: bool,
    pub generation_settings: GenerationSettings,
}

impl ModelManifest {
    pub fn supports(&self, capability: ModelCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    pub fn is_hardware_eligible(&self, hardware: &HardwareReport) -> bool {
        let (target_supported, minimum_ram) =
            match (hardware.os.as_str(), hardware.architecture.as_str()) {
                ("macos", "aarch64") => (
                    self.supports_macos_arm64 && hardware.metal_available,
                    self.minimum_ram_bytes,
                ),
                ("windows", "x86_64") => (
                    self.supports_windows_x64 && (!self.windows_requires_avx2 || hardware.avx2),
                    self.windows_minimum_ram_bytes,
                ),
                _ => (false, u64::MAX),
            };
        target_supported && hardware.total_memory_bytes >= minimum_ram
    }
}

pub const GEMMA_4_E2B_MODEL: Artifact = Artifact {
    id: "gemma-4-e2b-q4",
    kind: ArtifactKind::Model,
    filename: "gemma-4-E2B_q4_0-it.gguf",
    revision: "69536a21d70340464240401ba38223d805f6a709",
    url: "https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf/resolve/69536a21d70340464240401ba38223d805f6a709/gemma-4-E2B_q4_0-it.gguf?download=true",
    sha256: "3646b4c147cd235a44d91df1546d3b7d8e29b547dbe4e1f80856419aa455e6fd",
    approximate_bytes: 3_349_514_112,
    license: "Apache-2.0",
    license_url: "https://www.apache.org/licenses/LICENSE-2.0.txt",
};

pub const GEMMA_4_E2B_MMPROJ: Artifact = Artifact {
    id: "gemma-4-e2b-mmproj",
    kind: ArtifactKind::MultimodalProjector,
    filename: "gemma-4-E2B-it-mmproj.gguf",
    revision: "69536a21d70340464240401ba38223d805f6a709",
    url: "https://huggingface.co/google/gemma-4-E2B-it-qat-q4_0-gguf/resolve/69536a21d70340464240401ba38223d805f6a709/gemma-4-E2B-it-mmproj.gguf?download=true",
    sha256: "58c187648007cab392bd5678b87e862c3e8794017deb945feea2cf256195e96a",
    approximate_bytes: 986_833_312,
    license: "Apache-2.0",
    license_url: "https://www.apache.org/licenses/LICENSE-2.0.txt",
};

pub const GEMMA_4_E4B_MODEL: Artifact = Artifact {
    id: "gemma-4-e4b-q4",
    kind: ArtifactKind::Model,
    filename: "gemma-4-E4B_q4_0-it.gguf",
    revision: "7edc6763a77bbca236126a361613b834c5ea0f7a",
    url: "https://huggingface.co/google/gemma-4-E4B-it-qat-q4_0-gguf/resolve/7edc6763a77bbca236126a361613b834c5ea0f7a/gemma-4-E4B_q4_0-it.gguf?download=true",
    sha256: "e8b6a059ba86947a44ace84d6e5679795bc41862c25c30513142588f0e9dba1d",
    approximate_bytes: 5_154_939_136,
    license: "Apache-2.0",
    license_url: "https://www.apache.org/licenses/LICENSE-2.0.txt",
};

pub const GEMMA_4_E4B_MMPROJ: Artifact = Artifact {
    id: "gemma-4-e4b-mmproj",
    kind: ArtifactKind::MultimodalProjector,
    filename: "gemma-4-E4B-it-mmproj.gguf",
    revision: "7edc6763a77bbca236126a361613b834c5ea0f7a",
    url: "https://huggingface.co/google/gemma-4-E4B-it-qat-q4_0-gguf/resolve/7edc6763a77bbca236126a361613b834c5ea0f7a/gemma-4-E4B-it-mmproj.gguf?download=true",
    sha256: "c6398448d84a4836fdedf58f9775979e69ae0cc4dfdf4d697b5597693a555b12",
    approximate_bytes: 991_551_904,
    license: "Apache-2.0",
    license_url: "https://www.apache.org/licenses/LICENSE-2.0.txt",
};

const TEXT_ONLY: &[ModelCapability] = &[ModelCapability::StructuredText];
const GEMMA_MULTIMODAL: &[ModelCapability] = &[
    ModelCapability::StructuredText,
    ModelCapability::VisionDocument,
    ModelCapability::ShortAudioUnderstanding,
    ModelCapability::VideoUnderstanding,
];

const GEMMA_E2B_GENERATION: GenerationSettings = GenerationSettings {
    model_api_id: "gemma-4-e2b-q4",
    context_tokens: 4_096,
    max_input_tokens: 2_800,
    max_output_tokens: STRUCTURED_OUTPUT_TOKENS,
    temperature: 0.1,
    thinking_control: ThinkingControl::None,
};

const GEMMA_E4B_GENERATION: GenerationSettings = GenerationSettings {
    model_api_id: "gemma-4-e4b-q4",
    context_tokens: 4_096,
    max_input_tokens: 2_800,
    max_output_tokens: STRUCTURED_OUTPUT_TOKENS,
    temperature: 0.1,
    thinking_control: ThinkingControl::None,
};

const QWEN_GENERATION: GenerationSettings = GenerationSettings {
    model_api_id: "qwen3-1.7b-q8",
    context_tokens: 4_096,
    max_input_tokens: 2_800,
    max_output_tokens: STRUCTURED_OUTPUT_TOKENS,
    temperature: 0.1,
    thinking_control: ThinkingControl::NoThinkDirective,
};

pub static GEMMA_4_E2B: ModelManifest = ModelManifest {
    id: "gemma-4-e2b-q4",
    display_name: "Gemma 4 E2B Q4",
    artifact: &GEMMA_4_E2B_MODEL,
    multimodal_projector: Some(&GEMMA_4_E2B_MMPROJ),
    capabilities: GEMMA_MULTIMODAL,
    backend: ModelBackend::LlamaCpp,
    minimum_ram_bytes: 8 * GIB,
    windows_minimum_ram_bytes: 8 * GIB,
    recommended_ram_bytes: 8 * GIB,
    supports_macos_arm64: true,
    supports_windows_x64: true,
    windows_requires_avx2: true,
    generation_settings: GEMMA_E2B_GENERATION,
};

pub static GEMMA_4_E4B: ModelManifest = ModelManifest {
    id: "gemma-4-e4b-q4",
    display_name: "Gemma 4 E4B Q4",
    artifact: &GEMMA_4_E4B_MODEL,
    multimodal_projector: Some(&GEMMA_4_E4B_MMPROJ),
    capabilities: GEMMA_MULTIMODAL,
    backend: ModelBackend::LlamaCpp,
    minimum_ram_bytes: 12 * GIB,
    windows_minimum_ram_bytes: 16 * GIB,
    recommended_ram_bytes: 16 * GIB,
    supports_macos_arm64: true,
    supports_windows_x64: true,
    windows_requires_avx2: true,
    generation_settings: GEMMA_E4B_GENERATION,
};

pub static QWEN_3_1_7B: ModelManifest = ModelManifest {
    id: "qwen3-1.7b-q8",
    display_name: "Qwen3 1.7B Q8",
    artifact: &QWEN_MODEL,
    multimodal_projector: None,
    capabilities: TEXT_ONLY,
    backend: ModelBackend::LlamaCpp,
    minimum_ram_bytes: 8 * GIB,
    windows_minimum_ram_bytes: 8 * GIB,
    recommended_ram_bytes: 8 * GIB,
    supports_macos_arm64: true,
    supports_windows_x64: true,
    windows_requires_avx2: true,
    generation_settings: QWEN_GENERATION,
};

pub static MODEL_CATALOG: [&ModelManifest; 3] = [&GEMMA_4_E2B, &GEMMA_4_E4B, &QWEN_3_1_7B];

pub fn manifest_by_id(id: &str) -> Option<&'static ModelManifest> {
    MODEL_CATALOG
        .iter()
        .copied()
        .find(|manifest| manifest.id == id)
}

pub fn model_by_id(id: &str) -> Option<&'static ModelManifest> {
    manifest_by_id(id)
}

#[derive(Debug, Clone)]
pub struct ModelSelection {
    pub profile: ModelProfile,
    pub model_id: &'static str,
    pub manifest: &'static ModelManifest,
    pub degraded: bool,
    pub reason: String,
}

impl ModelSelection {
    pub fn legacy_qwen() -> Self {
        Self {
            profile: ModelProfile::Efficient,
            model_id: QWEN_3_1_7B.id,
            manifest: &QWEN_3_1_7B,
            degraded: false,
            reason: "Modelo Qwen legado instalado".to_owned(),
        }
    }

    pub fn generation_settings(&self) -> GenerationSettings {
        self.manifest.generation_settings
    }
}

pub fn selection_for_model(
    profile: ModelProfile,
    id: &str,
    reason: impl Into<String>,
) -> Option<ModelSelection> {
    let manifest = model_by_id(id)?;
    Some(ModelSelection {
        profile,
        model_id: manifest.id,
        manifest,
        degraded: false,
        reason: reason.into(),
    })
}

#[derive(Debug, Clone)]
pub struct ModelDecision {
    pub selection: Option<ModelSelection>,
    pub issues: Vec<String>,
}

impl ModelDecision {
    pub fn selected(&self) -> Option<&ModelSelection> {
        self.selection.as_ref()
    }
}

pub fn select_model(profile: ModelProfile, hardware: &HardwareReport) -> ModelDecision {
    let target_ok = matches!(
        (hardware.os.as_str(), hardware.architecture.as_str()),
        ("macos", "aarch64") | ("windows", "x86_64")
    );
    if !target_ok {
        return unsupported(format!(
            "No hay modelos soportados para {} {}",
            hardware.os, hardware.architecture
        ));
    }
    if hardware.os == "windows" && !hardware.avx2 {
        return unsupported("Windows requiere una CPU con AVX2".to_owned());
    }
    if hardware.os == "macos" && !hardware.metal_available {
        return unsupported("macOS arm64 requiere Metal disponible".to_owned());
    }
    if hardware.total_memory_bytes < GEMMA_4_E2B.minimum_ram_bytes {
        return unsupported(format!(
            "Se requieren al menos 8 GiB de RAM; se detectaron {:.1} GiB",
            hardware.total_memory_bytes as f64 / GIB as f64
        ));
    }

    let is_macos = hardware.os == "macos";
    let e4b_quality_eligible = if is_macos {
        hardware.total_memory_bytes >= 12 * GIB
    } else {
        hardware.total_memory_bytes >= 16 * GIB
    };
    let (manifest, degraded, reason) = match profile {
        ModelProfile::Automatic if is_macos && hardware.total_memory_bytes >= 12 * GIB => (
            &GEMMA_4_E4B,
            false,
            "Gemma 4 E4B aprovecha la memoria unificada disponible en esta Mac",
        ),
        ModelProfile::Automatic => (
            &GEMMA_4_E2B,
            false,
            "Gemma 4 E2B mantiene margen de memoria para la aplicación y el sistema",
        ),
        ModelProfile::Efficient => (
            &GEMMA_4_E2B,
            false,
            "El perfil eficiente prioriza menor uso de memoria y disco",
        ),
        ModelProfile::Quality if e4b_quality_eligible => (
            &GEMMA_4_E4B,
            false,
            "El hardware cumple el umbral del perfil de calidad",
        ),
        ModelProfile::Quality => (
            &GEMMA_4_E2B,
            true,
            "El perfil de calidad se reduce a E2B para conservar margen de memoria seguro",
        ),
    };

    ModelDecision {
        selection: Some(ModelSelection {
            profile,
            model_id: manifest.id,
            manifest,
            degraded,
            reason: reason.to_owned(),
        }),
        issues: Vec::new(),
    }
}

fn unsupported(issue: String) -> ModelDecision {
    ModelDecision {
        selection: None,
        issues: vec![issue],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hardware(os: &str, architecture: &str, ram_gib: u64, avx2: bool) -> HardwareReport {
        HardwareReport {
            os: os.to_owned(),
            architecture: architecture.to_owned(),
            total_memory_bytes: ram_gib * GIB,
            available_memory_bytes: ram_gib * GIB / 2,
            available_disk_bytes: 100 * GIB,
            avx2,
            metal_available: os == "macos" && architecture == "aarch64",
            supported_target: true,
            can_install: true,
            issues: Vec::new(),
        }
    }

    #[test]
    fn automatic_selects_e4b_for_a_sixteen_gib_mac() {
        let decision = select_model(
            ModelProfile::Automatic,
            &hardware("macos", "aarch64", 16, false),
        );
        assert_eq!(decision.selected().unwrap().model_id, GEMMA_4_E4B.id);
        assert!(!decision.selected().unwrap().degraded);
    }

    #[test]
    fn automatic_selects_e2b_for_an_eight_gib_windows_pc() {
        let decision = select_model(
            ModelProfile::Automatic,
            &hardware("windows", "x86_64", 8, true),
        );
        assert_eq!(decision.selected().unwrap().model_id, GEMMA_4_E2B.id);
    }

    #[test]
    fn automatic_selects_e2b_for_an_eight_gib_mac() {
        let decision = select_model(
            ModelProfile::Automatic,
            &hardware("macos", "aarch64", 8, false),
        );
        assert_eq!(decision.selected().unwrap().model_id, GEMMA_4_E2B.id);
    }

    #[test]
    fn quality_selects_e4b_at_each_platform_threshold() {
        let mac = select_model(
            ModelProfile::Quality,
            &hardware("macos", "aarch64", 12, false),
        );
        let windows = select_model(
            ModelProfile::Quality,
            &hardware("windows", "x86_64", 16, true),
        );
        assert_eq!(mac.selected().unwrap().model_id, GEMMA_4_E4B.id);
        assert_eq!(windows.selected().unwrap().model_id, GEMMA_4_E4B.id);
        assert!(!GEMMA_4_E4B.is_hardware_eligible(&hardware("windows", "x86_64", 12, true)));
    }

    #[test]
    fn quality_degrades_instead_of_overcommitting_memory() {
        let decision = select_model(
            ModelProfile::Quality,
            &hardware("windows", "x86_64", 8, true),
        );
        let selected = decision.selected().unwrap();
        assert_eq!(selected.model_id, GEMMA_4_E2B.id);
        assert!(selected.degraded);
    }

    #[test]
    fn windows_without_avx2_is_rejected() {
        let decision = select_model(
            ModelProfile::Automatic,
            &hardware("windows", "x86_64", 16, false),
        );
        assert!(decision.selection.is_none());
        assert!(decision.issues[0].contains("AVX2"));
    }

    #[test]
    fn unsupported_or_memory_constrained_hardware_is_rejected() {
        assert!(
            select_model(
                ModelProfile::Automatic,
                &hardware("linux", "x86_64", 32, true)
            )
            .selection
            .is_none()
        );
        assert!(
            select_model(
                ModelProfile::Automatic,
                &hardware("macos", "aarch64", 7, false)
            )
            .selection
            .is_none()
        );
    }

    #[test]
    fn manifests_pin_models_and_optional_projectors() {
        assert_eq!(GEMMA_4_E2B_MODEL.approximate_bytes, 3_349_514_112);
        assert_eq!(GEMMA_4_E4B_MODEL.approximate_bytes, 5_154_939_136);
        for manifest in MODEL_CATALOG {
            assert_eq!(manifest.artifact.sha256.len(), 64);
            assert!(!manifest.id.is_empty());
            assert_eq!(manifest.generation_settings.model_api_id, manifest.id);
            assert_eq!(
                manifest.generation_settings.max_output_tokens,
                STRUCTURED_OUTPUT_TOKENS
            );
        }
        assert_eq!(GEMMA_4_E2B.multimodal_projector.unwrap().sha256.len(), 64);
        assert_eq!(GEMMA_4_E4B.multimodal_projector.unwrap().sha256.len(), 64);
    }

    #[test]
    fn profile_and_capability_serde_are_stable() {
        assert_eq!(
            serde_json::to_string(&ModelProfile::Automatic).unwrap(),
            "\"automatic\""
        );
        assert_eq!(
            serde_json::from_str::<ModelCapability>("\"vision_document\"").unwrap(),
            ModelCapability::VisionDocument
        );
    }
}
