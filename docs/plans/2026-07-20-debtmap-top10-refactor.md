# Plan: Debtmap Top-10 Bottom-Up Complexity Refactor

## Goal

Reduce real complexity in `run_daemon()`, `exec::execute()`, and split the
`minibox-core::domain` god-object, per the approved design at
`docs/designs/2026-07-20-debtmap-top10-refactor-design.md`, with zero behavior change and
zero breaking API changes.

## Architecture

- Crates affected: `miniboxd` (Phase 1a), `mbx` (Phase 1b), `minibox-core` (Phase 2).
- No new traits/types — pure extraction/reorganization.
- Data flow: unchanged in all three phases (see design doc §Data Flow per phase).

## Tech Stack

- Rust edition 2024, existing workspace. No new dependencies.

## General rules for every task

- Never use `.unwrap()`/`.expect()` in the extracted production code (test code may keep
  existing `.unwrap()` usage under its existing `#[allow(...)]`).
- Preserve all existing `tracing::info!/warn!` structured fields verbatim — do not
  reformat them into message strings.
- Preserve all existing `#[cfg(...)]` gates exactly; do not widen or narrow them.
- Run `git branch --show-current` before every commit in this plan. If the result is `main`,
  stop and do not commit.

---

## Phase 1a — `crates/miniboxd/src/main.rs::run_daemon()`

### Task 1: Extract `init_daemon_tracing`

**Crate**: `miniboxd`
**File**: `crates/miniboxd/src/main.rs`

1. Above `run_daemon`, add:
   ```rust
   fn init_daemon_tracing() {
       #[cfg(feature = "otel")]
       let _otel_guard = {
           let otlp_endpoint = std::env::var("MINIBOX_OTLP_ENDPOINT").ok();
           minibox::daemon::telemetry::traces::init_tracing(otlp_endpoint.as_deref())
       };
       #[cfg(not(feature = "otel"))]
       minibox_core::init_tracing();
   }
   ```
   Note: under `#[cfg(feature = "otel")]` the guard must outlive the daemon process, so
   this helper cannot simply drop it at function return. Change the signature to return the
   guard so `run_daemon` keeps it alive:
   ```rust
   #[cfg(feature = "otel")]
   fn init_daemon_tracing() -> minibox::daemon::telemetry::traces::OtelGuard {
       let otlp_endpoint = std::env::var("MINIBOX_OTLP_ENDPOINT").ok();
       minibox::daemon::telemetry::traces::init_tracing(otlp_endpoint.as_deref())
   }
   #[cfg(not(feature = "otel"))]
   fn init_daemon_tracing() {
       minibox_core::init_tracing();
   }
   ```
2. In `run_daemon`, replace the `// ── Tracing ──` block (lines 353-360) with:
   ```rust
   #[cfg(feature = "otel")]
   let _otel_guard = init_daemon_tracing();
   #[cfg(not(feature = "otel"))]
   init_daemon_tracing();
   ```
3. Verify: `cargo check -p miniboxd` → compiles with default features, then
   `cargo check -p miniboxd --features otel` → compiles.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(miniboxd): extract init_daemon_tracing from run_daemon"`

### Task 2: Extract `select_and_validate_adapter_suite`

**Crate**: `miniboxd`
**File**: `crates/miniboxd/src/main.rs`

1. Add helper (placed after `init_daemon_tracing`):
   ```rust
   fn select_and_validate_adapter_suite() -> Result<AdapterSuite> {
       let suite = adapter_registry::adapter_from_env().map_err(|e| anyhow::anyhow!("{e}"))?;
       let available = adapter_registry::available_adapter_names();
       info!(
           selected_adapter = %suite,
           available_adapters = ?available,
           "adapter suite selected"
       );

       #[cfg(target_os = "linux")]
       adapter_registry::warn_if_native_without_root();

       #[cfg(target_os = "linux")]
       if suite == AdapterSuite::Native && !nix::unistd::getuid().is_root() {
           anyhow::bail!("miniboxd must run as root (native adapter suite)");
       }

       #[cfg(target_os = "linux")]
       if suite == AdapterSuite::Native {
           migrate_to_supervisor_cgroup();
       }

       Ok(suite)
   }
   ```
