---
status: archived
completed: "2026-03-17"
superseded_by: 2026-03-19-cross-platform-daemon.md
note: Superseded by broader cross-platform plan
---

# macbox + daemonbox + winbox Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract shared daemon logic into `daemonbox`, create `macbox` (macOS orchestration lib), ship `macboxd` (macOS-native daemon binary wired to Colima adapters), and lay the groundwork for `winbox`/`winboxd` (Windows via WSL2).

**Architecture:** `daemonbox` holds `handler.rs`, `state.rs`, `server.rs` (moved from `miniboxd`) — zero infrastructure knowledge, depends only on domain port traits from `minibox`. `macbox` owns macOS-specific concerns: Colima preflight, VM lifecycle, path conventions, and adapter wiring. `macboxd` is a thin `main.rs` that sequences startup and delegates everything. `miniboxd/src/lib.rs` becomes a transparent re-export shim so existing tests require no changes. The same pattern then extends to Windows via `winbox`/`winboxd`.

**Platform dependency graph:**

```
miniboxd  → daemonbox → minibox   (Linux)
macboxd   → daemonbox → minibox   (macOS)
macboxd   → macbox    → minibox
winboxd   → daemonbox → minibox   (Windows)
winboxd   → winbox    → minibox
```

**Tech Stack:** Rust workspace, `minibox` (Colima/WSL2 adapters, domain traits), `nix` (POSIX signal/wait), `tokio` (async), `thiserror`/`anyhow`.

**Spec:** `docs/superpowers/specs/2026-03-17-macbox-daemonbox-design.md`

---

## File Map

### New files

| File                              | Responsibility                                                            |
| --------------------------------- | ------------------------------------------------------------------------- |
| `crates/daemonbox/Cargo.toml`     | Crate manifest                                                            |
| `crates/daemonbox/src/lib.rs`     | Public module exports (`pub mod handler; pub mod state; pub mod server;`) |
| `crates/daemonbox/src/handler.rs` | Moved from `miniboxd` — request handlers, `HandlerDependencies`           |
| `crates/daemonbox/src/state.rs`   | Moved from `miniboxd` — `DaemonState`, `ContainerRecord`                  |
| `crates/daemonbox/src/server.rs`  | Moved from `miniboxd` — Unix socket connection handler                    |
| `crates/macbox/Cargo.toml`        | Crate manifest                                                            |
| `crates/macbox/src/lib.rs`        | Public API: `preflight`, `ensure_vm_running`, `colima_deps`, re-exports   |
| `crates/macbox/src/preflight.rs`  | `ColimaStatus`, `MacboxError`, `preflight()`, `ensure_vm_running()`       |
| `crates/macbox/src/paths.rs`      | macOS default path functions                                              |
| `crates/macboxd/Cargo.toml`       | Crate manifest                                                            |
| `crates/macboxd/src/main.rs`      | macOS daemon entry point                                                  |
| `crates/winbox/Cargo.toml`        | Crate manifest                                                            |
| `crates/winbox/src/lib.rs`        | Public API: `preflight`, `ensure_wsl_running`, `wsl_deps`, re-exports     |
| `crates/winbox/src/preflight.rs`  | `Wsl2Status`, `WinboxError`, `preflight()`, `ensure_wsl_running()`        |
| `crates/winbox/src/paths.rs`      | Windows default path functions (data dir, run dir, socket path)           |
| `crates/winboxd/Cargo.toml`       | Crate manifest                                                            |
| `crates/winboxd/src/main.rs`      | Windows daemon entry point (`compile_error!` on non-Windows)              |

### Modified files

| File                          | Change                                                                                     |
| ----------------------------- | ------------------------------------------------------------------------------------------ |
| `Cargo.toml`                  | Add `daemonbox`, `macbox`, `macboxd` to workspace members + workspace.dependencies         |
| `crates/miniboxd/Cargo.toml`  | Add `daemonbox = { workspace = true }`                                                     |
| `crates/miniboxd/src/lib.rs`  | Replace module declarations with `pub use daemonbox::*` re-exports                         |
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
minibox = { workspace = true }
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
minibox = { workspace = true, features = [] }
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
cargo check -p daemonbox -p miniboxd -p minibox
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
cargo check -p daemonbox -p miniboxd -p minibox
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
minibox = { workspace = true }
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
use minibox::adapters::{ColimaFilesystem, ColimaLimiter, ColimaRegistry, ColimaRuntime};
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
cargo check -p macbox -p daemonbox -p minibox
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
minibox = { workspace = true }
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
//! Colima adapter suite from minibox.

