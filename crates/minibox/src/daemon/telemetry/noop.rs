//! No-op metrics recorder — all methods are empty.
//!
//! Used in tests and when metrics are disabled.

use minibox_core::domain::MetricsRecorder;

/// Metrics recorder that silently discards all metric operations.
pub struct NoOpMetricsRecorder;

impl NoOpMetricsRecorder {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoOpMetricsRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRecorder for NoOpMetricsRecorder {
    fn increment_counter(&self, _name: &str, _labels: &[(&str, &str)]) {}
    fn record_histogram(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
    fn set_gauge(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::MetricsRecorder;
    use std::sync::Arc;

    #[test]
    fn noop_recorder_compiles_as_trait_object() {
        let recorder: Arc<dyn MetricsRecorder> = Arc::new(NoOpMetricsRecorder::new());
        recorder.increment_counter("test", &[]);
        recorder.record_histogram("test", 1.0, &[]);
        recorder.set_gauge("test", 1.0, &[]);
    }
}
