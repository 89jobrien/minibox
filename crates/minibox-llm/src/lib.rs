//! Multi-provider LLM client library for minibox.
//!
//! `minibox-llm` provides a unified interface for sending completion requests
//! to multiple LLM backends (Anthropic Claude, OpenAI, Google Gemini) with
//! automatic fallback, exponential-backoff retries, and optional structured
//! JSON output via provider-native schema mechanisms.
//!
//! # Architecture
//!
//! The core abstraction is the [`LlmProvider`] trait. Concrete implementations
//! (`AnthropicProvider`, `OpenAiProvider`, `GeminiProvider`) are gated behind
//! Cargo feature flags (`anthropic`, `openai`, `gemini`). Each provider can be
//! wrapped in a [`RetryingProvider`] to add transient-error retry logic.
//! [`FallbackChain`] holds an ordered list of providers and tries them in
//! sequence, returning the first successful response.
//!
//! # Quick start
//!
//! ```ignore
//! use minibox_llm::{FallbackChain, ainvoke, invoke};
//!
//! // Build a chain from environment variables (reads ANTHROPIC_API_KEY,
//! // OPENAI_API_KEY, GEMINI_API_KEY). Each provider is automatically wrapped
//! // in a RetryingProvider with default retry settings.
//! let chain = FallbackChain::from_env();
//!
//! // Async invocation (requires an async runtime):
//! let resp = ainvoke!(chain, "Summarize this text").await?;
//!
//! // Sync invocation (spawns an internal Tokio runtime):
//! let resp = invoke!(chain, "Summarize this text")?;
//!
//! println!("{}", resp.text);
//! ```
//!
//! # Configuration
//!
//! Use [`ProviderConfig`] to tune HTTP connect/request timeouts, and
//! [`RetryConfig`] to tune retry behaviour. Pass both to
//! [`FallbackChain::from_env_with_config`]:
//!
//! ```ignore
//! use minibox_llm::{FallbackChain, ProviderConfig, RetryConfig};
//! use std::time::Duration;
//!
//! let chain = FallbackChain::from_env_with_config(
//!     ProviderConfig {
//!         connect_timeout: Duration::from_secs(5),
//!         request_timeout: Duration::from_secs(30),
//!     },
//!     RetryConfig {
//!         max_retries: 3,
//!         backoff_base: Duration::from_millis(500),
//!     },
//! );
//! ```
//!
//! # Structured output
//!
//! Pass a [`JsonSchema`] in the request to request JSON-structured output.
//!
//! # Error handling
//!
//! All errors are surfaced as [`LlmError`]. Use [`LlmError::is_transient`] to
//! decide whether to retry. [`HttpStatusError`] carries the raw HTTP status and
//! response body and is classified as transient for 429 / 5xx and permanent for
//! other 4xx.

#[cfg(feature = "anthropic")]
pub mod anthropic;
pub mod chain;
pub mod config;
pub mod error;
#[cfg(feature = "gemini")]
pub mod gemini;
#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "openai")]
pub mod openai;
pub mod provider;
pub mod retry;
pub mod types;

pub use chain::FallbackChain;
pub use error::{HttpStatusError, LlmError};
pub use provider::{LlmProvider, ProviderConfig};
pub use retry::{RetryConfig, RetryingProvider};
pub use types::{
    CompletionRequest, CompletionResponse, ContentBlock, InferenceRequest, InferenceResponse,
    JsonSchema, Message, Role, ToolDefinition, Usage,
};

/// Async LLM invocation macro. Returns a `Future` that resolves to
/// `Result<CompletionResponse, LlmError>`.
///
/// The first argument is any expression that implements an async `complete`
/// method accepting a `&CompletionRequest` (typically a [`FallbackChain`] or a
/// [`RetryingProvider`]). The second argument is the user prompt. Optional
/// keyword arguments map directly to fields on [`CompletionRequest`].
///
/// # Supported keyword arguments
///
/// | Key          | Type                   | Notes                              |
/// |--------------|------------------------|------------------------------------|
/// | `system`     | `impl Into<String>`    | Sets [`CompletionRequest::system`] |
/// | `max_tokens` | `u32`                  |                                    |
/// | `schema`     | [`JsonSchema`]         | Request structured JSON output     |
/// | `timeout`    | `std::time::Duration`  | Per-request timeout override       |
/// | `max_retries`| `u32`                  | Override retry count for this call |
///
/// # Examples
///
/// ```ignore
/// let resp = ainvoke!(chain, "Summarize this").await?;
/// let resp = ainvoke!(chain, "Summarize", system: "Be concise", max_tokens: 512).await?;
/// ```
#[macro_export]
macro_rules! ainvoke {
    ($chain:expr, $prompt:expr $(, $key:ident : $val:expr)* $(,)?) => {
        async {
            let request = $crate::CompletionRequest {
                prompt: $prompt.into(),
                $( $key: $crate::ainvoke!(@wrap $key $val), )*
                ..$crate::CompletionRequest::default()
            };
            $chain.complete(&request).await
        }
    };
    (@wrap system $val:expr) => { Some($val.into()) };
    (@wrap schema $val:expr) => { Some($val) };
    (@wrap timeout $val:expr) => { Some($val) };
    (@wrap max_retries $val:expr) => { Some($val) };
    (@wrap max_tokens $val:expr) => { $val };
}

/// Sync (blocking) LLM invocation macro. Only works with [`FallbackChain`],
/// which provides the `complete_sync` method backed by an internal Tokio runtime.
///
/// Supports the same keyword arguments as [`ainvoke!`]. Do not call from within
/// an existing Tokio async context — use [`ainvoke!`] instead.
///
/// # Examples
///
/// ```ignore
/// let resp = invoke!(chain, "Summarize this")?;
/// let resp = invoke!(chain, "Summarize", system: "Be concise", max_tokens: 512)?;
/// ```
#[macro_export]
macro_rules! invoke {
    ($chain:expr, $prompt:expr $(, $key:ident : $val:expr)* $(,)?) => {{
        let request = $crate::CompletionRequest {
            prompt: $prompt.into(),
            $( $key: $crate::invoke!(@wrap $key $val), )*
            ..$crate::CompletionRequest::default()
        };
        $chain.complete_sync(&request)
    }};
    (@wrap system $val:expr) => { Some($val.into()) };
    (@wrap schema $val:expr) => { Some($val) };
    (@wrap timeout $val:expr) => { Some($val) };
    (@wrap max_retries $val:expr) => { Some($val) };
    (@wrap max_tokens $val:expr) => { $val };
}
