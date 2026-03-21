use std::time::Duration;

use async_trait::async_trait;

use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse};

/// Retry configuration for transient error handling.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub backoff_base: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            backoff_base: Duration::from_secs(1),
        }
    }
}

/// Wraps any `LlmProvider` with retry logic for transient errors.
pub struct RetryingProvider<P: LlmProvider> {
    pub(crate) inner: P,
    config: RetryConfig,
}

impl<P: LlmProvider> RetryingProvider<P> {
    pub fn new(inner: P, config: RetryConfig) -> Self {
        Self { inner, config }
    }
}

#[async_trait]
impl<P: LlmProvider> LlmProvider for RetryingProvider<P> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let max = request.max_retries.unwrap_or(self.config.max_retries);
        let backoff_cap = Duration::from_secs(30);

        for attempt in 0..=max {
            match self.inner.complete(request).await {
                Ok(resp) => return Ok(resp),
                Err(e) if e.is_transient() && attempt < max => {
                    let delay =
                        (self.config.backoff_base * 2u32.saturating_pow(attempt)).min(backoff_cap);
                    tracing::warn!(
                        provider = self.inner.name(),
                        attempt = attempt + 1,
                        delay_ms = delay.as_millis() as u64,
                        error = %e,
                        "llm: transient error, retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("loop always returns: 0..=max covers all attempts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::HttpStatusError;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingProvider {
        call_count: AtomicU32,
        fail_times: u32,
        transient: bool,
    }

    impl CountingProvider {
        fn new(fail_times: u32, transient: bool) -> Self {
            Self {
                call_count: AtomicU32::new(0),
                fail_times,
                transient,
            }
        }

        fn calls(&self) -> u32 {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl LlmProvider for CountingProvider {
        fn name(&self) -> &str {
            "counting"
        }

        async fn complete(
            &self,
            _request: &CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_times {
                let status = if self.transient { 503 } else { 401 };
                Err(LlmError::ProviderError {
                    provider: "counting".to_string(),
                    source: Box::new(HttpStatusError {
                        status,
                        body: "fail".to_string(),
                    }),
                })
            } else {
                Ok(CompletionResponse {
                    text: "ok".to_string(),
                    provider: "counting".to_string(),
                    usage: None,
                })
            }
        }
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            prompt: "test".to_string(),
            ..CompletionRequest::default()
        }
    }

    #[tokio::test]
    async fn no_retry_on_success() {
        let provider = CountingProvider::new(0, true);
        let retrying = RetryingProvider::new(
            provider,
            RetryConfig {
                max_retries: 2,
                backoff_base: Duration::from_millis(1),
            },
        );
        let resp = retrying.complete(&req()).await.unwrap();
        assert_eq!(resp.text, "ok");
        assert_eq!(retrying.inner.calls(), 1);
    }

    #[tokio::test]
    async fn retries_transient_errors() {
        let provider = CountingProvider::new(2, true); // fail twice, succeed third
        let retrying = RetryingProvider::new(
            provider,
            RetryConfig {
                max_retries: 2,
                backoff_base: Duration::from_millis(1),
            },
        );
        let resp = retrying.complete(&req()).await.unwrap();
        assert_eq!(resp.text, "ok");
        assert_eq!(retrying.inner.calls(), 3);
    }

    #[tokio::test]
    async fn no_retry_on_permanent_error() {
        let provider = CountingProvider::new(1, false); // permanent error
        let retrying = RetryingProvider::new(
            provider,
            RetryConfig {
                max_retries: 2,
                backoff_base: Duration::from_millis(1),
            },
        );
        let result = retrying.complete(&req()).await;
        assert!(result.is_err());
        assert_eq!(retrying.inner.calls(), 1); // no retry
    }

    #[tokio::test]
    async fn exhausts_retries_on_persistent_transient_error() {
        let provider = CountingProvider::new(10, true); // always fails
        let retrying = RetryingProvider::new(
            provider,
            RetryConfig {
                max_retries: 2,
                backoff_base: Duration::from_millis(1),
            },
        );
        let result = retrying.complete(&req()).await;
        assert!(result.is_err());
        assert_eq!(retrying.inner.calls(), 3); // 1 initial + 2 retries
    }

    #[tokio::test]
    async fn request_level_retry_override() {
        let provider = CountingProvider::new(10, true);
        let retrying = RetryingProvider::new(
            provider,
            RetryConfig {
                max_retries: 5,
                backoff_base: Duration::from_millis(1),
            },
        );
        let mut r = req();
        r.max_retries = Some(1); // override: only 1 retry
        let result = retrying.complete(&r).await;
        assert!(result.is_err());
        assert_eq!(retrying.inner.calls(), 2); // 1 initial + 1 retry
    }

    #[tokio::test]
    async fn name_delegates_to_inner() {
        let provider = CountingProvider::new(0, true);
        let retrying = RetryingProvider::new(provider, RetryConfig::default());
        assert_eq!(retrying.name(), "counting");
    }
}
