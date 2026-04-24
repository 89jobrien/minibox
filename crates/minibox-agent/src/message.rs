//! Domain types for multi-turn LLM conversations with tool use.
//!
//! Types are re-exported from `minibox-llm` so that `minibox-agent` consumers
//! do not need to depend on `minibox-llm` directly for message types.

pub use minibox_llm::{
    ContentBlock, InferenceRequest, InferenceResponse, Message, Role, StopReason, ToolDefinition,
};

/// Alias for `minibox_llm::Usage` used in inference context.
pub use minibox_llm::Usage as InferenceUsage;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_user_creates_user_role() {
        let m = Message::user("hello");
        assert_eq!(m.role, Role::User);
        assert!(matches!(&m.content[0], ContentBlock::Text { text } if text == "hello"));
    }

    #[test]
    fn message_assistant_creates_assistant_role() {
        let m = Message::assistant(vec![ContentBlock::Text { text: "hi".into() }]);
        assert_eq!(m.role, Role::Assistant);
    }

    #[test]
    fn inference_response_has_tool_calls_true_when_tool_use_present() {
        let resp = InferenceResponse {
            content: vec![ContentBlock::ToolUse {
                id: "1".into(),
                name: "bash".into(),
                input: serde_json::json!({}),
            }],
            stop_reason: StopReason::ToolUse,
            usage: None,
            provider: "test".into(),
        };
        assert!(resp.has_tool_calls());
    }

    #[test]
    fn inference_response_text_extracts_text_blocks() {
        let resp = InferenceResponse {
            content: vec![ContentBlock::Text {
                text: "hello world".into(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: None,
            provider: "test".into(),
        };
        assert_eq!(resp.text(), "hello world");
    }

    #[test]
    fn inference_response_has_tool_calls_false_for_text_only() {
        let resp = InferenceResponse {
            content: vec![ContentBlock::Text {
                text: "done".into(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: None,
            provider: "test".into(),
        };
        assert!(!resp.has_tool_calls());
    }
}
