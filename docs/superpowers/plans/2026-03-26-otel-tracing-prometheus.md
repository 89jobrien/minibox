# OTEL Tracing & Prometheus Metrics — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add OpenTelemetry trace export and Prometheus metrics to miniboxd, following the hexagonal architecture with a `MetricsRecorder` domain port and infrastructure adapters.

**Architecture:** `MetricsRecorder` trait in minibox-core (domain port) with three implementations: `PrometheusMetricsRecorder` (production), `NoOpMetricsRecorder` (disabled/tests), `RecordingMetricsRecorder` (test assertions). OTEL trace bridge via `tracing-opentelemetry` layer. Prometheus `/metrics` HTTP endpoint via axum.

**Tech Stack:** opentelemetry 0.31, opentelemetry_sdk 0.31, opentelemetry-otlp 0.31, tracing-opentelemetry 0.32, prometheus-client 0.23, axum 0.7, dashmap 6

**Spec:** `docs/superpowers/specs/2026-03-26-otel-tracing-prometheus-design.md`

---

## File Structure

### New files

| File | Responsibility |
|------|---------------|
| `crates/daemonbox/src/telemetry/mod.rs` | Re-exports, `TelemetryConfig` |
| `crates/daemonbox/src/telemetry/prometheus.rs` | `PrometheusMetricsRecorder` adapter |
| `crates/daemonbox/src/telemetry/noop.rs` | `NoOpMetricsRecorder` |
| `crates/daemonbox/src/telemetry/traces.rs` | OTEL trace exporter setup, `OtelGuard` |
| `crates/daemonbox/src/telemetry/server.rs` | axum `/metrics` HTTP server |

### Modified files

| File | Change |
|------|--------|
| `Cargo.toml` (workspace) | Add workspace deps: opentelemetry, opentelemetry_sdk, opentelemetry-otlp, tracing-opentelemetry, prometheus-client, axum, dashmap |
| `crates/minibox-core/Cargo.toml` | No changes needed — trait has no OTEL deps |
| `crates/minibox-core/src/domain.rs` | Add `MetricsRecorder` trait + `DynMetricsRecorder` alias |
| `crates/minibox-core/src/adapters/mocks.rs` | Add `RecordingMetricsRecorder` test double |
| `crates/daemonbox/Cargo.toml` | Add deps: opentelemetry, opentelemetry_sdk, opentelemetry-otlp, tracing-opentelemetry, tracing-subscriber, prometheus-client, axum, dashmap |
| `crates/daemonbox/src/lib.rs` | Add `pub mod telemetry;` |
| `crates/daemonbox/src/handler.rs` | Add `metrics: DynMetricsRecorder` to `HandlerDependencies`, instrument handlers |
| `crates/miniboxd/Cargo.toml` | No new deps needed — traces and metrics are in daemonbox |
| `crates/miniboxd/src/main.rs` | Replace `tracing_subscriber::fmt().init()` with `init_tracing()`, wire metrics recorder + server |

---

## Task 1: Add Workspace Dependencies

**Files:**
- Modify: `Cargo.toml` (workspace root, lines 26–72)

- [ ] **Step 1: Add new workspace dependencies**

Add after the existing `tracing-subscriber` line (line 34):

```toml
# Traces (OTEL bridge + OTLP export)
opentelemetry = "0.31"
opentelemetry_sdk = "0.31"
opentelemetry-otlp = { version = "0.31", features = ["grpc-tonic"] }
tracing-opentelemetry = "0.32"

# Metrics (direct Prometheus client — NOT the discontinued opentelemetry-prometheus)
prometheus-client = "0.23"

# Infrastructure
axum = { version = "0.7", features = ["tokio"] }
dashmap = "6"
```

- [ ] **Step 2: Verify workspace compiles**

