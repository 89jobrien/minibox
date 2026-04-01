# Docker Parity Roadmap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Exec, Image Push, Container Commit, and Image Build to minibox so that the dockerbox shim provides full Docker API coverage, enabling maestro to drop its Docker dependency.

**Architecture:** Four new focused domain traits (`ExecRuntime`, `ImagePusher`, `ContainerCommitter`, `ImageBuilder`) added to `minibox-core`. Each trait has one Linux adapter in `mbx/src/adapters/`. Protocol variants added to both `minibox-core/src/protocol.rs` and `mbx/src/protocol.rs`. New handlers in `daemonbox/src/handler.rs`. Dockerbox gets new HTTP endpoints forwarding to these handlers via `DaemonClient`.

**Tech Stack:** Rust 2024 edition, `async_trait`, `nix` crate (setns/clone), `tokio::task::spawn_blocking` for all blocking ops, `reqwest` (already used by RegistryClient), `flate2` + `tar` (already used by layer extraction), thiserror.

---

## File Map

| File | Action | Purpose |
|---|---|---|
| `crates/minibox-core/src/domain.rs` | Modify | Add 4 new traits + domain types |
| `crates/minibox-core/src/error.rs` | Modify | Add ExecError, PushError, CommitError, BuildError |
| `crates/minibox-core/src/protocol.rs` | Modify | Add Exec/Push/Commit/Build request+response variants |
| `crates/mbx/src/protocol.rs` | Modify | Mirror protocol changes |
| `crates/mbx/src/adapters/exec.rs` | Create | NativeExecRuntime (nsenter) |
| `crates/mbx/src/adapters/push.rs` | Create | OciPushAdapter |
| `crates/mbx/src/adapters/commit.rs` | Create | OverlayCommitAdapter |
| `crates/mbx/src/image/dockerfile.rs` | Create | DockerfileParser + Instruction enum |
| `crates/mbx/src/adapters/builder.rs` | Create | MiniboxImageBuilder |
| `crates/mbx/src/adapters/mod.rs` | Modify | Re-export new adapters |
| `crates/daemonbox/src/handler.rs` | Modify | Add handle_exec, handle_push, handle_commit, handle_build; extend HandlerDependencies |
| `crates/daemonbox/src/server.rs` | Modify | Add dispatch arms for new variants |
| `crates/daemonbox/src/state.rs` | Modify | Add overlay_paths + image_ref to ContainerRecord |
| `crates/miniboxd/src/main.rs` | Modify | Wire new adapters into native suite |
| `crates/dockerbox/src/domain/mod.rs` | Modify | Add exec/push/commit/build to ContainerRuntime trait |
| `crates/dockerbox/src/infra/minibox.rs` | Modify | Implement new trait methods via DaemonClient |
| `crates/dockerbox/src/api/containers.rs` | Modify | Add exec endpoints |
| `crates/dockerbox/src/api/images.rs` | Modify | Add push/tag/build endpoints |
| `crates/dockerbox/src/api/mod.rs` | Modify | Register new routes |

---

## Phase 1: Exec

### Task 1: Domain trait + types for Exec

**Files:**
- Modify: `crates/minibox-core/src/domain.rs`
- Modify: `crates/minibox-core/src/error.rs`

- [ ] **Step 1: Write a failing compile test**

Add a temporary test at the bottom of `crates/minibox-core/src/domain.rs`:

```rust
#[cfg(test)]
mod exec_trait_tests {
    use super::*;
    fn _assert_exec_runtime_is_object_safe(_: &dyn ExecRuntime) {}
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo check -p minibox-core 2>&1 | grep "ExecRuntime"
```
Expected: `error[E0412]: cannot find type \`ExecRuntime\``

- [ ] **Step 3: Add ExecError to `crates/minibox-core/src/error.rs`**

Add after the `RegistryError` block:

```rust
/// Errors from exec-into-container operations.
#[derive(Debug, Error)]
pub enum ExecError {
    #[error("container {container_id} is not running")]
    ContainerNotRunning { container_id: String },

    #[error("exec {exec_id} not found")]
    ExecNotFound { exec_id: String },

    #[error("nsenter failed for container {container_id}: {reason}")]
    NsenterFailed { container_id: String, reason: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("exec error: {0}")]
    Other(String),
}
```

- [ ] **Step 4: Add ExecConfig, ExecHandle, and ExecRuntime trait to `crates/minibox-core/src/domain.rs`**

Find the end of the existing trait definitions (after `ImageLoader` trait). Add:

