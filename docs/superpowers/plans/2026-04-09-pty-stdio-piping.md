# PTY/Stdio Piping for Interactive Containers — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add PTY allocation + stdin piping so `minibox exec -it alpine sh` and
`minibox run -it alpine sh` open a fully-interactive terminal session inside the container.

**Architecture:** Three-layer change: (1) protocol gains `ResizePty` + `SendInput` request
variants and `tty: bool` on `Run`; (2) `NativeExecRuntime` branches on `config.tty` — `false`
keeps existing pipes, `true` calls `openpty(3)`, forks with the slave as the controlling
terminal, and reads the master for output; (3) CLI detects `-i`/`-t` flags, sets terminal
raw mode, proxies stdin to a new `SendInput` request, and forwards `SIGWINCH` as `ResizePty`.
No new crates needed — `libc` and `nix` are already workspace deps.

**Tech Stack:** Rust 2024 edition, `nix` (openpty, signal handling), `libc` (tcsetattr,
TIOCSWINSZ, fork, setns), `tokio` (async channels), `serde_json` (protocol), `tempfile` (tests).

---

## File Map

| File                                      | Change                                                                   |
| ----------------------------------------- | ------------------------------------------------------------------------ |
| `crates/minibox-core/src/protocol.rs`     | Add `ResizePty` + `SendInput` request variants; add `tty` field to `Run` |
| `crates/minibox-core/src/domain.rs`       | Add `stdin_tx` channel field to `ExecConfig`                             |
| `crates/minibox/src/adapters/exec.rs`     | PTY branch in `run_exec_blocking`; stdin relay task                      |
| `crates/daemonbox/src/handler.rs`         | Wire `SendInput`/`ResizePty` dispatch; pass stdin channel                |
| `crates/daemonbox/src/server.rs`          | Add `SendInput`/`ResizePty` to dispatch                                  |
| `crates/minibox-cli/src/terminal.rs`      | New: raw-mode guard + terminal_size()                                    |
| `crates/minibox-cli/src/commands/exec.rs` | `-it` flag, raw mode, stdin task, SIGWINCH forwarding                    |
| `crates/minibox-cli/src/commands/run.rs`  | `-it` flag, same terminal setup as exec                                  |
| `crates/minibox-cli/src/main.rs`          | Add `-i`/`-t` flags to `Exec` and `Run` subcommands                      |

---

## Task 1: Protocol — `ResizePty` + `SendInput` variants + `tty` on `Run`

**Files:**

- Modify: `crates/minibox-core/src/protocol.rs`

- [ ] **Step 1: Add `tty` field to `DaemonRequest::Run`**

In `crates/minibox-core/src/protocol.rs`, find the `Run` variant and add after the `name`
field (last field, around line 107):

```rust
        /// If `true`, allocate a PTY and stream stdin/stdout as a terminal session.
        #[serde(default)]
        tty: bool,
```

- [ ] **Step 2: Add `SendInput` and `ResizePty` variants**

After the `Exec` variant (around line 170), insert:

```rust
    /// Send raw bytes to a running exec or run session stdin (base64-encoded).
    SendInput {
        session_id: String,
        data: String,
    },

    /// Notify the daemon the client terminal was resized.
    ResizePty {
        session_id: String,
        cols: u16,
        rows: u16,
    },
```

- [ ] **Step 3: Write serialisation tests**

At the bottom of the `#[cfg(test)]` block in `protocol.rs`:

```rust
    #[test]
    fn send_input_roundtrip() {
        use base64::Engine as _;
        let bytes = b"ls\n";
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        let req = DaemonRequest::SendInput {
            session_id: "sess1".to_string(),
            data: data.clone(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"SendInput\""), "{json}");
        let back: DaemonRequest = serde_json::from_str(&json).unwrap();
        match back {
            DaemonRequest::SendInput { data: d, .. } => assert_eq!(d, data),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn resize_pty_roundtrip() {
        let req = DaemonRequest::ResizePty {
            session_id: "sess1".to_string(),
            cols: 120,
            rows: 40,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"ResizePty\""), "{json}");
        let back: DaemonRequest = serde_json::from_str(&json).unwrap();
        match back {
            DaemonRequest::ResizePty { cols, rows, .. } => {
                assert_eq!(cols, 120);
                assert_eq!(rows, 40);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn run_request_tty_defaults_false() {
        // Old clients omitting `tty` must still deserialise cleanly.
        let json = r#"{"type":"Run","image":"alpine","tag":"latest","command":["sh"],"memory_limit_bytes":null,"cpu_weight":null}"#;
        let req: DaemonRequest = serde_json::from_str(json).unwrap();
        match req {
            DaemonRequest::Run { tty, .. } => assert!(!tty),
            _ => panic!("wrong variant"),
        }
    }
```

