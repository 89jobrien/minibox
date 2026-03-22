use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A single completion request sent to an LLM provider.
///
/// Build one directly or use the [`ainvoke!`](crate::ainvoke) /
/// [`invoke!`](crate::invoke) macros which fill in defaults for you.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    /// The user-visible prompt text.
    pub prompt: String,

    /// Optional system prompt. Providers that support a separate system role
    /// (Anthropic, OpenAI) use it natively; others prepend it to the user turn.
    pub system: Option<String>,

    /// Maximum number of tokens to generate. Defaults to `1024`.
    pub max_tokens: u32,

    /// When set, the provider is asked to return a JSON object conforming to
    /// this schema rather than free-form text. The mechanism varies by provider:
    /// Anthropic uses tool-use, OpenAI uses `response_format`, and Gemini uses
    /// `responseSchema`.
    pub schema: Option<JsonSchema>,

    /// Per-request HTTP timeout. Overrides the [`ProviderConfig`](crate::ProviderConfig)
    /// `request_timeout` for this call only.
    pub timeout: Option<Duration>,

    /// Per-request retry limit. Overrides [`RetryConfig`](crate::RetryConfig)
    /// `max_retries` for this call only. Only transient errors are retried.
    pub max_retries: Option<u32>,
}

impl Default for CompletionRequest {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            system: None,
            max_tokens: 1024,
            schema: None,
            timeout: None,
            max_retries: None,
        }
    }
}

/// The response returned by a successful completion call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    /// The generated text. When a [`JsonSchema`] was requested this contains
    /// the raw JSON string returned by the provider.
    pub text: String,

    /// Display name of the provider that produced this response, e.g.
    /// `"anthropic/claude-sonnet-4-6"` or `"openai/gpt-4.1"`.
    pub provider: String,

    /// Token counts reported by the provider, if available.
    pub usage: Option<Usage>,
}

/// Token-usage statistics accompanying a completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    /// Number of tokens in the prompt / input.
    pub input_tokens: u32,

    /// Number of tokens in the generated completion / output.
    pub output_tokens: u32,
}

/// A JSON Schema definition used to request structured output from a provider.
///
/// The schema is passed as a raw [`serde_json::Value`] so callers can construct
/// it with `serde_json::json!()`. Some providers (Gemini) strip unsupported
/// JSON Schema keywords automatically before sending.
#[derive(Debug, Clone)]
pub struct JsonSchema {
    /// A short identifier for the schema, used as the tool or format name sent
    /// to the provider API (e.g. `"sentiment_result"`).
    pub name: String,

    /// The JSON Schema document describing the expected output shape.
    pub schema: serde_json::Value,
}
