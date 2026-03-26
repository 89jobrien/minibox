# OTEL Tracing & Prometheus Metrics Design

**Date:** 2026-03-26
**Status:** Approved
**Scope:** Add OpenTelemetry trace export and Prometheus metrics to miniboxd + agentbox

## Overview

Extend minibox's existing `tracing` infrastructure with OTEL trace export and add a Prometheus-compatible metrics subsystem. Follows the hexagonal architecture: metrics are a domain port (`MetricsRecorder` trait), with a Prometheus adapter in the infrastructure layer.

Two phases:

- **Phase 1 — miniboxd core** (this spec, sections 1–9): Wire `MetricsRecorder` trait, Prometheus adapter, OTEL trace bridge, `/metrics` endpoint. This is the foundation — must land first.
- **Phase 2 — agentbox** (section 10): Add Prometheus metrics to the Go agent runtime — turn count tracking, token/cost passthrough from Claude Agent SDK, Claude Code OTEL bridge. Depends on Phase 1 patterns being proven but is otherwise independent code (Go, not Rust).

## Approach

**Approach: `tracing-opentelemetry` bridge (traces) + `prometheus-client` (metrics)**

- Traces: existing `tracing` spans bridged to OTEL via `tracing-opentelemetry`, exported over OTLP
- Metrics: domain `MetricsRecorder` trait → `PrometheusMetricsRecorder` adapter (using `prometheus-client` crate) → axum `/metrics` HTTP endpoint
- OTEL SDK handles traces only; metrics use the official Prometheus Rust client directly
- Existing console logging (`tracing_subscriber::fmt`) unchanged

### Rejected alternatives

- **`opentelemetry-prometheus` crate:** Discontinued as of 0.29. Depends on unmaintained `protobuf` crate with security vulnerabilities. The OTEL team recommends migrating to OTLP push, but that requires Prometheus 3.0+ with `--web.enable-otlp-receiver`.
- **OTEL SDK for both traces and metrics (OTLP push):** Requires Prometheus 3.0+ with OTLP receiver enabled. We don't control the Prometheus deployment, and a pull-based `/metrics` endpoint is universally compatible.
- **Full OTEL SDK, no tracing bridge:** Requires reworking existing tracing init and span macros; OTEL Rust SDK less mature than `tracing`.

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
   │ (trait/port)   │    │ (prometheus-client)     │
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

| Metric                                   | Type      | Labels                    |
| ---------------------------------------- | --------- | ------------------------- |
| `minibox_container_ops_total`            | counter   | `op`, `adapter`, `status` |
| `minibox_container_op_duration_seconds`  | histogram | `op`, `adapter`           |
| `minibox_active_containers`              | gauge     | `adapter`                 |
| `minibox_active_connections`             | gauge     | —                         |
| `minibox_image_pull_duration_seconds`    | histogram | `registry`                |
| `minibox_image_pull_bytes_total`         | counter   | `registry`                |
| `minibox_image_cache_hits_total`         | counter   | —                         |
| `minibox_cgroup_cpu_usage_seconds`       | gauge     | `container_id`            |
| `minibox_cgroup_memory_bytes`            | gauge     | `container_id`            |
| `minibox_cgroup_pids`                    | gauge     | `container_id`            |
| `minibox_overlay_mount_duration_seconds` | histogram | —                         |
| `minibox_request_errors_total`           | counter   | `op`, `error_kind`        |
| `minibox_daemon_uptime_seconds`          | gauge     | —                         |

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
    registry: prometheus_client::registry::Registry,
    counters: DashMap<String, Family<Vec<(String, String)>, Counter>>,
    histograms: DashMap<String, Family<Vec<(String, String)>, Histogram>>,
    gauges: DashMap<String, Family<Vec<(String, String)>, Gauge>>,
}
```

- Uses `prometheus-client` (the official Prometheus Rust client maintained by the Prometheus team)
- Lazily creates/caches metric families via `DashMap<String, Family<...>>`
- Exposes `fn encode_metrics(&self) -> String` for the HTTP server (Prometheus text exposition format)
- No dependency on the OTEL SDK — metrics are a separate concern from traces

### NoOpMetricsRecorder

All methods are empty. Used in tests and when metrics are disabled.

### Trace Exporter (traces.rs)

Replaces the current `tracing_subscriber::fmt().init()` in main.rs:

```rust
pub fn init_tracing(otlp_endpoint: Option<&str>) -> OtelGuard {
    let env_filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("miniboxd=info".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer().boxed();

    if let Some(endpoint) = otlp_endpoint {
        // Build OTLP exporter (uses http-proto + reqwest by default in 0.31)
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()?;

        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_batch_exporter(exporter)  // SDK manages its own threads since 0.28
            .build();

        let tracer = provider.tracer("miniboxd");
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer).boxed();

        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .with(otel_layer)
            .init();

        return OtelGuard { provider: Some(provider) };
    }

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    OtelGuard { provider: None }
}

