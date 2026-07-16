use std::{collections::HashMap, time::Duration};

use airwiki_types::{ConceptType, EnrichmentDraft};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::EMBEDDING_DIMENSIONS;

pub const GENERATION_CONTEXT_TOKENS: usize = 4_096;
pub const MAX_GENERATION_INPUT_TOKENS: usize = 2_800;
/// Structured enrichment is deliberately compact. On the minimum supported
/// hardware, allowing 1,024 generated tokens can consume the entire HTTP
/// deadline even though the useful JSON normally fits comfortably below 384.
pub const MAX_GENERATION_OUTPUT_TOKENS: usize = 384;
pub const E5_MODEL_REPOSITORY: &str = "intfloat/multilingual-e5-small";
pub const E5_MODEL_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";

const MAX_TITLE_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 180;
const MAX_LANGUAGE_CHARS: usize = 16;
const MAX_TAGS: usize = 5;
const MAX_TAG_CHARS: usize = 40;
const MAX_ENTITIES: usize = 3;
const MAX_ENTITY_NAME_CHARS: usize = 96;
const MAX_ENTITY_KIND_CHARS: usize = 32;
const MAX_LINKS: usize = 2;
const MAX_LINK_LABEL_CHARS: usize = 80;
const MAX_LINK_TARGET_CHARS: usize = 180;
const MAX_SUMMARY_CHARS: usize = 360;
const MAX_CLASSIFICATION_EXPLANATION_CHARS: usize = 120;
const MIN_GENERATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_GENERATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
const GENERATION_REQUEST_BASE_TIMEOUT: Duration = Duration::from_secs(120);

const SUMMARY_SYSTEM_PROMPT: &str = "Resume fielmente el texto empresarial en un máximo de 70 palabras. No inventes datos ni incluyas razonamiento. Devuelve solamente un objeto JSON compacto.";
const ENRICHMENT_SYSTEM_PROMPT: &str = "Analiza el documento sin inventar. Propón metadatos, nunca permisos, colección ni publicación. Usa un título de hasta 10 palabras, descripción de hasta 20 palabras, resumen de hasta 45 palabras y explicación de hasta 12 palabras. Propón entre 3 y 5 tags breves; incluye como máximo 3 entidades y 2 enlaces, solo si aparecen explícitamente. Omite términos genéricos. Devuelve solamente un objeto JSON compacto, sin Markdown ni razonamiento.";

#[async_trait]
pub trait GenerationProvider: Send + Sync {
    fn model_id(&self) -> &str;
    async fn enrich(&self, document_text: &str) -> Result<EnrichmentDraft>;
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn model_id(&self) -> &str;
    fn dimensions(&self) -> usize {
        EMBEDDING_DIMENSIONS
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// OpenAI-compatible client for a llama.cpp sidecar bound to loopback.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationRuntimeConfig {
    pub model_id: String,
    pub temperature: f32,
    pub max_input_tokens: usize,
    pub max_output_tokens: usize,
    pub thinking_directive: Option<String>,
}

impl GenerationRuntimeConfig {
    /// Safe defaults for a model that does not use Qwen-specific directives.
    pub fn for_model(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            temperature: 0.1,
            max_input_tokens: MAX_GENERATION_INPUT_TOKENS,
            max_output_tokens: MAX_GENERATION_OUTPUT_TOKENS,
            thinking_directive: None,
        }
    }

    /// Compatibility configuration for the model used by the original MVP.
    pub fn legacy_qwen() -> Self {
        Self {
            thinking_directive: Some("/no_think".to_owned()),
            ..Self::for_model("qwen3-1.7b-q8")
        }
    }

    fn validate(&self) -> Result<()> {
        if self.model_id.trim().is_empty() {
            bail!("generation model ID must not be empty");
        }
        if !self.temperature.is_finite() || !(0.0..=2.0).contains(&self.temperature) {
            bail!("generation temperature must be finite and between 0 and 2");
        }
        if self.max_input_tokens == 0 {
            bail!("generation input token limit must be positive");
        }
        if self.max_output_tokens == 0 || self.max_output_tokens > MAX_GENERATION_OUTPUT_TOKENS {
            bail!(
                "generation output token limit must be between 1 and {MAX_GENERATION_OUTPUT_TOKENS}"
            );
        }
        if self
            .max_input_tokens
            .checked_add(self.max_output_tokens)
            .is_none_or(|total| total > GENERATION_CONTEXT_TOKENS)
        {
            bail!(
                "generation input and output token limits exceed the {GENERATION_CONTEXT_TOKENS}-token context"
            );
        }
        if let Some(directive) = self.thinking_directive.as_deref() {
            if directive != "/no_think" {
                bail!("unsupported generation thinking directive");
            }
            if !self.model_id.to_ascii_lowercase().contains("qwen") {
                bail!("/no_think may only be configured for a Qwen model");
            }
        }
        Ok(())
    }
}

impl Default for GenerationRuntimeConfig {
    fn default() -> Self {
        Self::legacy_qwen()
    }
}

#[derive(Debug, Clone)]
pub struct LlamaServerProvider {
    client: reqwest::Client,
    endpoint: String,
    config: GenerationRuntimeConfig,
    request_timeout: Duration,
}

impl LlamaServerProvider {
    pub fn new(endpoint: impl Into<String>, bearer_token: &str) -> Result<Self> {
        Self::with_config(endpoint, bearer_token, GenerationRuntimeConfig::default())
    }

    pub fn with_config(
        endpoint: impl Into<String>,
        bearer_token: &str,
        config: GenerationRuntimeConfig,
    ) -> Result<Self> {
        let request_timeout = recommended_request_timeout(&config);
        Self::with_config_and_timeout(endpoint, bearer_token, config, request_timeout)
    }

