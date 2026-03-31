# macOS VZ.framework — VzAdapter + vsock + Full End-to-End Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the VZ.framework VM into a full `VzAdapter` implementing all four domain traits (`ImageRegistry`, `FilesystemProvider`, `ResourceLimiter`, `ContainerRuntime`) by proxying JSON-over-newline commands to minibox-agent inside the VM over vsock, replacing the Colima adapter on macOS when `MINIBOX_ADAPTER=vz` is set.

**Architecture:** `VzAdapter` is a composition root that holds an `Arc<VzVm>` (booted lazily on first use) and a vsock connection pool. Each trait method serializes a `DaemonRequest` variant, writes it newline-delimited over vsock port 9000, and reads back `DaemonResponse` replies. The in-VM `miniboxd` (`minibox-agent`) already implements all the logic; the adapter is pure forwarding with no business logic. `macbox::start()` gains a `vz` feature branch: if `MINIBOX_ADAPTER=vz` and the `vz` feature is compiled in, it boots a `VzVm` and constructs a `HandlerDependencies` backed by the four `VzAdapter` instances. The minibox-agent's PID-1 init script mounts `/proc`, `/sys`, `/dev`, the two virtiofs shares, brings up loopback, then execs `miniboxd` with `MINIBOX_ADAPTER=native` on vsock.

**Tech Stack:** Rust, `tokio` (vsock via `tokio-vsock` crate), `serde_json`, `objc2` VZ.framework bindings (from Plan A), existing minibox JSON protocol.

**Prerequisite:** Plan A (`2026-03-29-macos-vz-vm-image-pipeline.md`) must be complete — VM image directory must exist at `~/.mbx/vm/`.

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `crates/macbox/src/vz/vsock.rs` | Async vsock connection to guest port 9000 |
| Create | `crates/macbox/src/vz/proxy.rs` | `VzProxy` — send request, stream responses |
| Create | `crates/macbox/src/vz/adapter.rs` | Four `VzAdapter*` structs implementing domain traits |
| Modify | `crates/macbox/src/vz/mod.rs` | Re-export vsock, proxy, adapter |
| Modify | `crates/macbox/src/lib.rs` | VZ branch in `start()` |
| Modify | `crates/macbox/Cargo.toml` | Add `tokio-vsock` to `vz` feature |
| Create | `crates/macbox/src/vz/agent_init.rs` | Alpine init script builder (written to rootfs) |
| Modify | `crates/miniboxd/src/main.rs` | `MINIBOX_ADAPTER=vz` dispatch on macOS |
| Create | `crates/macbox/tests/vz_adapter_smoke.rs` | Integration smoke test (requires VM image) |

---

### Task 1: vsock connection module

**Files:**
- Modify: `crates/macbox/Cargo.toml`
- Create: `crates/macbox/src/vz/vsock.rs`

The guest listens on `AF_VSOCK` CID=3 port 9000. On macOS host we use `VZVirtioSocketDevice` to create client connections. The `tokio-vsock` crate provides `VsockStream` which wraps the macOS VM vsock fd.

- [ ] **Step 1: Add `tokio-vsock` to Cargo.toml**

```toml
# In crates/macbox/Cargo.toml, under [features] vz = [...]:
# Add "dep:tokio-vsock" to the vz feature list.

[dependencies]
tokio-vsock = { version = "0.5", optional = true }
```

And add `"dep:tokio-vsock"` to the `vz` feature list.

- [ ] **Step 2: Write the failing test**

```rust
// crates/macbox/src/vz/vsock.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_addr_constants() {
        assert_eq!(GUEST_CID, 3);
        assert_eq!(AGENT_PORT, 9000);
    }
}
```

- [ ] **Step 3: Run test to confirm it fails**

```
cargo test -p macbox --features vz guest_addr_constants
```
Expected: compile error — module does not exist.

- [ ] **Step 4: Implement**

```rust
// crates/macbox/src/vz/vsock.rs
//! Async vsock connection helpers for the minibox VZ guest.

use anyhow::{Context, Result};
use tokio_vsock::VsockStream;

/// CID assigned to the Linux guest by Virtualization.framework.
pub const GUEST_CID: u32 = 3;
/// Port minibox-agent listens on inside the VM.
pub const AGENT_PORT: u32 = 9000;

/// Open a single vsock connection to the guest agent.
///
/// Retries up to `max_attempts` times with 500ms delay to allow the agent
/// time to come up after VM boot.
pub async fn connect_to_agent(max_attempts: u32) -> Result<VsockStream> {
    let mut last_err = anyhow::anyhow!("no attempts made");
    for attempt in 0..max_attempts {
        match VsockStream::connect(GUEST_CID, AGENT_PORT).await {
            Ok(stream) => {
                tracing::debug!(attempt, "vz: vsock connected to agent");
                return Ok(stream);
            }
            Err(e) => {
                last_err = anyhow::anyhow!("vsock connect attempt {attempt}: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
    Err(last_err).context("vz: could not connect to agent after retries")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_addr_constants() {
        assert_eq!(GUEST_CID, 3);
        assert_eq!(AGENT_PORT, 9000);
    }
}
```

