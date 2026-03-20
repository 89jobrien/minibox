# Cross-Platform Daemon Design

**Date:** 2026-03-19 (revised from 2026-03-17)
**Status:** Draft
**Scope:** Unified `miniboxd` binary with platform-adaptive backends — Linux (native namespaces), macOS (Virtualization.framework + Colima fallback), Windows (HCS + WSL2 fallback). Shared `daemonbox` application layer. Platform orchestration isolated in `macbox` and `winbox` library crates.

---

## Problem

`miniboxd` has a `compile_error!("miniboxd requires Linux")` guard in `main.rs`. The Colima adapters in `minibox-lib` are fully implemented and tested but unreachable from macOS because there is no macOS-native daemon binary. Running the daemon on macOS requires either SSHing into a Colima VM or cross-compiling — both fragile and not the right UX.

Windows has no support at all.

Separately, the original design proposed a separate `macboxd` binary. After review, a unified binary with platform-dispatched `main` bodies is cleaner: one binary to build, one binary to distribute, one binary to document.

---

## Goals

1. Single `miniboxd` binary compiles and runs natively on Linux, macOS, and Windows.
2. Each platform has a native container backend:
   - Linux → namespaces + overlay FS + cgroups v2 (existing)
   - macOS → Apple Virtualization.framework with shared Linux VM
   - Windows → Host Compute Service (HCS) with Windows container images
3. macOS keeps Colima as a supported fallback adapter.
4. Windows keeps WSL2 as a supported fallback adapter (Linux OCI images).
5. Platform-specific orchestration (preflight, path conventions, adapter wiring) is isolated in `macbox` and `winbox` library crates.
6. `daemonbox` is platform-agnostic for all domain logic. Controlled `#[cfg(unix)]` / `#[cfg(windows)]` seams are permitted **only** for OS-level process signaling (`SIGTERM`/`SIGKILL`, `waitpid`) — these cannot be abstracted behind domain traits without leaking OS concepts into the domain.
7. Hexagonal architecture is preserved throughout: adapters wired only in composition roots.

---

## Non-Goals

- No TTY, exec, networking, or log capture.
- No launchd unit or Windows Service registration (this iteration).
- No rootless Linux support.
- No per-container VMs on macOS (shared VM model for Virtualization.framework).
- No Windows ARM support in this iteration.
- No changes to the GKE adapter suite.
- No new container features.
- **Phase 2 (not this iteration):** Virtualization.framework in-VM agent (vsock-based `VfRuntime`). The `vf.rs` adapter file and its types are scaffolded but the in-VM agent binary is deferred. macOS native path for this iteration is functional via Colima fallback while VF scaffolding is built.
- **Unwired adapters:** `docker_desktop.rs` remains a library-only stub — not wired in this iteration.

---

## Architecture

See [`docs/diagrams/`](../../diagrams/) for visual references.

### Platform Dispatch

`miniboxd/main.rs` contains three `#[cfg]`-gated `main` functions. On Linux the existing composition root runs inline. On macOS and Windows, `main` delegates entirely to `macbox::start()` or `winbox::start()` respectively. Those functions own preflight, path resolution, adapter wiring, socket binding, signal handling, and the server loop — the same responsibilities that Linux's `main` has today.

```
miniboxd/main.rs
  #[cfg(target_os = "linux")]   → inline Linux wiring (unchanged logic)
  #[cfg(target_os = "macos")]   → macbox::start().await
  #[cfg(target_os = "windows")] → winbox::start().await
```

### Layer Mapping

