---
status: future
note: Implementation spec for {App}-orch. Not started. See {App}-orch-design.md for full type definitions and architecture.
---

> **ARCHIVED** — This document is not authoritative. See the current docs in the repo root.

# {App}-orch Implementation Handoff

## What This Is

Implementation spec for `{App}-orch`, a self-evolving container agent orchestrator.
Read `docs/{App}-orch-design.md` first for the full type definitions and architecture.

## Implementation Order

Build bottom-up. Each step produces compiling, testable code before moving to the next.

### Step 1: Scaffold

Create the crate skeleton and register it in the workspace.

**Files to create:**

- `crates/{App}-orch/Cargo.toml`
- `crates/{App}-orch/src/lib.rs`
- `crates/{App}-orch/src/main.rs` (stub)

**Files to modify:**

- `Cargo.toml` (root) -- add `"crates/{App}-orch"` to workspace members, add `toml = "0.8"` and
  `rusqlite = { version = "0.32", features = ["bundled"] }` to `[workspace.dependencies]`

**Verify:** `cargo check -p {App}-orch`

### Step 2: Domain Layer

Write all traits, types, and errors. This is the foundation everything else depends on.

**Files to create:**

- `crates/{App}-orch/src/domain.rs` -- all 5 trait definitions, all domain types (see design doc),
  `OrchestratorError` enum, `AsAny` trait (local copy), `Dyn*` type aliases
- `crates/{App}-orch/src/domain/safety.rs` -- `SafetyPolicy` struct, `HarnessDiff::validate()`,
  `HarnessDiff::requires_approval()`, `HarnessDiff::apply()`

**Key decisions:**

- Own `AsAny` trait -- do NOT depend on `{App}-macros`. The `adapt!()` macro resolves
  `crate::domain::AsAny` at the call site, which would be {App}-orch's domain, but it's cleaner
  to just hand-write the 5-line impl for each adapter.
- `HarnessProfile` derives `Serialize, Deserialize` (TOML-compatible via serde).
- `CompletionRequest`, `ApprovalRequest`, `ContainerHandle` are NOT serializable -- they're
  in-process only.

**Verify:** `cargo check -p {App}-orch`

### Step 3: Mock Adapters

Enables TDD for all subsequent steps.

**Files to create:**

- `crates/{App}-orch/src/adapters/mod.rs` -- re-exports
- `crates/{App}-orch/src/adapters/mocks.rs` -- `MockDaemonClient`, `MockModelClient`,
  `MockProfileStore`, `MockTelemetryStore`, `MockApprovalGate`

**Pattern to follow:** `crates/mbx/src/adapters/mocks.rs`

- `Arc<Mutex<InternalState>>` for interior mutability
- Builder methods for configuring behavior
- Call-count accessors for assertions
- Hand-written `impl AsAny for MockFoo`

**Verify:** `cargo test -p {App}-orch`

### Step 4: Inner Loop

Core business logic -- run containers, collect metrics, ask model for critique, retry.

**Files to create:**

- `crates/{App}-orch/src/inner_loop.rs`

**Struct:**

```rust
pub struct TaskRunner {
    daemon: DynDaemonClient,
    model: DynModelClient,
    telemetry: DynTelemetryStore,
    profiles: DynProfileStore,
}
```

**Methods:**

- `pub async fn run_once(profile, spec, memory) -> Result<JobRun>` -- single attempt
- `pub async fn run_with_budget(profile, spec, max_attempts, memory) -> Result<Vec<JobRun>>` --
  retry loop with model critique between attempts

**Tests:** All use mocks. Test cases:

- Success on first attempt
- Failure then success on retry
- Budget exhaustion (max attempts reached)
- OOM handling (model suggests memory bump)
- Timeout handling

**Verify:** `cargo test -p {App}-orch -- inner_loop`

### Step 5: Outer Loop

Meta-agent proposes profile changes, canary eval, promote-or-discard.

**Files to create:**

- `crates/{App}-orch/src/outer_loop.rs`

**Struct:**

```rust
pub struct HarnessEvolver {
    daemon: DynDaemonClient,
    model: DynModelClient,
    telemetry: DynTelemetryStore,
    profiles: DynProfileStore,
    approval: DynApprovalGate,
    policy: SafetyPolicy,
}
```

**Methods:**

- `pub async fn evolve_once(eval_suite) -> Result<EvolutionOutcome>`

**Tests:** All use mocks. Test cases:

- Successful evolution (candidate beats baseline)
- Candidate worse than baseline (discarded)
- Safety violation (diff exceeds hard cap)
- Approval gate blocks network escalation
- Approval gate approves syscall change

**Verify:** `cargo test -p {App}-orch -- outer_loop`

