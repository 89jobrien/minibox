//! The agentic loop: [`Agent`] drives a tool-use conversation via
//! [`minibox_llm::LlmProvider::infer()`].
//!
//! # Architecture
//!
//! - Port: [`minibox_llm::LlmProvider`] (async inference)
//! - Port: [`crate::tools::ToolExecutor`] (tool execution)
//! - Domain: [`Agent`] — orchestrates the loop, fires events, captures observations
//!
//! The agent never imports infrastructure crates directly; both ports are
//! injected as `Box<dyn …>`.

use async_trait::async_trait;

use crate::message::{ContentBlock, InferenceRequest, InferenceResponse, Message, ToolDefinition};
use minibox_llm::LlmError;

/// Port: an LLM backend that supports multi-turn conversation with tool use.
///
/// This is distinct from [`minibox_llm::LlmProvider`], which is a single-turn
/// text-completion interface. Adapters that wrap a provider supporting structured
/// tool-use (e.g. Anthropic's messages API) implement this trait.
#[async_trait]
pub trait InferenceLlmProvider: Send + Sync {
    /// Human-readable name identifying this provider.
    fn name(&self) -> &str;

    /// Send a multi-turn inference request and return the model's response.
    async fn infer(&self, req: &InferenceRequest) -> Result<InferenceResponse, LlmError>;
}

use crate::events::{Event, EventContext, EventManager};
use crate::observation::{Observation, ObservationManager};
use crate::tools::{ToolExecutor, ToolInput};

/// Hard limit on tool-use rounds per `run_turn()` call to prevent infinite loops.
const MAX_TOOL_ROUNDS: usize = 50;

/// Output from a completed [`Agent::run_turn`] call.
#[derive(Debug)]
pub struct TurnResult {
    /// The final text response from the model (after all tool uses are resolved).
    pub text: String,
    /// All observations captured during this turn.
    pub observations: Vec<Observation>,
}

/// Error from the agent loop.
#[derive(Debug, thiserror::Error)]
pub enum AgentLoopError {
    /// An LLM call failed. Source [`minibox_llm::LlmError`] is preserved.
    #[error("LLM infer failed")]
    Llm(#[from] minibox_llm::LlmError),
    /// A tool call failed. Source [`crate::tools::ToolError`] is preserved.
    #[error("tool error")]
    Tool(#[from] crate::tools::ToolError),
    /// The loop exceeded [`MAX_TOOL_ROUNDS`] without reaching a text response.
    #[error("exceeded max tool rounds ({MAX_TOOL_ROUNDS})")]
    MaxRoundsExceeded,
}

/// Drives the multi-turn agentic loop against an [`LlmProvider`] and a
/// [`ToolExecutor`].
///
/// Constructed via [`Agent::new`]; holds `Box<dyn …>` ports so there is no
/// generic-parameter explosion at call sites.
pub struct Agent {
    llm: Box<dyn InferenceLlmProvider>,
    tools: Box<dyn ToolExecutor>,
    events: EventManager,
    system: Option<String>,
    tool_defs: Vec<ToolDefinition>,
}

impl Agent {
    /// Create a new agent.
    pub fn new(
        llm: Box<dyn InferenceLlmProvider>,
        tools: Box<dyn ToolExecutor>,
        system: Option<String>,
        tool_defs: Vec<ToolDefinition>,
    ) -> Self {
        Self {
            llm,
            tools,
            events: EventManager::new(),
            system,
            tool_defs,
        }
    }

    /// Register an event handler.
    pub fn on_event(&mut self, handler: Box<dyn crate::events::EventHandler>) {
        self.events.register(handler);
    }

    /// Run a single turn of the agentic loop against the given conversation
    /// history. Appends assistant and tool-result messages to `messages` and
    /// returns the final text response.
    ///
    /// Internally loops until the model emits a text-only response (no more
    /// tool uses) or [`MAX_TOOL_ROUNDS`] is exceeded.
    pub async fn run_turn(
        &self,
        messages: &mut Vec<Message>,
        session_id: &str,
        turn: usize,
    ) -> Result<TurnResult, AgentLoopError> {
        let mut obs_mgr = ObservationManager::new();

        let ctx = EventContext {
            session_id: session_id.to_owned(),
            turn,
        };

        for _round in 0..MAX_TOOL_ROUNDS {
            let req = InferenceRequest {
                messages: messages.clone(),
                tools: self.tool_defs.clone(),
                system: self.system.clone(),
                max_tokens: 4096,
                temperature: None,
            };

            let response = self
                .llm
                .infer(&req)
                .await
                .map_err(AgentLoopError::Llm)?;

            // Append the assistant response to history.
            messages.push(Message::assistant(response.content.clone()));

            if !response.has_tool_calls() {
                // No tool calls — collect final text and return.
                let text = response.text();
                return Ok(TurnResult {
                    text,
                    observations: obs_mgr.into_observations(),
                });
            }

            // Process all tool use blocks.
            let mut tool_results: Vec<ContentBlock> = Vec::new();

            for block in &response.content {
                if let ContentBlock::ToolUse { id, name, input } = block {
                    let tool_input = ToolInput {
                        name: name.clone(),
                        args: input.clone(),
                    };

                    self.events.fire(&Event::PreToolUse {
                        ctx: ctx.clone(),
                        input: tool_input.clone(),
                    });

                    let tool_output = self
                        .tools
                        .execute(tool_input.clone())
                        .map_err(AgentLoopError::Tool)?;

                    self.events.fire(&Event::PostToolUse {
                        ctx: ctx.clone(),
                        input: tool_input.clone(),
                        output: tool_output.clone(),
                    });

                    obs_mgr.record(Observation::capture(
                        session_id,
                        turn,
                        tool_input,
                        tool_output.clone(),
                    ));

                    tool_results.push(ContentBlock::ToolResult {
                        tool_use_id: id.clone(),
                        content: tool_output.content,
                    });
                }
            }

            // Append tool results as a user turn.
            messages.push(Message::tool_results(tool_results));
        }

        Err(AgentLoopError::MaxRoundsExceeded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{InMemoryToolExecutor, ToolOutput};
    use crate::message::InferenceResponse;
    use minibox_llm::LlmError;

    use async_trait::async_trait;

    // ── Mock LlmProvider ─────────────────────────────────────────────────────

    /// Returns a scripted sequence of InferenceResponses.
    struct ScriptedLlm {
        responses: std::sync::Mutex<std::collections::VecDeque<InferenceResponse>>,
    }

    impl ScriptedLlm {
        fn new(responses: Vec<InferenceResponse>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses.into()),
            }
        }
    }

    #[async_trait]
    impl InferenceLlmProvider for ScriptedLlm {
        fn name(&self) -> &str {
            "scripted"
        }

        async fn infer(&self, _req: &InferenceRequest) -> Result<InferenceResponse, LlmError> {
            self.responses
                .lock()
                .expect("lock")
                .pop_front()
                .ok_or_else(|| LlmError::AllProvidersFailed("no more responses".into()))
        }
    }

    fn text_response(text: &str) -> InferenceResponse {
        InferenceResponse {
            content: vec![ContentBlock::Text {
                text: text.to_owned(),
            }],
            stop_reason: "end_turn".to_owned(),
            usage: None,
            provider: "scripted".to_owned(),
        }
    }

    fn tool_use_response(id: &str, name: &str, input: serde_json::Value) -> InferenceResponse {
        InferenceResponse {
            content: vec![ContentBlock::ToolUse {
                id: id.to_owned(),
                name: name.to_owned(),
                input,
            }],
            stop_reason: "tool_use".to_owned(),
            usage: None,
            provider: "scripted".to_owned(),
        }
    }

    #[tokio::test]
    async fn run_turn_returns_text_when_no_tool_use() {
        let llm = ScriptedLlm::new(vec![text_response("Hello from model")]);
        let tools = InMemoryToolExecutor::new();
        let agent = Agent::new(Box::new(llm), Box::new(tools), None, vec![]);

        let mut messages = vec![Message::user("hi")];
        let result = agent
            .run_turn(&mut messages, "sess-1", 0)
            .await
            .expect("run_turn");

        assert_eq!(result.text, "Hello from model");
        assert!(result.observations.is_empty());
    }

    #[tokio::test]
    async fn run_turn_executes_tool_and_sends_result_back() {
        // First response: tool use; second: final text
        let llm = ScriptedLlm::new(vec![
            tool_use_response("call-1", "bash", serde_json::json!({"command": "echo hi"})),
            text_response("tool result received"),
        ]);

        let mut tools = InMemoryToolExecutor::new();
        tools.register("bash", ToolOutput::ok("bash", "hi\n"));

        let agent = Agent::new(Box::new(llm), Box::new(tools), None, vec![]);
        let mut messages = vec![Message::user("run bash")];
        let result = agent
            .run_turn(&mut messages, "sess-2", 0)
            .await
            .expect("run_turn");

        assert_eq!(result.text, "tool result received");
        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0].input.name, "bash");
    }