```
┌───────────────────────────────────────────────────────────────────────┐
│                   Composition Roots (driving side)                    │
│   miniboxd/main.rs (Linux)                                            │
│   macbox::start()  (macOS)    winbox::start()  (Windows)              │
├───────────────────────────────────────────────────────────────────────┤
│                   Application Core (daemonbox)                        │
│              handler.rs   state.rs   server.rs                        │
│            (depends only on domain port traits)                       │
├───────────────────────────────────────────────────────────────────────┤
│                   Domain Ports (minibox-lib/domain.rs)                │
│   ContainerRuntime  FilesystemProvider  ImageRegistry                 │
│   ResourceLimiter                                                     │
├───────────────────────────────────────────────────────────────────────┤
│                   Driven Adapters (minibox-lib/adapters/)             │
│  Linux:   LinuxNamespaceRuntime  OverlayFilesystem                    │
│           CgroupV2Limiter  DockerHubRegistry                          │
│  macOS:   VfRuntime  VfFilesystem  VfRegistry                         │
│           ColimaRuntime  ColimaFilesystem  ColimaRegistry             │
│  Windows: HcsRuntime  HcsFilesystem  HcsRegistry  JobObjectLimiter    │
│           Wsl2Runtime  Wsl2Filesystem  Wsl2Registry                   │
└───────────────────────────────────────────────────────────────────────┘
```

`daemonbox` has zero knowledge of concrete adapters or transports. Adapter wiring and transport selection happen exclusively in the composition root of each platform.

### Transport Abstraction

`daemonbox/server.rs` currently takes a `tokio::net::UnixListener` directly. Windows uses Named Pipes (`tokio::net::windows::named_pipe`), which has an incompatible API. To keep `daemonbox` platform-agnostic, `server.rs` is refactored to accept a generic `ServerListener` trait:

```rust
// daemonbox/src/server.rs

/// Platform-agnostic listener abstraction. Implemented by UnixListener (Linux/macOS)
/// and NamedPipeListener (Windows). Concrete implementations live in platform crates.
pub trait ServerListener: Send + 'static {
    type Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static;
    // async fn in traits is stable since Rust 1.75 (RPITIT). The workspace
    // requires 1.85, so no #[async_trait] needed.
    fn accept(&self) -> impl std::future::Future<Output = anyhow::Result<Self::Stream>> + Send + '_;
}

pub async fn run_server<L: ServerListener>(
    listener: L,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    require_root_auth: bool,
) -> anyhow::Result<()>;
```

Uses `impl Trait` in return position (static dispatch) — `run_server` is monomorphised per listener type. `Box<dyn ServerListener>` is not used; no `#[async_trait]` required.

`run_server` drives the accept loop and calls the existing `handle_connection` helper. `handle_connection` is refactored to be generic over the stream type:

```rust
async fn handle_connection<S>(
    stream: S,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    require_root_auth: bool,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
```

The `SO_PEERCRED` auth block inside `handle_connection` remains gated `#[cfg(target_os = "linux")]` as it is today (already in the existing code). The stream type is driven by `L::Stream` from the `ServerListener` impl — `UnixStream` on Linux/macOS, `NamedPipeServer`'s stream type on Windows.

`daemonbox` does **not** provide concrete `ServerListener` implementations — those live in each platform's composition root. `miniboxd/main.rs` (Linux) wraps `UnixListener`. `macbox::start()` wraps `UnixListener`. `winbox::start()` wraps `NamedPipeServer`.

### Process Signaling Seam in `daemonbox`

`handler.rs` uses `nix::sys::signal::kill` and `nix::sys::wait::waitpid` for `handle_stop` and `daemon_wait_for_exit`. `nix` is a POSIX library — it compiles on Linux **and macOS**. The seam is only at the Unix/Windows boundary.

`daemonbox/Cargo.toml` moves `nix` to `[target.'cfg(unix)'.dependencies]`. `handler.rs` gates the `nix` imports and signal/wait code with `#[cfg(unix)]`.

**Windows stubs — exact shape:**

```rust
// handler.rs

#[cfg(windows)]
async fn stop_inner(id: &str, state: &Arc<DaemonState>) -> Result<()> {
    // Windows process signaling is handled by the HCS/WSL2 runtime adapter.
    // Direct SIGTERM is not available. Return an error until the Windows
    // runtime adapter implements stop via HCS/wsl.exe.
    let _ = state.get_container(id).await;
    anyhow::bail!("handle_stop not yet implemented on Windows — use the HCS/WSL2 adapter stop path")
}

#[cfg(windows)]
fn daemon_wait_for_exit(_pid: u32, _id: &str, _state: Arc<DaemonState>) {
    // No-op on Windows. Container stays "Running" in DaemonState until
    // handle_stop or handle_remove is called explicitly.
    // Windows runtime adapters track exit via HCS events or wsl.exe polling
    // in the adapter layer, not here.
}
```

