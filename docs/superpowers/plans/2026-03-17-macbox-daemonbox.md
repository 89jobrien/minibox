# macbox + daemonbox Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract shared daemon logic into `daemonbox`, create `macbox` (macOS orchestration lib), and ship `macboxd` (macOS-native daemon binary wired to Colima adapters).

**Architecture:** `daemonbox` holds `handler.rs`, `state.rs`, `server.rs` (moved from `miniboxd`) — zero infrastructure knowledge, depends only on domain port traits from `minibox-lib`. `macbox` owns macOS-specific concerns: Colima preflight, VM lifecycle, path conventions, and adapter wiring. `macboxd` is a thin `main.rs` that sequences startup and delegates everything. `miniboxd/src/lib.rs` becomes a transparent re-export shim so existing tests require no changes.

**Tech Stack:** Rust workspace, `minibox-lib` (Colima adapters, domain traits), `nix` (POSIX signal/wait), `tokio` (async), `thiserror`/`anyhow`.

**Spec:** `docs/superpowers/specs/2026-03-17-macbox-daemonbox-design.md`

---

## File Map

### New files
| File | Responsibility |
|------|---------------|
| `crates/daemonbox/Cargo.toml` | Crate manifest |
| `crates/daemonbox/src/lib.rs` | Public module exports (`pub mod handler; pub mod state; pub mod server;`) |
| `crates/daemonbox/src/handler.rs` | Moved from `miniboxd` — request handlers, `HandlerDependencies` |
| `crates/daemonbox/src/state.rs` | Moved from `miniboxd` — `DaemonState`, `ContainerRecord` |
| `crates/daemonbox/src/server.rs` | Moved from `miniboxd` — Unix socket connection handler |
| `crates/macbox/Cargo.toml` | Crate manifest |
| `crates/macbox/src/lib.rs` | Public API: `preflight`, `ensure_vm_running`, `colima_deps`, re-exports |
| `crates/macbox/src/preflight.rs` | `ColimaStatus`, `MacboxError`, `preflight()`, `ensure_vm_running()` |
| `crates/macbox/src/paths.rs` | macOS default path functions |
| `crates/macboxd/Cargo.toml` | Crate manifest |
| `crates/macboxd/src/main.rs` | macOS daemon entry point |

### Modified files
| File | Change |
|------|--------|
| `Cargo.toml` | Add `daemonbox`, `macbox`, `macboxd` to workspace members + workspace.dependencies |
| `crates/miniboxd/Cargo.toml` | Add `daemonbox = { workspace = true }` |
| `crates/miniboxd/src/lib.rs` | Replace module declarations with `pub use daemonbox::*` re-exports |
| `crates/miniboxd/src/main.rs` | No change needed — imports via `miniboxd::handler/state` still resolve through lib.rs shim |

---

## Task 1: Create `daemonbox` crate and migrate miniboxd

**Files:**
- Create: `crates/daemonbox/Cargo.toml`
- Create: `crates/daemonbox/src/lib.rs`
- Create: `crates/daemonbox/src/handler.rs` (copy from miniboxd)
- Create: `crates/daemonbox/src/state.rs` (copy from miniboxd)
- Create: `crates/daemonbox/src/server.rs` (copy from miniboxd)
- Modify: `Cargo.toml` (root)
- Modify: `crates/miniboxd/Cargo.toml`
- Modify: `crates/miniboxd/src/lib.rs`

- [ ] **Step 1: Add `daemonbox` to workspace**

In root `Cargo.toml`, add to `[workspace]` members:
```toml
"crates/daemonbox",
```

Add to `[workspace.dependencies]`:
```toml
daemonbox = { path = "crates/daemonbox" }
```

- [ ] **Step 2: Create `crates/daemonbox/Cargo.toml`**

```toml
[package]
name = "daemonbox"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
minibox-lib = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
nix = { workspace = true }
libc = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
minibox-lib = { workspace = true, features = [] }
```

- [ ] **Step 3: Copy `handler.rs`, `state.rs`, `server.rs` into `crates/daemonbox/src/`**

Copy the three files verbatim. The `use crate::state::...` and `use crate::handler::...` references inside them are all intra-crate and will resolve correctly in the new crate.

Verify: no `use miniboxd::` references exist in those three files.

- [ ] **Step 4: Create `crates/daemonbox/src/lib.rs`**

```rust
//! Shared daemon application layer.
//!
//! Contains the request handlers, in-memory state, and Unix socket server
//! used by both `miniboxd` (Linux) and `macboxd` (macOS).

pub mod handler;
pub mod server;
pub mod state;
```