Run: `cargo check --workspace`
Expected: PASS (no code uses the new deps yet)

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "deps: add opentelemetry, prometheus, axum, dashmap workspace deps"
```

---

## Task 2: Add `MetricsRecorder` Domain Trait

**Files:**
- Modify: `crates/minibox-core/src/domain.rs` (after line 83, the Dyn* aliases section)

- [ ] **Step 1: Write the test for `MetricsRecorder` trait object creation**

Add to the `#[cfg(test)] mod tests` block at the bottom of `domain.rs`:

```rust
    // --- MetricsRecorder tests ---

    /// Verify that a no-op MetricsRecorder can be constructed and used as a trait object.
    #[test]
    fn test_metrics_recorder_trait_object() {
        struct StubRecorder;
        impl MetricsRecorder for StubRecorder {
            fn increment_counter(&self, _name: &str, _labels: &[(&str, &str)]) {}
            fn record_histogram(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
            fn set_gauge(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
        }

        let recorder: Arc<dyn MetricsRecorder> = Arc::new(StubRecorder);
        recorder.increment_counter("test_counter", &[("key", "val")]);
        recorder.record_histogram("test_hist", 1.5, &[]);
        recorder.set_gauge("test_gauge", 42.0, &[("a", "b")]);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox-core test_metrics_recorder_trait_object`
Expected: FAIL — `MetricsRecorder` not found

- [ ] **Step 3: Add the `MetricsRecorder` trait and type alias**

In `domain.rs`, after the `DynNetworkProvider` type alias (line 83), add:

```rust
/// Type alias for a shared, dynamic [`MetricsRecorder`] implementation.
pub type DynMetricsRecorder = Arc<dyn MetricsRecorder>;
```

Then, after the `AsAny` trait block (line 97) and before the `ImageRegistry` section (line 99), add a new section:

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p minibox-core test_metrics_recorder_trait_object`
Expected: PASS

- [ ] **Step 5: Verify full workspace compiles**

Run: `cargo check --workspace`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-core/src/domain.rs
git commit -m "feat(core): add MetricsRecorder domain trait (port)"
```

---

## Task 3: Add `RecordingMetricsRecorder` Test Double

**Files:**
- Modify: `crates/minibox-core/src/adapters/mocks.rs`

- [ ] **Step 1: Write the test**

Add a new test file `crates/minibox-core/tests/recording_metrics_test.rs`:

```rust
use minibox_core::adapters::mocks::RecordingMetricsRecorder;
use minibox_core::domain::MetricsRecorder;

#[test]
fn recording_metrics_captures_counter() {
    let recorder = RecordingMetricsRecorder::new();
    recorder.increment_counter("minibox_container_ops_total", &[("op", "run"), ("status", "ok")]);
    recorder.increment_counter("minibox_container_ops_total", &[("op", "stop"), ("status", "ok")]);

    let counters = recorder.counters();
    assert_eq!(counters.len(), 2);
    assert_eq!(counters[0].0, "minibox_container_ops_total");
    assert_eq!(counters[0].1, vec![("op".to_string(), "run".to_string()), ("status".to_string(), "ok".to_string())]);
}

#[test]
fn recording_metrics_captures_histogram() {
    let recorder = RecordingMetricsRecorder::new();
    recorder.record_histogram("minibox_container_op_duration_seconds", 0.5, &[("op", "run")]);

    let histograms = recorder.histograms();
    assert_eq!(histograms.len(), 1);
    assert_eq!(histograms[0].0, "minibox_container_op_duration_seconds");
    assert!((histograms[0].1 - 0.5).abs() < f64::EPSILON);
}

#[test]
fn recording_metrics_captures_gauge() {
    let recorder = RecordingMetricsRecorder::new();
    recorder.set_gauge("minibox_active_containers", 3.0, &[("adapter", "native")]);

    let gauges = recorder.gauges();
    assert_eq!(gauges.len(), 1);
    assert_eq!(gauges[0].0, "minibox_active_containers");
    assert!((gauges[0].1 - 3.0).abs() < f64::EPSILON);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox-core recording_metrics`