- [ ] **Step 4: Run the new tests**

```bash
cargo test -p minibox-core send_input_roundtrip resize_pty_roundtrip run_request_tty_defaults
```

Expected: 3 PASS.

- [ ] **Step 5: Compile-check workspace; fix exhaustive match errors**

```bash
cargo check --workspace
```

For any `non_exhaustive_patterns` error on `DaemonRequest`, add:

```rust
DaemonRequest::SendInput { .. } | DaemonRequest::ResizePty { .. } => {}
```

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-core/src/protocol.rs
git commit -m "feat(protocol): add ResizePty, SendInput variants; tty field on Run"
```

---

## Task 2: Domain — `stdin_tx` + `resize_rx` in `ExecConfig`

**Files:**

- Modify: `crates/minibox-core/src/domain.rs`
- Modify: `crates/minibox/src/adapters/exec.rs`
- Modify: `crates/daemonbox/src/handler.rs`

`ExecConfig` gains two new optional fields. Because `mpsc::Receiver` is not `Clone`, we
remove the `Clone` derive and add `resize_rx`. The `resize_tx` end is stored in the handler's
registry; the `rx` end is passed through `ExecConfig` into the blocking exec task.

- [ ] **Step 1: Update `ExecConfig` in `domain.rs`**

Replace the struct (around line 767):

```rust
/// Configuration for running a command inside a running container.
/// Not Clone — contains channel receivers.
#[derive(Debug)]
pub struct ExecConfig {
    pub cmd: Vec<String>,
    pub env: Vec<String>,
    pub working_dir: Option<std::path::PathBuf>,
    pub tty: bool,
    /// Stdin bytes channel (handler → exec adapter). None = no stdin relay.
    pub stdin_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
    /// PTY resize events channel (handler → exec adapter). None = no resize.
    pub resize_rx: Option<tokio::sync::mpsc::Receiver<(u16, u16)>>,
}
```

- [ ] **Step 2: Fix `exec.rs` unit test — add new fields**

In `crates/minibox/src/adapters/exec.rs` test:

```rust
    #[test]
    fn exec_config_fields() {
        let cfg = ExecConfig {
            cmd: vec!["echo".to_string(), "hello".to_string()],
            env: vec!["HOME=/root".to_string()],
            working_dir: None,
            tty: false,
            stdin_tx: None,
            resize_rx: None,
        };
        assert_eq!(cfg.cmd[0], "echo");
        assert_eq!(cfg.env[0], "HOME=/root");
        assert!(cfg.stdin_tx.is_none());
        assert!(cfg.resize_rx.is_none());
    }
```

- [ ] **Step 3: Fix handler `ExecConfig` construction**

In `crates/daemonbox/src/handler.rs` around line 1765:

```rust
    let config = minibox_core::domain::ExecConfig {
        cmd,
        env,
        working_dir: working_dir.map(std::path::PathBuf::from),
        tty,
        stdin_tx: None,   // wired in Task 4
        resize_rx: None,  // wired in Task 4
    };
```

- [ ] **Step 4: Compile-check**

```bash
cargo check --workspace
```

Fix any `Clone` call-sites on `ExecConfig` — replace with manual field-by-field construction
or pass by ownership.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-core/src/domain.rs crates/minibox/src/adapters/exec.rs \
        crates/daemonbox/src/handler.rs
git commit -m "feat(domain): add stdin_tx, resize_rx to ExecConfig for PTY relay"
```

---

## Task 3: PTY allocation in `NativeExecRuntime`

**Files:**

- Modify: `crates/minibox/src/adapters/exec.rs`

- [ ] **Step 1: Add a failing PTY unit test**

