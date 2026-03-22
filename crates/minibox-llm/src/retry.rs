use std::time::Duration;

use async_trait::async_trait;

use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse};

/// Retry policy for transient LLM errors.
///
/// The retry delay follows an exponential backoff formula capped at 30 seconds:
///
/// ```text
/// delay(attempt) = min(backoff_base * 2^attempt, 30s)
/// ```
///
/// Where `attempt` is zero-indexed (the first retry uses `backoff_base * 2^0 = backoff_base`).
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts after the initial failure. A value of
    /// `2` means at most 3 total calls (1 initial + 2 retries). Defaults to `2`.
    pub max_retries: u32,

    /// Starting delay for exponential backoff. Each subsequent retry doubles
    /// this value, capped at 30 seconds. Defaults to `1s`.
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

/// Wraps any [`LlmProvider`] with exponential-backoff retry logic for transient errors.
///
/// Only errors for which [`LlmError::is_transient`] returns `true` are retried
/// (HTTP 429, 5xx, and network-level timeouts/connection failures). Permanent
/// errors (4xx, schema parse failures) are returned immediately without retry.
///
/// The retry limit can be overridden on a per-request basis via
/// [`CompletionRequest::max_retries`], which takes precedence over the
/// [`RetryConfig`] supplied at construction time.
///
/// # Backoff formula
///
/// ```text
/// delay(attempt) = min(backoff_base * 2^attempt, 30s)
/// ```
///
/// The first retry waits `backoff_base`, the second `backoff_base * 2`, and so on.
pub struct RetryingProvider<P: LlmProvider> {
    /// The wrapped inner provider. Exposed as `pub(crate)` for test inspection.
    pub(crate) inner: P,
    /// Retry policy applied when the inner provider returns a transient error.
    config: RetryConfig,
}

impl<P: LlmProvider> RetryingProvider<P> {
    /// Wrap `inner` with the given retry policy.
    pub fn new(inner: P, config: RetryConfig) -> Self {
        Self { inner, config }
    }
}

#[async_trait]
impl<P: LlmProvider> LlmProvider for RetryingProvider<P> {
    /// Delegates to the inner provider's name.
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// Attempt the completion, retrying on transient errors up to `max_retries` times.
    ///
    /// The effective retry limit is `request.max_retries.unwrap_or(config.max_retries)`.
    /// Between attempts the task sleeps for the exponential-backoff delay, which is
    /// computed as `min(backoff_base * 2^attempt, 30s)`.
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

    /// A test provider that fails a configurable number of times before succeeding.
    struct CountingProvider {
        call_count: AtomicU32,
        /// Number of leading calls that return an error.
        fail_times: u32,
        /// Whether failing calls use a transient (503) or permanent (401) status.
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

        /// Return the total number of times `complete` has been called.
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