/// Held by main() — on drop, calls `provider.shutdown()` to flush pending spans.
/// No-op if OTLP was not configured.
///
/// Note: `global::shutdown_tracer_provider()` was removed in 0.28.
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

- `otlp_endpoint = None` → existing fmt-only behavior, no trace export
- `otlp_endpoint = Some(url)` → adds OTEL layer, spans become distributed traces
- Existing `info!`, `debug!`, `#[instrument]` all work unchanged

### Metrics HTTP Server (server.rs)

```rust
pub async fn run_metrics_server(
    bind_addr: SocketAddr,
    recorder: Arc<PrometheusMetricsRecorder>,
) -> Result<(SocketAddr, JoinHandle<()>)> {
    let app = axum::Router::new()
        .route("/metrics", get(move || {
            let recorder = recorder.clone();
            async move { recorder.encode_metrics() }
        }));

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let actual_addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    Ok((actual_addr, handle))
}
```

- Spawned as a separate Tokio task from main.rs
- Only serves `/metrics`
- No auth (localhost-only by default)
- Uses axum 0.7 API (`axum::serve` + `TcpListener`, not the removed `axum::Server::bind`)

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

let (_metrics_addr, _metrics_handle) =
    telemetry::server::run_metrics_server(metrics_addr, metrics_recorder.clone()).await?;

let deps = HandlerDependencies {
    // ... existing fields ...
    metrics: metrics_recorder,
};
```

## Environment Variables

| Variable                | Default              | Purpose                                                 |
| ----------------------- | -------------------- | ------------------------------------------------------- |
| `MINIBOX_METRICS_ADDR`  | `127.0.0.1:9090`     | Bind address for Prometheus `/metrics` endpoint         |
| `MINIBOX_OTLP_ENDPOINT` | _(unset = disabled)_ | OTLP collector endpoint (e.g., `http://localhost:4317`) |

## Dependencies

New workspace-level dependencies in `Cargo.toml`:

```toml
# Traces (OTEL bridge + OTLP export)
opentelemetry = "0.31"
opentelemetry_sdk = "0.31"
opentelemetry-otlp = { version = "0.31", features = ["grpc-tonic"] }
tracing-opentelemetry = "0.32"

# Metrics (direct Prometheus client, NOT the discontinued opentelemetry-prometheus)
prometheus-client = "0.23"

# Infrastructure
axum = { version = "0.7", features = ["tokio"] }
dashmap = "6"
```

### Version notes

- OTEL ecosystem aligned at 0.31 (Sep 2025). Breaking changes from 0.28: `global::shutdown_tracer_provider()` removed (use `SdkTracerProvider::shutdown()`), `install_batch(runtime::Tokio)` removed (SDK manages its own threads), OTLP defaults changed to `http-proto` + `reqwest`.
- `opentelemetry-prometheus` is discontinued (depends on unmaintained `protobuf` crate). Replaced with `prometheus-client` — the official Prometheus Rust client maintained by the Prometheus team.
- `opentelemetry` no longer needs `features = ["metrics"]` — metrics are not routed through OTEL SDK.
- `opentelemetry_sdk` no longer needs `features = ["rt-tokio", "metrics"]` — batch exporter manages its own threads since 0.28.

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

