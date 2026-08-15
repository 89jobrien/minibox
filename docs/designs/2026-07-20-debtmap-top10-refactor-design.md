# Design: Debtmap Top-10 Bottom-Up Complexity Refactor

## Goal

Reduce real cyclomatic/cognitive complexity in `run_daemon()`, `exec::execute()`, and the
`minibox-core::domain` god-object, in bottom-up-by-risk order, without changing behavior,
public API compatibility (for downstream crates), or the wire protocol.

## Approved Approach

Approach A (bottom-up by risk) from brainstorm: extract-function refactors on the two
sequential/protocol-loop functions first (lowest to moderate risk), then split `domain.rs`
into focused submodules last (highest blast radius). `mbx/main.rs::run()` is confirmed a
debtmap false positive (CLI dispatch table) and is excluded from all phases.

## Crate Ownership

- **Phase 1a**: `miniboxd` — `crates/miniboxd/src/main.rs` only. No new crate.
- **Phase 1b**: `mbx` — `crates/mbx/src/commands/exec.rs` only. No new crate.
- **Phase 2**: `minibox-core` — `crates/minibox-core/src/domain.rs` split into
  `crates/minibox-core/src/domain/` submodules. No new crate; `minibox-core` already owns
  all these types.
- **Affected crates** (Phase 2 only, via re-exports — see Integration Points): `minibox`,
  `miniboxd`, `mbx`, `smolbox`, `minibox-testsuite`, and any adapter crate that does
  `use minibox_core::domain::*`.

## Phase 1a — `run_daemon()` extraction

### Public API

No new public API. All extracted helpers are private (`fn`, not `pub fn`) to
`crates/miniboxd/src/main.rs`, matching the existing visibility of `run_daemon()` and
`build_handler_deps()`.

```rust
fn init_daemon_tracing();
// existing #[cfg(feature = "otel")] / not(otel) split moves in as-is

fn select_and_validate_adapter_suite() -> Result<AdapterSuite>;
// wraps: adapter_from_env(), available_adapter_names() logging,
// #[cfg(linux)] warn_if_native_without_root(), root-check bail!,
// #[cfg(linux)] migrate_to_supervisor_cgroup()

fn prepare_daemon_directories(paths: &DaemonPaths) -> Result<()>;
// wraps: DirBuilder loop over images_dir/containers_dir/run_dir/run_containers_dir

async fn build_metrics_recorder() -> Result<Arc<dyn minibox_core::domain::MetricsRecorder>>;
// wraps: existing #[cfg(feature = "metrics")] / not(metrics) block, incl. run_metrics_server call

fn resolve_container_policy(config: &miniboxd::config::DaemonConfig) -> ContainerPolicy;
// wraps: env_policy = ContainerPolicy::from_env() + config-precedence merge

fn bind_and_secure_socket(sock_path: &Path) -> Result<UnixListener>;
// wraps: stale-socket removal, UnixListener::bind, MINIBOX_SOCKET_MODE/GROUP handling,
// chmod/chown, permission logging

fn install_shutdown_signal_handlers() -> Result<impl std::future::Future<Output = ()>>;
// wraps: sigterm/sigint signal() setup + the `shutdown` async block
```

`run_daemon()` itself becomes a short sequence of calls to the above plus the existing
`build_handler_deps()` and `minibox::daemon::server::run_server()` calls, preserving the
banner-comment ordering already in the file.

### Data Flow

1. Source: `main()` → `run_daemon(config)` (unchanged entry point).
2. Transform: each banner-commented block becomes one helper call; `run_daemon()` threads
   their outputs (`AdapterSuite`, `DaemonPaths`, `Arc<DaemonState>`, `ContainerPolicy`,
   `UnixListener`, shutdown future) into `build_handler_deps()` and `run_server()` exactly as
   today.
3. Sink: unchanged — `run_server()` runs the accept loop; cleanup/logging on return.

### Integration Points

- No signature change to `run_daemon()` or `build_handler_deps()`.
- No new feature flags; existing `#[cfg(feature = "otel")]`, `#[cfg(feature = "metrics")]`,
  `#[cfg(target_os = "linux")]` gates move into their respective helper bodies unchanged.
- No breaking changes — this is `miniboxd`'s binary crate, nothing downstream depends on it.

## Phase 1b — `exec::execute()` extraction

### Public API

`execute()`'s signature is unchanged (it's the CLI command entry point called from
`mbx/main.rs::run()`). New helpers are private to `crates/mbx/src/commands/exec.rs`:

```rust
#[cfg(unix)]
fn spawn_stdin_relay_task(socket_path: PathBuf, exec_id: String);
// wraps: the tokio::spawn stdin-read-and-SendInput loop

#[cfg(unix)]
fn send_initial_pty_size(socket_path: &Path, exec_id: &str) -> impl std::future::Future<Output = ()>;
// wraps: the initial terminal_size() + ResizePty send

#[cfg(unix)]
fn spawn_sigwinch_forwarder_task(socket_path: PathBuf, exec_id: String);
// wraps: the SignalKind::window_change() signal() + spawn loop, incl. the Err(e) eprintln arm

fn handle_container_output(stream: OutputStreamKind, data: &str) -> Result<()>;
// wraps: base64 decode + stdout/stderr write_all + flush match arms

fn handle_exec_started(
    exec_id: String,
    tty: bool,
    socket_path: &Path,
);
// wraps: the `if tty { ... }` block — calls spawn_stdin_relay_task,
// send_initial_pty_size, spawn_sigwinch_forwarder_task
```

