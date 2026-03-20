---
status: archived
completed: "2026-03-18"
superseded_by: 2026-03-19-cross-platform-daemon.md
note: Superseded by cross-platform plan; winbox implemented
---
# winboxd — WSL2 Named Pipe Proxy Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `winboxd.exe` — a Windows Named Pipe proxy that relays the minibox JSON-over-newline protocol to `miniboxd` running unmodified inside a WSL2 distro.

**Architecture:** `winboxd.exe` listens on `\\.\pipe\miniboxd`. For each client connection it spawns `wsl.exe -- socat - UNIX-CONNECT:/run/minibox/miniboxd.sock` as a child process and relays raw bytes between the Named Pipe and the child's stdin/stdout. `miniboxd` runs inside WSL2 with the native Linux adapter — no changes to `miniboxd` or any adapter. Windows CLI clients set `WINBOX_PIPE_PATH` (or use the default); Linux CLI clients running inside WSL2 connect directly to the Unix socket as before.

```
Windows CLI  →  \\.\pipe\miniboxd  →  winboxd.exe  →  wsl.exe socat  →  /run/minibox/miniboxd.sock  →  miniboxd (WSL2)
```

**Roadmap note:** This proxy approach is the first stepping stone. The eventual goal is a native `winboxd.exe` that runs container operations directly on Windows (Hyper-V isolation or Windows containers) without requiring a WSL2 relay. That native backend is a separate plan; this plan ships the proxy so the CLI works on Windows today.

**Non-goals:** Docker Desktop integration, native Windows container backend, `wslapi` crate, Windows-native CLI support, `daemonbox` refactor. Those are separate plans.

**Tech Stack:** Rust workspace, `tokio::net::windows::named_pipe` (already in workspace `tokio = { features = ["full"] }`), `anyhow`, `thiserror`, `tracing`.

**Note on auth:** `miniboxd` with the native adapter requires clients to have UID 0 (SO_PEERCRED check). The `socat` child runs as the user who started `winboxd`. For local development, start `miniboxd` in WSL2 with `MINIBOX_SOCKET_MODE=0666` to allow any user to connect. Production hardening (socket group, sudoers) is out of scope.

**Prerequisite:** `socat` installed in the WSL2 distro (`sudo apt install socat`).

---

## File Map

### New files

| File                             | Responsibility                                                     |
| -------------------------------- | ------------------------------------------------------------------ |
| `crates/winbox/Cargo.toml`       | Crate manifest                                                     |
| `crates/winbox/src/lib.rs`       | `pub mod preflight; pub mod paths;` + re-exports                   |
| `crates/winbox/src/preflight.rs` | `Wsl2Status`, `WinboxError`, `preflight()`, `ensure_wsl_running()` |
| `crates/winbox/src/paths.rs`     | Windows default pipe name and WSL socket path                      |
| `crates/winboxd/Cargo.toml`      | Crate manifest                                                     |
| `crates/winboxd/src/main.rs`     | Named Pipe accept loop + socat relay + signal handling             |

### Modified files

| File         | Change                                              |
| ------------ | --------------------------------------------------- |
| `Cargo.toml` | Add `winbox`, `winboxd` to workspace members + deps |

---

## Task 1: `winbox` library crate

**Files:**

- Create: `crates/winbox/Cargo.toml`
- Create: `crates/winbox/src/lib.rs`
- Create: `crates/winbox/src/preflight.rs`
- Create: `crates/winbox/src/paths.rs`
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Add `winbox` to workspace**

In root `Cargo.toml`, add to `[workspace]` members:

```toml
"crates/winbox",
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
anyhow = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

- [ ] **Step 3: Create directory structure**

```bash
mkdir -p crates/winbox/src crates/winboxd/src
```

- [ ] **Step 4: Write failing tests — create stub `crates/winbox/src/preflight.rs`**

Write the types, error variants, and tests but leave the function bodies as `todo!()`:

```rust
//! WSL2 availability checks.
//!
//! Shells out to `wsl.exe` — no `wslapi.dll` binding needed.
//!
//! Note on distro detection: `wsl --list --quiet` emits UTF-16LE on Windows,
//! which is unparseable with `from_utf8_lossy`. Instead we use
//! `wsl.exe -d <distro> --exec echo ok` — its exit code is the reliable signal.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wsl2Status {
    /// WSL2 is installed and the miniboxd socket is present in the distro.
    Running,
    /// WSL2 is installed and the distro is registered, but the socket is absent.
    Stopped,
    /// `wsl.exe` is not on PATH or the command fails to execute.
    NotInstalled,
}