2. In `run_daemon`, replace the `// ── Adapter suite ──` through
   `// ── Cgroup self-migration ──` blocks (original lines 371-397) with:
   ```rust
   let suite = select_and_validate_adapter_suite()?;
   ```
3. Verify: `cargo check -p miniboxd`, `cargo check -p miniboxd --target x86_64-unknown-linux-musl` if cross toolchain is installed locally, otherwise skip cross-check (macOS dev machine — Linux-gated branches are not exercised by `cargo check` on macOS per `CLAUDE.md`).
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(miniboxd): extract select_and_validate_adapter_suite from run_daemon"`

### Task 3: Extract `prepare_daemon_directories`

**Crate**: `miniboxd`
**File**: `crates/miniboxd/src/main.rs`

1. Add helper:
   ```rust
   fn prepare_daemon_directories(paths: &DaemonPaths) -> Result<()> {
       const OWNER_RWX_PERMS: u32 = 0o700;
       use std::os::unix::fs::DirBuilderExt;
       for dir in &[
           paths.images_dir.as_path(),
           paths.containers_dir.as_path(),
           paths.run_dir.as_path(),
           paths.run_containers_dir.as_path(),
       ] {
           std::fs::DirBuilder::new()
               .recursive(true)
               .mode(OWNER_RWX_PERMS)
               .create(dir)
               .with_context(|| format!("creating directory {}", dir.display()))?;
       }
       Ok(())
   }
   ```
2. In `run_daemon`, replace the `// ── Directories ──` block (original lines 414-430) with:
   ```rust
   prepare_daemon_directories(&paths)?;
   ```
3. Verify: `cargo check -p miniboxd`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(miniboxd): extract prepare_daemon_directories from run_daemon"`

### Task 4: Extract `build_metrics_recorder`

**Crate**: `miniboxd`
**File**: `crates/miniboxd/src/main.rs`

1. Add helper:
   ```rust
   #[cfg(feature = "metrics")]
   async fn build_metrics_recorder() -> Result<Arc<dyn minibox_core::domain::MetricsRecorder>> {
       const DEFAULT_METRICS_ADDR: &str = "127.0.0.1:9090";
       let metrics_addr: std::net::SocketAddr = std::env::var("MINIBOX_METRICS_ADDR")
           .unwrap_or_else(|_| DEFAULT_METRICS_ADDR.to_string())
           .parse()
           .context("parsing MINIBOX_METRICS_ADDR")?;
       let recorder = Arc::new(minibox::daemon::telemetry::PrometheusMetricsRecorder::new());
       Ok(
           match minibox::daemon::telemetry::server::run_metrics_server(
               metrics_addr,
               recorder.clone(),
           )
           .await
           {
               Ok((_addr, _handle)) => {
                   info!(addr = %_addr, "metrics server listening");
                   recorder as Arc<dyn minibox_core::domain::MetricsRecorder>
               }
               Err(e) => {
                   tracing::warn!(addr = %metrics_addr, error = %e, "metrics server failed to bind; continuing without metrics");
                   Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new())
               }
           },
       )
   }

   #[cfg(not(feature = "metrics"))]
   async fn build_metrics_recorder() -> Result<Arc<dyn minibox_core::domain::MetricsRecorder>> {
       Ok(Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()))
   }
   ```
2. In `run_daemon`, replace the `// ── Metrics ──` block (original lines 449-473) with:
   ```rust
   let metrics_recorder = build_metrics_recorder().await?;
   ```
3. Verify: `cargo check -p miniboxd`, `cargo check -p miniboxd --features metrics`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(miniboxd): extract build_metrics_recorder from run_daemon"`