Expected: FAIL — `RecordingMetricsRecorder` not found

- [ ] **Step 3: Implement `RecordingMetricsRecorder`**

Add to the bottom of `crates/minibox-core/src/adapters/mocks.rs` (before any `#[cfg(test)]` blocks):

```rust
// ---------------------------------------------------------------------------
// RecordingMetricsRecorder
// ---------------------------------------------------------------------------

/// Test double that captures all metric calls for assertion.
///
/// Thread-safe via `Mutex`. Intended for unit tests that need to verify
/// specific metrics were emitted with correct names, values, and labels.
#[derive(Debug, Clone)]
pub struct RecordingMetricsRecorder {
    state: Arc<Mutex<RecordingMetricsState>>,
}

#[derive(Debug, Default)]
struct RecordingMetricsState {
    counters: Vec<(String, Vec<(String, String)>)>,
    histograms: Vec<(String, f64, Vec<(String, String)>)>,
    gauges: Vec<(String, f64, Vec<(String, String)>)>,
}

impl RecordingMetricsRecorder {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(RecordingMetricsState::default())),
        }
    }

    /// Return all recorded counter increments as `(name, labels)` pairs.
    pub fn counters(&self) -> Vec<(String, Vec<(String, String)>)> {
        self.state.lock().unwrap().counters.clone()
    }

    /// Return all recorded histogram observations as `(name, value, labels)` triples.
    pub fn histograms(&self) -> Vec<(String, f64, Vec<(String, String)>)> {
        self.state.lock().unwrap().histograms.clone()
    }

    /// Return all recorded gauge settings as `(name, value, labels)` triples.
    pub fn gauges(&self) -> Vec<(String, f64, Vec<(String, String)>)> {
        self.state.lock().unwrap().gauges.clone()
    }
}

impl crate::domain::MetricsRecorder for RecordingMetricsRecorder {
    fn increment_counter(&self, name: &str, labels: &[(&str, &str)]) {
        let owned_labels: Vec<(String, String)> = labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.state.lock().unwrap().counters.push((name.to_string(), owned_labels));
    }

    fn record_histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        let owned_labels: Vec<(String, String)> = labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.state.lock().unwrap().histograms.push((name.to_string(), value, owned_labels));
    }

    fn set_gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]) {
        let owned_labels: Vec<(String, String)> = labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.state.lock().unwrap().gauges.push((name.to_string(), value, owned_labels));
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p minibox-core recording_metrics`
Expected: PASS (3 tests)

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-core/src/adapters/mocks.rs crates/minibox-core/tests/recording_metrics_test.rs
git commit -m "feat(core): add RecordingMetricsRecorder test double"
```

---

## Task 4: Add `NoOpMetricsRecorder` and `PrometheusMetricsRecorder`

**Files:**
- Create: `crates/daemonbox/src/telemetry/mod.rs`
- Create: `crates/daemonbox/src/telemetry/noop.rs`
- Create: `crates/daemonbox/src/telemetry/prometheus.rs`
- Modify: `crates/daemonbox/Cargo.toml`
- Modify: `crates/daemonbox/src/lib.rs`

- [ ] **Step 1: Add dependencies to daemonbox/Cargo.toml**

Add to `[dependencies]`:

```toml
opentelemetry = { workspace = true }
opentelemetry_sdk = { workspace = true }
opentelemetry-otlp = { workspace = true }
tracing-opentelemetry = { workspace = true }
tracing-subscriber = { workspace = true }
prometheus-client = { workspace = true }
dashmap = { workspace = true }
axum = { workspace = true }
```

- [ ] **Step 2: Create `telemetry/mod.rs`**

```rust
//! Telemetry infrastructure adapters: metrics recording, trace export, metrics HTTP server.

mod noop;
mod prometheus_adapter;
pub mod server;
pub mod traces;