- [ ] **Step 5: Run test**

```
cargo test -p macbox --features vz guest_addr_constants
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/macbox/src/vz/vsock.rs crates/macbox/Cargo.toml
git commit -m "feat(macbox/vz): vsock connect_to_agent with retry"
```

---

### Task 2: `VzProxy` — serialize requests, stream responses

**Files:**
- Create: `crates/macbox/src/vz/proxy.rs`

`VzProxy` wraps a vsock connection and implements the same request/response pattern as `minibox-client`: write one JSON line, read JSON lines until a terminal response.

- [ ] **Step 1: Write the failing test**

```rust
// crates/macbox/src/vz/proxy.rs
#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::protocol::{DaemonRequest, DaemonResponse};
    use tokio::io::duplex;

    #[tokio::test]
    async fn proxy_reads_single_terminal_response() {
        // Simulate a stream that returns a single Success response.
        let (client, mut server) = duplex(1024);
        let resp = DaemonResponse::Success { message: "ok".into() };
        let line = serde_json::to_string(&resp).unwrap() + "\n";
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            server.write_all(line.as_bytes()).await.unwrap();
        });

        let mut proxy = VzProxy::new(client);
        let req = DaemonRequest::ListContainers;
        let responses = proxy.send_request(&req).await.unwrap();
        assert_eq!(responses.len(), 1);
        assert!(matches!(&responses[0], DaemonResponse::Success { .. }));
    }
}
```

- [ ] **Step 2: Run test to confirm it fails**

```
cargo test -p macbox --features vz proxy_reads_single
```
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
// crates/macbox/src/vz/proxy.rs
//! JSON-over-newline request/response proxy for the minibox-agent vsock channel.

use anyhow::{Context, Result};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Determines whether a response ends the stream for a given request.
fn is_terminal(resp: &DaemonResponse) -> bool {
    !matches!(resp, DaemonResponse::ContainerOutput { .. })
}

/// A single vsock connection to the in-VM agent. Not `Clone` — each request
/// that needs streaming should hold its own connection (connections are cheap).
pub struct VzProxy<S> {
    stream: S,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> VzProxy<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    /// Send `req` and collect all responses until the terminal one.
    ///
    /// For non-streaming requests (pull, stop, rm, ps) this returns a Vec with
    /// one element. For ephemeral run, it returns ContainerOutput... ContainerStopped.
    pub async fn send_request(&mut self, req: &DaemonRequest) -> Result<Vec<DaemonResponse>> {
        // Write request line
        let mut line = serde_json::to_string(req).context("serializing request")?;
        line.push('\n');
        let (reader_half, mut writer_half) = tokio::io::split(&mut self.stream);
        writer_half
            .write_all(line.as_bytes())
            .await
            .context("writing request to vsock")?;

        // Read responses
        let mut buf_reader = BufReader::new(reader_half);
        let mut responses = Vec::new();
        loop {
            let mut resp_line = String::new();
            let n = buf_reader
                .read_line(&mut resp_line)
                .await
                .context("reading response from vsock")?;
            if n == 0 {
                // EOF — agent closed connection
                break;
            }
            let resp: DaemonResponse =
                serde_json::from_str(resp_line.trim()).context("parsing response")?;
            let terminal = is_terminal(&resp);
            responses.push(resp);
            if terminal {
                break;
            }
        }
        Ok(responses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::protocol::{DaemonRequest, DaemonResponse};
    use tokio::io::duplex;

    #[tokio::test]
    async fn proxy_reads_single_terminal_response() {
        let (client, mut server) = duplex(1024);
        let resp = DaemonResponse::Success { message: "ok".into() };
        let line = serde_json::to_string(&resp).unwrap() + "\n";
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            server.write_all(line.as_bytes()).await.unwrap();
        });
        let mut proxy = VzProxy::new(client);
        let req = DaemonRequest::ListContainers;
        let responses = proxy.send_request(&req).await.unwrap();
        assert_eq!(responses.len(), 1);
        assert!(matches!(&responses[0], DaemonResponse::Success { .. }));
    }

