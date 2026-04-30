# Plan: Maestro–Minibox Adapter Wiring

**Date:** 2026-04-30
**Status:** pending
**Scope:** `minibox` workspace only — no changes to `maestro` repo

## Context

"Maestro integration" in the minibox backlog (todos tagged `maestro, wiring`) means wiring
the commit/build/push adapter chain so that macOS-local dogfooding and future CI use cases
work end-to-end. This is not a change to the Toptal Maestro repo — it is minibox becoming
a capable container backend for the same workflows Maestro's GKE pipeline runs.

**Default adapter policy (as of 2026-04-30):** `MINIBOX_ADAPTER` defaults to `smolvm` with
automatic fallback to `krun` when the `smolvm` binary is absent from PATH. This is enforced
in `adapter_registry::adapter_from_env()`. All commit/build/push wiring in this plan targets
the smolvm and krun adapter suites first, with native Linux as a secondary target.

The current state:

- `OverlayCommitAdapter` (`adapters/commit.rs`) — implemented, tested, native Linux only
- `OciPushAdapter` (`adapters/push.rs`) — implemented, native Linux only (re-tars extracted
  dirs)
- `ColimaImagePusher` (`adapters/colima_push.rs`) — implemented, exports Docker archive +
  loads into Colima VM + `nerdctl push`, but `CommitConfig` metadata (rootfs path, source
  image) is not persisted into `ContainerRecord`
- `BackendDescriptor` / `BackendCapabilitySet` (`adapters/conformance.rs`) — scaffolded with
  `Commit`, `BuildFromContext`, `PushToRegistry` capability flags
- `DaemonState::ContainerRecord` — does not carry rootfs metadata
- `adapter_from_env()` — auto-selects smolvm, falls back to krun (implemented 2026-04-30)

The blocking gaps are:

1. `ContainerRecord` missing rootfs metadata (source image, layers dir, upper dir path)
2. No `ContainerCommitter` for Colima (commit from Colima VM container upperdir)
3. `handle_commit` / `handle_push` handlers exist in handler.rs but are not wired into all
   adapter suites in `miniboxd/src/main.rs`
4. `fork()` UB in `exec.rs` — called inside an active Tokio runtime (GH issue)
5. No backend-agnostic build conformance test suite for `ImageBuilder`
6. No end-to-end dogfood test (macOS: pull → run → commit → push cycle)

---

## Tasks (ordered by dependency)

### Task 1 — Define rootfs metadata contract

**File:** `crates/minibox/src/daemon/state.rs`

Add to `ContainerRecord`:

```rust
/// Path to the overlay upper dir (None for non-native adapters).
pub rootfs_upper_dir: Option<PathBuf>,
/// Path to the merged overlay mountpoint.
pub rootfs_merged_dir: Option<PathBuf>,
/// Source image reference used to create the container.
pub source_image_ref: Option<String>,
/// Image layers digest list (for re-commit chains).
pub source_layer_digests: Vec<String>,
```

All fields `#[serde(default)]` for backward-compat with persisted JSON. Update
`ContainerRecord::new()` and all construction sites (native, gke, colima suites in
`miniboxd/src/main.rs`).

**Tests:** snapshot test in `minibox-core/tests/` confirming `ContainerRecord` round-trips
with new fields present and absent.

---

### Task 2 — Fix fork() UB in exec.rs

**File:** `crates/minibox/src/adapters/exec.rs`

`NativeExecRuntime::run_in_container` calls `fork()` inline in the Tokio async context.
This is POSIX UB under concurrent Tokio load (fork inside multithreaded process corrupts
lock state in child).

Fix: wrap the `fork`/`setns`/`execvp` sequence in `tokio::task::spawn_blocking`. The
blocking closure takes ownership of the raw fd handles. The outer async fn awaits the
`JoinHandle` and maps errors.

**Constraint:** `spawn_blocking` closure cannot hold `mpsc::Sender` across the boundary
without a `move` capture. Use a `oneshot` channel to return the child PID back to the caller,
then drive stdin/resize relay in a separate `tokio::spawn` task.

**Tests:** unit test asserting `run_in_container` does not block the Tokio runtime (mock
state returning a fixed PID, no actual fork).