    /// Builds a provider with an explicit per-request deadline. Production
    /// callers normally use [`Self::with_config`], which derives a conservative
    /// deadline from the selected model and output budget. The override keeps
    /// the transport independently testable and permits hardware-specific
    /// tuning without changing the inference protocol.
    pub fn with_config_and_timeout(
        endpoint: impl Into<String>,
        bearer_token: &str,
        config: GenerationRuntimeConfig,
        request_timeout: Duration,
    ) -> Result<Self> {
        let endpoint = endpoint.into().trim_end_matches('/').to_owned();
        if !(endpoint.starts_with("http://127.0.0.1:") || endpoint.starts_with("http://localhost:"))
        {
            bail!("llama-server endpoint must use loopback HTTP");
        }
        if bearer_token.is_empty() {
            bail!("llama-server bearer token must not be empty");
        }
        if request_timeout.is_zero() {
            bail!("llama-server request timeout must be positive");
        }
        config.validate()?;
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {bearer_token}"))?,
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            endpoint,
            config,
            request_timeout,
        })
    }

    fn completion_body(&self, system: &str, user: &str, schema: Value) -> Value {
        let user = self.config.thinking_directive.as_deref().map_or_else(
            || user.to_owned(),
            |directive| format!("{directive}\n{user}"),
        );
        json!({
            "model": self.config.model_id,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "temperature": self.config.temperature,
            "max_tokens": self.config.max_output_tokens,
            "stream": false,
            "response_format": {
                "type": "json_schema",
                "json_schema": {"name": "airwiki_enrichment", "strict": true, "schema": schema}
            }
        })
    }

    async fn completion(&self, system: &str, user: &str, schema: Value) -> Result<String> {
        let body = self.completion_body(system, user, schema);
        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.endpoint))
            .timeout(self.request_timeout)
            .json(&body)
            .send()
            .await
            .map_err(|error| self.request_error("request", error))?
            .error_for_status()
            .context("llama-server rejected the request")?;
        let value: Value = response
            .json()
            .await
            .map_err(|error| self.request_error("response body", error))?;
        parse_completion_response(&value, self.config.max_output_tokens)
    }

    fn request_error(&self, stage: &str, error: reqwest::Error) -> anyhow::Error {
        let model_id = &self.config.model_id;
        if error.is_timeout() {
            anyhow::Error::new(error).context(format!(
                "llama-server {stage} for model {model_id} timed out after {}",
                format_duration(self.request_timeout)
            ))
        } else {
            anyhow::Error::new(error)
                .context(format!("llama-server {stage} failed for model {model_id}"))
        }
    }

    async fn summarize_piece(&self, text: &str) -> Result<String> {
        let content = self
            .completion(
                SUMMARY_SYSTEM_PROMPT,
                text,
                json!({
                    "type": "object",
                    "properties": {"summary": {"type": "string", "maxLength": MAX_SUMMARY_CHARS}},
                    "required": ["summary"],
                    "additionalProperties": false
                }),
            )
            .await?;
        let value = parse_json_content(&content)?;
        value
            .get("summary")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| anyhow!("summary response did not include summary"))
    }

    async fn bounded_input(&self, document_text: &str) -> Result<String> {
        if approximate_generation_tokens(document_text) <= self.config.max_input_tokens {
            return Ok(document_text.to_owned());
        }
        let summary_batch_tokens = self.config.max_input_tokens.min(2_400);
        let mut summaries = Vec::new();
        for piece in split_for_generation(document_text, summary_batch_tokens) {
            summaries.push(self.summarize_piece(&piece).await?);
        }

        // Reduce every branch in batches until the complete hierarchy fits.
        // Never truncate the concatenation: later pages can contain the facts
        // that determine type, ownership, dates or tags.
        for _ in 0..16 {
            let combined = summaries.join("\n\n");
            if approximate_generation_tokens(&combined) <= self.config.max_input_tokens {
                return Ok(combined);
            }
            let batches = pack_generation_batches(&summaries, summary_batch_tokens);
            let mut reduced = Vec::with_capacity(batches.len());
            for batch in batches {
                reduced.push(self.summarize_piece(&batch).await?);
            }
            summaries = reduced;
        }
        bail!("hierarchical summary did not converge within the safety limit")
    }
}

fn recommended_request_timeout(config: &GenerationRuntimeConfig) -> Duration {
    // This is a deadline rather than an expected latency. Routed models can
    // emit structured JSON materially more slowly under memory pressure,
    // especially on an 8 GiB Windows node. Scale the allowance with the maximum
    // permitted output while retaining finite lower and upper bounds.
    let model_id = config.model_id.to_ascii_lowercase();
    let millis_per_output_token = if model_id.contains("e4b") {
        750_u64
    } else if model_id.contains("e2b") || model_id.contains("gemma") {
        500
    } else {
        250
    };
    let output_allowance = Duration::from_millis(
        u64::try_from(config.max_output_tokens)
            .unwrap_or(u64::MAX)
            .saturating_mul(millis_per_output_token),
    );
    GENERATION_REQUEST_BASE_TIMEOUT
        .saturating_add(output_allowance)
        .clamp(
            MIN_GENERATION_REQUEST_TIMEOUT,
            MAX_GENERATION_REQUEST_TIMEOUT,
        )
}

fn format_duration(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{} s", duration.as_secs())
    } else {
        format!("{} ms", duration.as_millis())
    }
}

// GGUF tokenization is byte-backed, so UTF-8 byte length is a safe upper bound
// without shipping a second Qwen tokenizer. It deliberately overestimates
// Latin text while remaining conservative for CJK, emoji and source code.
fn approximate_generation_tokens(text: &str) -> usize {
    text.len()
}