- [ ] **Step 5: Add `daemonbox` dependency to `miniboxd`**

In `crates/miniboxd/Cargo.toml`, add:
```toml
daemonbox = { workspace = true }
```

- [ ] **Step 6: Replace `miniboxd/src/lib.rs` with re-export shim**

```rust
//! miniboxd library — re-exports from daemonbox for backward compatibility.
//!
//! These re-exports exist so that integration tests importing
//! `miniboxd::handler`, `miniboxd::state`, or `miniboxd::server` continue
//! to compile without changes after the move to `daemonbox`.

#[doc(hidden)]
pub use daemonbox::handler;
#[doc(hidden)]
pub use daemonbox::server;
#[doc(hidden)]
pub use daemonbox::state;
```

- [ ] **Step 7: Verify the relevant crates compile**

```bash
cargo check -p daemonbox -p miniboxd -p minibox-lib
```

Expected: `Finished` with no errors. If there are import errors, check that no file in `daemonbox/src/` still references `miniboxd::` — they should only use `crate::`.

Note: use per-crate `-p` flags throughout this plan rather than `--workspace` — `miniboxd` has a `compile_error!` on macOS and `macboxd` will have one on Linux, so `--workspace` will always fail on one platform.

- [ ] **Step 8: Delete the original files from `miniboxd/src/`**

```bash
rm crates/miniboxd/src/handler.rs
rm crates/miniboxd/src/state.rs
rm crates/miniboxd/src/server.rs
```

- [ ] **Step 9: Verify again**

```bash
cargo check -p daemonbox -p miniboxd -p minibox-lib
```

Expected: `Finished` with no errors.

- [ ] **Step 10: Run existing handler tests**

```bash
cargo nextest run -p miniboxd -p daemonbox
```

Expected: all tests pass. The tests in `crates/miniboxd/tests/handler_tests.rs` import `miniboxd::handler` and `miniboxd::state` — these resolve through the lib.rs shim to `daemonbox`.

- [ ] **Step 10a: Clippy + fmt on new crate**

```bash
cargo fmt -p daemonbox --check
cargo clippy -p daemonbox -- -D warnings
```

Expected: no warnings, no fmt diff.

- [ ] **Step 11: Commit**

```bash
git add crates/daemonbox/ crates/miniboxd/src/lib.rs crates/miniboxd/Cargo.toml Cargo.toml
git commit -m "refactor: extract handler/state/server into daemonbox crate"
```

---

## Task 2: Create `macbox` library

**Files:**
- Create: `crates/macbox/Cargo.toml`
- Create: `crates/macbox/src/lib.rs`
- Create: `crates/macbox/src/preflight.rs`
- Create: `crates/macbox/src/paths.rs`
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Add `macbox` to workspace**

In root `Cargo.toml`, add to `[workspace]` members:
```toml
"crates/macbox",
```

Add to `[workspace.dependencies]`:
```toml
macbox = { path = "crates/macbox" }
```

- [ ] **Step 2: Create `crates/macbox/Cargo.toml`**

```toml
[package]
name = "macbox"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
daemonbox = { workspace = true }
minibox-lib = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 3: Write failing tests for `preflight.rs`**

Create `crates/macbox/src/preflight.rs`:

```rust
//! Colima VM lifecycle management.

use anyhow::Result;
use std::process::Command;
use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColimaStatus {
    Running,
    Stopped,
    NotInstalled,
}

#[derive(Error, Debug)]
pub enum MacboxError {
    #[error("Colima is not installed — run `brew install colima`")]
    ColimaNotInstalled,
    #[error("Colima VM failed to start: {0}")]
    VmStartFailed(String),
    #[error("preflight check failed: {0}")]
    PreflightFailed(String),
}

/// Check whether Colima is installed and whether the VM is running.
pub fn preflight() -> Result<ColimaStatus, MacboxError> {
    todo!()
}

