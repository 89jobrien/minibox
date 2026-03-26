# OTEL Tracing & Prometheus Metrics Design

**Date:** 2026-03-26
**Status:** Approved
**Scope:** Add OpenTelemetry trace export and Prometheus metrics to miniboxd

## Overview

Extend minibox's existing `tracing` infrastructure with OTEL trace export and add a Prometheus-compatible metrics subsystem. Follows the hexagonal architecture: metrics are a domain port (`MetricsRecorder` trait), with a Prometheus adapter in the infrastructure layer.

## Approach

**Approach A: `tracing-opentelemetry` bridge + `opentelemetry-prometheus`**

- Traces: existing `tracing` spans bridged to OTEL via `tracing-opentelemetry`, exported over OTLP
- Metrics: domain `MetricsRecorder` trait → `PrometheusMetricsRecorder` adapter → axum `/metrics` HTTP endpoint
- Single OTEL SDK for both traces and metrics
- Existing console logging (`tracing_subscriber::fmt`) unchanged

### Rejected alternatives

- **Approach B (separate `metrics` crate):** Two independent observability stacks to maintain; metrics aren't OTEL-native.
- **Approach C (full OTEL SDK, no tracing bridge):** Requires reworking existing tracing init and span macros; OTEL Rust SDK less mature than `tracing`.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                  Composition Root                   │
│               (miniboxd/src/main.rs)                │
│  Wires telemetry providers, starts metrics server   │
└──────────┬──────────────────────┬───────────────────┘
           │                      │
   ┌───────▼────────┐    ┌───────▼────────────────┐
   │  Domain Layer  │    │  Infrastructure         │
   │ (minibox-core) │    │     Adapters            │
   │                │    │                         │
   │ MetricsRecorder│◄───│ PrometheusMetricsRecorder│
   │ (trait/port)   │    │ (opentelemetry-prom)    │
   │                │    │                         │
   │                │    │ OtelTraceExporter       │
   │                │    │ (tracing-opentelemetry) │
   │                │    │                         │
   │                │    │ MetricsHttpServer       │
   │                │    │ (axum, /metrics)        │
   └────────────────┘    └────────────────────────┘
```

No new crate. The `MetricsRecorder` trait goes in `minibox-core/src/domain.rs`. Adapters go in `daemonbox/src/telemetry/`.

## Domain Port

```rust
/// Port for recording operational metrics.
/// Adapters: Prometheus (production), NoOp (testing/disabled).
pub trait MetricsRecorder: Send + Sync {
    fn increment_counter(&self, name: &str, labels: &[(&str, &str)]);
    fn record_histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]);
    fn set_gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]);
}
```

- Three methods cover all Prometheus metric types (counters, histograms, gauges)
- String-based names + labels keeps domain free of OTEL/Prometheus types
- `Send + Sync` required for cross-task sharing

## Canonical Metrics

| Metric | Type | Labels |
|--------|------|--------|
| `minibox_container_ops_total` | counter | `op`, `adapter`, `status` |
| `minibox_container_op_duration_seconds` | histogram | `op`, `adapter` |
| `minibox_active_containers` | gauge | `adapter` |
| `minibox_active_connections` | gauge | — |
| `minibox_image_pull_duration_seconds` | histogram | `registry` |
| `minibox_image_pull_bytes_total` | counter | `registry` |
| `minibox_image_cache_hits_total` | counter | — |
| `minibox_cgroup_cpu_usage_seconds` | gauge | `container_id` |
| `minibox_cgroup_memory_bytes` | gauge | `container_id` |
| `minibox_cgroup_pids` | gauge | `container_id` |
| `minibox_overlay_mount_duration_seconds` | histogram | — |
| `minibox_request_errors_total` | counter | `op`, `error_kind` |
| `minibox_daemon_uptime_seconds` | gauge | — |

## Infrastructure Adapters

### Module layout

```
daemonbox/src/telemetry/
├── mod.rs              // re-exports, TelemetryConfig
├── prometheus.rs       // PrometheusMetricsRecorder
├── noop.rs             // NoOpMetricsRecorder
├── traces.rs           // OTEL trace exporter setup
└── server.rs           // axum /metrics HTTP server
```

### PrometheusMetricsRecorder

```rust
pub struct PrometheusMetricsRecorder {
    meter: opentelemetry::metrics::Meter,
    exporter: opentelemetry_prometheus::PrometheusExporter,
}
```

- Creates an OTEL `MeterProvider` with Prometheus exporter on construction
- Lazily creates/caches OTEL instrument handles via `DashMap<String, InstrumentHandle>`
- Exposes `fn registry(&self) -> &prometheus::Registry` for the HTTP server

### NoOpMetricsRecorder

All methods are empty. Used in tests and when metrics are disabled.

### Trace Exporter (traces.rs)

Replaces the current `tracing_subscriber::fmt().init()` in main.rs:

```rust
pub fn init_tracing(otlp_endpoint: Option<&str>) -> OtelGuard {
    let mut layers = vec![
        tracing_subscriber::fmt::layer().boxed(),
        tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("miniboxd=info".parse().unwrap()).boxed(),
    ];

    if let Some(endpoint) = otlp_endpoint {
        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_exporter(/* OTLP gRPC to endpoint */)
            .install_batch(opentelemetry_sdk::runtime::Tokio);
        layers.push(tracing_opentelemetry::layer().with_tracer(tracer).boxed());
    }

    tracing_subscriber::registry().with(layers).init();
    OtelGuard { shutdown_tracer: otlp_endpoint.is_some() }
}