At the bottom of `#[cfg(test)]` in `crates/minibox/src/adapters/exec.rs`:

```rust
    #[cfg(target_os = "linux")]
    #[test]
    fn pty_exec_echo_roundtrip() {
        use minibox_core::protocol::{DaemonResponse, OutputStreamKind};
        use tokio::sync::mpsc;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, mut rx) = mpsc::channel::<DaemonResponse>(32);
        let (_resize_tx, resize_rx) = tokio::sync::mpsc::channel::<(u16, u16)>(8);
        let config = ExecConfig {
            cmd: vec!["/bin/echo".to_string(), "pty-ok".to_string()],
            env: vec![],
            working_dir: None,
            tty: true,
            stdin_tx: None,
            resize_rx: Some(resize_rx),
        };
        let our_pid = std::process::id();

        std::thread::spawn(move || {
            rt.block_on(async {
                tokio::task::spawn_blocking(move || {
                    run_exec_blocking(our_pid, "test-pty-1", config, tx);
                })
                .await
                .unwrap();
            });
        });

        let responses: Vec<DaemonResponse> = {
            let rt2 = tokio::runtime::Runtime::new().unwrap();
            rt2.block_on(async {
                let mut out = vec![];
                loop {
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        rx.recv(),
                    ).await {
                        Ok(Some(r)) => {
                            let done = matches!(r, DaemonResponse::ContainerStopped { .. });
                            out.push(r);
                            if done { break; }
                        }
                        _ => break,
                    }
                }
                out
            })
        };

        let has_output = responses.iter().any(|r| {
            if let DaemonResponse::ContainerOutput { data, .. } = r {
                let bytes = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
                String::from_utf8_lossy(&bytes).contains("pty-ok")
            } else { false }
        });
        assert!(has_output, "expected pty-ok in output; got: {responses:?}");
        assert!(
            responses.iter().any(|r| matches!(r, DaemonResponse::ContainerStopped { exit_code: 0 })),
            "expected ContainerStopped(0)"
        );
    }
```

- [ ] **Step 2: Run test — confirm it fails**

```bash
cargo test -p minibox pty_exec_echo_roundtrip -- --nocapture 2>&1 | tail -10
```

Expected: compile error (`run_exec_blocking` signature mismatch) or test failure.

- [ ] **Step 3: Refactor `run_exec_blocking` to own `config` and branch on `tty`**

Change signature to take `config` by value (not reference) so we can move `resize_rx` out:

```rust
fn run_exec_blocking(
    container_pid: u32,
    exec_id: &str,
    config: ExecConfig,
    tx: mpsc::Sender<DaemonResponse>,
) {
    if config.tty {
        run_pty_exec(container_pid, exec_id, config, tx);
    } else {
        run_pipe_exec(container_pid, exec_id, &config, tx);
    }
}
```

Update the `spawn_blocking` call in `run_in_container` to pass `config` by value (it's
already `let config = config.clone()` — now just `let config = ...` directly by moving).

- [ ] **Step 4: Extract existing fork+pipe into `run_pipe_exec`**

Move the existing body of `run_exec_blocking` into:

```rust
fn run_pipe_exec(
    container_pid: u32,
    exec_id: &str,
    config: &ExecConfig,
    tx: mpsc::Sender<DaemonResponse>,
) {
    // ... existing pipe + fork + waitpid + stream_fd_to_channel code unchanged ...
}
```

- [ ] **Step 5: Implement `run_pty_exec`**

