# macbox + daemonbox Design Spec

**Date:** 2026-03-17
**Status:** Draft
**Scope:** Two new workspace crates — `daemonbox` (shared daemon application layer) and `macbox` (macOS orchestration library) — plus a new `macboxd` binary.

---

## Problem

`miniboxd` has a `compile_error!("miniboxd requires Linux")` guard in `main.rs`. The Colima adapters in `minibox-lib` are fully implemented and tested, but they are unreachable from macOS because there is no macOS-native daemon binary to wire them into. Running the daemon on macOS currently requires SSHing into a Colima VM to compile and execute a Linux binary — fragile, slow, and not the right UX.

Separately, the daemon application logic (`handler.rs`, `state.rs`, `server.rs`) lives inside the `miniboxd` binary crate, making it impossible for a second daemon binary to share it without duplication.

---

## Goals

1. Enable `macboxd` to compile and run natively on macOS.
2. Extract shared daemon logic into `daemonbox` so both `miniboxd` and `macboxd` depend on it — no duplication.
3. Give `macbox` ownership of macOS-specific orchestration (Colima preflight, VM lifecycle, path conventions, adapter wiring).
4. Preserve and reinforce the hexagonal architecture: `daemonbox` depends only on domain port traits; adapter wiring stays in composition roots.

---

## Non-Goals

- No new container features.
- No TTY, exec, networking, or log capture.
- No changes to the Linux adapter suites (native, GKE).
- `macboxd` does not need a systemd unit (launchd is out of scope for this iteration).

---

## Architecture

### Hexagonal Layer Mapping

```
┌─────────────────────────────────────────────────────┐
│              Driving Side (composition roots)        │
│         miniboxd/main.rs   macboxd/main.rs           │
├─────────────────────────────────────────────────────┤
│              Application Core                        │
│                  daemonbox                           │
│        handler.rs  state.rs  server.rs               │
│     (depends only on domain port traits)             │
├─────────────────────────────────────────────────────┤
│              Domain Ports (traits)                   │
│           minibox-lib/src/domain.rs                  │
├─────────────────────────────────────────────────────┤
│              Driven Adapters                         │
│  minibox-lib/src/adapters/ (Linux, GKE, Colima)      │
│  macbox (VM lifecycle, macOS paths, adapter helpers) │
└─────────────────────────────────────────────────────┘
```

`daemonbox` has zero knowledge of concrete adapters. The composition roots (`main.rs` in each daemon binary) are the only place where adapters are wired to the application core.

---

## New Crates

### `crates/daemonbox` — Shared Daemon Application Layer

**Purpose:** All daemon logic that is not platform-specific.

**Contents (moved from `miniboxd/src/`):**
- `handler.rs` — request handlers for each `DaemonRequest` variant
- `state.rs` — in-memory `DaemonState` (container records, spawn semaphore)
- `server.rs` — Unix socket listener, `SO_PEERCRED` auth, per-connection task

**Dependencies:** `minibox-lib`, `tokio`, `serde_json`, `tracing`, `nix`, `uuid`, `chrono`, `anyhow`

**What it does NOT contain:** any `#[cfg(target_os = "linux")]` compile guards; any concrete adapter types; any path defaults.

**After extraction, `miniboxd/src/` contains only `main.rs`** — adapter wiring, path resolution, and daemon startup.

---

### `crates/macbox` — macOS Orchestration Library

**Purpose:** macOS-specific infrastructure that `macboxd/main.rs` uses to build its adapter set and verify the environment before starting.

**Public API:**

```rust
/// Check Colima installation and VM state.
pub fn preflight() -> Result<ColimaStatus, MacboxError>;

/// Ensure Colima VM is running; starts it if stopped.
pub async fn ensure_vm_running() -> Result<(), MacboxError>;

/// Build HandlerDependencies wired with the Colima adapter suite.
pub fn colima_deps(containers_base: PathBuf, run_containers_base: PathBuf)
    -> HandlerDependencies;

/// macOS default paths.
pub mod paths {
    pub fn data_dir() -> PathBuf;   // ~/Library/Application Support/macbox
    pub fn run_dir() -> PathBuf;    // /tmp/macbox
    pub fn socket_path() -> PathBuf; // /tmp/macbox/macboxd.sock
}
```

**`ColimaStatus`:**
```rust
pub enum ColimaStatus {
    Running,
    Stopped,
    NotInstalled,
}
```