### Task 5: Extract `resolve_container_policy`

**Crate**: `miniboxd`
**File**: `crates/miniboxd/src/main.rs`

1. Add helper:
   ```rust
   fn resolve_container_policy(config: &miniboxd::config::DaemonConfig) -> ContainerPolicy {
       let env_policy = ContainerPolicy::from_env();
       ContainerPolicy {
           allow_bind_mounts: config
               .policy
               .allow_bind_mounts
               .unwrap_or(env_policy.allow_bind_mounts),
           allow_privileged: config
               .policy
               .allow_privileged
               .unwrap_or(env_policy.allow_privileged),
           ..Default::default()
       }
   }
   ```
2. In `run_daemon`, replace the policy-building block (original lines 478-490, the comment
   plus `env_policy`/`policy` construction) with:
   ```rust
   let policy = resolve_container_policy(&config);
   ```
   Keep the existing `tracing::info!(allow_bind_mounts = ..., allow_privileged = ..., "container policy configured (config > env > default)")` call immediately after, unchanged.
3. Verify: `cargo check -p miniboxd`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(miniboxd): extract resolve_container_policy from run_daemon"`

### Task 6: Extract `bind_and_secure_socket`

**Crate**: `miniboxd`
**File**: `crates/miniboxd/src/main.rs`

1. Add helper:
   ```rust
   fn bind_and_secure_socket(sock_path: &Path) -> Result<UnixListener> {
       if sock_path.exists() {
           warn!("removing stale socket at {}", sock_path.display());
           std::fs::remove_file(sock_path)
               .with_context(|| format!("removing stale socket {}", sock_path.display()))?;
       }

       let raw_listener = UnixListener::bind(sock_path)
           .with_context(|| format!("binding Unix socket at {}", sock_path.display()))?;

       use std::os::unix::fs::PermissionsExt;
       const DEFAULT_SOCKET_PERMS: u32 = 0o600;
       let mut mode = DEFAULT_SOCKET_PERMS;
       if let Ok(mode_str) = std::env::var("MINIBOX_SOCKET_MODE") {
           let mode_str = mode_str.trim();
           let mode_str = mode_str.strip_prefix("0o").unwrap_or(mode_str);
           match u32::from_str_radix(mode_str, 8) {
               Ok(parsed) => mode = parsed,
               Err(err) => warn!("invalid MINIBOX_SOCKET_MODE={mode_str}: {err}"),
           }
       }

       if let Ok(group_name) = std::env::var("MINIBOX_SOCKET_GROUP") {
           let group_name = group_name.trim();
           if !group_name.is_empty() {
               if let Some(group) = nix::unistd::Group::from_name(group_name)
                   .with_context(|| format!("looking up group {group_name}"))?
               {
                   nix::unistd::chown(sock_path, None, Some(group.gid))
                       .with_context(|| format!("setting socket group to {group_name}"))?;
                   info!("socket group set to {group_name}");
               } else {
                   warn!("MINIBOX_SOCKET_GROUP={group_name} not found");
               }
           }
       }

       let metadata = std::fs::metadata(sock_path)?;
       let mut permissions = metadata.permissions();
       permissions.set_mode(mode);
       std::fs::set_permissions(sock_path, permissions)
           .with_context(|| format!("setting socket permissions to {mode:04o}"))?;
       info!("socket permissions set to {mode:04o}");

       Ok(raw_listener)
   }
   ```
2. In `run_daemon`, replace the socket-bind-and-permissions block (original lines 511-557)
   with:
   ```rust
   let sock_path = &paths.socket_path;
   let raw_listener = bind_and_secure_socket(sock_path)?;
   info!("listening on {}", sock_path.display());
   ```
3. Verify: `cargo check -p miniboxd`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(miniboxd): extract bind_and_secure_socket from run_daemon"`

### Task 7: Extract `install_shutdown_signal_handlers`