    #[tokio::test]
    async fn proxy_collects_streaming_output() {
        use minibox_core::protocol::OutputStreamKind;
        let (client, mut server) = duplex(4096);
        let output = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: "hello\n".into(),
        };
        let stopped = DaemonResponse::ContainerStopped {
            exit_code: 0,
        };
        let mut out_line = serde_json::to_string(&output).unwrap();
        out_line.push('\n');
        let mut stop_line = serde_json::to_string(&stopped).unwrap();
        stop_line.push('\n');
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            server.write_all(out_line.as_bytes()).await.unwrap();
            server.write_all(stop_line.as_bytes()).await.unwrap();
        });
        let mut proxy = VzProxy::new(client);
        let req = DaemonRequest::ListContainers;
        let responses = proxy.send_request(&req).await.unwrap();
        assert_eq!(responses.len(), 2);
        assert!(matches!(&responses[0], DaemonResponse::ContainerOutput { .. }));
        assert!(matches!(&responses[1], DaemonResponse::ContainerStopped { .. }));
    }
}
```

- [ ] **Step 4: Run tests**

```
cargo test -p macbox --features vz proxy_reads proxy_collects
```
Expected: both PASS.

- [ ] **Step 5: Wire new module into `vz/mod.rs`**

```rust
pub mod vsock;
pub mod proxy;
pub mod bindings;
pub mod vm;
pub use vm::VzVm;
pub use proxy::VzProxy;
```

- [ ] **Step 6: Commit**

```bash
git add crates/macbox/src/vz/proxy.rs crates/macbox/src/vz/mod.rs
git commit -m "feat(macbox/vz): VzProxy — JSON-over-vsock request/response"
```

---

### Task 3: Four VzAdapter trait implementations

**Files:**
- Create: `crates/macbox/src/vz/adapter.rs`

Each adapter holds `Arc<VzVm>` and opens a fresh vsock connection per call. For simplicity (and correctness), connections are not pooled — vsock connections inside the VM are cheap.

- [ ] **Step 1: Write failing tests**

```rust
// crates/macbox/src/vz/adapter.rs
#[cfg(test)]
mod tests {
    // Structural tests — verify adapters implement the right traits.
    use super::*;
    use linuxbox::domain::{ContainerRuntime, FilesystemProvider, ImageRegistry, ResourceLimiter};

    fn assert_image_registry<T: ImageRegistry>() {}
    fn assert_container_runtime<T: ContainerRuntime>() {}
    fn assert_filesystem_provider<T: FilesystemProvider>() {}
    fn assert_resource_limiter<T: ResourceLimiter>() {}

