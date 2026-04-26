# winbox: WSL2 Proxy Daemon for Windows

**Date**: 2026-04-26
**Status**: Draft
**Scope**: `crates/winbox/` + `crates/miniboxd/src/main.rs` (Windows path) +
`crates/mbx/` (client transport)
**GH Issues**: #45, #87

## Overview

Make minibox functional on Windows by implementing a **WSL2 proxy** architecture:
`winboxd.exe` listens on a Windows Named Pipe (`\\.\pipe\miniboxd`), relays
JSON-over-newline traffic to a `miniboxd` instance running inside WSL2, and
forwards responses back to the Windows client. No native HCS containers in this
phase -- containers run inside WSL2's Linux kernel using the existing native
adapter.

Phase 1 uses a socat-based per-connection relay (simplest, no admin privileges).
Phase 2 upgrades to AF_HYPERV/vsock for persistent kernel-level transport
(Podman-proven pattern, eliminates per-connection process spawn overhead).

## Architecture

### Phase 1: socat relay

```
Windows host                          WSL2 distro
--------------                        -----------
mbx.exe --Named Pipe--> winboxd.exe   socat relay --Unix socket--> miniboxd
         (client)        (proxy)       (child proc)                 (native)
```

**Transport chain**: Named Pipe --> stdin/stdout pipe (socat child) --> Unix
socket (WSL2).

### Phase 2: Hyper-V socket relay

```
Windows host                          WSL2 distro
--------------                        -----------
mbx.exe --Named Pipe--> winboxd.exe --AF_HYPERV--> vsock-shim --> miniboxd
         (client)        (proxy)       (kernel)     (in-VM)        (native)
```

**Transport chain**: Named Pipe --> AF_HYPERV socket (kernel, zero-copy) -->
AF_VSOCK listener inside WSL2 --> Unix socket.

winboxd.exe is NOT a full daemon -- it has no handler logic, no state, no
adapters. It is a transparent byte relay with preflight checks.

## Phased Delivery

### Phase 1: Named Pipe listener + socat relay (this spec)

- `winbox::start()` becomes a real entry point (replaces bail stub)
- Named Pipe server using `tokio::net::windows::named_pipe`
- Preflight: verify WSL2 distro running, miniboxd socket present
- Per-connection: spawn `wsl.exe -- socat - UNIX-CONNECT:...`
- Bidirectional byte relay between Named Pipe and socat stdin/stdout
- Auth: Named Pipe ACL (owner-only) -- no SO_PEERCRED equivalent

### Phase 2: AF_HYPERV / vsock upgrade

- Replace socat relay with persistent AF_HYPERV socket connection
- Inside WSL2: small `minibox-vsock-shim` binary listens on AF_VSOCK,
  forwards to miniboxd Unix socket (or miniboxd listens on vsock directly)
- One-time admin setup: register Hyper-V socket GUID in Windows Registry
  (same pattern as Podman Machine)
- Connection multiplexing over a single kernel socket -- no per-connection
  process spawn
- Health monitoring: winboxd pings over vsock to detect miniboxd availability