pub use noop::NoOpMetricsRecorder;
pub use prometheus_adapter::PrometheusMetricsRecorder;
```

Note: the Prometheus adapter file is named `prometheus_adapter.rs` to avoid shadowing the `prometheus-client` crate import.

- [ ] **Step 3: Create `telemetry/noop.rs`**

```rust
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
```

- [ ] **Step 4: Create `telemetry/prometheus_adapter.rs`**

```rust
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
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
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
        let registry = self.registry.lock().unwrap();
        let mut buffer = String::new();
        encode(&mut buffer, &registry).unwrap_or_default();
        buffer
    }

    fn get_or_create_counter(&self, name: &str) -> Family<Labels, Counter> {
        self.counters
            .entry(name.to_string())
            .or_insert_with(|| {
                let family = Family::<Labels, Counter>::default();
                self.registry
                    .lock()
                    .unwrap()
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
                    .unwrap()
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
                    .unwrap()
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
        // prometheus-client Gauge uses i64 by default; use set for atomic store.
        // For f64 gauges, cast to i64 (sufficient for our use cases).
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
        recorder.increment_counter("minibox_container_ops_total", &[("op", "run"), ("status", "ok")]);
        recorder.record_histogram("minibox_container_op_duration_seconds", 0.123, &[("op", "run")]);
        recorder.set_gauge("minibox_active_containers", 2.0, &[("adapter", "native")]);

        let output = recorder.encode_metrics();
        assert!(output.contains("minibox_container_ops_total"), "missing counter in output:\n{output}");
        assert!(output.contains("minibox_container_op_duration_seconds"), "missing histogram in output:\n{output}");
        assert!(output.contains("minibox_active_containers"), "missing gauge in output:\n{output}");
    }
}
```

- [ ] **Step 5: Add `pub mod telemetry;` to `daemonbox/src/lib.rs`**

Add the module declaration.

- [ ] **Step 6: Run tests**

Run: `cargo test -p daemonbox telemetry`
Expected: PASS (noop + prometheus tests)

- [ ] **Step 7: Commit**

```bash
git add crates/daemonbox/Cargo.toml crates/daemonbox/src/lib.rs crates/daemonbox/src/telemetry/
git commit -m "feat(daemonbox): add NoOp and Prometheus MetricsRecorder adapters"
```

---

## Task 5: Add OTEL Trace Exporter

**Files:**
- Create: `crates/daemonbox/src/telemetry/traces.rs`

- [ ] **Step 1: Write the test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_tracing_without_otlp_does_not_panic() {
        // This can only be called once per process; other tests may conflict.
        // Use a separate integration test if needed.
        // For now, verify the function signature compiles.
        let _guard = init_tracing(None);
    }
}
```

- [ ] **Step 2: Implement `traces.rs`**

```rust
//! OTEL trace exporter setup.
//!
//! Replaces the bare `tracing_subscriber::fmt().init()` in main.rs with a
//! layered subscriber that optionally adds OTLP trace export.
//!
//! Uses opentelemetry 0.31 APIs:
//! - `SdkTracerProvider` (not the removed `TracerProvider`)
//! - `.with_batch_exporter()` without runtime param (SDK manages its own threads since 0.28)
//! - `provider.shutdown()` on the instance (not the removed `global::shutdown_tracer_provider()`)

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

/// Initialize the tracing subscriber with optional OTLP trace export.
///
/// - `otlp_endpoint = None` → fmt-only logging (existing behavior)
/// - `otlp_endpoint = Some(url)` → fmt + OTEL trace export to the given endpoint
///
/// Returns an [`OtelGuard`] that must be held for the lifetime of the program.
/// On drop, it flushes pending spans via `SdkTracerProvider::shutdown()`.
pub fn init_tracing(otlp_endpoint: Option<&str>) -> OtelGuard {
    let env_filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("miniboxd=info".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer().boxed();

    if let Some(endpoint) = otlp_endpoint {
        match build_otel_layer(endpoint) {
            Ok((otel_layer, provider)) => {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt_layer)
                    .with(otel_layer)
                    .init();
                return OtelGuard {
                    provider: Some(provider),
                };
            }
            Err(e) => {
                // Fall back to fmt-only if OTEL init fails.
                eprintln!("OTEL trace init failed, falling back to fmt-only: {e}");
            }
        }
    }

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    OtelGuard { provider: None }
}

fn build_otel_layer(
    endpoint: &str,
) -> Result<
    (
        Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync>,
        opentelemetry_sdk::trace::SdkTracerProvider,
    ),
    Box<dyn std::error::Error>,
> {
    use opentelemetry_otlp::WithExportConfig;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;

    // SDK manages batch export threads internally since 0.28 — no runtime param needed.
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();

    let tracer = provider.tracer("miniboxd");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer).boxed();

    Ok((layer, provider))
}

/// Guard that shuts down the OTEL tracer provider on drop.
///
/// Hold this in `main()`. If OTLP was not configured, drop is a no-op.
///
/// Note: `global::shutdown_tracer_provider()` was removed in opentelemetry 0.28.
/// Must call `.shutdown()` on the `SdkTracerProvider` instance directly.
pub struct OtelGuard {
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            if let Err(e) = provider.shutdown() {
                eprintln!("OTEL tracer shutdown error: {e}");
            }
        }
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p daemonbox`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/daemonbox/src/telemetry/traces.rs crates/daemonbox/Cargo.toml
git commit -m "feat(daemonbox): add OTEL trace exporter with optional OTLP bridge"
```

---

## Task 6: Add Metrics HTTP Server

**Files:**
- Create: `crates/daemonbox/src/telemetry/server.rs`

- [ ] **Step 1: Write the integration test**

Create `crates/daemonbox/tests/metrics_server_test.rs`:

```rust
use daemonbox::telemetry::PrometheusMetricsRecorder;
use daemonbox::telemetry::server::run_metrics_server;
use minibox_core::domain::MetricsRecorder;
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::test]
async fn metrics_endpoint_returns_prometheus_format() {
    let recorder = Arc::new(PrometheusMetricsRecorder::new());
    recorder.increment_counter("test_counter_total", &[("label", "value")]);

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

    let (actual_addr, server_handle) = run_metrics_server(addr, recorder)
        .await
        .expect("server start");

    let url = format!("http://{actual_addr}/metrics");
    let body = reqwest::get(&url).await.expect("GET /metrics").text().await.expect("body");

    assert!(body.contains("test_counter_total"), "body should contain metric name; got:\n{body}");

    server_handle.abort();
}
```

- [ ] **Step 2: Add reqwest dev-dependency to daemonbox**

In `crates/daemonbox/Cargo.toml` `[dev-dependencies]`:

```toml
reqwest = { workspace = true }
tokio = { workspace = true }
minibox-core = { workspace = true }
```

- [ ] **Step 3: Implement `server.rs`**

```rust
//! Prometheus metrics HTTP server.
//!
//! Exposes a `/metrics` endpoint that returns Prometheus text exposition format.
//! Spawned as a separate Tokio task from the composition root.

