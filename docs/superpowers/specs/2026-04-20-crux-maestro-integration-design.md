# Crux-Minibox Integration Design

**Date:** 2026-04-20
**Status:** Draft
**Author:** Joseph O'Brien

## Overview

Deepen the integration between crux (agentic Rust DSL) and minibox
(container runtime) so that crux is the primary orchestration layer for
agent work, and minibox is the infrastructure provider. Crux owns
scheduling, agent coordination, pipelines, and traces. Minibox provides
containers, process management, secrets, and networking.

The long-term goal: crux pipelines orchestrate agent work inside minibox
containers, with the same pipeline definitions usable inside maestro
(Toptal) containers via the crux plugin protocol — making crux the
portable orchestration layer across both personal and employer
infrastructure.

## Existing State

`minibox-agent` already bridges crux into minibox:

- Depends on published `crux-agentic` (0.2.3) and `cruxai-core` (0.2.1)
- `FallbackChainAdapter` implements `crux_agentic::LlmProvider` port,
  wrapping `minibox-llm::FallbackChain`
- `CruxLlmStep` records LLM calls as crux steps with replay/budget

What's missing: pipeline execution, multi-agent coordination, trace
storage, container-level integration, and the bidirectional plugin
protocol.

## Goals

1. **Structured agent orchestration** — crux pipelines replace ad-hoc
   agent invocations with traceable, branching, multi-step workflows
2. **Runtime observability** — `Crux<T>` traces give minibox containers
   structured execution logs with step-level granularity
3. **Multi-agent coordination** — single-container (subprocess) and
   multi-container agent fan-out via crux combinators
4. **Pipeline-driven workflows** — `.cruxx` YAML pipelines as the
   primary way to define and execute agent work
5. **Maestro portability** — crux pipelines run identically in minibox
   containers and maestro K8s pods via the plugin protocol

## Crate Topology

### New crate: `cruxx-types` (crux workspace)

Extracted from `cruxx-core`. Contains only serializable types with
minimal dependencies (`serde`, `chrono`, `ulid`).

Exports:

- `Crux<T>` — execution trace fused with result
- `Step`, `StepKind`, `StepStatus` — recorded unit of work
- `CruxId`, `TaskId` — identifiers
- `Budget`, `BudgetKind` — token/step/time/cost limits
- `CruxErr` — error type
- `RecoveryKind` — serializable enum subset of `Recovery<T>` (minus
  closure variants `RetryWith`, `Escalate`)

`cruxx-core` re-exports everything from `cruxx-types` — no breaking
change for existing consumers. Published crates (`cruxai-core`,
`crux-agentic`) update to depend on `cruxx-types` instead of inlining
the types.

### Expanded: `minibox-agent` (minibox workspace)

Currently a thin LLM bridge. Expands to become the crux integration
hub:

- **Existing:** `FallbackChainAdapter`, `CruxLlmStep`, `AgentError`
- **New:** `PipelineRunner` — loads `.cruxx` files, wires handler
  registry with minibox-specific handlers, executes pipelines
- **New:** `TraceStore` trait + `FileTraceStore` adapter
- **New:** Minibox handler set for the crux `HandlerRegistry`
  (`minibox::container::*`, `minibox::exec::*`, `minibox::env::*`)
- **New:** `MiniboxPlugin` binary — implements `cruxx-plugin` JSON-RPC
  protocol, exposing minibox container ops as crux pipeline handlers

New dependencies: `cruxx-script` (pipeline loading/running),
`cruxx-plugin` (plugin protocol), `minibox-client` (daemon
communication), `minibox-secrets` (credential resolution).

### No new middleware in maestro

Maestro integration happens later, through the same `cruxx-plugin`
protocol. A future `maestro-crux-plugin` binary exposes
`maestro::session::*` handlers. Crux pipelines call either
`minibox::container::*` or `maestro::session::*` depending on where they
run — the pipeline YAML is the same, only the `plugins.toml` differs.

## Bidirectional Plugin Protocol

### Minibox as a crux plugin

A plugin binary (`minibox-crux-plugin`) runs alongside `crux run` and
implements the `cruxx-plugin` JSON-RPC protocol (newline-delimited JSON
over stdin/stdout). It exposes minibox infrastructure:

