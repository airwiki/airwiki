use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use crate::{GenerationSettings, LlamaEndpoint, STRUCTURED_OUTPUT_TOKENS, ThinkingControl};

const MAX_APPROX_INPUT_TOKENS: usize = 2_800;
const JSON_GRAMMAR: &str = r#"root ::= object
object ::= "{" ws (string ":" ws value ("," ws string ":" ws value)*)? "}" ws
value ::= object | array | string | number | ("true" | "false" | "null") ws
array ::= "[" ws (value ("," ws value)*)? "]" ws
string ::= "\"" ([^"\\] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F]{4}))* "\"" ws
number ::= ("-"? ([0-9] | [1-9] [0-9]*) ("." [0-9]+)? ([eE] [+-]? [0-9]+)?) ws
ws ::= ([ \t\n] ws)?"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationConfig {
    pub model_id: String,
    pub max_input_tokens: usize,
    pub max_output_tokens: u16,
    pub temperature: f32,
    pub thinking_control: ThinkingControl,
    pub timeout: Duration,
}

impl Default for GenerationConfig {
    fn default() -> Self {
        Self {
            model_id: "qwen3-1.7b-q8".to_owned(),
            max_input_tokens: MAX_APPROX_INPUT_TOKENS,
            max_output_tokens: STRUCTURED_OUTPUT_TOKENS,
            temperature: 0.1,
            thinking_control: ThinkingControl::NoThinkDirective,
            timeout: Duration::from_secs(180),
        }
    }
}

impl GenerationConfig {
    pub fn from_settings(settings: GenerationSettings) -> Self {
        Self {
            model_id: settings.model_api_id.to_owned(),
            max_input_tokens: settings.max_input_tokens as usize,
            max_output_tokens: settings.max_output_tokens,
            temperature: settings.temperature,
            thinking_control: settings.thinking_control,
            timeout: Duration::from_secs(180),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.model_id.trim().is_empty() {
            bail!("generation model ID must not be empty");
        }
        if self.max_input_tokens == 0 || self.max_input_tokens > MAX_APPROX_INPUT_TOKENS {
            bail!("generation input limit must be between 1 and {MAX_APPROX_INPUT_TOKENS}");
        }
        if self.max_output_tokens == 0 || self.max_output_tokens > STRUCTURED_OUTPUT_TOKENS {
            bail!("generation output limit must be between 1 and {STRUCTURED_OUTPUT_TOKENS}");
        }
        if !self.temperature.is_finite() || !(0.0..=2.0).contains(&self.temperature) {
            bail!("generation temperature must be finite and between 0 and 2");
        }
        if self.thinking_control == ThinkingControl::NoThinkDirective
            && !self.model_id.to_ascii_lowercase().contains("qwen")
        {
            bail!("/no_think may only be configured for a Qwen model");
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct LlamaClient {
    endpoint: LlamaEndpoint,
    http: reqwest::Client,
    concurrency: Arc<Semaphore>,
    config: GenerationConfig,
}

impl LlamaClient {
    pub fn new(endpoint: LlamaEndpoint, config: GenerationConfig) -> Result<Self> {
        config.validate()?;
        let http = reqwest::Client::builder().timeout(config.timeout).build()?;
        Ok(Self {
            endpoint,
            http,
            concurrency: Arc::new(Semaphore::new(1)),
            config,
        })
    }

    pub async fn complete_json(&self, system: &str, input: &str) -> Result<Value> {
        // This conservative bound avoids accidentally overflowing the 4096-token context. Exact
        // chunking uses the embedding tokenizer upstream; generation still refuses oversized input.
        let approximate_tokens = input.chars().count().div_ceil(3);
        if approximate_tokens > self.config.max_input_tokens {
            bail!(
                "generation input exceeds the {}-token safety limit",
                self.config.max_input_tokens
            );
        }
        let _permit = self.concurrency.acquire().await?;
        let system = format!("{system}\nReturn only one JSON object.");
        let input = match self.config.thinking_control {
            ThinkingControl::None => input.to_owned(),
            ThinkingControl::NoThinkDirective => format!("/no_think\n{input}"),
        };
        let body = json!({
            "model": self.config.model_id,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": input}
            ],
            "temperature": self.config.temperature,
            "max_tokens": self.config.max_output_tokens,
            "stream": false,
            "grammar": JSON_GRAMMAR
        });
        let response = self
            .http
            .post(format!("{}/v1/chat/completions", self.endpoint.base_url))
            .bearer_auth(self.endpoint.bearer_token())
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<CompletionResponse>()
            .await?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .context("llama-server returned no completion choice")?;
        if choice.finish_reason.as_deref() == Some("length") {
            bail!(
                "llama-server response was truncated at {} output tokens",
                self.config.max_output_tokens
            );
        }
        let raw = choice.message.content;
        let clean = strip_wrappers(&raw);
        serde_json::from_str(clean).with_context(|| "llama-server returned invalid JSON")
    }
}

fn strip_wrappers(raw: &str) -> &str {
    let mut value = raw.trim();
    if let Some(end) = value.find("</think>") {
        value = value[end + "</think>".len()..].trim();
    }
    value = value.strip_prefix("```json").unwrap_or(value).trim();
    value = value.strip_prefix("```").unwrap_or(value).trim();
    value = value.strip_suffix("```").unwrap_or(value).trim();
    value
}

#[derive(Debug, Deserialize)]
struct CompletionResponse {
    choices: Vec<CompletionChoice>,
}

#[derive(Debug, Deserialize)]
struct CompletionChoice {
    message: CompletionMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CompletionMessage {
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_thinking_and_markdown_fences() {
        let raw = "<think>hidden</think>\n```json\n{\"ok\":true}\n```";
        assert_eq!(strip_wrappers(raw), "{\"ok\":true}");
    }

    #[test]
    fn rejects_qwen_thinking_directive_for_other_models() {
        let config = GenerationConfig {
            model_id: "gemma-4-e2b-q4".into(),
            ..GenerationConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn structured_generation_budget_is_capped_at_384_tokens() {
        let config = GenerationConfig::default();
        assert_eq!(config.max_output_tokens, STRUCTURED_OUTPUT_TOKENS);

        let oversized = GenerationConfig {
            max_output_tokens: STRUCTURED_OUTPUT_TOKENS + 1,
            ..config
        };
        let error = oversized.validate().unwrap_err();
        assert!(error.to_string().contains("between 1 and 384"));
    }
}
