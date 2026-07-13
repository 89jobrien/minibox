//! Conformance tests for the [`MetricsRecorder`] trait contract.
//!
//! All tests use `MockMetricsRecorder` — no real metrics are emitted.

use minibox::testing::mocks::metrics::{MetricEvent, MockMetricsRecorder};
use minibox_core::domain::MetricsRecorder;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// `increment_counter` records a Counter event.
crate::conformance_test! {
    name: "increment_counter_records_event",
    adapter: "metrics",
    capability: Metrics,
    category: Unit,
    |ctx| {
        let mock = MockMetricsRecorder::new();
        mock.increment_counter("requests_total", &[]);
        ctx.assert_eq(1, mock.event_count(), "event_count after one counter");
        let events = mock.events();
        ctx.assert_true(
            matches!(&events[0], MetricEvent::Counter { name } if name == "requests_total"),
            "first event should be Counter(requests_total)",
        );
        ctx.result()
    }
}

// `record_histogram` records a Histogram event with the correct value.
crate::conformance_test! {
    name: "record_histogram_stores_value",
    adapter: "metrics",
    capability: Metrics,
    category: Unit,
    |ctx| {
        let mock = MockMetricsRecorder::new();
        mock.record_histogram("latency_seconds", 0.123, &[]);
        let events = mock.events();
        ctx.assert_eq(1, events.len(), "one event expected");
        ctx.assert_true(
            matches!(&events[0], MetricEvent::Histogram { value, .. } if (*value - 0.123).abs() < 1e-9),
            "histogram value should match",
        );
        ctx.result()
    }
}

// `set_gauge` records a Gauge event.
crate::conformance_test! {
    name: "set_gauge_records_event",
    adapter: "metrics",
    capability: Metrics,
    category: Unit,
    |ctx| {
        let mock = MockMetricsRecorder::new();
        mock.set_gauge("memory_bytes", 4096.0, &[("container", "c1")]);
        let events = mock.events();
        ctx.assert_eq(1, events.len(), "one event expected");
        ctx.assert_true(
            matches!(&events[0], MetricEvent::Gauge { name, .. } if name == "memory_bytes"),
            "event should be Gauge(memory_bytes)",
        );
        ctx.result()
    }
}

// fresh recorder has zero events.
crate::conformance_test! {
    name: "fresh_recorder_has_no_events",
    adapter: "metrics",
    capability: Metrics,
    category: EdgeCase,
    |ctx| {
        let mock = MockMetricsRecorder::new();
        ctx.assert_eq(
            0,
            mock.event_count(),
            "fresh recorder must have zero events",
        );
        ctx.result()
    }
}