**Crate**: `miniboxd`
**File**: `crates/miniboxd/src/main.rs`

1. Add helper:
   ```rust
   fn install_shutdown_signal_handlers() -> Result<impl std::future::Future<Output = ()>> {
       use tokio::signal::unix::{SignalKind, signal};
       let mut sigterm = signal(SignalKind::terminate()).context("SIGTERM handler")?;
       let mut sigint = signal(SignalKind::interrupt()).context("SIGINT handler")?;
       Ok(async move {
           tokio::select! {
               _ = sigterm.recv() => { info!("received SIGTERM, shutting down"); }
               _ = sigint.recv()  => { info!("received SIGINT, shutting down");  }
           }
       })
   }
   ```
2. In `run_daemon`, replace the `// ── Signal handling ──` block (original lines 561-570)
   with:
   ```rust
   let shutdown = install_shutdown_signal_handlers()?;
   ```
3. Verify: `cargo check -p miniboxd`.
4. Full-phase verification (run once, after this final Phase 1a task):
   ```
   cargo xtask verify
   ```
   Expected: fmt check, workspace check, and clippy (`-D warnings`) all pass with no new
   warnings introduced by the 7 extractions.
5. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(miniboxd): extract install_shutdown_signal_handlers from run_daemon"`

---

## Phase 1b — `crates/mbx/src/commands/exec.rs::execute()`

### Task 8: Extract `handle_container_output`

**Crate**: `mbx`
**File**: `crates/mbx/src/commands/exec.rs`

1. Add helper (place above `execute`):
   ```rust
   fn handle_container_output(stream: OutputStreamKind, data: &str) -> Result<()> {
       let bytes = base64::engine::general_purpose::STANDARD
           .decode(data)
           .context("failed to decode exec output chunk")?;
       match stream {
           OutputStreamKind::Stdout => {
               std::io::stdout().write_all(&bytes)?;
               std::io::stdout().flush()?;
           }
           OutputStreamKind::Stderr => {
               std::io::stderr().write_all(&bytes)?;
               std::io::stderr().flush()?;
           }
       }
       Ok(())
   }
   ```
2. In `execute`, replace the `DaemonResponse::ContainerOutput { stream, data } => { ... }`
   match arm body with:
   ```rust
   DaemonResponse::ContainerOutput { stream, data } => {
       handle_container_output(stream, &data)?;
   }
   ```
3. Verify: `cargo check -p mbx`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(mbx): extract handle_container_output from exec::execute"`

### Task 9: Extract `spawn_stdin_relay_task`

**Crate**: `mbx`
**File**: `crates/mbx/src/commands/exec.rs`

1. Add helper:
   ```rust
   #[cfg(unix)]
   fn spawn_stdin_relay_task(socket_path: std::path::PathBuf, exec_id: String) {
       tokio::spawn(async move {
           use tokio::io::AsyncReadExt as _;
           let writer = DaemonWriter::with_socket(&socket_path);
           let mut stdin = tokio::io::stdin();
           let mut buf = [0u8; 256];
           loop {
               match stdin.read(&mut buf).await {
                   Ok(0) | Err(_) => break,
                   Ok(n) => {
                       let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                       let req = DaemonRequest::SendInput {
                           session_id: minibox_core::domain::SessionId::from(exec_id.clone()),
                           data,
                       };
                       if writer.send(req).await.is_err() {
                           eprintln!("exec: stdin relay: daemon connection lost");
                           break;
                       }
                   }
               }
           }
       });
   }
   ```
2. In `execute`, inside the `DaemonResponse::ExecStarted { exec_id }` arm's `if tty { ... }`
   block, replace the `tokio::spawn(async move { ... })` stdin-relay block with:
   ```rust
   spawn_stdin_relay_task(sp.clone(), exec_id.clone());
   ```
3. Verify: `cargo check -p mbx`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(mbx): extract spawn_stdin_relay_task from exec::execute"`

