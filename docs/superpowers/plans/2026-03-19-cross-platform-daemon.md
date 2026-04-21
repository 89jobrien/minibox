---
status: done
completed: "2026-03-19"
branch: feat/cross-platform-daemon
note: Platform dispatch, macbox/winbox, ServerListener all shipped
---

# Cross-Platform Daemon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `miniboxd` compile and run natively on Linux, macOS (via Colima), and Windows (via HCS+WSL2) using a platform-dispatched `main` and two new platform orchestration crates.

**Architecture:** `miniboxd/main.rs` gains three `#[cfg(target_os)]`-gated `main` functions; macOS and Windows delegate to `macbox::start()` and `winbox::start()`. `daemonbox` is refactored to accept a `ServerListener` trait so the accept loop is transport-agnostic. `PeerCreds` returned from `accept()` keeps `SO_PEERCRED` logic out of the generic server loop. Phase 1 ships macOS via Colima only; VF types are scaffolded as stubs.

**Tech Stack:** Rust 2024 / 1.85+, Tokio, `nix` (Unix only), `thiserror`, `anyhow`, `dirs` (paths)

---

## File Map

| Action | File                                              | Responsibility                                                                                        |
| ------ | ------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| Modify | `Cargo.toml`                                      | Add `macbox`, `winbox` workspace members + `dirs` dep                                                 |
| Modify | `crates/daemonbox/Cargo.toml`                     | Move `nix` to `cfg(unix)` dep                                                                         |
| Modify | `crates/daemonbox/src/lib.rs`                     | Fix stale `macboxd` doc comment                                                                       |
| Modify | `crates/daemonbox/src/server.rs`                  | Add `ServerListener` + `PeerCreds`; `run_server<L, F>`; generic `handle_connection<S>`                |
| Modify | `crates/daemonbox/src/handler.rs`                 | Gate `nix` with `#[cfg(unix)]`; add `#[cfg(windows)]` stubs for `stop_inner` + `daemon_wait_for_exit` |
| Rename | `crates/minibox/src/adapters/wsl.rs` to `wsl2.rs` | Rename `Wsl*` types to `Wsl2*`                                                                        |
| Modify | `crates/minibox/src/adapters/mod.rs`              | Gate wsl2/docker_desktop by platform; add vf, hcs under cfg                                           |
| Create | `crates/minibox/src/adapters/vf.rs`               | `VfRuntime`, `VfFilesystem`, `VfRegistry` stubs (`cfg(macos)`)                                        |
| Create | `crates/minibox/src/adapters/hcs.rs`              | `HcsRuntime`, `HcsFilesystem`, `HcsRegistry`, `JobObjectLimiter` stubs (`cfg(windows)`)               |
| Create | `crates/macbox/Cargo.toml`                        | macOS-only crate manifest                                                                             |
| Create | `crates/macbox/src/lib.rs`                        | `start()`, `MacboxError`, `colima_deps()`, `UnixServerListener`                                       |
| Create | `crates/macbox/src/preflight.rs`                  | `MacboxStatus`, `preflight()`, `start_colima()`, injectable `Executor`                                |
| Create | `crates/macbox/src/paths.rs`                      | `data_dir()`, `run_dir()`, `socket_path()`                                                            |
| Create | `crates/winbox/Cargo.toml`                        | Windows-only crate manifest                                                                           |
| Create | `crates/winbox/src/lib.rs`                        | `start()`, `WinboxError`                                                                              |
| Create | `crates/winbox/src/preflight.rs`                  | `WinboxStatus`, `preflight()`, injectable `Executor`                                                  |
| Create | `crates/winbox/src/paths.rs`                      | `data_dir()`, `run_dir()`, `pipe_name()`                                                              |
| Create | `crates/winbox/src/hcs.rs`                        | `hcs_deps()` stub                                                                                     |
| Create | `crates/winbox/src/wsl2.rs`                       | `wsl2_deps()` stub                                                                                    |
| Modify | `crates/miniboxd/Cargo.toml`                      | Platform-gated deps: `macbox` on macOS, `winbox` on Windows                                           |
| Modify | `crates/miniboxd/src/main.rs`                     | Remove `compile_error!`; add macOS + Windows `main`; replace inline loop with `run_server`            |
| Modify | `crates/minibox-cli/src/main.rs`                  | Platform-aware `default_socket_path()` + `connect()`                                                  |

---

## Task 1: Workspace plumbing

**Files:** `Cargo.toml`

- [ ] **Step 1: Add workspace members and `dirs` dep**

  In `Cargo.toml`, add to `[workspace] members`:

  ```
  "crates/macbox",
  "crates/winbox",
  ```

  Add to `[workspace.dependencies]`:

  ```toml
  macbox = { path = "crates/macbox" }
  winbox = { path = "crates/winbox" }
  dirs = "5"
  ```

- [ ] **Step 2: Verify workspace metadata parses**

  Run: `cargo metadata --no-deps --format-version 1 2>&1 | head -5`

  Expected: JSON output (crate dirs don't exist yet — that's fine).

---

