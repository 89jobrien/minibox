//! Conformance tests for the [`MetricsRecorder`] trait contract.
//!
//! All tests use `MockMetricsRecorder` — no real metrics are emitted.

use minibox::testing::mocks::metrics::{MetricEvent, MockMetricsRecorder};
use minibox_core::domain::MetricsRecorder;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// increment_counter records a Counter event.
pub struct IncrementCounterRecordsEvent;
impl ConformanceTest for IncrementCounterRecordsEvent {
    fn name(&self) -> &str {
        "increment_counter_records_event"
    }
    fn adapter(&self) -> &str {
        "metrics"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
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

/// record_histogram records a Histogram event with the correct value.
pub struct RecordHistogramStoresValue;
impl ConformanceTest for RecordHistogramStoresValue {
    fn name(&self) -> &str {
        "record_histogram_stores_value"
    }
    fn adapter(&self) -> &str {
        "metrics"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
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

/// set_gauge records a Gauge event.
pub struct SetGaugeRecordsEvent;
impl ConformanceTest for SetGaugeRecordsEvent {
    fn name(&self) -> &str {
        "set_gauge_records_event"
    }
    fn adapter(&self) -> &str {
        "metrics"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
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

/// fresh recorder has zero events.
pub struct FreshRecorderHasNoEvents;
impl ConformanceTest for FreshRecorderHasNoEvents {
    fn name(&self) -> &str {
        "fresh_recorder_has_no_events"
    }
    fn adapter(&self) -> &str {
        "metrics"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockMetricsRecorder::new();
        ctx.assert_eq(
            0,
            mock.event_count(),
            "fresh recorder must have zero events",
        );
        ctx.result()
    }
}

/// Return all metrics conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(IncrementCounterRecordsEvent),
        Box::new(RecordHistogramStoresValue),
        Box::new(SetGaugeRecordsEvent),
        Box::new(FreshRecorderHasNoEvents),
    ]
}