use super::PrometheusMetricsRecorder;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Start the metrics HTTP server.
///
/// Takes an `Arc<PrometheusMetricsRecorder>` — the same instance injected into
/// handlers — and encodes its registry on each `/metrics` request.
///
/// Returns the actual bound address (useful when port 0 is used in tests)
/// and a `JoinHandle` for the server task.
pub async fn run_metrics_server(
    bind_addr: SocketAddr,
    recorder: Arc<PrometheusMetricsRecorder>,
) -> anyhow::Result<(SocketAddr, JoinHandle<()>)> {
    use axum::routing::get;

    let app = axum::Router::new().route(
        "/metrics",
        get(move || {
            let recorder = recorder.clone();
            async move { recorder.encode_metrics() }
        }),
    );

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|e| anyhow::anyhow!("metrics server bind {bind_addr}: {e}"))?;
    let actual_addr = listener.local_addr()?;

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "metrics server exited with error");
        }
    });

    Ok((actual_addr, handle))
}
```

- [ ] **Step 4: Run the integration test**

Run: `cargo test -p daemonbox metrics_endpoint_returns_prometheus_format`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/daemonbox/src/telemetry/server.rs crates/daemonbox/tests/metrics_server_test.rs crates/daemonbox/Cargo.toml
git commit -m "feat(daemonbox): add /metrics HTTP server for Prometheus scraping"
```

