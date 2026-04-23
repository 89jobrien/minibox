use crate::error::LlmError;
use crate::types::{
    CompletionRequest, CompletionResponse, ContentBlock, InferenceRequest, InferenceResponse,
};
use async_trait::async_trait;
use std::time::Duration;

/// Core abstraction for an LLM backend.
///
/// Implement this trait to add a new provider. The trait is object-safe and
/// `Send + Sync` so implementations can be boxed into a [`FallbackChain`](crate::FallbackChain).
///
/// Concrete implementations ship with this crate behind feature flags:
/// - `anthropic` — [`AnthropicProvider`](crate::anthropic::AnthropicProvider)
/// - `openai` — [`OpenAiProvider`](crate::openai::OpenAiProvider)
/// - `gemini` — [`GeminiProvider`](crate::gemini::GeminiProvider)
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Human-readable name identifying this provider and model, e.g.
    /// `"anthropic/claude-sonnet-4-6"`. Used in tracing events and error messages.
    fn name(&self) -> &str;

    /// Send a completion request and return the provider's response.
    ///
    /// Errors should be wrapped in [`LlmError::ProviderError`]. Transient HTTP
    /// errors should use [`HttpStatusError`](crate::HttpStatusError) as the
    /// source so that [`RetryingProvider`](crate::RetryingProvider) can classify
    /// them correctly via [`LlmError::is_transient`](crate::LlmError::is_transient).
    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Send a multi-turn inference request. Providers that support native
    /// multi-turn can override this. The default implementation adapts the
    /// last user message into a [`CompletionRequest`] and calls [`complete`](Self::complete).
    async fn infer(&self, request: &InferenceRequest) -> Result<InferenceResponse, LlmError> {
        // Default: use the last user-turn text as the prompt.
        let prompt = request
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                m.content.iter().find_map(|b| {
                    if let ContentBlock::Text { text } = b {
                        Some(text.clone())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();

        let completion_req = CompletionRequest {
            prompt,
            system: request.system.clone(),
            max_tokens: request.max_tokens,
            schema: None,
            timeout: None,
            max_retries: None,
        };

        let resp = self.complete(&completion_req).await?;
        Ok(InferenceResponse {
            content: vec![ContentBlock::Text { text: resp.text }],
            stop_reason: "end_turn".to_string(),
            usage: resp.usage,
            provider: resp.provider,
        })
    }
}

/// HTTP-level configuration applied when constructing a provider's `reqwest` client.
///
/// Both timeouts are set on the underlying `reqwest::Client` and act as
/// defaults for every request made by that provider. A per-request override
/// can be specified via [`CompletionRequest::timeout`].
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Maximum time to wait for the TCP connection to be established.
    /// Defaults to 10 seconds.
    pub connect_timeout: Duration,

    /// Maximum time to wait for the complete HTTP response after the request
    /// is sent. Defaults to 60 seconds.
    pub request_timeout: Duration,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(60),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompletionResponse, Message};

    struct EchoProvider;

    #[async_trait]
    impl LlmProvider for EchoProvider {
        fn name(&self) -> &str {
            "echo"
        }
        async fn complete(
            &self,
            request: &CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                text: request.prompt.clone(),
                provider: "echo".to_string(),
                usage: None,
            })
        }
    }

    #[tokio::test]
    async fn infer_default_wraps_complete() {
        let provider = EchoProvider;
        let req = InferenceRequest {
            messages: vec![Message::user("hello world")],
            ..InferenceRequest::default()
        };
        let resp = provider.infer(&req).await.unwrap();
        assert_eq!(resp.provider, "echo");
        assert_eq!(resp.text(), "hello world");
        assert_eq!(resp.stop_reason, "end_turn");
    }

    #[tokio::test]
    async fn infer_picks_last_text_block() {
        let provider = EchoProvider;
        let req = InferenceRequest {
            messages: vec![Message::user("first"), Message::user("last")],
            ..InferenceRequest::default()
        };
        let resp = provider.infer(&req).await.unwrap();
        assert_eq!(resp.text(), "last");
    }
}
