# Design: Persistent Containers + Exec for smolvm

## Goal

Give the smolvm adapter suite (macOS default) persistent-container create and
exec-into-running-container support, at protocol parity with the native
Linux adapter, so `mbx create`/`mbx exec` and MCP `minibox_create`/
`minibox_exec` work the same way on macOS as they already do on Linux.

## Approved Approach

Full parity: implement real `ExecRuntime` support for smolvm using its native
persistent-machine lifecycle (`machine create`/`start`/`exec`/`stop`/
`delete`), reusing the existing adapter-agnostic protocol
(`DaemonRequest::Exec`) and CLI (`mbx exec`) unchanged. Rejected alternative:
MCP-only shortcut bypassing the daemon protocol — narrower, doesn't benefit
the CLI, and diverges from how every other capability in this codebase is
wired (protocol → handler → adapter, not MCP → adapter directly).

## Context Map

### Files to Modify

| File                                                                                        | Purpose                                                | Changes Needed                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ------------------------------------------------------------------------------------------- | ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/minibox/src/daemon/state.rs`                                                        | `ContainerRecord` struct                               | add `runtime_id: Option<String>` field (`#[serde(default)]`, additive)                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `crates/minibox/src/daemon/handler/run.rs`                                                  | Builds/stores `ContainerRecord`, calls `spawn_process` | thread `spawn_result.runtime_id` into the stored record (currently captured locally at handler/run.rs:929,1089 and passed only to `wait_for_exit`, never persisted)                                                                                                                                                                                                                                                                                                                                                          |
| `crates/minibox/src/adapters/smolvm.rs`                                                     | `SmolVmRuntime`, `SmolVmRegistry`                      | `spawn_process`: branch on `ephemeral` — ephemeral keeps today's `machine run` fast path unchanged; non-ephemeral switches to `machine create --name <id> --image <ref> --net` + `machine start`, returns `SpawnResult { runtime_id: Some(id), pid: 0, output_reader: None }`. New `SmolVmExecRuntime` type implementing `ExecRuntime::run_in_container` via `smolvm machine exec --name <runtime_id> -- <cmd>`. `wait_for_exit`/stop/rm-adjacent logic updated for the non-ephemeral case (see handler/lifecycle.rs below). |
| `crates/minibox/src/daemon/handler/exec.rs`                                                 | `handle_exec`                                          | no logic change expected — already generic over `deps.exec.exec_runtime`; confirm during implementation that container_id → runtime_id lookup path exists (via `state.get_container(id)` → `record.runtime_id`)                                                                                                                                                                                                                                                                                                              |
| `crates/minibox/src/daemon/handler/mod.rs` (or wherever `handle_stop`/`handle_remove` live) | container lifecycle handlers                           | for smolvm-backed persistent containers, `Stop`/`Remove` must invoke `smolvm machine stop`/`machine delete -f` against `runtime_id` instead of the current native-only assumption (process already exited synchronously)                                                                                                                                                                                                                                                                                                     |
| `crates/miniboxd/src/main.rs`                                                               | `build_smolvm_handler_dependencies` (line ~1038)       | change `exec.exec_runtime: None` → `Some(smolvm_exec_runtime(state))`. Single-suite change — `ExecDeps` already exists on `HandlerDependencies`, no schema change, no other suite touched.                                                                                                                                                                                                                                                                                                                                   |
| `crates/mcp/src/types.rs`                                                                   | MCP input/output types                                 | new `CreateContainerInput`/`CreateContainerOutput` (mirrors `RunContainerInput` minus `auto_remove`), `ExecContainerInput`/`ExecContainerOutput` (mirrors `ContainerIdInput` + command/env, and `RunContainerOutput` minus `container_id`)                                                                                                                                                                                                                                                                                   |
| `crates/mcp/src/tools/containers.rs`                                                        | tool implementations                                   | new `create`/`exec` functions following the `run`/`stop` pattern; gate both with `policy.validate_mutation(...)`                                                                                                                                                                                                                                                                                                                                                                                                             |
| `crates/mcp/src/server.rs`                                                                  | `#[tool]` router                                       | new `minibox_create`/`minibox_exec` methods                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `crates/mcp/README.md`                                                                      | permission table                                       | add `minibox_create`, `minibox_exec` rows under the `MINIBOX_MCP_ALLOW_MUTATION` gate                                                                                                                                                                                                                                                                                                                                                                                                                                        |

### Dependencies (may need updates)