---

## Task 7: Wire `MetricsRecorder` into `HandlerDependencies`

**Files:**
- Modify: `crates/daemonbox/src/handler.rs` (lines 17–77)
- Modify: `crates/miniboxd/src/main.rs` (lines 306–368)

- [ ] **Step 1: Add `metrics` field to `HandlerDependencies`**

In `handler.rs`, add to the imports (line 17–26):

```rust
use minibox_core::domain::DynMetricsRecorder;
```

Add to the `HandlerDependencies` struct (after `run_containers_base`, line 76):

```rust
    /// Metrics recorder for operational observability.
    pub metrics: DynMetricsRecorder,
```

- [ ] **Step 2: Fix all `HandlerDependencies` construction sites**

In `crates/miniboxd/src/main.rs`, add to each `HandlerDependencies` block (Native, Gke, Colima):

```rust
    metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
```

This is a temporary no-op — Task 9 wires the real Prometheus recorder.

Also update any test files that construct `HandlerDependencies`. Search with:
`cargo check --workspace 2>&1 | grep "missing field"`

Fix each site by adding `metrics: Arc::new(NoOpMetricsRecorder::new())` or the `RecordingMetricsRecorder` equivalent.

- [ ] **Step 3: Verify workspace compiles**

Run: `cargo check --workspace`
Expected: PASS

- [ ] **Step 4: Run all tests**

Run: `cargo xtask test-unit`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/daemonbox/src/handler.rs crates/miniboxd/src/main.rs
git commit -m "feat(daemonbox): add metrics field to HandlerDependencies"
```

---

## Task 8: Instrument Handlers with Metrics

**Files:**
- Modify: `crates/daemonbox/src/handler.rs`

- [ ] **Step 1: Write a test verifying handler emits metrics**

Create `crates/daemonbox/tests/handler_metrics_test.rs`:

```rust
//! Verify that handlers record metrics via the injected MetricsRecorder.

use daemonbox::handler::HandlerDependencies;
use daemonbox::state::DaemonState;
use daemonbox::telemetry::NoOpMetricsRecorder;
use minibox_core::adapters::mocks::{
    MockFilesystem, MockLimiter, MockRegistry, MockRuntime, RecordingMetricsRecorder,
};
use minibox_core::domain::DynMetricsRecorder;
use minibox_core::image::ImageStore;
use std::sync::Arc;
use tokio::sync::mpsc;

fn test_deps(metrics: DynMetricsRecorder) -> Arc<HandlerDependencies> {
    Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest")),
        ghcr_registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: Arc::new(minibox_core::adapters::mocks::MockNetwork::new()),
        containers_base: tempfile::TempDir::new().unwrap().into_path(),
        run_containers_base: tempfile::TempDir::new().unwrap().into_path(),
        metrics,
    })
}

