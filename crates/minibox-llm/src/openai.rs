use async_trait::async_trait;

use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse, Usage};

/// LLM provider backed by the OpenAI Chat Completions API.
///
/// Enabled by the `openai` feature flag. Uses the
/// `https://api.openai.com/v1/chat/completions` endpoint.
///
/// Default model: `gpt-4.1`. Construct via [`from_env`](OpenAiProvider::from_env)
/// (reads `OPENAI_API_KEY`) or [`new`](OpenAiProvider::new) / [`with_config`](OpenAiProvider::with_config)
/// when the key is already available.
///
/// Structured output is implemented via OpenAI's `response_format` field with
/// `type: json_schema` and `strict: true`. The raw content string from the first
/// choice is returned as the response text.
pub struct OpenAiProvider {
    key: String,
    model: String,
    /// Display name returned by [`name`](OpenAiProvider::name), e.g. `"openai/gpt-4.1"`.
    display_name: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    /// Construct with default HTTP timeouts (10s connect, 60s request).
    pub fn new(key: String, model: String) -> Self {
        Self::with_config(key, model, &crate::ProviderConfig::default())
    }

    /// Construct with explicit HTTP timeout configuration.
    ///
    /// The `reqwest::Client` is built once here and reused for every request
    /// made by this provider instance.
    pub fn with_config(key: String, model: String, config: &crate::ProviderConfig) -> Self {
        let display_name = format!("openai/{model}");
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
// Reads OPENAI_API_KEY; default model is gpt-4.1.
provide!(OpenAiProvider, "OPENAI_API_KEY", "gpt-4.1");

#[async_trait]
impl LlmProvider for OpenAiProvider {
    /// Returns the display name, e.g. `"openai/gpt-4.1"`.
    fn name(&self) -> &str {
        &self.display_name
    }

    /// Send a completion request to the OpenAI Chat Completions API.
    ///
    /// When [`CompletionRequest::system`] is set, it is prepended to the
    /// `messages` array as a `system`-role message.
    ///
    /// When [`CompletionRequest::schema`] is set, `response_format` is set to
    /// `json_schema` with `strict: true`. The model is expected to return valid
    /// JSON matching the schema as the `content` of the first choice message.
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

        let mut messages = Vec::new();
        if let Some(system) = &request.system {
            messages.push(serde_json::json!({"role": "system", "content": system}));
        }
        messages.push(serde_json::json!({"role": "user", "content": &request.prompt}));

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "messages": messages,
        });

        if let Some(schema) = &request.schema {
            // Request structured JSON output via OpenAI's json_schema response format.
            body["response_format"] = serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": &schema.name,
                    "schema": &schema.schema,
                    "strict": true,
                },
            });
        }

        let mut req = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.key))
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

        let text = resp_body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        // Structured output mode: an empty content string means the model did
        // not return the expected JSON — treat as a parse failure.
        if request.schema.is_some() && text.is_empty() {
            return Err(LlmError::SchemaParseError(
                "empty content in structured output response".to_string(),
            ));
        }

        // OpenAI reports usage under prompt_tokens / completion_tokens.
        let usage = resp_body["usage"].as_object().map(|u| Usage {
            input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
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

    #[test]
    fn from_env_returns_none_without_key() {
        let provider = OpenAiProvider::from_env();
        assert!(provider.is_none());
    }

    #[test]
    fn from_key_creates_provider_with_default_model() {
        let provider = OpenAiProvider::from_key("sk-test".to_string());
        assert_eq!(provider.name(), "openai/gpt-4.1");
    }
}