    #[test]
    fn adapter_implements_all_traits() {
        assert_image_registry::<VzRegistry>();
        assert_container_runtime::<VzRuntime>();
        assert_filesystem_provider::<VzFilesystem>();
        assert_resource_limiter::<VzLimiter>();
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

```
cargo test -p macbox --features vz adapter_implements_all
```
Expected: compile error.

- [ ] **Step 3: Implement all four adapters**

```rust
// crates/macbox/src/vz/adapter.rs
//! Domain trait adapters that forward to minibox-agent inside the VZ Linux VM.

use anyhow::{Context, Result};
use minibox_core::protocol::{DaemonRequest, DaemonResponse, RunContainer};
use std::sync::Arc;
use tokio::runtime::Handle;

use super::vm::VzVm;
use super::vsock::connect_to_agent;
use super::proxy::VzProxy;

// ── Shared helper ────────────────────────────────────────────────────────────

/// Open a vsock connection, send `req`, return the first terminal response.
///
/// Panics if called outside a Tokio runtime context (all callers go via
/// `spawn_blocking`, so `Handle::current()` is valid).
fn call_agent(req: &DaemonRequest) -> Result<DaemonResponse> {
    let handle = Handle::current();
    handle.block_on(async move {
        let stream = connect_to_agent(60).await?;
        let mut proxy = VzProxy::new(stream);
        let mut responses = proxy.send_request(req).await?;
        responses
            .into_iter()
            .last()
            .context("agent returned no response")
    })
}

// ── VzRegistry ───────────────────────────────────────────────────────────────

pub struct VzRegistry {
    _vm: Arc<VzVm>,
}

impl VzRegistry {
    pub fn new(vm: Arc<VzVm>) -> Self {
        Self { _vm: vm }
    }
}

impl linuxbox::domain::ImageRegistry for VzRegistry {
    fn pull_image(&self, image_ref: &str) -> Result<()> {
        // Split image_ref into name and optional tag (e.g. "alpine:3.21" → "alpine", "3.21")
        let (image, tag) = match image_ref.split_once(':') {
            Some((name, t)) => (name.to_string(), Some(t.to_string())),
            None => (image_ref.to_string(), None),
        };
        let req = DaemonRequest::Pull { image, tag };
        let resp = call_agent(&req)?;
        match resp {
            DaemonResponse::Success { .. } => Ok(()),
            DaemonResponse::Error { message } => {
                anyhow::bail!("agent pull error: {message}")
            }
            other => anyhow::bail!("unexpected pull response: {other:?}"),
        }
    }

    fn image_exists(&self, _image_ref: &str) -> Result<bool> {
        // Return false to always attempt pull — agent caches images inside the VM.
        Ok(false)
    }
}

// ── VzFilesystem ─────────────────────────────────────────────────────────────

pub struct VzFilesystem {
    _vm: Arc<VzVm>,
}

impl VzFilesystem {
    pub fn new(vm: Arc<VzVm>) -> Self {
        Self { _vm: vm }
    }
}

impl linuxbox::domain::FilesystemProvider for VzFilesystem {
    fn create_container_fs(&self, _container_id: &str, _image_ref: &str) -> Result<std::path::PathBuf> {
        // The in-VM agent handles all filesystem setup; the host side doesn't
        // need to do anything. Return a placeholder path that is never used.
        Ok(std::path::PathBuf::from("/vz/placeholder"))
    }

    fn destroy_container_fs(&self, _container_id: &str) -> Result<()> {
        // Cleanup happens inside the VM.
        Ok(())
    }
}

// ── VzLimiter ────────────────────────────────────────────────────────────────

pub struct VzLimiter {
    _vm: Arc<VzVm>,
}

impl VzLimiter {
    pub fn new(vm: Arc<VzVm>) -> Self {
        Self { _vm: vm }
    }
}

impl linuxbox::domain::ResourceLimiter for VzLimiter {
    fn apply_limits(
        &self,
        _container_id: &str,
        _limits: &minibox_core::domain::ResourceLimits,
    ) -> Result<()> {
        // The in-VM agent applies cgroup limits inside the guest — nothing to do on host.
        Ok(())
    }

    fn remove_limits(&self, _container_id: &str) -> Result<()> {
        Ok(())
    }
}

// ── VzRuntime ────────────────────────────────────────────────────────────────

pub struct VzRuntime {
    vm: Arc<VzVm>,
}

impl VzRuntime {
    pub fn new(vm: Arc<VzVm>) -> Self {
        Self { vm }
    }
}

impl linuxbox::domain::ContainerRuntime for VzRuntime {
    fn create_container(
        &self,
        config: &minibox_core::domain::ContainerConfig,
    ) -> Result<minibox_core::domain::ContainerHandle> {
        let (image, tag) = match config.image.split_once(':') {
            Some((name, t)) => (name.to_string(), Some(t.to_string())),
            None => (config.image.clone(), None),
        };
        let req = DaemonRequest::Run {
            image,
            tag,
            command: config.command.clone(),
            memory_limit_bytes: config.memory_limit_bytes,
            cpu_weight: config.cpu_shares.map(|s| s as u64),
            ephemeral: false,
            network: None,
            env: config.env.clone(),
            mounts: config.bind_mounts.iter().map(|m| minibox_core::protocol::BindMount {
                host_path: m.host_path.clone(),
                container_path: m.container_path.clone(),
                read_only: m.read_only,
            }).collect(),
            privileged: config.privileged,
        };
        let resp = call_agent(&req)?;
        match resp {
            DaemonResponse::ContainerCreated { id } => {
                Ok(minibox_core::domain::ContainerHandle { id })
            }
            DaemonResponse::Error { message } => {
                anyhow::bail!("agent run error: {message}")
            }
            other => anyhow::bail!("unexpected run response: {other:?}"),
        }
    }

    fn stop_container(&self, container_id: &str) -> Result<()> {
        let req = DaemonRequest::Stop { id: container_id.to_string() };
        let resp = call_agent(&req)?;
        match resp {
            DaemonResponse::Success { .. } => Ok(()),
            DaemonResponse::Error { message } => anyhow::bail!("agent stop error: {message}"),
            other => anyhow::bail!("unexpected stop response: {other:?}"),
        }
    }

    fn remove_container(&self, container_id: &str) -> Result<()> {
        let req = DaemonRequest::Remove { id: container_id.to_string() };
        let resp = call_agent(&req)?;
        match resp {
            DaemonResponse::Success { .. } => Ok(()),
            DaemonResponse::Error { message } => anyhow::bail!("agent rm error: {message}"),
            other => anyhow::bail!("unexpected rm response: {other:?}"),
        }
    }

    fn list_containers(&self) -> Result<Vec<minibox_core::domain::ContainerRecord>> {
        let req = DaemonRequest::List;
        let resp = call_agent(&req)?;
        match resp {
            DaemonResponse::ContainerList { containers } => {
                // ContainerInfo → ContainerRecord conversion
                Ok(containers.into_iter().map(|c| minibox_core::domain::ContainerRecord {
                    id: c.id,
                    image: c.image,
                    command: c.command,
                    status: c.status,
                    created_at: c.created_at,
                }).collect())
            }
            DaemonResponse::Error { message } => anyhow::bail!("agent ps error: {message}"),
            other => anyhow::bail!("unexpected ps response: {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use linuxbox::domain::{ContainerRuntime, FilesystemProvider, ImageRegistry, ResourceLimiter};

    fn assert_image_registry<T: ImageRegistry>() {}
    fn assert_container_runtime<T: ContainerRuntime>() {}
    fn assert_filesystem_provider<T: FilesystemProvider>() {}
    fn assert_resource_limiter<T: ResourceLimiter>() {}

    #[test]
    fn adapter_implements_all_traits() {
        assert_image_registry::<VzRegistry>();
        assert_container_runtime::<VzRuntime>();
        assert_filesystem_provider::<VzFilesystem>();
        assert_resource_limiter::<VzLimiter>();
    }
}
```

- [ ] **Step 4: Export from `vz/mod.rs`**

```rust
pub mod adapter;
pub use adapter::{VzFilesystem, VzLimiter, VzRegistry, VzRuntime};
```

- [ ] **Step 5: Run tests**

```
cargo test -p macbox --features vz adapter_implements
```
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/macbox/src/vz/adapter.rs crates/macbox/src/vz/mod.rs
git commit -m "feat(macbox/vz): four VzAdapter trait impls forwarding to in-VM agent"
```

---

### Task 4: Agent PID-1 init script

**Files:**
- Create: `crates/macbox/src/vz/agent_init.rs`

The Alpine rootfs has `/sbin/init → minibox-agent`, but minibox-agent expects a running system (mounts, network). We need a shell init script or we run minibox-agent directly as PID 1 and handle init duties in code. **Design decision from spec:** minibox-agent IS miniboxd and handles init in code (mount /proc, /sys, etc. before entering the accept loop). We need a small `/etc/inittab`-compatible entry or we rely on passing `init=/sbin/minibox-agent` to the kernel cmdline.

Since we already set `init=/sbin/init` → `minibox-agent` in the kernel cmdline and `minibox-agent` IS `miniboxd`, we need `miniboxd` to detect "running as PID 1" and perform init duties before starting the accept loop.

This task writes the `agent_init.rs` helper that generates an `/etc/init.d/minibox-agent` Alpine init script and installs it into the rootfs so the agent starts properly. It also writes `/etc/inittab` to skip the Alpine default getty and boot straight to minibox-agent.

- [ ] **Step 1: Write failing test**

```rust
// crates/macbox/src/vz/agent_init.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inittab_contains_agent_entry() {
        let content = generate_inittab();
        assert!(content.contains("minibox-agent"));
        assert!(content.contains("::sysinit:"));
    }

