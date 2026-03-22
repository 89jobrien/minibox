use crate::error::LlmError;
use crate::provider::{LlmProvider, ProviderConfig};
use crate::retry::RetryConfig;
use crate::types::{CompletionRequest, CompletionResponse};
use std::sync::OnceLock;

/// An ordered chain of [`LlmProvider`] instances tried in sequence until one succeeds.
///
/// Each provider is called in the order it was added. If a provider fails for
/// any reason (transient or permanent), the chain moves on to the next one.
/// If every provider fails, [`LlmError::AllProvidersFailed`] is returned with
/// a semicolon-separated list of all individual errors.
///
/// # Building a chain
///
/// The most common construction path reads API keys from the environment:
///
/// ```ignore
/// // Uses default ProviderConfig (10s connect, 60s request) and RetryConfig (2 retries, 1s base).
/// let chain = FallbackChain::from_env();
///
/// // With explicit timeouts and retry settings:
/// let chain = FallbackChain::from_env_with_config(provider_config, retry_config);
///
/// // Fully manual — inject any providers you like:
/// let chain = FallbackChain::new(vec![Box::new(my_provider)]);
/// ```
///
/// When built via `from_env` / `from_env_with_config`, each discovered provider
/// is automatically wrapped in a [`RetryingProvider`](crate::RetryingProvider)
/// so transient errors are retried before the chain falls back to the next provider.
pub struct FallbackChain {
    /// The ordered list of providers to attempt. Each is tried in sequence.
    providers: Vec<Box<dyn LlmProvider>>,
}

impl FallbackChain {
    /// Construct a chain from an explicit list of providers.
    ///
    /// Providers are tried in the order given. Wrap providers in
    /// [`RetryingProvider`](crate::RetryingProvider) before passing them here if
    /// you want per-provider retry logic.
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        Self { providers }
    }

    /// Send a completion request, trying each provider in order.
    ///
    /// Returns the first successful [`CompletionResponse`]. If all providers
    /// fail, returns [`LlmError::AllProvidersFailed`] containing a
    /// semicolon-separated summary of each provider's error.
    pub async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let mut errors = Vec::new();
        for provider in &self.providers {
            match provider.complete(request).await {
                Ok(response) => {
                    tracing::info!(provider = provider.name(), "llm: completion succeeded");
                    return Ok(response);
                }
                Err(e) => {
                    tracing::warn!(
                        provider = provider.name(),
                        error = %e,
                        "llm: provider failed, trying next"
                    );
                    errors.push(format!("{}: {e}", provider.name()));
                }
            }
        }
        Err(LlmError::AllProvidersFailed(errors.join("; ")))
    }
}

impl FallbackChain {
    /// Build a chain from environment variables using default HTTP and retry configuration.
    ///
    /// Reads `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, and `GEMINI_API_KEY`
    /// (subject to the `anthropic`, `openai`, and `gemini` feature flags
    /// respectively). Available providers are added in that order and each is
    /// wrapped in a [`RetryingProvider`](crate::RetryingProvider) with default
    /// settings (2 retries, 1-second backoff base).
    ///
    /// Logs a `warn` event for each provider whose key is missing and a `warn`
    /// if no providers are available at all.
    pub fn from_env() -> Self {
        Self::from_env_with_config(ProviderConfig::default(), RetryConfig::default())
    }

    /// Build a chain from environment variables with explicit HTTP and retry configuration.
    ///
    /// The `provider_config` controls HTTP-level timeouts for each provider's
    /// `reqwest` client. The `retry_config` controls exponential-backoff retry
    /// behaviour applied by the [`RetryingProvider`](crate::RetryingProvider)
    /// wrapper around each discovered provider.
    ///
    /// Provider order: Anthropic → OpenAI → Gemini (subject to feature flags).
    pub fn from_env_with_config(
        provider_config: ProviderConfig,
        retry_config: RetryConfig,
    ) -> Self {
        let mut providers: Vec<Box<dyn LlmProvider>> = Vec::new();

        #[cfg(feature = "anthropic")]
        if let Some(p) = crate::anthropic::AnthropicProvider::from_env_with_config(&provider_config)
        {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(crate::retry::RetryingProvider::new(
                p,
                retry_config.clone(),
            )));
        } else {
            tracing::warn!(provider = "anthropic", "llm: provider skipped (no key)");
        }

