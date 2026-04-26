//! Telemetry infrastructure adapters: metrics recording, trace export, metrics HTTP server.

mod noop;
pub use noop::NoOpMetricsRecorder;

#[cfg(feature = "metrics")]
mod prometheus_adapter;
#[cfg(feature = "metrics")]
pub mod server;
#[cfg(feature = "metrics")]
pub use prometheus_adapter::PrometheusMetricsRecorder;

#[cfg(feature = "otel")]
pub mod traces;