### Step 6: SocketDaemonClient Adapter

**Files to create:**

- `crates/{App}-orch/src/adapters/daemon_client.rs`

**Reuse:** `{App}::protocol::{DaemonRequest, DaemonResponse, ContainerInfo, encode_request,
decode_response}` and the `send_request` pattern from `crates/{App}-cli/src/commands/mod.rs:28`.

**Key behavior:** `wait_container` polls `list_containers` every 500 ms until state is
`Stopped`/`Failed` or timeout expires.

**Tests:** Unit tests with a mock Unix socket (or just validate request serialization).

### Step 7: TomlProfileStore Adapter

**Layout:**

```text
profiles/
  active.txt          # Contains the ID of the active profile
  default-v1.toml     # Profile files
  evolved-v2.toml
```

**Tests:** Use `tempfile::TempDir`. Save/load/list roundtrip, active tracking.

### Step 8: SqliteTelemetryStore Adapter

**Schema:** See design doc. Auto-creates tables on first use.

**`aggregate_metrics` impl:** SQL aggregation for counts. Percentiles computed in Rust
(sort durations, index at p50/p95 positions).

**Tests:** Use `tempfile::NamedTempFile` for DB path. Record, query, aggregate roundtrip.

### Step 9: Model Client Adapters

**Anthropic adapter:**

- POST `https://api.anthropic.com/v1/messages`
- Headers: `x-api-key`, `anthropic-version: 2023-06-01`, `content-type: application/json`
- Env: `ANTHROPIC_API_KEY`, optional `ANTHROPIC_MODEL` (default `claude-sonnet-4-20250514`)

**OpenAI adapter:**

- POST `{base_url}/v1/chat/completions`
- Headers: `Authorization: Bearer {key}`, `content-type: application/json`
- Env: `OPENAI_API_KEY`, `OPENAI_BASE_URL` (default `https://api.openai.com`),
  optional `OPENAI_MODEL` (default `gpt-4o-mini`)

**Tests:** Mock HTTP responses or just test request serialization. Real API calls in
integration tests only.

### Step 10: TerminalApprovalGate Adapter

**Files to create:**

- `crates/{App}-orch/src/adapters/terminal_approval.rs`

Prints proposed change to stdout, reads `y/n` from stdin via `tokio::task::spawn_blocking`.

### Step 11: Composition Root

**Files to modify:**

- `crates/{App}-orch/src/main.rs` -- clap CLI, adapter wiring

**Subcommands:** `run`, `evolve`, `init`, `stats`, `profile`

**Adapter selection:** `{App}_ORCH_MODEL` env var: `anthropic` (default) or `openai`.

### Step 12: Verification

```bash
cargo check -p {App}-orch
cargo clippy -p {App}-orch -- -D warnings
cargo fmt --all --check
cargo test -p {App}-orch
cargo build -p {App}-orch --release
```

## Key Files to Reuse

| Existing file                             | What to reuse                                                     |
| ----------------------------------------- | ----------------------------------------------------------------- |
| `crates/mbx/src/protocol.rs`              | `DaemonRequest`, `DaemonResponse`, `ContainerInfo`, encode/decode |
| `crates/mbx/src/domain.rs`                | `ResourceConfig` (map to/from `ResourceHints`)                    |
| `crates/{App}-cli/src/commands/mod.rs:28` | `send_request()` Unix socket pattern                              |
| `crates/mbx/src/adapters/mocks.rs`        | Mock adapter pattern (`Arc<Mutex<State>>`, builders)              |
| `crates/{App}d/src/handler.rs`            | `HandlerDependencies` pattern for DI structs                      |

## Conventions to Follow

- `#[async_trait]` for async trait definitions
- `Arc<dyn Trait>` for dependency injection (not generics -- keeps API surface clean)
- `thiserror` for domain errors, `anyhow` for adapter errors mapped via `#[from]`
- `tracing` for structured logging (not `println!`)
- `#[cfg(test)] mod tests` at bottom of each file
- All domain types derive `Debug, Clone`; serializable ones also derive `Serialize, Deserialize`
- No `unwrap()` in production code; `expect()` only for invariants with clear messages

## What NOT to Do

- Do NOT add `{App}-macros` as a dependency. Hand-write `impl AsAny` for each adapter.
- Do NOT add Linux-only deps (`nix`, `libc`). This crate must compile on macOS.
- Do NOT add `compile_error!()` platform guards. Unlike `{App}d`/`{App}-cli`, this crate
  is cross-platform.
- Do NOT put business logic in adapters. Adapters only translate between domain and infra.
- Do NOT make the model client adapters call real APIs in unit tests. Use mocks.