```rust
fn run_pty_exec(
    container_pid: u32,
    exec_id: &str,
    config: ExecConfig,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let send_err = |msg: String| {
        let rt = tokio::runtime::Handle::current();
        let _ = rt.block_on(tx.send(DaemonResponse::Error { message: msg }));
    };

    // Open namespace fds.
    let ns_base = format!("/proc/{container_pid}/ns");
    let ns_names = ["mnt", "pid", "net", "uts", "ipc"];
    let ns_fds: Vec<std::fs::File> = ns_names.iter()
        .filter_map(|ns| {
            std::fs::File::open(format!("{ns_base}/{ns}"))
                .map_err(|e| warn!(ns = %ns, error = %e, "exec: failed to open ns fd"))
                .ok()
        })
        .collect();
    if ns_fds.len() != ns_names.len() {
        send_err(format!("exec: could not open all ns fds for pid {container_pid}"));
        return;
    }

    // Allocate PTY via nix (safe bindings over openpty(3)).
    let pty = match nix::pty::openpty(None, None) {
        Ok(p) => p,
        Err(e) => {
            send_err(format!("exec: openpty failed: {e}"));
            return;
        }
    };
    use std::os::fd::IntoRawFd as _;
    let master_fd = pty.master.into_raw_fd();
    let slave_fd  = pty.slave.into_raw_fd();

    // SAFETY: about to fork; all fds are managed explicitly below.
    let child_pid = unsafe { libc::fork() };
    match child_pid {
        -1 => {
            send_err("exec: pty fork failed".to_string());
            // SAFETY: fork failed so only this process exists; close both fds.
            unsafe { libc::close(master_fd); libc::close(slave_fd); }
        }
        0 => {
            // ── Child ──────────────────────────────────────────────────────
            for f in &ns_fds {
                use std::os::fd::AsRawFd;
                // SAFETY: setns joins namespace; f is a valid open fd.
                unsafe { libc::setns(f.as_raw_fd(), 0) };
            }
            // SAFETY: setsid creates new session; safe in child after fork.
            unsafe { libc::setsid() };
            // SAFETY: TIOCSCTTY acquires slave as controlling terminal in the new session.
            unsafe { libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0i32) };
            // SAFETY: dup2 duplicates slave fd into stdin/stdout/stderr slots.
            unsafe {
                libc::dup2(slave_fd, 0);
                libc::dup2(slave_fd, 1);
                libc::dup2(slave_fd, 2);
                if slave_fd > 2 { libc::close(slave_fd); }
                libc::close(master_fd);
            }
            let cmd_cstr = match std::ffi::CString::new(config.cmd[0].clone()) {
                Ok(c) => c,
                Err(_) => unsafe { libc::_exit(127) },
            };
            let args: Vec<std::ffi::CString> = config.cmd.iter()
                .filter_map(|s| std::ffi::CString::new(s.as_str()).ok()).collect();
            let envp: Vec<std::ffi::CString> = config.env.iter()
                .filter_map(|s| std::ffi::CString::new(s.as_str()).ok()).collect();
            let args_ptrs: Vec<*const libc::c_char> = args.iter()
                .map(|s| s.as_ptr()).chain(std::iter::once(std::ptr::null())).collect();
            let envp_ptrs: Vec<*const libc::c_char> = envp.iter()
                .map(|s| s.as_ptr()).chain(std::iter::once(std::ptr::null())).collect();
            // SAFETY: execve replaces this process image.
            unsafe {
                libc::execve(cmd_cstr.as_ptr(), args_ptrs.as_ptr(), envp_ptrs.as_ptr());
                libc::_exit(127);
            }
        }
        child => {
            // ── Parent ─────────────────────────────────────────────────────
            // SAFETY: slave is now owned by child; parent closes its copy.
            unsafe { libc::close(slave_fd) };

            // Resize relay thread.
            if let Some(mut resize_rx) = config.resize_rx {
                let mfd = master_fd;
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
                    rt.block_on(async {
                        while let Some((cols, rows)) = resize_rx.recv().await {
                            let ws = libc::winsize {
                                ws_col: cols, ws_row: rows,
                                ws_xpixel: 0, ws_ypixel: 0,
                            };
                            // SAFETY: TIOCSWINSZ ioctl on master_fd updates PTY window size.
                            unsafe { libc::ioctl(mfd, libc::TIOCSWINSZ as _, &ws); }
                        }
                    });
                });
            }

            // Stream master → ContainerOutput (PTY merges stdout+stderr).
            stream_fd_to_channel(master_fd, OutputStreamKind::Stdout, &tx);

            let mut status: libc::c_int = 0;
            // SAFETY: child is a valid PID from fork.
            unsafe { libc::waitpid(child, &mut status, 0) };
            let exit_code = if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { -1 };
            let rt = tokio::runtime::Handle::current();
            let _ = rt.block_on(tx.send(DaemonResponse::ContainerStopped { exit_code }));
            info!(exec_id = %exec_id, exit_code, "exec: pty process exited");
        }
    }
}
```