### Task 10: Extract `send_initial_pty_size`

**Crate**: `mbx`
**File**: `crates/mbx/src/commands/exec.rs`

1. Add helper:
   ```rust
   #[cfg(unix)]
   async fn send_initial_pty_size(socket_path: &std::path::Path, exec_id: &str) {
       let (cols, rows) = crate::terminal::terminal_size();
       let _ = DaemonWriter::with_socket(socket_path)
           .send(DaemonRequest::ResizePty {
               session_id: minibox_core::domain::SessionId::from(exec_id.to_string()),
               cols,
               rows,
           })
           .await;
   }
   ```
2. In `execute`'s `if tty { ... }` block, replace the "Initial terminal size" block
   (the `#[cfg(unix)] { let (cols, rows) = ...; ... }`) with:
   ```rust
   #[cfg(unix)]
   send_initial_pty_size(&sp, &exec_id).await;
   ```
3. Verify: `cargo check -p mbx`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(mbx): extract send_initial_pty_size from exec::execute"`

### Task 11: Extract `spawn_sigwinch_forwarder_task`

**Crate**: `mbx`
**File**: `crates/mbx/src/commands/exec.rs`

1. Add helper:
   ```rust
   #[cfg(unix)]
   fn spawn_sigwinch_forwarder_task(socket_path: std::path::PathBuf, exec_id: String) {
       use tokio::signal::unix::{SignalKind, signal};
       match signal(SignalKind::window_change()) {
           Ok(mut sigwinch) => {
               tokio::spawn(async move {
                   let writer = DaemonWriter::with_socket(&socket_path);
                   while sigwinch.recv().await.is_some() {
                       let (cols, rows) = crate::terminal::terminal_size();
                       let _ = writer
                           .send(DaemonRequest::ResizePty {
                               session_id: minibox_core::domain::SessionId::from(exec_id.clone()),
                               cols,
                               rows,
                           })
                           .await;
                   }
               });
           }
           Err(e) => {
               eprintln!(
                   "exec: SIGWINCH handler unavailable; terminal resize will not be forwarded: {e}"
               );
           }
       }
   }
   ```
2. In `execute`'s `if tty { ... }` block, replace the "SIGWINCH forwarding" block with:
   ```rust
   #[cfg(unix)]
   spawn_sigwinch_forwarder_task(sp.clone(), exec_id.clone());
   ```
3. Verify: `cargo check -p mbx`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(mbx): extract spawn_sigwinch_forwarder_task from exec::execute"`

### Task 12: Extract `handle_exec_started`

**Crate**: `mbx`
**File**: `crates/mbx/src/commands/exec.rs`

1. Add helper (after the three helpers above exist):
   ```rust
   async fn handle_exec_started(exec_id: String, tty: bool, socket_path: &std::path::Path) {
       if tty {
           #[cfg(unix)]
           {
               spawn_stdin_relay_task(socket_path.to_path_buf(), exec_id.clone());
               send_initial_pty_size(socket_path, &exec_id).await;
               spawn_sigwinch_forwarder_task(socket_path.to_path_buf(), exec_id.clone());
           }
       }
   }
   ```
   Note: on non-`unix` targets `tty` is always forced `false` earlier in `execute()`
   (`let tty = tty && std::io::stdout().is_terminal();` — but the `is_terminal()` gate is
   platform-independent while the spawn helpers are `#[cfg(unix)]` only); confirm this
   compiles on the non-unix path by wrapping the body in `#[cfg(unix)]` as shown so the
   `if tty` check with no unix body is a no-op on non-unix, matching current behavior
   exactly (the original code has no non-unix `if tty` body either).
2. In `execute`, replace the entire `DaemonResponse::ExecStarted { exec_id } => { if tty { ... } }`
   arm with:
   ```rust
   DaemonResponse::ExecStarted { exec_id } => {
       handle_exec_started(exec_id, tty, &sp).await;
   }
   ```