    #[test]
    fn rc_local_mounts_proc_and_sys() {
        let content = generate_rc_local();
        assert!(content.contains("mount -t proc"));
        assert!(content.contains("mount -t sysfs"));
        assert!(content.contains("mount -t devtmpfs"));
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

```
cargo test -p macbox --features vz inittab_contains agent_mounts
```
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
// crates/macbox/src/vz/agent_init.rs
//! Generates Alpine init configuration files installed into the VM rootfs.
//!
//! These are written by `cargo xtask build-vm-image` into `rootfs/etc/`
//! so the VM boots directly into minibox-agent.

use anyhow::{Context, Result};
use std::path::Path;

/// Generate `/etc/inittab` content that boots minibox-agent as PID 1 supervisor.
pub fn generate_inittab() -> String {
    // Alpine busybox inittab format: id:runlevel:action:process
    "::sysinit:/etc/init.d/rcS
::once:/sbin/minibox-agent
::ctrlaltdel:/sbin/reboot
::shutdown:/bin/umount -a -r
"
    .to_string()
}

/// Generate `/etc/init.d/rcS` content: minimal system prep before agent starts.
pub fn generate_rc_local() -> String {
    r#"#!/bin/sh
# Minimal system initialization for minibox-agent VM
set -e

# Mount virtual filesystems
mount -t proc proc /proc 2>/dev/null || true
mount -t sysfs sys /sys 2>/dev/null || true
mount -t devtmpfs dev /dev 2>/dev/null || true
mount -t tmpfs tmpfs /tmp 2>/dev/null || true

# Mount virtiofs shares
mkdir -p /var/lib/minibox/images /var/lib/minibox/containers
mount -t virtiofs mbx-images /var/lib/minibox/images 2>/dev/null || true
mount -t virtiofs mbx-containers /var/lib/minibox/containers 2>/dev/null || true

# Loopback interface
ip link set lo up 2>/dev/null || true

# Hostname
hostname minibox-vm 2>/dev/null || true
"#
    .to_string()
}

/// Install init files into `rootfs_dir/etc/`.
pub fn install_init_files(rootfs_dir: &Path) -> Result<()> {
    let etc = rootfs_dir.join("etc");
    std::fs::create_dir_all(&etc).context("creating rootfs/etc")?;

    // /etc/inittab
    let inittab = etc.join("inittab");
    std::fs::write(&inittab, generate_inittab())
        .with_context(|| format!("writing {}", inittab.display()))?;

    // /etc/init.d/rcS
    let initd = etc.join("init.d");
    std::fs::create_dir_all(&initd).context("creating rootfs/etc/init.d")?;
    let rcs = initd.join("rcS");
    std::fs::write(&rcs, generate_rc_local())
        .with_context(|| format!("writing {}", rcs.display()))?;

    // Make rcS executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&rcs, perms)
            .with_context(|| format!("chmod rcS {}", rcs.display()))?;
    }

    println!("  init    {}", rcs.display());
    println!("  inittab {}", inittab.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inittab_contains_agent_entry() {
        let content = generate_inittab();
        assert!(content.contains("minibox-agent"));
        assert!(content.contains("::sysinit:"));
    }

    #[test]
    fn rc_local_mounts_proc_and_sys() {
        let content = generate_rc_local();
        assert!(content.contains("mount -t proc"));
        assert!(content.contains("mount -t sysfs"));
        assert!(content.contains("mount -t devtmpfs"));
    }

    #[test]
    fn rc_local_mounts_virtiofs_shares() {
        let content = generate_rc_local();
        assert!(content.contains("virtiofs"));
        assert!(content.contains("mbx-images"));
        assert!(content.contains("mbx-containers"));
    }
}
```

- [ ] **Step 4: Wire into xtask `build_vm_image`**

In `crates/xtask/src/vm_image.rs`, after `build_and_install_agent`, add a call to install init files. Since `agent_init.rs` is in `macbox`, duplicate the logic directly in `vm_image.rs` to avoid a circular dep (xtask should not depend on macbox):

```rust
// In crates/xtask/src/vm_image.rs

/// Install minimal init files into rootfs so the agent boots correctly.
pub fn install_init_files(rootfs_dir: &Path) -> Result<()> {
    let etc = rootfs_dir.join("etc");
    std::fs::create_dir_all(&etc).context("creating rootfs/etc")?;

    let inittab = "::sysinit:/etc/init.d/rcS\n::once:/sbin/minibox-agent\n::ctrlaltdel:/sbin/reboot\n::shutdown:/bin/umount -a -r\n";
    std::fs::write(etc.join("inittab"), inittab)
        .context("writing /etc/inittab")?;

    let initd = etc.join("init.d");
    std::fs::create_dir_all(&initd)?;
    let rcs = r#"#!/bin/sh
set -e
mount -t proc proc /proc 2>/dev/null || true
mount -t sysfs sys /sys 2>/dev/null || true
mount -t devtmpfs dev /dev 2>/dev/null || true
mount -t tmpfs tmpfs /tmp 2>/dev/null || true
mkdir -p /var/lib/minibox/images /var/lib/minibox/containers
mount -t virtiofs mbx-images /var/lib/minibox/images 2>/dev/null || true
mount -t virtiofs mbx-containers /var/lib/minibox/containers 2>/dev/null || true
ip link set lo up 2>/dev/null || true
hostname minibox-vm 2>/dev/null || true
"#;
    let rcs_path = initd.join("rcS");
    std::fs::write(&rcs_path, rcs).context("writing rcS")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&rcs_path, std::fs::Permissions::from_mode(0o755))?;
    }
    println!("  init    rootfs/etc/inittab + etc/init.d/rcS");
    Ok(())
}
```

Then call `install_init_files(&rootfs_dir)?;` in `build_vm_image()` after `build_and_install_agent`.

- [ ] **Step 5: Run tests**

```
cargo test -p macbox --features vz inittab_contains rc_local_mounts
```
Expected: all 3 PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/macbox/src/vz/agent_init.rs crates/macbox/src/vz/mod.rs crates/xtask/src/vm_image.rs
git commit -m "feat(macbox/vz): agent PID-1 init script — mounts virtiofs, execs agent"
```

