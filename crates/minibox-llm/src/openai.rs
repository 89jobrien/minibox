use async_trait::async_trait;

use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse, Usage};

pub struct OpenAiProvider {
    key: String,
    model: String,
    display_name: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(key: String, model: String) -> Self {
        let display_name = format!("openai/{model}");
        Self {
            key,
            model,
            display_name,
            client: reqwest::Client::new(),
        }
    }

    pub fn from_env() -> Option<Self> {
        Self::from_env_with_key(std::env::var("OPENAI_API_KEY").ok())
    }

    pub(crate) fn from_env_with_key(key: Option<String>) -> Option<Self> {
        key.map(|k| Self::new(k, "gpt-4.1".to_string()))
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.display_name
    }

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
            body["response_format"] = serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": &schema.name,
                    "schema": &schema.schema,
                    "strict": true,
                },
            });
        }

        let resp = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;

        let status = resp.status();
        let resp_body: serde_json::Value =
            resp.json().await.map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;

        if !status.is_success() {
            let msg = resp_body["error"]["message"]
                .as_str()
                .unwrap_or("unknown API error");
            return Err(LlmError::ProviderError {
                provider: self.name().to_string(),
                source: msg.to_string().into(),
            });
        }

        let text = resp_body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

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
        let provider = OpenAiProvider::from_env_with_key(None);
        assert!(provider.is_none());
    }

    #[test]
    fn from_env_returns_some_with_key() {
        let provider = OpenAiProvider::from_env_with_key(Some("sk-test".to_string()));
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().name(), "openai/gpt-4.1");
    }
}