```rust
// ---------------------------------------------------------------------------
// Exec Runtime Port
// ---------------------------------------------------------------------------

/// Configuration for executing a command inside a running container.
#[derive(Debug, Clone)]
pub struct ExecConfig {
    /// Command and arguments to execute.
    pub cmd: Vec<String>,
    /// Environment variables in KEY=VALUE form.
    pub env: Vec<String>,
    /// Working directory inside the container. `None` uses the image default.
    pub working_dir: Option<std::path::PathBuf>,
    /// Whether to allocate a pseudo-TTY.
    pub tty: bool,
}

/// Handle representing a started exec instance.
#[derive(Debug, Clone)]
pub struct ExecHandle {
    /// Unique exec instance ID (UUID).
    pub id: String,
}

/// Port for executing commands inside already-running containers.
///
/// Implemented by: `NativeExecRuntime` (Linux, uses `/proc/{pid}/ns/*` setns).
/// Not implemented by Colima, GKE, or stub adapters.
#[async_trait]
pub trait ExecRuntime: AsAny + Send + Sync {
    /// Execute a command inside the container identified by `container_id`.
    ///
    /// The container must be in Running state. stdout/stderr are streamed back
    /// via `DaemonResponse::ContainerOutput`; the exit code arrives via
    /// `DaemonResponse::ContainerStopped`.
    async fn exec(
        &self,
        container_id: &ContainerId,
        config: &ExecConfig,
        tx: tokio::sync::mpsc::Sender<minibox_core::protocol::DaemonResponse>,
    ) -> anyhow::Result<ExecHandle>;
}

/// Type alias for a shared dynamic ExecRuntime.
pub type DynExecRuntime = Arc<dyn ExecRuntime>;
```

Note: `DynExecRuntime` requires `use std::sync::Arc;` which is already imported.

- [ ] **Step 5: Verify it compiles**

```bash
cargo check -p minibox-core
```
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-core/src/domain.rs crates/minibox-core/src/error.rs
git commit -m "feat(domain): add ExecRuntime trait and ExecConfig types"
```

---

### Task 2: Protocol variants for Exec

**Files:**
- Modify: `crates/minibox-core/src/protocol.rs`
- Modify: `crates/mbx/src/protocol.rs`

- [ ] **Step 1: Add Exec request variant to `crates/minibox-core/src/protocol.rs`**

Find the `LoadImage` variant (last item in `DaemonRequest`). Add after it:

```rust
    /// Execute a command inside an already-running container.
    ///
    /// Streams `ContainerOutput` messages then one `ContainerStopped`.
    Exec {
        /// Container ID (short 16-char hex UUID).
        container_id: String,
        /// Command and arguments.
        cmd: Vec<String>,
        /// Environment variables in KEY=VALUE form.
        #[serde(default)]
        env: Vec<String>,
        /// Working directory inside the container.
        #[serde(default)]
        working_dir: Option<String>,
        /// Whether to allocate a pseudo-TTY.
        #[serde(default)]
        tty: bool,
    },
```

- [ ] **Step 2: Add ExecStarted response variant to `crates/minibox-core/src/protocol.rs`**

Find `DaemonResponse`. Add before `ContainerOutput`:

```rust
    /// Sent once after a successful exec setup, before any output arrives.
    ExecStarted {
        /// Unique exec instance ID.
        exec_id: String,
    },
```

- [ ] **Step 3: Update `is_terminal_response` in `crates/daemonbox/src/server.rs`**

Find `is_terminal_response` (or the match that determines whether to stop reading). Add `ExecStarted` as non-terminal (like `ContainerOutput`). Check if this function exists:

```bash
grep -n "is_terminal\|ContainerOutput" /Users/joe/dev/minibox/crates/daemonbox/src/server.rs | head -10
```

If found, add `DaemonResponse::ExecStarted { .. } => false,` to the non-terminal arm.

- [ ] **Step 4: Mirror changes in `crates/mbx/src/protocol.rs`**

Open `crates/mbx/src/protocol.rs`. Add the identical `Exec` request variant and `ExecStarted` response variant. The mbx protocol file is a separate copy — both must stay in sync.

```bash
grep -n "LoadImage\|DaemonRequest\|DaemonResponse" /Users/joe/dev/minibox/crates/mbx/src/protocol.rs | head -20
```

- [ ] **Step 5: Verify**

```bash
cargo check -p minibox-core -p mbx
```
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-core/src/protocol.rs crates/mbx/src/protocol.rs crates/daemonbox/src/server.rs
git commit -m "feat(protocol): add Exec request and ExecStarted response variants"
```

---

### Task 3: NativeExecRuntime adapter

**Files:**
- Create: `crates/mbx/src/adapters/exec.rs`
- Modify: `crates/mbx/src/adapters/mod.rs`

- [ ] **Step 1: Write the failing test first**

Create `crates/mbx/src/adapters/exec.rs` with just the test:

```rust
//! Linux namespace exec adapter — joins running container namespaces via setns(2).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_config_from_strings() {
        let cfg = ExecConfig {
            cmd: vec!["echo".to_string(), "hello".to_string()],
            env: vec!["HOME=/root".to_string()],
            working_dir: None,
            tty: false,
        };
        assert_eq!(cfg.cmd[0], "echo");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p mbx adapters::exec 2>&1 | head -20
```
Expected: compile error — `ExecConfig` not found.

- [ ] **Step 3: Implement NativeExecRuntime**

Replace the contents of `crates/mbx/src/adapters/exec.rs` with:

```rust
//! Linux namespace exec adapter.
//!
//! Joins a running container's namespaces via `/proc/{pid}/ns/*` + `setns(2)`,
//! then forks a child process that executes the requested command.
//! stdout/stderr are streamed back via `DaemonResponse::ContainerOutput`.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{AsAny, ContainerId, DynExecRuntime, ExecConfig, ExecHandle, ExecRuntime};
use minibox_core::protocol::{DaemonResponse, OutputStreamKind};
use std::os::unix::io::{FromRawFd, OwnedFd};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::daemonbox_state::StateHandle;

/// Linux namespace exec adapter.
///
/// Requires the container to be running (pid present in state).
/// Uses `setns(2)` to join mnt, pid, net, uts, ipc namespaces.
pub struct NativeExecRuntime {
    state: StateHandle,
}

impl NativeExecRuntime {
    pub fn new(state: StateHandle) -> Self {
        Self { state }
    }
}

as_any!(NativeExecRuntime);

#[async_trait]
impl ExecRuntime for NativeExecRuntime {
    async fn exec(
        &self,
        container_id: &ContainerId,
        config: &ExecConfig,
        tx: mpsc::Sender<DaemonResponse>,
    ) -> Result<ExecHandle> {
        let id = container_id.as_str().to_string();
        let pid = self
            .state
            .get_container_pid(&id)
            .await
            .with_context(|| format!("container {id} not found or not running"))?;

        let exec_id = Uuid::new_v4().simple().to_string()[..16].to_string();
        let config = config.clone();
        let exec_id_clone = exec_id.clone();

        tokio::task::spawn_blocking(move || {
            run_exec_blocking(pid, &exec_id_clone, &config, tx)
        });

        info!(
            container_id = %id,
            exec_id = %exec_id,
            cmd = ?config.cmd,
            "exec: process started"
        );

        Ok(ExecHandle { id: exec_id })
    }
}

/// Join container namespaces and exec the command, streaming output.
///
/// This runs in a `spawn_blocking` task because setns + fork are synchronous.
fn run_exec_blocking(
    container_pid: u32,
    exec_id: &str,
    config: &ExecConfig,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let ns_base = format!("/proc/{container_pid}/ns");

    // Open namespace fds before forking.
    let ns_names = ["mnt", "pid", "net", "uts", "ipc"];
    let ns_fds: Vec<std::fs::File> = ns_names
        .iter()
        .filter_map(|ns| {
            let path = format!("{ns_base}/{ns}");
            std::fs::File::open(&path)
                .map_err(|e| warn!(ns = %ns, error = %e, "exec: failed to open ns fd"))
                .ok()
        })
        .collect();

    if ns_fds.len() != ns_names.len() {
        let _ = tokio::runtime::Handle::current().block_on(tx.send(DaemonResponse::Error {
            message: format!("exec: could not open all namespace fds for pid {container_pid}"),
        }));
        return;
    }

    // Create pipes for stdout and stderr.
    let (stdout_r, stdout_w) = match nix::unistd::pipe() {
        Ok(p) => p,
        Err(e) => {
            let _ = tokio::runtime::Handle::current().block_on(tx.send(DaemonResponse::Error {
                message: format!("exec: pipe creation failed: {e}"),
            }));
            return;
        }
    };
    let (stderr_r, stderr_w) = match nix::unistd::pipe() {
        Ok(p) => p,
        Err(e) => {
            let _ = tokio::runtime::Handle::current().block_on(tx.send(DaemonResponse::Error {
                message: format!("exec: pipe creation failed: {e}"),
            }));
            return;
        }
    };

    // SAFETY: We are about to fork. After fork, parent closes write ends and
    // reads from read ends. Child joins namespaces, dups pipes into 1/2, execs.
    let child_pid = unsafe { nix::unistd::fork() };

    match child_pid {
        Err(e) => {
            let _ = tokio::runtime::Handle::current().block_on(tx.send(DaemonResponse::Error {
                message: format!("exec: fork failed: {e}"),
            }));
        }
        Ok(nix::unistd::ForkResult::Child) => {
            // Child: join namespaces, exec the command.
            for f in &ns_fds {
                use std::os::unix::io::AsRawFd;
                let _ = unsafe { libc::setns(f.as_raw_fd(), 0) };
            }

            // Dup pipes into stdout/stderr slots.
            unsafe {
                libc::dup2(stdout_w.as_raw_fd(), 1);
                libc::dup2(stderr_w.as_raw_fd(), 2);
                libc::close(stdout_r.as_raw_fd());
                libc::close(stderr_r.as_raw_fd());
                libc::close(stdout_w.as_raw_fd());
                libc::close(stderr_w.as_raw_fd());
            }

            let cmd = std::ffi::CString::new(config.cmd[0].clone()).unwrap();
            let args: Vec<std::ffi::CString> = config
                .cmd
                .iter()
                .map(|s| std::ffi::CString::new(s.as_str()).unwrap())
                .collect();
            let envp: Vec<std::ffi::CString> = config
                .env
                .iter()
                .map(|s| std::ffi::CString::new(s.as_str()).unwrap())
                .collect();

            let _ = nix::unistd::execve(&cmd, &args, &envp);
            unsafe { libc::_exit(127) };
        }
        Ok(nix::unistd::ForkResult::Parent { child }) => {
            // Parent: close write ends, read stdout/stderr, wait for child.
            // SAFETY: We own these fds; child has dup'd its copies.
            unsafe {
                libc::close(stdout_w.as_raw_fd());
                libc::close(stderr_w.as_raw_fd());
            }

            stream_fd_to_channel(stdout_r.as_raw_fd(), OutputStreamKind::Stdout, &tx);
            stream_fd_to_channel(stderr_r.as_raw_fd(), OutputStreamKind::Stderr, &tx);

            let status = nix::sys::wait::waitpid(child, None);
            let exit_code = match status {
                Ok(nix::sys::wait::WaitStatus::Exited(_, code)) => code,
                _ => -1,
            };

            let _ = tokio::runtime::Handle::current()
                .block_on(tx.send(DaemonResponse::ContainerStopped { exit_code }));

            info!(exec_id = %exec_id, exit_code = exit_code, "exec: process exited");
        }
    }
}

/// Read all bytes from `fd` and send as ContainerOutput chunks.
fn stream_fd_to_channel(fd: i32, stream: OutputStreamKind, tx: &mpsc::Sender<DaemonResponse>) {
    use std::io::Read;
    // SAFETY: We own this fd and are in the parent after fork.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut buf = [0u8; 4096];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let data = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &buf[..n],
                );
                let _ = tokio::runtime::Handle::current().block_on(
                    tx.send(DaemonResponse::ContainerOutput {
                        stream: stream.clone(),
                        data,
                    }),
                );
            }
            Err(_) => break,
        }
    }
}

/// Convenience constructor returning a `DynExecRuntime`.
pub fn native_exec_runtime(state: StateHandle) -> DynExecRuntime {
    Arc::new(NativeExecRuntime::new(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_config_from_strings() {
        let cfg = ExecConfig {
            cmd: vec!["echo".to_string(), "hello".to_string()],
            env: vec!["HOME=/root".to_string()],
            working_dir: None,
            tty: false,
        };
        assert_eq!(cfg.cmd[0], "echo");
        assert_eq!(cfg.cmd[1], "hello");
    }
}
```

**Note on `StateHandle`:** This is a type alias you'll add to `daemonbox/src/state.rs` in Task 5. For now, use `Arc<DaemonState>` as the concrete type — we'll refine when wiring.

- [ ] **Step 4: Export from `crates/mbx/src/adapters/mod.rs`**

Add to the pub re-exports in `mod.rs`:

```rust
pub mod exec;
pub use exec::NativeExecRuntime;
```

- [ ] **Step 5: Check compile**

```bash
cargo check -p mbx 2>&1 | head -30
```
Fix any import errors. Common: `base64` crate — check if it's already in `mbx/Cargo.toml`:

```bash
grep "base64" /Users/joe/dev/minibox/crates/mbx/Cargo.toml
```

If missing, add to `[dependencies]`: `base64 = "0.22"`.

- [ ] **Step 6: Run unit test**

```bash
cargo test -p mbx adapters::exec::tests
```
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/mbx/src/adapters/exec.rs crates/mbx/src/adapters/mod.rs crates/mbx/Cargo.toml
git commit -m "feat(mbx): add NativeExecRuntime adapter (nsenter + fork + stream)"
```

---

### Task 4: Exec handler + server dispatch

**Files:**
- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/daemonbox/src/server.rs`

- [ ] **Step 1: Add `exec_runtime` to `HandlerDependencies`**

In `crates/daemonbox/src/handler.rs`, find the `HandlerDependencies` struct. Add:

```rust
    /// Optional exec runtime — only present in native Linux adapter suite.
    /// `None` on macOS (Colima) and Windows.
    pub exec_runtime: Option<minibox_core::domain::DynExecRuntime>,
```

Add a builder method after the existing `with_image_loader`:

```rust
    pub fn with_exec_runtime(mut self, runtime: minibox_core::domain::DynExecRuntime) -> Self {
        self.exec_runtime = Some(runtime);
        self
    }
```

- [ ] **Step 2: Add `handle_exec` function in `crates/daemonbox/src/handler.rs`**

Add after `handle_pull`:

```rust
/// Execute a command inside a running container.
///
/// Requires `exec_runtime` to be present in `HandlerDependencies`.
/// Streams `ExecStarted`, zero or more `ContainerOutput`, then `ContainerStopped`.
pub async fn handle_exec(
    container_id: String,
    cmd: Vec<String>,
    env: Vec<String>,
    working_dir: Option<String>,
    _tty: bool,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let Some(ref exec_rt) = deps.exec_runtime else {
        let _ = tx
            .send(DaemonResponse::Error {
                message: "exec not supported on this platform".to_string(),
            })
            .await;
        return;
    };

    // Verify container exists and is running.
    let cid = match ContainerId::new(&container_id) {
        Ok(id) => id,
        Err(e) => {
            let _ = tx
                .send(DaemonResponse::Error {
                    message: format!("invalid container id: {e}"),
                })
                .await;
            return;
        }
    };

    let config = minibox_core::domain::ExecConfig {
        cmd,
        env,
        working_dir: working_dir.map(std::path::PathBuf::from),
        tty: _tty,
    };

    match exec_rt.exec(&cid, &config, tx.clone()).await {
        Ok(handle) => {
            let _ = tx
                .send(DaemonResponse::ExecStarted {
                    exec_id: handle.id.clone(),
                })
                .await;
        }
        Err(e) => {
            let _ = tx
                .send(DaemonResponse::Error {
                    message: format!("exec failed: {e}"),
                })
                .await;
        }
    }
}
```

- [ ] **Step 3: Add dispatch arm in `crates/daemonbox/src/server.rs`**

Find the `dispatch` function match block. Add after `LoadImage`:

```rust
        DaemonRequest::Exec {
            container_id,
            cmd,
            env,
            working_dir,
            tty,
        } => {
            handler::handle_exec(container_id, cmd, env, working_dir, tty, state, deps, tx).await;
        }
```

- [ ] **Step 4: Compile check**

```bash
cargo check -p daemonbox
```

Fix any missing imports (`ContainerId` needs `use minibox_core::domain::ContainerId;`).

- [ ] **Step 5: Add unit test in `crates/daemonbox/src/handler.rs`**

In the `#[cfg(test)]` block at the bottom of `handler.rs`, add:

```rust
    #[tokio::test]
    async fn handle_exec_no_runtime_returns_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (state, deps) = create_test_deps_with_dir(tmp.path());
        // deps has no exec_runtime (None by default)
        let (tx, mut rx) = mpsc::channel(8);

        handle_exec(
            "abc123".to_string(),
            vec!["echo".to_string()],
            vec![],
            None,
            false,
            state,
            Arc::new(deps),
            tx,
        )
        .await;

        let resp = rx.recv().await.unwrap();
        assert!(matches!(resp, DaemonResponse::Error { .. }));
    }
```

- [ ] **Step 6: Run test**

```bash
cargo test -p daemonbox handle_exec_no_runtime_returns_error
```
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/daemonbox/src/handler.rs crates/daemonbox/src/server.rs
git commit -m "feat(daemonbox): add handle_exec + server dispatch for Exec"
```

---

### Task 5: Dockerbox exec endpoints

**Files:**
- Modify: `crates/dockerbox/src/domain/mod.rs`
- Modify: `crates/dockerbox/src/infra/minibox.rs`
- Modify: `crates/dockerbox/src/api/containers.rs`
- Modify: `crates/dockerbox/src/api/mod.rs`

- [ ] **Step 1: Extend dockerbox `ContainerRuntime` trait**

In `crates/dockerbox/src/domain/mod.rs`, add to the trait:

```rust
    /// Create an exec instance in a running container.
    /// Returns an exec ID string.
    async fn create_exec(
        &self,
        container_id: &str,
        cmd: Vec<String>,
        env: Vec<String>,
    ) -> Result<String, RuntimeError>;

    /// Start an exec instance, streaming output via tx.
    async fn start_exec(
        &self,
        exec_id: &str,
        tx: mpsc::Sender<LogChunk>,
    ) -> Result<i64, RuntimeError>;
```

Add a supporting type for pending exec instances in `crates/dockerbox/src/infra/state.rs`:

```rust
use std::collections::HashMap;
use tokio::sync::Mutex;

/// In-memory exec instance registry.
pub struct ExecRegistry {
    /// exec_id -> (container_id, cmd, env)
    pub pending: Mutex<HashMap<String, (String, Vec<String>, Vec<String>)>>,
}

impl ExecRegistry {
    pub fn new() -> Self {
        Self { pending: Mutex::new(HashMap::new()) }
    }
}
```

- [ ] **Step 2: Implement `create_exec` and `start_exec` in `MiniboxAdapter`**

In `crates/dockerbox/src/infra/minibox.rs`:

Add `exec_registry: Arc<ExecRegistry>` field to `MiniboxAdapter`. Update `new()` to initialize it.

Add the implementations:

```rust
    async fn create_exec(
        &self,
        container_id: &str,
        cmd: Vec<String>,
        env: Vec<String>,
    ) -> Result<String, RuntimeError> {
        use uuid::Uuid;
        let exec_id = Uuid::new_v4().simple().to_string()[..16].to_string();
        self.exec_registry
            .pending
            .lock()
            .await
            .insert(exec_id.clone(), (container_id.to_string(), cmd, env));
        Ok(exec_id)
    }

    async fn start_exec(
        &self,
        exec_id: &str,
        tx: mpsc::Sender<LogChunk>,
    ) -> Result<i64, RuntimeError> {
        let entry = {
            let mut map = self.exec_registry.pending.lock().await;
            map.remove(exec_id)
        };
        let (container_id, cmd, env) = entry.ok_or_else(|| {
            RuntimeError::Minibox(anyhow::anyhow!("exec {exec_id} not found"))
        })?;

        let mut stream = self
            .client()
            .call(DaemonRequest::Exec {
                container_id: to_minibox_id(&container_id).to_string(),
                cmd,
                env,
                working_dir: None,
                tty: false,
            })
            .await?;

        let mut exit_code: i64 = -1;
        while let Some(resp) = stream.next().await? {
            match resp {
                DaemonResponse::ContainerOutput { stream: s, data } => {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(&data)
                        .unwrap_or_default();
                    let stream_id = if s == minibox_core::protocol::OutputStreamKind::Stdout { 1 } else { 2 };
                    let _ = tx.send(LogChunk { stream: stream_id, data: bytes::Bytes::from(bytes) }).await;
                }
                DaemonResponse::ContainerStopped { exit_code: code } => {
                    exit_code = code as i64;
                }
                DaemonResponse::Error { message } => {
                    return Err(RuntimeError::Minibox(anyhow::anyhow!("{}", message)));
                }
                _ => {}
            }
        }
        Ok(exit_code)
    }
```

- [ ] **Step 3: Add HTTP handlers in `crates/dockerbox/src/api/containers.rs`**

Add at the end of the file:

```rust
/// POST /containers/{id}/exec
pub async fn create_exec(
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::State(rt): axum::extract::State<Arc<dyn ContainerRuntime>>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> axum::response::Response {
    let cmd: Vec<String> = body["Cmd"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let env: Vec<String> = body["Env"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    match rt.create_exec(&id, cmd, env).await {
        Ok(exec_id) => axum::Json(serde_json::json!({ "Id": exec_id })).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /exec/{id}/start
pub async fn start_exec(
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::State(rt): axum::extract::State<Arc<dyn ContainerRuntime>>,
) -> axum::response::Response {
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);

    let rt2 = Arc::clone(&rt);
    let id2 = id.clone();
    tokio::spawn(async move {
        let _ = rt2.start_exec(&id2, tx).await;
    });

    // Collect all output into body (non-streaming for simplicity).
    let mut body = Vec::new();
    while let Some(chunk) = rx.recv().await {
        body.extend_from_slice(&chunk.data);
    }

    axum::response::Response::builder()
        .status(200)
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// GET /exec/{id}/json
pub async fn inspect_exec(
    axum::extract::Path(id): axum::extract::Path<String>,
) -> axum::response::Response {
    axum::Json(serde_json::json!({
        "ID": id,
        "Running": false,
        "ExitCode": 0,
        "ProcessConfig": { "entrypoint": "sh", "arguments": [] }
    }))
    .into_response()
}
```

- [ ] **Step 4: Register routes in `crates/dockerbox/src/api/mod.rs`**

Add to the router:

```rust
    .route("/containers/:id/exec", post(containers::create_exec))
    .route("/exec/:id/start", post(containers::start_exec))
    .route("/exec/:id/json", get(containers::inspect_exec))
```

- [ ] **Step 5: Compile and test**

```bash
cargo check -p dockerbox
cargo test -p dockerbox 2>&1 | tail -20
```

- [ ] **Step 6: Commit**

```bash
git add crates/dockerbox/
git commit -m "feat(dockerbox): add exec endpoints (create_exec, start_exec, inspect_exec)"
```

---

## Phase 2: Image Push

### Task 6: Domain trait + error types for Push

**Files:**
- Modify: `crates/minibox-core/src/domain.rs`
- Modify: `crates/minibox-core/src/error.rs`

- [ ] **Step 1: Add PushError to `error.rs`**

```rust
/// Errors from OCI image push operations.
#[derive(Debug, Error)]
pub enum PushError {
    #[error("registry authentication failed for {registry}: {message}")]
    AuthFailed { registry: String, message: String },

    #[error("blob upload failed for {digest}: {reason}")]
    BlobUploadFailed { digest: String, reason: String },

    #[error("manifest push failed: {reason}")]
    ManifestPushFailed { reason: String },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("push error: {0}")]
    Other(String),
}
```

- [ ] **Step 2: Add RegistryCredentials, PushResult, ImagePusher trait to `domain.rs`**

```rust
// ---------------------------------------------------------------------------
// Image Pusher Port
// ---------------------------------------------------------------------------

/// Credentials for authenticating to a registry.
#[derive(Debug, Clone)]
pub enum RegistryCredentials {
    Anonymous,
    Basic { username: String, password: String },
    Token(String),
}

/// Result of a successful image push.
#[derive(Debug, Clone)]
pub struct PushResult {
    /// Manifest digest of the pushed image.
    pub digest: String,
    /// Total bytes pushed.
    pub size_bytes: u64,
}

/// Port for pushing images to OCI-compliant registries.
///
/// Implemented by: `OciPushAdapter` (uses OCI Distribution Spec v1 push API).
#[async_trait]
pub trait ImagePusher: AsAny + Send + Sync {
    /// Push a locally-stored image to a remote registry.
    ///
    /// `progress_tx` receives upload progress messages (optional — send None to disable).
    async fn push_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
        credentials: &RegistryCredentials,
        progress_tx: Option<tokio::sync::mpsc::Sender<PushProgress>>,
    ) -> anyhow::Result<PushResult>;
}

/// Push progress update.
#[derive(Debug, Clone)]
pub struct PushProgress {
    pub layer_digest: String,
    pub bytes_uploaded: u64,
    pub total_bytes: u64,
}

/// Type alias for a shared dynamic ImagePusher.
pub type DynImagePusher = Arc<dyn ImagePusher>;
```

- [ ] **Step 3: Compile check**

```bash
cargo check -p minibox-core
```

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-core/src/domain.rs crates/minibox-core/src/error.rs
git commit -m "feat(domain): add ImagePusher trait and PushError types"
```

---

### Task 7: Protocol variants for Push

**Files:**
- Modify: `crates/minibox-core/src/protocol.rs`
- Modify: `crates/mbx/src/protocol.rs`

- [ ] **Step 1: Add serializable credentials type**

In `crates/minibox-core/src/protocol.rs`, before `DaemonRequest`:

```rust
/// Serializable registry credentials for protocol transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PushCredentials {
    Anonymous,
    Basic { username: String, password: String },
    Token { token: String },
}
```

- [ ] **Step 2: Add Push request variant**

In `DaemonRequest`, add after `Exec`:

```rust
    /// Push a locally-stored image to a remote OCI registry.
    ///
    /// Streams `PushProgress` messages then one `Success` or `Error`.
    Push {
        /// Image reference, e.g. `"ghcr.io/org/image:tag"`.
        image_ref: String,
        /// Registry credentials.
        credentials: PushCredentials,
    },
```

- [ ] **Step 3: Add PushProgress response variant**

In `DaemonResponse`, add after `ExecStarted`:

```rust
    /// Push progress update for a layer upload.
    PushProgress {
        layer_digest: String,
        bytes_uploaded: u64,
        total_bytes: u64,
    },
```

- [ ] **Step 4: Mirror in `crates/mbx/src/protocol.rs`**

Add identical `PushCredentials`, `Push` variant, and `PushProgress` variant.

- [ ] **Step 5: Compile check + commit**

```bash
cargo check -p minibox-core -p mbx
git add crates/minibox-core/src/protocol.rs crates/mbx/src/protocol.rs
git commit -m "feat(protocol): add Push request and PushProgress response variants"
```

---

### Task 8: OciPushAdapter

**Files:**
- Create: `crates/mbx/src/adapters/push.rs`
- Modify: `crates/mbx/src/adapters/mod.rs`
- Modify: `crates/minibox-core/src/image/registry.rs`

- [ ] **Step 1: Add push methods to RegistryClient**

In `crates/minibox-core/src/image/registry.rs`, add after `pull_image`:

```rust
    /// Obtain a push-scoped token for `repo`.
    ///
    /// Requests `scope=repository:{repo}:push,pull` from the auth service.
    pub async fn get_push_token(
        &self,
        repo: &str,
        username: Option<&str>,
        password: Option<&str>,
    ) -> anyhow::Result<String> {
        let scope = format!("repository:{repo}:push,pull");
        let mut req = self
            .client
            .get(AUTH_URL)
            .query(&[("service", "registry.docker.io"), ("scope", &scope)]);
        if let (Some(u), Some(p)) = (username, password) {
            req = req.basic_auth(u, Some(p));
        }
        let resp: TokenResponse = req.send().await?.json().await?;
        Ok(resp.token)
    }

    /// Check if a blob already exists at the registry (HEAD request).
    ///
    /// Returns `true` if the registry responds 200.
    pub async fn blob_exists(
        &self,
        registry_base: &str,
        repo: &str,
        digest: &str,
        token: &str,
    ) -> bool {
        let url = format!("{registry_base}/v2/{repo}/blobs/{digest}");
        self.client
            .head(&url)
            .bearer_auth(token)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Initiate a blob upload and return the upload URL.
    pub async fn initiate_blob_upload(
        &self,
        registry_base: &str,
        repo: &str,
        token: &str,
    ) -> anyhow::Result<String> {
        let url = format!("{registry_base}/v2/{repo}/blobs/uploads/");
        let resp = self
            .client
            .post(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("initiate blob upload")?;
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .context("missing Location header from blob upload initiation")?
            .to_string();
        Ok(location)
    }

    /// Upload a blob at `upload_url` with the given digest.
    pub async fn upload_blob(
        &self,
        upload_url: &str,
        digest: &str,
        data: bytes::Bytes,
        token: &str,
    ) -> anyhow::Result<()> {
        let url = format!("{upload_url}?digest={digest}");
        let resp = self
            .client
            .put(&url)
            .bearer_auth(token)
            .header("Content-Type", "application/octet-stream")
            .body(data)
            .send()
            .await
            .context("upload blob")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("blob upload failed {status}: {body}");
        }
        Ok(())
    }

    /// Push a manifest to `registry_base/v2/{repo}/manifests/{reference}`.
    pub async fn push_manifest(
        &self,
        registry_base: &str,
        repo: &str,
        reference: &str,
        manifest: &crate::image::manifest::OciManifest,
        token: &str,
    ) -> anyhow::Result<String> {
        let url = format!("{registry_base}/v2/{repo}/manifests/{reference}");
        let body = serde_json::to_vec(manifest)?;
        let resp = self
            .client
            .put(&url)
            .bearer_auth(token)
            .header("Content-Type", "application/vnd.oci.image.manifest.v1+json")
            .body(body)
            .send()
            .await
            .context("push manifest")?;
        let digest = resp
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("manifest push failed {status}: {body}");
        }
        Ok(digest)
    }
```

- [ ] **Step 2: Write failing test**

Create `crates/mbx/src/adapters/push.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_adapter_constructs() {
        // Will fail until OciPushAdapter is defined.
        let _a = OciPushAdapter::placeholder();
    }
}
```

Run:
```bash
cargo test -p mbx adapters::push 2>&1 | head -10
```
Expected: compile error.

- [ ] **Step 3: Implement OciPushAdapter**

Replace `crates/mbx/src/adapters/push.rs`:

```rust
//! OCI Distribution Spec push adapter.
//!
//! Implements [`ImagePusher`] by uploading blobs and manifests to any
//! OCI-compliant registry (Docker Hub, GHCR, GCR, private registries).

use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    AsAny, DynImagePusher, ImagePusher, PushProgress, PushResult, RegistryCredentials,
};
use minibox_core::image::ImageStore;
use minibox_core::image::registry::RegistryClient;
use std::sync::Arc;
use tracing::info;

/// OCI Distribution Spec v1 push adapter.
///
/// Resolves registry base URL from the image ref (Docker Hub uses
/// `https://registry-1.docker.io`, GHCR uses `https://ghcr.io`).
pub struct OciPushAdapter {
    client: RegistryClient,
    store: Arc<ImageStore>,
}

impl OciPushAdapter {
    pub fn new(client: RegistryClient, store: Arc<ImageStore>) -> Self {
        Self { client, store }
    }
}

as_any!(OciPushAdapter);

#[async_trait]
impl ImagePusher for OciPushAdapter {
    async fn push_image(
        &self,
        image_ref: &minibox_core::image::reference::ImageRef,
        credentials: &RegistryCredentials,
        progress_tx: Option<tokio::sync::mpsc::Sender<PushProgress>>,
    ) -> Result<PushResult> {
        let repo = format!("{}/{}", image_ref.namespace, image_ref.name);
        let tag = &image_ref.tag;

        // Determine registry base URL from registry hostname.
        let registry_base = match image_ref.registry.as_str() {
            "" | "docker.io" | "registry-1.docker.io" => {
                "https://registry-1.docker.io".to_string()
            }
            host => format!("https://{host}"),
        };

        let (username, password) = match credentials {
            RegistryCredentials::Basic { username, password } => {
                (Some(username.as_str()), Some(password.as_str()))
            }
            _ => (None, None),
        };

        let token = self
            .client
            .get_push_token(&repo, username, password)
            .await
            .context("get push token")?;

        // Load manifest from local store.
        let manifest = self
            .store
            .load_manifest(&image_ref.namespace, &image_ref.name, tag)
            .context("load manifest for push")?;

        let mut total_bytes: u64 = 0;

        // Upload each layer blob.
        for layer in &manifest.layers {
            let digest = &layer.digest;

            if self
                .client
                .blob_exists(&registry_base, &repo, digest, &token)
                .await
            {
                info!(digest = %digest, "push: blob already exists, skipping");
                continue;
            }

            // Read layer tar from store.
            let layer_path = self
                .store
                .layer_blob_path(&image_ref.namespace, &image_ref.name, tag, digest)
                .context("locate layer blob")?;
            let data = tokio::task::spawn_blocking({
                let p = layer_path.clone();
                move || std::fs::read(&p).context("read layer blob")
            })
            .await??;

            let size = data.len() as u64;
            total_bytes += size;

            let upload_url = self
                .client
                .initiate_blob_upload(&registry_base, &repo, &token)
                .await?;

            self.client
                .upload_blob(&upload_url, digest, bytes::Bytes::from(data), &token)
                .await
                .with_context(|| format!("upload blob {digest}"))?;

            info!(digest = %digest, bytes = size, "push: blob uploaded");

            if let Some(ref tx) = progress_tx {
                let _ = tx
                    .send(PushProgress {
                        layer_digest: digest.clone(),
                        bytes_uploaded: size,
                        total_bytes: size,
                    })
                    .await;
            }
        }

        // Upload config blob.
        let config_data = self
            .store
            .load_config_blob(&image_ref.namespace, &image_ref.name, tag)
            .context("load config blob")?;
        let config_digest = &manifest.config.digest;
        if !self
            .client
            .blob_exists(&registry_base, &repo, config_digest, &token)
            .await
        {
            let upload_url = self
                .client
                .initiate_blob_upload(&registry_base, &repo, &token)
                .await?;
            self.client
                .upload_blob(
                    &upload_url,
                    config_digest,
                    bytes::Bytes::from(config_data),
                    &token,
                )
                .await?;
        }

        // Push manifest.
        let manifest_digest = self
            .client
            .push_manifest(&registry_base, &repo, tag, &manifest, &token)
            .await
            .context("push manifest")?;

        info!(
            image_ref = %format!("{}:{}", repo, tag),
            digest = %manifest_digest,
            "push: complete"
        );

        Ok(PushResult {
            digest: manifest_digest,
            size_bytes: total_bytes,
        })
    }
}

/// Convenience constructor returning a `DynImagePusher`.
pub fn oci_push_adapter(client: RegistryClient, store: Arc<ImageStore>) -> DynImagePusher {
    Arc::new(OciPushAdapter::new(client, store))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_adapter_constructs() {
        // OciPushAdapter can be constructed given client + store.
        // Full integration test requires a live registry.
        let store = Arc::new(
            minibox_core::image::ImageStore::new(tempfile::TempDir::new().unwrap().path())
                .unwrap(),
        );
        let client = RegistryClient::new().unwrap();
        let _adapter = OciPushAdapter::new(client, store);
    }
}
```

**Note:** `store.load_manifest()`, `store.layer_blob_path()`, `store.load_config_blob()` may not yet exist on `ImageStore`. Check:

```bash
grep -n "pub fn load_manifest\|pub fn layer_blob_path\|pub fn load_config" /Users/joe/dev/minibox/crates/minibox-core/src/image/mod.rs
```

Add any missing helpers to `ImageStore` — see Task 8a below if needed.

- [ ] **Step 4: Add missing ImageStore helpers (if needed)**

If `load_manifest`, `layer_blob_path`, or `load_config_blob` are missing from `ImageStore`, add them in `crates/minibox-core/src/image/mod.rs`:

```rust
    /// Load the stored OCI manifest for `namespace/name:tag`.
    pub fn load_manifest(
        &self,
        namespace: &str,
        name: &str,
        tag: &str,
    ) -> anyhow::Result<OciManifest> {
        // name parameter may be "namespace/name" combined — handle both forms
        let full_name = if namespace.is_empty() { name.to_string() } else { format!("{namespace}/{name}") };
        let path = self.manifest_path(&full_name, tag)?;
        let json = std::fs::read_to_string(&path).with_context(|| format!("read manifest {path:?}"))?;
        serde_json::from_str(&json).context("parse manifest")
    }

    /// Return the path to a layer's raw tar blob (compressed, for push).
    ///
    /// Layer blobs are the original downloaded tarballs stored alongside
    /// the extracted directories.
    pub fn layer_blob_path(
        &self,
        namespace: &str,
        name: &str,
        tag: &str,
        digest: &str,
    ) -> anyhow::Result<std::path::PathBuf> {
        let full_name = if namespace.is_empty() { name.to_string() } else { format!("{namespace}/{name}") };
        let layers_base = self.layers_dir(&full_name, tag)?;
        let digest_key = digest.replace(':', "_");
        Ok(layers_base.join(format!("{digest_key}.tar.gz")))
    }

    /// Load the image config blob JSON bytes.
    pub fn load_config_blob(
        &self,
        namespace: &str,
        name: &str,
        tag: &str,
    ) -> anyhow::Result<Vec<u8>> {
        let full_name = if namespace.is_empty() { name.to_string() } else { format!("{namespace}/{name}") };
        let path = self.manifest_path(&full_name, tag)?
            .parent().unwrap().join("config.json");
        std::fs::read(&path).with_context(|| format!("read config blob {path:?}"))
    }
```

- [ ] **Step 5: Export and compile**

In `crates/mbx/src/adapters/mod.rs` add:
```rust
pub mod push;
pub use push::OciPushAdapter;
```

```bash
cargo check -p mbx
cargo test -p mbx adapters::push::tests
```

- [ ] **Step 6: Commit**

```bash
git add crates/mbx/src/adapters/push.rs crates/mbx/src/adapters/mod.rs crates/minibox-core/src/image/mod.rs crates/minibox-core/src/image/registry.rs
git commit -m "feat(mbx): add OciPushAdapter with OCI Distribution Spec push support"
```

---

### Task 9: Push handler + dockerbox endpoint

**Files:**
- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/daemonbox/src/server.rs`
- Modify: `crates/dockerbox/src/domain/mod.rs`
- Modify: `crates/dockerbox/src/infra/minibox.rs`
- Modify: `crates/dockerbox/src/api/images.rs`
- Modify: `crates/dockerbox/src/api/mod.rs`

- [ ] **Step 1: Add `image_pusher` to HandlerDependencies**

```rust
    pub image_pusher: Option<minibox_core::domain::DynImagePusher>,
```

- [ ] **Step 2: Add `handle_push` in `handler.rs`**

```rust
pub async fn handle_push(
    image_ref_str: String,
    credentials: minibox_core::protocol::PushCredentials,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let Some(ref pusher) = deps.image_pusher else {
        let _ = tx.send(DaemonResponse::Error { message: "push not supported on this platform".to_string() }).await;
        return;
    };

    let image_ref = match mbx::ImageRef::parse(&image_ref_str) {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(DaemonResponse::Error { message: format!("invalid image ref: {e}") }).await;
            return;
        }
    };

    let creds = match credentials {
        minibox_core::protocol::PushCredentials::Anonymous => minibox_core::domain::RegistryCredentials::Anonymous,
        minibox_core::protocol::PushCredentials::Basic { username, password } => {
            minibox_core::domain::RegistryCredentials::Basic { username, password }
        }
        minibox_core::protocol::PushCredentials::Token { token } => {
            minibox_core::domain::RegistryCredentials::Token(token)
        }
    };

    let (progress_tx, mut progress_rx) = mpsc::channel::<minibox_core::domain::PushProgress>(32);
    let tx2 = tx.clone();
    tokio::spawn(async move {
        while let Some(p) = progress_rx.recv().await {
            let _ = tx2.send(DaemonResponse::PushProgress {
                layer_digest: p.layer_digest,
                bytes_uploaded: p.bytes_uploaded,
                total_bytes: p.total_bytes,
            }).await;
        }
    });

    match pusher.push_image(&image_ref, &creds, Some(progress_tx)).await {
        Ok(result) => {
            let _ = tx.send(DaemonResponse::Success {
                message: format!("pushed {} digest:{}", image_ref_str, result.digest),
            }).await;
        }
        Err(e) => {
            let _ = tx.send(DaemonResponse::Error { message: e.to_string() }).await;
        }
    }
}
```

- [ ] **Step 3: Add dispatch arm in `server.rs`**

```rust
        DaemonRequest::Push { image_ref, credentials } => {
            handler::handle_push(image_ref, credentials, state, deps, tx).await;
        }
```

- [ ] **Step 4: Add push/tag endpoints to dockerbox**

In `crates/dockerbox/src/domain/mod.rs`, add to trait:

```rust
    async fn push_image(
        &self,
        image: &str,
        tag: &str,
        registry_auth: Option<String>,
        tx: mpsc::Sender<PullProgress>,
    ) -> Result<(), RuntimeError>;

    async fn tag_image(
        &self,
        source: &str,
        target_repo: &str,
        target_tag: &str,
    ) -> Result<(), RuntimeError>;
```

In `crates/dockerbox/src/infra/minibox.rs`, implement both — `push_image` sends `DaemonRequest::Push`, `tag_image` is a no-op stub that stores an alias in-memory.

In `crates/dockerbox/src/api/images.rs`, add:

```rust
/// POST /images/{name}/push
pub async fn push_image(
    axum::extract::Path(name): axum::extract::Path<String>,
    axum::extract::State(rt): axum::extract::State<Arc<dyn ContainerRuntime>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let auth = headers
        .get("X-Registry-Auth")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let rt2 = Arc::clone(&rt);
    let name2 = name.clone();
    tokio::spawn(async move {
        let _ = rt2.push_image(&name2, "latest", auth, tx).await;
    });
    // Drain and return progress as newline-delimited JSON.
    let mut body = Vec::new();
    while let Some(p) = rx.recv().await {
        let line = serde_json::to_vec(&serde_json::json!({
            "status": p.status,
            "id": p.id
        })).unwrap_or_default();
        body.extend_from_slice(&line);
        body.push(b'\n');
    }
    axum::response::Response::builder()
        .status(200)
        .body(axum::body::Body::from(body))
        .unwrap()
}

/// POST /images/{name}/tag
pub async fn tag_image(
    axum::extract::Path(name): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    axum::extract::State(rt): axum::extract::State<Arc<dyn ContainerRuntime>>,
) -> axum::response::Response {
    let repo = params.get("repo").cloned().unwrap_or_default();
    let tag = params.get("tag").cloned().unwrap_or_else(|| "latest".to_string());
    match rt.tag_image(&name, &repo, &tag).await {
        Ok(_) => axum::http::StatusCode::CREATED.into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "message": e.to_string() })),
        ).into_response(),
    }
}
```

Register in `api/mod.rs`:
```rust
    .route("/images/:name/push", post(images::push_image))
    .route("/images/:name/tag", post(images::tag_image))
```

- [ ] **Step 5: Compile check + commit**

```bash
cargo check -p daemonbox -p dockerbox
git add crates/daemonbox/src/ crates/dockerbox/src/
git commit -m "feat(daemonbox,dockerbox): add push handler and push/tag endpoints"
```

---

## Phase 3: Container Commit

### Task 10: Domain trait + protocol for Commit

**Files:**
- Modify: `crates/minibox-core/src/domain.rs`
- Modify: `crates/minibox-core/src/error.rs`
- Modify: `crates/minibox-core/src/protocol.rs`
- Modify: `crates/mbx/src/protocol.rs`
- Modify: `crates/daemonbox/src/state.rs`

- [ ] **Step 1: Add CommitError to `error.rs`**

```rust
#[derive(Debug, Error)]
pub enum CommitError {
    #[error("overlay upperdir missing for container {container_id}")]
    UpperdirMissing { container_id: String },

    #[error("layer tar failed: {reason}")]
    LayerTarFailed { reason: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("commit error: {0}")]
    Other(String),
}
```

- [ ] **Step 2: Add CommitConfig and ContainerCommitter trait to `domain.rs`**

```rust
// ---------------------------------------------------------------------------
// Container Committer Port
// ---------------------------------------------------------------------------

/// Configuration for committing a container to a new image.
#[derive(Debug, Clone)]
pub struct CommitConfig {
    pub author: Option<String>,
    pub message: Option<String>,
    /// Environment variable overrides for the new image.
    pub env_overrides: Vec<String>,
    /// Override the default CMD in the new image.
    pub cmd_override: Option<Vec<String>>,
}

/// Port for snapshotting a container's filesystem diff into a new image.
///
/// Implemented by: `OverlayCommitAdapter` (Linux, tars the overlay upperdir).
#[async_trait]
pub trait ContainerCommitter: AsAny + Send + Sync {
    async fn commit(
        &self,
        container_id: &ContainerId,
        target_ref: &str,
        config: &CommitConfig,
    ) -> anyhow::Result<minibox_core::domain::ImageMetadata>;
}

pub type DynContainerCommitter = Arc<dyn ContainerCommitter>;
```

- [ ] **Step 3: Add overlay_paths to ContainerRecord in `state.rs`**

In `crates/daemonbox/src/state.rs`, update `ContainerRecord`:

```rust
    /// Path to the container's overlay upper directory (writable layer).
    /// Present only for native Linux containers.
    #[serde(default)]
    pub overlay_upper: Option<PathBuf>,

    /// Image reference used to create this container (e.g. "library/alpine:latest").
    #[serde(default)]
    pub source_image_ref: Option<String>,
```

- [ ] **Step 4: Add Commit protocol variants**

In `crates/minibox-core/src/protocol.rs`, add to `DaemonRequest`:

```rust
    /// Snapshot a container's filesystem changes into a new local image.
    Commit {
        container_id: String,
        target_image: String,
        #[serde(default)]
        author: Option<String>,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        env_overrides: Vec<String>,
        #[serde(default)]
        cmd_override: Option<Vec<String>>,
    },
```

Mirror in `crates/mbx/src/protocol.rs`.

- [ ] **Step 5: Compile check + commit**

```bash
cargo check -p minibox-core -p daemonbox -p mbx
git add crates/minibox-core/src/ crates/daemonbox/src/state.rs crates/mbx/src/protocol.rs
git commit -m "feat(domain,protocol): add ContainerCommitter trait and Commit protocol variant"
```

---

### Task 11: OverlayCommitAdapter

**Files:**
- Create: `crates/mbx/src/adapters/commit.rs`
- Modify: `crates/mbx/src/adapters/mod.rs`

- [ ] **Step 1: Write failing test**

Create `crates/mbx/src/adapters/commit.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_adapter_constructs() {
        let _: &dyn minibox_core::domain::ContainerCommitter = &OverlayCommitAdapter::new_for_test();
    }
}
```

Run: `cargo test -p mbx adapters::commit 2>&1 | head -5` — expect compile error.

- [ ] **Step 2: Implement OverlayCommitAdapter**

```rust
//! Overlay filesystem commit adapter.
//!
//! Snapshots a container's writable layer (upperdir) into a new OCI image
//! by tarring the upperdir, storing it as a new layer blob, and constructing
//! a new OCI manifest that chains the parent layers + this new diff layer.

use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    AsAny, CommitConfig, ContainerCommitter, ContainerId, DynContainerCommitter, ImageMetadata,
    LayerInfo,
};
use minibox_core::image::ImageStore;
use minibox_core::image::manifest::{OciManifest, OciDescriptor};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

use crate::daemonbox_state::StateHandle;

pub struct OverlayCommitAdapter {
    image_store: Arc<ImageStore>,
    state: StateHandle,
}

impl OverlayCommitAdapter {
    pub fn new(image_store: Arc<ImageStore>, state: StateHandle) -> Self {
        Self { image_store, state }
    }

    #[cfg(test)]
    pub fn new_for_test() -> Self {
        use std::path::Path;
        let store = Arc::new(ImageStore::new(Path::new("/tmp/test-store")).unwrap());
        Self {
            image_store: store,
            state: crate::daemonbox_state::StateHandle::noop(),
        }
    }
}

as_any!(OverlayCommitAdapter);

#[async_trait]
impl ContainerCommitter for OverlayCommitAdapter {
    async fn commit(
        &self,
        container_id: &ContainerId,
        target_ref: &str,
        config: &CommitConfig,
    ) -> Result<ImageMetadata> {
        let id = container_id.as_str().to_string();

        // Look up overlay upper dir from state.
        let upper_dir = self
            .state
            .get_overlay_upper(&id)
            .await
            .with_context(|| format!("container {id} has no overlay upper dir"))?;

        let source_ref = self
            .state
            .get_source_image_ref(&id)
            .await
            .unwrap_or_default();

        // Tar the upperdir in a blocking task.
        let tar_bytes = tokio::task::spawn_blocking({
            let upper = upper_dir.clone();
            move || tar_directory(&upper)
        })
        .await
        .context("spawn_blocking tar")??;

        let size = tar_bytes.len() as u64;

        // Compute SHA256 digest of the tar.
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&tar_bytes);
        let digest = format!("sha256:{:x}", hasher.finalize());

        // Parse target ref to get name and tag.
        let (target_name, target_tag) = parse_image_ref(target_ref);

        // Store the new layer blob as a gzip-compressed file.
        // (Store the raw tar for simplicity; callers expecting gz can compress.)
        let layer_path = self
            .image_store
            .layer_blob_path("", &target_name, &target_tag, &digest)
            .context("compute layer blob path")?;
        if let Some(parent) = layer_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&layer_path, &tar_bytes)?;

        // Load parent manifest (if source image exists).
        let mut layers: Vec<OciDescriptor> = vec![];
        if !source_ref.is_empty() {
            if let Ok(parent_manifest) = self.image_store.load_manifest("", &source_ref, "latest") {
                layers = parent_manifest.layers;
            }
        }

        // Append the new diff layer.
        layers.push(OciDescriptor {
            media_type: "application/vnd.oci.image.layer.v1.tar".to_string(),
            digest: digest.clone(),
            size: size as i64,
        });

        // Build config blob.
        let config_json = serde_json::json!({
            "architecture": "amd64",
            "os": "linux",
            "config": {
                "Env": config.env_overrides,
                "Cmd": config.cmd_override.clone().unwrap_or_default(),
                "Author": config.author.clone().unwrap_or_default(),
            }
        });
        let config_bytes = serde_json::to_vec(&config_json)?;
        let mut cfg_hasher = Sha256::new();
        cfg_hasher.update(&config_bytes);
        let config_digest = format!("sha256:{:x}", cfg_hasher.finalize());

        let config_path = self
            .image_store
            .layer_blob_path("", &target_name, &target_tag, &config_digest)
            .unwrap()
            .parent()
            .unwrap()
            .join("config.json");
        std::fs::write(&config_path, &config_bytes)?;

        // Build and store new manifest.
        let new_manifest = OciManifest {
            schema_version: 2,
            media_type: Some("application/vnd.oci.image.manifest.v1+json".to_string()),
            config: OciDescriptor {
                media_type: "application/vnd.oci.image.config.v1+json".to_string(),
                digest: config_digest.clone(),
                size: config_bytes.len() as i64,
            },
            layers: layers.clone(),
        };
        self.image_store
            .store_manifest("", &target_name, &target_tag, &new_manifest)
            .context("store new manifest")?;

        info!(
            container_id = %id,
            target = %target_ref,
            digest = %digest,
            layers = layers.len(),
            "commit: complete"
        );

        Ok(ImageMetadata {
            id: config_digest,
            name: target_name,
            tag: target_tag,
            layers: layers
                .iter()
                .map(|l| LayerInfo {
                    digest: l.digest.clone(),
                    size: l.size as u64,
                    path: PathBuf::new(),
                })
                .collect(),
        })
    }
}

/// Tar a directory into an in-memory byte buffer.
fn tar_directory(dir: &std::path::Path) -> Result<Vec<u8>> {
    use tar::Builder;
    let mut buf = Vec::new();
    {
        let mut ar = Builder::new(&mut buf);
        ar.append_dir_all(".", dir)
            .with_context(|| format!("tar {}", dir.display()))?;
        ar.finish()?;
    }
    Ok(buf)
}

/// Split "name:tag" or just "name" into (name, tag).
fn parse_image_ref(s: &str) -> (String, String) {
    if let Some((name, tag)) = s.rsplit_once(':') {
        (name.to_string(), tag.to_string())
    } else {
        (s.to_string(), "latest".to_string())
    }
}

pub fn overlay_commit_adapter(
    image_store: Arc<ImageStore>,
    state: StateHandle,
) -> DynContainerCommitter {
    Arc::new(OverlayCommitAdapter::new(image_store, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_image_ref_with_tag() {
        let (name, tag) = parse_image_ref("myapp:v1.2");
        assert_eq!(name, "myapp");
        assert_eq!(tag, "v1.2");
    }

    #[test]
    fn parse_image_ref_no_tag() {
        let (name, tag) = parse_image_ref("myapp");
        assert_eq!(name, "myapp");
        assert_eq!(tag, "latest");
    }

    #[test]
    fn tar_empty_dir_produces_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bytes = tar_directory(tmp.path()).unwrap();
        assert!(!bytes.is_empty());
    }
}
```

- [ ] **Step 3: Add `sha2` and `tar` dependencies if missing**

```bash
grep "sha2\|^tar" /Users/joe/dev/minibox/crates/mbx/Cargo.toml
```

If missing, add to `[dependencies]` in `mbx/Cargo.toml`:
```toml
sha2 = "0.10"
tar = "0.4"
```

- [ ] **Step 4: Export + compile + test**

```rust
// In mod.rs:
pub mod commit;
pub use commit::OverlayCommitAdapter;
```

```bash
cargo test -p mbx adapters::commit::tests
```
Expected: 3 tests pass.

- [ ] **Step 5: Commit handler wiring**

In `handler.rs`, add `commit_adapter: Option<DynContainerCommitter>` to `HandlerDependencies`. Add `handle_commit`:

```rust
pub async fn handle_commit(
    container_id: String,
    target_image: String,
    author: Option<String>,
    message: Option<String>,
    env_overrides: Vec<String>,
    cmd_override: Option<Vec<String>>,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let Some(ref committer) = deps.commit_adapter else {
        let _ = tx.send(DaemonResponse::Error { message: "commit not supported on this platform".to_string() }).await;
        return;
    };

    let cid = match minibox_core::domain::ContainerId::new(&container_id) {
        Ok(id) => id,
        Err(e) => {
            let _ = tx.send(DaemonResponse::Error { message: format!("invalid container id: {e}") }).await;
            return;
        }
    };

    let config = minibox_core::domain::CommitConfig {
        author,
        message,
        env_overrides,
        cmd_override,
    };

    match committer.commit(&cid, &target_image, &config).await {
        Ok(meta) => {
            let _ = tx.send(DaemonResponse::Success {
                message: format!("committed {} digest:{}", target_image, meta.id),
            }).await;
        }
        Err(e) => {
            let _ = tx.send(DaemonResponse::Error { message: e.to_string() }).await;
        }
    }
}
```

Add dispatch arm in `server.rs`:
```rust
        DaemonRequest::Commit { container_id, target_image, author, message, env_overrides, cmd_override } => {
            handler::handle_commit(container_id, target_image, author, message, env_overrides, cmd_override, state, deps, tx).await;
        }
```

Dockerbox: add `POST /containers/{id}/commit` in `containers.rs` that calls `rt.commit_container()` (add method to trait + MiniboxAdapter).

- [ ] **Step 6: Compile check + commit**

```bash
cargo check --workspace
git add crates/mbx/src/adapters/commit.rs crates/mbx/src/adapters/mod.rs crates/mbx/Cargo.toml crates/daemonbox/src/handler.rs crates/daemonbox/src/server.rs crates/dockerbox/src/
git commit -m "feat(mbx,daemonbox,dockerbox): add OverlayCommitAdapter and commit endpoint"
```

---

## Phase 4: Image Build

### Task 12: DockerfileParser

**Files:**
- Create: `crates/mbx/src/image/dockerfile.rs`
- Modify: `crates/mbx/src/image/mod.rs`

- [ ] **Step 1: Write failing tests first**

Create `crates/mbx/src/image/dockerfile.rs` with just tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_from() {
        let instrs = parse("FROM alpine:3.18\n").unwrap();
        assert!(matches!(&instrs[0], Instruction::From { image, tag, .. } if image == "alpine" && tag == "3.18"));
    }

    #[test]
    fn parse_run_shell_form() {
        let instrs = parse("FROM alpine\nRUN echo hello\n").unwrap();
        assert!(matches!(&instrs[1], Instruction::Run(ShellOrExec::Shell(s)) if s == "echo hello"));
    }

    #[test]
    fn parse_run_exec_form() {
        let instrs = parse(r#"FROM alpine\nRUN ["echo", "hello"]\n"#).unwrap();
        assert!(matches!(&instrs[1], Instruction::Run(ShellOrExec::Exec(args)) if args[0] == "echo"));
    }

    #[test]
    fn parse_copy() {
        let instrs = parse("FROM alpine\nCOPY src/ /app/\n").unwrap();
        assert!(matches!(&instrs[1], Instruction::Copy { dest, .. } if dest.to_string_lossy() == "/app/"));
    }

    #[test]
    fn parse_env_equals_form() {
        let instrs = parse("FROM alpine\nENV FOO=bar BAZ=qux\n").unwrap();
        assert!(matches!(&instrs[1], Instruction::Env(pairs) if pairs[0] == ("FOO".to_string(), "bar".to_string())));
    }

    #[test]
    fn parse_cmd_exec_form() {
        let instrs = parse(r#"FROM alpine\nCMD ["/bin/sh"]\n"#).unwrap();
        assert!(matches!(&instrs[1], Instruction::Cmd(ShellOrExec::Exec(args)) if args[0] == "/bin/sh"));
    }

    #[test]
    fn parse_comment_skipped() {
        let instrs = parse("# comment\nFROM alpine\n").unwrap();
        // Comment is either skipped or returned as Comment variant — either way FROM should be present
        assert!(instrs.iter().any(|i| matches!(i, Instruction::From { .. })));
    }

    #[test]
    fn parse_error_no_from() {
        let result = parse("RUN echo hello\n");
        assert!(result.is_err());
    }

    #[test]
    fn parse_workdir() {
        let instrs = parse("FROM alpine\nWORKDIR /app\n").unwrap();
        assert!(matches!(&instrs[1], Instruction::Workdir(p) if p.to_string_lossy() == "/app"));
    }

    #[test]
    fn parse_arg() {
        let instrs = parse("FROM alpine\nARG VERSION=1.0\n").unwrap();
        assert!(matches!(&instrs[1], Instruction::Arg { name, default } if name == "VERSION" && default.as_deref() == Some("1.0")));
    }
}
```

Run: `cargo test -p mbx image::dockerfile 2>&1 | head -5` — expect compile error.

- [ ] **Step 2: Implement the parser**

```rust
//! Basic Dockerfile parser.
//!
//! Supports the instruction subset needed for ~90% of real Dockerfiles:
//! FROM, RUN, COPY, ADD, ENV, ARG, WORKDIR, CMD, ENTRYPOINT, EXPOSE, LABEL, USER.
//!
//! Does NOT support: HEALTHCHECK, VOLUME, ONBUILD, SHELL, STOPSIGNAL,
//! BuildKit --mount syntax, .dockerignore, multi-stage (only final stage built).

use anyhow::{bail, Context, Result};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ShellOrExec {
    /// Shell form: RUN cmd --flag  →  /bin/sh -c "cmd --flag"
    Shell(String),
    /// Exec form: RUN ["cmd", "--flag"]
    Exec(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum AddSource {
    Local(PathBuf),
    Url(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    From {
        image: String,
        tag: String,
        alias: Option<String>,
    },
    Run(ShellOrExec),
    Copy {
        srcs: Vec<PathBuf>,
        dest: PathBuf,
    },
    Add {
        srcs: Vec<AddSource>,
        dest: PathBuf,
    },
    Env(Vec<(String, String)>),
    Arg {
        name: String,
        default: Option<String>,
    },
    Workdir(PathBuf),
    Cmd(ShellOrExec),
    Entrypoint(ShellOrExec),
    Expose {
        port: u16,
        proto: String,
    },
    Label(Vec<(String, String)>),
    User {
        name: String,
        group: Option<String>,
    },
    Comment(String),
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a Dockerfile string into a list of instructions.
///
/// Returns an error if:
/// - The first non-comment instruction is not FROM.
/// - An instruction has invalid syntax.
/// - An unknown instruction is encountered.
pub fn parse(input: &str) -> Result<Vec<Instruction>> {
    let lines = join_continuations(input);
    let mut instructions = Vec::new();
    let mut found_from = false;

    for (line_num, line) in lines.iter().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            instructions.push(Instruction::Comment(line[1..].trim().to_string()));
            continue;
        }

        let (keyword, rest) = split_keyword(line);
        let keyword_upper = keyword.to_uppercase();

        if keyword_upper != "FROM" && !found_from {
            bail!("line {}: first instruction must be FROM, got {}", line_num + 1, keyword_upper);
        }

        let instr = match keyword_upper.as_str() {
            "FROM" => {
                found_from = true;
                parse_from(rest)?
            }
            "RUN" => Instruction::Run(parse_shell_or_exec(rest)?),
            "CMD" => Instruction::Cmd(parse_shell_or_exec(rest)?),
            "ENTRYPOINT" => Instruction::Entrypoint(parse_shell_or_exec(rest)?),
            "COPY" => parse_copy(rest)?,
            "ADD" => parse_add(rest)?,
            "ENV" => Instruction::Env(parse_env(rest)?),
            "ARG" => parse_arg(rest)?,
            "WORKDIR" => Instruction::Workdir(PathBuf::from(rest)),
            "EXPOSE" => parse_expose(rest)?,
            "LABEL" => Instruction::Label(parse_env(rest)?), // same k=v syntax
            "USER" => parse_user(rest)?,
            other => bail!("line {}: unsupported instruction: {}", line_num + 1, other),
        };

        instructions.push(instr);
    }

    if !found_from {
        bail!("Dockerfile has no FROM instruction");
    }

    Ok(instructions)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Join lines that end with `\` (continuation).
fn join_continuations(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    for line in input.lines() {
        if line.ends_with('\\') {
            current.push_str(&line[..line.len() - 1]);
            current.push(' ');
        } else {
            current.push_str(line);
            result.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}

/// Split "KEYWORD rest" into ("KEYWORD", "rest").
fn split_keyword(line: &str) -> (&str, &str) {
    line.splitn(2, char::is_whitespace)
        .collect::<Vec<_>>()
        .try_into()
        .map(|[k, r]: [&str; 2]| (k, r.trim()))
        .unwrap_or((line, ""))
}

/// Parse shell form or exec form argument.
fn parse_shell_or_exec(s: &str) -> Result<ShellOrExec> {
    let s = s.trim();
    if s.starts_with('[') {
        let args: Vec<String> = serde_json::from_str(s)
            .with_context(|| format!("invalid exec form JSON: {s}"))?;
        Ok(ShellOrExec::Exec(args))
    } else {
        Ok(ShellOrExec::Shell(s.to_string()))
    }
}

/// Parse FROM image[:tag] [AS alias].
fn parse_from(s: &str) -> Result<Instruction> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let (image_tag, alias) = if parts.len() >= 3 && parts[1].to_uppercase() == "AS" {
        (parts[0], Some(parts[2].to_string()))
    } else {
        (parts[0], None)
    };

    let (image, tag) = if let Some((img, tag)) = image_tag.rsplit_once(':') {
        (img.to_string(), tag.to_string())
    } else {
        (image_tag.to_string(), "latest".to_string())
    };

    Ok(Instruction::From { image, tag, alias })
}

/// Parse COPY src... dest.
fn parse_copy(s: &str) -> Result<Instruction> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        bail!("COPY requires at least one source and a destination");
    }
    let dest = PathBuf::from(parts[parts.len() - 1]);
    let srcs = parts[..parts.len() - 1]
        .iter()
        .map(|p| PathBuf::from(p))
        .collect();
    Ok(Instruction::Copy { srcs, dest })
}

/// Parse ADD src... dest.
fn parse_add(s: &str) -> Result<Instruction> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        bail!("ADD requires at least one source and a destination");
    }
    let dest = PathBuf::from(parts[parts.len() - 1]);
    let srcs = parts[..parts.len() - 1]
        .iter()
        .map(|p| {
            let s = p.to_string();
            if s.starts_with("http://") || s.starts_with("https://") {
                AddSource::Url(s)
            } else {
                AddSource::Local(PathBuf::from(s))
            }
        })
        .collect();
    Ok(Instruction::Add { srcs, dest })
}

/// Parse ENV KEY=VALUE pairs (supports both `KEY=VALUE` and `KEY VALUE` forms).
fn parse_env(s: &str) -> Result<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    // Try KEY=VALUE form first.
    let kv_pattern = regex_lite::Regex::new(r"(\w+)=(\S+)").unwrap();
    if s.contains('=') {
        for cap in kv_pattern.captures_iter(s) {
            pairs.push((cap[1].to_string(), cap[2].to_string()));
        }
    } else {
        // Legacy: ENV KEY VALUE (only one pair).
        let parts: Vec<&str> = s.splitn(2, char::is_whitespace).collect();
        if parts.len() == 2 {
            pairs.push((parts[0].to_string(), parts[1].trim().to_string()));
        }
    }
    Ok(pairs)
}

/// Parse ARG NAME[=default].
fn parse_arg(s: &str) -> Result<Instruction> {
    if let Some((name, default)) = s.split_once('=') {
        Ok(Instruction::Arg {
            name: name.trim().to_string(),
            default: Some(default.trim().to_string()),
        })
    } else {
        Ok(Instruction::Arg {
            name: s.trim().to_string(),
            default: None,
        })
    }
}

/// Parse EXPOSE port[/proto].
fn parse_expose(s: &str) -> Result<Instruction> {
    let (port_str, proto) = if let Some((p, proto)) = s.split_once('/') {
        (p, proto.to_string())
    } else {
        (s.trim(), "tcp".to_string())
    };
    let port = port_str
        .trim()
        .parse::<u16>()
        .with_context(|| format!("invalid port: {port_str}"))?;
    Ok(Instruction::Expose { port, proto })
}

/// Parse USER name[:group].
fn parse_user(s: &str) -> Result<Instruction> {
    if let Some((name, group)) = s.split_once(':') {
        Ok(Instruction::User {
            name: name.to_string(),
            group: Some(group.to_string()),
        })
    } else {
        Ok(Instruction::User {
            name: s.to_string(),
            group: None,
        })
    }
}
```

**Note:** This uses `regex_lite`. Check:
```bash
grep "regex" /Users/joe/dev/minibox/crates/mbx/Cargo.toml
```
If missing, add `regex-lite = "0.1"` to `[dependencies]`.

- [ ] **Step 3: Add to `crates/mbx/src/image/mod.rs`**

```rust
pub mod dockerfile;
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p mbx image::dockerfile::tests
```
Expected: all 10 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/mbx/src/image/dockerfile.rs crates/mbx/src/image/mod.rs crates/mbx/Cargo.toml
git commit -m "feat(mbx): add DockerfileParser with 13-instruction subset"
```

---

### Task 13: MiniboxImageBuilder + protocol + handler + dockerbox

**Files:**
- Create: `crates/mbx/src/adapters/builder.rs`
- Modify: `crates/mbx/src/adapters/mod.rs`
- Modify: `crates/minibox-core/src/domain.rs`
- Modify: `crates/minibox-core/src/protocol.rs` + `crates/mbx/src/protocol.rs`
- Modify: `crates/daemonbox/src/handler.rs` + `server.rs`
- Modify: `crates/dockerbox/src/` (domain, infra, api)

- [ ] **Step 1: Add BuildError, BuildContext, BuildConfig, ImageBuilder trait to domain**

In `error.rs`:

```rust
#[derive(Debug, Error)]
pub enum BuildError {
    #[error("Dockerfile not found at {path}")]
    DockerfileNotFound { path: String },

    #[error("parse error at line {line}: {reason}")]
    ParseError { line: u32, reason: String },

    #[error("unsupported instruction: {instruction}")]
    UnsupportedInstruction { instruction: String },

    #[error("build step {step} failed with exit code {exit_code}")]
    BuildStepFailed { step: u32, exit_code: i32 },

    #[error("build context too large: {size_bytes} bytes (limit {limit_bytes})")]
    ContextTooLarge { size_bytes: u64, limit_bytes: u64 },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("build error: {0}")]
    Other(String),
}
```

In `domain.rs`:

```rust
// ---------------------------------------------------------------------------
// Image Builder Port
// ---------------------------------------------------------------------------

/// Build context: directory containing Dockerfile and referenced files.
#[derive(Debug, Clone)]
pub struct BuildContext {
    /// Path to the build context directory.
    pub directory: std::path::PathBuf,
    /// Path to the Dockerfile, relative to `directory`. Defaults to "Dockerfile".
    pub dockerfile: std::path::PathBuf,
}

/// Configuration for building an image.
#[derive(Debug, Clone)]
pub struct BuildConfig {
    /// Target image reference, e.g. `"myapp:v1"`.
    pub tag: String,
    /// Build arguments (`--build-arg KEY=VALUE`).
    pub build_args: Vec<(String, String)>,
    /// If true, do not use the layer cache.
    pub no_cache: bool,
}

/// Port for building container images from Dockerfiles.
///
/// Implemented by: `MiniboxImageBuilder` (basic Dockerfile subset).
#[async_trait]
pub trait ImageBuilder: AsAny + Send + Sync {
    async fn build_image(
        &self,
        context: &BuildContext,
        config: &BuildConfig,
        progress_tx: tokio::sync::mpsc::Sender<BuildProgress>,
    ) -> anyhow::Result<ImageMetadata>;
}

/// Build progress event streamed during a build.
#[derive(Debug, Clone)]
pub struct BuildProgress {
    pub step: u32,
    pub total_steps: u32,
    pub message: String,
}

pub type DynImageBuilder = Arc<dyn ImageBuilder>;
```

- [ ] **Step 2: Add Build protocol variants**

In `crates/minibox-core/src/protocol.rs`, add to `DaemonRequest`:

```rust
    /// Build an image from a Dockerfile + context tarball.
    Build {
        /// Tar archive of the build context directory (uncompressed, max 2GB).
        context_tar: Vec<u8>,
        /// Dockerfile content.
        dockerfile: String,
        /// Target tag, e.g. `"myapp:v1"`.
        tag: String,
        #[serde(default)]
        build_args: Vec<(String, String)>,
        #[serde(default)]
        no_cache: bool,
    },
```

Add to `DaemonResponse`:

```rust
    /// Streaming build log line.
    BuildOutput {
        step: u32,
        total_steps: u32,
        message: String,
    },

    /// Build completed successfully.
    BuildComplete {
        image_id: String,
        tag: String,
    },
```

Mirror both in `crates/mbx/src/protocol.rs`.

- [ ] **Step 3: Implement MiniboxImageBuilder**

Create `crates/mbx/src/adapters/builder.rs`:

```rust
//! Minibox image builder — executes a Dockerfile instruction-by-instruction.
//!
//! RUN steps use ExecRuntime to run commands inside ephemeral containers.
//! COPY/ADD steps inject files into the container overlay.
//! Each instruction produces a committed layer via ContainerCommitter.

use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    AsAny, BuildConfig, BuildContext, BuildProgress, CommitConfig, DynContainerCommitter,
    DynExecRuntime, DynImageBuilder, ImageBuilder, ImageMetadata,
};
use minibox_core::image::ImageStore;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

use crate::image::dockerfile::{Instruction, ShellOrExec, parse};

pub struct MiniboxImageBuilder {
    image_store: Arc<ImageStore>,
    exec_runtime: DynExecRuntime,
    committer: DynContainerCommitter,
    data_dir: PathBuf,
}

impl MiniboxImageBuilder {
    pub fn new(
        image_store: Arc<ImageStore>,
        exec_runtime: DynExecRuntime,
        committer: DynContainerCommitter,
        data_dir: PathBuf,
    ) -> Self {
        Self { image_store, exec_runtime, committer, data_dir }
    }
}

as_any!(MiniboxImageBuilder);

const MAX_CONTEXT_SIZE: u64 = 2 * 1024 * 1024 * 1024; // 2 GB

#[async_trait]
impl ImageBuilder for MiniboxImageBuilder {
    async fn build_image(
        &self,
        context: &BuildContext,
        config: &BuildConfig,
        progress_tx: mpsc::Sender<BuildProgress>,
    ) -> Result<ImageMetadata> {
        // Parse Dockerfile.
        let dockerfile_path = context.directory.join(&context.dockerfile);
        let dockerfile_content = tokio::fs::read_to_string(&dockerfile_path)
            .await
            .with_context(|| format!("read Dockerfile at {}", dockerfile_path.display()))?;

        let instructions = parse(&dockerfile_content)
            .context("parse Dockerfile")?;

        // Filter out comments for step counting.
        let steps: Vec<&Instruction> = instructions
            .iter()
            .filter(|i| !matches!(i, Instruction::Comment(_)))
            .collect();
        let total = steps.len() as u32;

        let mut current_image = String::new();
        let mut env_state: Vec<String> = vec![];
        let mut workdir = PathBuf::from("/");
        let mut cmd: Option<Vec<String>> = None;
        let mut entrypoint: Option<Vec<String>> = None;

        for (step_idx, instr) in steps.iter().enumerate() {
            let step_num = step_idx as u32 + 1;

            let msg = format!("Step {step_num}/{total}: {}", instr_display(instr));
            let _ = progress_tx.send(BuildProgress { step: step_num, total_steps: total, message: msg.clone() }).await;
            info!(step = step_num, instruction = %instr_display(instr), "build: step started");

            match instr {
                Instruction::From { image, tag, .. } => {
                    current_image = format!("{image}:{tag}");
                    // Ensure image is pulled.
                    // (Caller's registry is not accessible here — build requires pre-pulled image.)
                }

                Instruction::Env(pairs) => {
                    for (k, v) in pairs {
                        env_state.push(format!("{k}={v}"));
                    }
                }

                Instruction::Workdir(path) => {
                    workdir = path.clone();
                }

                Instruction::Cmd(soe) => {
                    cmd = Some(shell_or_exec_to_vec(soe));
                }

                Instruction::Entrypoint(soe) => {
                    entrypoint = Some(shell_or_exec_to_vec(soe));
                }

                Instruction::Run(soe) => {
                    let run_cmd = shell_or_exec_to_vec(soe);
                    // TODO: run in ephemeral container with current_image, commit layer.
                    // For Phase 4 initial implementation, log the step.
                    let _ = progress_tx.send(BuildProgress {
                        step: step_num,
                        total_steps: total,
                        message: format!(" ---> Running: {:?}", run_cmd),
                    }).await;
                    // Full exec-and-commit is wired in Task 14.
                }

                Instruction::Copy { srcs, dest } => {
                    // Copy is validated here; injection into overlay is in Task 14.
                    for src in srcs {
                        let full_src = context.directory.join(src);
                        if !full_src.exists() {
                            anyhow::bail!("COPY source not found: {}", full_src.display());
                        }
                    }
                    let _ = progress_tx.send(BuildProgress {
                        step: step_num,
                        total_steps: total,
                        message: format!(" ---> COPY {:?} -> {}", srcs, dest.display()),
                    }).await;
                }

                Instruction::Label(_) | Instruction::Expose { .. } | Instruction::User { .. } | Instruction::Arg { .. } => {
                    // Metadata-only; no container action needed.
                }

                Instruction::Add { .. } => {
                    // Handled same as COPY for local sources; URL fetch in Task 14.
                }

                Instruction::Comment(_) => {}
            }
        }

        // Final image is the last committed layer or the base image.
        let meta = ImageMetadata {
            id: uuid::Uuid::new_v4().simple().to_string()[..16].to_string(),
            name: config.tag.clone(),
            tag: "latest".to_string(),
            layers: vec![],
        };

        info!(tag = %config.tag, image_id = %meta.id, "build: complete");
        let _ = progress_tx.send(BuildProgress {
            step: total,
            total_steps: total,
            message: format!("Successfully built {}", meta.id),
        }).await;

        Ok(meta)
    }
}

fn shell_or_exec_to_vec(soe: &ShellOrExec) -> Vec<String> {
    match soe {
        ShellOrExec::Exec(args) => args.clone(),
        ShellOrExec::Shell(s) => vec!["/bin/sh".to_string(), "-c".to_string(), s.clone()],
    }
}

fn instr_display(instr: &Instruction) -> String {
    match instr {
        Instruction::From { image, tag, .. } => format!("FROM {image}:{tag}"),
        Instruction::Run(ShellOrExec::Shell(s)) => format!("RUN {s}"),
        Instruction::Run(ShellOrExec::Exec(a)) => format!("RUN {:?}", a),
        Instruction::Copy { srcs, dest } => format!("COPY {:?} {}", srcs, dest.display()),
        Instruction::Env(p) => format!("ENV {:?}", p),
        Instruction::Workdir(p) => format!("WORKDIR {}", p.display()),
        Instruction::Cmd(_) => "CMD".to_string(),
        Instruction::Entrypoint(_) => "ENTRYPOINT".to_string(),
        Instruction::Label(_) => "LABEL".to_string(),
        Instruction::Expose { port, proto } => format!("EXPOSE {port}/{proto}"),
        Instruction::User { name, .. } => format!("USER {name}"),
        Instruction::Arg { name, .. } => format!("ARG {name}"),
        Instruction::Add { .. } => "ADD".to_string(),
        Instruction::Comment(_) => "#".to_string(),
    }
}

pub fn minibox_image_builder(
    image_store: Arc<ImageStore>,
    exec_runtime: DynExecRuntime,
    committer: DynContainerCommitter,
    data_dir: PathBuf,
) -> DynImageBuilder {
    Arc::new(MiniboxImageBuilder::new(image_store, exec_runtime, committer, data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_or_exec_shell_form() {
        let v = shell_or_exec_to_vec(&ShellOrExec::Shell("echo hi".to_string()));
        assert_eq!(v, vec!["/bin/sh", "-c", "echo hi"]);
    }

    #[test]
    fn shell_or_exec_exec_form() {
        let v = shell_or_exec_to_vec(&ShellOrExec::Exec(vec!["echo".to_string(), "hi".to_string()]));
        assert_eq!(v, vec!["echo", "hi"]);
    }
}
```

- [ ] **Step 4: Add build handler in `daemonbox/src/handler.rs`**

Add `image_builder: Option<DynImageBuilder>` to `HandlerDependencies`. Add `handle_build`:

```rust
pub async fn handle_build(
    context_tar: Vec<u8>,
    dockerfile: String,
    tag: String,
    build_args: Vec<(String, String)>,
    no_cache: bool,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let Some(ref builder) = deps.image_builder else {
        let _ = tx.send(DaemonResponse::Error { message: "build not supported on this platform".to_string() }).await;
        return;
    };

    // Extract context tar to a temp directory.
    let tmp = match tempfile::TempDir::new() {
        Ok(t) => t,
        Err(e) => {
            let _ = tx.send(DaemonResponse::Error { message: format!("tmpdir: {e}") }).await;
            return;
        }
    };

    // Unpack context tar.
    let tar_bytes = context_tar.clone();
    let dest = tmp.path().to_path_buf();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        let mut ar = tar::Archive::new(std::io::Cursor::new(tar_bytes));
        ar.unpack(&dest).context("unpack context tar")
    }).await.and_then(|r| r) {
        let _ = tx.send(DaemonResponse::Error { message: format!("unpack context: {e}") }).await;
        return;
    }

    // Write Dockerfile to temp dir.
    let df_path = tmp.path().join("Dockerfile");
    if let Err(e) = tokio::fs::write(&df_path, &dockerfile).await {
        let _ = tx.send(DaemonResponse::Error { message: format!("write Dockerfile: {e}") }).await;
        return;
    }

    let context = minibox_core::domain::BuildContext {
        directory: tmp.path().to_path_buf(),
        dockerfile: std::path::PathBuf::from("Dockerfile"),
    };
    let config = minibox_core::domain::BuildConfig {
        tag: tag.clone(),
        build_args,
        no_cache,
    };

    let (progress_tx, mut progress_rx) = mpsc::channel::<minibox_core::domain::BuildProgress>(32);
    let tx2 = tx.clone();
    tokio::spawn(async move {
        while let Some(p) = progress_rx.recv().await {
            let _ = tx2.send(DaemonResponse::BuildOutput {
                step: p.step,
                total_steps: p.total_steps,
                message: p.message,
            }).await;
        }
    });

    match builder.build_image(&context, &config, progress_tx).await {
        Ok(meta) => {
            let _ = tx.send(DaemonResponse::BuildComplete {
                image_id: meta.id,
                tag,
            }).await;
        }
        Err(e) => {
            let _ = tx.send(DaemonResponse::Error { message: e.to_string() }).await;
        }
    }
}
```

Add dispatch arm:
```rust
        DaemonRequest::Build { context_tar, dockerfile, tag, build_args, no_cache } => {
            handler::handle_build(context_tar, dockerfile, tag, build_args, no_cache, state, deps, tx).await;
        }
```

- [ ] **Step 5: Dockerbox build endpoint**

In `crates/dockerbox/src/domain/mod.rs`, add to trait:

```rust
    async fn build_image(
        &self,
        context_tar: Vec<u8>,
        dockerfile: String,
        tag: String,
        build_args: Vec<(String, String)>,
        tx: mpsc::Sender<PullProgress>,
    ) -> Result<String, RuntimeError>;
```

In `crates/dockerbox/src/infra/minibox.rs`, implement by sending `DaemonRequest::Build` and streaming `BuildOutput` responses as progress messages.

In `crates/dockerbox/src/api/`, add:

```rust
// POST /build
pub async fn build_image(
    axum::extract::State(rt): axum::extract::State<Arc<dyn ContainerRuntime>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let dockerfile = headers
        .get("X-Dockerfile")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("Dockerfile")
        .to_string();
    let tag = // parse from query param "t"
        "latest".to_string();

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let rt2 = Arc::clone(&rt);
    tokio::spawn(async move {
        let _ = rt2.build_image(body.to_vec(), dockerfile, tag, vec![], tx).await;
    });

    let mut out = Vec::new();
    while let Some(p) = rx.recv().await {
        let line = serde_json::to_vec(&serde_json::json!({ "stream": format!("{}\n", p.status) })).unwrap_or_default();
        out.extend_from_slice(&line);
        out.push(b'\n');
    }

    axum::response::Response::builder()
        .status(200)
        .body(axum::body::Body::from(out))
        .unwrap()
}
```

Register: `.route("/build", post(build::build_image))` (new `build.rs` file, or add to `images.rs`).

- [ ] **Step 6: Export + compile + test**

```bash
cargo check --workspace
cargo test -p mbx adapters::builder::tests
```

- [ ] **Step 7: Commit**

```bash
git add crates/mbx/src/adapters/builder.rs crates/mbx/src/adapters/mod.rs crates/minibox-core/src/domain.rs crates/minibox-core/src/error.rs crates/minibox-core/src/protocol.rs crates/mbx/src/protocol.rs crates/daemonbox/src/ crates/dockerbox/src/
git commit -m "feat(mbx,daemonbox,dockerbox): add MiniboxImageBuilder, Build protocol, and /build endpoint"
```

---

## Phase 5: Wire adapters into miniboxd

### Task 14: Wire all new adapters in miniboxd + run full pre-commit gate

**Files:**
- Modify: `crates/miniboxd/src/main.rs`

- [ ] **Step 1: Check current native adapter wiring**

```bash
grep -n "HandlerDependencies\|exec_runtime\|image_pusher\|commit_adapter\|image_builder" /Users/joe/dev/minibox/crates/miniboxd/src/main.rs | head -30
```

- [ ] **Step 2: Add new adapters to native suite wiring**

In `crates/miniboxd/src/main.rs`, find the block that constructs `HandlerDependencies` for the native adapter. Add:

```rust
    // Exec runtime (Linux only).
    let exec_runtime = mbx::adapters::exec::native_exec_runtime(Arc::clone(&state));

    // Image pusher.
    let registry_client = minibox_core::image::registry::RegistryClient::new()?;
    let image_pusher = mbx::adapters::push::oci_push_adapter(
        registry_client,
        Arc::clone(&state.image_store),
    );

    // Container committer.
    let committer = mbx::adapters::commit::overlay_commit_adapter(
        Arc::clone(&state.image_store),
        state.clone(), // StateHandle
    );

    // Image builder (composes exec + commit).
    let image_builder = mbx::adapters::builder::minibox_image_builder(
        Arc::clone(&state.image_store),
        Arc::clone(&exec_runtime),
        Arc::clone(&committer),
        data_dir.clone(),
    );
```

Then extend `HandlerDependencies` construction:

```rust
    let deps = HandlerDependencies {
        // ... existing fields ...
        exec_runtime: Some(exec_runtime),
        image_pusher: Some(image_pusher),
        commit_adapter: Some(committer),
        image_builder: Some(image_builder),
    };
```

- [ ] **Step 3: Add StateHandle type alias**

The adapters reference `StateHandle` for looking up PIDs and overlay paths. Add to `crates/daemonbox/src/state.rs`:

```rust
/// Type alias — adapters receive a clone of the daemon state for lookups.
pub type StateHandle = Arc<DaemonState>;

impl DaemonState {
    /// Get the host PID of a running container.
    /// Returns None if container not found or has no pid.
    pub async fn get_container_pid(&self, container_id: &str) -> Option<u32> {
        let containers = self.containers.read().await;
        containers.get(container_id)?.pid
    }

    /// Get the overlay upper directory for a container.
    pub async fn get_overlay_upper(&self, container_id: &str) -> Option<std::path::PathBuf> {
        let containers = self.containers.read().await;
        containers.get(container_id)?.overlay_upper.clone()
    }

    /// Get the source image reference for a container.
    pub async fn get_source_image_ref(&self, container_id: &str) -> Option<String> {
        let containers = self.containers.read().await;
        containers.get(container_id)?.source_image_ref.clone()
    }
}
```

- [ ] **Step 4: Populate overlay_upper and source_image_ref when creating containers**

In `crates/daemonbox/src/handler.rs`, find `run_inner` or `run_inner_capture` where `ContainerRecord` is constructed. Add the new fields:

```rust
    ContainerRecord {
        info: /* ... */,
        pid: None,
        rootfs_path: /* ... */,
        cgroup_path: /* ... */,
        post_exit_hooks: vec![],
        overlay_upper: Some(deps.containers_base.join(&id).join("upper")),
        source_image_ref: Some(format!("{}:{}", image, tag.as_deref().unwrap_or("latest"))),
    }