fn split_for_generation(text: &str, max_tokens: usize) -> Vec<String> {
    let max_bytes = max_tokens.max(1);
    let mut pieces = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if !current.is_empty() && current.len() + character.len_utf8() > max_bytes {
            pieces.push(std::mem::take(&mut current));
        }
        current.push(character);
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

fn pack_generation_batches(items: &[String], max_tokens: usize) -> Vec<String> {
    let mut batches = Vec::new();
    let mut current = String::new();
    for item in items {
        for piece in split_for_generation(item, max_tokens) {
            let candidate = if current.is_empty() {
                piece.clone()
            } else {
                format!("{current}\n\n{piece}")
            };
            if !current.is_empty() && approximate_generation_tokens(&candidate) > max_tokens {
                batches.push(std::mem::take(&mut current));
                current = piece;
            } else {
                current = candidate;
            }
        }
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

#[async_trait]
impl GenerationProvider for LlamaServerProvider {
    fn model_id(&self) -> &str {
        &self.config.model_id
    }

    async fn enrich(&self, document_text: &str) -> Result<EnrichmentDraft> {
        let bounded = self.bounded_input(document_text).await?;
        let content = self
            .completion(ENRICHMENT_SYSTEM_PROMPT, &bounded, enrichment_schema())
            .await?;
        let mut draft: EnrichmentDraft = serde_json::from_value(parse_json_content(&content)?)
            .context("LLM enrichment did not match the required schema")?;
        draft.sanitize();
        draft.tags.truncate(MAX_TAGS);
        draft.entities.truncate(MAX_ENTITIES);
        draft.links.truncate(MAX_LINKS);
        draft.summary = draft.summary.chars().take(MAX_SUMMARY_CHARS).collect();
        Ok(draft)
    }
}

fn enrichment_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "type": {"type": "string", "enum": ["Document", "Policy", "Procedure", "Runbook", "Reference", "Report"]},
            "title": {"type": "string", "maxLength": MAX_TITLE_CHARS},
            "description": {"type": "string", "maxLength": MAX_DESCRIPTION_CHARS},
            "language": {"type": "string", "maxLength": MAX_LANGUAGE_CHARS},
            "tags": {"type": "array", "maxItems": MAX_TAGS, "items": {"type": "string", "maxLength": MAX_TAG_CHARS}},
            "entities": {"type": "array", "maxItems": MAX_ENTITIES, "items": {"type": "object", "properties": {"name": {"type": "string", "maxLength": MAX_ENTITY_NAME_CHARS}, "kind": {"type": "string", "maxLength": MAX_ENTITY_KIND_CHARS}}, "required": ["name", "kind"], "additionalProperties": false}},
            "links": {"type": "array", "maxItems": MAX_LINKS, "items": {"type": "object", "properties": {"label": {"type": "string", "maxLength": MAX_LINK_LABEL_CHARS}, "target": {"type": "string", "maxLength": MAX_LINK_TARGET_CHARS}}, "required": ["label", "target"], "additionalProperties": false}},
            "summary": {"type": "string", "maxLength": MAX_SUMMARY_CHARS},
            "classification_confidence": {"type": "number", "minimum": 0, "maximum": 1},
            "classification_explanation": {"type": "string", "maxLength": MAX_CLASSIFICATION_EXPLANATION_CHARS}
        },
        "required": ["type", "title", "description", "language", "tags", "entities", "links", "summary", "classification_confidence", "classification_explanation"],
        "additionalProperties": false
    })
}

fn parse_completion_response(value: &Value, max_output_tokens: usize) -> Result<String> {
    let choice = value
        .pointer("/choices/0")
        .ok_or_else(|| anyhow!("llama-server response contained no completion choice"))?;
    if choice.get("finish_reason").and_then(Value::as_str) == Some("length") {
        bail!(
            "llama-server response was truncated at the configured maximum of {max_output_tokens} output tokens (finish_reason=length)"
        );
    }
    choice
        .pointer("/message/content")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("llama-server response contained no assistant content"))
}

fn parse_json_content(content: &str) -> Result<Value> {
    let without_thought = if let Some(end) = content.rfind("</think>") {
        &content[end + "</think>".len()..]
    } else {
        content
    };
    let trimmed = without_thought.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .strip_suffix("```")
        .unwrap_or(trimmed)
        .trim();
    serde_json::from_str(trimmed).context("LLM returned invalid JSON")
}

#[derive(Debug, Default, Clone)]
pub struct DeterministicGenerationProvider;

#[async_trait]
impl GenerationProvider for DeterministicGenerationProvider {
    fn model_id(&self) -> &str {
        "deterministic-test-generator"
    }

