//! Runtime-independent metrics recording port.

// ---------------------------------------------------------------------------
// Metrics Recorder Port
// ---------------------------------------------------------------------------

/// Port for recording operational metrics.
///
/// Adapters: `PrometheusMetricsRecorder` (production), `NoOpMetricsRecorder`
/// (testing/disabled), `RecordingMetricsRecorder` (test assertions).
///
/// String-based names and labels keep the domain free of OTEL/Prometheus types.
pub trait MetricsRecorder: Send + Sync {
    /// Increment a counter by 1.
    fn increment_counter(&self, name: &str, labels: &[(&str, &str)]);
    /// Record a value in a histogram (e.g., duration in seconds).
    fn record_histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]);
    /// Set a gauge to an absolute value.
    fn set_gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]);
}
