//! `FallbackChainAdapter` — adapter implementing `crux_agentic::LlmProvider`
//! for minibox-llm's `FallbackChain`.
//!
//! # Port / Adapter placement
//!
//! - Port:    `crux_agentic::LlmProvider` (defined in crux-agentic)
//! - Adapter: `FallbackChainAdapter` (this file) — translates `LlmRequest` →
//!   `CompletionRequest`, calls `FallbackChain::complete`, maps errors to `CruxErr`.

use std::sync::Arc;

use crux_agentic::LlmProvider;
use crux_agentic::provider::{LlmRequest, LlmResponse};
use cruxai_core::prelude::CruxErr;
use minibox_llm::{CompletionRequest, FallbackChain};

/// Adapter: wraps a [`FallbackChain`] and implements [`LlmProvider`].
///
/// Error mapping: `minibox_llm::LlmError → CruxErr::step_failed("minibox_llm", msg)`.
#[derive(Clone)]
pub struct FallbackChainAdapter(Arc<FallbackChain>);

impl FallbackChainAdapter {
    /// Wrap an existing shared `FallbackChain`.
    pub fn new(chain: Arc<FallbackChain>) -> Self {
        Self(chain)
    }

    /// Build from environment variables — reads `ANTHROPIC_API_KEY`,
    /// `OPENAI_API_KEY`, `GEMINI_API_KEY`.
    pub fn from_env() -> Self {
        Self(Arc::new(FallbackChain::from_env()))
    }

    /// Return a shared reference to the inner [`FallbackChain`].
    ///
    /// Used by [`crate::step::CruxLlmStep::invoke`] to call the chain directly
    /// with typed `LlmError` propagation, bypassing the `CruxErr` string boundary.
    pub(crate) fn chain(&self) -> Arc<FallbackChain> {
        Arc::clone(&self.0)
    }
}

impl LlmProvider for FallbackChainAdapter {
    fn complete(
        &self,
        req: LlmRequest,
    ) -> impl std::future::Future<Output = Result<LlmResponse, CruxErr>> + Send {
        let chain = Arc::clone(&self.0);
        async move {
            chain
                .complete(&CompletionRequest {
                    prompt: req.prompt,
                    system: req.system,
                    max_tokens: req.max_tokens,
                    ..Default::default()
                })
                .await
                .map(|r| LlmResponse {
                    text: r.text,
                    provider: r.provider,
                    metadata: None,
                })
                .map_err(|e| CruxErr::step_failed("minibox_llm", e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Smoke test: `FallbackChainAdapter::from_env()` constructs without panic
    /// even when API keys are absent (FallbackChain just has an empty provider list).
    #[test]
    fn from_env_constructs_without_panic() {
        let _adapter = FallbackChainAdapter::from_env();
    }

    /// Verify the adapter implements the `LlmProvider` port.
    /// We can't call `complete` without a live API — just assert Send + Sync.
    #[test]
    fn adapter_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FallbackChainAdapter>();
    }
}