/// Ensure Colima VM is running, starting it if currently stopped.
pub async fn ensure_vm_running() -> Result<(), MacboxError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colima_status_variants_are_distinct() {
        assert_ne!(ColimaStatus::Running, ColimaStatus::Stopped);
        assert_ne!(ColimaStatus::Stopped, ColimaStatus::NotInstalled);
    }

    #[test]
    fn macbox_error_messages_are_actionable() {
        let err = MacboxError::ColimaNotInstalled;
        assert!(err.to_string().contains("brew install colima"));
        let err = MacboxError::VmStartFailed("timeout".to_string());
        assert!(err.to_string().contains("timeout"));
    }
}
```

- [ ] **Step 4: Run tests to confirm they fail (todo! panics)**

```bash
cargo test -p macbox 2>&1 | head -20
```

Expected: test binary compiles, `colima_status_variants_are_distinct` passes (no `todo!`), `macbox_error_messages_are_actionable` passes.

- [ ] **Step 5: Implement `preflight()`**

Replace the `todo!()` in `preflight()`:

```rust
pub fn preflight() -> Result<ColimaStatus, MacboxError> {
    // Check if `colima` binary exists on PATH.
    let which = Command::new("which").arg("colima").output();
    match which {
        Err(_) | Ok(ref o) if !o.status.success() => {
            return Ok(ColimaStatus::NotInstalled);
        }
        _ => {}
    }

    // `colima status` exits 0 when running, non-zero when stopped.
    let status = Command::new("colima")
        .arg("status")
        .output()
        .map_err(|e| MacboxError::PreflightFailed(e.to_string()))?;

    if status.status.success() {
        Ok(ColimaStatus::Running)
    } else {
        Ok(ColimaStatus::Stopped)
    }
}
```

- [ ] **Step 6: Implement `ensure_vm_running()`**

Replace the `todo!()` in `ensure_vm_running()`:

```rust
pub async fn ensure_vm_running() -> Result<(), MacboxError> {
    match preflight()? {
        ColimaStatus::Running => {
            info!("Colima VM is already running");
            Ok(())
        }
        ColimaStatus::NotInstalled => Err(MacboxError::ColimaNotInstalled),
        ColimaStatus::Stopped => {
            warn!("Colima VM is stopped — starting...");
            let output = tokio::process::Command::new("colima")
                .arg("start")
                .output()
                .await
                .map_err(|e| MacboxError::VmStartFailed(e.to_string()))?;
            if output.status.success() {
                info!("Colima VM started");
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                Err(MacboxError::VmStartFailed(stderr))
            }
        }
    }
}
```

- [ ] **Step 7: Write and implement `paths.rs`**

Create `crates/macbox/src/paths.rs`:

```rust
//! macOS default paths for macboxd.

use std::path::PathBuf;

/// Persistent data directory: ~/Library/Application Support/macbox
pub fn data_dir() -> PathBuf {
    dirs_home()
        .map(|h| h.join("Library/Application Support/macbox"))
        .unwrap_or_else(|| PathBuf::from("/tmp/macbox-data"))
}

/// Runtime directory for sockets and PID files.
pub fn run_dir() -> PathBuf {
    PathBuf::from("/tmp/macbox")
}

/// Unix socket path.
pub fn socket_path() -> PathBuf {
    run_dir().join("macboxd.sock")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_is_under_run_dir() {
        let sock = socket_path();
        assert!(sock.starts_with(run_dir()));
    }

    #[test]
    fn data_dir_contains_macbox() {
        let d = data_dir();
        assert!(d.to_string_lossy().contains("macbox"));
    }
}
```

- [ ] **Step 8: Write and implement `colima_deps()` in `lib.rs`**

Create `crates/macbox/src/lib.rs`:

```rust
//! macOS orchestration library for macboxd.
//!
//! Provides Colima VM lifecycle management, macOS path conventions, and
//! adapter wiring for the macboxd daemon.
//!
//! This crate compiles on all platforms but is only useful on macOS —
//! `macboxd` guards non-macOS usage at startup via `compile_error!`.

pub mod paths;
pub mod preflight;

pub use preflight::{ColimaStatus, MacboxError, ensure_vm_running, preflight};

use daemonbox::handler::HandlerDependencies;
use minibox_lib::adapters::{ColimaFilesystem, ColimaLimiter, ColimaRegistry, ColimaRuntime};
use std::path::PathBuf;
use std::sync::Arc;

