/// Generates `from_env()`, `from_env_with_config()`, and test-only `from_key()` for a provider.
macro_rules! provide {
    ($provider:ty, $env_var:expr, $default_model:expr) => {
        impl $provider {
            pub fn from_env() -> Option<Self> {
                Self::from_env_with_config(&$crate::ProviderConfig::default())
            }

            pub fn from_env_with_config(config: &$crate::ProviderConfig) -> Option<Self> {
                std::env::var($env_var)
                    .ok()
                    .map(|k| Self::with_config(k, $default_model.to_string(), config))
            }

            /// Test helper — inject a key without reading the environment.
            #[cfg(test)]
            pub(crate) fn from_key(key: String) -> Self {
                Self::new(key, $default_model.to_string())
            }
        }
    };
}

#[cfg(feature = "anthropic")]
pub mod anthropic;
pub mod chain;
pub mod error;
#[cfg(feature = "gemini")]
pub mod gemini;
#[cfg(feature = "openai")]
pub mod openai;
pub mod provider;
pub mod retry;
pub mod types;

pub use chain::FallbackChain;
pub use error::{HttpStatusError, LlmError};
pub use provider::{LlmProvider, ProviderConfig};
pub use retry::{RetryConfig, RetryingProvider};
pub use types::{CompletionRequest, CompletionResponse, JsonSchema, Usage};

/// Async LLM invocation. Returns a future that resolves to `Result<CompletionResponse, LlmError>`.
///
/// ```ignore
/// let resp = ainvoke!(chain, "Summarize this").await?;
/// let resp = ainvoke!(chain, "Summarize", system: "Be concise", max_tokens: 512).await?;
/// ```
#[macro_export]
macro_rules! ainvoke {
    ($chain:expr, $prompt:expr $(, $key:ident : $val:expr)* $(,)?) => {
        $chain.complete(&$crate::CompletionRequest {
            prompt: $prompt.into(),
            $( $key: $crate::ainvoke!(@wrap $key $val), )*
            ..$crate::CompletionRequest::default()
        })
    };
    (@wrap system $val:expr) => { Some($val.into()) };
    (@wrap schema $val:expr) => { Some($val) };
    (@wrap timeout $val:expr) => { Some($val) };
    (@wrap max_retries $val:expr) => { Some($val) };
    (@wrap max_tokens $val:expr) => { $val };
}

/// Sync (blocking) LLM invocation. Only works with `FallbackChain`.
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
