//! Error types for minibox-agent.
//!
//! `AgentError` is the domain error type. Infrastructure adapters map their
//! own error types into `AgentError` — crux's `CruxErr` and minibox-llm's
//! `LlmError` are both converted here so the domain stays clean.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("LLM call failed: {0}")]
    Llm(String),

    #[error("agent step failed: {0}")]
    Step(String),

    #[error("budget exceeded")]
    BudgetExceeded,

    #[error("agent error: {0}")]
    Other(String),
}

impl From<minibox_llm::LlmError> for AgentError {
    fn from(e: minibox_llm::LlmError) -> Self {
        AgentError::Llm(e.to_string())
    }
}

impl From<cruxai_core::types::error::CruxErr> for AgentError {
    fn from(e: cruxai_core::types::error::CruxErr) -> Self {
        AgentError::Step(e.to_string())
    }
}
