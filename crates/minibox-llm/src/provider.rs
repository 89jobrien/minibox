use crate::error::LlmError;
use crate::types::{CompletionRequest, CompletionResponse};
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
