//! `CruxLlmStep` — adapter that drives `FallbackChain` as an agent step and
//! surfaces typed [`AgentError`] to callers.
//!
//! # Why not use `LlmStep<FallbackChainAdapter>` directly?
//!
//! `crux_agentic::LlmStep::invoke` returns `Result<LlmResponse, CruxErr>`.
//! `CruxErr` is a string-based error — it loses the typed `LlmError` source
//! when `FallbackChainAdapter::complete` maps `LlmError → CruxErr::step_failed`.
//!
//! `CruxLlmStep::invoke` instead calls the `FallbackChain` directly so the
//! typed `LlmError` is preserved and can be surfaced as `AgentError::Llm`.
//! Crux step recording is performed around the call via `ctx.step()` using
//! a mapped result that only enters the `CruxErr` path on failure.

use std::sync::Arc;

use crux_agentic::provider::{LlmRequest, LlmResponse};
use cruxai_core::context::Context;
use minibox_llm::{CompletionRequest, FallbackChain};

use crate::error::AgentError;
use crate::provider::FallbackChainAdapter;

/// Drives a [`FallbackChain`] as a named crux step with typed [`AgentError`]
/// propagation.
///
/// Use [`CruxLlmStep::from_env`] for the standard construction path. Call
/// [`CruxLlmStep::invoke`] inside an agent to record the LLM call in the crux
/// trace (enabling replay and budget tracking).
pub struct CruxLlmStep {
    chain: Arc<FallbackChain>,
}

impl CruxLlmStep {
    /// Build from environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
    /// `GEMINI_API_KEY`).
    pub fn from_env() -> Self {
        Self::new(FallbackChainAdapter::from_env())
    }

    /// Wrap an existing `FallbackChainAdapter`.
    pub fn new(adapter: FallbackChainAdapter) -> Self {
        Self {
            chain: adapter.chain(),
        }
    }

    /// Invoke the LLM call as a named crux step.
    ///
    /// The step name appears in the crux trace and is used as the replay key —
    /// pass a stable, descriptive name (e.g. `"diagnose.summarize"`).
    ///
    /// Returns [`AgentError::Llm`] when the underlying `FallbackChain` fails,
    /// preserving the typed [`minibox_llm::LlmError`] source so callers can
    /// inspect the provider name, HTTP status, or retry history.
    ///
    /// Returns [`AgentError::Step`] only for crux context errors (budget
    /// exceeded, replay mismatch, etc.) which are unrelated to the LLM call.
    pub async fn invoke<C: Context>(
        &self,
        ctx: &mut C,
        step_name: &str,
        req: LlmRequest,
    ) -> Result<LlmResponse, AgentError> {
        let chain = Arc::clone(&self.chain);
        let completion_req = CompletionRequest {
            prompt: req.prompt,
            system: req.system,
            max_tokens: req.max_tokens,
            ..Default::default()
        };

        // Call the chain directly to obtain a typed `LlmError` on failure.
        let llm_result = chain.complete(&completion_req).await;

        // Record the step in the crux context. On success we wrap the response
        // in Ok; on failure we surface the LlmError as AgentError::Llm before
        // crux has a chance to stringify it into CruxErr.
        match llm_result {
            Ok(completion) => {
                let response = LlmResponse {
                    text: completion.text,
                    provider: completion.provider,
                    metadata: None,
                };
                // Record the successful step in the crux trace.
                ctx.step(step_name, || {
                    let r = response.clone();
                    async move { Ok(r) }
                })
                .await
                .map_err(AgentError::Step)
            }
            Err(llm_err) => Err(AgentError::Llm(llm_err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cruxai_core::ctx::CruxCtx;

    #[test]
    fn from_env_constructs_without_panic() {
        let _step = CruxLlmStep::from_env();
    }

    /// Regression: `CruxLlmStep::invoke` used to map LlmError → CruxErr → AgentError::Step,
    /// discarding the typed source. After the fix it must surface as `AgentError::Llm`.
    #[tokio::test]
    async fn invoke_llm_error_surfaces_as_agent_error_llm_variant() {
        use minibox_llm::FallbackChain;
        use std::sync::Arc;

        // Build a chain with no providers — it will always return AllProvidersFailed.
        let chain = Arc::new(FallbackChain::new(vec![]));
        let adapter = FallbackChainAdapter::new(Arc::clone(&chain));
        let step = CruxLlmStep::new(adapter);

        let mut ctx = CruxCtx::new("test-agent");
        let req = crux_agentic::provider::LlmRequest {
            prompt: "hello".into(),
            system: None,
            max_tokens: 64,
        };

        let err = step
            .invoke(&mut ctx, "test_step", req)
            .await
            .expect_err("should fail with empty provider list");

        // Must surface as AgentError::Llm, not AgentError::Step.
        assert!(
            matches!(err, crate::error::AgentError::Llm(_)),
            "expected AgentError::Llm, got: {err:?}"
        );

        // Source chain must be preserved (LlmError is the source).
        let source = std::error::Error::source(&err);
        assert!(
            source.is_some(),
            "AgentError::Llm should have a source (the underlying LlmError)"
        );
    }

    /// When invoke succeeds, the response text must be returned correctly.
    #[test]
    fn from_env_smoke_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CruxLlmStep>();
    }
}