---

### Task 3 — Colima filesystem metadata for commit

**File:** `crates/minibox/src/adapters/colima.rs` + new
`crates/minibox/src/adapters/colima_commit.rs`

Colima containers run inside the Lima VM. The container upperdir is not directly accessible
from the macOS host. Options:

**Chosen approach:** `limactl shell` → `nerdctl commit` inside the VM, then export the
resulting image as a Docker archive via `nerdctl save` → load into the local `ImageStore`
via the host-side image loading path.

New `ColimaContainerCommitter`:

```rust
pub struct ColimaContainerCommitter {
    executor: LimaExecutor,    // Arc<dyn Fn(&[&str]) -> Result<String>>
    image_store: Arc<ImageStore>,
    image_loader: DynImageLoader,
    export_dir: PathBuf,
}
```

`ContainerCommitter::commit()` implementation:
1. Run `nerdctl commit <container_id> <target_ref>` inside VM via executor
2. Run `nerdctl save <target_ref> -o <export_path>` inside VM via executor
3. Copy the archive from the Lima shared mount to local
4. Call `image_loader.load_image()` to import into local `ImageStore`
5. Return `ImageMetadata` from the loaded manifest

**Tests:** unit test with a mock `LimaExecutor` that records commands; assert correct
command sequence for `commit` + `save`.

---

### Task 4 — Wire commit/push into all adapter suites

**File:** `crates/miniboxd/src/main.rs`

Currently `HandlerDependencies` wires `committer` and `pusher` only for the `native` suite.
Wire them for `colima` and `gke` suites:

- `colima`: `ColimaContainerCommitter` (Task 3) + `ColimaImagePusher` (already exists)
- `gke`: `NoopContainerCommitter` (return `Err("commit not supported on GKE adapter")`)
  + `NoopImagePusher` (same pattern) — explicit errors are better than missing arms

**Note:** `HandlerDependencies` fields are Linux-gated (`#[cfg(target_os = "linux")]`).
Verify colima suite compiles on macOS with `cargo check -p miniboxd`.

---

### Task 5 — Backend-agnostic build conformance tests

**File:** `crates/minibox-core/tests/conformance_build.rs` (new)

Using the existing `BackendDescriptor` / `BuildContextFixture` / `WritableUpperDirFixture`
from `conformance.rs`, add tests:

- `commit_roundtrip` — commit upper dir → load metadata → assert layer digest present
- `push_rejects_unauthenticated` — push to localhost:5000 without credentials → assert err
- `build_from_context_produces_image` — only runs if `BuildFromContext` capability declared

Wire into `cargo xtask test-conformance` via the existing auto-discovery gate.

---

### Task 6 — macOS dogfood e2e test

**File:** `crates/miniboxd/tests/mac_dogfood_e2e.rs` (new, `#[cfg(target_os = "macos")]`)

Requires `MINIBOX_ADAPTER=colima` and a running Colima instance. Gated by
`require_capability!(ColimaDaemon)` from preflight.

Steps:
1. Pull `alpine:latest` via daemon client
2. Run ephemeral container, confirm exit 0
3. Run non-ephemeral container, get container ID
4. Commit container to `localhost:5001/test/dogfood:e2e`
5. Push to local registry (started by the test via `limactl` or skipped if unavailable)
6. Confirm push result has a valid digest

---

## Acceptance Criteria

- [ ] `ContainerRecord` persists rootfs metadata; existing state files load without error
- [ ] `exec.rs` fork is inside `spawn_blocking`; no clippy warnings
- [ ] `ColimaContainerCommitter` has unit tests passing on macOS
- [ ] `colima` and `gke` adapter suites in `miniboxd/src/main.rs` compile with commit/push wired
- [ ] `cargo xtask test-conformance` includes build conformance tests; all pass or skip cleanly
- [ ] Dogfood e2e test passes on a macOS host with Colima running
- [ ] `cargo xtask pre-commit` clean on macOS after all changes

## Out of Scope

- Toptal Maestro repo changes
- Windows (winbox) commit/push — tracked separately under `Winbox Hyper-V WSL2`
- VZ adapter commit/push — blocked by Apple bug GH #61