**Prior art**: Podman Machine on Windows uses exactly this pattern via
[gvisor-tap-vsock/gvproxy](https://github.com/containers/gvisor-tap-vsock).
Docker Desktop also uses AF_HYPERV for WSL2 communication. The approach is
production-proven at scale.

**Trade-offs vs socat**:

| Aspect                | socat (Phase 1)             | AF_HYPERV (Phase 2)            |
| --------------------- | --------------------------- | ------------------------------ |
| Per-connection cost   | fork wsl.exe + socat        | Zero (muxed kernel socket)     |
| Latency               | ~50-100ms spawn overhead    | Sub-ms (kernel path)           |
| Dependencies          | socat in WSL2 distro        | None (kernel built-in)         |
| Admin privileges      | None                        | One-time GUID registration     |
| Complexity            | Low                         | Medium (vsock shim + registry) |
| Reliability           | Good (process isolation)    | Better (no process churn)      |

### Phase 3: Auto-provision miniboxd inside WSL2

- `winbox` detects whether miniboxd is running inside WSL2
- If not: copies/installs miniboxd binary into the WSL2 distro
- Starts miniboxd as a background process
- Health-check the socket before accepting connections

### Phase 4: Native HCS / WSL2-kernel adapters (future)

- `HcsRuntime`, `Wsl2Runtime` become real adapter implementations
- winboxd becomes a full daemon with its own `HandlerDependencies`
- Named Pipe server talks directly to daemonbox handler, no relay

## Module Layout

```
crates/winbox/src/
+-- lib.rs             -- start(), WinboxError, module declarations
+-- paths.rs           -- data_dir(), run_dir(), pipe_name() [exists]
+-- preflight.rs       -- WinboxStatus, check_wsl2(), check_socket()
|                         [exists, needs new check_socket()]
+-- relay.rs           -- NEW: WslRelay trait + SocatRelay impl
+-- pipe_listener.rs   -- NEW: NamedPipeListener impl ServerListener
+-- hcs.rs             -- stub (unchanged)
+-- wsl2.rs            -- stub (unchanged)
```

Phase 2 additions:

```
+-- relay/
|   +-- mod.rs         -- WslRelay trait definition
|   +-- socat.rs       -- SocatRelay (Phase 1)
|   +-- hyperv.rs      -- HyperVRelay (Phase 2)
+-- vsock_shim/        -- Standalone binary for WSL2 side (Phase 2)
```

## WslRelay Trait

Abstract the relay mechanism so Phase 2 is a drop-in replacement:

```rust
use anyhow::Result;
use tokio::io::{AsyncRead, AsyncWrite};

#[async_trait::async_trait]
pub trait WslRelay: Send + Sync {
    /// Relay bytes bidirectionally between the Named Pipe stream and the
    /// WSL2 miniboxd instance. Returns when either side closes.
    async fn relay<S>(&self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static;
}
```

`start()` is generic over `WslRelay`, selected at startup based on
availability: try AF_HYPERV first (Phase 2), fall back to socat (Phase 1).

## NamedPipeListener

Implements `daemonbox::server::ServerListener` over Windows Named Pipes. This is
the Windows equivalent of `MacUnixListener` in macbox.

```rust
use tokio::net::windows::named_pipe::{ServerOptions, NamedPipeServer};

pub struct NamedPipeListener {
    pipe_name: String,
}

impl NamedPipeListener {
    pub fn bind(pipe_name: &str) -> Result<Self> {
        // Create first instance to validate the name
        let _server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe_name)
            .context("binding Named Pipe")?;
        Ok(Self { pipe_name: pipe_name.to_string() })
    }
}

impl ServerListener for NamedPipeListener {
    type Stream = NamedPipeServer;

    async fn accept(&self) -> Result<(Self::Stream, Option<PeerCreds>)> {
        let server = ServerOptions::new()
            .create(&self.pipe_name)
            .context("creating pipe instance")?;
        server.connect().await.context("waiting for client")?;
        // Named Pipes have no SO_PEERCRED; auth via pipe ACL
        Ok((server, None))
    }
}
```

**Note**: `NamedPipeServer` implements `AsyncRead + AsyncWrite`, satisfying the
`ServerListener::Stream` bound. Each `accept()` creates a new pipe instance --
this is the standard Windows Named Pipe server pattern.

**Constraint**: `daemonbox::server::ServerListener` requires `Stream: Unpin`.
`NamedPipeServer` is `Unpin` in tokio. Verify at build time.

## SocatRelay (Phase 1)

Per-connection relay that bridges the Named Pipe stream to miniboxd inside WSL2.

```rust
pub struct SocatRelay {
    distro: String,
    socket_path: String,
}

#[async_trait::async_trait]
impl WslRelay for SocatRelay {
    async fn relay<S>(&self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let mut child = tokio::process::Command::new("wsl.exe")
            .args(["-d", &self.distro, "--", "socat", "-",
                   &format!("UNIX-CONNECT:{}", self.socket_path)])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("spawning socat relay")?;

        let child_stdin = child.stdin.take().context("socat stdin")?;
        let child_stdout = child.stdout.take().context("socat stdout")?;

        let (read_half, write_half) = tokio::io::split(stream);

        let to_socat = tokio::io::copy(&mut read_half, &mut child_stdin);
        let from_socat = tokio::io::copy(&mut child_stdout, &mut write_half);

        tokio::select! {
            r = to_socat => { r.context("pipe->socat")?; }
            r = from_socat => { r.context("socat->pipe")?; }
        };

        child.kill().await.ok();
        Ok(())
    }
}
```

**Key design decisions**:

- `socat` over raw `wsl.exe` stdin forwarding: socat handles partial reads,
  buffering, and clean shutdown. Raw `wsl.exe` stdin has known truncation issues
  with binary-like JSON payloads.
- `stderr` inherits to the daemon's stderr for debugging visibility.
- `child.kill()` on relay completion prevents orphaned socat processes.

## HyperVRelay (Phase 2)

Persistent kernel-level relay using AF_HYPERV sockets. This is the same
transport Podman Machine and Docker Desktop use for WSL2 communication.

```rust
pub struct HyperVRelay {
    vm_id: uuid::Uuid,       // WSL2 VM GUID
    service_id: uuid::Uuid,  // Registered Hyper-V socket service GUID
}

#[async_trait::async_trait]
impl WslRelay for HyperVRelay {
    async fn relay<S>(&self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        // Connect to the vsock shim inside WSL2 via AF_HYPERV
        let hv_stream = hyperv_connect(self.vm_id, self.service_id)
            .await
            .context("connecting to WSL2 via AF_HYPERV")?;

        let (pipe_read, pipe_write) = tokio::io::split(stream);
        let (hv_read, hv_write) = tokio::io::split(hv_stream);

        let to_vm = tokio::io::copy(&mut pipe_read, &mut hv_write);
        let from_vm = tokio::io::copy(&mut hv_read, &mut pipe_write);

        tokio::select! {
            r = to_vm => { r.context("pipe->hyperv")?; }
            r = from_vm => { r.context("hyperv->pipe")?; }
        };

        Ok(())
    }
}

/// Connect to a Hyper-V socket (AF_HYPERV).
///
/// Requires one-time admin setup:
///   1. Register service GUID in Windows Registry under
///      HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\
///      GuestCommunicationServices\{service_id}
///   2. WSL2 VM must be running
///
/// Uses raw Windows socket API since tokio has no AF_HYPERV wrapper.
async fn hyperv_connect(
    vm_id: uuid::Uuid,
    service_id: uuid::Uuid,
) -> Result<impl AsyncRead + AsyncWrite> {
    // Implementation uses windows-sys or windows crate for:
    //   socket(AF_HYPERV, SOCK_STREAM, HV_PROTOCOL_RAW)
    //   connect(sockaddr_hv { vm_id, service_id })
    // Then wraps in tokio::io via AsyncFd or spawn_blocking
    todo!("Phase 2 implementation")
}
```

### vsock-shim (WSL2 side)

Small Rust binary that runs inside the WSL2 distro, bridging AF_VSOCK to the
miniboxd Unix socket:

```rust
// crates/minibox-vsock-shim/src/main.rs
// Listens on AF_VSOCK port 6789, forwards each connection to
// /run/minibox/miniboxd.sock via Unix socket.
// Stateless, single-purpose, ~50 lines.
```

**Alternative**: miniboxd itself could listen on AF_VSOCK directly (add a
`--vsock-port` flag). This eliminates the shim but couples miniboxd to the
Windows deployment model. Defer this decision to Phase 2 implementation.

### One-time setup

Phase 2 requires a one-time admin command to register the Hyper-V socket:

```powershell
# Run once as Administrator
$guid = "00000000-facb-11e6-bd58-64006a7986d3"  # minibox service GUID
$path = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\GuestCommunicationServices\$guid"
New-Item -Path $path -Force
New-ItemProperty -Path $path -Name "ElementName" -Value "minibox" -Force
```

`winbox preflight` detects whether the GUID is registered and reports the
appropriate status. If unregistered, Phase 1 socat relay is used as fallback.

## Preflight Changes

Add `check_socket()` to the existing `preflight.rs`:

```rust
/// Verify miniboxd socket exists inside WSL2 distro.
pub fn check_socket(exec: &Executor, socket_path: &str) -> bool {
    exec(&["wsl", "-d", DEFAULT_DISTRO, "--", "test", "-S", socket_path])
        .is_ok()
}

/// Check if the Hyper-V socket GUID is registered (Phase 2).
pub fn check_hyperv_guid(service_id: &str) -> bool {
    let path = format!(
        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Virtualization\GuestCommunicationServices\{}",
        service_id
    );
    // Query registry -- returns true if key exists
    std::process::Command::new("reg")
        .args(["query", &path])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
```

Update `preflight()` to return a richer status:

```rust
pub enum WinboxStatus {
    Wsl2Ready,          // WSL2 running + miniboxd socket present
    Wsl2NoSocket,       // WSL2 running but no miniboxd socket
    Hcs,                // HCS available (future)
    HcsAndWsl2,         // Both available (future)
    NoBackendAvailable, // Neither
}

/// Relay transport available on this system.
pub enum RelayTransport {
    HyperV,   // AF_HYPERV GUID registered + vsock shim running
    Socat,    // socat available in WSL2 distro
    None,     // Neither -- cannot relay
}

pub fn detect_transport(exec: &Executor) -> RelayTransport {
    if check_hyperv_guid(MINIBOX_SERVICE_GUID) {
        return RelayTransport::HyperV;
    }
    if exec(&["wsl", "-d", DEFAULT_DISTRO, "--", "which", "socat"]).is_ok() {
        return RelayTransport::Socat;
    }
    RelayTransport::None
}
```

## start() Implementation

Replace the Phase 1 stub:

```rust
pub async fn start() -> Result<()> {
    init_tracing();
    info!("miniboxd (Windows/WSL2 proxy) starting");

    let exec = default_executor();
    let status = preflight::preflight(&exec);

    match status {
        WinboxStatus::Wsl2Ready => {}
        WinboxStatus::Wsl2NoSocket => {
            anyhow::bail!(
                "WSL2 is running but miniboxd socket not found at {}. \
                 Start miniboxd inside WSL2 first: wsl -d {} -- sudo miniboxd",
                DEFAULT_SOCKET_PATH, DEFAULT_DISTRO
            );
        }
        _ => return Err(WinboxError::NoBackendAvailable.into()),
    }

    let pipe_name = std::env::var("MINIBOX_PIPE_NAME")
        .unwrap_or_else(|_| paths::pipe_name());
    let socket_path = std::env::var("MINIBOX_WSL_SOCKET")
        .unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string());
    let distro = std::env::var("MINIBOX_WSL_DISTRO")
        .unwrap_or_else(|_| DEFAULT_DISTRO.to_string());

    // Select relay transport: prefer AF_HYPERV, fall back to socat
    let relay: Arc<dyn WslRelay> = match preflight::detect_transport(&exec) {
        RelayTransport::HyperV => {
            info!("using AF_HYPERV relay (kernel transport)");
            Arc::new(HyperVRelay::new()?)
        }
        RelayTransport::Socat => {
            info!("using socat relay (process-per-connection)");
            Arc::new(SocatRelay::new(&distro, &socket_path))
        }
        RelayTransport::None => {
            anyhow::bail!(
                "no relay transport available. Install socat in WSL2 \
                 (`wsl -d {} -- sudo apt install socat`) or register \
                 the Hyper-V socket GUID (see docs)",
                distro
            );
        }
    };

    let listener = NamedPipeListener::bind(&pipe_name)?;
    info!(pipe = %pipe_name, "listening");

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            result = listener.accept_raw() => {
                let stream = result?;
                let relay = Arc::clone(&relay);
                tokio::spawn(async move {
                    if let Err(e) = relay.relay(stream).await {
                        warn!(error = %e, "relay: connection error");
                    }
                });
            }
            _ = &mut shutdown => {
                info!("shutdown signal received");
                break;
            }
        }
    }

    info!("miniboxd (Windows/WSL2 proxy) stopped");
    Ok(())
}
```

**Note**: The proxy does NOT use `daemonbox::server::run_server` -- it bypasses
the handler layer entirely. Each connection is a raw byte relay to the WSL2
miniboxd instance. This keeps winboxd stateless and simple.

## Client Transport (mbx)

`crates/mbx/` needs a Windows transport path. The client currently connects via
Unix socket. Add a `#[cfg(target_os = "windows")]` path using
`tokio::net::windows::named_pipe::ClientOptions`:

```rust
#[cfg(target_os = "windows")]
async fn connect() -> Result<impl AsyncRead + AsyncWrite> {
    use tokio::net::windows::named_pipe::ClientOptions;
    let pipe_name = std::env::var("MINIBOX_PIPE_NAME")
        .unwrap_or_else(|_| r"\\.\pipe\miniboxd".to_string());
    let client = ClientOptions::new()
        .open(&pipe_name)
        .context("connecting to Named Pipe")?;
    Ok(client)
}
```

Also update `minibox-client` (`DaemonClient`) with the same platform gate.

## Environment Variables

| Variable              | Default                          | Purpose                            |
| --------------------- | -------------------------------- | ---------------------------------- |
| `MINIBOX_PIPE_NAME`   | `\\.\pipe\miniboxd`             | Named Pipe path                    |
| `MINIBOX_WSL_DISTRO`  | `Ubuntu`                         | WSL2 distro name                   |
| `MINIBOX_WSL_SOCKET`  | `/run/minibox/miniboxd.sock`     | Socket path inside WSL2            |
| `MINIBOX_DATA_DIR`    | `%APPDATA%\minibox`              | Windows-side data (unused Phase 1) |
| `MINIBOX_RUN_DIR`     | `%LOCALAPPDATA%\minibox`         | Windows-side runtime               |
| `MINIBOX_RELAY`       | (auto-detect)                    | Force relay: `socat` or `hyperv`   |

## Auth Model

Named Pipes support ACLs natively. Phase 1 uses the default ACL (pipe creator
only). `require_root_auth` is `false` -- the proxy is stateless and all
privileged operations happen inside WSL2 where miniboxd enforces SO_PEERCRED.

Future: add `SecurityDescriptor` to restrict pipe access to Administrators group.

## Dependencies

New deps for winbox (Windows-only):

```toml
[target.'cfg(target_os = "windows")'.dependencies]
tokio = { workspace = true, features = ["net", "process", "io-util"] }
async-trait = { workspace = true }
```

Phase 2 additions:

```toml
uuid = { version = "1", features = ["v4"] }
windows-sys = { version = "0.59", features = ["Win32_Networking_WinSock"] }
```

No new crates for Phase 1 -- tokio's Windows Named Pipe support is built-in.
`socat` must be installed inside the WSL2 distro (standard package).

## Testing Strategy

| Layer          | Approach                                     | Platform     |
| -------------- | -------------------------------------------- | ------------ |
| `preflight`    | Injectable executor (existing pattern)       | Any (mocked) |
| `paths`        | Unit tests (existing)                        | Any          |
| `WslRelay`     | Mock impl returning canned bytes             | Any          |
| `SocatRelay`   | Integration test with mock socat (echo loop) | Windows      |
| `HyperVRelay`  | Integration test with vsock loopback         | Windows      |
| `pipe_listener`| Structural test: bind + connect + roundtrip  | Windows      |
| `start()`      | E2E: winboxd + WSL2 miniboxd + mbx.exe       | Windows+WSL2 |

Cross-platform CI note: relay and pipe_listener tests are `#[cfg(target_os =
"windows")]`. They won't run on Linux/macOS CI. The existing macOS CI
(`cargo check --workspace`) will continue to skip winbox due to `compile_error!`.

## What Does Not Change

- `daemonbox` -- no modifications needed. The proxy bypasses daemonbox entirely.
- `linuxbox` / `minibox-core` -- no changes.
- `macbox` -- no changes.
- Linux `miniboxd` entry point -- no changes.
- HCS/WSL2 adapter stubs in `linuxbox/src/adapters/` -- untouched (Phase 4).

## References

- [npiperelay](https://github.com/jstarks/npiperelay) -- Go tool bridging
  Named Pipes to WSL2 stdin/stdout (same pattern as our socat relay)
- [gvisor-tap-vsock](https://github.com/containers/gvisor-tap-vsock) -- Podman's
  gvproxy using AF_VSOCK for host-VM communication
- [Podman Machine architecture](https://www.redhat.com/en/blog/podman-mac-machine-architecture) --
  how Podman uses vsock + gvproxy on macOS/Windows
- [Hyper-V socket integration services](https://learn.microsoft.com/en-us/windows-server/virtualization/hyper-v/make-integration-service) --
  Microsoft docs on AF_HYPERV GUID registration
- [WSL2 networking architecture](https://deepwiki.com/microsoft/WSL/3.3-networking-architecture) --
  WSL2 internal networking and vsock usage

## Open Questions

1. **Default WSL2 distro name**: `Ubuntu` is the most common default, but users
   may have a different distro. Should we auto-detect via `wsl --list --verbose`?
   Current answer: env var override is sufficient for Phase 1.

2. **socat availability**: Not all WSL2 distros have socat pre-installed. Phase 1
   requires manual install (`apt install socat`). Phase 3 (auto-provision) could
   install socat as part of miniboxd setup.

3. **vsock shim vs miniboxd flag**: Phase 2 needs a vsock listener inside WSL2.
   Options: (a) standalone `minibox-vsock-shim` binary, (b) `miniboxd --vsock`
   flag. Decision deferred to Phase 2 -- shim is simpler, flag is cleaner.

4. **AF_HYPERV Rust bindings**: No mature Rust crate for AF_HYPERV sockets.
   Phase 2 will use raw `windows-sys` FFI. If a community crate emerges before
   implementation, prefer it.