- [ ] **Step 6: Run PTY unit test**

```bash
cargo test -p minibox pty_exec_echo_roundtrip -- --nocapture 2>&1 | tail -20
```

Expected: PASS.

- [ ] **Step 7: Run full unit suite**

```bash
cargo xtask test-unit 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/minibox/src/adapters/exec.rs
git commit -m "feat(exec): PTY allocation path in NativeExecRuntime (openpty + setsid + TIOCSCTTY)"
```

---

## Task 4: Handler wiring — `SendInput` + `ResizePty` + `PtySessionRegistry`

**Files:**

- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/daemonbox/src/server.rs`

- [ ] **Step 1: Add `PtySessionRegistry` to `HandlerDependencies`**

In `crates/daemonbox/src/handler.rs`, add near the top (after existing `use` statements):

```rust
use std::collections::HashMap;
use tokio::sync::Mutex as TokioMutex;

/// Tracks live PTY session channels keyed by session ID (container_id for execs).
#[derive(Default)]
pub struct PtySessionRegistry {
    pub resize: HashMap<String, tokio::sync::mpsc::Sender<(u16, u16)>>,
    pub stdin: HashMap<String, tokio::sync::mpsc::Sender<Vec<u8>>>,
}

pub type SharedPtyRegistry = Arc<TokioMutex<PtySessionRegistry>>;
```

Add to `HandlerDependencies` struct:

```rust
    pub pty_sessions: SharedPtyRegistry,
```

In every `HandlerDependencies { ... }` construction site, add:

```rust
    pty_sessions: Arc::new(TokioMutex::new(PtySessionRegistry::default())),
```

Run `cargo check -p daemonbox` to find all construction sites.

- [ ] **Step 2: Create channels in `handle_exec` and register them**

In `handle_exec`, just before constructing `ExecConfig`:

```rust
    let (resize_tx, resize_rx) = tokio::sync::mpsc::channel::<(u16, u16)>(8);
    let (stdin_ch_tx, _stdin_ch_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
    let session_key = container_id.clone();
    {
        let mut reg = deps.pty_sessions.lock().await;
        reg.resize.insert(session_key.clone(), resize_tx);
        if tty {
            reg.stdin.insert(session_key, stdin_ch_tx.clone());
        }
    }
    let config = minibox_core::domain::ExecConfig {
        cmd,
        env,
        working_dir: working_dir.map(std::path::PathBuf::from),
        tty,
        stdin_tx: if tty { Some(stdin_ch_tx) } else { None },
        resize_rx: if tty { Some(resize_rx) } else { None },
    };
```

- [ ] **Step 3: Add `handle_send_input` and `handle_resize_pty`**

```rust
pub async fn handle_send_input(
    session_id: String,
    data: String,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use base64::Engine as _;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(&data) {
        Ok(b) => b,
        Err(e) => {
            send_error(&tx, "handle_send_input", format!("base64 decode: {e}")).await;
            return;
        }
    };
    let reg = deps.pty_sessions.lock().await;
    match reg.stdin.get(&session_id) {
        Some(stdin_tx) => {
            if stdin_tx.send(bytes).await.is_err() {
                warn!(session_id = %session_id, "send_input: stdin channel closed");
            }
        }
        None => {
            send_error(&tx, "handle_send_input",
                format!("no active tty session: {session_id}")).await;
            return;
        }
    }
    if tx.send(DaemonResponse::Success).await.is_err() {
        warn!(session_id = %session_id, "send_input: client disconnected");
    }
}

pub async fn handle_resize_pty(
    session_id: String,
    cols: u16,
    rows: u16,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let reg = deps.pty_sessions.lock().await;
    match reg.resize.get(&session_id) {
        Some(resize_tx) => {
            if resize_tx.send((cols, rows)).await.is_err() {
                warn!(session_id = %session_id, "resize_pty: channel closed");
            }
        }
        None => {
            send_error(&tx, "handle_resize_pty",
                format!("no active tty session: {session_id}")).await;
            return;
        }
    }
    if tx.send(DaemonResponse::Success).await.is_err() {
        warn!(session_id = %session_id, "resize_pty: client disconnected");
    }
}
```

- [ ] **Step 4: Dispatch from `server.rs`**

In `crates/daemonbox/src/server.rs`, in the `match request` dispatch block, add:

```rust
            DaemonRequest::SendInput { session_id, data } => {
                let deps = Arc::clone(&deps);
                let tx = tx.clone();
                tokio::spawn(async move {
                    handle_send_input(session_id, data, deps, tx).await;
                });
            }
            DaemonRequest::ResizePty { session_id, cols, rows } => {
                let deps = Arc::clone(&deps);
                let tx = tx.clone();
                tokio::spawn(async move {
                    handle_resize_pty(session_id, cols, rows, deps, tx).await;
                });
            }
```

Add to the `use crate::handler::{...}` import at the top of `server.rs`:
`handle_send_input, handle_resize_pty`.

- [ ] **Step 5: Write handler unit tests**

In `crates/daemonbox/tests/handler_tests.rs`:

```rust
#[tokio::test]
async fn send_input_unknown_session_returns_error() {
    use base64::Engine as _;
    let tmp = tempfile::TempDir::new().unwrap();
    let (deps, _state) = create_test_deps_with_dir(tmp.path()).await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let data = base64::engine::general_purpose::STANDARD.encode(b"hello");
    handle_send_input("no-such-session".to_string(), data, deps, tx).await;
    let resp = rx.recv().await.unwrap();
    assert!(matches!(resp, DaemonResponse::Error { .. }), "got {resp:?}");
}

#[tokio::test]
async fn resize_pty_unknown_session_returns_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (deps, _state) = create_test_deps_with_dir(tmp.path()).await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    handle_resize_pty("no-such-session".to_string(), 80, 24, deps, tx).await;
    let resp = rx.recv().await.unwrap();
    assert!(matches!(resp, DaemonResponse::Error { .. }), "got {resp:?}");
}
```

- [ ] **Step 6: Run handler tests**

```bash
cargo test -p daemonbox send_input_unknown_session resize_pty_unknown_session -- --nocapture
```

Expected: 2 PASS.

- [ ] **Step 7: Full compile + unit suite**

```bash
cargo check --workspace && cargo xtask test-unit 2>&1 | tail -10
```

- [ ] **Step 8: Commit**

```bash
git add crates/daemonbox/src/handler.rs crates/daemonbox/src/server.rs \
        crates/daemonbox/tests/handler_tests.rs