/// Build `HandlerDependencies` wired with the Colima adapter suite.
///
/// Pass in the resolved `containers_base` and `run_containers_base` paths
/// from the daemon's startup configuration.
pub fn colima_deps(
    containers_base: PathBuf,
    run_containers_base: PathBuf,
) -> HandlerDependencies {
    HandlerDependencies {
        registry: Arc::new(ColimaRegistry::new()),
        filesystem: Arc::new(ColimaFilesystem::new()),
        resource_limiter: Arc::new(ColimaLimiter::new()),
        runtime: Arc::new(ColimaRuntime::new()),
        containers_base,
        run_containers_base,
    }
}
```

- [ ] **Step 9: Fmt, clippy, and tests on macbox**

```bash
cargo fmt -p macbox --check
cargo clippy -p macbox -- -D warnings
cargo nextest run -p macbox
```

Expected: no fmt diff, no warnings, all tests pass.

- [ ] **Step 10: Check workspace compiles on macOS**

```bash
cargo check -p macbox -p daemonbox -p minibox-lib
```

Expected: `Finished` with no errors.

- [ ] **Step 11: Commit**

```bash
git add crates/macbox/ Cargo.toml
git commit -m "feat: add macbox lib — Colima preflight, paths, adapter wiring"
```

---

## Task 3: Create `macboxd` binary

**Files:**
- Create: `crates/macboxd/Cargo.toml`
- Create: `crates/macboxd/src/main.rs`
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Add `macboxd` to workspace**

In root `Cargo.toml`, add to `[workspace]` members:
```toml
"crates/macboxd",
```

- [ ] **Step 2: Create `crates/macboxd/Cargo.toml`**

```toml
[package]
name = "macboxd"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "macboxd"
path = "src/main.rs"

[dependencies]
macbox = { workspace = true }
daemonbox = { workspace = true }
minibox-lib = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

- [ ] **Step 3: Write `crates/macboxd/src/main.rs`**

```rust
//! macboxd — macOS container runtime daemon.
//!
//! Delegates all container operations to a Colima (Lima) VM via the
//! Colima adapter suite from minibox-lib.

#[cfg(not(target_os = "macos"))]
compile_error!("macboxd requires macOS");

use anyhow::{Context, Result};
use daemonbox::state::DaemonState;
use macbox::{colima_deps, ensure_vm_running, paths, preflight, ColimaStatus};
use minibox_lib::image::ImageStore;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    // ── Tracing ──────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("macboxd=info".parse().unwrap()),
        )
        .init();

    info!("macboxd starting");

    // ── Preflight ────────────────────────────────────────────────────────
    match preflight().context("Colima preflight check failed")? {
        ColimaStatus::NotInstalled => {
            anyhow::bail!("Colima is not installed. Run: brew install colima && colima start");
        }
        ColimaStatus::Stopped => {
            info!("Colima VM is stopped, starting...");
            ensure_vm_running()
                .await
                .context("failed to start Colima VM")?;
        }
        ColimaStatus::Running => {
            info!("Colima VM is running");
        }
    }

    // ── Resolve paths ────────────────────────────────────────────────────
    // All paths are overridable via env vars for testing.
    let data_dir = std::env::var("MACBOX_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| paths::data_dir());
    let run_dir = std::env::var("MACBOX_RUN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| paths::run_dir());
    let socket_path = std::env::var("MACBOX_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| paths::socket_path());

    // Sub-directories derived from the two roots:
    //   data_dir/images      — OCI layer cache (used by ImageStore)
    //   data_dir/containers  — per-container overlay dirs
    //   run_dir/containers   — per-container PID files and runtime state
    let images_dir = data_dir.join("images");
    let containers_dir = data_dir.join("containers");
    let run_containers_dir = run_dir.join("containers");

    // ── Directories ──────────────────────────────────────────────────────
    for dir in &[&images_dir, &containers_dir, &run_dir, &run_containers_dir] {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating directory {}", dir.display()))?;
    }

    // ── State ────────────────────────────────────────────────────────────
    // ImageStore manages the OCI layer cache on disk.
    // DaemonState tracks running containers in memory (persisted to data_dir).
    let image_store = ImageStore::new(&images_dir).context("creating image store")?;
    let state = Arc::new(DaemonState::new(image_store, &data_dir));
    state.load_from_disk().await;
    info!("state loaded");

    // ── Dependency injection ─────────────────────────────────────────────
    let deps = Arc::new(colima_deps(containers_dir, run_containers_dir));
    info!("Colima adapter suite wired");

    // ── Socket ───────────────────────────────────────────────────────────
    if socket_path.exists() {
        warn!("removing stale socket at {}", socket_path.display());
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("binding socket at {}", socket_path.display()))?;

    info!("listening on {}", socket_path.display());

    // ── Signal handling ──────────────────────────────────────────────────
    let mut sigterm = signal(SignalKind::terminate()).context("SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("SIGINT handler")?;

    // ── Accept loop ──────────────────────────────────────────────────────
    // macboxd accepts connections from any local user — the Colima VM handles
    // privilege internally, no UID 0 requirement on the macOS side.
    let require_root_auth = false;

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        // Spawn a task per connection so the accept loop is never blocked.
                        let state_c = Arc::clone(&state);
                        let deps_c = Arc::clone(&deps);
                        tokio::spawn(async move {
                            if let Err(e) = daemonbox::server::handle_connection(
                                stream, state_c, deps_c, require_root_auth,
                            ).await {
                                error!("connection error: {e:#}");
                            }
                        });
                    }
                    Err(e) => error!("accept error: {e}"),
                }
            }
            _ = sigterm.recv() => { info!("SIGTERM received, shutting down"); break; }
            _ = sigint.recv() => { info!("SIGINT received, shutting down"); break; }
        }
    }

    // ── Cleanup ──────────────────────────────────────────────────────────
    let _ = std::fs::remove_file(&socket_path);
    info!("macboxd stopped");
    Ok(())
}
```