---

### Task 5: Wire VZ branch into `macbox::start()`

**Files:**
- Modify: `crates/macbox/src/lib.rs`
- Modify: `crates/miniboxd/src/main.rs`

When `MINIBOX_ADAPTER=vz` is set on macOS and the `vz` feature is compiled in, `macbox::start()` boots a `VzVm` and constructs `HandlerDependencies` using the four `Vz*` adapters instead of the Colima suite.

- [ ] **Step 1: Write the failing test**

```rust
// In crates/macbox/src/lib.rs (cfg-gated)
#[cfg(all(test, feature = "vz"))]
mod vz_start_tests {
    #[test]
    fn vz_adapter_env_is_detected() {
        // Test that "vz" is recognized as a valid adapter name.
        assert!(is_vz_adapter());

        // Setting a different adapter should return false.
        // (This test is structural — it just confirms the function exists.)
    }
}
```

- [ ] **Step 2: Run to confirm it fails**

```
cargo test -p macbox --features vz vz_adapter_env
```
Expected: compile error — `is_vz_adapter` not defined.

- [ ] **Step 3: Add VZ branch to `lib.rs`**

```rust
// crates/macbox/src/lib.rs — add before pub async fn start():

#[cfg(feature = "vz")]
fn is_vz_adapter() -> bool {
    std::env::var("MINIBOX_ADAPTER")
        .map(|v| v == "vz")
        .unwrap_or(false)
}

// ... inside pub async fn start(), before the existing Colima wiring section:
#[cfg(feature = "vz")]
if is_vz_adapter() {
    return start_vz(data_dir, run_dir, socket_path, images_dir, containers_dir, run_containers_dir, state).await;
}
```