    async fn enrich(&self, document_text: &str) -> Result<EnrichmentDraft> {
        let normalized = document_text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if normalized.is_empty() {
            bail!("cannot enrich an empty document");
        }
        let first_line = document_text
            .lines()
            .find(|line| !line.trim().is_empty())
            .context("cannot enrich an empty document")?;
        let title = first_line.trim().trim_start_matches('#').trim();
        let lowercase = normalized.to_lowercase();
        let concept_type = if lowercase.contains("paso ") || lowercase.contains("procedimiento") {
            ConceptType::Procedure
        } else if lowercase.contains("política") || lowercase.contains("policy") {
            ConceptType::Policy
        } else if lowercase.contains("incidente") || lowercase.contains("runbook") {
            ConceptType::Runbook
        } else {
            ConceptType::Document
        };
        let language = if [" el ", " la ", " de ", " para "]
            .iter()
            .any(|word| format!(" {lowercase} ").contains(word))
        {
            "es"
        } else {
            "en"
        };
        let mut frequencies = HashMap::<String, usize>::new();
        for word in lowercase.split(|character: char| !character.is_alphanumeric()) {
            if word.len() >= 5 {
                *frequencies.entry(word.to_owned()).or_default() += 1;
            }
        }
        let mut frequencies = frequencies.into_iter().collect::<Vec<_>>();
        frequencies.sort_by(|(word_a, count_a), (word_b, count_b)| {
            count_b.cmp(count_a).then_with(|| word_a.cmp(word_b))
        });
        Ok(EnrichmentDraft {
            concept_type,
            title: title.chars().take(120).collect(),
            description: normalized.chars().take(300).collect(),
            language: language.into(),
            tags: frequencies
                .into_iter()
                .take(10)
                .map(|(word, _)| word)
                .collect(),
            entities: Vec::new(),
            links: Vec::new(),
            summary: normalized.chars().take(1_000).collect(),
            classification_confidence: 0.5,
            classification_explanation: "Proveedor determinista para pruebas".into(),
        })
    }
}

/// Stable hash-projection embeddings suitable for repeatable tests and offline CI.
#[derive(Debug, Default, Clone)]
pub struct DeterministicEmbeddingProvider;

#[async_trait]
impl EmbeddingProvider for DeterministicEmbeddingProvider {
    fn model_id(&self) -> &str {
        "deterministic-e5-test-double"
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| deterministic_embedding(text))
            .collect())
    }
}

fn deterministic_embedding(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    for word in text
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        let digest = Sha256::digest(word.as_bytes());
        let index = usize::from(u16::from_le_bytes([digest[0], digest[1]])) % vector.len();
        let sign = if digest[2] & 1 == 0 { 1.0 } else { -1.0 };
        vector[index] += sign;
    }
    normalize(&mut vector);
    vector
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        vector.iter_mut().for_each(|value| *value /= norm);
    }
}

#[cfg(feature = "fastembed-runtime")]
pub mod fastembed_provider {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use anyhow::{Context, Result, anyhow, bail};
    use async_trait::async_trait;
    use fastembed::{
        InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
    };
    use sha2::{Digest, Sha256};
    use thiserror::Error;

    use super::{E5_MODEL_REVISION, EmbeddingProvider};
    use crate::EMBEDDING_DIMENSIONS;
    use crate::ingest::Tokenizer;

    const EMBEDDING_DEADLINE_BATCH_SIZE: usize = 8;
    const EMBEDDING_DEADLINE_PER_BATCH: Duration = Duration::from_millis(750);
    const MAX_EMBEDDING_DEADLINE: Duration = Duration::from_secs(30);

    #[derive(Debug, Clone, Copy)]
    struct SnapshotAsset {
        relative_path: &'static str,
        sha256: &'static str,
    }

    // SECURITY: these hashes intentionally duplicate airwiki-inference's
    // download manifest. airwiki-core is the final trust boundary before ONNX
    // Runtime and cannot depend on the outer inference crate in production.
    // A cross-crate test below requires both manifests to change together.
    const E5_ONNX_ASSET: SnapshotAsset = SnapshotAsset {
        relative_path: "onnx/model.onnx",
        sha256: "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665",
    };
    const E5_TOKENIZER_ASSET: SnapshotAsset = SnapshotAsset {
        relative_path: "tokenizer.json",
        sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    };
    const E5_CONFIG_ASSET: SnapshotAsset = SnapshotAsset {
        relative_path: "config.json",
        sha256: "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959",
    };
    const E5_SPECIAL_TOKENS_ASSET: SnapshotAsset = SnapshotAsset {
        relative_path: "special_tokens_map.json",
        sha256: "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7",
    };
    const E5_TOKENIZER_CONFIG_ASSET: SnapshotAsset = SnapshotAsset {
        relative_path: "tokenizer_config.json",
        sha256: "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
    };
    const E5_SNAPSHOT_ASSETS: [SnapshotAsset; 5] = [
        E5_ONNX_ASSET,
        E5_TOKENIZER_ASSET,
        E5_CONFIG_ASSET,
        E5_SPECIAL_TOKENS_ASSET,
        E5_TOKENIZER_CONFIG_ASSET,
    ];