git commit -m "feat(handler): SendInput + ResizePty dispatch + PtySessionRegistry"
```

---

## Task 5: CLI — `-it` flag, raw mode, stdin relay, SIGWINCH forwarding

**Files:**

- Create: `crates/minibox-cli/src/terminal.rs`
- Modify: `crates/minibox-cli/src/main.rs`
- Modify: `crates/minibox-cli/src/commands/exec.rs`
- Modify: `crates/minibox-cli/src/commands/run.rs`

- [ ] **Step 1: Create `terminal.rs`**

Create `crates/minibox-cli/src/terminal.rs`:

```rust
//! Terminal raw-mode setup/restore and window-size query.

use std::io;
#[cfg(unix)]
use std::os::fd::AsRawFd;

/// Restores terminal settings on drop.
#[cfg(unix)]
pub struct RawModeGuard {
    fd: i32,
    saved: libc::termios,
}

#[cfg(unix)]
impl RawModeGuard {
    /// Put stdin into raw mode.
    pub fn enter() -> io::Result<Self> {
        let fd = io::stdin().as_raw_fd();
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: tcgetattr reads termios for fd; fd is always-open stdin.
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = saved;
        // SAFETY: cfmakeraw sets raw mode on the in-memory struct; no syscall yet.
        unsafe { libc::cfmakeraw(&mut raw) };
        // SAFETY: tcsetattr applies new settings immediately (TCSANOW).
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(RawModeGuard { fd, saved })
    }
}

#[cfg(unix)]
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        // SAFETY: restoring saved termios; fd (stdin) is still valid.
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved) };
    }
}

