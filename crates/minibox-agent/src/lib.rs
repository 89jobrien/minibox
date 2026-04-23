//! minibox-agent — crux-backed agent execution layer for minibox.
//!
//! Bridges minibox-llm's `FallbackChain` into crux's `LlmProvider` / `LlmStep`
//! model, giving minibox agents replay, budget tracking, and structured tracing
//! for free.
//!
//! # Architecture
//!
//! ```text
//! minibox-agent (this crate)
//! ├── Port:    crux_agentic::LlmProvider  (from crux-agentic 0.2.3)
//! ├── Adapter: FallbackChainAdapter       (wraps FallbackChain as LlmProvider)
//! ├── Step:    CruxLlmStep               (newtype over LlmStep<FallbackChainAdapter>)
//! └── Error:   AgentError               (domain error; maps LlmError + CruxErr)
//! ```
//!
//! # Quick start
//!
//! ```ignore
//! use minibox_agent::{AgentError, CruxLlmStep, LlmRequest, CruxCtx};
//!
//! async fn summarize(ctx: &mut CruxCtx, input: String) -> Result<String, AgentError> {
//!     let step = CruxLlmStep::from_env();
//!     let resp = step.invoke(ctx, "summarize.call", LlmRequest {
//!         prompt: input,
//!         ..Default::default()
//!     }).await?;
//!     Ok(resp.text)
//! }
//! ```

pub mod agent;
pub mod trajectory;
pub mod conversation;
pub mod error;
pub mod events;
pub mod hooks;
pub mod message;
pub mod observation;
pub mod provider;
pub mod session_log;
pub mod step;
pub mod tools;
pub mod trace;

pub use trace::FileTraceStore;

// Re-export the crux-agentic public surface so consumers only need minibox-agent.
pub use crux_agentic::{LlmProvider, LlmRequest, LlmResponse, LlmStep};

// Domain types from this crate.
pub use error::AgentError;
pub use provider::FallbackChainAdapter;
pub use step::CruxLlmStep;

// Re-export crux-core types for one-dep convenience.
pub use cruxai_core::ctx::CruxCtx;