#[tokio::test]
async fn handle_pull_records_metrics() {
    let recorder = Arc::new(RecordingMetricsRecorder::new());
    let tmp = tempfile::TempDir::new().unwrap();
    let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());
    let state = Arc::new(DaemonState::new(store, tmp.path()));

    let deps = test_deps(recorder.clone());
    let (tx, _rx) = mpsc::channel(16);

    // Pull uses handle_pull which should record metrics.
    let _resp = daemonbox::handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state,
        deps,
    )
    .await;

    let counters = recorder.counters();
    assert!(
        counters.iter().any(|(name, _)| name == "minibox_container_ops_total"),
        "handle_pull should record minibox_container_ops_total; got: {counters:?}"
    );
}
```

Note: This test will need adjustment based on exact mock APIs. The key structure is: inject `RecordingMetricsRecorder`, call a handler, assert metrics were recorded.

- [ ] **Step 2: Add metrics recording to `handle_pull`**

In `handle_pull` (line 965), wrap the existing logic to record duration and status:

```rust
pub async fn handle_pull(
    image: String,
    tag: Option<String>,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let start = std::time::Instant::now();

    // ... existing logic ...

    let (status, response) = match registry.pull_image(&image_ref).await {
        Ok(_metadata) => ("ok", DaemonResponse::Success {
            message: format!("pulled {image}:{tag}"),
        }),
        Err(e) => {
            error!("handle_pull error: {e:#}");
            ("error", DaemonResponse::Error {
                message: format!("{e:#}"),
            })
        }
    };

    deps.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "pull"), ("adapter", "daemon"), ("status", status)],
    );
    deps.metrics.record_histogram(
        "minibox_container_op_duration_seconds",
        start.elapsed().as_secs_f64(),
        &[("op", "pull"), ("adapter", "daemon")],
    );

    response
}
```

- [ ] **Step 3: Add metrics recording to `run_inner`**

After the container is spawned successfully (around line 678), add:

```rust
    deps.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "run"), ("adapter", "daemon"), ("status", "ok")],
    );
```

In the error path of the spawn task (around line 670), add:

```rust
    deps.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "run"), ("adapter", "daemon"), ("status", "error")],
    );
```

- [ ] **Step 4: Add metrics recording to `handle_stop`**

After `stop_inner` returns:

```rust
    let status = if result.is_ok() { "ok" } else { "error" };
    deps.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "stop"), ("adapter", "daemon"), ("status", status)],
    );
```

- [ ] **Step 5: Add metrics recording to `handle_remove`**

Same pattern as stop:

```rust
    let status = if result.is_ok() { "ok" } else { "error" };
    deps.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "remove"), ("adapter", "daemon"), ("status", status)],
    );
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p daemonbox handler_metrics`
Expected: PASS

Run: `cargo xtask test-unit`
Expected: PASS (all existing tests still pass)

- [ ] **Step 7: Commit**

```bash
git add crates/daemonbox/src/handler.rs crates/daemonbox/tests/handler_metrics_test.rs
git commit -m "feat(daemonbox): instrument handlers with MetricsRecorder"
```

---

## Task 9: Wire Telemetry in Composition Root

**Files:**
- Modify: `crates/miniboxd/src/main.rs`

- [ ] **Step 1: Replace tracing init in main.rs**

Replace lines 245–251:

```rust
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("miniboxd=info".parse().unwrap()),
        )
        .init();
```

With:

```rust
    let otlp_endpoint = std::env::var("MINIBOX_OTLP_ENDPOINT").ok();
    let _otel_guard = daemonbox::telemetry::traces::init_tracing(otlp_endpoint.as_deref());
```

- [ ] **Step 3: Wire Prometheus recorder and metrics server**

After the `state loaded from disk` log line (line 304), add:

```rust
    // ── Metrics ─────────────────────────────────────────────────────────
    let metrics_addr: std::net::SocketAddr = std::env::var("MINIBOX_METRICS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9090".to_string())
        .parse()
        .context("parsing MINIBOX_METRICS_ADDR")?;

    let metrics_recorder = Arc::new(daemonbox::telemetry::PrometheusMetricsRecorder::new());

    let (_metrics_addr, _metrics_handle) =
        daemonbox::telemetry::server::run_metrics_server(metrics_addr, metrics_recorder.clone())
            .await
            .context("starting metrics server")?;
    info!(addr = %_metrics_addr, "metrics server listening");
