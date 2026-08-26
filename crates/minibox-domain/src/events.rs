//! Container lifecycle event values and emission port.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// A structured event emitted during container and image lifecycles.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContainerEvent {
    /// A container record was created.
    Created {
        /// Container identifier.
        id: String,
        /// Source image reference.
        image: String,
        /// Time the event occurred.
        timestamp: SystemTime,
    },
    /// A container process started.
    Started {
        /// Container identifier.
        id: String,
        /// Host process identifier.
        pid: u32,
        /// Time the event occurred.
        timestamp: SystemTime,
    },
    /// A container process exited.
    Stopped {
        /// Container identifier.
        id: String,
        /// Process exit code.
        exit_code: i32,
        /// Time the event occurred.
        timestamp: SystemTime,
    },
    /// A running container was paused.
    Paused {
        /// Container identifier.
        id: String,
        /// Time the event occurred.
        timestamp: SystemTime,
    },
    /// A paused container resumed.
    Resumed {
        /// Container identifier.
        id: String,
        /// Time the event occurred.
        timestamp: SystemTime,
    },
    /// A container was terminated by the out-of-memory killer.
    OomKilled {
        /// Container identifier.
        id: String,
        /// Time the event occurred.
        timestamp: SystemTime,
    },
    /// An image pull completed.
    ImagePulled {
        /// Pulled image reference.
        image: String,
        /// Downloaded image size in bytes.
        size_bytes: u64,
        /// Time the event occurred.
        timestamp: SystemTime,
    },
    /// An image was removed from local storage.
    ImageRemoved {
        /// Removed image reference.
        image: String,
        /// Time the event occurred.
        timestamp: SystemTime,
    },
    /// Unused images were pruned.
    ImagePruned {
        /// Number of images removed.
        count: usize,
        /// Number of bytes freed.
        freed_bytes: u64,
        /// Time the event occurred.
        timestamp: SystemTime,
    },
}

/// Write-only port for lifecycle event emission.
pub trait EventSink: Send + Sync {
    /// Emit an event without blocking the caller.
    fn emit(&self, event: ContainerEvent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_event_roundtrips_json() {
        let event = ContainerEvent::Stopped {
            id: "abc".to_string(),
            exit_code: 7,
            timestamp: SystemTime::UNIX_EPOCH,
        };

        let encoded = serde_json::to_string(&event).expect("event must serialize");
        let decoded: ContainerEvent =
            serde_json::from_str(&encoded).expect("event must deserialize");

        assert!(matches!(
            decoded,
            ContainerEvent::Stopped {
                id,
                exit_code: 7,
                ..
            } if id == "abc"
        ));
    }
}