- [ ] **Step 4: Build `macboxd` on macOS**

```bash
cargo build -p macboxd
```

Expected: `Finished` — binary at `target/debug/macboxd`. Fix any compilation errors before proceeding.

- [ ] **Step 5: Smoke test**

In one terminal:
```bash
MACBOX_RUN_DIR=/tmp/macbox-test MACBOX_DATA_DIR=/tmp/macbox-test-data ./target/debug/macboxd
```

Expected log output:
```
INFO macboxd starting
INFO Colima VM is running
INFO state loaded
INFO Colima adapter suite wired
INFO listening on /tmp/macbox-test/macboxd.sock
```

- [ ] **Step 6: Commit**

```bash
git add crates/macboxd/ Cargo.toml
git commit -m "feat: add macboxd — macOS-native daemon binary"
```

---

## Task 4: Final verification

- [ ] **Step 1: Full test suite (macOS)**

```bash
cargo nextest run -p minibox-lib -p daemonbox -p macbox
```

Expected: all tests pass.

- [ ] **Step 2: miniboxd tests still pass**

```bash
cargo nextest run -p miniboxd
```

Expected: all tests pass (handler/state tests resolve through lib.rs shim).

- [ ] **Step 3: Clippy + fmt on all new crates**

```bash
cargo fmt -p daemonbox -p macbox -p macboxd --check
cargo clippy -p daemonbox -p macbox -p macboxd -- -D warnings
```

Expected: no fmt diff, no warnings. Fix all warnings before proceeding.

- [ ] **Step 4: Full check on macOS (per-crate, not --workspace)**

```bash
cargo check -p daemonbox -p macbox -p macboxd -p minibox-lib -p minibox-cli
```

Note: always use per-crate `-p` flags. `miniboxd` has `compile_error!` on macOS; `macboxd` has `compile_error!` on Linux. `cargo check --workspace` will always fail on one platform.

Expected: `Finished` with no errors.

- [ ] **Step 5: Linux regression check (per-crate)**

Run in Colima VM or on a Linux machine:
```bash
cargo check -p daemonbox -p miniboxd -p minibox-lib -p minibox-cli -p minibox-bench
```

Note: omit `macbox` and `macboxd` — `macbox` compiles anywhere but the Colima adapters it depends on should be tested on the platform where they're built. `macboxd` has `compile_error!` on non-macOS.

Expected: `Finished` with no errors.

- [ ] **Step 6: End-to-end smoke test with Colima running**

```bash
# In one terminal
./target/debug/macboxd &

# In another terminal
MINIBOX_SOCKET_PATH=/tmp/macbox/macboxd.sock ./target/debug/minibox pull alpine
MINIBOX_SOCKET_PATH=/tmp/macbox/macboxd.sock ./target/debug/minibox ps
```

Expected: `pull` succeeds (or returns a meaningful Colima adapter error), `ps` returns the container list.

- [ ] **Step 7: Final commit**

```bash
git add -p
git commit -m "chore: macbox + daemonbox — final verification pass"
```

---

## Verification Summary

| Check | Command | Expected |
|-------|---------|----------|
| daemonbox compiles | `cargo check -p daemonbox` | No errors |
| macbox compiles | `cargo check -p macbox` | No errors (macOS only) |
| macboxd compiles | `cargo build -p macboxd` | Binary produced |
| miniboxd tests | `cargo test -p miniboxd --lib` | All pass |
| daemonbox tests | `cargo test -p daemonbox` | All pass |
| macbox tests | `cargo test -p macbox` | All pass |
| Linux workspace | `cargo build --workspace` (Linux) | No regressions |