Add the VZ start function (after `start()`):

```rust
#[cfg(feature = "vz")]
async fn start_vz(
    data_dir: std::path::PathBuf,
    run_dir: std::path::PathBuf,
    socket_path: std::path::PathBuf,
    images_dir: std::path::PathBuf,
    containers_dir: std::path::PathBuf,
    run_containers_dir: std::path::PathBuf,
    state: Arc<DaemonState>,
) -> Result<()> {
    use std::sync::Arc;
    use vz::vm::{VzVm, VzVmConfig};
    use vz::{VzFilesystem, VzLimiter, VzRegistry, VzRuntime};

    let vm_dir = vz::vm::default_vm_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".mbx").join("vm"));

    info!("vz: booting Linux VM from {}", vm_dir.display());

    let config = VzVmConfig {
        vm_dir: vm_dir.clone(),
        images_dir: images_dir.clone(),
        containers_dir: containers_dir.clone(),
        memory_bytes: 1 * 1024 * 1024 * 1024, // 1 GiB
        cpu_count: 2,
    };

    // Boot in a blocking task — VZ.framework calls are synchronous.
    let vm = tokio::task::spawn_blocking(move || VzVm::boot(config))
        .await
        .context("spawn_blocking VzVm::boot")??;

    // Wait for agent to accept connections.
    info!("vz: waiting for agent on vsock port {}", vz::vsock::AGENT_PORT);
    vz::vsock::connect_to_agent(60)
        .await
        .context("vz: agent did not come up within 30s")?;
    info!("vz: agent ready");

    let vm = Arc::new(vm);

    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(VzRegistry::new(Arc::clone(&vm))),
        ghcr_registry: Arc::new(VzRegistry::new(Arc::clone(&vm))),
        filesystem: Arc::new(VzFilesystem::new(Arc::clone(&vm))),
        resource_limiter: Arc::new(VzLimiter::new(Arc::clone(&vm))),
        runtime: Arc::new(VzRuntime::new(Arc::clone(&vm))),
        network_provider: Arc::new(NoopNetwork::new()),
        containers_base: containers_dir,
        run_containers_base: run_containers_dir,
    });

    // ── Socket (same as Colima path) ─────────────────────────────────────
    if socket_path.exists() {
        warn!("removing stale socket at {}", socket_path.display());
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
    }
    let raw_listener = tokio::net::UnixListener::bind(&socket_path)
        .with_context(|| format!("binding socket at {}", socket_path.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    }
    info!("vz: listening on {}", socket_path.display());

    let shutdown = async {
        tokio::signal::ctrl_c().await.ok();
        info!("vz: received Ctrl-C, shutting down");
        vm.stop();
    };

    daemonbox::server::run_server(
        MacUnixListener(raw_listener),
        state,
        deps,
        false,
        shutdown,
    )
    .await?;

    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }
    info!("miniboxd (macOS/vz) stopped");
    Ok(())
}
```

Add `default_vm_dir` helper to `vz/vm.rs`:

```rust
pub fn default_vm_dir() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".mbx").join("vm"))
}
```

- [ ] **Step 4: Run test**

```
cargo test -p macbox --features vz vz_adapter_env
```
Expected: PASS.

- [ ] **Step 5: Check full compilation**

```bash
cargo check -p macbox --features vz
cargo check -p miniboxd
```
Expected: both compile clean.

- [ ] **Step 6: Commit**

```bash
git add crates/macbox/src/lib.rs crates/macbox/src/vz/vm.rs
git commit -m "feat(macbox): MINIBOX_ADAPTER=vz branch in start() — boots VzVm + wires adapters"
```

---

### Task 6: Integration smoke test

**Files:**
- Create: `crates/macbox/tests/vz_adapter_smoke.rs`

This test only runs on macOS when the VM image exists. It boots the VM, sends a `ListContainers` request via the proxy, and expects an empty list back.