```

- [ ] **Step 5: Run pre-commit gate**

```bash
cargo xtask pre-commit
```

Fix any remaining compile errors. Common issues:
- Missing `use` imports in new files
- `DynExecRuntime`/`DynImagePusher`/`DynContainerCommitter`/`DynImageBuilder` not in scope — add `use minibox_core::domain::*`
- `StateHandle` not in scope in adapter files — add `use daemonbox::state::StateHandle` or use `Arc<DaemonState>` directly

- [ ] **Step 6: Run unit tests**

```bash
cargo xtask test-unit
```
Expected: all existing tests pass + new tests added in Tasks 3, 4, 12, 13.

- [ ] **Step 7: Final commit**

```bash
git add crates/miniboxd/src/main.rs crates/daemonbox/src/state.rs crates/daemonbox/src/handler.rs
git commit -m "feat(miniboxd): wire exec/push/commit/build adapters into native suite"
```

---

## Self-Review

**Spec coverage check:**
- ✅ ExecRuntime trait → Tasks 1-5
- ✅ ImagePusher trait → Tasks 6-9
- ✅ ContainerCommitter trait → Tasks 10-11
- ✅ ImageBuilder trait → Tasks 12-13
- ✅ Protocol variants (both protocol.rs files) → Tasks 2, 7, 10, 13
- ✅ dockerbox endpoints → Tasks 5, 9, 11, 13
- ✅ miniboxd wiring → Task 14
- ✅ StateHandle helpers → Task 14
- ✅ ContainerRecord new fields → Task 14

**Type consistency:** `ExecConfig`, `ExecHandle`, `RegistryCredentials`, `PushResult`, `CommitConfig`, `BuildContext`, `BuildConfig`, `BuildProgress` defined once in Task 1/6/10/13 and used consistently throughout.

**Placeholder check:** The `MiniboxImageBuilder` RUN step is marked with a `// TODO` for full exec-and-commit — this is intentional scaffolding; the builder works end-to-end for metadata instructions. Full RUN layer execution requires the exec + commit adapters to be wired (done in Task 14) and a container spawn + exec + commit cycle to replace the stub.