| File                                        | Relationship                                                                                                                                                  |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/minibox-core/src/domain/exec.rs`    | defines `ExecSpec`/`ExecHandle`/`ExecRuntime` — consumed as-is, no changes expected (already VM-agnostic)                                                     |
| `crates/minibox-core/src/domain/runtime.rs` | defines `SpawnResult`/`ContainerSpawnConfig`/`RuntimeCapabilities` — no changes; `SpawnResult.runtime_id` already exists and is exactly the hook needed       |
| `crates/minibox-core/src/protocol.rs`       | `DaemonRequest::Exec`/`DaemonResponse::ExecStarted` — already generic, no changes                                                                             |
| `crates/mbx/src/commands/exec.rs`           | CLI `mbx exec` — already adapter-agnostic, no changes expected                                                                                                |
| `docs/core/ARCHITECTURE.mbx.md`             | Primary Ports table + Adapter Suite Coverage Matrix (lines ~77-181) — `ExecRuntime` row currently shows "native only"; must flip to include smolvm once wired |

### Test Coverage

| Test                                                                                                                 | Covers                                                                  | Gap                                                                                                                                                                                               |
| -------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/minibox/tests/conformance_exec.rs`                                                                           | `ExecRuntime` trait contract via `MockExecRuntime`                      | pattern to follow for a new smolvm-exec conformance file; does not itself need changes                                                                                                            |
| `crates/minibox/tests/smolvm_conformance_tests.rs`                                                                   | smolvm `BackendDescriptor` capabilities, registry/runtime unit behavior | needs new test(s) asserting exec capability once wired (currently no `BackendCapability::Exec`-style flag exists — may need one added to `minibox_core::domain::BackendCapability`)               |
| `crates/minibox/tests/daemon_handler_exec_tests.rs`                                                                  | `handle_exec` handler behavior                                          | needs a case covering VM-backed (non-native) `runtime_id` dispatch, currently only exercises native/mock paths per prior research                                                                 |
| `crates/minibox/tests/conformance_state.rs`, `daemon_state_persistence_tests.rs`, `daemon_state_repository_tests.rs` | `ContainerRecord`/state persistence                                     | needs a case covering the new `runtime_id` field round-trips through state save/load (JSON persistence)                                                                                           |
| _(none found)_                                                                                                       | smolvm persistent create/start/stop/delete lifecycle                    | **gap** — no existing test touches `machine create`/`machine exec` from Rust; new tests required, following `SmolVmExecutor` injection pattern already used for `load_image` tests in `smolvm.rs` |

### Reference Patterns

| File                                                                       | Pattern to Follow                                                                                                                                                                                                     |
| -------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/minibox/src/adapters/exec.rs` (`native_exec_runtime`)              | shape of an `ExecRuntime` impl + its constructor function — `SmolVmExecRuntime`/`smolvm_exec_runtime(state)` should mirror this signature style even though the transport differs (nsenter vs. `smolvm machine exec`) |
| `crates/minibox/src/adapters/smolvm.rs` (`SmolVmExecutor`/`with_executor`) | existing test-injection pattern for faking `smolvm` subprocess calls — reuse for the new exec runtime's tests                                                                                                         |
| `crates/mcp/src/tools/containers.rs` (`stop`/`rm` via `simple_id_request`) | template for `minibox_exec`'s policy-gated, id-addressed tool function                                                                                                                                                |
| `crates/mcp/src/types.rs` (`RunContainerInput`/`ContainerIdInput`)         | template for new `CreateContainerInput`/`ExecContainerInput`                                                                                                                                                          |

### Risk

- [ ] `ContainerRecord` is used by daemon state persistence (JSON on disk) — adding `runtime_id` as `#[serde(default)]` is additive/backward-compatible, matches existing convention for every other optional field on this struct (all use `#[serde(default)]`). No migration needed.
- [ ] No breaking API changes to `DaemonRequest`/`DaemonResponse`/CLI — this is purely additive at the protocol level (smolvm starts returning a working `ExecRuntime` where it previously returned `None`).
- [ ] New MCP tools change `crates/mcp`'s public tool surface — additive only (two new tools), existing tools/schemas unchanged. `server_lists_expected_tools` test needs updating to include them.
- [ ] Concurrency: smolvm does not document exec-call concurrency safety against the same named machine (confirmed via empirical testing — no locking/queueing mentioned). `SmolVmExecRuntime` must serialize `exec` calls per `runtime_id` (e.g. `Mutex<HashMap<String, ()>>` keyed by machine name) to avoid racing writes to the shared overlay.
- [ ] Reconciliation gap (flagged, not blocking): no daemon-restart recovery exists today for orphaned persistent smolvm machines, mirroring the existing native-only PID reconciliation gap noted in `progress.mbx.md`. Out of scope for this design; worth a follow-up backlog item.

## Crate Ownership

- **Owner crate**: `minibox` (`crates/minibox`) — owns all adapter implementations (`adapters::smolvm`), the daemon handler layer, and daemon state; this feature is entirely adapter + handler work, no new crate needed.
- **Affected crates**: `minibox-core` (no changes expected — existing `ExecRuntime`/`SpawnResult` types are sufficient), `miniboxd` (one-line wiring change), `mcp` (new tools).

## Public API

### Traits

No new traits. `ExecRuntime` (existing, `crates/minibox-core/src/domain/exec.rs`) is implemented by the new adapter type:

```rust
// existing, unchanged
pub trait ExecRuntime: AsAny + Send + Sync {
    async fn run_in_container(
        &self,
        container_id: &ContainerId,
        spec: ExecSpec,
        tx: DynProgressSink<crate::protocol::DaemonResponse>,
    ) -> anyhow::Result<ExecHandle>;
}
```

### Types

```rust
// crates/minibox/src/adapters/smolvm.rs

/// `SmolVM` implementation of `ExecRuntime`. Runs commands against an
/// already-running persistent smolvm machine, addressed by the container's
/// `runtime_id` (the smolvm machine name).
pub struct SmolVmExecRuntime {
    state: Arc<DaemonState>,
    /// Serializes concurrent `machine exec` calls per machine name — smolvm
    /// does not document concurrent-exec safety against a shared overlay.
    locks: Mutex<HashMap<String, Arc<TokioMutex<()>>>>,
}
```

```rust
// crates/minibox/src/daemon/state.rs — additive field on existing struct

pub struct ContainerRecord {
    // ...existing fields unchanged...
    /// Adapter-managed handle for containers whose lifecycle isn't a plain
    /// host PID (e.g. a persistent smolvm/krun VM name). `None` for native
    /// containers and for ephemeral VM-backed runs that already completed.
    #[serde(default)]
    pub runtime_id: Option<String>,
}
```

```rust
// crates/mcp/src/types.rs

pub struct CreateContainerInput {
    pub image: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub mounts: Vec<MountInput>,
    #[serde(default)]
    pub memory_limit_bytes: Option<u64>,
    #[serde(default)]
    pub cpu_weight: Option<u64>,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub privileged: Option<bool>,
}

pub struct CreateContainerOutput {
    pub container_id: String,
}

pub struct ExecContainerInput {
    pub id: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
}

pub struct ExecContainerOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub truncated: bool,
}
```

### Functions

```rust
// crates/minibox/src/adapters/smolvm.rs
pub fn smolvm_exec_runtime(state: Arc<DaemonState>) -> Arc<dyn minibox_core::domain::ExecRuntime>;

// crates/mcp/src/tools/containers.rs
pub async fn create(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: CreateContainerInput,
) -> Result<CreateContainerOutput>;

pub async fn exec(
    client: &MiniboxDaemonClient,
    policy: &AgentPolicy,
    input: ExecContainerInput,
) -> Result<ExecContainerOutput>;
```

## Data Flow

1. **Create**: `mbx create`/MCP `minibox_create` → `DaemonRequest::Run { ephemeral: false, auto_remove: false, .. }` (existing variant, no new protocol needed) → `handle_run` → `SmolVmRuntime::spawn_process` → `smolvm machine create` + `machine start` → `SpawnResult.runtime_id` → persisted onto `ContainerRecord.runtime_id`.
2. **Exec**: `mbx exec <id> <cmd>`/MCP `minibox_exec` → `DaemonRequest::Exec` (existing, unchanged) → `handle_exec` → looks up `ContainerRecord.runtime_id` for `<id>` → `deps.exec.exec_runtime` (now `Some(SmolVmExecRuntime)`) → `smolvm machine exec --name <runtime_id> -- <cmd>` → streamed `ContainerOutput`/`ContainerStopped` back to caller, same as native.
3. **Teardown**: `mbx stop`/`mbx rm` → existing handlers, extended to call `smolvm machine stop`/`machine delete -f` against `runtime_id` when the container is smolvm-backed and non-ephemeral.

## Hexagonal Boundaries

- **Port** (trait): `ExecRuntime` in `minibox_core::domain::exec` (existing, unchanged)
- **Adapter** (impl): `SmolVmExecRuntime` in `minibox::adapters::smolvm` (new)
- **Port** (trait): `ContainerRuntime` in `minibox_core::domain::runtime` (existing, unchanged signature — `spawn_process` behavior branches internally on `ephemeral`, no trait change)
- **Adapter** (impl): `SmolVmRuntime` in `minibox::adapters::smolvm` (existing, extended)

## Out of Scope

- krun parity (explicitly deferred as a stretch phase; krun has zero existing exec scaffolding, clean slate, separate design if pursued).
- Daemon-restart reconciliation for orphaned persistent smolvm machines (flagged risk, follow-up item).
- Any change to `DaemonRequest`/`DaemonResponse` wire format — none needed.
- Interactive TTY/PTY support for smolvm exec (native supports `-it` via `nsenter`+PTY; smolvm's `machine exec -i -t` exists but wiring PTY streaming through `ExecRuntime` for a VM transport is a separate scope decision, not required for the base create+exec use case).

## Risk

- [ ] Breaking API changes: **no** — all changes are additive (`#[serde(default)]` field, new MCP tools, new adapter type, one wiring line changed from `None` to `Some`).
- [ ] New external dependency: **no** — reuses the already-required `smolvm` CLI binary.
- [ ] Feature flag required: **no** — follows the existing `MINIBOX_ADAPTER=smolvm` suite selection; gated at the MCP layer via existing `MINIBOX_MCP_ALLOW_MUTATION`.