**Dependencies:** `minibox-lib`, `macros`, `anyhow`, `thiserror`, `tokio`, `tracing`

**Platform gate:** `#[cfg(target_os = "macos")]` on the crate — compile error on non-macOS.

---

### `crates/macboxd` — macOS Daemon Binary

**Purpose:** macOS-native daemon binary. Structurally identical to `miniboxd/main.rs` but uses `macbox` for adapter wiring and path conventions.

**`main.rs` responsibilities:**
1. Init tracing.
2. Call `macbox::preflight()` — fail fast if Colima not installed.
3. Call `macbox::ensure_vm_running()` — start VM if stopped.
4. Resolve paths via `macbox::paths::*` (overridable via env vars for tests).
5. Create directories, remove stale socket.
6. Build `HandlerDependencies` via `macbox::colima_deps(...)`.
7. Start `daemonbox` server loop.
8. Handle SIGTERM / SIGINT for clean shutdown.

**No `#[cfg(target_os = "linux")]` guards anywhere in this crate.**

---

## Migration: `miniboxd`

`miniboxd/src/` before:
```
main.rs
handler.rs
state.rs
server.rs
```

`miniboxd/src/` after:
```
main.rs   ← only file; imports handler/state/server from daemonbox
```

`miniboxd/Cargo.toml` gains `daemonbox = { workspace = true }`.

The `compile_error!("miniboxd requires Linux")` guard stays in `miniboxd/main.rs` — Linux-only is still correct for the native adapter suite.

---

## Dependency Graph

```
miniboxd  ──► daemonbox ──► minibox-lib
macboxd   ──► daemonbox ──► minibox-lib
macboxd   ──► macbox    ──► minibox-lib
minibox-cli ──► minibox-lib  (unchanged)
```

---

## File Map

| Action | Path |
|--------|------|
| Create | `crates/daemonbox/Cargo.toml` |
| Create | `crates/daemonbox/src/lib.rs` |
| Move   | `crates/miniboxd/src/handler.rs` → `crates/daemonbox/src/handler.rs` |
| Move   | `crates/miniboxd/src/state.rs` → `crates/daemonbox/src/state.rs` |
| Move   | `crates/miniboxd/src/server.rs` → `crates/daemonbox/src/server.rs` |
| Modify | `crates/miniboxd/src/main.rs` — replace module declarations with `use daemonbox::*` |
| Modify | `crates/miniboxd/Cargo.toml` — add `daemonbox` dependency |
| Modify | `Cargo.toml` — add `daemonbox`, `macbox`, `macboxd` to workspace members |
| Create | `crates/macbox/Cargo.toml` |
| Create | `crates/macbox/src/lib.rs` |
| Create | `crates/macbox/src/preflight.rs` |
| Create | `crates/macbox/src/paths.rs` |
| Create | `crates/macboxd/Cargo.toml` |
| Create | `crates/macboxd/src/main.rs` |

---

## Error Handling

`macbox` defines `MacboxError`:

```rust
#[derive(thiserror::Error, Debug)]
pub enum MacboxError {
    #[error("Colima is not installed — run `brew install colima`")]
    ColimaNotInstalled,
    #[error("Colima VM failed to start: {0}")]
    VmStartFailed(String),
    #[error("preflight check failed: {0}")]
    PreflightFailed(String),
}
```

`macboxd` exits with a clear message on any `MacboxError` before binding the socket.

---

## Testing

- `daemonbox` unit tests: move existing `miniboxd` handler/state tests into `daemonbox/src/`.
- `macbox` unit tests: mock `ColimaStatus` variants, test path resolution, test `preflight()` with injected executor (same pattern as `ColimaRegistry`'s `LimaExecutor`).
- `macboxd` integration: manual smoke test (`macboxd &` → `minibox pull alpine` → `minibox run alpine -- echo hi`). No automated E2E for macOS yet.
- Existing `miniboxd` tests must pass unchanged after the move.

---

## Success Criteria

1. `cargo build -p macboxd` succeeds on macOS.
2. `cargo build --workspace` succeeds on Linux (no regressions).
3. All existing lib and handler tests pass.
4. `macboxd` starts, calls preflight, and accepts connections on macOS with Colima running.
5. `minibox pull alpine && minibox run alpine -- /bin/echo hello` succeeds end-to-end via `macboxd`.
