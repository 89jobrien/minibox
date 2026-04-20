//! `CruxLlmStep` — newtype adapter that bridges `FallbackChainAdapter` into the
//! crux `Context` step machinery via `crux_agentic::LlmStep`.
//!
//! # Why a newtype instead of a type alias?
//!
//! Type aliases cannot have inherent methods in stable Rust, so `from_env()` and
//! `invoke()` would be unreachable through a bare `type CruxLlmStep = ...`. The
//! newtype also hides the generic parameter from callers.

use crux_agentic::LlmStep;
use crux_agentic::provider::{LlmRequest, LlmResponse};
use cruxai_core::context::Context;

use crate::error::AgentError;
use crate::provider::FallbackChainAdapter;

/// Newtype wrapping `LlmStep<FallbackChainAdapter>`.
///
/// Use [`CruxLlmStep::from_env`] for the standard construction path. Call
/// [`CruxLlmStep::invoke`] inside an agent to record the LLM call in the crux
/// trace (enabling replay and budget tracking).
pub struct CruxLlmStep(LlmStep<FallbackChainAdapter>);

impl CruxLlmStep {
    /// Build from environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
    /// `GEMINI_API_KEY`).
    pub fn from_env() -> Self {
        Self(LlmStep::new(FallbackChainAdapter::from_env()))
    }

    /// Wrap an existing `FallbackChainAdapter`.
    pub fn new(adapter: FallbackChainAdapter) -> Self {
        Self(LlmStep::new(adapter))
    }

    /// Invoke the LLM call as a named crux step.
    ///
    /// The step name appears in the crux trace and is used as the replay key —
    /// pass a stable, descriptive name (e.g. `"diagnose.summarize"`).
    ///
    /// Returns [`AgentError`] (not raw `CruxErr`) so the domain boundary is
    /// enforced at the public API surface.
    pub async fn invoke<C: Context>(
        &self,
        ctx: &mut C,
        step_name: &str,
        req: LlmRequest,
    ) -> Result<LlmResponse, AgentError> {
        self.0.invoke(ctx, step_name, req).await.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_constructs_without_panic() {
        let _step = CruxLlmStep::from_env();
    }
}
