//! Conversation history and `@file` reference resolution.
//!
//! [`Conversation`] holds an ordered list of [`minibox_llm::Message`] turns and
//! provides helpers for appending user/assistant messages. `@filename` tokens in
//! user message text are expanded to the file's contents before being sent to the
//! model.

use crate::message::{ContentBlock, Message, Role};

/// Manages the ordered list of messages for a multi-turn conversation.
#[derive(Debug, Default)]
pub struct Conversation {
    messages: Vec<Message>,
}

impl Conversation {
    /// Create an empty conversation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a pre-built message.
    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Append a user turn, resolving any `@filename` references in `text`.
    ///
    /// Tokens of the form `@path/to/file` are replaced with the file's contents
    /// wrapped in a markdown code fence. Unreadable files are left as-is.
    pub fn push_user(&mut self, text: impl Into<String>) {
        let resolved = resolve_file_refs(text.into());
        self.messages.push(Message::user(resolved));
    }

    /// Append an assistant turn from raw text.
    pub fn push_assistant(&mut self, text: impl Into<String>) {
        self.messages
            .push(Message::assistant(vec![ContentBlock::Text {
                text: text.into(),
            }]));
    }

    /// Return a reference to the full message list.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Return the role of the last message, if any.
    pub fn last_role(&self) -> Option<&Role> {
        self.messages.last().map(|m| &m.role)
    }

    /// Number of turns in the conversation.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// `true` if there are no messages.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// Replace `@filename` tokens in `text` with the file's contents.
///
/// Tokens must be word-boundary–delimited: preceded by a space, tab, or start of
/// string and followed by a space, tab, newline, or end of string.
fn resolve_file_refs(text: String) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '@' {
            // Collect the path token until whitespace or end
            let path: String = chars.by_ref().take_while(|c| !c.is_whitespace()).collect();

            if path.is_empty() {
                result.push('@');
            } else {
                match std::fs::read_to_string(&path) {
                    Ok(contents) => {
                        result.push_str(&format!("\n```\n{contents}\n```\n"));
                    }
                    Err(_) => {
                        // Leave unresolvable refs as-is
                        result.push('@');
                        result.push_str(&path);
                    }
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Role;

    #[test]
    fn push_user_appends_user_message() {
        let mut conv = Conversation::new();
        conv.push_user("hello");
        assert_eq!(conv.len(), 1);
        assert_eq!(conv.last_role(), Some(&Role::User));
    }

    #[test]
    fn push_assistant_appends_assistant_message() {
        let mut conv = Conversation::new();
        conv.push_assistant("I am the assistant");
        assert_eq!(conv.len(), 1);
        assert_eq!(conv.last_role(), Some(&Role::Assistant));
    }

    #[test]
    fn resolve_file_ref_existing_file() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let path = dir.path().join("ctx.txt");
        std::fs::write(&path, "file contents here").expect("write");

        let mut conv = Conversation::new();
        conv.push_user(format!("@{}", path.display()));

        let msg = &conv.messages()[0];
        let text = match &msg.content[0] {
            ContentBlock::Text { text } => text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(
            text.contains("file contents here"),
            "expected file contents, got: {text}"
        );
    }

    #[test]
    fn unresolvable_ref_is_left_as_is() {
        let mut conv = Conversation::new();
        conv.push_user("see @/nonexistent/file.txt for details");

        let msg = &conv.messages()[0];
        let text = match &msg.content[0] {
            ContentBlock::Text { text } => text.clone(),
            _ => panic!("expected text block"),
        };
        assert!(
            text.contains("@/nonexistent/file.txt"),
            "expected raw ref, got: {text}"
        );
    }

    #[test]
    fn empty_conversation_is_empty() {
        let conv = Conversation::new();
        assert!(conv.is_empty());
        assert_eq!(conv.len(), 0);
    }
}
