//! Telemetry infrastructure adapters: metrics recording, trace export, metrics HTTP server.

mod noop;
mod prometheus_adapter;

pub use noop::NoOpMetricsRecorder;
pub use prometheus_adapter::PrometheusMetricsRecorder;