`ContainerStopped` and `Error` and the `other` catch-all stay inline in `execute()`'s match
(they're one-liners that call `std::process::exit`, not worth extracting — extracting a
function that unconditionally exits the process has no reuse value and complicates the
`_raw_guard` drop ordering for no benefit).

### Data Flow

1. Source: `DaemonResponse` stream from `DaemonClient::call()` (unchanged).
2. Transform: `execute()`'s `while let Some(response) = stream.next().await` loop dispatches
   each variant to the helper above instead of an inline block; `ExecStarted` handling moves
   to `handle_exec_started`, `ContainerOutput` to `handle_container_output`.
3. Sink: unchanged — stdout/stderr writes, process exit codes.

### Integration Points

- No change to `DaemonRequest`/`DaemonResponse` (protocol untouched, per constraint).
- `_raw_guard` (the `RawModeGuard`) must stay owned by `execute()`'s stack frame — none of the
  extracted helpers take ownership of it; `handle_exec_started` only needs `tty`/`exec_id`,
  not the guard.
- No breaking changes — `commands::exec` is not part of any published/reused API surface.

## Phase 2 — `domain.rs` module split

### Context-map findings

`domain.rs` (3775 lines) contains two structurally unrelated clusters that happen to share a
file:

1. **Workflow engine** (lines ~77–453, ~2600–3775): `WorkflowStep`, `WorkflowDef`,
   `StepStatus`, `PhaseOutcome`, `StepRunner`/`StepRunnerRegistry` + 4 built-in runners,
   `StepCompletion`, `resolve_step_vars`, `propagate_output`, `steps_before`,
   `resume_workflow`, `evaluate_if_guard`, `resolve_expr`, `resolve_output_ref`. This is a
   step-execution DSL layered on top of the container ports below it — it depends on the
   runtime traits but the runtime traits do not depend on it.
2. **Container runtime ports** (lines ~454–2599, ~2942–2961): the hexagonal ports proper —
   image registry/pull/push/build/commit, filesystem/rootfs, resource limiting, container
   runtime spawn/exec, PTY allocation, VM checkpoint, backend capabilities, error/state/id
   types.

### Public API

No trait/type signatures change. This phase is a **file reorganization with `pub use`
re-exports**, not an API redesign — every existing `minibox_core::domain::<Name>` path must
keep resolving after the split, since 9+ downstream crates import from it directly (per
debtmap coupling: `Ce=9`).

New module tree under `crates/minibox-core/src/domain/`:

```rust
// domain/mod.rs
mod error;
mod ids;
mod state;
mod filesystem;
mod runtime;
mod image;
mod exec;
mod pty;
mod checkpoint;
mod capability;
mod metrics;
mod workflow;

pub use error::*;
pub use ids::*;
pub use state::*;
pub use filesystem::*;
pub use runtime::*;
pub use image::*;
pub use exec::*;
pub use pty::*;
pub use checkpoint::*;
pub use capability::*;
pub use metrics::*;
pub use workflow::*;
```

Module-to-content mapping (types/traits move verbatim, no renaming):

- `error.rs` — `DomainError`
- `ids.rs` — `ContainerId`, `SessionId`
- `state.rs` — `ContainerState`
- `filesystem.rs` — `RootfsSetup`, `ChildInit`, `FilesystemProvider`,
  `BackendRootfsMetadata`, `RootfsLayout`, `BindMount`
- `runtime.rs` — `ContainerRuntime`, `ResourceLimiter`, `ResourceConfig`,
  `RuntimeCapabilities`, `SpawnResult`, `HookSpec`, `ContainerHooks`,
  `ContainerSpawnConfig`, `AsAny`
- `image.rs` — `ImageRegistry`, `RegistryRouter`, `ImageLoader`, `ImageMetadata`,
  `LayerInfo`, `ImagePusher`, `RegistryCredentials`, `PushResult`, `PushProgress`,
  `ContainerCommitter`, `CommitConfig`, `ImageBuilder`, `BuildContext`, `BuildConfig`,
  `BuildProgress`
- `exec.rs` — `ExecRuntime`, `ExecSpec`, `ExecHandle`, `ProgressSink`
- `pty.rs` — `PtyAllocator`, `PtyConfig`, `PtyHandle`, `NullPtyAllocator`,
  `MockPtyAllocator`