    /// Sanitized failures while opening or reading the pinned E5 snapshot.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
    pub enum E5SnapshotLoadError {
        #[error("pinned multilingual-e5-small snapshot directory is unavailable")]
        SnapshotUnavailable,
        #[error("pinned multilingual-e5-small revision marker is unavailable")]
        RevisionMarkerUnavailable,
        #[error("pinned multilingual-e5-small revision does not match")]
        RevisionMismatch,
        #[error("pinned multilingual-e5-small asset is unavailable: {asset}")]
        AssetUnavailable { asset: &'static str },
        #[error("pinned multilingual-e5-small asset is not a regular file: {asset}")]
        AssetNotRegular { asset: &'static str },
        #[error("pinned multilingual-e5-small asset failed integrity verification: {asset}")]
        IntegrityMismatch { asset: &'static str },
    }

    async fn run_fastembed<T, F>(operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        tokio::task::spawn_blocking(operation)
            .await
            .context("embedding worker task failed")?
    }

    async fn run_fastembed_serialized<T, F>(
        inference_permit: Arc<tokio::sync::Semaphore>,
        deadline: Duration,
        operation: F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        let inference = async move {
            let permit = inference_permit
                .acquire_owned()
                .await
                .map_err(|_| anyhow!("embedding inference is unavailable"))?;
            run_fastembed(move || {
                // Tokio cannot cancel a running blocking job. Keep the permit
                // inside it so a timed-out inference still serializes the ONNX
                // session until the native call actually returns.
                let _permit = permit;
                operation()
            })
            .await
        };
        tokio::time::timeout(deadline, inference)
            .await
            .map_err(|_| anyhow!("embedding inference timed out"))?
    }

    fn embedding_deadline(text_count: usize) -> Duration {
        let batches = text_count.max(1).div_ceil(EMBEDDING_DEADLINE_BATCH_SIZE);
        let multiplier = u32::try_from(batches).unwrap_or(u32::MAX);
        EMBEDDING_DEADLINE_PER_BATCH
            .saturating_mul(multiplier)
            .min(MAX_EMBEDDING_DEADLINE)
    }

    /// A fully materialized Hugging Face snapshot. The constructor refuses an
    /// unpinned directory, so the runtime can never silently fetch model HEAD.
    #[derive(Clone)]
    pub struct PinnedE5Snapshot {
        root: PathBuf,
    }

    impl std::fmt::Debug for PinnedE5Snapshot {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("PinnedE5Snapshot")
                .field("revision", &E5_MODEL_REVISION)
                .finish_non_exhaustive()
        }
    }

    impl PinnedE5Snapshot {
        pub fn open(root: impl AsRef<Path>) -> std::result::Result<Self, E5SnapshotLoadError> {
            let root = root.as_ref().to_path_buf();
            let metadata = std::fs::symlink_metadata(&root)
                .map_err(|_| E5SnapshotLoadError::SnapshotUnavailable)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(E5SnapshotLoadError::SnapshotUnavailable);
            }
            let directory_is_revision = root
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == E5_MODEL_REVISION);
            if !directory_is_revision {
                return Err(E5SnapshotLoadError::RevisionMismatch);
            }

            let marker_path = root.join("revision.txt");
            let marker_metadata = std::fs::symlink_metadata(&marker_path)
                .map_err(|_| E5SnapshotLoadError::RevisionMarkerUnavailable)?;
            if marker_metadata.file_type().is_symlink() || !marker_metadata.is_file() {
                return Err(E5SnapshotLoadError::RevisionMarkerUnavailable);
            }
            let marker = std::fs::read(marker_path)
                .map_err(|_| E5SnapshotLoadError::RevisionMarkerUnavailable)?;
            let expected_marker = format!("{E5_MODEL_REVISION}\n");
            if marker != expected_marker.as_bytes() {
                return Err(E5SnapshotLoadError::RevisionMismatch);
            }

            for asset in E5_SNAPSHOT_ASSETS {
                verify_asset_metadata(&root, asset)?;
            }
            Ok(Self { root })
        }

        pub fn root(&self) -> &Path {
            &self.root
        }

        fn read(&self, asset: SnapshotAsset) -> std::result::Result<Vec<u8>, E5SnapshotLoadError> {
            verify_asset_metadata(&self.root, asset)?;
            let bytes = std::fs::read(self.root.join(asset.relative_path)).map_err(|_| {
                E5SnapshotLoadError::AssetUnavailable {
                    asset: asset.relative_path,
                }
            })?;
            let actual = hex::encode(Sha256::digest(&bytes));
            if actual != asset.sha256 {
                return Err(E5SnapshotLoadError::IntegrityMismatch {
                    asset: asset.relative_path,
                });
            }
            Ok(bytes)
        }
    }