3. Verify: `cargo check -p mbx`, then run the existing regression test:
   ```
   cargo nextest run -p mbx -- exec_sends_correct_request
   ```
   Expected: PASS (this test drives `execute()` end-to-end through a mock Unix socket
   server and asserts on the request payload — it exercises this exact code path).
4. Full-phase verification:
   ```
   cargo xtask verify
   cargo nextest run -p mbx
   ```
   Expected: fmt/clippy clean, all `mbx` tests green.
5. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(mbx): extract handle_exec_started from exec::execute"`

---

## Phase 2 — `crates/minibox-core/src/domain.rs` → `domain/` split

Each task below moves one cohesive cluster into its own file. Before cutting, re-run
`rg -n "^(pub )?(struct|enum|trait|impl|fn)" crates/minibox-core/src/domain.rs` to confirm
current line boundaries (they will shift after each preceding task removes lines) — do not
rely on stale line numbers from the design doc once Task 2.1 has run.

### Task 13: Scaffold `domain/` and move `error.rs`, `ids.rs`, `state.rs`

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain.rs` → new
`crates/minibox-core/src/domain/{mod.rs,error.rs,ids.rs,state.rs}`

1. Create directory `crates/minibox-core/src/domain/`.
2. Create `crates/minibox-core/src/domain/error.rs` containing the `DomainError` enum and its
   `impl DomainError` block, cut verbatim from `domain.rs`.
3. Create `crates/minibox-core/src/domain/ids.rs` containing `ContainerId` (struct + all
   impls: `impl ContainerId`, `impl Display`, `impl AsRef<str>`) and `SessionId` (struct +
   all impls: `impl SessionId`, `impl Display`, `impl AsRef<str>`, `impl Deref`,
   `impl From<String>`, `impl From<&str>`), cut verbatim.
4. Create `crates/minibox-core/src/domain/state.rs` containing `ContainerState` enum and its
   `impl ContainerState` + `impl Display` blocks, cut verbatim.
5. Create `crates/minibox-core/src/domain/mod.rs`:
   ```rust
   mod error;
   mod ids;
   mod state;

   pub use error::*;
   pub use ids::*;
   pub use state::*;
   ```
   (Remaining unmigrated types stay in `domain.rs` temporarily — see step 6.)
6. Rename `crates/minibox-core/src/domain.rs` to
   `crates/minibox-core/src/domain/legacy.rs`, remove the moved items (`DomainError`,
   `ContainerId`, `SessionId`, `ContainerState` and their impls) from it, and add
   `mod legacy; pub use legacy::*;` to `domain/mod.rs`. This keeps every not-yet-migrated
   type compiling under the new directory layout for the remaining tasks in this phase.
7. Verify: `cargo check --workspace` — every downstream `minibox_core::domain::{DomainError,
   ContainerId, SessionId, ContainerState}` import must still resolve via the re-exports.
8. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(minibox-core): scaffold domain/ module, move error/ids/state"`

### Task 14: Move `filesystem.rs`

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain/legacy.rs` →
`crates/minibox-core/src/domain/filesystem.rs`

1. Create `crates/minibox-core/src/domain/filesystem.rs` containing (cut verbatim from
   `legacy.rs`): `BindMount` struct + `impl BindMount`, `RootfsSetup` trait, `ChildInit`
   trait, `FilesystemProvider` trait + its blanket `impl<T: ...> FilesystemProvider for T`,
   `BackendRootfsMetadata` enum + `impl BackendRootfsMetadata`, `RootfsLayout` struct.
2. Add `mod filesystem; pub use filesystem::*;` to `domain/mod.rs`.
3. Verify: `cargo check --workspace`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(minibox-core): move filesystem port types out of domain legacy"`