- `checkpoint.rs` — `VmCheckpoint`, `SnapshotInfo`, `NoopVmCheckpoint`
- `capability.rs` — `BackendCapability`, `BackendCapabilitySet`
- `metrics.rs` — `MetricsRecorder`
- `workflow.rs` — `StepRetry`, `ExprVar`, `WorkflowStep`, `WorkflowDef`, `PhaseOutcome`,
  `StepStatus`, `StepCapability`, `StepContext`, `StepOutput`, `StepRunnerCapability`,
  `StepRunner`, `StepRunnerRegistry`, `ContainerRunStepRunner`, `ImagePullStepRunner`,
  `ExecStepRunner`, `OverlaySnapshotStepRunner`, `StepCompletion`,
  `determine_step_completion`, `ResolvedStep`, `resolve_step_vars`, `propagate_output`,
  `steps_before`, `resume_workflow`, `evaluate_if_guard`, `resolve_expr`,
  `resolve_output_ref`, `meets_min_priority`, `determine_final_phase`

That is 12 modules rather than debtmap's suggested 7 — driven by the actual cluster
boundaries found in this pass rather than an arbitrary target count; several of debtmap's
"7 modules" (e.g. lumping PTY with checkpoint, or capability with runtime) would recreate
mixed-responsibility files. `workflow.rs` will still be the largest (~1300 lines) but is
internally cohesive (one DSL), unlike the current file.

### Data Flow

1. Source: `crates/minibox-core/src/lib.rs:51` — `pub mod domain;` (unchanged line; now
   resolves to `domain/mod.rs` instead of `domain.rs`).
2. Transform: none — types move, no logic changes. `workflow.rs`'s built-in `StepRunner`
   impls (`ContainerRunStepRunner` etc.) reference `runtime`/`exec`/`image`/`filesystem`
   types via `super::` or `crate::domain::` paths — same crate, so this is not a new
   dependency edge, just an intra-crate `mod` boundary.
3. Sink: `pub use` re-exports in `domain/mod.rs` preserve every existing
   `minibox_core::domain::<Name>` import path used by downstream crates.

### Hexagonal Boundaries

- **Ports** (traits, unchanged): `ImageRegistry`, `RootfsSetup`, `ChildInit`,
  `ResourceLimiter`, `ContainerRuntime`, `ExecRuntime`, `ImagePusher`,
  `ContainerCommitter`, `ImageBuilder`, `PtyAllocator`, `VmCheckpoint`,
  `MetricsRecorder`, `StepRunner` — all stay `pub trait` in their new module.
- **Adapters** (impls, unchanged location): remain in `crates/minibox/src/adapters/**`,
  `crates/smolbox/**`, etc. — this phase touches zero adapter files.

### Integration Points

- `crates/minibox-core/src/lib.rs` line 51 (`pub mod domain;`) is unchanged — Rust resolves
  `mod domain;` to `domain/mod.rs` automatically once `domain.rs` is replaced by a directory.
- Every downstream `use minibox_core::domain::{X, Y};` continues to compile unchanged because
  of the blanket `pub use` re-exports in `domain/mod.rs`.
- `as_any!`/`adapt!` macro expansion sites (flagged in `CLAUDE.md` as needing the `AsAny`
  re-export) are covered — `AsAny` re-exports from `runtime.rs` via `domain/mod.rs` exactly
  as it did from the flat `domain.rs`.
- No `Cargo.toml` changes — same crate, same dependency graph.

## Out of Scope

- `crates/mbx/src/main.rs::run()` — confirmed debtmap false positive (CLI dispatch table),
  not refactored.
- The 5 "no callers detected" debtmap findings (binary entrypoints / match-dispatched
  handlers) — confirmed not dead code, no action.
- `agentbox/cmd/agentbox/main.go::runCouncil()` — Go file, separate subproject, not governed
  by minibox Rust conventions.
- Any change to `DaemonRequest`/`DaemonResponse` wire types.
- Any change to adapter implementations (`crates/minibox/src/adapters/**`).
- Renaming any existing public type, trait, or method during the Phase 2 split — this is a
  pure file/module reorganization, not an API redesign.
- Workflow-engine logic changes (Phase 2 relocates `workflow.rs` verbatim; a future design
  could evaluate whether the workflow DSL belongs in `minibox-core` at all, but that is a
  separate architectural question out of scope here).

## Risk

- [ ] Breaking API changes: **no** — all phases preserve every existing public path via
  re-exports (Phase 2) or unchanged signatures (Phase 1a/1b).
- [ ] New external dependency: **no**.
- [ ] Feature flag required: **no** — existing `#[cfg]` gates are preserved, not added to.

## Verification per phase

- Phase 1a: `cargo check -p miniboxd`, `cargo xtask verify` (fmt/clippy -D warnings), manual
  smoke: `miniboxd` still starts and binds its socket (existing e2e/daemon tests cover this).
- Phase 1b: `cargo check -p mbx`, `cargo xtask verify`, `cargo test -p mbx` (the existing
  `exec_sends_correct_request` test must still pass unmodified — it calls `execute()`
  end-to-end, so it validates the extraction without needing new tests).
- Phase 2: `cargo check --workspace` (catches any downstream import path that isn't covered
  by a re-export), `cargo xtask verify`, `cargo test --workspace` (domain.rs has existing
  unit tests colocated with the types being moved — they move with their module).
