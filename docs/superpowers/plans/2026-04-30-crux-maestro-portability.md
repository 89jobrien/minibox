# Plan: Crux–Minibox–Maestro Portability

**Date:** 2026-04-30
**Status:** pending
**Supersedes:** `docs/superpowers/specs/archived/2026-04-20-crux-maestro-integration-design.md`

## Adapter Default (as of 2026-04-30)

`MINIBOX_ADAPTER` defaults to `smolvm` with automatic fallback to `krun` when the `smolvm`
binary is absent. The `handle_pipeline` handler and `minibox-crux-plugin` binary in this plan
must work under both smolvm and krun — they use the daemon client (Unix socket) and do not
depend on the specific adapter internals.

## What Changed Since the Archived Design

The archived design (2026-04-20) assumed `minibox-agent` existed as a bridge crate. It
does not — the crate listed in the archived spec was never created. The crux workspace has
however progressed significantly:

| Component | Archived assumption | Current reality |
|-----------|--------------------|--------------------|
| `cruxx-types` | "needs to be extracted" | **Exists** at `crux/crates/cruxx-types` — `Crux<T>`, `Step`, `Budget`, `CruxId` etc. all present |
| `cruxx-plugin` | "future plugin protocol" | **Exists** at `crux/crates/cruxx-plugin` — full `PluginHost` + `Declare`/`Invoke`/`Shutdown` JSON-RPC over stdio |
| `cruxx-script` | "load .cruxx pipelines" | **Exists** at `crux/crates/cruxx-script` |
| `minibox-agent` | "bridge crate" | **Does not exist** — no crate in minibox workspace |
| `handle_pipeline` | "new daemon handler" | **Protocol snapshot exists** (`run_pipeline_request_snapshot.snap`) but handler not wired |

This plan is updated to match current crate topology and avoids recreating what already exists.

---

## Goal

A single `.cruxx` pipeline YAML runs identically:

- In a minibox container on macOS/Linux via the `minibox-crux-plugin` binary
- In a Maestro GKE pod via a `maestro-crux-plugin` binary (future, not in this plan)

Crux owns scheduling, tracing, and pipeline logic. Minibox provides the container
infrastructure via the plugin protocol.

---

## Crate Topology (updated)

### No new library crate needed

`cruxx-types` already provides the shared serializable types. The plugin protocol is
already in `cruxx-plugin`. No new `minibox-agent` library crate is needed.

### New binary: `minibox-crux-plugin`

A thin binary in the minibox workspace that:
1. Reads `cruxx-plugin::protocol::Request` (JSON lines) from stdin
2. Responds with `cruxx-plugin::protocol::Response` (JSON lines) on stdout
3. On `Declare`: returns the minibox handler list
4. On `Invoke { handler, input }`: routes to the appropriate minibox operation

**Location:** `crates/minibox-crux-plugin/` (new crate, binary only)

**Dependencies:**
- `cruxx-plugin` (published, `crates.io`) — protocol types + `PluginBridge` helper
- `minibox-core` (workspace) — `DaemonClient` for daemon communication
- `tokio` (async runtime)
- `serde_json` (input/output marshalling)

### Handlers exposed by `minibox-crux-plugin`

| Handler name | Maps to | Notes |
|-------------|---------|-------|
| `minibox::container::run` | `DaemonClient::run_container` | Creates container, returns container ID |
| `minibox::container::stop` | `DaemonClient::stop_container` | |
| `minibox::container::rm` | `DaemonClient::remove_container` | |
| `minibox::container::exec` | `DaemonClient::exec` | Linux native only; returns error on macOS |
| `minibox::container::ps` | `DaemonClient::list_containers` | |
| `minibox::container::logs` | `DaemonClient::logs` | |
| `minibox::image::pull` | `DaemonClient::pull_image` | |
| `minibox::image::build` | `DaemonClient::build_image` | Requires `mbx build` (see CI wiring plan) |
| `minibox::image::push` | `DaemonClient::push_image` | |

Each handler input is a `serde_json::Value` that maps to the relevant `DaemonRequest`
variant. Each handler output is a `serde_json::Value` wrapping the `DaemonResponse`.

---

## Protocol: `handle_pipeline` in daemon

The `RunPipeline` `DaemonRequest` variant already has a snapshot test confirming the wire
format. The `handle_pipeline` handler needs to be implemented in
`crates/minibox/src/daemon/handler.rs`.

### `handle_pipeline` implementation

```
1. Receive RunPipeline { pipeline_path, input, image, budget, env }
2. Pull image (default: "alpine:latest") if not cached
3. Create container with the pipeline file bind-mounted at /pipeline.cruxx
4. Set entrypoint: ["crux", "run", "/pipeline.cruxx"]
5. Inject budget as CRUXX_BUDGET_JSON env var (serialized Budget from cruxx-types)
6. Inject minibox-crux-plugin as CRUXX_PLUGIN_PATH env var
7. Stream ContainerOutput responses as the pipeline runs
8. On container exit: emit PipelineComplete { trace, container_id, exit_code }
```

`PipelineComplete.trace` is `serde_json::Value` — the raw JSON written by `crux run` to
a mounted volume (`/trace.json`). The daemon reads this file after container exits and
includes it in the response. No `cruxx-types` dependency in minibox-core — trace is opaque
JSON.