```

- [ ] **Step 4: Update `HandlerDependencies` construction to use real recorder**

Replace the temporary `NoOpMetricsRecorder::new()` in all three adapter suite blocks with:

```rust
    metrics: metrics_recorder.clone(),
```

(The `metrics_recorder` is `Arc<PrometheusMetricsRecorder>` which implements `MetricsRecorder`, and `DynMetricsRecorder = Arc<dyn MetricsRecorder>` — the `Arc<PrometheusMetricsRecorder>` coerces to `Arc<dyn MetricsRecorder>`.)

- [ ] **Step 5: Verify full workspace compiles**

Run: `cargo check --workspace`
Expected: PASS

- [ ] **Step 6: Run macOS quality gates**

Run: `cargo fmt --all --check && cargo clippy -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -p minibox-llm -p minibox-secrets -- -D warnings && cargo xtask test-unit`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/miniboxd/src/main.rs
git commit -m "feat(miniboxd): wire OTEL tracing, Prometheus metrics, /metrics endpoint"
```

---

## Task 10: Update CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add telemetry env vars to Environment Variables section**

After the existing `MINIBOX_ADAPTER` entry, add:

```markdown
- `MINIBOX_METRICS_ADDR` — Prometheus `/metrics` bind address (default: `127.0.0.1:9090`)
- `MINIBOX_OTLP_ENDPOINT` — OTLP collector endpoint for trace export (unset = disabled)
```

- [ ] **Step 2: Add telemetry module to Architecture Overview**

In the `daemonbox/src/` section under Key Modules, add:

```markdown
- `telemetry/mod.rs`: Metrics and tracing infrastructure adapters
- `telemetry/prometheus_adapter.rs`: `PrometheusMetricsRecorder` — `prometheus-client` crate
- `telemetry/noop.rs`: `NoOpMetricsRecorder` for tests and disabled metrics
- `telemetry/traces.rs`: OTEL trace exporter setup with optional OTLP bridge
- `telemetry/server.rs`: axum `/metrics` HTTP endpoint
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: add telemetry module and env vars to CLAUDE.md"
```

---

## Deferred (follow-up work, not in this plan)

These canonical metrics from the spec are **not instrumented** in this plan. They require touching additional subsystems and should be added incrementally once the plumbing (Tasks 1–9) is proven:

- `minibox_active_containers` gauge — requires a periodic or event-driven gauge update from `DaemonState`
- `minibox_active_connections` gauge — requires instrumentation in `daemonbox::server`
- `minibox_image_pull_duration_seconds` / `minibox_image_pull_bytes_total` / `minibox_image_cache_hits_total` — requires instrumentation in image registry adapters
- `minibox_cgroup_cpu_usage_seconds` / `minibox_cgroup_memory_bytes` / `minibox_cgroup_pids` — Linux-only, requires periodic cgroup stat collection
- `minibox_overlay_mount_duration_seconds` — requires instrumentation in `FilesystemProvider`
- `minibox_request_errors_total` — requires error categorization in server layer
- `minibox_daemon_uptime_seconds` — requires a startup-time gauge in main.rs

These are straightforward `deps.metrics.record_*()` calls once the infrastructure exists.

---

## Summary

| Task | What | Files | Tests |
|------|------|-------|-------|
| 1 | Workspace deps | `Cargo.toml` | cargo check |
| 2 | `MetricsRecorder` trait | `domain.rs` | 1 unit |
| 3 | `RecordingMetricsRecorder` | `mocks.rs` | 3 unit |
| 4 | NoOp + Prometheus adapters | `telemetry/` (3 files) | 2 unit |
| 5 | OTEL trace exporter | `traces.rs` | 1 unit |
| 6 | Metrics HTTP server | `server.rs` | 1 integration |
| 7 | Wire into HandlerDependencies | `handler.rs`, `main.rs` | cargo check |
| 8 | Instrument handlers | `handler.rs` | 1 integration |
| 9 | Composition root wiring | `main.rs` | quality gates |
| 10 | Documentation | `CLAUDE.md` | — |
