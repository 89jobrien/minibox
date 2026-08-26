//! Application state for the minibox TUI.

use minibox_core::events::ContainerEvent;
use minibox_core::protocol::ContainerInfo;

/// Bounded history of recent lifecycle events shown in the log pane.
const MAX_EVENT_LOG: usize = 200;

/// Top-level application state, updated by [`crate::event`] and rendered by [`crate::ui`].
pub struct App {
    /// Containers currently reported by the daemon.
    pub containers: Vec<ContainerInfo>,
    /// Selected container row index.
    pub selected: usize,
    /// Formatted lifecycle event history.
    pub events: Vec<String>,
    /// Whether the main loop should exit.
    pub should_quit: bool,
    /// Most recent daemon or polling error.
    pub last_error: Option<String>,
}

impl App {
    /// Creates empty application state.
    pub const fn new() -> Self {
        Self {
            containers: Vec::new(),
            selected: 0,
            events: Vec::new(),
            should_quit: false,
            last_error: None,
        }
    }

    /// Replaces the container list and clamps the selected row.
    pub fn set_containers(&mut self, containers: Vec<ContainerInfo>) {
        self.containers = containers;
        if self.selected >= self.containers.len() {
            self.selected = self.containers.len().saturating_sub(1);
        }
    }

    /// Appends a formatted lifecycle event to the bounded history.
    pub fn push_event(&mut self, event: &ContainerEvent) {
        self.events.push(format_event(event));
        if self.events.len() > MAX_EVENT_LOG {
            self.events.remove(0);
        }
    }

    /// Selects the next container row, wrapping at the end.
    pub fn select_next(&mut self) {
        if !self.containers.is_empty() {
            self.selected = (self.selected + 1) % self.containers.len();
        }
    }

    /// Selects the previous container row, wrapping at the beginning.
    pub fn select_prev(&mut self) {
        if !self.containers.is_empty() {
            self.selected = (self.selected + self.containers.len() - 1) % self.containers.len();
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

fn format_event(event: &ContainerEvent) -> String {
    match event {
        ContainerEvent::Created { id, image, .. } => {
            format!("created   {id}  ({image})")
        }
        ContainerEvent::Started { id, pid, .. } => {
            format!("started   {id}  pid={pid}")
        }
        ContainerEvent::Stopped { id, exit_code, .. } => {
            format!("stopped   {id}  exit={exit_code}")
        }
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn container(id: &str) -> ContainerInfo {
        ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "sh".to_string(),
            state: "running".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: None,
        }
    }

    #[test]
    fn select_next_wraps_around() {
        let mut app = App::new();
        app.set_containers(vec![container("a"), container("b"), container("c")]);
        assert_eq!(app.selected, 0);
        app.select_next();
        assert_eq!(app.selected, 1);
        app.select_next();
        app.select_next();
        assert_eq!(app.selected, 0, "should wrap back to the first container");
    }

    #[test]
    fn select_prev_wraps_around() {
        let mut app = App::new();
        app.set_containers(vec![container("a"), container("b")]);
        assert_eq!(app.selected, 0);
        app.select_prev();
        assert_eq!(app.selected, 1, "should wrap to the last container");
    }

    #[test]
    fn select_on_empty_list_is_noop() {
        let mut app = App::new();
        app.select_next();
        app.select_prev();
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn set_containers_clamps_selection_when_list_shrinks() {
        let mut app = App::new();
        app.set_containers(vec![container("a"), container("b"), container("c")]);
        app.selected = 2;
        app.set_containers(vec![container("a")]);
        assert_eq!(
            app.selected, 0,
            "selection must be clamped into the new bounds"
        );
    }

    #[test]
    fn push_event_bounds_history_length() {
        let mut app = App::new();
        for i in 0..(MAX_EVENT_LOG + 10) {
            app.push_event(&ContainerEvent::Started {
                id: format!("c{i}"),
                pid: 1,
                timestamp: SystemTime::now(),
            });
        }
        assert_eq!(app.events.len(), MAX_EVENT_LOG);
        assert!(
            app.events[0].contains(&format!("c{}", 10)),
            "oldest events should be evicted first, got: {}",
            app.events[0]
        );
    }

    #[test]
    fn format_event_covers_known_variants() {
        let created = ContainerEvent::Created {
            id: "abc123".to_string(),
            image: "alpine:latest".to_string(),
            timestamp: SystemTime::now(),
        };
        assert_eq!(format_event(&created), "created   abc123  (alpine:latest)");

        let started = ContainerEvent::Started {
            id: "abc123".to_string(),
            pid: 42,
            timestamp: SystemTime::now(),
        };
        assert_eq!(format_event(&started), "started   abc123  pid=42");
    }
}