- [ ] **Step 1: Create the smoke test file**

```rust
// crates/macbox/tests/vz_adapter_smoke.rs
//! Integration smoke test for the VZ adapter.
//!
//! Requires:
//!   - macOS + Apple Silicon (or x86 Mac with VZ.framework)
//!   - VM image at ~/.mbx/vm/ (run `cargo xtask build-vm-image` first)
//!   - `vz` feature compiled in
//!
//! Skipped automatically if VM image is absent.

#![cfg(all(target_os = "macos", feature = "vz"))]

use macbox::vz::vm::{VzVm, VzVmConfig};
use macbox::vz::vsock::connect_to_agent;
use macbox::vz::proxy::VzProxy;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

fn vm_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap().join(".mbx").join("vm")
}

fn vm_image_available() -> bool {
    let d = vm_dir();
    d.join("boot").join("vmlinuz-virt").exists()
        && d.join("rootfs").join("sbin").join("minibox-agent").exists()
}

#[tokio::test]
async fn vz_smoke_list_containers_returns_empty() {
    if !vm_image_available() {
        eprintln!("SKIP: VM image not found at ~/.mbx/vm/ — run `cargo xtask build-vm-image`");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let config = VzVmConfig {
        vm_dir: vm_dir(),
        images_dir: tmp.path().join("images"),
        containers_dir: tmp.path().join("containers"),
        memory_bytes: 512 * 1024 * 1024,
        cpu_count: 1,
    };

    std::fs::create_dir_all(&config.images_dir).unwrap();
    std::fs::create_dir_all(&config.containers_dir).unwrap();

    // Boot VM
    let vm = tokio::task::spawn_blocking(move || VzVm::boot(config))
        .await
        .unwrap()
        .expect("VM boot failed");

    // Wait for agent
    let stream = connect_to_agent(60).await.expect("agent did not come up");
    let mut proxy = VzProxy::new(stream);

    // Send List
    let responses = proxy
        .send_request(&DaemonRequest::List)
        .await
        .expect("request failed");

    assert!(!responses.is_empty(), "agent returned no responses");
    let last = responses.last().unwrap();
    assert!(
        matches!(last, DaemonResponse::ContainerList { .. }),
        "expected ContainerList, got {last:?}"
    );

    vm.stop();
}
```

Add `tempfile` to macbox dev-dependencies in `Cargo.toml`:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Verify it compiles (without running — VM not available in CI)**

```bash
cargo test -p macbox --features vz --no-run
```
Expected: compiles, test binary built.

- [ ] **Step 3: Commit**

```bash
git add crates/macbox/tests/vz_adapter_smoke.rs crates/macbox/Cargo.toml
git commit -m "test(macbox/vz): smoke test for VzAdapter — boots VM, checks ListContainers"
```

---

### Task 7: Pre-commit gate + CLAUDE.md update

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Run pre-commit gate**

```bash
cargo xtask pre-commit
```
Expected: PASS — `cargo fmt --all --check` + clippy + release build all green.

Fix any clippy warnings before committing. Common issues:
- Unused `vm` field → add `#[allow(dead_code)]` or use `_vm` naming
- `Retained::into_raw` without `from_raw` → ensure all raw pointers are eventually managed

- [ ] **Step 2: Update CLAUDE.md**

In the "Adapter Suites" section, update:

```
**Adapter Suites**: `MINIBOX_ADAPTER` env var selects between `native`, `gke`, `colima`, and `vz`.
- `vz`: macOS Apple Silicon — boots an Alpine Linux VM via Virtualization.framework, forwards commands to in-VM miniboxd over vsock. Requires `macbox` compiled with `--features vz` and VM image at `~/.mbx/vm/` (run `cargo xtask build-vm-image`).
```

In the "Current Limitations" section, remove or update:
> Adapter wiring incomplete: `vf` adapter is now wired as `vz` when `MINIBOX_ADAPTER=vz`.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md — document VZ adapter suite and build-vm-image"
```

---

## End-to-End Verification

After all tasks complete, verify the full stack:

```bash
# 1. Build VM image (if not already done)
cargo xtask build-vm-image
ls ~/.mbx/vm/manifest.json  # Should exist

# 2. Build macbox with vz feature
cargo build --release -p miniboxd --features macbox/vz

# 3. Start daemon with VZ adapter
MINIBOX_ADAPTER=vz ./target/release/miniboxd &

# 4. Send a ps command
./target/release/minibox ps
# Expected: empty container list (or error if agent not ready yet — retry)

# 5. Pull an image
./target/release/minibox pull alpine
# Expected: pulls via in-VM agent

# 6. Run a container
./target/release/minibox run alpine -- echo "hello from VM"
# Expected: prints "hello from VM" with exit 0

# 7. Verify pre-commit gate
cargo xtask pre-commit
```
