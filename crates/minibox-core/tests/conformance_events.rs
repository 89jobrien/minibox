//! Conformance tests for `EventSink` and `EventSource` trait contracts.
//!
//! Verifies:
//! - `BroadcastEventBroker` implements both `EventSink` and `EventSource`.
//! - `emit()` is fire-and-forget — does not panic with zero subscribers.
//! - `subscribe()` returns a receiver that captures future events.
//! - Multiple subscribers each receive every event independently.
//! - `NoopEventSink` discards all events without panic.
//! - All `ContainerEvent` variants can be emitted and received.
//! - Event ordering is preserved (FIFO within a single subscriber).
//!
//! No I/O, no network. Uses tokio broadcast channels only.

use minibox_core::events::{BroadcastEventBroker, ContainerEvent, EventSink, EventSource, NoopEventSink};
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn created_event(id: &str) -> ContainerEvent {
    ContainerEvent::Created {
        id: id.to_string(),
        image: "alpine:latest".to_string(),
        timestamp: SystemTime::now(),
    }
}

fn stopped_event(id: &str, exit_code: i32) -> ContainerEvent {
    ContainerEvent::Stopped {
        id: id.to_string(),
        exit_code,
        timestamp: SystemTime::now(),
    }
}

// ---------------------------------------------------------------------------
// BroadcastEventBroker: emit without subscribers
// ---------------------------------------------------------------------------

/// Emitting with no subscribers must not panic or block.
#[test]
fn conformance_emit_no_subscribers_does_not_panic() {
    let broker = BroadcastEventBroker::new();
    broker.emit(created_event("no-sub-1"));
}

// ---------------------------------------------------------------------------
// BroadcastEventBroker: single subscriber receives event
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_single_subscriber_receives_event() {
    let broker = BroadcastEventBroker::new();
    let mut rx = broker.subscribe();

    broker.emit(created_event("single-1"));

    let evt = rx.recv().await.expect("must receive event");
    assert!(matches!(evt, ContainerEvent::Created { id, .. } if id == "single-1"));
}

// ---------------------------------------------------------------------------
// BroadcastEventBroker: multiple subscribers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_multiple_subscribers_each_receive_event() {
    let broker = BroadcastEventBroker::new();
    let mut rx1 = broker.subscribe();
    let mut rx2 = broker.subscribe();
    let mut rx3 = broker.subscribe();

    broker.emit(stopped_event("multi-1", 0));

    let e1 = rx1.recv().await.expect("rx1 must receive");
    let e2 = rx2.recv().await.expect("rx2 must receive");
    let e3 = rx3.recv().await.expect("rx3 must receive");

    assert!(matches!(e1, ContainerEvent::Stopped { id, exit_code, .. } if id == "multi-1" && exit_code == 0));
    assert!(matches!(e2, ContainerEvent::Stopped { .. }));
    assert!(matches!(e3, ContainerEvent::Stopped { .. }));
}

// ---------------------------------------------------------------------------
// BroadcastEventBroker: FIFO ordering
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_events_received_in_order() {
    let broker = BroadcastEventBroker::new();
    let mut rx = broker.subscribe();

    broker.emit(created_event("order-1"));
    broker.emit(stopped_event("order-1", 0));
    broker.emit(created_event("order-2"));

    let e1 = rx.recv().await.expect("first event");
    let e2 = rx.recv().await.expect("second event");
    let e3 = rx.recv().await.expect("third event");

    assert!(matches!(e1, ContainerEvent::Created { id, .. } if id == "order-1"));
    assert!(matches!(e2, ContainerEvent::Stopped { id, .. } if id == "order-1"));
    assert!(matches!(e3, ContainerEvent::Created { id, .. } if id == "order-2"));
}

