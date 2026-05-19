//! Mock [`MetricsRecorder`] for conformance testing.

use minibox_core::domain::MetricsRecorder;
use std::sync::Mutex;

/// A single recorded metric event.
#[derive(Debug, Clone)]
pub enum MetricEvent {
    /// A counter was incremented.
    Counter { name: String },
    /// A histogram value was recorded.
    Histogram { name: String, value: f64 },
    /// A gauge was set.
    Gauge { name: String, value: f64 },
}

/// Mock metrics recorder that captures all events in memory.
///
/// Call [`MockMetricsRecorder::events`] after exercising the adapter to assert
/// on which metrics were recorded and in which order.
#[derive(Debug)]
pub struct MockMetricsRecorder {
    events: Mutex<Vec<MetricEvent>>,
}

impl MockMetricsRecorder {
    /// Create a fresh mock with no recorded events.
    pub fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of all recorded events.
    pub fn events(&self) -> Vec<MetricEvent> {
        self.events.lock().expect("lock").clone()
    }

    /// Number of recorded events.
    pub fn event_count(&self) -> usize {
        self.events.lock().expect("lock").len()
    }
}

impl Default for MockMetricsRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRecorder for MockMetricsRecorder {
    fn increment_counter(&self, name: &str, _labels: &[(&str, &str)]) {
        self.events
            .lock()
            .expect("lock")
            .push(MetricEvent::Counter {
                name: name.to_string(),
            });
    }

    fn record_histogram(&self, name: &str, value: f64, _labels: &[(&str, &str)]) {
        self.events
            .lock()
            .expect("lock")
            .push(MetricEvent::Histogram {
                name: name.to_string(),
                value,
            });
    }

    fn set_gauge(&self, name: &str, value: f64, _labels: &[(&str, &str)]) {
        self.events.lock().expect("lock").push(MetricEvent::Gauge {
            name: name.to_string(),
            value,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_counter_event() {
        let mock = MockMetricsRecorder::new();
        mock.increment_counter("requests_total", &[]);
        assert_eq!(mock.event_count(), 1);
        let events = mock.events();
        assert!(matches!(&events[0], MetricEvent::Counter { name } if name == "requests_total"));
    }

    #[test]
    fn records_histogram_and_gauge() {
        let mock = MockMetricsRecorder::new();
        mock.record_histogram("latency_seconds", 0.42, &[]);
        mock.set_gauge("memory_bytes", 1024.0, &[]);
        assert_eq!(mock.event_count(), 2);
    }
}
