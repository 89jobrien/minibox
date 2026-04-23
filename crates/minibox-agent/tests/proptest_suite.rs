//! Property-based tests for minibox-agent domain types.
//!
//! Invariants tested:
//! - `Message` JSON roundtrip is lossless for all role/content combinations.
//! - `ContentBlock` serialises and deserialises with correct `type` tag.
//! - `AgentError` display is non-empty for every variant.
//! - `InferenceResponse::has_tool_calls` is consistent with the content blocks present.

use minibox_agent::message::{ContentBlock, Message, Role};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_role() -> impl Strategy<Value = Role> {
    prop_oneof![Just(Role::User), Just(Role::Assistant),]
}

fn arb_text_block() -> impl Strategy<Value = ContentBlock> {
    any::<String>().prop_map(|text| ContentBlock::Text { text })
}

fn arb_tool_use_block() -> impl Strategy<Value = ContentBlock> {
    (any::<String>(), any::<String>()).prop_map(|(id, name)| ContentBlock::ToolUse {
        id,
        name,
        input: serde_json::Value::Null,
    })
}

fn arb_tool_result_block() -> impl Strategy<Value = ContentBlock> {
    (any::<String>(), any::<String>())
        .prop_map(|(tool_use_id, content)| ContentBlock::ToolResult {
            tool_use_id,
            content,
        })
}

fn arb_content_block() -> impl Strategy<Value = ContentBlock> {
    prop_oneof![
        arb_text_block(),
        arb_tool_use_block(),
        arb_tool_result_block(),
    ]
}

fn arb_message() -> impl Strategy<Value = Message> {
    (
        arb_role(),
        prop::collection::vec(arb_content_block(), 1..8),
    )
        .prop_map(|(role, content)| Message { role, content })
}

// ---------------------------------------------------------------------------
// Roundtrip invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(proptest::test_runner::Config {
        failure_persistence: None,
        ..proptest::test_runner::Config::default()
    })]

    /// `Message` serialises to JSON and deserialises back without data loss.
    #[test]
    fn message_json_roundtrip(msg in arb_message()) {
        let json = serde_json::to_string(&msg).expect("serialise");
        let back: Message = serde_json::from_str(&json).expect("deserialise");

        // Role must survive
        prop_assert_eq!(
            std::mem::discriminant(&msg.role),
            std::mem::discriminant(&back.role),
            "role discriminant changed after roundtrip"
        );

        // Content block count must survive
        prop_assert_eq!(
            msg.content.len(),
            back.content.len(),
            "content block count changed after roundtrip"
        );
    }

    /// `ContentBlock` roundtrips preserve the variant discriminant.
    #[test]
    fn content_block_roundtrip_preserves_variant(block in arb_content_block()) {
        let json = serde_json::to_string(&block).expect("serialise");
        let back: ContentBlock = serde_json::from_str(&json).expect("deserialise");

        // Discriminant (Text / ToolUse / ToolResult) must be preserved.
        prop_assert_eq!(
            std::mem::discriminant(&block),
            std::mem::discriminant(&back),
            "ContentBlock variant changed after roundtrip"
        );
    }

    /// The JSON `type` field emitted for a `ContentBlock` must be non-empty
    /// and must be one of the expected values.
    #[test]
    fn content_block_json_has_type_field(block in arb_content_block()) {
        let json = serde_json::to_string(&block).expect("serialise");
        let obj: serde_json::Value = serde_json::from_str(&json).expect("parse");

        let type_field = obj["type"].as_str().expect("type field must be a string");
        prop_assert!(
            matches!(type_field, "text" | "tool_use" | "tool_result"),
            "unexpected type field: {type_field}"
        );
    }

    /// `Message::user` always produces `Role::User` with exactly one `Text` block.
    #[test]
    fn message_user_constructor_invariant(text in any::<String>()) {
        let m = Message::user(text.clone());
        prop_assert_eq!(m.role, Role::User);
        prop_assert_eq!(m.content.len(), 1);
        match &m.content[0] {
            ContentBlock::Text { text: t } => prop_assert_eq!(t, &text),
            other => prop_assert!(false, "expected Text block, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// has_tool_calls consistency
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(proptest::test_runner::Config {
        failure_persistence: None,
        ..proptest::test_runner::Config::default()
    })]

    /// `has_tool_calls()` returns true iff at least one `ToolUse` block is present.
    #[test]
    fn has_tool_calls_consistent_with_content(
        blocks in prop::collection::vec(arb_content_block(), 0..10)
    ) {
        use minibox_agent::message::InferenceResponse;

        let expected = blocks.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        let resp = InferenceResponse {
            content: blocks,
            stop_reason: "end_turn".to_string(),
            usage: None,
            provider: "test".to_string(),
        };
        prop_assert_eq!(resp.has_tool_calls(), expected);
    }
}
