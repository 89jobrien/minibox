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