| Handler                    | Purpose                                |
| -------------------------- | -------------------------------------- |
| `minibox::container::run`  | Create and start a container           |
| `minibox::container::stop` | Stop a container                       |
| `minibox::container::rm`   | Remove a container                     |
| `minibox::container::exec` | Execute a command in running container |
| `minibox::container::ps`   | List containers                        |
| `minibox::container::logs` | Stream container output                |
| `minibox::image::pull`     | Pull an OCI image                      |
| `minibox::env::inject`     | Resolve secrets via minibox-secrets    |

The plugin binary wraps `minibox-client::DaemonClient` calls — it
communicates with `miniboxd` over the Unix socket, not by shelling out
to the CLI.

Plugin manifest (`~/.cruxx/plugins.toml` or `.cruxx/plugins.toml`):

```toml
[[plugin]]
name = "minibox"
path = "minibox-crux-plugin"
```

### Crux as a minibox agent runner

`daemonbox` gains a new handler: `handle_pipeline`. When a
`RunPipeline` request arrives (new protocol variant), the daemon:

1. Pulls the container image if needed
2. Creates the container with overlay FS
3. Copies/mounts the `.cruxx` pipeline + input into the container
4. Runs `crux run` as the container entrypoint with
   `minibox-crux-plugin` as a registered plugin
5. Streams `Crux<T>` trace JSON back via `ContainerOutput` protocol
6. Stores the trace via `TraceStore` on completion

This is a higher-level operation than `RunContainer` — it bundles image
pull + container create + pipeline execution + trace collection.

## Multi-Agent Coordination

### Single-container mode

A crux pipeline runs within one minibox container. It uses crux
combinators to coordinate agents as subprocesses:

- **`delegate()`** — invoke a named agent (e.g., Claude Code via
  `shell::exec` calling `claude --prompt "..." --output-format json`)
- **`join_all()`** — parallel fan-out, each arm gets its own subprocess
- **`speculate()`** — race multiple approaches, pick best or first
  success
- **`pipe()`** — sequential multi-step chains
- **`route_on_confidence()`** — confidence-band dispatch

### Multi-container mode

A pipeline step calls `minibox::container::run` (via the minibox plugin) to
spin up a new container, then `minibox::container::exec` to dispatch work.
The pipeline blocks until the container exits and collects the result.

`join_all()` parallelizes this naturally — each arm starts its own
container, waits for completion, results merge back into the parent
trace.

**Trace aggregation:** Each child container writes its own `Crux<T>`
trace to a mounted volume. The orchestrating pipeline reads child traces
and attaches them as `children` on the parent `Crux<T>`.

**Budget propagation:** The parent pipeline's `Budget` is split across
delegations. Each `minibox::container::run` call passes a budget allocation
as an environment variable, enforced by the `crux run` instance in the
child container.

## Observability

### Trace storage

`TraceStore` trait in `minibox-agent` with one initial backend:

- **`FileTraceStore`** — writes to `~/.minibox/traces/<pipeline>-<timestamp>.json`.

The trait is a hexagonal port. Future adapters (SQLite, remote API) can
be added without changing the pipeline runner.

```rust
pub trait TraceStore: Send + Sync {
    fn store(&self, trace: &Crux<serde_json::Value>) -> Result<()>;
    fn list(&self, filter: &TraceFilter) -> Result<Vec<TraceSummary>>;
    fn load(&self, id: &CruxId) -> Result<Option<Crux<serde_json::Value>>>;
}
```

### Trace format

`Crux<T>` serializes natively via serde. Each trace includes:

- Full step tree (name, kind, status, confidence, duration,
  input/output hashes)
- Child traces from delegations (including multi-container)
- Budget consumption per step
- Causal chain for failures (`causal_chain()`)

### Surfacing traces

| Surface | Mechanism                                             |
| ------- | ----------------------------------------------------- |
| CLI     | `minibox traces list` — list recent traces            |
| CLI     | `minibox traces show <id>` — render trace tree        |
| TUI     | New `dashbox` tab — trace viewer with step drill-down |
| File    | `~/.minibox/traces/` directory for external tools     |

## Pipeline-Driven Workflows

### Pipeline discovery

`PipelineRunner` looks for `.cruxx` files in priority order:

