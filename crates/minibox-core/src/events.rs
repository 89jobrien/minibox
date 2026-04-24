//! Container lifecycle event types and pub/sub ports.
//!
//! `EventSink` is the write port — handlers call `emit()`.
//! `EventSource` is the read port — dashbox and CLI subscribe.
//! `BroadcastEventBroker` is the single adapter implementing both ports.

use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use tokio::sync::broadcast;

/// A structured event emitted by the minibox daemon during container lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContainerEvent {
    Created {
        id: String,
        image: String,
        timestamp: SystemTime,
    },
    Started {
        id: String,
        pid: u32,
        timestamp: SystemTime,
    },
    Stopped {
        id: String,
        exit_code: i32,
        timestamp: SystemTime,
    },
    Paused {
        id: String,
        timestamp: SystemTime,
    },
    Resumed {
        id: String,
        timestamp: SystemTime,
    },
    OomKilled {
        id: String,
        timestamp: SystemTime,
    },
    ImagePulled {
        image: String,
        size_bytes: u64,
        timestamp: SystemTime,
    },
    ImageRemoved {
        image: String,
        timestamp: SystemTime,
    },
    ImagePruned {
        count: usize,
        freed_bytes: u64,
        timestamp: SystemTime,
    },
}

/// Port: write-only event emission. Handlers depend on this.
pub trait EventSink: Send + Sync {
    /// Emit an event. Fire-and-forget — never blocks.
    fn emit(&self, event: ContainerEvent);
}

/// Port: subscribe to the event stream. Dashbox and CLI depend on this.
pub trait EventSource: Send + Sync {
    /// Returns a receiver that will receive all future events.
    /// Lagged receivers (too slow to consume) receive `RecvError::Lagged`.
    fn subscribe(&self) -> broadcast::Receiver<ContainerEvent>;
}

/// Adapter: tokio broadcast channel. Implements both `EventSink` and `EventSource`.
///
/// Capacity: 1024 events. Slow consumers receive `RecvError::Lagged` and skip
/// missed events — this is intentional (events are best-effort observability).
#[derive(Clone)]
pub struct BroadcastEventBroker {
    tx: broadcast::Sender<ContainerEvent>,
}

impl BroadcastEventBroker {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }
}

impl Default for BroadcastEventBroker {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSink for BroadcastEventBroker {
    fn emit(&self, event: ContainerEvent) {
        // send() errors only if there are no receivers — that's fine.
        let _ = self.tx.send(event);
    }
}

impl EventSource for BroadcastEventBroker {
    fn subscribe(&self) -> broadcast::Receiver<ContainerEvent> {
        self.tx.subscribe()
    }
}

/// No-op sink for tests and platforms where events are not needed.
pub struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _event: ContainerEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_emit_and_receive() {
        let broker = BroadcastEventBroker::new();
        let mut rx = broker.subscribe();

        broker.emit(ContainerEvent::Created {
            id: "abc".to_string(),
            image: "alpine".to_string(),
            timestamp: SystemTime::now(),
        });

        let evt = rx.recv().await.unwrap();
        assert!(matches!(evt, ContainerEvent::Created { id, .. } if id == "abc"));
    }

    #[test]
    fn test_noop_sink_does_not_panic() {
        let sink = NoopEventSink;
        sink.emit(ContainerEvent::Stopped {
            id: "x".to_string(),
            exit_code: 0,
            timestamp: SystemTime::now(),
        });
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let broker = BroadcastEventBroker::new();
        let mut rx1 = broker.subscribe();
        let mut rx2 = broker.subscribe();

        broker.emit(ContainerEvent::Paused {
            id: "c1".to_string(),
            timestamp: SystemTime::now(),
        });

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert!(matches!(e1, ContainerEvent::Paused { .. }));
        assert!(matches!(e2, ContainerEvent::Paused { .. }));
    }
}