/// Returns `(cols, rows)` from `TIOCGWINSZ`, or `(80, 24)` as fallback.
#[cfg(unix)]
pub fn terminal_size() -> (u16, u16) {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ ioctl reads window size from stdout fd.
    let ok = unsafe { libc::ioctl(io::stdout().as_raw_fd(), libc::TIOCGWINSZ as _, &mut ws) };
    if ok == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        (ws.ws_col, ws.ws_row)
    } else {
        (80, 24)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn terminal_size_returns_nonzero() {
        // Not a TTY in CI — always returns fallback (80, 24).
        let (cols, rows) = super::terminal_size();
        assert!(cols > 0 && rows > 0);
    }
}
```

- [ ] **Step 2: Register module + check `Cargo.toml`**

In `crates/minibox-cli/src/main.rs`, add:

```rust
pub(crate) mod terminal;
```

Ensure `minibox-cli/Cargo.toml` has under `[target.'cfg(unix)'.dependencies]`:

```toml
libc = { workspace = true }
nix  = { workspace = true }
```

Run `cargo check -p minibox-cli` — fix any missing dep errors.

- [ ] **Step 3: Add `-i`/`-t` flags to `Exec` and `Run` CLI variants**

In `crates/minibox-cli/src/main.rs`, find the `Exec` variant of `Commands` and add:

```rust
    Exec {
        #[arg(value_name = "CONTAINER")]
        container_id: String,
        #[arg(last = true, required = true)]
        cmd: Vec<String>,
        /// Allocate a pseudo-TTY.
        #[arg(short = 't', long = "tty")]
        tty: bool,
        /// Keep stdin open.
        #[arg(short = 'i', long = "interactive")]
        interactive: bool,
    },
```

Update the dispatch arm:

```rust
        Commands::Exec { container_id, cmd, tty, interactive } => {
            commands::exec::execute(container_id, cmd, tty || interactive, socket_path).await
        }
