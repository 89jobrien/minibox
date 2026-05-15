//! Prometheus metrics adapter implementing the `MetricsRecorder` domain port.
//!
//! Uses the `prometheus-client` crate (official Prometheus Rust client) directly.
//! OTEL SDK is NOT involved in metrics — it handles traces only.

use dashmap::DashMap;
use minibox_core::domain::MetricsRecorder;
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use std::sync::{Arc, Mutex};

/// Label set type for dynamic string labels.
type Labels = Vec<(String, String)>;

/// Production metrics recorder backed by `prometheus-client`.
///
/// Creates metric families lazily and caches them in a `DashMap` for
/// lock-free concurrent access from handler tasks. The inner `Registry`
/// is behind a `Mutex` because `prometheus-client` requires `&mut` for
/// registration.
pub struct PrometheusMetricsRecorder {
    registry: Arc<Mutex<Registry>>,
    counters: DashMap<String, Family<Labels, Counter>>,
    histograms: DashMap<String, Family<Labels, Histogram>>,
    gauges: DashMap<String, Family<Labels, Gauge>>,
}

impl PrometheusMetricsRecorder {
    /// Create a new recorder with its own Prometheus registry.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Mutex::new(Registry::default())),
            counters: DashMap::new(),
            histograms: DashMap::new(),
            gauges: DashMap::new(),
        }
    }

    /// Encode all registered metrics as Prometheus text exposition format.
    pub fn encode_metrics(&self) -> String {
        let registry = self.registry.lock().unwrap(); // allow:unwrap — poisoned mutex is unrecoverable
        let mut buffer = String::new();
        encode(&mut buffer, &registry).unwrap_or_default();
        buffer
    }
}

impl Default for PrometheusMetricsRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl PrometheusMetricsRecorder {
    fn get_or_create_counter(&self, name: &str) -> Family<Labels, Counter> {
        self.counters
            .entry(name.to_string())
            .or_insert_with(|| {
                let family = Family::<Labels, Counter>::default();
                self.registry
                    .lock()
                    .unwrap() // allow:unwrap — poisoned mutex is unrecoverable
                    .register(name, name, family.clone());
                family
            })
            .clone()
    }

    fn get_or_create_histogram(&self, name: &str) -> Family<Labels, Histogram> {
        self.histograms
            .entry(name.to_string())
            .or_insert_with(|| {
                let family = Family::<Labels, Histogram>::new_with_constructor(|| {
                    Histogram::new(exponential_buckets(0.001, 2.0, 16))
                });
                self.registry
                    .lock()
                    .unwrap() // allow:unwrap — poisoned mutex is unrecoverable
                    .register(name, name, family.clone());
                family
            })
            .clone()
    }

    fn get_or_create_gauge(&self, name: &str) -> Family<Labels, Gauge> {
        self.gauges
            .entry(name.to_string())
            .or_insert_with(|| {
                let family = Family::<Labels, Gauge>::default();
                self.registry
                    .lock()
                    .unwrap() // allow:unwrap — poisoned mutex is unrecoverable
                    .register(name, name, family.clone());
                family
            })
            .clone()
    }
}

fn to_labels(labels: &[(&str, &str)]) -> Labels {
    labels
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

impl MetricsRecorder for PrometheusMetricsRecorder {
    fn increment_counter(&self, name: &str, labels: &[(&str, &str)]) {
        let family = self.get_or_create_counter(name);
        family.get_or_create(&to_labels(labels)).inc();
    }

    fn record_histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        let family = self.get_or_create_histogram(name);
        family.get_or_create(&to_labels(labels)).observe(value);
    }

    fn set_gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        let family = self.get_or_create_gauge(name);
        // prometheus-client 0.23 Gauge defaults to i64; cast from f64 is sufficient
        // for our use cases (active container counts, queue depths, etc.).
        #[allow(clippy::cast_possible_truncation)]
        family.get_or_create(&to_labels(labels)).set(value as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::MetricsRecorder;

    #[test]
    fn prometheus_recorder_creates_and_records() {
        let recorder = PrometheusMetricsRecorder::new();
        recorder.increment_counter(
            "minibox_container_ops_total",
            &[("op", "run"), ("status", "ok")],
        );
        recorder.record_histogram(
            "minibox_container_op_duration_seconds",
            0.123,
            &[("op", "run")],
        );
        recorder.set_gauge("minibox_active_containers", 2.0, &[("adapter", "native")]);

        let output = recorder.encode_metrics();
        assert!(
            output.contains("minibox_container_ops_total"),
            "missing counter in output:\n{output}"
        );
        assert!(
            output.contains("minibox_container_op_duration_seconds"),
            "missing histogram in output:\n{output}"
        );
        assert!(
            output.contains("minibox_active_containers"),
            "missing gauge in output:\n{output}"
        );
    }
}
