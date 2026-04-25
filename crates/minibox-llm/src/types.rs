use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::Assistant,
            content,
        }
    }

    pub fn tool_results(results: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::User,
            content: results,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct InferenceRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub system: Option<String>,
    pub max_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct InferenceResponse {
    pub content: Vec<ContentBlock>,
}

impl InferenceResponse {
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn has_tool_calls(&self) -> bool {
        self.content
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
    }
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prompt: String,
    pub system: Option<String>,
    pub max_tokens: u32,
    pub schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_block_text_serde_roundtrip() {
        let block = ContentBlock::Text {
            text: "hello".to_string(),
        };
        let json = serde_json::to_string(&block).unwrap();
        let deserialized: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, deserialized);
    }

    #[test]
    fn content_block_tool_use_serde_roundtrip() {
        let block = ContentBlock::ToolUse {
            id: "t1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "ls"}),
        };
        let json = serde_json::to_string(&block).unwrap();
        let deserialized: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(block, deserialized);
    }

    #[test]
    fn message_user_constructor() {
        let msg = Message::user("hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.len(), 1);
        assert_eq!(
            msg.content[0],
            ContentBlock::Text {
                text: "hello".to_string()
            }
        );
    }

    #[test]
    fn inference_response_text_concatenates() {
        let resp = InferenceResponse {
            content: vec![
                ContentBlock::Text {
                    text: "hello ".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "t1".to_string(),
                    name: "x".to_string(),
                    input: serde_json::Value::Null,
                },
                ContentBlock::Text {
                    text: "world".to_string(),
                },
            ],
        };
        assert_eq!(resp.text(), "hello world");
    }

    #[test]
    fn inference_response_has_tool_calls() {
        let with_tools = InferenceResponse {
            content: vec![ContentBlock::ToolUse {
                id: "t1".to_string(),
                name: "bash".to_string(),
                input: serde_json::Value::Null,
            }],
        };
        assert!(with_tools.has_tool_calls());

        let without_tools = InferenceResponse {
            content: vec![ContentBlock::Text {
                text: "hi".to_string(),
            }],
        };
        assert!(!without_tools.has_tool_calls());
    }
}