- **PrometheusMetricsRecorder:** Record metrics, call `encode_metrics()`, assert Prometheus text format contains expected names and labels
- **Metrics HTTP server:** Spawn on random port, GET `/metrics`, assert 200 with valid Prometheus exposition format
- **Trace exporter:** Verify `init_tracing(None)` falls back to fmt-only without panicking

### Platform Gating

- Cgroup metrics (`cgroup_cpu`, `cgroup_memory`, `cgroup_pids`) are Linux-only
- Tests for cgroup metric recording get `#[cfg(target_os = "linux")]`
- A separate test verifies the metrics recorder is still called on macOS (with zero/skipped values) to prevent silent coverage loss

---

## Phase 2: Agentbox Metrics

### Context

Agentbox (`agentbox/`) is a Go module that orchestrates AI agents (council, meta-agent, commit-msg) via the Claude Agent SDK. Each agent spawns a `claude` CLI subprocess that runs an autonomous tool-use loop. Three observability gaps exist:

1. **No turn count tracking** — agents can recurse indefinitely without visibility or hard limits
2. **No token/cost passthrough** — the SDK returns `total_cost_usd`, `usage`, and `num_turns` on every `ResultMessage`, but agentbox discards them
3. **No bridge to Claude Code OTEL** — each spawned `claude` process can export its own OTEL metrics (`claude_code.token.usage`, `claude_code.cost.usage`, etc.) but agentbox doesn't configure or aggregate them

### Approach

Use `prometheus/client_golang` directly. Agentbox is a short-lived CLI tool, not a long-running daemon, so metrics are exposed two ways:

- **Push**: write final metric values into the existing JSONL telemetry records (`agent-runs.jsonl`)
- **Pull** (optional): if `--metrics-port` is set, start a `promhttp.Handler()` on that port for the duration of the run — useful when agentbox is run inside a minibox container with a sidecar Prometheus scraper

### Canonical Agentbox Metrics

| Metric                                  | Type      | Labels                         | Source                                                |
| --------------------------------------- | --------- | ------------------------------ | ----------------------------------------------------- |
| `agentbox_agent_runs_total`             | counter   | `script`, `status`             | orchestrator                                          |
| `agentbox_agent_duration_seconds`       | histogram | `script`                       | orchestrator                                          |
| `agentbox_agent_turns_total`            | counter   | `script`, `agent_name`         | SDK `ResultMessage.num_turns`                         |
| `agentbox_agent_turn_limit_hit_total`   | counter   | `script`, `agent_name`         | SDK `ResultMessage.subtype == "error_max_turns"`      |
| `agentbox_agent_budget_limit_hit_total` | counter   | `script`, `agent_name`         | SDK `ResultMessage.subtype == "error_max_budget_usd"` |
| `agentbox_tokens_total`                 | counter   | `script`, `agent_name`, `type` | SDK `ResultMessage.usage`                             |
| `agentbox_cost_usd_total`               | counter   | `script`, `agent_name`         | SDK `ResultMessage.total_cost_usd`                    |
| `agentbox_pubsub_messages_total`        | counter   | `topic`                        | `ChannelBroker.Publish()`                             |
| `agentbox_pubsub_drops_total`           | counter   | `topic`                        | `ChannelBroker.Publish()` non-blocking drop           |

`type` label on `agentbox_tokens_total`: `input`, `output`, `cache_read`, `cache_creation` — mirrors Claude Code's `claude_code.token.usage` type attribute.

### Turn Count Safety

The Claude Agent SDK provides `max_turns` / `maxTurns` (caps tool-use round trips) and `max_budget_usd` / `maxBudgetUsd` (caps spend). When either limit is hit, the SDK returns a `ResultMessage` with subtype `error_max_turns` or `error_max_budget_usd`.

Agentbox should:

1. **Set `max_turns` on every `AgentConfig`** — default 50, overridable via `--max-turns` CLI flag
2. **Set `max_budget_usd`** — default $2.00 per agent, overridable via `--max-budget`
3. **Record limit-hit events** as prometheus counters (`agentbox_agent_turn_limit_hit_total`)
4. **Log a warning** when an agent exceeds 80% of its turn budget (early warning before hard stop)

This prevents infinite recursion in meta-agent spawned sub-agents, which is the primary risk Charlie identified.

### Token/Cost Passthrough

