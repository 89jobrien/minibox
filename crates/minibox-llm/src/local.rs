/// Ollama local LLM provider — talks to localhost:11434 via the Ollama /api/chat API.
///
/// Gate this module with the `local` Cargo feature.
use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{
    CompletionRequest, CompletionResponse, ContentBlock, InferenceRequest, InferenceResponse, Role,
    Usage,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "llama3";

/// Provider adapter for a locally-running [Ollama](https://ollama.com) instance.
///
/// Communicates with `http://localhost:11434/api/chat` (or the URL specified
/// via `OLLAMA_BASE_URL`). The model is read from `OLLAMA_MODEL` (default:
/// `"llama3"`).
pub struct OllamaProvider {
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    /// Construct a provider from explicit values.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Construct a provider from environment variables.
    ///
    /// - `OLLAMA_BASE_URL` — base URL (default: `http://localhost:11434`)
    /// - `OLLAMA_MODEL`    — model name (default: `llama3`)
    pub fn from_env() -> Self {
        let base_url =
            std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        Self::new(base_url, model)
    }

    /// Probe whether Ollama is reachable at the configured base URL.
    ///
    /// Makes a GET to `{base_url}/api/tags`. Returns `true` if the server
    /// responds with a 2xx status, `false` otherwise.
    pub async fn is_available(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        self.client
            .get(&url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

// ── Ollama wire types ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct OllamaChatRequest<'a> {
    model: &'a str,
    messages: Vec<OllamaMessage<'a>>,
    stream: bool,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

// ── LlmProvider impl ─────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama/local"
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut messages = Vec::new();
        if let Some(system) = &request.system {
            messages.push(OllamaMessage {
                role: "system",
                content: system.as_str(),
            });
        }
        messages.push(OllamaMessage {
            role: "user",
            content: request.prompt.as_str(),
        });

        let body = OllamaChatRequest {
            model: &self.model,
            messages,
            stream: false,
        };

        let url = format!("{}/api/chat", self.base_url);
        let http_resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;

        if !http_resp.status().is_success() {
            let status = http_resp.status().as_u16();
            let body_text = http_resp.text().await.unwrap_or_default();
            return Err(LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(crate::error::HttpStatusError {
                    status,
                    body: body_text,
                }),
            });
        }

        let parsed: OllamaChatResponse =
            http_resp
                .json()
                .await
                .map_err(|e| LlmError::ProviderError {
                    provider: self.name().to_string(),
                    source: Box::new(e),
                })?;

        let usage = match (parsed.prompt_eval_count, parsed.eval_count) {
            (Some(i), Some(o)) => Some(Usage {
                input_tokens: i,
                output_tokens: o,
            }),
            _ => None,
        };

        Ok(CompletionResponse {
            text: parsed.message.content,
            provider: self.name().to_string(),
            usage,
        })
    }

    /// Multi-turn override: maps each [`Message`] to an Ollama message,
    /// preserving the full conversation history.
    async fn infer(&self, request: &InferenceRequest) -> Result<InferenceResponse, LlmError> {
        let mut messages: Vec<OllamaMessage<'_>> = Vec::new();

        // Collect text content from each turn. Tool-use/result blocks are
        // flattened to their text representation for the Ollama wire format.
        let mut flat: Vec<(String, String)> = Vec::new(); // (role, content)
        if let Some(system) = &request.system {
            flat.push(("system".to_string(), system.clone()));
        }
        for msg in &request.messages {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            let text: String = msg
                .content
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => text.clone(),
                    ContentBlock::ToolResult { content, .. } => content.clone(),
                    ContentBlock::ToolUse { name, .. } => format!("[tool call: {name}]"),
                })
                .collect::<Vec<_>>()
                .join("\n");
            flat.push((role.to_string(), text));
        }

        // Build borrowed slice for the request body.
        for (role, content) in &flat {
            messages.push(OllamaMessage {
                role: role.as_str(),
                content: content.as_str(),
            });
        }

        let body = OllamaChatRequest {
            model: &self.model,
            messages,
            stream: false,
        };

        let url = format!("{}/api/chat", self.base_url);
        let http_resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;

        if !http_resp.status().is_success() {
            let status = http_resp.status().as_u16();
            let body_text = http_resp.text().await.unwrap_or_default();
            return Err(LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(crate::error::HttpStatusError {
                    status,
                    body: body_text,
                }),
            });
        }

        let parsed: OllamaChatResponse =
            http_resp
                .json()
                .await
                .map_err(|e| LlmError::ProviderError {
                    provider: self.name().to_string(),
                    source: Box::new(e),
                })?;

        let usage = match (parsed.prompt_eval_count, parsed.eval_count) {
            (Some(i), Some(o)) => Some(Usage {
                input_tokens: i,
                output_tokens: o,
            }),
            _ => None,
        };

        Ok(InferenceResponse {
            content: vec![ContentBlock::Text {
                text: parsed.message.content,
            }],
            stop_reason: "end_turn".to_string(),
            usage,
            provider: self.name().to_string(),
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_uses_defaults() {
        let provider = OllamaProvider::from_env();
        assert_eq!(provider.base_url, DEFAULT_BASE_URL);
        assert_eq!(provider.model, DEFAULT_MODEL);
    }

    #[test]
    fn from_env_reads_env_vars() {
        // SAFETY: Rust 2024 requires unsafe for env mutation. These tests run
        // sequentially within this module — no parallel env mutation.
        unsafe {
            std::env::set_var("OLLAMA_BASE_URL", "http://custom:11434");
            std::env::set_var("OLLAMA_MODEL", "mistral");
        }
        let provider = OllamaProvider::from_env();
        // SAFETY: cleanup
        unsafe {
            std::env::remove_var("OLLAMA_BASE_URL");
            std::env::remove_var("OLLAMA_MODEL");
        }
        assert_eq!(provider.base_url, "http://custom:11434");
        assert_eq!(provider.model, "mistral");
    }

    #[test]
    fn name_is_ollama_local() {
        let p = OllamaProvider::new(DEFAULT_BASE_URL, DEFAULT_MODEL);
        assert_eq!(p.name(), "ollama/local");
    }
}