    #[tokio::test]
    async fn llm_error_source_is_preserved_in_agent_loop_error() {
        // Regression: AgentLoopError::Llm used to store String, discarding
        // the upstream source. Verify std::error::Error::source() is non-None.
        let llm_err = LlmError::AllProvidersFailed("timeout".into());
        let loop_err: AgentLoopError = AgentLoopError::from(llm_err);
        let source = std::error::Error::source(&loop_err);
        assert!(
            source.is_some(),
            "AgentLoopError::Llm should preserve upstream source"
        );
        assert!(
            source.unwrap().to_string().contains("timeout"),
            "source should reference original LlmError"
        );
    }

    #[tokio::test]
    async fn tool_error_source_is_preserved_in_agent_loop_error() {
        // Regression: AgentLoopError::Tool used to store String, discarding
        // the upstream ToolError source.
        use crate::tools::ToolError;
        let tool_err = ToolError::UnknownTool("bash".into());
        let loop_err: AgentLoopError = AgentLoopError::from(tool_err);
        let source = std::error::Error::source(&loop_err);
        assert!(
            source.is_some(),
            "AgentLoopError::Tool should preserve upstream source"
        );
    }

    #[tokio::test]
    async fn run_turn_fires_pre_and_post_tool_events() {
        use crate::events::{EventHandler, RecordingHandler};
        use std::sync::Arc;

        struct Delegate(Arc<RecordingHandler>);
        impl EventHandler for Delegate {
            fn handle(&self, event: &Event) {
                self.0.handle(event);
            }
        }

        let llm = ScriptedLlm::new(vec![
            tool_use_response("id-1", "bash", serde_json::json!({"command": "echo"})),
            text_response("done"),
        ]);
        let mut tools = InMemoryToolExecutor::new();
        tools.register("bash", ToolOutput::ok("bash", ""));

        let handler = Arc::new(RecordingHandler::new());
        let mut agent = Agent::new(Box::new(llm), Box::new(tools), None, vec![]);
        agent.on_event(Box::new(Delegate(Arc::clone(&handler))));

        let mut messages = vec![Message::user("go")];
        agent
            .run_turn(&mut messages, "sess-3", 0)
            .await
            .expect("ok");

        let events = handler.recorded();
        assert!(
            events.iter().any(|e| matches!(e, Event::PreToolUse { .. })),
            "should have PreToolUse"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::PostToolUse { .. })),
            "should have PostToolUse"
        );
    }
}