        #[cfg(feature = "openai")]
        if let Some(p) = crate::openai::OpenAiProvider::from_env_with_config(&provider_config) {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(crate::retry::RetryingProvider::new(
                p,
                retry_config.clone(),
            )));
        } else {
            tracing::warn!(provider = "openai", "llm: provider skipped (no key)");
        }

        #[cfg(feature = "gemini")]
        if let Some(p) = crate::gemini::GeminiProvider::from_env_with_config(&provider_config) {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(crate::retry::RetryingProvider::new(
                p,
                retry_config.clone(),
            )));
        } else {
            tracing::warn!(provider = "gemini", "llm: provider skipped (no key)");
        }

        if providers.is_empty() {
            tracing::warn!("llm: no providers available — all API keys missing");
        }

        Self { providers }
    }
}

/// Lazily-initialised Tokio runtime used by `complete_sync`.
///
/// A single runtime is shared across all `complete_sync` calls for the lifetime
/// of the process. Using `OnceLock` avoids the overhead of creating a new
/// runtime for every blocking call.
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

impl FallbackChain {
    /// Blocking wrapper around [`complete`](FallbackChain::complete) for callers
    /// that are not in an async context.
    ///
    /// Internally this uses a lazily-created Tokio multi-threaded runtime shared
    /// across all calls. The same runtime is reused for the lifetime of the process.
    ///
    /// The [`invoke!`](crate::invoke) macro calls this method.
    ///
    /// # Panics
    ///
    /// Panics if called from within an existing Tokio runtime (e.g. inside an
    /// `async fn` or `tokio::task::spawn_blocking`). Use [`complete`](FallbackChain::complete)
    /// directly in async contexts.
    pub fn complete_sync(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let rt = RUNTIME.get_or_init(|| {
            tokio::runtime::Runtime::new().expect("failed to create tokio runtime")
        });
        rt.block_on(self.complete(request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CompletionRequest;
    use crate::{ainvoke, invoke};
    use std::sync::Mutex;

    /// Serialises environment-variable mutations across parallel tests.
    // SAFETY: Rust 2024 requires unsafe for set_var/remove_var. The Mutex
    // ensures only one test modifies the environment at a time.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn request(prompt: &str) -> CompletionRequest {
        CompletionRequest {
            prompt: prompt.to_string(),
            system: None,
            max_tokens: 100,
            schema: None,
            timeout: None,
            max_retries: None,
        }
    }

    /// Minimal mock provider that returns a pre-configured Ok or Err.
    struct MockProvider {
        response: Result<String, String>,
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }
        async fn complete(
            &self,
            _request: &CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            match &self.response {
                Ok(text) => Ok(CompletionResponse {
                    text: text.clone(),
                    provider: "mock".to_string(),
                    usage: None,
                }),
                Err(msg) => Err(LlmError::ProviderError {
                    provider: "mock".to_string(),
                    source: msg.clone().into(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn empty_chain_returns_all_providers_failed() {
        let chain = FallbackChain::new(vec![]);
        let result = chain.complete(&request("hello")).await;
        assert!(matches!(result, Err(LlmError::AllProvidersFailed(_))));
    }

    #[tokio::test]
    async fn single_provider_returns_response() {
        let chain = FallbackChain::new(vec![Box::new(MockProvider {
            response: Ok("hello".to_string()),
        })]);
        let result = chain.complete(&request("test")).await.unwrap();
        assert_eq!(result.text, "hello");
        assert_eq!(result.provider, "mock");
    }

    #[tokio::test]
    async fn falls_back_to_second_provider_on_failure() {
        let chain = FallbackChain::new(vec![
            Box::new(MockProvider {
                response: Err("down".to_string()),
            }),
            Box::new(MockProvider {
                response: Ok("fallback".to_string()),
            }),
        ]);
        let result = chain.complete(&request("test")).await.unwrap();
        assert_eq!(result.text, "fallback");
    }

    #[tokio::test]
    async fn all_fail_returns_error_with_details() {
        let chain = FallbackChain::new(vec![
            Box::new(MockProvider {
                response: Err("err1".to_string()),
            }),
            Box::new(MockProvider {
                response: Err("err2".to_string()),
            }),
        ]);
        let result = chain.complete(&request("test")).await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("err1"), "error should contain first failure");
        assert!(err.contains("err2"), "error should contain second failure");
    }

    #[test]
    fn sync_wrapper_works_outside_async() {
        let chain = FallbackChain::new(vec![Box::new(MockProvider {
            response: Ok("sync".to_string()),
        })]);
        let result = chain.complete_sync(&request("test")).unwrap();
        assert_eq!(result.text, "sync");
    }

    #[tokio::test]
    async fn retrying_chain_retries_transient_then_succeeds() {
        use crate::error::HttpStatusError;
        use std::sync::atomic::{AtomicU32, Ordering};

        struct TransientThenOk {
            calls: AtomicU32,
        }

        #[async_trait::async_trait]
        impl LlmProvider for TransientThenOk {
            fn name(&self) -> &str {
                "transient-ok"
            }
            async fn complete(
                &self,
                _r: &CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Err(LlmError::ProviderError {
                        provider: "transient-ok".to_string(),
                        source: Box::new(HttpStatusError {
                            status: 503,
                            body: "down".to_string(),
                        }),
                    })
                } else {
                    Ok(CompletionResponse {
                        text: "recovered".to_string(),
                        provider: "transient-ok".to_string(),
                        usage: None,
                    })
                }
            }
        }

        let provider = TransientThenOk {
            calls: AtomicU32::new(0),
        };
        let retrying = crate::RetryingProvider::new(
            provider,
            crate::RetryConfig {
                max_retries: 2,
                backoff_base: std::time::Duration::from_millis(1),
            },
        );
        let chain = FallbackChain::new(vec![Box::new(retrying)]);
        let resp = chain.complete(&request("test")).await.unwrap();
        assert_eq!(resp.text, "recovered");
    }

    #[test]
    fn from_env_with_no_keys_creates_empty_chain() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // SAFETY: Rust 2024 requires unsafe for env mutation. ENV_MUTEX serializes access.
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("GEMINI_API_KEY");
        }
        let chain = FallbackChain::from_env();
        let result = chain.complete_sync(&request("test"));
        assert!(matches!(result, Err(LlmError::AllProvidersFailed(_))));
    }

    #[tokio::test]
    async fn ainvoke_macro_minimal() {
        let chain = FallbackChain::new(vec![Box::new(MockProvider {
            response: Ok("macro works".to_string()),
        })]);
        let resp = ainvoke!(chain, "test prompt").await.unwrap();
        assert_eq!(resp.text, "macro works");
    }

    #[tokio::test]
    async fn ainvoke_macro_with_options() {
        let chain = FallbackChain::new(vec![Box::new(MockProvider {
            response: Ok("with opts".to_string()),
        })]);
        let resp = ainvoke!(chain, "test",
            system: "be helpful",
            max_tokens: 2048,
        )
        .await
        .unwrap();
        assert_eq!(resp.text, "with opts");
    }

    #[test]
    fn invoke_macro_sync() {
        let chain = FallbackChain::new(vec![Box::new(MockProvider {
            response: Ok("sync macro".to_string()),
        })]);
        let resp = invoke!(chain, "test prompt").unwrap();
        assert_eq!(resp.text, "sync macro");
    }
}
