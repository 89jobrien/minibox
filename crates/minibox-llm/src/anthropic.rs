use async_trait::async_trait;

use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse, Usage};

/// LLM provider backed by the Anthropic Messages API.
///
/// Enabled by the `anthropic` feature flag. Uses the
/// `https://api.anthropic.com/v1/messages` endpoint with
/// `anthropic-version: 2023-06-01`.
///
/// Default model: `claude-sonnet-4-6`. Construct via [`from_env`](AnthropicProvider::from_env)
/// (reads `ANTHROPIC_API_KEY`) or [`new`](AnthropicProvider::new) / [`with_config`](AnthropicProvider::with_config)
/// when the key is already available.
///
/// Structured output is implemented via Anthropic's tool-use mechanism: the
/// schema is registered as a tool and `tool_choice` forces the model to call it,
/// yielding the `input` field of the resulting `tool_use` block as the response text.
pub struct AnthropicProvider {
    key: String,
    model: String,
    /// Display name returned by [`name`](AnthropicProvider::name), e.g. `"anthropic/claude-sonnet-4-6"`.
    display_name: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    /// Construct with default HTTP timeouts (10s connect, 60s request).
    pub fn new(key: String, model: String) -> Self {
        Self::with_config(key, model, &crate::ProviderConfig::default())
    }

    /// Construct with explicit HTTP timeout configuration.
    ///
    /// The `reqwest::Client` is built once here and reused for every request
    /// made by this provider instance.
    pub fn with_config(key: String, model: String, config: &crate::ProviderConfig) -> Self {
        let display_name = format!("anthropic/{model}");
        let client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .expect("failed to build reqwest client");
        Self {
            key,
            model,
            display_name,
            client,
        }
    }
}

// Generates from_env(), from_env_with_config(), and from_key() (test-only).
// Reads ANTHROPIC_API_KEY; default model is claude-sonnet-4-6.
minibox_macros::provide!(AnthropicProvider, "ANTHROPIC_API_KEY", "claude-sonnet-4-6");

#[async_trait]
impl LlmProvider for AnthropicProvider {
    /// Returns the display name, e.g. `"anthropic/claude-sonnet-4-6"`.
    fn name(&self) -> &str {
        &self.display_name
    }

    /// Send a completion request to the Anthropic Messages API.
    ///
    /// When [`CompletionRequest::schema`] is set, the schema is sent as an
    /// Anthropic tool definition and `tool_choice` forces the model to call it.
    /// The returned text is the JSON-serialised `input` field of the
    /// `tool_use` response block.
    ///
    /// When [`CompletionRequest::timeout`] is set, it overrides the client-level
    /// request timeout for this call only.
    ///
    /// Non-2xx responses are wrapped in [`HttpStatusError`](crate::HttpStatusError)
    /// with the `error.message` JSON field extracted when possible.
    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let start = std::time::Instant::now();
        tracing::debug!(
            provider = self.name(),
            model = %self.model,
            max_tokens = request.max_tokens,
            schema = request.schema.as_ref().map(|s| s.name.as_str()),
            "llm: sending request"
        );

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "messages": [{"role": "user", "content": &request.prompt}],
        });

        if let Some(system) = &request.system {
            body["system"] = serde_json::json!(system);
        }

        if let Some(schema) = &request.schema {
            // Use tool-use to force structured JSON output.
            body["tools"] = serde_json::json!([{
                "name": &schema.name,
                "description": "Respond with structured output",
                "input_schema": &schema.schema,
            }]);
            body["tool_choice"] = serde_json::json!({
                "type": "tool",
                "name": &schema.name,
            });
        }

        let mut req = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body);

        if let Some(t) = request.timeout {
            req = req.timeout(t);
        }

        let resp = req.send().await.map_err(|e| LlmError::ProviderError {
            provider: self.name().to_string(),
            source: Box::new(e),
        })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(String::from))
                .unwrap_or_else(|| body.clone());
            return Err(LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(crate::error::HttpStatusError {
                    status: status.as_u16(),
                    body: message,
                }),
            });
        }

        let resp_body: serde_json::Value =
            resp.json().await.map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;

        let text = if request.schema.is_some() {
            // Extract the tool_use block's input as JSON text.
            resp_body["content"]
                .as_array()
                .and_then(|blocks| blocks.iter().find(|b| b["type"] == "tool_use"))
                .map(|b| b["input"].to_string())
                .ok_or_else(|| {
                    LlmError::SchemaParseError("no tool_use block in response".to_string())
                })?
        } else {
            resp_body["content"]
                .as_array()
                .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
                .and_then(|b| b["text"].as_str())
                .unwrap_or("")
                .to_string()
        };

        let usage = resp_body["usage"].as_object().map(|u| Usage {
            input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
        });

        tracing::debug!(
            provider = self.name(),
            elapsed_ms = start.elapsed().as_millis() as u64,
            input_tokens = usage.as_ref().map(|u| u.input_tokens),
            output_tokens = usage.as_ref().map(|u| u.output_tokens),
            "llm: response received"
        );

        Ok(CompletionResponse {
            text,
            provider: self.name().to_string(),
            usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialises environment-variable mutations across parallel tests.
    // SAFETY: Rust 2024 requires unsafe for set_var/remove_var. The Mutex
    // ensures only one test modifies the environment at a time.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn from_env_returns_none_without_key() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev = std::env::var("ANTHROPIC_API_KEY").ok();
        unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
        let provider = AnthropicProvider::from_env();
        if let Some(k) = prev {
            unsafe { std::env::set_var("ANTHROPIC_API_KEY", k) };
        }
        assert!(provider.is_none());
    }

    #[test]
    fn from_key_creates_provider_with_default_model() {
        let provider = AnthropicProvider::from_key("sk-test".to_string());
        assert_eq!(provider.name(), "anthropic/claude-sonnet-4-6");
    }
}