```

Do the same for `Commands::Run` (add `tty: bool, interactive: bool`; pass `tty || interactive`
to `run::execute`).

- [ ] **Step 4: Rewrite `exec::execute` with PTY support**

Replace the entire function in `crates/minibox-cli/src/commands/exec.rs`:

```rust
pub async fn execute(
    container_id: String,
    cmd: Vec<String>,
    tty: bool,
    socket_path: &std::path::Path,
) -> Result<()> {
    use std::io::IsTerminal as _;
    let tty = tty && std::io::stdout().is_terminal();

    let request = DaemonRequest::Exec {
        container_id,
        cmd,
        env: vec![],
        working_dir: None,
        tty,
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client.call(request).await.context("failed to call daemon")?;

    #[cfg(unix)]
    let _raw_guard = if tty {
        Some(crate::terminal::RawModeGuard::enter().context("raw mode enter")?)
    } else {
        None
    };

    let sp = socket_path.to_path_buf();

    while let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ExecStarted { exec_id } => {
                if tty {
                    // Stdin relay task.
                    let sp2 = sp.clone();
                    let sid = exec_id.clone();
                    tokio::spawn(async move {
                        use base64::Engine as _;
                        use tokio::io::AsyncReadExt as _;
                        let mut stdin = tokio::io::stdin();
                        let mut buf = [0u8; 256];
                        loop {
                            match stdin.read(&mut buf).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => {
                                    let data = base64::engine::general_purpose::STANDARD
                                        .encode(&buf[..n]);
                                    let req = DaemonRequest::SendInput {
                                        session_id: sid.clone(),
                                        data,
                                    };
                                    let _ = DaemonClient::with_socket(&sp2).call(req).await;
                                }
                            }
                        }
                    });

                    // Initial terminal size.
                    #[cfg(unix)]
                    {
                        let (cols, rows) = crate::terminal::terminal_size();
                        let _ = DaemonClient::with_socket(&sp)
                            .call(DaemonRequest::ResizePty {
                                session_id: exec_id.clone(),
                                cols,
                                rows,
                            })
                            .await;
                    }

                    // SIGWINCH forwarding thread.
                    #[cfg(unix)]
                    {
                        let sp3 = sp.clone();
                        let sid2 = exec_id.clone();
                        let rt_handle = tokio::runtime::Handle::current();
                        std::thread::spawn(move || {
                            use nix::sys::signal::{SigSet, Signal};
                            let mut mask = SigSet::empty();
                            mask.add(Signal::SIGWINCH);
                            // SAFETY: sigprocmask blocks SIGWINCH in this thread for sigwait.
                            unsafe {
                                libc::sigprocmask(
                                    libc::SIG_BLOCK, mask.as_ref(), std::ptr::null_mut(),
                                );
                            }
                            let mut sig: libc::c_int = 0;
                            loop {
                                // SAFETY: sigwait blocks until a signal in mask arrives.
                                if unsafe { libc::sigwait(mask.as_ref(), &mut sig) } != 0 {
                                    break;
                                }
                                let (cols, rows) = crate::terminal::terminal_size();
                                let sp4 = sp3.clone();
                                let sid3 = sid2.clone();
                                rt_handle.spawn(async move {
                                    let _ = DaemonClient::with_socket(&sp4)
                                        .call(DaemonRequest::ResizePty {
                                            session_id: sid3,
                                            cols,
                                            rows,
                                        })
                                        .await;
                                });
                            }
                        });
                    }
                }
            }
            DaemonResponse::ContainerOutput { data, .. } => {
                use base64::Engine as _;
                use std::io::Write as _;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .context("decode exec output")?;
                std::io::stdout().write_all(&bytes)?;
                std::io::stdout().flush()?;
            }
            DaemonResponse::ContainerStopped { exit_code } => {
                #[cfg(unix)]
                drop(_raw_guard);
                std::process::exit(exit_code);
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("exec: unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 5: Wire `tty` into `run::execute`**

In `crates/minibox-cli/src/commands/run.rs`:

1. Add `tty: bool` parameter to `execute`.
2. Add `tty` to `DaemonRequest::Run { ... }` construction.
3. Set `ephemeral: tty || <existing ephemeral>`.
4. After `DaemonResponse::ContainerCreated { container_id }`, apply the same raw-mode +
   stdin-relay + SIGWINCH pattern as exec Step 4, using `container_id` as `session_id`.

- [ ] **Step 6: Run existing CLI tests**

```bash
cargo test -p minibox-cli -- --nocapture 2>&1 | tail -20
```

Expected: all previously passing tests still pass.

- [ ] **Step 7: Pre-commit gate**

```bash
cargo xtask pre-commit 2>&1 | tail -20
```

Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/minibox-cli/src/terminal.rs crates/minibox-cli/src/main.rs \
        crates/minibox-cli/src/commands/exec.rs crates/minibox-cli/src/commands/run.rs \
        crates/minibox-cli/Cargo.toml
git commit -m "feat(cli): -it flag, raw mode, stdin relay, SIGWINCH forwarding for exec + run"
```

---

## Self-Review

**Spec coverage (minibox-16):**

| Requirement                                        | Task                                                             |
| -------------------------------------------------- | ---------------------------------------------------------------- |
| PTY allocation inside container namespace          | Task 3                                                           |
| Binary framing for output (base64 ContainerOutput) | Reuses existing; Task 3                                          |
| Terminal resize SIGWINCH forwarding                | Task 5 (CLI) + Task 3 (TIOCSWINSZ) + Task 4 (ResizePty dispatch) |
| CLI `-it` flag                                     | Task 5 Step 3                                                    |
| `minibox run -it`                                  | Task 5 Step 5                                                    |
| Protocol `ResizePty` + `SendInput`                 | Task 1                                                           |
| Stdin forwarding                                   | Task 4 (registry + channels) + Task 5 (relay task)               |
| Maestro Phase 2 unblock                            | All tasks combined                                               |

**Type consistency:**

- `ExecConfig.resize_rx` defined Task 2, used Task 3 Step 5.
- `ExecConfig.stdin_tx` defined Task 2, used Task 4 Step 2.
- `PtySessionRegistry` / `SharedPtyRegistry` defined Task 4 Step 1, used Task 4 Steps 2–4.
- `RawModeGuard` defined Task 5 Step 1, used Task 5 Steps 4–5.
- `terminal_size()` defined Task 5 Step 1, used Task 5 Steps 4–5.
- `handle_send_input` / `handle_resize_pty` defined Task 4 Step 3, imported in `server.rs` Task 4 Step 4.
- `run_pipe_exec` / `run_pty_exec` defined Task 3 Steps 4–5, called from `run_exec_blocking` Task 3 Step 3.