1. Passed explicitly: `minibox run-pipeline <path>`
2. Project-specific: `<workspace>/.cruxx/pipelines/`
3. User-global: `~/.cruxx/pipelines/`

### Default pipeline

A `work.cruxx` default pipeline wraps the common pattern: single Claude
Code invocation with a prompt as input, output captured as trace.

### CLI integration

New `minibox-cli` subcommand:

```bash
minibox run-pipeline <pipeline.cruxx> [--input <input.json>]
minibox traces list [--since 24h]
minibox traces show <trace-id>
```

`run-pipeline` is sugar for: pull image (if needed) + create container +
run `crux run` with minibox plugin + stream trace back.

## Maestro Portability

The same `.cruxx` pipeline runs in both minibox and maestro. The only
difference is which plugin binary is available:

| Environment | Plugin binary                  | Handlers available                         |
| ----------- | ------------------------------ | ------------------------------------------ |
| minibox     | `minibox-crux-plugin`          | `minibox::container::*`, `minibox::env::*` |
| maestro     | `maestro-crux-plugin` (future) | `maestro::session::*`, `maestro::env::*`   |

Pipeline authors use abstract handler names where possible. For
infrastructure-specific operations, the handler namespace (`minibox::` vs
`maestro::`) makes the dependency explicit.

A future `cruxx-infra` trait in the crux workspace could abstract over
both — `ContainerProvider` with `run`, `exec`, `stop` — but this is
premature until both plugin binaries exist.

## Protocol Changes

### New `DaemonRequest` variant

```rust
RunPipeline {
    pipeline_path: String,
    input: Option<serde_json::Value>,
    image: Option<String>,       // default: alpine or a crux-ready image
    budget: Option<Budget>,
    #[serde(default)]
    env: Vec<(String, String)>,
}
```

### New `DaemonResponse` variant

```rust
PipelineComplete {
    trace: Crux<serde_json::Value>,
    container_id: String,
    exit_code: i32,
}
```

Both require `#[serde(default)]` on new fields per minibox protocol
conventions. Snapshot tests must be added to `minibox-core`.

## Open Questions

- **~~cruxx-run packaging~~** — RESOLVED: `crux` is already a published
  binary (`crux run`, `crux plan`). Bake the `crux` binary into a
  `cruxx-runtime` base image alongside `minibox-crux-plugin`. No
  separate `cruxx-run` needed.
- **~~Pipeline versioning~~** — RESOLVED: Yes. Add `version: 1` field
  at top of `.cruxx` files for forward-compatible parsing.
- **~~Trace retention~~** — RESOLVED: 7-day retention, 500MB cap.
  `FileTraceStore` rotates oldest when either limit is hit. Override
  via `MINIBOX_TRACE_RETENTION_DAYS` / `MINIBOX_TRACE_MAX_MB`.
- **~~Image for pipeline containers~~** — RESOLVED: Dedicated
  `cruxx-runtime` image (Alpine + `crux` + `minibox-crux-plugin` +
  jq, curl, git). Published alongside crux releases.
- **~~VPS mode~~** — RESOLVED: Works out of the box.
  `MINIBOX_ADAPTER=native` manages real containers; multi-container
  pipelines issue `minibox::container::run` through the plugin hitting the
  same daemon socket. Validate with an e2e test.

## Summary

| Layer         | Component                           | Purpose                                              |
| ------------- | ----------------------------------- | ---------------------------------------------------- |
| Types         | `cruxx-types` (new, crux workspace) | Shared serializable types                            |
| Bridge        | `minibox-agent` (expanded)          | Pipeline runner, trace store, handler wiring         |
| Plugin: M->C  | `minibox-crux-plugin` (new binary)  | Minibox ops exposed to crux pipelines                |
| Plugin: C->M  | `handle_pipeline` (new handler)     | Crux pipelines as minibox daemon requests            |
| Coordination  | Single-container                    | Subprocess agents via crux combinators               |
| Coordination  | Multi-container                     | Cross-container via `minibox::container::*` handlers |
| Observability | `TraceStore`                        | File-based trace persistence                         |
| Observability | CLI/TUI                             | Trace listing and viewing                            |
| Workflows     | `.cruxx` pipelines                  | Primary agent work definition format                 |
| Portability   | Plugin protocol                     | Same pipelines run in minibox and maestro            |