`daemon_wait_for_exit` is only spawned from the reaper task in `run_inner`. On Windows the `spawn_blocking` call that wraps it is still made — it just returns immediately. Container state will remain "Running" until explicit stop/remove. This is the **only** `#[cfg]` permitted in `daemonbox` handler logic — all other platform differences belong in adapters.

---

## Platform Adapter Matrix

| `MINIBOX_ADAPTER`       | Platform      | Runtime backend                   | Image type  | Root required      |
| ----------------------- | ------------- | --------------------------------- | ----------- | ------------------ |
| `native` (default)      | Linux         | namespaces + overlay + cgroups v2 | Linux OCI   | yes                |
| `gke`                   | Linux         | proot + copy FS                   | Linux OCI   | no                 |
| `colima`                | Linux / macOS | Colima/limactl delegate           | Linux OCI   | no (VM handles it) |
| `vf` (default macOS)    | macOS         | Apple Virtualization.framework    | Linux OCI   | no                 |
| `hcs` (default Windows) | Windows       | Host Compute Service              | Windows OCI | Administrator      |
| `wsl2`                  | Windows       | WSL2 distro delegate              | Linux OCI   | no                 |

**Auto-selection defaults** (when `MINIBOX_ADAPTER` is unset):

- Linux → `native`
- macOS → `vf`; if Virtualization.framework unavailable → `colima`; if neither → fatal error
- Windows → `hcs`; if HCS unavailable → `wsl2`; if neither → fatal error

**Explicit `MINIBOX_ADAPTER` on macOS/Windows:** `macbox::start()` and `winbox::start()` read `MINIBOX_ADAPTER` before calling `preflight()`. An explicit value bypasses the auto-fallback chain and fails fast if that backend is unavailable.

---

## New Crates

### `crates/macbox` — macOS Orchestration Library

**Purpose:** macOS-specific infrastructure for adapter selection, preflight checks, path conventions, and VM lifecycle management.

**Platform gate:** `#[cfg(target_os = "macos")]` — compile error on non-macOS.

**Public API:**

```rust
/// Entry point — called by miniboxd/main.rs on macOS.
pub async fn start() -> anyhow::Result<()>;

/// Determine which backend is available and should be used.
pub fn preflight() -> Result<MacboxStatus, MacboxError>;

/// Build HandlerDependencies wired with Virtualization.framework adapters.
/// Boots the shared Linux VM if not already running.
pub async fn vf_deps(
    containers_base: PathBuf,
    run_containers_base: PathBuf,
) -> Result<HandlerDependencies, MacboxError>;

/// Build HandlerDependencies wired with Colima adapters (fallback).
pub fn colima_deps(
    containers_base: PathBuf,
    run_containers_base: PathBuf,
) -> HandlerDependencies;

pub mod paths {
    pub fn data_dir() -> PathBuf;    // ~/Library/Application Support/minibox
    pub fn run_dir() -> PathBuf;     // /tmp/minibox
    pub fn socket_path() -> PathBuf; // /tmp/minibox/miniboxd.sock
}
```

**`MacboxStatus`:**

```rust
pub enum MacboxStatus {
    VirtualizationFramework,  // VF available (macOS 11+, Apple Silicon preferred)
    Colima,                   // VF unavailable, Colima running
    ColimaNotRunning,         // VF unavailable, Colima installed but stopped
    NoBackendAvailable,       // neither VF nor Colima present
}
```

**Virtualization.framework model:**

- A single shared lightweight Linux VM boots on first container operation (lazy start).
- VM uses a bundled minimal kernel + initrd (no user-space distro required).
- Container processes run inside the VM via virtio-vsock channel.
- `VfRuntime` adapter in `minibox-lib` communicates with a small in-VM agent over vsock.
- VM is reused across containers; shut down on daemon exit.
- Apple Silicon: uses hardware virtualization (fast). Intel Macs: uses Hypervisor.framework (slower).

