use crate::error::LlmError;
use crate::provider::LlmProvider;
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

    fn request(prompt: &str) -> CompletionRequest {
        CompletionRequest {
            prompt: prompt.to_string(),
            system: None,
            max_tokens: 100,
            schema: None,
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
}