### Task 15: Move `runtime.rs`

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain/legacy.rs` →
`crates/minibox-core/src/domain/runtime.rs`

1. Create `crates/minibox-core/src/domain/runtime.rs` containing: `AsAny` trait,
   `ContainerRuntime` trait, `ResourceLimiter` trait, `ResourceConfig` struct,
   `RuntimeCapabilities` struct, `SpawnResult` struct, `HookSpec` struct,
   `ContainerHooks` struct, `ContainerSpawnConfig` struct — cut verbatim from `legacy.rs`.
2. Add `mod runtime; pub use runtime::*;` to `domain/mod.rs`.
3. Verify: `cargo check --workspace` — this is the highest-risk move in Phase 2 because
   `AsAny` is referenced by the `as_any!`/`adapt!` macros called from `crates/minibox/src/adapters/**`
   (per `CLAUDE.md`: "do not remove re-exports needed by `as_any!`/`adapt!` macro
   expansion"). Confirm zero errors before proceeding.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(minibox-core): move runtime port types out of domain legacy"`

### Task 16: Move `image.rs`

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain/legacy.rs` →
`crates/minibox-core/src/domain/image.rs`

1. Create `crates/minibox-core/src/domain/image.rs` containing: `ImageRegistry` trait,
   `RegistryRouter` trait, `ImageLoader` trait, `ImageMetadata` struct, `LayerInfo` struct,
   `ImagePusher` trait, `RegistryCredentials` enum, `PushResult` struct, `PushProgress`
   struct, `ContainerCommitter` trait, `CommitConfig` struct, `ImageBuilder` trait,
   `BuildContext` struct, `BuildConfig` struct, `BuildProgress` struct — cut verbatim.
2. Add `mod image; pub use image::*;` to `domain/mod.rs`.
3. Verify: `cargo check --workspace`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(minibox-core): move image port types out of domain legacy"`

### Task 17: Move `exec.rs`

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain/legacy.rs` →
`crates/minibox-core/src/domain/exec.rs`

1. Create `crates/minibox-core/src/domain/exec.rs` containing: `ExecSpec` struct,
   `ExecHandle` struct, `ProgressSink<T>` trait + its two impls (`for
   tokio::sync::mpsc::Sender<T>` and `for Arc<dyn ProgressSink<T>>`), `ExecRuntime` trait —
   cut verbatim.
2. Add `mod exec; pub use exec::*;` to `domain/mod.rs`.
3. Verify: `cargo check --workspace`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(minibox-core): move exec port types out of domain legacy"`

### Task 18: Move `pty.rs`

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain/legacy.rs` →
`crates/minibox-core/src/domain/pty.rs`

1. Create `crates/minibox-core/src/domain/pty.rs` containing: `PtyConfig` struct,
   `PtyHandle` struct, `PtyAllocator` trait, `NullPtyAllocator` struct + `impl PtyAllocator`,
   `MockPtyAllocator` struct + `impl MockPtyAllocator` + `impl PtyAllocator` — cut verbatim.
2. Add `mod pty; pub use pty::*;` to `domain/mod.rs`.
3. Verify: `cargo check --workspace`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(minibox-core): move pty port types out of domain legacy"`

### Task 19: Move `checkpoint.rs`

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain/legacy.rs` →
`crates/minibox-core/src/domain/checkpoint.rs`

1. Create `crates/minibox-core/src/domain/checkpoint.rs` containing: `SnapshotInfo` struct,
   `VmCheckpoint` trait, `NoopVmCheckpoint` struct + `impl VmCheckpoint` — cut verbatim.
2. Add `mod checkpoint; pub use checkpoint::*;` to `domain/mod.rs`.
3. Verify: `cargo check --workspace`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(minibox-core): move checkpoint port types out of domain legacy"`

### Task 20: Move `capability.rs`

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain/legacy.rs` →
`crates/minibox-core/src/domain/capability.rs`

1. Create `crates/minibox-core/src/domain/capability.rs` containing: `BackendCapability`
   enum, `BackendCapabilitySet` struct + `impl BackendCapabilitySet` (this impl block is
   large — confirm its full extent with `rg -n "^impl BackendCapabilitySet"` and
   `rg -n "^pub (enum|struct|trait|fn)"` for the next item after it before cutting, since it
   spans roughly 650 lines in the original file) — cut verbatim.
2. Add `mod capability; pub use capability::*;` to `domain/mod.rs`.
3. Verify: `cargo check --workspace`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(minibox-core): move capability types out of domain legacy"`

### Task 21: Move `metrics.rs`

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain/legacy.rs` →
`crates/minibox-core/src/domain/metrics.rs`

1. Create `crates/minibox-core/src/domain/metrics.rs` containing: `MetricsRecorder` trait —
   cut verbatim.
2. Add `mod metrics; pub use metrics::*;` to `domain/mod.rs`.
3. Verify: `cargo check --workspace`.
4. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(minibox-core): move MetricsRecorder out of domain legacy"`

### Task 22: Move `workflow.rs` and delete `legacy.rs`

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain/legacy.rs` →
`crates/minibox-core/src/domain/workflow.rs`; delete
`crates/minibox-core/src/domain/legacy.rs`

1. Create `crates/minibox-core/src/domain/workflow.rs` containing every remaining item from
   `legacy.rs` — at this point that is exactly the workflow-engine cluster: `StepRetry`,
   `ExprVar`, `WorkflowStep`, `WorkflowDef`, `PhaseOutcome`, `StepStatus` + `impl From<StepStatus>
   for StepState`, `determine_final_phase`, `StepCapability`, `StepContext`, `StepOutput`,
   `StepRunnerCapability`, `StepRunner` trait, `StepRunnerRegistry` + `impl
   StepRunnerRegistry` + `impl Default`, `ContainerRunStepRunner` + `impl StepRunner`,
   `ImagePullStepRunner` + `impl StepRunner`, `ExecStepRunner` + `impl StepRunner`,
   `OverlaySnapshotStepRunner` + `impl StepRunner`, `StepCompletion`,
   `determine_step_completion`, `ResolvedStep`, `resolve_step_vars`, `propagate_output`,
   `steps_before`, `resume_workflow`, `evaluate_if_guard`, `resolve_expr`,
   `resolve_output_ref`, `meets_min_priority`. Cut verbatim — this should leave `legacy.rs`
   empty of items (imports only).
2. Delete `crates/minibox-core/src/domain/legacy.rs`.
3. In `domain/mod.rs`, replace `mod legacy; pub use legacy::*;` with
   `mod workflow; pub use workflow::*;`.
4. Verify each module's internal `use` statements: since these types previously all lived in
   one file, cross-module references (e.g. `workflow.rs`'s `ContainerRunStepRunner` referring
   to runtime/exec/image/filesystem types) now need explicit `use super::{runtime::*,
   exec::*, image::*, filesystem::*};` or `use crate::domain::{...};` imports — add them as
   needed per compiler errors.
5. Verify: `cargo check --workspace` — must be completely clean, no unresolved imports
   anywhere in the workspace.
6. Full-phase verification:
   ```
   cargo xtask verify
   cargo test --workspace
   ```
   Expected: fmt/clippy clean; all existing `domain.rs`-colocated unit tests (which moved
   with their respective type modules) still pass, plus every downstream crate's tests that
   depend on `minibox_core::domain::*` types.
7. Run: `git branch --show-current` (must not be `main`).
   Commit: `git commit -m "refactor(minibox-core): move workflow engine types out of domain legacy, delete legacy.rs"`

---

## Final verification (after all three phases)

```
cargo xtask verify
cargo test --workspace
cargo xtask borrow-fixtures
```

Expected: all green, zero new clippy warnings, zero behavior change. `domain.rs` no longer
exists as a single file; `crates/minibox-core/src/domain/` contains 12 focused modules per
the approved design doc.
