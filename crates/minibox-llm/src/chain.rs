use crate::error::LlmError;
use crate::provider::{LlmProvider, ProviderConfig};
use crate::retry::RetryConfig;
use crate::types::{CompletionRequest, CompletionResponse};
use std::sync::OnceLock;

pub struct FallbackChain {
    providers: Vec<Box<dyn LlmProvider>>,
}

impl FallbackChain {
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        Self { providers }
    }

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
    pub fn from_env() -> Self {
        Self::from_env_with_config(ProviderConfig::default(), RetryConfig::default())
    }

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

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

impl FallbackChain {
    /// Blocking wrapper for callers not in an async context.
    ///
    /// # Panics
    ///
    /// Panics if called from within an existing Tokio runtime (e.g. inside
    /// `spawn_blocking`). Use `complete()` directly in async contexts.
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
    use std::sync::Mutex;

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
        // unsafe: Rust 2024 requires unsafe for env mutation. Mutex serializes access.
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("GEMINI_API_KEY");
        }
        let chain = FallbackChain::from_env();
        let result = chain.complete_sync(&request("test"));
        assert!(matches!(result, Err(LlmError::AllProvidersFailed(_))));
    }
}