#[cfg(not(target_os = "macos"))]
compile_error!("macboxd requires macOS");

use anyhow::{Context, Result};
use daemonbox::state::DaemonState;
use macbox::{colima_deps, ensure_vm_running, paths, preflight, ColimaStatus};
use minibox::image::ImageStore;
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

## Task 4: Create `winbox` library and `winboxd` binary

> **Note:** This task targets Windows. Build and test on a Windows machine with WSL2 installed. The WSL2 adapter already exists in `minibox/src/adapters/wsl.rs`.

### Architecture: Named Pipe proxy + WSL2 distro

**Key insight from research:** `tokio::net::UnixListener` is `#[cfg(unix)]` and does not compile on Windows. The cleanest Windows architecture mirrors Docker Desktop:

1. **`winboxd.exe`** — Windows Service (`windows-service` crate) listening on `\\.\pipe\miniboxd` (Tokio Named Pipe, built into `tokio::net::windows::named_pipe`).
2. **WSL2 distro** — runs the existing Linux `miniboxd` inside WSL2, unchanged.
3. **Proxy layer** — `winboxd.exe` forwards the JSON-over-newline protocol from the Named Pipe to the Linux `miniboxd` Unix socket exposed at `/mnt/wsl/minibox/miniboxd.sock` (the shared tmpfs between all WSL2 distros).
4. **`minibox.exe` CLI** on Windows connects to `\\.\pipe\miniboxd`.

```
Windows CLI  →  \\.\pipe\miniboxd  →  winboxd.exe  →  /mnt/wsl/minibox/miniboxd.sock  →  miniboxd (WSL2)
```

The Linux `miniboxd` inside WSL2 is **unmodified** — full namespace/cgroups container support via the existing native adapter.

### Crate additions for `winbox`/`winboxd`

```toml
[target.'cfg(windows)'.dependencies]
tokio           = { features = ["rt-multi-thread", "net", "io-util"] }  # Named Pipe built-in
windows-service = "0.7"   # Windows SCM: install/start/stop/control handlers
wslapi          = "0.1"   # wslapi.dll bindings: distro detection, launch, config
windows         = { version = "0.58", features = ["Win32_System_Registry",
                    "Win32_Foundation", "Win32_Security"] }  # registry + ACLs
```

**Files:**

- Create: `crates/winbox/Cargo.toml`
- Create: `crates/winbox/src/lib.rs`
- Create: `crates/winbox/src/preflight.rs`
- Create: `crates/winbox/src/paths.rs`
- Create: `crates/winboxd/Cargo.toml`
- Create: `crates/winboxd/src/main.rs`
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Add `winbox` and `winboxd` to workspace**

In root `Cargo.toml`, add to `[workspace]` members:

```toml
"crates/winbox",
"crates/winboxd",
```

Add to `[workspace.dependencies]`:

```toml
winbox = { path = "crates/winbox" }
```

- [ ] **Step 2: Create `crates/winbox/Cargo.toml`**

```toml
[package]
name = "winbox"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
daemonbox = { workspace = true }
minibox = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 3: Create `crates/winbox/src/preflight.rs`**

Use `wslapi` for detection and `wsl.exe` child process for launch (no undocumented COM APIs):

```rust
//! WSL2 lifecycle management.
//!
//! Uses `wslapi.dll` bindings for distro detection and `wsl.exe` process
//! invocation for distro startup — the same approach used by Docker Desktop
//! and Rancher Desktop.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wsl2Status {
    /// miniboxd-wsl distro is registered and the miniboxd socket is reachable.
    Running,
    /// WSL2 is installed but the miniboxd distro is not yet started.
    Stopped,
    /// WSL2 (`wslapi.dll`) is not installed or not available.
    NotInstalled,
}

