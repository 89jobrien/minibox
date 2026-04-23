//! Error types for minibox-agent.
//!
//! `AgentError` is the domain error type. Infrastructure adapters map their
//! own error types into `AgentError` — crux's `CruxErr` and minibox-llm's
//! `LlmError` are both converted here so the domain stays clean.
//!
//! Source errors are preserved (not stringified) so callers can downcast,
//! pattern-match, or inspect the original failure.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    /// An LLM call failed. The source [`LlmError`](minibox_llm::LlmError)
    /// is preserved — downcast to inspect provider name, HTTP status, etc.
    #[error("LLM call failed")]
    Llm(#[from] minibox_llm::LlmError),

    /// A crux agent step failed. The source
    /// [`CruxErr`](cruxai_core::types::error::CruxErr) is preserved.
    #[error("agent step failed")]
    Step(#[from] cruxai_core::types::error::CruxErr),

}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_llm::LlmError;

    #[test]
    fn from_llm_error_preserves_source() {
        let llm_err = LlmError::AllProvidersFailed("claude: timeout".into());
        let agent_err = AgentError::from(llm_err);

        assert!(matches!(agent_err, AgentError::Llm(_)));
        // Display includes thiserror message
        let msg = agent_err.to_string();
        assert_eq!(msg, "LLM call failed");

        // Source chain preserves the original
        let source = std::error::Error::source(&agent_err).unwrap();
        assert!(
            source.to_string().contains("claude: timeout"),
            "source should contain original message, got: {source}"
        );
    }

    #[test]
    fn from_llm_error_downcast() {
        let llm_err = LlmError::SchemaParseError("bad json".into());
        let agent_err = AgentError::from(llm_err);

        match &agent_err {
            AgentError::Llm(inner) => {
                assert!(matches!(inner, LlmError::SchemaParseError(_)));
            }
            _ => panic!("expected Llm variant"),
        }
    }

    #[test]
    fn from_crux_err_preserves_source() {
        let crux_err =
            cruxai_core::types::error::CruxErr::step_failed("test_step", "something broke");
        let agent_err = AgentError::from(crux_err);

        assert!(matches!(agent_err, AgentError::Step(_)));
        let source = std::error::Error::source(&agent_err).unwrap();
        assert!(
            source.to_string().contains("something broke"),
            "source should contain original message, got: {source}"
        );
    }

    #[test]
    fn debug_format_is_useful() {
        let llm_err = LlmError::AllProvidersFailed("openai: 429".into());
        let agent_err = AgentError::from(llm_err);
        let debug = format!("{agent_err:?}");
        assert!(
            debug.contains("AllProvidersFailed"),
            "Debug should show inner variant, got: {debug}"
        );
    }

    /// Every variant in AgentError must have a construction site.
    /// This test will fail to compile if speculative variants (BudgetExceeded,
    /// Other) are present but not constructible from domain logic.
    #[test]
    fn no_unused_variants_budget_exceeded() {
        // Confirm BudgetExceeded does NOT exist by exhaustively matching.
        // If this test fails to compile, the variant was re-added without a
        // construction site — remove it or wire it properly.
        let err: AgentError = AgentError::Llm(LlmError::AllProvidersFailed("x".into()));
        match err {
            AgentError::Llm(_) => {}
            AgentError::Step(_) => {}
            // No BudgetExceeded, no Other — exhaustive match proves it.
        }
    }
}