**Phase 2 note:** The in-VM agent (vsock-based minibox helper that runs Linux namespaces inside the VM) is deferred. `vf.rs` in `minibox-lib` is scaffolded with stub types that return `Err("VF adapter not yet implemented")`.

**Phase 1 `preflight()` behaviour on macOS:** `preflight()` in Phase 1 does **not** attempt Virtualization.framework detection — it always returns `MacboxStatus::Colima` (or `ColimaNotRunning` / `NoBackendAvailable` based on Colima state). VF detection is added in Phase 2. This avoids the problem of stubs appearing "available" and failing at runtime rather than at preflight.

**`ColimaNotRunning` handling:** when `preflight()` returns `ColimaNotRunning`, `start()` attempts `colima start` (via `limactl`) automatically. If that succeeds, startup continues. If it fails, `start()` returns a `MacboxError::VmStartFailed` with the limactl stderr.

**`macbox/src/vf.rs` vs `minibox-lib/src/adapters/vf.rs`:**

- `minibox-lib/src/adapters/vf.rs` — `VfRuntime`, `VfFilesystem`, `VfRegistry` trait implementations. Phase 1: stubs. Phase 2: real implementations that communicate over vsock with the in-VM agent.
- `macbox/src/vf.rs` — VM lifecycle: boot, health check, shutdown, vsock channel setup. Phase 2 only. **Not created in Phase 1** — omit from Phase 1 file map.

**`MacboxError`:**

```rust
#[derive(thiserror::Error, Debug)]
pub enum MacboxError {
    #[error("no container backend available — install Colima (`brew install colima`) or upgrade to macOS 11+")]
    NoBackendAvailable,
    #[error("Colima is not installed — run `brew install colima`")]
    ColimaNotInstalled,
    #[error("Colima VM failed to start: {0}")]
    VmStartFailed(String),
    #[error("Virtualization.framework error: {0}")]
    VfError(String),
    #[error("preflight check failed: {0}")]
    PreflightFailed(String),
}
```

**`HandlerDependencies` coupling note:** `macbox` returns `daemonbox::handler::HandlerDependencies` directly. This is intentional — `macbox` is a composition root helper, not a domain layer. `HandlerDependencies` is `pub` with all public fields and must remain so. Any structural change to `HandlerDependencies` is a breaking change to `macbox` and `winbox`.

**Dependencies:** `daemonbox`, `minibox-lib`, `anyhow`, `thiserror`, `tokio`, `tracing`, `objc2`, `objc2-virtualization`

---

### `crates/winbox` — Windows Orchestration Library

**Purpose:** Windows-specific infrastructure for adapter selection, preflight checks, path conventions, and HCS/WSL2 lifecycle management.

**Platform gate:** `#[cfg(target_os = "windows")]` — compile error on non-Windows.

**Public API:**

```rust
/// Entry point — called by miniboxd/main.rs on Windows.
pub async fn start() -> anyhow::Result<()>;

/// Determine which backend is available and should be used.
pub fn preflight() -> Result<WinboxStatus, WinboxError>;

/// Build HandlerDependencies wired with HCS adapters (Windows containers).
pub fn hcs_deps(
    containers_base: PathBuf,
    run_containers_base: PathBuf,
) -> Result<HandlerDependencies, WinboxError>;

/// Build HandlerDependencies wired with WSL2 adapters (Linux OCI images).
pub fn wsl2_deps(
    containers_base: PathBuf,
    run_containers_base: PathBuf,
) -> Result<HandlerDependencies, WinboxError>;

pub mod paths {
    pub fn data_dir() -> PathBuf;    // %APPDATA%\minibox
    pub fn run_dir() -> PathBuf;     // %LOCALAPPDATA%\Temp\minibox
    pub fn pipe_path() -> String;    // \\.\pipe\miniboxd
}
```

**`WinboxStatus`:**

```rust
pub enum WinboxStatus {
    Hcs,            // Windows Containers feature enabled, HCS available
    Wsl2,           // WSL2 available, HCS not
    HcsAndWsl2,     // both available; HCS is preferred
    NoBackendAvailable,
}
```

