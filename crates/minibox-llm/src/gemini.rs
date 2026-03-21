use async_trait::async_trait;

use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse, Usage};

const UNSUPPORTED_KEYWORDS: &[&str] = &[
    "$schema",
    "$ref",
    "$id",
    "$comment",
    "additionalProperties",
    "patternProperties",
    "if",
    "then",
    "else",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
];

pub(crate) fn sanitize_schema(schema: &serde_json::Value) -> serde_json::Value {
    match schema {
        serde_json::Value::Object(map) => {
            let filtered: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter(|(k, _)| !UNSUPPORTED_KEYWORDS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), sanitize_schema(v)))
                .collect();
            serde_json::Value::Object(filtered)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(sanitize_schema).collect())
        }
        other => other.clone(),
    }
}

pub struct GeminiProvider {
    key: String,
    model: String,
    display_name: String,
    client: reqwest::Client,
}

impl GeminiProvider {
    pub fn new(key: String, model: String) -> Self {
        Self::with_config(key, model, &crate::ProviderConfig::default())
    }

    pub fn with_config(key: String, model: String, config: &crate::ProviderConfig) -> Self {
        let display_name = format!("google/{model}");
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

provide!(GeminiProvider, "GEMINI_API_KEY", "gemini-2.5-flash");

#[async_trait]
impl LlmProvider for GeminiProvider {
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

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model,
        );

        let mut body = serde_json::json!({
            "contents": [{"parts": [{"text": &request.prompt}]}],
        });

        if let Some(system) = &request.system {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{"text": system}],
            });
        }

        let mut generation_config = serde_json::json!({
            "maxOutputTokens": request.max_tokens,
        });

        if let Some(schema) = &request.schema {
            generation_config["responseMimeType"] = "application/json".into();
            generation_config["responseSchema"] = sanitize_schema(&schema.schema);
        }

        body["generationConfig"] = generation_config;

        let mut req = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.key)
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

        let text = resp_body["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        if request.schema.is_some() && text.is_empty() {
            return Err(LlmError::SchemaParseError(
                "empty content in structured output response".to_string(),
            ));
        }

        let usage = resp_body["usageMetadata"].as_object().map(|u| Usage {
            input_tokens: u["promptTokenCount"].as_u64().unwrap_or(0) as u32,
            output_tokens: u["candidatesTokenCount"].as_u64().unwrap_or(0) as u32,
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
        let provider = GeminiProvider::from_env();
        assert!(provider.is_none());
    }

    #[test]
    fn from_key_creates_provider_with_default_model() {
        let provider = GeminiProvider::from_key("test-key".to_string());
        assert_eq!(provider.name(), "google/gemini-2.5-flash");
    }

    #[test]
    fn sanitize_strips_unsupported_keywords() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        });
        let sanitized = sanitize_schema(&schema);
        assert!(sanitized.get("additionalProperties").is_none());
        assert!(sanitized.get("$schema").is_none());
        assert!(sanitized.get("type").is_some());
        assert!(sanitized.get("properties").is_some());
    }
}
