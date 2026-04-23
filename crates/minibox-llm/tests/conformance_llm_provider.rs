//! Conformance tests for the LlmProvider trait contract.
//!
//! Verifies:
//! - Any `LlmProvider` impl returns a non-empty name.
//! - A mock provider can be constructed and used in a FallbackChain.
//! - Provider errors are properly propagated through the chain.
//! - All providers in a chain are tried in order on error.
//! - A successful provider response short-circuits the chain.
//!
//! No network, no API keys required.

use async_trait::async_trait;
use minibox_llm::error::LlmError;
use minibox_llm::provider::LlmProvider;
use minibox_llm::types::{CompletionRequest, CompletionResponse};
use minibox_llm::chain::FallbackChain;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Mock provider that tracks call count
// ---------------------------------------------------------------------------

struct CountingProvider {
    name: String,
    /// Number of times complete() has been called
    call_count: Arc<AtomicUsize>,
    /// If Some, return this error; if None, return success
    should_fail: bool,
}

impl CountingProvider {
    fn new(name: impl Into<String>, should_fail: bool) -> Self {
        Self {
            name: name.into(),
            call_count: Arc::new(AtomicUsize::new(0)),
            should_fail,
        }
    }

    fn call_count(&self) -> usize {
        self.call_count.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl LlmProvider for CountingProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn complete(
        &self,
        _request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail {
            Err(LlmError::ProviderError {
                provider: self.name.clone(),
                source: Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "test error",
                )),
            })
        } else {
            Ok(CompletionResponse {
                text: "test response".to_string(),
                provider: self.name.clone(),
                usage: Some(minibox_llm::types::Usage {
                    input_tokens: 10,
                    output_tokens: 20,
                }),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// LlmProvider trait contract tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn llm_provider_name_is_non_empty() {
    let provider = CountingProvider::new("test-provider", false);
    assert!(!provider.name().is_empty());
}

#[tokio::test]
async fn llm_provider_name_reflects_construction_parameter() {
    let name = "my-llm-backend";
    let provider = CountingProvider::new(name, false);
    assert_eq!(provider.name(), name);
}

#[tokio::test]
async fn llm_provider_complete_success_returns_response() {
    let provider = CountingProvider::new("test", false);
    let request = CompletionRequest {
        prompt: "hello".to_string(),
        system: None,
        max_tokens: 100,
        timeout: None,
        max_retries: None,
        schema: None,
    };
    let response = provider.complete(&request).await;
    assert!(response.is_ok());
    let resp = response.unwrap();
    assert_eq!(resp.text, "test response");
}

#[tokio::test]
async fn llm_provider_complete_failure_returns_error() {
    let provider = CountingProvider::new("test", true);
    let request = CompletionRequest {
        prompt: "hello".to_string(),
        system: None,
        max_tokens: 100,
        timeout: None,
        max_retries: None,
        schema: None,
    };
    let response = provider.complete(&request).await;
    assert!(response.is_err());
}

#[tokio::test]
async fn llm_provider_complete_tracks_invocations() {
    let provider = CountingProvider::new("test", false);
    let request = CompletionRequest {
        prompt: "hello".to_string(),
        system: None,
        max_tokens: 100,
        timeout: None,
        max_retries: None,
        schema: None,
    };
    assert_eq!(provider.call_count(), 0);
    let _ = provider.complete(&request).await;
    assert_eq!(provider.call_count(), 1);
    let _ = provider.complete(&request).await;
    assert_eq!(provider.call_count(), 2);
}

// ---------------------------------------------------------------------------
// FallbackChain contract tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fallback_chain_first_success_returns_immediately() {
    let p1 = CountingProvider::new("first", false);
    let first_call_count = p1.call_count.clone();
    let p2 = CountingProvider::new("second", false);
    let second_call_count = p2.call_count.clone();

    let chain = FallbackChain::new(vec![
        Box::new(p1),
        Box::new(p2),
    ]);

    let request = CompletionRequest {
        prompt: "test".to_string(),
        system: None,
        max_tokens: 100,
        timeout: None,
        max_retries: None,
        schema: None,
    };

    let response = chain.complete(&request).await;
    assert!(response.is_ok());
    // First provider should be called exactly once
    assert_eq!(first_call_count.load(Ordering::Relaxed), 1);
    // Second provider should never be called
    assert_eq!(second_call_count.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn fallback_chain_tries_next_on_failure() {
    let p1 = CountingProvider::new("first", true);
    let first_call_count = p1.call_count.clone();
    let p2 = CountingProvider::new("second", false);
    let second_call_count = p2.call_count.clone();

    let chain = FallbackChain::new(vec![
        Box::new(p1),
        Box::new(p2),
    ]);

    let request = CompletionRequest {
        prompt: "test".to_string(),
        system: None,
        max_tokens: 100,
        timeout: None,
        max_retries: None,
        schema: None,
    };

    let response = chain.complete(&request).await;
    assert!(response.is_ok());
    // First provider should fail once
    assert_eq!(first_call_count.load(Ordering::Relaxed), 1);
    // Second provider should be called as fallback
    assert_eq!(second_call_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn fallback_chain_all_failures_returns_all_providers_failed() {
    let p1 = CountingProvider::new("first", true);
    let p2 = CountingProvider::new("second", true);

    let chain = FallbackChain::new(vec![
        Box::new(p1),
        Box::new(p2),
    ]);

    let request = CompletionRequest {
        prompt: "test".to_string(),
        system: None,
        max_tokens: 100,
        timeout: None,
        max_retries: None,
        schema: None,
    };

    let response = chain.complete(&request).await;
    assert!(matches!(response, Err(LlmError::AllProvidersFailed(_))));
    if let Err(LlmError::AllProvidersFailed(msg)) = response {
        assert!(msg.contains("first"));
        assert!(msg.contains("second"));
    }
}

#[tokio::test]
async fn fallback_chain_empty_chain_returns_all_providers_failed() {
    let chain: FallbackChain = FallbackChain::new(vec![]);

    let request = CompletionRequest {
        prompt: "test".to_string(),
        system: None,
        max_tokens: 100,
        timeout: None,
        max_retries: None,
        schema: None,
    };

    let response = chain.complete(&request).await;
    assert!(matches!(response, Err(LlmError::AllProvidersFailed(_))));
}

#[tokio::test]
async fn fallback_chain_response_contains_provider_name() {
    let p = CountingProvider::new("my-test-provider", false);
    let chain = FallbackChain::new(vec![Box::new(p)]);

    let request = CompletionRequest {
        prompt: "test".to_string(),
        system: None,
        max_tokens: 100,
        timeout: None,
        max_retries: None,
        schema: None,
    };

    let response = chain.complete(&request).await.unwrap();
    assert_eq!(response.provider, "my-test-provider");
}

#[tokio::test]
async fn fallback_chain_tries_all_providers_in_order() {
    let p1 = CountingProvider::new("first", true);
    let first_calls = p1.call_count.clone();
    let p2 = CountingProvider::new("second", true);
    let second_calls = p2.call_count.clone();
    let p3 = CountingProvider::new("third", true);
    let third_calls = p3.call_count.clone();

    let chain = FallbackChain::new(vec![
        Box::new(p1),
        Box::new(p2),
        Box::new(p3),
    ]);

    let request = CompletionRequest {
        prompt: "test".to_string(),
        system: None,
        max_tokens: 100,
        timeout: None,
        max_retries: None,
        schema: None,
    };

    let response = chain.complete(&request).await;
    assert!(response.is_err());

    // All three should have been tried exactly once
    assert_eq!(first_calls.load(Ordering::Relaxed), 1);
    assert_eq!(second_calls.load(Ordering::Relaxed), 1);
    assert_eq!(third_calls.load(Ordering::Relaxed), 1);
}
