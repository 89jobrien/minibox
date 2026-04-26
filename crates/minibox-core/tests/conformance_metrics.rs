//! Conformance tests for the `MetricsRecorder` trait contract.
//!
//! Verifies:
//! - Trait methods accept arbitrary metric names and label sets.
//! - Zero labels are accepted (empty slice).
//! - Multiple labels are accepted.
//! - Negative histogram values are accepted (no panic).
//! - Negative gauge values are accepted (no panic).
//! - Trait object (`Arc<dyn MetricsRecorder>`) is constructable and callable.
//! - `StubRecorder` (inline test double) records calls for assertion.
//!
//! No I/O, no network.

use minibox_core::domain::MetricsRecorder;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// StubRecorder — test double that records calls
// ---------------------------------------------------------------------------

#[derive(Default)]
struct StubRecorder {
    counters: Mutex<Vec<(String, Vec<(String, String)>)>>,
    histograms: Mutex<Vec<(String, f64, Vec<(String, String)>)>>,
    gauges: Mutex<Vec<(String, f64, Vec<(String, String)>)>>,
}

impl MetricsRecorder for StubRecorder {
    fn increment_counter(&self, name: &str, labels: &[(&str, &str)]) {
        self.counters.lock().unwrap().push((
            name.to_string(),
            labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        ));
    }

    fn record_histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        self.histograms.lock().unwrap().push((
            name.to_string(),
            value,
            labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        ));
    }

    fn set_gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        self.gauges.lock().unwrap().push((
            name.to_string(),
            value,
            labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        ));
    }
}

// ---------------------------------------------------------------------------
// increment_counter
// ---------------------------------------------------------------------------

#[test]
fn conformance_counter_no_labels() {
    let recorder = StubRecorder::default();
    recorder.increment_counter("requests_total", &[]);

    let counters = recorder.counters.lock().unwrap();
    assert_eq!(counters.len(), 1);
    assert_eq!(counters[0].0, "requests_total");
    assert!(counters[0].1.is_empty());
}

#[test]
fn conformance_counter_with_labels() {
    let recorder = StubRecorder::default();
    recorder.increment_counter("http_requests", &[("method", "GET"), ("status", "200")]);

    let counters = recorder.counters.lock().unwrap();
    assert_eq!(counters.len(), 1);
    assert_eq!(counters[0].1.len(), 2);
    assert_eq!(counters[0].1[0], ("method".to_string(), "GET".to_string()));
    assert_eq!(counters[0].1[1], ("status".to_string(), "200".to_string()));
}

#[test]
fn conformance_counter_multiple_increments() {
    let recorder = StubRecorder::default();
    for _ in 0..5 {
        recorder.increment_counter("ops", &[]);
    }

    let counters = recorder.counters.lock().unwrap();
    assert_eq!(counters.len(), 5);
}

// ---------------------------------------------------------------------------
// record_histogram
// ---------------------------------------------------------------------------

#[test]
fn conformance_histogram_positive_value() {
    let recorder = StubRecorder::default();
    recorder.record_histogram("request_duration_seconds", 0.042, &[("path", "/api")]);

    let histograms = recorder.histograms.lock().unwrap();
    assert_eq!(histograms.len(), 1);
    assert_eq!(histograms[0].0, "request_duration_seconds");
    assert!((histograms[0].1 - 0.042).abs() < f64::EPSILON);
}

#[test]
fn conformance_histogram_zero_value() {
    let recorder = StubRecorder::default();
    recorder.record_histogram("latency", 0.0, &[]);

    let histograms = recorder.histograms.lock().unwrap();
    assert_eq!(histograms[0].1, 0.0);
}

#[test]
fn conformance_histogram_negative_value_does_not_panic() {
    let recorder = StubRecorder::default();
    // Negative values may be semantically invalid but must not panic.
    recorder.record_histogram("drift", -1.5, &[]);

    let histograms = recorder.histograms.lock().unwrap();
    assert_eq!(histograms.len(), 1);
}

// ---------------------------------------------------------------------------
// set_gauge
// ---------------------------------------------------------------------------

#[test]
fn conformance_gauge_positive() {
    let recorder = StubRecorder::default();
    recorder.set_gauge("active_containers", 7.0, &[]);

    let gauges = recorder.gauges.lock().unwrap();
    assert_eq!(gauges.len(), 1);
    assert_eq!(gauges[0].0, "active_containers");
    assert!((gauges[0].1 - 7.0).abs() < f64::EPSILON);
}

#[test]
fn conformance_gauge_negative_does_not_panic() {
    let recorder = StubRecorder::default();
    recorder.set_gauge("temperature", -40.0, &[]);

    let gauges = recorder.gauges.lock().unwrap();
    assert_eq!(gauges.len(), 1);
}

#[test]
fn conformance_gauge_overwrite_semantics() {
    let recorder = StubRecorder::default();
    recorder.set_gauge("mem_used", 100.0, &[]);
    recorder.set_gauge("mem_used", 200.0, &[]);

    let gauges = recorder.gauges.lock().unwrap();
    assert_eq!(gauges.len(), 2, "each set_gauge call must be recorded");
    assert!((gauges[1].1 - 200.0).abs() < f64::EPSILON);
}

// ---------------------------------------------------------------------------
// Trait object
// ---------------------------------------------------------------------------

#[test]
fn conformance_metrics_recorder_as_trait_object() {
    let recorder: Arc<dyn MetricsRecorder> = Arc::new(StubRecorder::default());
    recorder.increment_counter("test", &[]);
    recorder.record_histogram("test", 1.0, &[("a", "b")]);
    recorder.set_gauge("test", 0.0, &[]);
}

#[test]
fn conformance_metrics_recorder_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StubRecorder>();
}
