//! Agent lifecycle events: Pre/PostToolUse and OnError.
//!
//! [`EventManager`] holds a list of registered [`EventHandler`]s and dispatches
//! [`Event`]s to them in registration order. Handlers are called synchronously.

use crate::tools::{ToolInput, ToolOutput};

/// Context attached to every fired event.
#[derive(Debug, Clone)]
pub struct EventContext {
    /// Arbitrary string identifying the current session or turn.
    pub session_id: String,
    /// Zero-based turn index within the current session.
    pub turn: usize,
}

/// Events emitted by the agent during its agentic loop.
#[derive(Debug, Clone)]
pub enum Event {
    /// Fired immediately before a tool is executed.
    PreToolUse { ctx: EventContext, input: ToolInput },
    /// Fired immediately after a tool returns (success or soft error).
    PostToolUse {
        ctx: EventContext,
        input: ToolInput,
        output: ToolOutput,
    },
    /// Fired when the agentic loop encounters a hard error.
    OnError { ctx: EventContext, message: String },
}

/// Port: objects that wish to react to agent events implement this trait.
pub trait EventHandler: Send + Sync {
    /// Handle an event. Errors are ignored (handlers must not crash the loop).
    fn handle(&self, event: &Event);
}

/// Dispatches [`Event`]s to all registered [`EventHandler`]s.
#[derive(Default)]
pub struct EventManager {
    handlers: Vec<Box<dyn EventHandler>>,
}

impl EventManager {
    /// Create an empty manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler. Handlers fire in registration order.
    pub fn register(&mut self, handler: Box<dyn EventHandler>) {
        self.handlers.push(handler);
    }

    /// Fire `event` to all registered handlers.
    pub fn fire(&self, event: &Event) {
        for h in &self.handlers {
            h.handle(event);
        }
    }
}

// ── In-memory test double ─────────────────────────────────────────────────────

/// Test double: records every event it receives.
#[derive(Default)]
pub struct RecordingHandler {
    events: std::sync::Mutex<Vec<Event>>,
}

impl RecordingHandler {
    /// Create a new recording handler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return a snapshot of recorded events.
    pub fn recorded(&self) -> Vec<Event> {
        self.events.lock().expect("lock").clone()
    }
}

impl EventHandler for RecordingHandler {
    fn handle(&self, event: &Event) {
        self.events.lock().expect("lock").push(event.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolInput;
    use std::sync::Arc;

    fn make_ctx() -> EventContext {
        EventContext {
            session_id: "test-session".into(),
            turn: 0,
        }
    }

    #[test]
    fn event_manager_fires_pre_tool_use() {
        let handler = Arc::new(RecordingHandler::new());
        let mut mgr = EventManager::new();
        // Use a clone of the Arc as a Box<dyn EventHandler>
        mgr.register(Box::new(RecordingHandlerRef(Arc::clone(&handler))));

        let input = ToolInput {
            name: "bash".into(),
            args: serde_json::json!({"command": "echo hi"}),
        };
        mgr.fire(&Event::PreToolUse {
            ctx: make_ctx(),
            input,
        });

        let events = handler.recorded();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::PreToolUse { .. }));
    }

    #[test]
    fn event_manager_fires_post_tool_use() {
        let handler = Arc::new(RecordingHandler::new());
        let mut mgr = EventManager::new();
        mgr.register(Box::new(RecordingHandlerRef(Arc::clone(&handler))));

        let input = ToolInput {
            name: "read".into(),
            args: serde_json::json!({"path": "foo.txt"}),
        };
        let output = crate::tools::ToolOutput::ok("read", "contents");
        mgr.fire(&Event::PostToolUse {
            ctx: make_ctx(),
            input,
            output,
        });

        let events = handler.recorded();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::PostToolUse { .. }));
    }

    #[test]
    fn event_manager_fires_on_error() {
        let handler = Arc::new(RecordingHandler::new());
        let mut mgr = EventManager::new();
        mgr.register(Box::new(RecordingHandlerRef(Arc::clone(&handler))));

        mgr.fire(&Event::OnError {
            ctx: make_ctx(),
            message: "something went wrong".into(),
        });

        let events = handler.recorded();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::OnError { .. }));
    }

    #[test]
    fn multiple_handlers_all_receive_event() {
        let h1 = Arc::new(RecordingHandler::new());
        let h2 = Arc::new(RecordingHandler::new());
        let mut mgr = EventManager::new();
        mgr.register(Box::new(RecordingHandlerRef(Arc::clone(&h1))));
        mgr.register(Box::new(RecordingHandlerRef(Arc::clone(&h2))));

        mgr.fire(&Event::OnError {
            ctx: make_ctx(),
            message: "boom".into(),
        });

        assert_eq!(h1.recorded().len(), 1);
        assert_eq!(h2.recorded().len(), 1);
    }

    // Newtype so Arc<RecordingHandler> implements EventHandler via delegation.
    struct RecordingHandlerRef(Arc<RecordingHandler>);
    impl EventHandler for RecordingHandlerRef {
        fn handle(&self, event: &Event) {
            self.0.handle(event);
        }
    }
}
