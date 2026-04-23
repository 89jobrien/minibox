//! Property-based tests for minibox-llm retry and fallback logic.
//!
//! Invariants that must hold for arbitrary inputs:
//! - `FallbackChain::complete` tries all providers if all fail.
//! - `FallbackChain::complete` short-circuits on first success.
//! - `RetryingProvider` retries transient errors up to the configured limit.
//! - `RetryingProvider` returns permanent errors immediately without retry.
//!
//! No network, no API calls.

use async_trait::async_trait;
use minibox_llm::chain::FallbackChain;
use minibox_llm::error::{HttpStatusError, LlmError};
use minibox_llm::provider::LlmProvider;
use minibox_llm::retry::{RetryConfig, RetryingProvider};
use minibox_llm::types::{CompletionRequest, CompletionResponse};
use proptest::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Mock provider for property tests
// ---------------------------------------------------------------------------

struct CountingTransientProvider {
    name: String,
    /// Number of times to fail transiently before succeeding
    fail_count: usize,
    /// Number of times complete() has been called
    call_count: Arc<AtomicUsize>,
}

impl CountingTransientProvider {
    fn new(name: impl Into<String>, fail_count: usize) -> Self {
        Self {
            name: name.into(),
            fail_count,
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        self.call_count.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl LlmProvider for CountingTransientProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, _request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let current_call = self.call_count.fetch_add(1, Ordering::Relaxed);
        if current_call < self.fail_count {
            // Return transient error (HTTP 503) that should be retried
            Err(LlmError::ProviderError {
                provider: self.name.clone(),
                source: Box::new(HttpStatusError {
                    status: 503,
                    body: "service unavailable".to_string(),
                }),
            })
        } else {
            Ok(CompletionResponse {
                text: "success".to_string(),
                provider: self.name.clone(),
                usage: None,
            })
        }
    }
}

struct PermanentFailProvider {
    name: String,
    call_count: Arc<AtomicUsize>,
}

impl PermanentFailProvider {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        self.call_count.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl LlmProvider for PermanentFailProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(&self, _request: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        // Return a permanent error (schema parse error)
        Err(LlmError::SchemaParseError("bad schema".to_string()))
    }
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        cases: 32,
        ..ProptestConfig::default()
    })]

    /// `FallbackChain::complete` tries all providers in order when all fail permanently.
    #[test]
    fn fallback_chain_tries_all_providers_on_permanent_failure(
        num_providers in 1usize..=5usize
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut providers: Vec<Box<dyn LlmProvider>> = Vec::new();
            let mut call_counts = Vec::new();

            for i in 0..num_providers {
                let provider = PermanentFailProvider::new(format!("provider-{}", i));
                let count = provider.call_count.clone();
                call_counts.push(count);
                providers.push(Box::new(provider));
            }

            let chain = FallbackChain::new(providers);
            let request = CompletionRequest::default();

            let response = chain.complete(&request).await;
            // All should fail
            prop_assert!(response.is_err());

            // Each provider should have been tried exactly once
            for (i, count) in call_counts.iter().enumerate() {
                prop_assert_eq!(
                    count.load(Ordering::Relaxed), 1,
                    "provider {} should be tried exactly once", i
                );
            }
            Ok(())
        })?;
    }

    /// `FallbackChain::complete` short-circuits on first success.
    #[test]
    fn fallback_chain_short_circuits_on_first_success(
        num_providers in 2usize..=10usize
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Build the success provider first, at index 0.
            let success_provider = CountingTransientProvider::new("success", 0);
            let success_count = success_provider.call_count.clone();

            // Build fail providers for indices 1..num_providers, tracking their counts.
            let mut fail_counts = Vec::new();
            let mut providers: Vec<Box<dyn LlmProvider>> = vec![Box::new(success_provider)];
            for i in 1..num_providers {
                let provider = PermanentFailProvider::new(format!("provider-{}", i));
                fail_counts.push(provider.call_count.clone());
                providers.push(Box::new(provider));
            }

            let chain = FallbackChain::new(providers);
            let request = CompletionRequest::default();

            let response = chain.complete(&request).await;
            prop_assert!(response.is_ok());

            // Only the first provider (success) should have been called.
            prop_assert_eq!(success_count.load(Ordering::Relaxed), 1);

            // Remaining fail providers must not have been called.
            for count in fail_counts.iter() {
                prop_assert_eq!(count.load(Ordering::Relaxed), 0);
            }
            Ok(())
        })?;
    }

    /// `RetryingProvider` retries transient errors up to the configured limit.
    #[test]
    fn retrying_provider_retries_transient_errors(
        fail_count in 1usize..=5usize,
        max_retries in 0u32..=10u32
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let inner = CountingTransientProvider::new("test", fail_count);
            let call_count = inner.call_count.clone();

            let retrying = RetryingProvider::new(
                inner,
                RetryConfig {
                    max_retries,
                    backoff_base: std::time::Duration::from_millis(0),
                },
            );

            let request = CompletionRequest::default();

            // If fail_count <= max_retries, we should eventually succeed
            if fail_count as u32 <= max_retries {
                let response = retrying.complete(&request).await;
                prop_assert!(response.is_ok(), "should succeed after retries");
                // Should have called the inner provider fail_count + 1 times
                // (fail_count failures + 1 successful attempt)
                let expected_calls = fail_count + 1;
                let actual_calls = call_count.load(Ordering::Relaxed);
                prop_assert_eq!(
                    actual_calls as usize,
                    expected_calls,
                    "should call provider fail_count+1 times to reach success"
                );
            } else {
                // Too many failures, should eventually give up
                let response = retrying.complete(&request).await;
                prop_assert!(response.is_err(), "should fail if retries exhausted");
                // Should have called at most max_retries + 1 times
                let actual_calls = call_count.load(Ordering::Relaxed) as u32;
                prop_assert!(
                    actual_calls <= max_retries + 1,
                    "should call at most max_retries+1 times"
                );
            }
            Ok(())
        })?;
    }

    /// `RetryingProvider` returns permanent errors immediately without retrying.
    #[test]
    fn retrying_provider_permanent_errors_no_retry(
        max_retries in 0u32..=10u32
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let provider = PermanentFailProvider::new("test");
            let call_count = provider.call_count.clone();

            let retrying = RetryingProvider::new(
                provider,
                RetryConfig {
                    max_retries,
                    backoff_base: std::time::Duration::from_millis(0),
                },
            );

            let request = CompletionRequest::default();
            let response = retrying.complete(&request).await;

            prop_assert!(response.is_err());
            // Should have tried exactly once (no retries for permanent errors)
            prop_assert_eq!(
                call_count.load(Ordering::Relaxed), 1,
                "permanent errors should not be retried"
            );
            Ok(())
        })?;
    }

    /// `FallbackChain::complete` returns `AllProvidersFailed` when all providers fail.
    #[test]
    fn fallback_chain_all_providers_failed_error(
        num_providers in 1usize..=5usize
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let providers: Vec<Box<dyn LlmProvider>> = (0..num_providers)
                .map(|i| Box::new(PermanentFailProvider::new(format!("provider-{}", i))) as Box<dyn LlmProvider>)
                .collect();

            let chain = FallbackChain::new(providers);
            let request = CompletionRequest::default();

            let response = chain.complete(&request).await;
            match response {
                Err(LlmError::AllProvidersFailed(msg)) => {
                    // Message should contain all provider names
                    for i in 0..num_providers {
                        prop_assert!(
                            msg.contains(&format!("provider-{}", i)),
                            "error message should contain all provider names"
                        );
                    }
                    Ok(())
                }
                _ => {
                    prop_assert!(false, "expected AllProvidersFailed error");
                    Ok(())
                }
            }
        })?;
    }
}