The SDK `ResultMessage` includes:

```
total_cost_usd    float64     — authoritative cost for this query() call
usage             map         — input_tokens, output_tokens, cache_read_input_tokens, cache_creation_input_tokens
num_turns         int         — tool-use turns completed
session_id        string      — for potential session resume
```

After each agent completes, `ClaudeSDKRunner` extracts these fields and:

1. Records to prometheus counters (`agentbox_tokens_total`, `agentbox_cost_usd_total`)
2. Attaches to the `AgentResult` struct (new fields: `Turns`, `TokensInput`, `TokensOutput`, `CostUSD`)
3. `DualWriter.WriteRun()` includes them in the JSONL record

### Extended JSONL Record Format

```json
{
  "run_id": "2026-03-26T14:30:45Z",
  "script": "council",
  "args": { "base": "main", "mode": "core" },
  "status": "complete",
  "duration_s": 120.5,
  "turns_total": 24,
  "tokens_input": 45200,
  "tokens_output": 12800,
  "tokens_cache_read": 31000,
  "tokens_cache_creation": 8500,
  "cost_usd": 0.412,
  "agents": [
    {
      "name": "strict-critic",
      "turns": 8,
      "tokens_input": 15000,
      "tokens_output": 4200,
      "cost_usd": 0.134
    },
    {
      "name": "creative-explorer",
      "turns": 9,
      "tokens_input": 16500,
      "tokens_output": 4800,
      "cost_usd": 0.152
    },
    {
      "name": "general-analyst",
      "turns": 7,
      "tokens_input": 13700,
      "tokens_output": 3800,
      "cost_usd": 0.126
    }
  ],
  "output": "## Strict Critic\nScore: 0.72\n..."
}
```

### Claude Code OTEL Bridge

When agentbox spawns `claude` CLI subprocesses via the Agent SDK, those processes can independently export OTEL metrics if configured. Agentbox sets the following env vars on spawned processes:

```go
cmd.Env = append(os.Environ(),
    "CLAUDE_CODE_ENABLE_TELEMETRY=1",
    "OTEL_METRICS_EXPORTER=otlp",
    "OTEL_EXPORTER_OTLP_PROTOCOL=grpc",
    fmt.Sprintf("OTEL_EXPORTER_OTLP_ENDPOINT=%s", otlpEndpoint),
)
```

This means each agent's `claude` process pushes its own `claude_code.token.usage`, `claude_code.cost.usage`, `claude_code.active_time.total` etc. to the same OTLP collector that miniboxd uses. The `session.id` attribute on Claude Code metrics correlates with the `session_id` returned in agentbox's `ResultMessage`, providing end-to-end traceability.

Agentbox does **not** duplicate Claude Code's metrics — it records orchestrator-level metrics (`agentbox_*`) that complement them:

```
Claude Code OTEL (per-process):     agentbox Prometheus (orchestrator):
  claude_code.token.usage              agentbox_tokens_total
  claude_code.cost.usage               agentbox_cost_usd_total
  claude_code.active_time.total        agentbox_agent_duration_seconds
                                       agentbox_agent_turns_total
                                       agentbox_agent_turn_limit_hit_total
                                       agentbox_pubsub_messages_total
```

### Module Layout

```
agentbox/internal/metrics/
├── metrics.go          // metric definitions, registry, init
├── recorder.go         // MetricsRecorder interface + prometheus impl
├── noop.go             // no-op impl for tests
└── server.go           // optional promhttp handler (--metrics-port)
```

### Dependencies

New in `agentbox/go.mod`:

```
github.com/prometheus/client_golang v1.20
```

### Testing

- **Unit**: `noop.go` recorder in all existing tests — zero change to existing behavior
- **Unit**: `RecordingRecorder` test double captures metric calls, asserts correct names/labels/values after orchestrator runs
- **Integration**: start metrics server on random port, run a mock agent, GET `/metrics`, assert prometheus text format contains expected `agentbox_*` metrics
- **Turn limit**: mock `ClaudeSDKRunner` returns `ResultMessage{subtype: "error_max_turns"}`, verify `agentbox_agent_turn_limit_hit_total` incremented and warning logged