**Note:** The `cruxx-runtime` base image (Alpine + `crux` binary + `minibox-crux-plugin`)
must be built and published before this handler can be e2e tested. For now, the handler
accepts any image and the integration test uses a stub container that writes a minimal
`{"steps": []}` trace file.

---

## Tasks (ordered)

### Task 1 — `minibox-crux-plugin` binary scaffold

**New crate:** `crates/minibox-crux-plugin/`

```toml
[package]
name = "minibox-crux-plugin"
edition.workspace = true
license.workspace = true

[[bin]]
name = "minibox-crux-plugin"
path = "src/main.rs"

[dependencies]
cruxx-plugin = "0.x"    # published — pin to current version
minibox-core = { path = "../minibox-core" }
tokio = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }
```

`src/main.rs` — main loop:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Read lines from stdin, dispatch to handlers, write responses to stdout.
    // Use cruxx_plugin::bridge::run_plugin(handlers) if that helper exists,
    // otherwise hand-roll the read loop.
}
```

**Tests:** unit tests for each handler mapping function (pure JSON in → JSON out, mock
DaemonClient responses).

---

### Task 2 — `handle_pipeline` daemon handler

**File:** `crates/minibox/src/daemon/handler.rs`

Wire the existing `RunPipeline` protocol variant. The handler:

1. Pulls image via `self.deps.registry`
2. Creates container with:
   - `bind_mounts`: `[{ host: pipeline_path, container: "/pipeline.cruxx", readonly: true }]`
   - `env`: includes `CRUXX_BUDGET_JSON` and `CRUXX_PLUGIN_PATH`
   - `command`: `["crux", "run", "/pipeline.cruxx", "--output", "/trace.json"]`
3. Streams `ContainerOutput` messages (existing streaming path)
4. After container exits: reads `/trace.json` from the container's upper dir via
   `OverlayCommitAdapter::read_file_from_upper()` (new helper) or direct path access
5. Emits `PipelineComplete { trace, container_id, exit_code }`

**New protocol helper:** `DaemonResponse::PipelineComplete` already exists in the snapshot.
Confirm `is_terminal_response()` in `server.rs` includes it.

**Tests:**
- Unit test: `handle_pipeline` with mock deps; assert `PipelineComplete` emitted after
  `ContainerStopped`
- Snapshot test: `PipelineComplete` JSON wire format (update existing snapshot if needed)

---

### Task 3 — `TraceStore` trait + `FileTraceStore`

**File:** `crates/minibox-core/src/trace.rs` (already exists — check if `TraceStore` is there)

If `trace.rs` does not have `TraceStore`, add:

```rust
pub trait TraceStore: Send + Sync {
    fn store(&self, pipeline_id: &str, trace_json: &serde_json::Value) -> Result<()>;
    fn list(&self) -> Result<Vec<TraceSummary>>;
    fn load(&self, pipeline_id: &str) -> Result<Option<serde_json::Value>>;
}

pub struct FileTraceStore {
    base_dir: PathBuf,
    // retention_days: u32,  -- from MINIBOX_TRACE_RETENTION_DAYS or default 7
    // max_bytes: u64,       -- from MINIBOX_TRACE_MAX_MB or default 500*1024*1024
}
```

`FileTraceStore::store()` writes to `<base_dir>/<pipeline_id>-<unix_ts>.json` and prunes
oldest files when retention or size limits exceeded.

Wire into `DaemonState` via `Arc<dyn TraceStore>` field (optional, defaults to
`FileTraceStore` with `~/.minibox/traces/`).

---

### Task 4 — `mbx pipeline` CLI command

**File:** `crates/mbx/src/commands/pipeline.rs` (new)

```
mbx pipeline run <pipeline.cruxx> [--input <input.json>] [--image <image>]
mbx pipeline list [--since <duration>]
mbx pipeline show <trace-id>
```

`run` sends `RunPipeline` request and streams `ContainerOutput` to the terminal, then
prints the `PipelineComplete` trace summary.

`list` and `show` send new `ListTraces` / `GetTrace` daemon requests (new protocol
variants, `#[serde(default)]` on all fields).

---

### Task 5 — Maestro portability stub

**New file:** `docs/superpowers/specs/2026-04-30-maestro-crux-plugin-design.md`

Document the future `maestro-crux-plugin` binary — its handler namespace
(`maestro::session::*`), how it maps to `maestro-runtime` gRPC/HTTP calls, and the
`plugins.toml` switching mechanism. This is a design doc only — no implementation until
the minibox-crux-plugin binary is shipping and proven.

---

## Acceptance Criteria

- [ ] `minibox-crux-plugin` binary compiles and responds to `Declare` with the handler list
- [ ] `handle_pipeline` handler emits `PipelineComplete` after a container exits
- [ ] `FileTraceStore` writes and rotates trace files under `~/.minibox/traces/`
- [ ] `mbx pipeline run` works end-to-end against a stub container that outputs a minimal
  trace file
- [ ] `cargo xtask pre-commit` clean
- [ ] `PipelineComplete` snapshot test in `minibox-core` matches current wire format

## Out of Scope

- `maestro-crux-plugin` binary implementation (Toptal project, future)
- `cruxx-runtime` base image build/publish (crux workspace concern)
- Multi-container pipeline fan-out (future, after single-container path works)
- Budget propagation across containers (future)