## Task 2: `daemonbox` — `ServerListener` trait + generic server

**Files:** `crates/daemonbox/src/server.rs`, `crates/daemonbox/Cargo.toml`

The `SO_PEERCRED` auth is lifted into the `ServerListener::accept` return value so `run_server` has no `#[cfg]` blocks. The accept loop moves from `miniboxd/main.rs` into `daemonbox::server::run_server`.

- [ ] **Step 1: Move `nix` to unix-only dep in `daemonbox/Cargo.toml`**

  Replace `nix = { workspace = true }` under `[dependencies]` with:

  ```toml
  [target.'cfg(unix)'.dependencies]
  nix = { workspace = true }
  ```

- [ ] **Step 2: Write failing test for new API types**

  Append to `crates/daemonbox/src/server.rs`:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      #[test]
      fn peer_creds_fields_accessible() {
          let p = PeerCreds { uid: 1000, pid: 42 };
          assert_eq!(p.uid, 1000);
          assert_eq!(p.pid, 42);
      }
  }
  ```

- [ ] **Step 3: Run test to verify it fails**

  Run: `cargo test -p daemonbox 2>&1 | head -10`

  Expected: compile error — `PeerCreds` not found.

- [ ] **Step 4: Rewrite `server.rs`**

  Replace the full file with:

  ```rust
  //! Transport-agnostic daemon connection handler.
  //!
  //! Callers provide a [`ServerListener`] impl — Unix socket or Named Pipe.
  //! [`PeerCreds`] from `accept()` carries SO_PEERCRED data when available.

  use anyhow::{Context, Result};
  use minibox::protocol::{DaemonRequest, DaemonResponse};
  use std::future::Future;
  use std::sync::Arc;
  use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
  use tracing::{debug, error, info, warn};

  use crate::handler::{self, HandlerDependencies};
  use crate::state::DaemonState;

  const MAX_REQUEST_SIZE: usize = 1024 * 1024;

  /// Peer credentials from an accepted connection.
  #[derive(Debug, Clone)]
  pub struct PeerCreds {
      pub uid: u32,
      pub pid: i32,
  }

  /// Platform-agnostic server listener.
  pub trait ServerListener: Send + 'static {
      type Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static;
      async fn accept(&self) -> Result<(Self::Stream, Option<PeerCreds>)>;
  }

  /// Run the daemon accept loop until `shutdown` resolves.
  pub async fn run_server<L, F>(
      listener: L,
      state: Arc<DaemonState>,
      deps: Arc<HandlerDependencies>,
      require_root_auth: bool,
      shutdown: F,
  ) -> Result<()>
  where
      L: ServerListener,
      F: Future<Output = ()>,
  {
      tokio::pin!(shutdown);
      loop {
          tokio::select! {
              accept_result = listener.accept() => {
                  match accept_result {
                      Ok((stream, peer_creds)) => {
                          if let Some(ref creds) = peer_creds {
                              if require_root_auth && creds.uid != 0 {
                                  warn!(uid = creds.uid, pid = creds.pid, "rejecting non-root connection");
                                  continue;
                              }
                              info!(uid = creds.uid, pid = creds.pid, "accepted connection");
                          } else {
                              if require_root_auth {
                                  warn!("peer credentials unavailable; require_root_auth bypassed");
                              }
                              info!("accepted connection (no peer credentials)");
                          }
                          let state_clone = Arc::clone(&state);
                          let deps_clone = Arc::clone(&deps);
                          tokio::spawn(async move {
                              if let Err(e) = handle_connection(stream, state_clone, deps_clone).await {
                                  error!("connection error: {e:#}");
                              }
                          });
                      }
                      Err(e) => error!("accept error: {e}"),
                  }
              }
              _ = &mut shutdown => {
                  info!("shutdown signal received");
                  break;
              }
          }
      }
      Ok(())
  }

  /// Handle a single connection, generic over stream type.
  pub async fn handle_connection<S>(
      stream: S,
      state: Arc<DaemonState>,
      deps: Arc<HandlerDependencies>,
  ) -> Result<()>
  where
      S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
  {
      let (read_half, write_half) = tokio::io::split(stream);
      let mut reader = BufReader::new(read_half);
      let mut writer = BufWriter::new(write_half);
      let mut line = String::new();

      loop {
          line.clear();
          let bytes_read = reader.read_line(&mut line).await.context("reading from client")?;
          if bytes_read == 0 { debug!("client disconnected"); break; }

          if bytes_read > MAX_REQUEST_SIZE {
              warn!("rejecting oversized request: {bytes_read} bytes");
              let err = DaemonResponse::Error { message: format!("request too large ({bytes_read} bytes)") };
              let mut json = serde_json::to_string(&err)?;
              json.push('\n');
              writer.write_all(json.as_bytes()).await?;
              writer.flush().await?;
              continue;
          }

          let trimmed = line.trim();
          if trimmed.is_empty() { continue; }
          debug!("received {} bytes", trimmed.len());

          let response = match serde_json::from_str::<DaemonRequest>(trimmed) {
              Ok(req) => { info!("dispatching: {req:?}"); dispatch(req, Arc::clone(&state), Arc::clone(&deps)).await }
              Err(e) => { warn!("bad request: {e}"); DaemonResponse::Error { message: format!("invalid request: {e}") } }
          };

          let mut json = serde_json::to_string(&response).context("serializing response")?;
          json.push('\n');
          writer.write_all(json.as_bytes()).await.context("writing response")?;
          writer.flush().await.context("flushing")?;
      }
      Ok(())
  }

  async fn dispatch(req: DaemonRequest, state: Arc<DaemonState>, deps: Arc<HandlerDependencies>) -> DaemonResponse {
      match req {
          DaemonRequest::Run { image, tag, command, memory_limit_bytes, cpu_weight } =>
              handler::handle_run(image, tag, command, memory_limit_bytes, cpu_weight, state, deps).await,
          DaemonRequest::Stop { id } => handler::handle_stop(id, state).await,
          DaemonRequest::Remove { id } => handler::handle_remove(id, state, deps).await,
          DaemonRequest::List => handler::handle_list(state).await,
          DaemonRequest::Pull { image, tag } => handler::handle_pull(image, tag, state, deps).await,
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      #[test]
      fn peer_creds_fields_accessible() {
          let p = PeerCreds { uid: 1000, pid: 42 };
          assert_eq!(p.uid, 1000);
          assert_eq!(p.pid, 42);
      }
  }
  ```

- [ ] **Step 5: Run tests**

  Run: `cargo test -p daemonbox`

  Expected: `peer_creds_fields_accessible` passes; all existing tests pass.

- [ ] **Step 6: Fix stale doc in `daemonbox/src/lib.rs`**

  Change any reference to `macboxd` to `miniboxd (macOS)` in the module-level doc comment.

- [ ] **Step 7: Commit**

  ```
  git add crates/daemonbox/
  git commit -m "refactor(daemonbox): ServerListener + PeerCreds + generic run_server/handle_connection"
  ```

---

## Task 3: `daemonbox` — gate `nix` on Unix, add Windows stubs

**Files:** `crates/daemonbox/src/handler.rs`

`nix` is POSIX and works on Linux + macOS. Only Windows needs stubs.

- [ ] **Step 1: Baseline**

  Run: `cargo test -p daemonbox`

- [ ] **Step 2: Gate `stop_inner` with cfg**

  Replace `stop_inner` with two cfg-gated versions:

  ```rust
  #[cfg(unix)]
  async fn stop_inner(id: &str, state: &Arc<DaemonState>) -> Result<()> {
      use nix::sys::signal::{Signal, kill};
      use nix::unistd::Pid;

      let record = state.get_container(id).await
          .ok_or_else(|| DomainError::ContainerNotFound { id: id.to_string() })?;
      let pid = record.pid
          .ok_or_else(|| anyhow::anyhow!("container {id} has no PID"))?;
      let nix_pid = Pid::from_raw(pid as i32);

      info!("sending SIGTERM to container {id} (PID {pid})");
      kill(nix_pid, Signal::SIGTERM).ok();

      let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
      loop {
          tokio::time::sleep(Duration::from_millis(250)).await;
          if kill(nix_pid, None).is_err() { break; }
          if tokio::time::Instant::now() >= deadline {
              warn!("container {id} did not exit in 10s, SIGKILL");
              kill(nix_pid, Signal::SIGKILL).ok();
              break;
          }
      }
      state.update_container_state(id, "Stopped").await;
      Ok(())
  }

  #[cfg(windows)]
  async fn stop_inner(id: &str, _state: &Arc<DaemonState>) -> Result<()> {
      anyhow::bail!(
          "handle_stop not yet implemented on Windows for container {id} \
           — use the HCS/WSL2 adapter stop path"
      )
  }
  ```

  Remove any top-level `use nix::sys::signal` and `use nix::unistd::Pid` imports (move them inside the unix function body as shown above).

- [ ] **Step 3: Gate `daemon_wait_for_exit` with cfg**

  Replace with:

  ```rust
  #[cfg(unix)]
  fn daemon_wait_for_exit(pid: u32, id: &str, state: Arc<DaemonState>) {
      use nix::sys::wait::{WaitStatus, waitpid};
      use nix::unistd::Pid;
      let nix_pid = Pid::from_raw(pid as i32);
      match waitpid(nix_pid, None) {
          Ok(WaitStatus::Exited(_, code)) => info!("container {id} exited with code {code}"),
          Ok(WaitStatus::Signaled(_, sig, _)) => info!("container {id} killed by signal {sig}"),
          Ok(other) => info!("container {id} wait: {other:?}"),
          Err(e) => warn!("waitpid for {id}: {e}"),
      }
      match tokio::runtime::Handle::try_current() {
          Ok(h) => h.block_on(state.update_container_state(id, "Stopped")),
          Err(_) => {
              tokio::runtime::Builder::new_current_thread()
                  .enable_all().build().expect("rt")
                  .block_on(state.update_container_state(id, "Stopped"));
          }
      }
  }

  #[cfg(windows)]
  fn daemon_wait_for_exit(_pid: u32, _id: &str, _state: Arc<DaemonState>) {
      // No-op. Container stays "Running" until explicit stop/remove.
      // Windows adapters track exit via HCS events / wsl.exe polling.
  }
  ```

- [ ] **Step 4: Run tests + clippy**

  Run: `cargo test -p daemonbox && cargo clippy -p daemonbox -- -D warnings`

  Expected: all pass.

- [ ] **Step 5: Commit**

  ```
  git add crates/daemonbox/src/handler.rs crates/daemonbox/Cargo.toml
  git commit -m "refactor(daemonbox): gate nix on cfg(unix), add Windows stubs for stop/wait"
  ```

---

## Task 4: `miniboxd` — wire `UnixServerListener` to `run_server`

**Files:** `crates/miniboxd/src/main.rs`, `crates/miniboxd/Cargo.toml`

- [ ] **Step 1: Update `Cargo.toml`**

  Keep `daemonbox`, `anyhow`, `tokio`, `tracing-subscriber` as unconditional. Move Linux deps:

  ```toml
  [target.'cfg(target_os = "linux")'.dependencies]
  minibox = { workspace = true }
  nix = { workspace = true }
  ```

- [ ] **Step 2: Add `UnixServerListener` under Linux cfg in `main.rs`**

  After the `use` block (keep it under `#[cfg(target_os = "linux")]`), add:

  ```rust
  struct UnixServerListener(tokio::net::UnixListener);

  impl daemonbox::server::ServerListener for UnixServerListener {
      type Stream = tokio::net::UnixStream;
      async fn accept(&self) -> anyhow::Result<(Self::Stream, Option<daemonbox::server::PeerCreds>)> {
          let (stream, _addr) = self.0.accept().await?;
          let creds = {
              use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
              use std::os::unix::io::AsFd;
              getsockopt(&stream.as_fd(), PeerCredentials)
                  .ok()
                  .map(|c| daemonbox::server::PeerCreds { uid: c.uid(), pid: c.pid() })
          };
          Ok((stream, creds))
      }
  }
  ```

- [ ] **Step 3: Replace inline accept loop with `run_server`**

  Find the `loop { tokio::select! { ... } }` block in the Linux `main`. Replace with:

  ```rust
  let shutdown = async move {
      tokio::select! {
          _ = sigterm.recv() => info!("received SIGTERM, shutting down"),
          _ = sigint.recv() => info!("received SIGINT, shutting down"),
      }
  };
  daemonbox::server::run_server(
      UnixServerListener(listener),
      state,
      deps,
      require_root_auth,
      shutdown,
  ).await?;
  ```

  Remove the old `info!("accepted new client connection")` log and spawn block.

- [ ] **Step 4: Add macOS and Windows `main` stubs + remove `compile_error!`**

  Remove: `compile_error!("miniboxd requires Linux");`

  Add after the Linux main:

  ```rust
  #[cfg(target_os = "macos")]
  #[tokio::main]
  async fn main() -> anyhow::Result<()> {
      macbox::start().await
  }

  #[cfg(target_os = "windows")]
  #[tokio::main]
  async fn main() -> anyhow::Result<()> {
      winbox::start().await
  }
  ```

- [ ] **Step 5: Build and test**

  Run: `cargo build -p miniboxd && cargo test -p miniboxd`

  Expected: builds; all integration tests pass.

- [ ] **Step 6: Commit**

  ```
  git add crates/miniboxd/
  git commit -m "feat(miniboxd): remove compile_error, platform dispatch, wire UnixServerListener to run_server"
  ```

---

## Task 5: `minibox` — adapter scaffolding

**Files:** `wsl.rs`→`wsl2.rs`, `mod.rs`, new `vf.rs`, `hcs.rs`

- [ ] **Step 1: Rename wsl.rs to wsl2.rs**

  Run: `git mv crates/minibox/src/adapters/wsl.rs crates/minibox/src/adapters/wsl2.rs`

  In `wsl2.rs`, rename `WslRuntime`→`Wsl2Runtime`, `WslFilesystem`→`Wsl2Filesystem`, `WslLimiter`→`Wsl2Limiter` (struct names and impl blocks).

- [ ] **Step 2: Create `vf.rs` macOS stubs**

  Create `crates/minibox/src/adapters/vf.rs`. Use `colima.rs` as a template for trait impl structure. Implement every method body as `Err(anyhow::anyhow!("VfRuntime: not yet implemented (Phase 2)"))`.

  Types to create: `VfRuntime` (impl `ContainerRuntime`), `VfFilesystem` (impl `FilesystemProvider`), `VfRegistry` (impl `ImageRegistry`).

  Check exact trait method signatures in `crates/minibox/src/domain.rs` before writing.

- [ ] **Step 3: Create `hcs.rs` Windows stubs**

  Same pattern. Types: `HcsRuntime`, `HcsFilesystem`, `HcsRegistry`, `JobObjectLimiter` (impl `ResourceLimiter`). Error: `"HcsRuntime: not yet implemented"`.

- [ ] **Step 4: Update `mod.rs`**

  Replace module declarations and re-exports with platform-gated versions:

  ```rust
  #[cfg(target_os = "linux")] mod filesystem;
  #[cfg(target_os = "linux")] mod limiter;
  mod registry;
  #[cfg(target_os = "linux")] mod runtime;
  mod gke;
  mod colima;
  #[cfg(target_os = "macos")] mod docker_desktop;
  #[cfg(target_os = "macos")] mod vf;
  #[cfg(target_os = "windows")] mod hcs;
  #[cfg(target_os = "windows")] mod wsl2;
  pub mod mocks;

  #[cfg(target_os = "linux")] pub use filesystem::OverlayFilesystem;
  #[cfg(target_os = "linux")] pub use limiter::CgroupV2Limiter;
  pub use registry::DockerHubRegistry;
  #[cfg(target_os = "linux")] pub use runtime::LinuxNamespaceRuntime;
  #[cfg(target_os = "linux")] pub use gke::{CopyFilesystem, NoopLimiter, ProotRuntime};
  pub use colima::{ColimaFilesystem, ColimaLimiter, ColimaRegistry, ColimaRuntime};
  #[cfg(target_os = "macos")] pub use vf::{VfFilesystem, VfRegistry, VfRuntime};
  #[cfg(target_os = "windows")] pub use hcs::{HcsFilesystem, HcsRegistry, HcsRuntime, JobObjectLimiter};
  #[cfg(target_os = "windows")] pub use wsl2::{Wsl2Filesystem, Wsl2Limiter, Wsl2Runtime};
  ```

- [ ] **Step 5: Check and test**

  Run: `cargo check -p minibox && cargo test -p minibox`

  Fix any method signature mismatches by comparing against `domain.rs` and existing adapter impls.

- [ ] **Step 6: Commit**

  ```
  git add crates/minibox/src/adapters/
  git commit -m "feat(minibox): vf/hcs stubs, wsl→wsl2 rename, platform-gate adapters"
  ```

---

## Task 6: `macbox` crate

**Files:** `crates/macbox/`

Phase 1: Colima only. `preflight()` never returns `VirtualizationFramework`.

- [ ] **Step 1: Create `Cargo.toml`**

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
  tracing-subscriber = { workspace = true }
  dirs = { workspace = true }
  ```

- [ ] **Step 2: Write failing tests for `paths`**

  Create `crates/macbox/src/paths.rs` with `todo!()` implementations and test module:

  ```rust
  use std::path::PathBuf;
  pub fn data_dir() -> PathBuf { todo!() }
  pub fn run_dir() -> PathBuf { todo!() }
  pub fn socket_path() -> PathBuf { todo!() }

  #[cfg(test)]
  mod tests {
      use super::*;
      #[test] fn run_dir_is_tmp_minibox() { assert_eq!(run_dir(), PathBuf::from("/tmp/minibox")); }
      #[test] fn socket_under_run_dir() { assert!(socket_path().starts_with(run_dir())); }
      #[test] fn socket_filename() { assert_eq!(socket_path().file_name().unwrap(), "miniboxd.sock"); }
      #[test] fn data_dir_ends_minibox() { assert!(data_dir().ends_with("minibox")); }
  }
  ```

- [ ] **Step 3: Run paths tests — expect failure**

  Run: `cargo test -p macbox paths 2>&1 | head -15`

  Expected: panic from `todo!()`.

- [ ] **Step 4: Implement `paths.rs`**

  ```rust
  use std::path::PathBuf;

  pub fn data_dir() -> PathBuf {
      dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("minibox")
  }
  pub fn run_dir() -> PathBuf { PathBuf::from("/tmp/minibox") }
  pub fn socket_path() -> PathBuf { run_dir().join("miniboxd.sock") }
  ```

- [ ] **Step 5: Run paths tests — expect pass**

  Run: `cargo test -p macbox paths`

  Expected: 4 tests pass.

- [ ] **Step 6: Write failing tests for `preflight`**

  Create `crates/macbox/src/preflight.rs` with test module and `todo!()` bodies:

  ```rust
  use anyhow::Result;

  #[derive(Debug, Clone, PartialEq)]
  pub enum MacboxStatus {
      Colima,
      ColimaNotRunning,
      NoBackendAvailable,
      VirtualizationFramework,
  }

  pub type Executor = Box<dyn Fn(&[&str]) -> Result<String> + Send + Sync>;

  pub fn default_executor() -> Executor { todo!() }
  pub fn preflight(_exec: &Executor) -> MacboxStatus { todo!() }
  pub fn start_colima(_exec: &Executor) -> Result<()> { todo!() }

  #[cfg(test)]
  mod tests {
      use super::*;
      fn ok(s: &'static str) -> Executor { Box::new(move |_| Ok(s.to_string())) }
      fn fail() -> Executor { Box::new(|_| Err(anyhow::anyhow!("not found"))) }

      #[test] fn colima_running() { assert_eq!(preflight(&ok("colima is running")), MacboxStatus::Colima); }
      #[test] fn colima_stopped() { assert_eq!(preflight(&ok("colima is stopped")), MacboxStatus::ColimaNotRunning); }
      #[test] fn no_backend() { assert_eq!(preflight(&fail()), MacboxStatus::NoBackendAvailable); }
      #[test] fn start_ok() { assert!(start_colima(&ok("")).is_ok()); }
  }
  ```

- [ ] **Step 7: Run preflight tests — expect failure**

  Run: `cargo test -p macbox preflight 2>&1 | head -10`

  Expected: panic from `todo!()`.

- [ ] **Step 8: Implement `preflight.rs`**

  ```rust
  pub fn default_executor() -> Executor {
      Box::new(|args: &[&str]| {
          let out = std::process::Command::new(args[0]).args(&args[1..]).output()?;
          Ok(String::from_utf8_lossy(&out.stdout).into_owned())
      })
  }

  pub fn preflight(exec: &Executor) -> MacboxStatus {
      match exec(&["colima", "status"]) {
          Ok(o) if o.contains("running") => MacboxStatus::Colima,
          Ok(_) => MacboxStatus::ColimaNotRunning,
          Err(_) => MacboxStatus::NoBackendAvailable,
      }
  }

  pub fn start_colima(exec: &Executor) -> Result<()> {
      exec(&["colima", "start"])?;
      Ok(())
  }
  ```

- [ ] **Step 9: Run all macbox tests — expect pass**

  Run: `cargo test -p macbox`

  Expected: all pass.

- [ ] **Step 10: Create `src/lib.rs`**

  Create `crates/macbox/src/lib.rs`:

  ```rust
  #[cfg(not(target_os = "macos"))]
  compile_error!("macbox only compiles on macOS");

  pub mod paths;
  pub mod preflight;

  use anyhow::{Context, Result};
  use daemonbox::handler::HandlerDependencies;
  use daemonbox::server::{PeerCreds, ServerListener};
  use minibox::adapters::{ColimaFilesystem, ColimaLimiter, ColimaRegistry, ColimaRuntime};
  use minibox::image::ImageStore;
  use preflight::{MacboxStatus, default_executor};
  use std::path::PathBuf;
  use std::sync::Arc;
  use tokio::net::UnixListener;
  use tokio::signal::unix::{SignalKind, signal};
  use tracing::{info, warn};

  #[derive(thiserror::Error, Debug)]
  pub enum MacboxError {
      #[error("no container backend — install Colima (`brew install colima`)")]
      NoBackendAvailable,
      #[error("Colima VM failed to start: {0}")]
      VmStartFailed(String),
  }

  struct UnixServerListener(UnixListener);

  impl ServerListener for UnixServerListener {
      type Stream = tokio::net::UnixStream;
      async fn accept(&self) -> anyhow::Result<(Self::Stream, Option<PeerCreds>)> {
          let (stream, _) = self.0.accept().await?;
          Ok((stream, None))
      }
  }

  pub fn colima_deps(
      containers_base: PathBuf,
      run_containers_base: PathBuf,
      image_store: Arc<ImageStore>,
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

  pub async fn start() -> Result<()> {
      tracing_subscriber::fmt()
          .with_env_filter(
              tracing_subscriber::EnvFilter::from_default_env()
                  .add_directive("miniboxd=info".parse().unwrap()),
          )
          .init();

      info!("miniboxd (macOS) starting");
      let exec = default_executor();
      let mut status = preflight::preflight(&exec);

      if status == MacboxStatus::ColimaNotRunning {
          info!("Colima stopped — starting...");
          preflight::start_colima(&exec)
              .map_err(|e| MacboxError::VmStartFailed(e.to_string()))?;
          status = MacboxStatus::Colima;
      }

      if status != MacboxStatus::Colima {
          return Err(MacboxError::NoBackendAvailable.into());
      }

      let data_dir = std::env::var("MINIBOX_DATA_DIR").map(PathBuf::from)
          .unwrap_or_else(|_| paths::data_dir());
      let run_dir = std::env::var("MINIBOX_RUN_DIR").map(PathBuf::from)
          .unwrap_or_else(|_| paths::run_dir());
      let socket_path = std::env::var("MINIBOX_SOCKET_PATH").map(PathBuf::from)
          .unwrap_or_else(|_| paths::socket_path());

      for dir in &[data_dir.join("images"), data_dir.join("containers"),
                   run_dir.join("containers"), run_dir.clone()] {
          std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
      }

      // Check ImageStore::new signature in minibox/src/image/store.rs — adjust as needed.
      let image_store = Arc::new(ImageStore::new(&data_dir.join("images")).context("image store")?);
      // Check DaemonState::new signature in daemonbox/src/state.rs — adjust as needed.
      let state = Arc::new(daemonbox::state::DaemonState::new(Arc::clone(&image_store), &data_dir));
      state.load_from_disk().await;

      let deps = Arc::new(colima_deps(data_dir.join("containers"), run_dir.join("containers"), image_store));

      if socket_path.exists() {
          warn!("removing stale socket");
          std::fs::remove_file(&socket_path)?;
      }
      let listener = UnixListener::bind(&socket_path)
          .with_context(|| format!("binding {}", socket_path.display()))?;
      info!("listening on {}", socket_path.display());

      let mut sigterm = signal(SignalKind::terminate()).context("SIGTERM")?;
      let mut sigint = signal(SignalKind::interrupt()).context("SIGINT")?;
      let shutdown = async move {
          tokio::select! {
              _ = sigterm.recv() => info!("SIGTERM"),
              _ = sigint.recv() => info!("SIGINT"),
          }
      };

      daemonbox::server::run_server(UnixServerListener(listener), state, deps, false, shutdown).await?;

      let _ = std::fs::remove_file(&socket_path);
      info!("miniboxd (macOS) stopped");
      Ok(())
  }
  ```

  > **Before building:** check `ImageStore::new` in `minibox/src/image/` and `DaemonState::new` in `daemonbox/src/state.rs` for exact signatures. Adjust the constructor calls in `start()` to match.

- [ ] **Step 11: Build (on macOS)**

  Run: `cargo build -p macbox`

  Fix any constructor mismatches from Step 10.

- [ ] **Step 12: Commit**

  ```
  git add crates/macbox/
  git commit -m "feat(macbox): Colima preflight, paths, adapter wiring, start() entry point"
  ```

---

## Task 7: `winbox` crate

**Files:** `crates/winbox/`

Phase 1: all adapter wiring returns `Err`. Crate must compile on Windows.

- [ ] **Step 1: Create `Cargo.toml`**

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
  tracing-subscriber = { workspace = true }
  dirs = { workspace = true }
  ```

- [ ] **Step 2: Create `src/paths.rs` with tests**

  ```rust
  use std::path::PathBuf;

  pub fn data_dir() -> PathBuf {
      dirs::data_dir().unwrap_or_else(|| PathBuf::from("C:\\minibox")).join("minibox")
  }
  pub fn run_dir() -> PathBuf {
      dirs::cache_dir().unwrap_or_else(|| PathBuf::from("C:\\Temp")).join("minibox")
  }
  pub fn pipe_name() -> String { r"\\.\pipe\miniboxd".to_string() }

  #[cfg(test)]
  mod tests {
      use super::*;
      #[test] fn pipe_has_prefix() { assert!(pipe_name().starts_with(r"\\.\pipe\")); }
      #[test] fn data_dir_ends_minibox() { assert!(data_dir().ends_with("minibox")); }
  }
  ```

- [ ] **Step 3: Create `src/preflight.rs` with tests**

  ```rust
  use anyhow::Result;

  #[derive(Debug, Clone, PartialEq)]
  pub enum WinboxStatus { Hcs, Wsl2, HcsAndWsl2, NoBackendAvailable }

  pub type Executor = Box<dyn Fn(&[&str]) -> Result<String> + Send + Sync>;

  pub fn default_executor() -> Executor {
      Box::new(|args: &[&str]| {
          let out = std::process::Command::new(args[0]).args(&args[1..]).output()?;
          Ok(String::from_utf8_lossy(&out.stdout).into_owned())
      })
  }

  fn check_hcs(exec: &Executor) -> bool {
      exec(&["powershell", "-Command",
             "Get-WindowsOptionalFeature -Online -FeatureName Containers | Select-Object -ExpandProperty State"])
          .map(|o| o.trim() == "Enabled")
          .unwrap_or(false)
  }

  fn check_wsl2(exec: &Executor) -> bool {
      exec(&["wsl", "--status"]).map(|o| !o.is_empty()).unwrap_or(false)
  }

  pub fn preflight(exec: &Executor) -> WinboxStatus {
      match (check_hcs(exec), check_wsl2(exec)) {
          (true, true) => WinboxStatus::HcsAndWsl2,
          (true, false) => WinboxStatus::Hcs,
          (false, true) => WinboxStatus::Wsl2,
          _ => WinboxStatus::NoBackendAvailable,
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;
      fn fail() -> Executor { Box::new(|_| Err(anyhow::anyhow!("not found"))) }
      #[test] fn no_backend_when_both_fail() {
          assert_eq!(preflight(&fail()), WinboxStatus::NoBackendAvailable);
      }
  }
  ```

- [ ] **Step 4: Create `src/hcs.rs` and `src/wsl2.rs` stubs**

  `hcs.rs`:

  ```rust
  use anyhow::{Result, anyhow};
  use daemonbox::handler::HandlerDependencies;
  use std::path::PathBuf;
  pub fn hcs_deps(_c: PathBuf, _r: PathBuf) -> Result<HandlerDependencies> {
      Err(anyhow!("HCS not yet implemented"))
  }
  ```

  `wsl2.rs`:

  ```rust
  use anyhow::{Result, anyhow};
  use daemonbox::handler::HandlerDependencies;
  use std::path::PathBuf;
  pub fn wsl2_deps(_c: PathBuf, _r: PathBuf) -> Result<HandlerDependencies> {
      Err(anyhow!("WSL2 not yet implemented"))
  }
  ```

- [ ] **Step 5: Create `src/lib.rs`**

  ```rust
  #[cfg(not(target_os = "windows"))]
  compile_error!("winbox only compiles on Windows");

  pub mod hcs;
  pub mod paths;
  pub mod preflight;
  pub mod wsl2;

  use anyhow::Result;
  use preflight::{WinboxStatus, default_executor};
  use tracing::info;

  #[derive(thiserror::Error, Debug)]
  pub enum WinboxError {
      #[error("no backend — enable Windows Containers or install WSL2")]
      NoBackendAvailable,
      #[error("Administrator required for HCS")]
      NotAdministrator,
  }

  pub async fn start() -> Result<()> {
      tracing_subscriber::fmt()
          .with_env_filter(
              tracing_subscriber::EnvFilter::from_default_env()
                  .add_directive("miniboxd=info".parse().unwrap()),
          )
          .init();

      info!("miniboxd (Windows) starting");
      let exec = default_executor();
      let status = preflight::preflight(&exec);
      let adapter = std::env::var("MINIBOX_ADAPTER").unwrap_or_default();

      match (adapter.as_str(), &status) {
          ("wsl2", _) | (_, WinboxStatus::Wsl2) => {
              anyhow::bail!("WSL2 server loop not yet implemented (Phase 1 stub)")
          }
          ("hcs", _) | (_, WinboxStatus::Hcs) | (_, WinboxStatus::HcsAndWsl2) => {
              anyhow::bail!("HCS server loop not yet implemented (Phase 1 stub)")
          }
          _ => Err(WinboxError::NoBackendAvailable.into()),
      }
  }
  ```

- [ ] **Step 6: Run tests (Windows only; skip on Linux/macOS)**

  On Windows: `cargo test -p winbox`

  On Linux/macOS: skip (compile_error gates lib.rs). Paths/preflight modules are tested on all platforms if included without lib.rs — not needed here since they're Windows-targeted anyway.

- [ ] **Step 7: Commit**

  ```
  git add crates/winbox/
  git commit -m "feat(winbox): Windows orchestration crate — preflight, paths, start() stub"
  ```

---

## Task 8: `minibox-cli` — platform-aware transport

**Files:** `crates/minibox-cli/src/main.rs`

- [ ] **Step 1: Read current connect code**

  Run: `grep -n "UnixStream\|connect\|socket_path\|SOCKET" crates/minibox-cli/src/main.rs | head -20`

  Note the line numbers of the socket path variable and connect call.

- [ ] **Step 2: Write failing test**

  Add to `main.rs`:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      #[test]
      fn default_socket_path_non_empty() {
          assert!(!default_socket_path().is_empty());
      }
  }
  ```

  Run: `cargo test -p minibox-cli 2>&1 | head -10`

  Expected: `default_socket_path` not found.

- [ ] **Step 3: Add `default_socket_path()`**

  Add before `main`:

  ```rust
  fn default_socket_path() -> String {
      #[cfg(target_os = "linux")]
      { std::env::var("MINIBOX_SOCKET_PATH").unwrap_or_else(|_| "/run/minibox/miniboxd.sock".to_string()) }
      #[cfg(target_os = "macos")]
      { std::env::var("MINIBOX_SOCKET_PATH").unwrap_or_else(|_| "/tmp/minibox/miniboxd.sock".to_string()) }
      #[cfg(target_os = "windows")]
      { std::env::var("MINIBOX_PIPE_NAME").unwrap_or_else(|_| r"\\.\pipe\miniboxd".to_string()) }
  }
  ```

  Replace the existing hardcoded socket path string with a call to `default_socket_path()`.

- [ ] **Step 4: Run CLI tests**

  Run: `cargo test -p minibox-cli`

  Expected: all pass including `default_socket_path_non_empty`.

- [ ] **Step 5: Commit**

  ```
  git add crates/minibox-cli/
  git commit -m "feat(minibox-cli): platform-aware default socket/pipe path"
  ```

---

## Task 9: Final verification

- [ ] **Step 1: Linux workspace build**

  Run: `cargo build --workspace --exclude macbox --exclude winbox`

  Expected: success.

- [ ] **Step 2: Full test suite**

  Run: `cargo test --workspace --exclude macbox --exclude winbox`

  Expected: all pass.

- [ ] **Step 3: Clippy**

  Run: `cargo clippy --workspace --exclude macbox --exclude winbox -- -D warnings`

  Expected: no warnings.

- [ ] **Step 4: Format check**

  Run: `cargo fmt --all --check`

  Expected: no diff.

- [ ] **Step 5: macOS smoke test (if on macOS with Colima running)**

  ```
  MINIBOX_ADAPTER=colima cargo run -p miniboxd &
  sleep 2
  sudo cargo run -p minibox-cli -- pull alpine
  sudo cargo run -p minibox-cli -- run alpine -- /bin/echo hello
  ```

  Expected: "hello" output.

- [ ] **Step 6: Tag**

  ```
  git tag cross-platform-phase-1
  ```