#[derive(Error, Debug)]
pub enum WinboxError {
    #[error("WSL2 is not installed — run: wsl --install")]
    Wsl2NotInstalled,
    #[error("miniboxd WSL2 distro is not registered — run: winbox install-distro")]
    DistroNotRegistered,
    #[error("WSL2 failed to start: {0}")]
    Wsl2StartFailed(String),
    #[error("preflight check failed: {0}")]
    PreflightFailed(String),
}

/// The WSL2 distro name used by minibox.
pub const MINIBOX_DISTRO: &str = "minibox-wsl";

/// Check WSL2 availability and miniboxd distro registration.
///
/// Uses `wslapi.dll` to probe installation state without spawning processes.
pub fn preflight() -> Result<Wsl2Status, WinboxError> {
    // Attempt to load wslapi.dll — absent means WSL2 not installed.
    let lib = match wslapi::Library::new() {
        Ok(l) => l,
        Err(_) => return Ok(Wsl2Status::NotInstalled),
    };

    if !lib.is_distribution_registered(MINIBOX_DISTRO)
        .unwrap_or(false)
    {
        return Err(WinboxError::DistroNotRegistered);
    }

    // Distro registered; check if the shared socket exists on /mnt/wsl.
    // The socket is created by miniboxd on startup inside the distro.
    let socket_exists = std::process::Command::new("wsl")
        .args(["--distribution", MINIBOX_DISTRO, "--", "test", "-S",
               "/mnt/wsl/minibox/miniboxd.sock"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if socket_exists {
        Ok(Wsl2Status::Running)
    } else {
        Ok(Wsl2Status::Stopped)
    }
}

/// Start `miniboxd` inside the WSL2 distro if not already running.
///
/// Launches `miniboxd` as a background process inside the distro and
/// waits for the socket to appear on `/mnt/wsl/minibox/miniboxd.sock`.
pub async fn ensure_wsl_running() -> Result<(), WinboxError> {
    match preflight()? {
        Wsl2Status::Running => Ok(()),
        Wsl2Status::NotInstalled => Err(WinboxError::Wsl2NotInstalled),
        Wsl2Status::Stopped => {
            // Start miniboxd inside WSL2 in the background.
            tokio::process::Command::new("wsl")
                .args(["--distribution", MINIBOX_DISTRO, "--",
                       "nohup", "miniboxd", "&"])
                .spawn()
                .map_err(|e| WinboxError::Wsl2StartFailed(e.to_string()))?;

            // Poll until the socket appears (up to 10s).
            for _ in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if matches!(preflight()?, Wsl2Status::Running) {
                    return Ok(());
                }
            }
            Err(WinboxError::Wsl2StartFailed(
                "socket did not appear after 10s".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl2_status_variants_are_distinct() {
        assert_ne!(Wsl2Status::Running, Wsl2Status::Stopped);
        assert_ne!(Wsl2Status::Stopped, Wsl2Status::NotInstalled);
    }

    #[test]
    fn winbox_error_messages_are_actionable() {
        let err = WinboxError::Wsl2NotInstalled;
        assert!(err.to_string().contains("wsl --install"));
        let err = WinboxError::Wsl2StartFailed("timeout".to_string());
        assert!(err.to_string().contains("timeout"));
    }
}
```

- [ ] **Step 4: Create `crates/winbox/src/paths.rs`**

```rust
//! Windows default paths for winboxd.

use std::path::PathBuf;

/// Persistent data directory: %APPDATA%\winbox
pub fn data_dir() -> PathBuf {
    std::env::var("APPDATA")
        .map(|p| PathBuf::from(p).join("winbox"))
        .unwrap_or_else(|_| PathBuf::from(r"C:\ProgramData\winbox"))
}

/// Runtime directory for sockets and PID files.
pub fn run_dir() -> PathBuf {
    std::env::var("TEMP")
        .map(|p| PathBuf::from(p).join("winbox"))
        .unwrap_or_else(|_| PathBuf::from(r"C:\Temp\winbox"))
}

/// Named pipe / Unix socket path (WSL2 bridge).
pub fn socket_path() -> PathBuf {
    run_dir().join("winboxd.sock")
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
    fn data_dir_contains_winbox() {
        let d = data_dir();
        assert!(d.to_string_lossy().contains("winbox"));
    }
}
```

- [ ] **Step 5: Create `crates/winbox/src/lib.rs`**

```rust
//! Windows orchestration library for winboxd.
//!
//! Provides WSL2 lifecycle management and Windows path conventions for winboxd.
//!
//! Architecture: winboxd is a Named Pipe proxy — it does NOT embed daemonbox
//! directly. Instead it forwards the JSON-over-newline protocol from the
//! Windows Named Pipe to the Linux miniboxd Unix socket inside WSL2 via the
//! shared `/mnt/wsl/minibox/miniboxd.sock` path.

pub mod paths;
pub mod preflight;
pub mod proxy;

pub use preflight::{Wsl2Status, WinboxError, MINIBOX_DISTRO, ensure_wsl_running, preflight};
```

Also create `crates/winbox/src/proxy.rs` — thin byte-forwarding proxy between Named Pipe and WSL2 socket:

```rust
//! Named Pipe ↔ WSL2 Unix socket proxy.
//!
//! Forwards raw bytes in both directions so the existing JSON-over-newline
//! protocol works unchanged. No protocol awareness needed.

use anyhow::Result;
use tokio::io;
use tokio::net::windows::named_pipe::NamedPipeServer;

/// Path of the miniboxd Unix socket exposed on the WSL2 shared tmpfs.
/// All WSL2 distros in the same VM can see /mnt/wsl/.
pub const WSL_SOCKET_PATH: &str = "/mnt/wsl/minibox/miniboxd.sock";

/// Windows Named Pipe name exposed to Windows-side clients.
pub const NAMED_PIPE: &str = r"\\.\pipe\miniboxd";

/// Relay a single Named Pipe client connection to the WSL2 Unix socket.
///
/// Spawns bidirectional copy tasks and returns when either side closes.
pub async fn relay(pipe: NamedPipeServer) -> Result<()> {
    // Open a TCP connection to the WSL2 Unix socket via `wsl.exe --exec socat`
    // or directly via the Windows AF_UNIX support (Win10 1809+).
    // For maximum compatibility use a wsl.exe child process as a bridge.
    let mut child = tokio::process::Command::new("wsl")
        .args(["--distribution", crate::preflight::MINIBOX_DISTRO,
               "--exec", "socat", "-", &format!("UNIX-CONNECT:{WSL_SOCKET_PATH}")])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;

    let mut child_stdin = child.stdin.take().unwrap();
    let mut child_stdout = child.stdout.take().unwrap();

    let (mut pipe_reader, mut pipe_writer) = io::split(pipe);
    let (mut wsl_reader, mut wsl_writer) = (&mut child_stdout, &mut child_stdin);

    tokio::select! {
        r = io::copy(&mut pipe_reader, &mut wsl_writer) => { r?; }
        r = io::copy(&mut wsl_reader, &mut pipe_writer) => { r?; }
    }
    Ok(())
}
```

> **Implementation note:** `socat` must be installed in the WSL2 distro (`apt install socat`). An alternative is to use `tokio-uds-windows` (Azure crate) which provides `UnixStream` on Windows directly, avoiding the `socat` child process. Evaluate both during implementation.

- [ ] **Step 6: Create `crates/winboxd/Cargo.toml`**

```toml
[package]
name = "winboxd"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "winboxd"
path = "src/main.rs"

[dependencies]
winbox = { workspace = true }
daemonbox = { workspace = true }
minibox = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

- [ ] **Step 7: Create `crates/winboxd/src/main.rs`**

`winboxd` is a **Named Pipe proxy** — it does not embed `daemonbox`. It:

1. Runs preflight (WSL2 available, distro registered, miniboxd running inside WSL2)
2. Creates a Tokio Named Pipe server (`\\.\pipe\miniboxd`)
3. For each client connection, spawns a `winbox::proxy::relay` task

```rust
#[cfg(not(target_os = "windows"))]
compile_error!("winboxd requires Windows");

use anyhow::{Context, Result};
use tokio::net::windows::named_pipe::ServerOptions;
use tracing::{error, info};
use winbox::{ensure_wsl_running, preflight, proxy, Wsl2Status};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("winboxd=info".parse().unwrap()),
        )
        .init();

    info!("winboxd starting");

    // ── Preflight ─────────────────────────────────────────────────────────
    match preflight().context("WSL2 preflight failed")? {
        Wsl2Status::NotInstalled => {
            anyhow::bail!("WSL2 is not installed. Run: wsl --install");
        }
        Wsl2Status::Stopped => {
            info!("miniboxd WSL2 distro not running — starting...");
            ensure_wsl_running().await.context("failed to start miniboxd in WSL2")?;
        }
        Wsl2Status::Running => {
            info!("miniboxd WSL2 distro is running");
        }
    }

    // ── Named Pipe server ─────────────────────────────────────────────────
    info!("listening on {}", proxy::NAMED_PIPE);

    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(false)
            .create(proxy::NAMED_PIPE)
            .context("creating named pipe instance")?;

        // connect() waits for the next client.
        server.connect().await.context("waiting for client")?;
        info!("client connected");

        tokio::spawn(async move {
            if let Err(e) = proxy::relay(server).await {
                error!("relay error: {e:#}");
            }
        });
    }
}
```

- [ ] **Step 8: Fmt, clippy, and tests on winbox (Windows)**

```bash
cargo fmt -p winbox --check
cargo clippy -p winbox -- -D warnings
cargo nextest run -p winbox
```

Expected: no fmt diff, no warnings, all tests pass.

- [ ] **Step 9: Build winboxd on Windows**

```bash
cargo build -p winboxd
```

Expected: `Finished` — binary at `target\debug\winboxd.exe`.

- [ ] **Step 10: Commit**

```bash
git add crates/winbox/ crates/winboxd/ Cargo.toml
git commit -m "feat: add winbox lib and winboxd — Windows-native daemon via WSL2"
```

---

## Task 5: Final verification

- [ ] **Step 1: Full test suite (macOS)**

```bash
cargo nextest run -p minibox -p daemonbox -p macbox
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
cargo check -p daemonbox -p macbox -p macboxd -p minibox -p minibox-cli
```

Note: always use per-crate `-p` flags. `miniboxd` has `compile_error!` on macOS; `macboxd` has `compile_error!` on Linux. `cargo check --workspace` will always fail on one platform.

Expected: `Finished` with no errors.

- [ ] **Step 5: Linux regression check (per-crate)**

Run in Colima VM or on a Linux machine:

```bash
cargo check -p daemonbox -p miniboxd -p minibox -p minibox-cli -p minibox-bench
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

| Check              | Command                                | Platform | Expected        |
| ------------------ | -------------------------------------- | -------- | --------------- |
| daemonbox compiles | `cargo check -p daemonbox`             | any      | No errors       |
| macbox compiles    | `cargo check -p macbox`                | macOS    | No errors       |
| macboxd compiles   | `cargo build -p macboxd`               | macOS    | Binary produced |
| winbox compiles    | `cargo check -p winbox`                | Windows  | No errors       |
| winboxd compiles   | `cargo build -p winboxd`               | Windows  | Binary produced |
| miniboxd tests     | `cargo test -p miniboxd --lib`         | Linux    | All pass        |
| daemonbox tests    | `cargo test -p daemonbox`              | any      | All pass        |
| macbox tests       | `cargo test -p macbox`                 | macOS    | All pass        |
| winbox tests       | `cargo test -p winbox`                 | Windows  | All pass        |
| Linux workspace    | `cargo check -p daemonbox -p miniboxd` | Linux    | No regressions  |

## Platform Support Matrix

| Platform | Orchestration | Daemon   | Adapter(s)             | Status          |
| -------- | ------------- | -------- | ---------------------- | --------------- |
| Linux    | (none)        | miniboxd | Native (namespaces)    | Complete        |
| macOS    | macbox        | macboxd  | Colima, Docker Desktop | Tasks 1–3 above |
| Windows  | winbox        | winboxd  | WSL2                   | Task 4 above    |