**HCS (Windows Containers):**

- Uses `windows` crate (`windows::Win32::System::HostComputeSystem`) for container lifecycle.
- `HcsRegistry` pulls Windows container images from MCR / Docker Hub (Windows manifest variant).
- `JobObjectLimiter` uses Windows Job Objects for CPU + memory limits.
- `HcsFilesystem` manages Windows container layer storage (VHD-based).
- Requires Administrator privileges (analogous to Linux root).

**WSL2:**

- Delegates to a minibox agent process running inside a WSL2 distro.
- Linux OCI images — same image ecosystem as Linux native.
- Communication over a named pipe bridged into the WSL2 distro.

**Socket / IPC:**

- Windows uses Named Pipes (`\\.\pipe\miniboxd`) instead of Unix domain sockets.
- `tokio::net::windows::named_pipe` replaces `tokio::net::UnixListener`.
- `minibox-cli` detects platform and uses the appropriate transport.

**`WinboxError`:**

```rust
#[derive(thiserror::Error, Debug)]
pub enum WinboxError {
    #[error("no container backend available — enable Windows Containers feature or install WSL2")]
    NoBackendAvailable,
    #[error("HCS operation failed: {0}")]
    HcsError(String),
    #[error("WSL2 is not installed or not running")]
    Wsl2NotAvailable,
    #[error("Administrator privileges required for HCS containers")]
    NotAdministrator,
    #[error("preflight check failed: {0}")]
    PreflightFailed(String),
}
```

**Dependencies:** `daemonbox`, `minibox-lib`, `anyhow`, `thiserror`, `tokio`, `tracing`, `windows` (HCS, Job Objects, Named Pipes)

---

## Modified Crates

### `miniboxd`

- Remove `compile_error!("miniboxd requires Linux")`.
- Replace with three `#[cfg(target_os)]`-gated `main` functions.
- Move `nix` and Linux-specific imports under `#[cfg(target_os = "linux")]`.
- `lib.rs` re-export shim is unchanged (backward compat for integration tests).

**`Cargo.toml` after (platform-gated deps section — all existing unconditional deps retained):**

```toml
# Existing unconditional deps (daemonbox, anyhow, tokio, tracing-subscriber, etc.)
# are retained as-is. Only these sections change:

[target.'cfg(target_os = "linux")'.dependencies]
nix = { workspace = true }
minibox-lib = { workspace = true }
# (move linux-only deps here from [dependencies])

[target.'cfg(target_os = "macos")'.dependencies]
macbox = { path = "../macbox" }

[target.'cfg(target_os = "windows")'.dependencies]
winbox = { path = "../winbox" }
```

`daemonbox` is an unconditional dependency (all platforms need the application core). `HandlerDependencies` is `pub` in `daemonbox::handler` — platform crates import it as `daemonbox::handler::HandlerDependencies`.

### `minibox-cli`

Socket path resolution becomes platform-aware:

- Linux → Unix domain socket (`/run/minibox/miniboxd.sock`)
- macOS → Unix domain socket (`/tmp/minibox/miniboxd.sock`)
- Windows → Named Pipe (`\\.\pipe\miniboxd`)

All paths/names are overridable via `MINIBOX_SOCKET_PATH` (Linux/macOS) or `MINIBOX_PIPE_NAME` (Windows).

### `minibox-lib` — New Adapters

New and renamed adapter files in `crates/minibox-lib/src/adapters/`:

| File                | Action               | Platform                        | Implements                                                                      |
| ------------------- | -------------------- | ------------------------------- | ------------------------------------------------------------------------------- |
| `vf.rs`             | Create               | `#[cfg(target_os = "macos")]`   | `VfRuntime`, `VfFilesystem`, `VfRegistry` (stubs, Phase 2)                      |
| `hcs.rs`            | Create               | `#[cfg(target_os = "windows")]` | `HcsRuntime`, `HcsFilesystem`, `HcsRegistry`, `JobObjectLimiter`                |
| `wsl2.rs`           | Rename from `wsl.rs` | `#[cfg(target_os = "windows")]` | `Wsl2Runtime`, `Wsl2Filesystem`, `Wsl2Limiter` (rename from `WslRuntime`, etc.) |
| `docker_desktop.rs` | Keep as-is           | `#[cfg(target_os = "macos")]`   | Unwired stub — not wired in this iteration                                      |