/// Held by main() — on drop, calls `opentelemetry::global::shutdown_tracer_provider()`
/// to flush pending spans. No-op if OTLP was not configured.
pub struct OtelGuard { shutdown_tracer: bool }
impl Drop for OtelGuard {
    fn drop(&mut self) {
        if self.shutdown_tracer {
            opentelemetry::global::shutdown_tracer_provider();
        }
    }
}
```

- `otlp_endpoint = None` → existing fmt-only behavior, no trace export
- `otlp_endpoint = Some(url)` → adds OTEL layer, spans become distributed traces
- Existing `info!`, `debug!`, `#[instrument]` all work unchanged

### Metrics HTTP Server (server.rs)

```rust
pub async fn run_metrics_server(
    bind_addr: SocketAddr,
    registry: prometheus::Registry,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = axum::Router::new()
        .route("/metrics", get(/* encode registry to Prometheus text format */));
    axum::Server::bind(&bind_addr).serve(app.into_make_service()).await
}
```

- Spawned as a separate Tokio task from main.rs
- Only serves `/metrics`
- No auth (localhost-only by default)

## Integration

### HandlerDependencies

```rust
pub struct HandlerDependencies {
    pub registry: Box<dyn ImageRegistry>,
    pub filesystem: Box<dyn Filesystem>,
    pub resource_limiter: Box<dyn ResourceLimiter>,
    pub runtime: Box<dyn ContainerRuntime>,
    pub network_provider: Box<dyn NetworkProvider>,
    pub metrics: Arc<dyn MetricsRecorder>,  // new
}
```

`Arc` because the recorder is shared across concurrent handler tasks and the HTTP server.

### Handler Instrumentation Pattern

```rust
pub async fn handle_run(&self, req: RunRequest) -> Result<RunResponse, Error> {
    let start = std::time::Instant::now();
    let result = self.do_run(req).await;

    let status = if result.is_ok() { "ok" } else { "error" };
    self.deps.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "run"), ("adapter", &self.adapter_name), ("status", status)],
    );
    self.deps.metrics.record_histogram(
        "minibox_container_op_duration_seconds",
        start.elapsed().as_secs_f64(),
        &[("op", "run"), ("adapter", &self.adapter_name)],
    );

    result
}
```

Same pattern for `handle_stop`, `handle_remove`, `handle_list`, `handle_pull`.

### Composition Root (main.rs)

```rust
let metrics_addr = env::var("MINIBOX_METRICS_ADDR")
    .unwrap_or_else(|_| "127.0.0.1:9090".to_string())
    .parse::<SocketAddr>()?;
let otlp_endpoint = env::var("MINIBOX_OTLP_ENDPOINT").ok();

let _otel_guard = telemetry::traces::init_tracing(otlp_endpoint.as_deref());

let metrics_recorder = Arc::new(PrometheusMetricsRecorder::new());
let prometheus_registry = metrics_recorder.registry().clone();

tokio::spawn(telemetry::server::run_metrics_server(metrics_addr, prometheus_registry));

let deps = HandlerDependencies {
    // ... existing fields ...
    metrics: metrics_recorder,
};
```

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `MINIBOX_METRICS_ADDR` | `127.0.0.1:9090` | Bind address for Prometheus `/metrics` endpoint |
| `MINIBOX_OTLP_ENDPOINT` | *(unset = disabled)* | OTLP collector endpoint (e.g., `http://localhost:4317`) |

## Dependencies

New workspace-level dependencies in `Cargo.toml`:

```toml
opentelemetry = { version = "0.28", features = ["metrics"] }
opentelemetry_sdk = { version = "0.28", features = ["rt-tokio", "metrics"] }
opentelemetry-otlp = { version = "0.28", features = ["grpc-tonic"] }
opentelemetry-prometheus = "0.28"
tracing-opentelemetry = "0.29"
prometheus = "0.13"
axum = { version = "0.7", features = ["tokio"] }
dashmap = "6"
```

## Testing Strategy

### Unit Tests

- **NoOpMetricsRecorder** in all existing handler tests — zero behavior change, just add the field
- **RecordingMetricsRecorder** test double for verifying metrics are emitted:

```rust
pub struct RecordingMetricsRecorder {
    pub counters: Mutex<Vec<(String, Vec<(String, String)>)>>,
    pub histograms: Mutex<Vec<(String, f64, Vec<(String, String)>)>>,
    pub gauges: Mutex<Vec<(String, f64, Vec<(String, String)>)>>,
}
```

### Integration Tests

- **PrometheusMetricsRecorder:** Record metrics, encode registry, assert Prometheus text format contains expected names and labels
- **Metrics HTTP server:** Spawn on random port, GET `/metrics`, assert 200 with valid Prometheus exposition format
- **Trace exporter:** Verify `init_tracing(None)` falls back to fmt-only without panicking

### Platform Gating

- Cgroup metrics (`cgroup_cpu`, `cgroup_memory`, `cgroup_pids`) are Linux-only
- Tests for cgroup metric recording get `#[cfg(target_os = "linux")]`
- A separate test verifies the metrics recorder is still called on macOS (with zero/skipped values) to prevent silent coverage loss
