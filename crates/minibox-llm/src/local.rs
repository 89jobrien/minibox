use async_trait::async_trait;
use serde::Deserialize;

use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{
    CompletionRequest, CompletionResponse, ContentBlock, InferenceRequest, InferenceResponse,
    Role,
};

pub struct OllamaProvider {
    host: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(host: String, model: String) -> Self {
        Self {
            host,
            model,
            client: reqwest::Client::new(),
        }
    }

    pub fn from_env() -> Self {
        let host = std::env::var("OLLAMA_HOST")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        let model =
            std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2".to_string());
        Self::new(host, model)
    }

    pub async fn is_available(&self) -> bool {
        self.client
            .get(format!("{}/api/version", self.host))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let body = serde_json::json!({
            "model": self.model,
            "prompt": req.prompt,
            "system": req.system,
            "stream": false,
            "options": {
                "num_predict": req.max_tokens,
            }
        });
        let resp = self
            .client
            .post(format!("{}/api/generate", self.host))
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let parsed: GenerateResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Deserialization(e.to_string()))?;
        Ok(CompletionResponse {
            text: parsed.response,
        })
    }

    async fn infer(&self, req: &InferenceRequest) -> Result<InferenceResponse, LlmError> {
        let messages: Vec<serde_json::Value> = req
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::System => "system",
                };
                let text = m
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                serde_json::json!({"role": role, "content": text})
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": false,
            "options": {
                "num_predict": req.max_tokens,
            }
        });

        if let Some(system) = &req.system {
            if let Some(arr) = body["messages"].as_array_mut() {
                arr.insert(
                    0,
                    serde_json::json!({"role": "system", "content": system}),
                );
            }
        }

        let resp = self
            .client
            .post(format!("{}/api/chat", self.host))
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let chat: ChatResponse = resp
            .json()
            .await
            .map_err(|e| LlmError::Deserialization(e.to_string()))?;
        Ok(InferenceResponse {
            content: vec![ContentBlock::Text {
                text: chat.message.content,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_from_env_uses_defaults() {
        let provider = OllamaProvider::from_env();
        assert!(!provider.host.is_empty());
        assert!(!provider.model.is_empty());
    }
}