**`wsl.rs` → `wsl2.rs` migration:** The existing `WslRuntime`, `WslFilesystem`, `WslLimiter` types are renamed to `Wsl2Runtime`, `Wsl2Filesystem`, `Wsl2Limiter` to align with the adapter naming in the matrix. The implementation (subprocess delegation to `wsl.exe`) is preserved unchanged.

Existing `colima.rs` is unchanged. `DockerHubRegistry` is unchanged (shared by Linux + macOS VF + Windows WSL2).

---

## Dependency Graph

```
miniboxd ──[all]────► daemonbox ──► minibox-lib
         ──[linux]──► minibox-lib
         ──[linux]──► nix (direct, for migrate_to_supervisor_cgroup)
         ──[macos]──► macbox    ──► daemonbox ──► minibox-lib
                             └──────────────────► minibox-lib
         ──[win]───► winbox    ──► daemonbox ──► minibox-lib
                             └──────────────────► minibox-lib

minibox-cli ──► minibox-lib   (unchanged)
```

Full graph: see [`docs/diagrams/crate-dependency-graph.md`](../../diagrams/crate-dependency-graph.md)

---

## File Map

| Action | Path                                                                                                                                                                                                                                              |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Modify | `crates/miniboxd/src/main.rs` — remove `compile_error!`, add platform-gated mains                                                                                                                                                                 |
| Modify | `crates/miniboxd/Cargo.toml` — platform-gated deps                                                                                                                                                                                                |
| Modify | `crates/minibox-cli/src/main.rs` — platform-aware socket path                                                                                                                                                                                     |
| Create | `crates/macbox/Cargo.toml`                                                                                                                                                                                                                        |
| Create | `crates/macbox/src/lib.rs`                                                                                                                                                                                                                        |
| Create | `crates/macbox/src/preflight.rs`                                                                                                                                                                                                                  |
| Create | `crates/macbox/src/paths.rs`                                                                                                                                                                                                                      |
| Create | `crates/macbox/src/vf.rs` — VF VM lifecycle, vsock channel **(Phase 2 — do not create in Phase 1)**                                                                                                                                               |
| Create | `crates/winbox/Cargo.toml`                                                                                                                                                                                                                        |
| Create | `crates/winbox/src/lib.rs`                                                                                                                                                                                                                        |
| Create | `crates/winbox/src/preflight.rs`                                                                                                                                                                                                                  |
| Create | `crates/winbox/src/paths.rs`                                                                                                                                                                                                                      |
| Create | `crates/winbox/src/hcs.rs` — HCS wiring                                                                                                                                                                                                           |
| Create | `crates/winbox/src/wsl2.rs` — WSL2 delegate wiring                                                                                                                                                                                                |
| Create | `crates/minibox-lib/src/adapters/vf.rs`                                                                                                                                                                                                           |
| Create | `crates/minibox-lib/src/adapters/hcs.rs`                                                                                                                                                                                                          |
| Create | `crates/minibox-lib/src/adapters/wsl2.rs`                                                                                                                                                                                                         |
| Rename | `crates/minibox-lib/src/adapters/wsl.rs` → `wsl2.rs` — rename `WslRuntime/Filesystem/Limiter` → `Wsl2*`                                                                                                                                           |
| Modify | `crates/minibox-lib/src/adapters/mod.rs` — pub use new adapters under cfg; update wsl re-exports (breaking rename, workspace-internal only — no deprecation aliases needed)                                                                       |
| Modify | `crates/daemonbox/src/server.rs` — refactor to `ServerListener` trait + `run_server<L: ServerListener>`                                                                                                                                           |
| Modify | `crates/daemonbox/src/handler.rs` — gate `nix` imports + signal/wait code with `#[cfg(unix)]`; add `#[cfg(windows)]` stubs                                                                                                                        |
| Modify | `crates/daemonbox/Cargo.toml` — move `nix` to `[target.'cfg(unix)'.dependencies]`                                                                                                                                                                 |
| Modify | `crates/daemonbox/src/lib.rs` — update stale `macboxd` reference in module doc                                                                                                                                                                    |
| Modify | `crates/minibox-cli/src/transport.rs` (new file) or `main.rs` — platform-aware connect: `UnixStream` on Linux/macOS, `NamedPipeClient` on Windows                                                                                                 |
| Modify | `Cargo.toml` (workspace root) — add `macbox`, `winbox` to `[workspace.members]`; add `macbox = { path = "crates/macbox" }`, `winbox = { path = "crates/winbox" }`, `objc2`, `objc2-virtualization`, `windows` crate to `[workspace.dependencies]` |