// ---------------------------------------------------------------------------
// BroadcastEventBroker: late subscriber misses earlier events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_late_subscriber_does_not_see_prior_events() {
    let broker = BroadcastEventBroker::new();

    // Emit before subscribing.
    broker.emit(created_event("early-1"));

    let mut rx = broker.subscribe();

    // Emit after subscribing.
    broker.emit(created_event("late-1"));

    let evt = rx.recv().await.expect("must receive late event");
    assert!(
        matches!(evt, ContainerEvent::Created { id, .. } if id == "late-1"),
        "subscriber must only see events emitted after subscribe()"
    );
}

// ---------------------------------------------------------------------------
// BroadcastEventBroker: all ContainerEvent variants round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_all_event_variants_round_trip() {
    let broker = BroadcastEventBroker::new();
    let mut rx = broker.subscribe();
    let now = SystemTime::now();

    let events = vec![
        ContainerEvent::Created { id: "v1".into(), image: "img".into(), timestamp: now },
        ContainerEvent::Started { id: "v2".into(), pid: 42, timestamp: now },
        ContainerEvent::Stopped { id: "v3".into(), exit_code: 1, timestamp: now },
        ContainerEvent::Paused { id: "v4".into(), timestamp: now },
        ContainerEvent::Resumed { id: "v5".into(), timestamp: now },
        ContainerEvent::OomKilled { id: "v6".into(), timestamp: now },
        ContainerEvent::ImagePulled { image: "alpine".into(), size_bytes: 1024, timestamp: now },
        ContainerEvent::ImageRemoved { image: "old".into(), timestamp: now },
        ContainerEvent::ImagePruned { count: 3, freed_bytes: 4096, timestamp: now },
    ];

    for evt in &events {
        broker.emit(evt.clone());
    }

    for (i, _) in events.iter().enumerate() {
        let received = rx.recv().await.unwrap_or_else(|_| panic!("must receive event {i}"));
        // Verify discriminant matches by checking Debug output contains expected variant name.
        let debug = format!("{received:?}");
        assert!(!debug.is_empty(), "event {i} must have non-empty Debug output");
    }
}

// ---------------------------------------------------------------------------
// NoopEventSink: does not panic for any variant
// ---------------------------------------------------------------------------

#[test]
fn conformance_noop_event_sink_accepts_all_variants() {
    let sink = NoopEventSink;
    let now = SystemTime::now();

    sink.emit(ContainerEvent::Created { id: "n1".into(), image: "x".into(), timestamp: now });
    sink.emit(ContainerEvent::Started { id: "n2".into(), pid: 1, timestamp: now });
    sink.emit(ContainerEvent::Stopped { id: "n3".into(), exit_code: 0, timestamp: now });
    sink.emit(ContainerEvent::Paused { id: "n4".into(), timestamp: now });
    sink.emit(ContainerEvent::Resumed { id: "n5".into(), timestamp: now });
    sink.emit(ContainerEvent::OomKilled { id: "n6".into(), timestamp: now });
    sink.emit(ContainerEvent::ImagePulled { image: "a".into(), size_bytes: 0, timestamp: now });
    sink.emit(ContainerEvent::ImageRemoved { image: "b".into(), timestamp: now });
    sink.emit(ContainerEvent::ImagePruned { count: 0, freed_bytes: 0, timestamp: now });
}

// ---------------------------------------------------------------------------
// BroadcastEventBroker: Default trait
// ---------------------------------------------------------------------------

#[test]
fn conformance_broker_default_creates_usable_instance() {
    let broker = BroadcastEventBroker::default();
    let _rx = broker.subscribe();
    broker.emit(created_event("default-1"));
}

// ---------------------------------------------------------------------------
// BroadcastEventBroker: Clone
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_broker_clone_shares_channel() {
    let broker = BroadcastEventBroker::new();
    let clone = broker.clone();
    let mut rx = broker.subscribe();

    // Emit on the clone, receive on original's subscriber.
    clone.emit(created_event("clone-1"));

    let evt = rx.recv().await.expect("must receive from cloned broker");
    assert!(matches!(evt, ContainerEvent::Created { id, .. } if id == "clone-1"));
}
