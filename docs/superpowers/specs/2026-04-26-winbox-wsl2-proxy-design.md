# winbox: WSL2 Proxy Daemon for Windows

**Date**: 2026-04-26
**Status**: Draft
**Scope**: `crates/winbox/` + `crates/miniboxd/src/main.rs` (Windows path) +
`crates/mbx/` (client transport)
**GH Issues**: #45, #87

## Overview

Make minibox functional on Windows by implementing a **WSL2 proxy** architecture:
`winboxd.exe` listens on a Windows Named Pipe (`\\.\pipe\miniboxd`), relays
JSON-over-newline traffic to a `miniboxd` instance running inside WSL2 via `socat`,
and forwards responses back to the Windows client. No native HCS containers in this
phase -- containers run inside WSL2's Linux kernel using the existing native adapter.

This is the fastest path to Windows support. Native HCS/WSL2-kernel adapters remain
Phase 3 future work.

## Architecture

```
Windows host                          WSL2 distro
--------------                        -----------
mbx.exe --Named Pipe--> winboxd.exe   socat relay --Unix socket--> miniboxd
         (client)        (proxy)       (child proc)                 (native)
```

**Transport chain**: Named Pipe (Windows) --> stdin/stdout pipe (socat child) -->
Unix socket (WSL2).

winboxd.exe is NOT a full daemon -- it has no handler logic, no state, no adapters.
It is a transparent byte relay with preflight checks.

## Phased Delivery

### Phase 1: Named Pipe listener + socat relay (this spec)

- `winbox::start()` becomes a real entry point (replaces bail stub)
- Named Pipe server using `tokio::net::windows::named_pipe`
- Preflight: verify WSL2 distro running, miniboxd socket present
- Per-connection: spawn `wsl.exe -- socat - UNIX-CONNECT:/run/minibox/miniboxd.sock`
- Bidirectional byte relay between Named Pipe and socat stdin/stdout
- Auth: Named Pipe ACL (owner-only) -- no SO_PEERCRED equivalent

### Phase 2: Auto-provision miniboxd inside WSL2 (future)

- `winbox` detects whether miniboxd is running inside WSL2
- If not: copies/installs miniboxd binary into the WSL2 distro
- Starts miniboxd as a background process
- Health-check the socket before accepting connections

### Phase 3: Native HCS / WSL2-kernel adapters (future)

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
+-- relay.rs           -- NEW: SocatRelay, bidirectional byte pump
+-- pipe_listener.rs   -- NEW: NamedPipeListener impl ServerListener
+-- hcs.rs             -- stub (unchanged)
+-- wsl2.rs            -- stub (unchanged)
```

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

## SocatRelay

Per-connection relay that bridges the Named Pipe stream to miniboxd inside WSL2.

```rust
pub struct SocatRelay {
    distro: String,
    socket_path: String,
}

impl SocatRelay {
    pub fn new(distro: &str, socket_path: &str) -> Self { ... }

    /// Spawn socat child and relay bytes bidirectionally.
    ///
    /// Returns when either side closes or an error occurs.
    pub async fn relay<S>(&self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
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

        // Bidirectional copy: pipe-->socat and socat-->pipe
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

## Preflight Changes

Add `check_socket()` to the existing `preflight.rs`:

```rust
/// Verify miniboxd socket exists inside WSL2 distro.
pub fn check_socket(exec: &Executor, socket_path: &str) -> bool {
    exec(&["wsl", "-d", DEFAULT_DISTRO, "--", "test", "-S", socket_path])
        .is_ok()
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

    let listener = NamedPipeListener::bind(&pipe_name)?;
    info!(pipe = %pipe_name, "listening");

    let relay = Arc::new(SocatRelay::new(&distro, &socket_path));

    // Accept loop -- no root auth on Windows
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
```

No new crates required -- tokio's Windows Named Pipe support is built-in.
`socat` must be installed inside the WSL2 distro (standard package).

## Testing Strategy

| Layer          | Approach                                     | Platform     |
| -------------- | -------------------------------------------- | ------------ |
| `preflight`    | Injectable executor (existing pattern)       | Any (mocked) |
| `paths`        | Unit tests (existing)                        | Any          |
| `relay`        | Integration test with mock socat (echo loop) | Windows      |
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
- HCS/WSL2 adapter stubs in `linuxbox/src/adapters/` -- untouched (Phase 3).

## Open Questions

1. **Default WSL2 distro name**: `Ubuntu` is the most common default, but users
   may have a different distro. Should we auto-detect via `wsl --list --verbose`?
   Current answer: env var override is sufficient for Phase 1.

2. **socat availability**: Not all WSL2 distros have socat pre-installed. Phase 1
   requires manual install (`apt install socat`). Phase 2 could auto-install or
   bundle a static socat binary.

3. **Connection pooling**: Each client connection spawns a new socat process.
   Acceptable for dev use. If performance matters, Phase 2 could maintain a
   persistent socat connection pool.