**Note on `cargo check --workspace`:** Linux CI excludes platform crates: `cargo check --workspace --exclude macbox --exclude winbox`. Cross-platform compile checks are dedicated CI jobs: `cargo check -p miniboxd --target aarch64-apple-darwin` (macOS cross from Linux, requires `cargo-zigbuild` or a real macOS runner) and `cargo check -p miniboxd --target x86_64-pc-windows-msvc` (requires Windows runner or `cargo-xwin`).

---

## Error Handling

Each platform crate defines its own error type. `start()` converts to `anyhow::Error` before returning to `miniboxd/main.rs`, which exits with a clear message and non-zero status on any preflight or startup failure.

---

## Socket / IPC

| Platform | Transport          | Default path                 |
| -------- | ------------------ | ---------------------------- |
| Linux    | Unix domain socket | `/run/minibox/miniboxd.sock` |
| macOS    | Unix domain socket | `/tmp/minibox/miniboxd.sock` |
| Windows  | Named Pipe         | `\\.\pipe\miniboxd`          |

All paths are overridable via `MINIBOX_SOCKET_PATH` (Linux/macOS) or `MINIBOX_PIPE_NAME` (Windows). `daemonbox::server::run_server` accepts `impl ServerListener` — the concrete transport is created by the composition root and passed in.

---

## Testing

- **`macbox` unit tests:** mock `MacboxStatus` variants; test path resolution; test `preflight()` with injected executor (same pattern as `ColimaRegistry`'s `LimaExecutor`). Run on macOS CI only.
- **`winbox` unit tests:** mock `WinboxStatus`; test path resolution; test `preflight()` with injected executor. Run on Windows CI only.
- **VF adapter integration:** manual smoke test on macOS with Apple Silicon — `miniboxd &` → `minibox pull alpine` → `minibox run alpine -- echo hi`.
- **HCS adapter integration:** manual smoke test on Windows Server / Windows 11 with Windows Containers feature enabled.
- **WSL2 adapter integration:** manual smoke test on Windows 10/11 with WSL2 installed.
- **Linux regressions:** all existing lib, handler, and integration tests must pass unchanged (`cargo test --workspace` on Linux).
- **Cross-compilation checks:** `cargo check -p miniboxd --target aarch64-apple-darwin` and `cargo check -p miniboxd --target x86_64-pc-windows-msvc` run in CI to catch compile errors early without requiring real hardware.

---

## Success Criteria

1. `cargo build -p miniboxd` succeeds on Linux, macOS, and Windows.
2. `cargo build --workspace` succeeds on Linux with no regressions.
3. All existing lib and handler tests pass on Linux.
4. On macOS with Apple Silicon: `miniboxd` starts, completes preflight (VF path), and accepts connections.
5. On macOS with Colima running: `MINIBOX_ADAPTER=colima miniboxd` works as before.
6. On Windows with Windows Containers enabled: `miniboxd` starts on HCS path and accepts connections.
7. On Windows with WSL2: `MINIBOX_ADAPTER=wsl2 miniboxd` works.
8. `minibox pull alpine && minibox run alpine -- /bin/echo hello` succeeds on Linux.
9. On macOS with Colima: same command succeeds via `MINIBOX_ADAPTER=colima`. _(VF path deferred to Phase 2 — requires in-VM agent.)_