    fn verify_asset_metadata(
        root: &Path,
        asset: SnapshotAsset,
    ) -> std::result::Result<(), E5SnapshotLoadError> {
        let metadata = std::fs::symlink_metadata(root.join(asset.relative_path)).map_err(|_| {
            E5SnapshotLoadError::AssetUnavailable {
                asset: asset.relative_path,
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(E5SnapshotLoadError::AssetNotRegular {
                asset: asset.relative_path,
            });
        }
        Ok(())
    }

    pub struct FastEmbedE5Small {
        model: Arc<Mutex<TextEmbedding>>,
        inference_permit: Arc<tokio::sync::Semaphore>,
    }

    impl FastEmbedE5Small {
        pub fn from_snapshot(snapshot: &PinnedE5Snapshot, intra_threads: usize) -> Result<Self> {
            if intra_threads == 0 {
                bail!("embedding intra_threads must be positive");
            }
            let tokenizer_files = TokenizerFiles {
                tokenizer_file: snapshot.read(E5_TOKENIZER_ASSET)?,
                config_file: snapshot.read(E5_CONFIG_ASSET)?,
                special_tokens_map_file: snapshot.read(E5_SPECIAL_TOKENS_ASSET)?,
                tokenizer_config_file: snapshot.read(E5_TOKENIZER_CONFIG_ASSET)?,
            };
            let model =
                UserDefinedEmbeddingModel::new(snapshot.read(E5_ONNX_ASSET)?, tokenizer_files)
                    .with_pooling(Pooling::Mean);
            let options = InitOptionsUserDefined::new()
                .with_max_length(512)
                .with_intra_threads(intra_threads);
            Ok(Self {
                model: Arc::new(Mutex::new(
                    TextEmbedding::try_new_from_user_defined(model, options).map_err(|_| {
                        anyhow!("could not initialize pinned multilingual-e5-small")
                    })?,
                )),
                inference_permit: Arc::new(tokio::sync::Semaphore::new(1)),
            })
        }
    }

    #[async_trait]
    impl EmbeddingProvider for FastEmbedE5Small {
        fn model_id(&self) -> &str {
            "multilingual-e5-small@614241f622f53c4eeff9890bdc4f31cfecc418b3"
        }

        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            if texts.is_empty() {
                return Ok(Vec::new());
            }
            let model = Arc::clone(&self.model);
            let inference_permit = Arc::clone(&self.inference_permit);
            let texts = texts.to_vec();
            let deadline = embedding_deadline(texts.len());
            // Ingestion and queries intentionally share one FIFO permit. The
            // MVP has no separate priority queue: a query waiting behind an
            // active ingestion batch fails closed at its shorter deadline.
            run_fastembed_serialized(inference_permit, deadline, move || {
                let mut model = model
                    .lock()
                    .map_err(|_| anyhow!("embedding model lock poisoned"))?;
                let embeddings = model
                    .embed(texts, None)
                    .map_err(|_| anyhow!("multilingual-e5-small inference failed"))?;
                if embeddings
                    .iter()
                    .any(|embedding| embedding.len() != EMBEDDING_DIMENSIONS)
                {
                    return Err(anyhow!(
                        "unexpected multilingual-e5-small embedding dimensions"
                    ));
                }
                Ok(embeddings)
            })
            .await
        }
    }

    /// Exact tokenizer used by the pinned embeddings snapshot. Encoded IDs are
    /// represented internally as decimal strings to satisfy the core's runtime-
    /// independent tokenizer interface; callers only observe decoded chunks.
    pub struct E5Tokenizer {
        tokenizer: tokenizers::Tokenizer,
    }

    impl E5Tokenizer {
        pub fn from_snapshot(snapshot: &PinnedE5Snapshot) -> Result<Self> {
            let tokenizer_bytes = snapshot.read(E5_TOKENIZER_ASSET)?;
            let tokenizer = tokenizers::Tokenizer::from_bytes(tokenizer_bytes).map_err(|_| {
                anyhow!("could not initialize pinned multilingual-e5-small tokenizer")
            })?;
            Ok(Self { tokenizer })
        }
    }

    impl Tokenizer for E5Tokenizer {
        fn encode(&self, text: &str) -> Result<Vec<String>> {
            let encoded = self
                .tokenizer
                .encode(text, false)
                .map_err(|error| anyhow!(error.to_string()))?;
            Ok(encoded.get_ids().iter().map(u32::to_string).collect())
        }

        fn decode(&self, tokens: &[String]) -> Result<String> {
            let ids = tokens
                .iter()
                .map(|token| token.parse::<u32>().context("invalid tokenizer token ID"))
                .collect::<Result<Vec<_>>>()?;
            self.tokenizer
                .decode(&ids, true)
                .map_err(|error| anyhow!(error.to_string()))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn write_snapshot_fixture(root: &Path) {
            std::fs::create_dir_all(root.join("onnx")).unwrap();
            std::fs::write(root.join("revision.txt"), format!("{E5_MODEL_REVISION}\n")).unwrap();
            for asset in E5_SNAPSHOT_ASSETS {
                std::fs::write(root.join(asset.relative_path), []).unwrap();
            }
        }

        #[test]
        fn consumer_hashes_match_the_installer_manifest() {
            for asset in E5_SNAPSHOT_ASSETS {
                let installed = airwiki_inference::E5_FILES
                    .iter()
                    .find(|candidate| candidate.filename == asset.relative_path)
                    .unwrap();
                assert_eq!(asset.sha256, installed.sha256);
                assert_eq!(installed.revision, E5_MODEL_REVISION);
            }
        }

        #[test]
        fn snapshot_requires_the_revision_directory_and_exact_marker() {
            let temp = tempfile::tempdir().unwrap();
            let wrong_name = temp.path().join("wrong-revision");
            write_snapshot_fixture(&wrong_name);
            assert_eq!(
                PinnedE5Snapshot::open(&wrong_name).unwrap_err(),
                E5SnapshotLoadError::RevisionMismatch
            );

            let exact = temp.path().join(E5_MODEL_REVISION);
            write_snapshot_fixture(&exact);
            std::fs::write(exact.join("revision.txt"), E5_MODEL_REVISION).unwrap();
            assert_eq!(
                PinnedE5Snapshot::open(&exact).unwrap_err(),
                E5SnapshotLoadError::RevisionMismatch
            );
        }

        #[test]
        fn snapshot_read_hashes_the_same_bytes_it_returns() {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join(E5_MODEL_REVISION);
            write_snapshot_fixture(&root);
            let snapshot = PinnedE5Snapshot::open(&root).unwrap();

            assert_eq!(
                snapshot.read(E5_TOKENIZER_ASSET).unwrap_err(),
                E5SnapshotLoadError::IntegrityMismatch {
                    asset: "tokenizer.json"
                }
            );
            let tokenizer_error = E5Tokenizer::from_snapshot(&snapshot)
                .err()
                .expect("tampered tokenizer must be rejected");
            assert_eq!(
                tokenizer_error
                    .downcast_ref::<E5SnapshotLoadError>()
                    .copied(),
                Some(E5SnapshotLoadError::IntegrityMismatch {
                    asset: "tokenizer.json"
                })
            );
            assert!(!format!("{snapshot:?}").contains(&root.display().to_string()));
        }

        #[cfg(unix)]
        #[test]
        fn snapshot_rejects_symlinked_assets() {
            use std::os::unix::fs::symlink;

            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join(E5_MODEL_REVISION);
            write_snapshot_fixture(&root);
            let tokenizer = root.join(E5_TOKENIZER_ASSET.relative_path);
            std::fs::remove_file(&tokenizer).unwrap();
            symlink(root.join(E5_CONFIG_ASSET.relative_path), &tokenizer).unwrap();

            assert_eq!(
                PinnedE5Snapshot::open(root).unwrap_err(),
                E5SnapshotLoadError::AssetNotRegular {
                    asset: "tokenizer.json"
                }
            );
        }

        #[test]
        fn embedding_deadline_leaves_room_for_reranking_inside_the_lan_deadline() {
            assert_eq!(embedding_deadline(0), Duration::from_millis(750));
            assert_eq!(embedding_deadline(1), Duration::from_millis(750));
            assert_eq!(embedding_deadline(8), Duration::from_millis(750));
            assert_eq!(embedding_deadline(9), Duration::from_millis(1_500));
            assert_eq!(embedding_deadline(32), Duration::from_secs(3));
            assert_eq!(embedding_deadline(usize::MAX), MAX_EMBEDDING_DEADLINE);
        }

        #[tokio::test(flavor = "current_thread")]
        async fn fastembed_work_does_not_block_the_async_executor() {
            let started = std::time::Instant::now();
            let blocking = run_fastembed(|| {
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
                "blocking FastEmbed work stalled Tokio for {heartbeat_elapsed:?}"
            );
        }

        #[tokio::test(flavor = "current_thread")]
        async fn deadline_fails_closed_and_running_job_retains_its_permit() {
            let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
            let error =
                run_fastembed_serialized(Arc::clone(&semaphore), Duration::from_millis(5), || {
                    std::thread::sleep(Duration::from_millis(75));
                    Ok(())
                })
                .await
                .unwrap_err();

            assert_eq!(error.to_string(), "embedding inference timed out");
            assert_eq!(semaphore.available_permits(), 0);
            let released = tokio::time::timeout(
                Duration::from_secs(2),
                Arc::clone(&semaphore).acquire_owned(),
            )
            .await
            .expect("blocking embedding job did not release its permit")
            .unwrap();
            drop(released);
            assert_eq!(semaphore.available_permits(), 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_generation_is_structured_and_bounded() {
        let provider = DeterministicGenerationProvider;
        let draft = provider
            .enrich("# Política de pagos\nLa política define el proceso para pagos.")
            .await
            .unwrap();
        assert_eq!(draft.concept_type, ConceptType::Policy);
        assert_eq!(draft.language, "es");
        assert!(!draft.tags.is_empty());
    }

    #[tokio::test]
    async fn deterministic_embeddings_are_normalized_and_semantic_words_match() {
        let provider = DeterministicEmbeddingProvider;
        let vectors = provider
            .embed(&[
                "query: recuperar pagos".into(),
                "passage: recuperar pagos".into(),
            ])
            .await
            .unwrap();
        assert_eq!(vectors[0].len(), EMBEDDING_DIMENSIONS);
        let norm = vectors[0].iter().map(|value| value * value).sum::<f32>();
        assert!((norm - 1.0).abs() < 1e-5);
        let similarity = vectors[0]
            .iter()
            .zip(&vectors[1])
            .map(|(left, right)| left * right)
            .sum::<f32>();
        assert!(similarity > 0.5);
    }

    #[test]
    fn invalid_json_is_rejected() {
        assert!(parse_json_content("not json").is_err());
        assert_eq!(
            parse_json_content("```json\n{\"a\":1}\n```").unwrap()["a"],
            1
        );
    }

    #[test]
    fn remote_endpoint_must_be_loopback() {
        assert!(LlamaServerProvider::new("http://192.168.1.2:8080", "secret").is_err());
        assert!(LlamaServerProvider::new("http://127.0.0.1:8080", "secret").is_ok());
    }

    #[test]
    fn configured_provider_uses_the_selected_model_without_qwen_directives() {
        let provider = LlamaServerProvider::with_config(
            "http://127.0.0.1:8080",
            "secret",
            GenerationRuntimeConfig::for_model("gemma-4-e4b-q4"),
        )
        .unwrap();
        let body = provider.completion_body("system", "document", json!({"type": "object"}));

        assert_eq!(provider.model_id(), "gemma-4-e4b-q4");
        assert_eq!(body["model"], "gemma-4-e4b-q4");
        assert_eq!(body["messages"][1]["content"], "document");
        assert_eq!(body["max_tokens"], MAX_GENERATION_OUTPUT_TOKENS);
        assert!((body["temperature"].as_f64().unwrap() - 0.1).abs() < f64::from(f32::EPSILON));
    }

    #[test]
    fn request_deadline_scales_with_model_and_output_budget() {
        let mut e4b = GenerationRuntimeConfig::for_model("gemma-4-e4b-q4");
        e4b.max_output_tokens = 384;
        let mut e2b = GenerationRuntimeConfig::for_model("gemma-4-e2b-q4");
        e2b.max_output_tokens = 384;
        let mut qwen = GenerationRuntimeConfig::for_model("qwen3-1.7b-q8");
        qwen.max_output_tokens = 384;

        assert_eq!(recommended_request_timeout(&e4b), Duration::from_secs(408));
        assert_eq!(recommended_request_timeout(&e2b), Duration::from_secs(312));
        assert_eq!(recommended_request_timeout(&qwen), Duration::from_secs(216));

        qwen.max_output_tokens = 1;
        assert_eq!(
            recommended_request_timeout(&qwen),
            MIN_GENERATION_REQUEST_TIMEOUT
        );
    }

    #[tokio::test]
    async fn timeout_error_identifies_model_deadline_and_keeps_transport_cause() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let _connection = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(250));
        });
        let provider = LlamaServerProvider::with_config_and_timeout(
            format!("http://{address}"),
            "secret",
            GenerationRuntimeConfig::for_model("gemma-4-e4b-q4"),
            Duration::from_millis(25),
        )
        .unwrap();

        let error = provider
            .completion("system", "document", json!({"type": "object"}))
            .await
            .unwrap_err();
        let detailed = format!("{error:#}");

        assert!(detailed.contains("request for model gemma-4-e4b-q4 timed out after 25 ms"));
        assert!(
            error.chain().count() > 1,
            "missing reqwest cause: {detailed}"
        );
        assert_ne!(detailed, error.to_string());
        server.join().unwrap();
    }

    #[test]
    fn explicit_request_deadline_must_be_positive() {
        let error = LlamaServerProvider::with_config_and_timeout(
            "http://127.0.0.1:8080",
            "secret",
            GenerationRuntimeConfig::for_model("gemma-4-e4b-q4"),
            Duration::ZERO,
        )
        .unwrap_err();
        assert!(error.to_string().contains("timeout must be positive"));
    }

    #[test]
    fn legacy_qwen_request_is_the_only_default_with_no_think() {
        let provider = LlamaServerProvider::new("http://127.0.0.1:8080", "secret").unwrap();
        let body = provider.completion_body("system", "document", json!({"type": "object"}));
        assert_eq!(body["messages"][1]["content"], "/no_think\ndocument");

        let mut invalid = GenerationRuntimeConfig::for_model("gemma-4-e2b-q4");
        invalid.thinking_directive = Some("/no_think".to_owned());
        let error = LlamaServerProvider::with_config("http://127.0.0.1:8080", "secret", invalid)
            .unwrap_err();
        assert!(error.to_string().contains("only be configured for a Qwen"));
    }

    #[test]
    fn completion_length_is_reported_as_explicit_truncation() {
        let response = json!({
            "choices": [{
                "finish_reason": "length",
                "message": {"content": "{\"title\":\"truncated"}
            }]
        });
        let error = parse_completion_response(&response, MAX_GENERATION_OUTPUT_TOKENS).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("truncated"));
        assert!(message.contains("finish_reason=length"));
        assert!(message.contains("384"));
    }

    #[test]
    fn completed_response_content_is_accepted() {
        let response = json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {"content": "{\"summary\":\"ok\"}"}
            }]
        });
        assert_eq!(
            parse_completion_response(&response, MAX_GENERATION_OUTPUT_TOKENS).unwrap(),
            "{\"summary\":\"ok\"}"
        );
    }

    #[test]
    fn enrichment_schema_bounds_high_variance_output() {
        let schema = enrichment_schema();
        assert_eq!(MAX_GENERATION_OUTPUT_TOKENS, 384);
        assert_eq!(schema["properties"]["tags"]["maxItems"], MAX_TAGS);
        assert_eq!(schema["properties"]["tags"]["maxItems"], 5);
        assert_eq!(schema["properties"]["entities"]["maxItems"], 3);
        assert_eq!(schema["properties"]["links"]["maxItems"], MAX_LINKS);
        assert_eq!(schema["properties"]["links"]["maxItems"], 2);
        assert_eq!(
            schema["properties"]["summary"]["maxLength"],
            MAX_SUMMARY_CHARS
        );
        assert_eq!(
            schema["properties"]["description"]["maxLength"],
            MAX_DESCRIPTION_CHARS
        );
        assert_eq!(MAX_SUMMARY_CHARS, 360);
        assert_eq!(MAX_DESCRIPTION_CHARS, 180);
        assert!(ENRICHMENT_SYSTEM_PROMPT.contains("resumen de hasta 45 palabras"));
        assert!(SUMMARY_SYSTEM_PROMPT.contains("máximo de 70 palabras"));
    }

    #[test]
    fn runtime_rejects_output_budgets_above_the_structured_limit() {
        let mut config = GenerationRuntimeConfig::for_model("gemma-4-e4b-q4");
        config.max_output_tokens = MAX_GENERATION_OUTPUT_TOKENS + 1;
        let error = LlamaServerProvider::with_config("http://127.0.0.1:8080", "secret", config)
            .unwrap_err();
        assert!(error.to_string().contains("between 1 and 384"));
    }

    #[test]
    fn hierarchical_pieces_respect_the_conservative_token_budget() {
        let input = "á".repeat(25_001);
        assert!(approximate_generation_tokens(&input) > MAX_GENERATION_INPUT_TOKENS);
        let pieces = split_for_generation(&input, 2_400);
        assert!(pieces.len() > 1);
        assert!(
            pieces
                .iter()
                .all(|piece| approximate_generation_tokens(piece) <= 2_400)
        );
        assert_eq!(pieces.concat(), input);
    }

    #[test]
    fn utf8_byte_bound_is_safe_for_cjk_and_emoji() {
        let input = format!("{}{}", "界".repeat(2_801), "🧠".repeat(200));
        assert!(approximate_generation_tokens(&input) > MAX_GENERATION_INPUT_TOKENS);
        let pieces = split_for_generation(&input, MAX_GENERATION_INPUT_TOKENS);
        assert!(pieces.len() > 1);
        assert!(
            pieces
                .iter()
                .all(|piece| approximate_generation_tokens(piece) <= MAX_GENERATION_INPUT_TOKENS)
        );
        assert_eq!(pieces.concat(), input);
    }

    #[test]
    fn hierarchical_batches_cover_every_summary_without_prefix_truncation() {
        let summaries = (0..20)
            .map(|index| format!("BRANCH-{index:02} {}", "x".repeat(900)))
            .collect::<Vec<_>>();
        let batches = pack_generation_batches(&summaries, 1_000);

        assert!(batches.len() > 1);
        assert!(
            batches
                .iter()
                .all(|batch| approximate_generation_tokens(batch) <= 1_000)
        );
        let combined = batches.join("\n");
        for index in 0..20 {
            assert!(combined.contains(&format!("BRANCH-{index:02}")));
        }
    }
}