#[derive(Error, Debug)]
pub enum WinboxError {
    #[error("WSL2 is not installed — run: wsl --install")]
    Wsl2NotInstalled,
    #[error("WSL2 distro '{0}' is not registered — run: wsl --install -d <distro>")]
    DistroNotRegistered(String),
    #[error("socat is not installed in WSL2 distro '{0}' — run: sudo apt install socat")]
    SocatNotInstalled(String),
    #[error("WSL2 startup failed: {0}")]
    StartFailed(String),
    #[error("preflight check failed: {0}")]
    PreflightFailed(String),
}

/// Check WSL2 availability, distro registration, `socat` presence, and socket state.
pub fn preflight(distro: &str, socket_path: &str) -> Result<Wsl2Status, WinboxError> {
    todo!()
}

/// Start `miniboxd` inside the WSL2 distro (background) and poll for socket.
///
/// Only meaningful on Windows — gated so the crate compiles on all platforms.
#[cfg(target_os = "windows")]
pub async fn ensure_wsl_running(distro: &str, socket_path: &str) -> Result<(), WinboxError> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_variants_are_distinct() {
        assert_ne!(Wsl2Status::Running, Wsl2Status::Stopped);
        assert_ne!(Wsl2Status::Stopped, Wsl2Status::NotInstalled);
    }

    #[test]
    fn error_messages_are_actionable() {
        let e = WinboxError::Wsl2NotInstalled;
        assert!(e.to_string().contains("wsl --install"));

        let e = WinboxError::DistroNotRegistered("Ubuntu".into());
        assert!(e.to_string().contains("Ubuntu"));

        let e = WinboxError::SocatNotInstalled("Ubuntu".into());
        assert!(e.to_string().contains("socat"));
    }
}
```

- [ ] **Step 5: Run failing tests to confirm they panic on `todo!()`**

```bash
cargo nextest run -p winbox
```

Expected: `status_variants_are_distinct` and `error_messages_are_actionable` **pass** (they only construct types, no `todo!()` involved). Compilation succeeds.

- [ ] **Step 6: Implement `preflight()`**

Replace the `todo!()` in `preflight()`:

```rust
pub fn preflight(distro: &str, socket_path: &str) -> Result<Wsl2Status, WinboxError> {
    use std::process::Command;

    // Is wsl.exe accessible? Use `wsl.exe --status`; failure-to-spawn means not installed.
    match Command::new("wsl.exe").arg("--status").output() {
        Err(_) => return Ok(Wsl2Status::NotInstalled),
        Ok(o) if !o.status.success() => return Ok(Wsl2Status::NotInstalled),
        Ok(_) => {}
    }

    // Is the distro registered?
    // Avoid `wsl --list --quiet` — it emits UTF-16LE which is not UTF-8.
    // Instead probe by running a trivial command in the distro.
    let distro_ok = Command::new("wsl.exe")
        .args(["-d", distro, "--exec", "echo", "ok"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !distro_ok {
        return Err(WinboxError::DistroNotRegistered(distro.to_string()));
    }

    // Is socat installed in the distro?
    let socat_ok = Command::new("wsl.exe")
        .args(["-d", distro, "--", "which", "socat"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !socat_ok {
        return Err(WinboxError::SocatNotInstalled(distro.to_string()));
    }

    // Is the miniboxd socket present?
    let sock_ok = Command::new("wsl.exe")
        .args(["-d", distro, "--", "test", "-S", socket_path])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if sock_ok {
        Ok(Wsl2Status::Running)
    } else {
        Ok(Wsl2Status::Stopped)
    }
}
```

- [ ] **Step 7: Implement `ensure_wsl_running()`**

Replace the `todo!()` in `ensure_wsl_running()`:

```rust
#[cfg(target_os = "windows")]
pub async fn ensure_wsl_running(distro: &str, socket_path: &str) -> Result<(), WinboxError> {
    match preflight(distro, socket_path)? {
        Wsl2Status::Running => return Ok(()),
        Wsl2Status::NotInstalled => return Err(WinboxError::Wsl2NotInstalled),
        Wsl2Status::Stopped => {}
    }

    // Start miniboxd in the background inside WSL2 (as root, socket open to all users).
    tokio::process::Command::new("wsl.exe")
        .args([
            "-d", distro,
            "--",
            "sudo", "sh", "-c",
            "MINIBOX_SOCKET_MODE=0666 nohup miniboxd >/tmp/miniboxd.log 2>&1 &",
        ])
        .spawn()
        .map_err(|e| WinboxError::StartFailed(e.to_string()))?;

    // Poll until the socket appears (up to 10s).
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if matches!(preflight(distro, socket_path)?, Wsl2Status::Running) {
            return Ok(());
        }
    }
    Err(WinboxError::StartFailed(
        "miniboxd socket did not appear after 10s".to_string(),
    ))
}
```

- [ ] **Step 8: Run tests again — confirm they pass**

```bash
cargo nextest run -p winbox
```

Expected: all tests pass.

- [ ] **Step 9: Write `crates/winbox/src/paths.rs`**

```rust
//! Windows default paths and pipe names for winboxd.

/// Default WSL2 distro name used by minibox.
pub const DEFAULT_DISTRO: &str = "Ubuntu";

/// Windows Named Pipe name.
pub const DEFAULT_PIPE: &str = r"\\.\pipe\miniboxd";

/// Path of miniboxd Unix socket inside the WSL2 distro.
pub const DEFAULT_WSL_SOCKET: &str = "/run/minibox/miniboxd.sock";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_name_is_valid_windows_path() {
        assert!(DEFAULT_PIPE.starts_with(r"\\.\pipe\"));
    }

    #[test]
    fn wsl_socket_is_absolute() {
        assert!(DEFAULT_WSL_SOCKET.starts_with('/'));
    }
}
```

- [ ] **Step 10: Write `crates/winbox/src/lib.rs`**

```rust
//! Windows orchestration library for winboxd.
//!
//! Provides WSL2 preflight checks and default path/pipe constants.
//! Compiles on all platforms for cross-platform CI.
//! `ensure_wsl_running` is gated to `#[cfg(target_os = "windows")]`.

pub mod paths;
pub mod preflight;

pub use paths::{DEFAULT_DISTRO, DEFAULT_PIPE, DEFAULT_WSL_SOCKET};
pub use preflight::{Wsl2Status, WinboxError, preflight};

#[cfg(target_os = "windows")]
pub use preflight::ensure_wsl_running;
```

- [ ] **Step 11: Fmt, clippy, tests**

```bash
cargo fmt -p winbox --check
cargo clippy -p winbox -- -D warnings
cargo nextest run -p winbox
```

Expected: no diff, no warnings, all tests pass.

- [ ] **Step 12: Commit**

```bash
git add crates/winbox/ Cargo.toml
git commit -m "feat: add winbox crate — WSL2 preflight and path constants"
```

---

## Task 2: `winboxd` binary

**Files:**

- Create: `crates/winboxd/Cargo.toml`
- Create: `crates/winboxd/src/main.rs`
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Add `winboxd` to workspace**

In root `Cargo.toml`, add to `[workspace]` members:

```toml
"crates/winboxd",
```

No workspace.dependencies entry needed (it's a binary, nothing depends on it).

- [ ] **Step 2: Create `crates/winboxd/Cargo.toml`**

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
tokio = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

- [ ] **Step 3: Create `crates/winboxd/src/main.rs`**

```rust
//! winboxd — Windows Named Pipe proxy for miniboxd running in WSL2.
//!
//! Accepts connections on `\\.\pipe\miniboxd` (or `WINBOX_PIPE_PATH`) and
//! relays raw bytes to miniboxd's Unix socket inside WSL2 via a socat bridge.
//!
//! # Environment variables
//!
//! | Variable               | Default                      | Purpose                         |
//! |------------------------|------------------------------|---------------------------------|
//! | `WINBOX_PIPE_PATH`     | `\\.\pipe\miniboxd`          | Named Pipe to listen on         |
//! | `WINBOX_WSL_DISTRO`    | `Ubuntu`                     | WSL2 distro containing miniboxd |
//! | `WINBOX_WSL_SOCKET`    | `/run/minibox/miniboxd.sock` | Unix socket path inside WSL2    |
//! | `WINBOX_SKIP_PREFLIGHT`| unset                        | Set to `1` to skip preflight    |

#[cfg(not(target_os = "windows"))]
compile_error!("winboxd requires Windows");

use anyhow::{Context, Result};
use tokio::io;
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use tracing::{error, info, warn};
use winbox::{DEFAULT_DISTRO, DEFAULT_PIPE, DEFAULT_WSL_SOCKET};
use winbox::{Wsl2Status, ensure_wsl_running, preflight};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("winboxd=info".parse().unwrap()),
        )
        .init();

    info!("winboxd starting");

    let pipe_path = std::env::var("WINBOX_PIPE_PATH")
        .unwrap_or_else(|_| DEFAULT_PIPE.to_string());
    let distro = std::env::var("WINBOX_WSL_DISTRO")
        .unwrap_or_else(|_| DEFAULT_DISTRO.to_string());
    let wsl_socket = std::env::var("WINBOX_WSL_SOCKET")
        .unwrap_or_else(|_| DEFAULT_WSL_SOCKET.to_string());
    let skip_preflight = std::env::var("WINBOX_SKIP_PREFLIGHT")
        .map(|v| v == "1")
        .unwrap_or(false);

    // ── Preflight ─────────────────────────────────────────────────────────
    if !skip_preflight {
        match preflight(&distro, &wsl_socket).context("WSL2 preflight failed")? {
            Wsl2Status::NotInstalled => {
                anyhow::bail!("WSL2 is not installed. Run: wsl --install");
            }
            Wsl2Status::Stopped => {
                info!("miniboxd not running in WSL2 — starting");
                ensure_wsl_running(&distro, &wsl_socket)
                    .await
                    .context("failed to start miniboxd in WSL2")?;
            }
            Wsl2Status::Running => {
                info!("miniboxd is running in WSL2 distro '{distro}'");
            }
        }
    } else {
        warn!("WINBOX_SKIP_PREFLIGHT=1 — skipping WSL2 checks");
    }

    info!("listening on {pipe_path}");

    // ── Accept loop ───────────────────────────────────────────────────────
    // first_pipe_instance(true) on the first call so the OS rejects a second
    // winboxd instance trying to claim the same pipe name. Subsequent calls in
    // the loop pass false to create additional server instances for new clients.
    let mut first_instance = true;

    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(first_instance)
            .pipe_mode(PipeMode::Byte)
            .create(&pipe_path)
            .with_context(|| format!("creating named pipe {pipe_path}"))?;
        first_instance = false;

        // Wait for a client or a Ctrl+C signal.
        tokio::select! {
            result = server.connect() => {
                result.context("waiting for pipe client")?;
                info!("client connected");
                let distro_c = distro.clone();
                let socket_c = wsl_socket.clone();
                tokio::spawn(async move {
                    if let Err(e) = relay(server, &distro_c, &socket_c).await {
                        error!("relay error: {e:#}");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => {
                info!("received Ctrl+C, shutting down");
                break;
            }
        }
    }

    info!("winboxd stopped");
    Ok(())
}

/// Relay bytes between a Named Pipe client and `socat` inside WSL2.
///
/// Spawns: `wsl.exe -d <distro> -- socat - UNIX-CONNECT:<socket>`
/// then copies in both directions until either side closes.
async fn relay(pipe: NamedPipeServer, distro: &str, wsl_socket: &str) -> Result<()> {
    let mut child = tokio::process::Command::new("wsl.exe")
        .args([
            "-d", distro,
            "--",
            "socat", "-",
            &format!("UNIX-CONNECT:{wsl_socket}"),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawning socat bridge")?;

    let mut child_stdin = child.stdin.take().unwrap();
    let mut child_stdout = child.stdout.take().unwrap();
    let (mut pipe_reader, mut pipe_writer) = io::split(pipe);

    // Copy in both directions; stop when either side closes.
    tokio::select! {
        r = io::copy(&mut pipe_reader, &mut child_stdin) => {
            r.context("pipe→socat copy")?;
        }
        r = io::copy(&mut child_stdout, &mut pipe_writer) => {
            r.context("socat→pipe copy")?;
        }
    }

    let _ = child.kill().await;
    Ok(())
}
```

- [ ] **Step 4: Build `winboxd` on Windows**

```bash
cargo build -p winboxd
```

Expected: `Finished` — binary at `target\debug\winboxd.exe`. Fix any compilation errors before proceeding.

- [ ] **Step 5: Check `winbox` compiles on Linux/macOS (CI gate)**

```bash
# Run on macOS or Linux (omit winboxd — it has compile_error! on non-Windows)
cargo check -p winbox -p minibox-lib
```

Expected: no errors.

- [ ] **Step 6: Clippy + fmt on winboxd (Windows)**

```bash
cargo fmt -p winboxd --check
cargo clippy -p winboxd -- -D warnings
```

Expected: no diff, no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/winboxd/ Cargo.toml
git commit -m "feat: add winboxd — Windows Named Pipe proxy for WSL2 miniboxd"
```

---

## Task 3: End-to-end smoke test

**Prerequisite:** Windows machine with WSL2, `socat` installed in WSL2 distro, `miniboxd` binary available inside the distro.

- [ ] **Step 1: Start `miniboxd` in WSL2**

Inside WSL2 (run in a WSL2 terminal):

```bash
# Install socat if missing
sudo apt install -y socat

# Start miniboxd with open socket permissions so the socat bridge can connect
sudo MINIBOX_SOCKET_MODE=0666 ./target/release/miniboxd
```

Expected log:

```
INFO miniboxd starting
INFO adapter suite: Native
INFO listening on /run/minibox/miniboxd.sock
```

- [ ] **Step 2: Start `winboxd.exe` on Windows**

In a Windows terminal:

```cmd
set WINBOX_WSL_DISTRO=Ubuntu
.\target\debug\winboxd.exe
```

Expected log:

```
INFO winboxd starting
INFO miniboxd is running in WSL2 distro 'Ubuntu'
INFO listening on \\.\pipe\miniboxd
```

- [ ] **Step 3: Pipe-level smoke test via PowerShell**

```powershell
$pipe = [System.IO.Pipes.NamedPipeClientStream]::new('.', 'miniboxd',
    [System.IO.Pipes.PipeDirection]::InOut)
$pipe.Connect(5000)
$writer = [System.IO.StreamWriter]::new($pipe)
$reader = [System.IO.StreamReader]::new($pipe)
$writer.WriteLine('{"type":"ListContainers"}')
$writer.Flush()
$response = $reader.ReadLine()
Write-Output "Response: $response"
$pipe.Dispose()
```

Expected: JSON response containing the container list (e.g. `{"containers":[]}`).

- [ ] **Step 4: Verify clean shutdown**

Press `Ctrl+C` in the winboxd terminal. Expected log:

```
INFO received Ctrl+C, shutting down
INFO winboxd stopped
```

- [ ] **Step 5: Final commit**

```bash
git add -p
git commit -m "chore: winboxd smoke test verified"
```

---

## Verification Summary

| Check                           | Command                                   | Platform | Expected               |
| ------------------------------- | ----------------------------------------- | -------- | ---------------------- |
| winbox compiles (all platforms) | `cargo check -p winbox`                   | any      | No errors              |
| winbox tests                    | `cargo nextest run -p winbox`             | any      | All pass               |
| winboxd compiles                | `cargo build -p winboxd`                  | Windows  | Binary produced        |
| winboxd clippy                  | `cargo clippy -p winboxd -- -D warnings`  | Windows  | No warnings            |
| miniboxd unaffected             | `cargo nextest run -p miniboxd`           | Linux    | No regressions         |
| End-to-end relay                | PowerShell pipe client → miniboxd in WSL2 | Windows  | JSON response received |
| Graceful shutdown               | Ctrl+C in winboxd terminal                | Windows  | "winboxd stopped" log  |
