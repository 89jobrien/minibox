# Container Networking Phase 1: NetworkMode + None/Host Adapters

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the `NetworkProvider` trait into the container lifecycle with `NetworkMode` dispatch, and implement `None` (loopback-only) and `Host` (shared namespace) adapters.

**Architecture:** Extend the existing `NetworkConfig` with a `NetworkMode` enum. Add `DynNetworkProvider` to `HandlerDependencies`. Create a `NoopNetwork` adapter (current behavior, explicit) and a `HostNetwork` adapter (skips `CLONE_NEWNET`). Wire `network_provider.setup()`/`attach()`/`cleanup()` into the container lifecycle in `handler.rs`. Add `--network` flag to CLI and `network` field to protocol.

**Tech Stack:** Rust, async-trait, serde, clap, tokio, nix

**Spec:** `docs/superpowers/specs/2026-03-23-container-networking-design.md`

---

### Task 1: Add `NetworkMode` enum to domain

**Files:**

- Modify: `crates/minibox/src/domain/networking.rs:1-178`

- [ ] **Step 1: Write the test for NetworkMode serde round-trip**

Add to the bottom of `networking.rs` (new test module):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_mode_serde_roundtrip() {
        for mode in [NetworkMode::None, NetworkMode::Bridge, NetworkMode::Host, NetworkMode::Tailnet] {
            let json = serde_json::to_string(&mode).expect("serialize");
            let back: NetworkMode = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(mode, back);
        }
    }

    #[test]
    fn network_mode_default_is_none() {
        assert_eq!(NetworkMode::default(), NetworkMode::None);
    }

    #[test]
    fn network_config_default_has_none_mode() {
        let config = NetworkConfig::default();
        assert_eq!(config.mode, NetworkMode::None);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox domain::networking::tests --no-run 2>&1 | head -20`
Expected: compile error — `NetworkMode` not defined

- [ ] **Step 3: Add `NetworkMode` enum and update `NetworkConfig`**

Add `NetworkMode` enum before the `NetworkProvider` trait:

```rust
/// Network mode for a container.
///
/// Determines which networking adapter handles container network setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NetworkMode {
    /// No networking (loopback only). Current default behavior.
    #[default]
    None,
    /// Linux bridge with veth pair and iptables NAT.
    Bridge,
    /// Share host network namespace (skip CLONE_NEWNET).
    Host,
    /// WireGuard mesh via Headscale/Tailscale.
    Tailnet,
}
```

Add `pub mode: NetworkMode` as the first field in `NetworkConfig`, and update `Default` impl to set `mode: NetworkMode::None`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p minibox domain::networking::tests -- --nocapture`
Expected: 3 tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/src/domain/networking.rs
git commit -m "feat(domain): add NetworkMode enum to NetworkConfig"
```

---

### Task 2: Add `DynNetworkProvider` type alias and mock

**Files:**

- Modify: `crates/minibox/src/domain.rs:72-84`
- Modify: `crates/minibox/src/adapters/mocks.rs`

- [ ] **Step 1: Write test for MockNetwork**

Add to `crates/minibox/src/adapters/mocks.rs` test module:

```rust
#[tokio::test]
async fn test_mock_network_setup() {
    let net = MockNetwork::new();
    assert_eq!(net.setup_count(), 0);
    let result = net.setup("container-1", &NetworkConfig::default()).await;
    assert!(result.is_ok());
    assert_eq!(net.setup_count(), 1);
}

#[tokio::test]
async fn test_mock_network_cleanup() {
    let net = MockNetwork::new();
    let result = net.cleanup("container-1").await;
    assert!(result.is_ok());
    assert_eq!(net.cleanup_count(), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox mocks::tests::test_mock_network --no-run 2>&1 | head -20`
Expected: compile error — `MockNetwork` not defined

- [ ] **Step 3: Add `DynNetworkProvider` alias to `domain.rs`**

After line 83 in `domain.rs`, add:

```rust
/// Type alias for a shared, dynamic [`NetworkProvider`] implementation.
pub type DynNetworkProvider = Arc<dyn NetworkProvider>;
```

- [ ] **Step 4: Implement `MockNetwork` in `mocks.rs`**

Add after the `MockRuntime` section:

```rust
// ---------------------------------------------------------------------------
// MockNetwork
// ---------------------------------------------------------------------------

/// Mock implementation of [`NetworkProvider`] for testing.
///
/// Simulates network operations without any real network setup. Tracks
/// setup/attach/cleanup call counts and can be configured to fail on demand.
#[derive(Debug, Clone)]
pub struct MockNetwork {
    state: Arc<Mutex<MockNetworkState>>,
}

#[derive(Debug)]
struct MockNetworkState {
    setup_should_succeed: bool,
    attach_should_succeed: bool,
    cleanup_should_succeed: bool,
    setup_count: usize,
    attach_count: usize,
    cleanup_count: usize,
}

impl MockNetwork {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockNetworkState {
                setup_should_succeed: true,
                attach_should_succeed: true,
                cleanup_should_succeed: true,
                setup_count: 0,
                attach_count: 0,
                cleanup_count: 0,
            })),
        }
    }

    pub fn with_setup_failure(self) -> Self {
        self.state.lock().unwrap().setup_should_succeed = false;
        self
    }

    pub fn setup_count(&self) -> usize {
        self.state.lock().unwrap().setup_count
    }

    pub fn cleanup_count(&self) -> usize {
        self.state.lock().unwrap().cleanup_count
    }
}

#[async_trait]
impl NetworkProvider for MockNetwork {
    async fn setup(&self, _container_id: &str, _config: &NetworkConfig) -> Result<String> {
        let mut state = self.state.lock().unwrap();
        state.setup_count += 1;
        if !state.setup_should_succeed {
            anyhow::bail!("mock network setup failure");
        }
        Ok("/mock/netns".to_string())
    }

    async fn attach(&self, _container_id: &str, _pid: u32) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.attach_count += 1;
        if !state.attach_should_succeed {
            anyhow::bail!("mock network attach failure");
        }
        Ok(())
    }

    async fn cleanup(&self, _container_id: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.cleanup_count += 1;
        if !state.cleanup_should_succeed {
            anyhow::bail!("mock network cleanup failure");
        }
        Ok(())
    }

    async fn stats(&self, _container_id: &str) -> Result<NetworkStats> {
        Ok(NetworkStats::default())
    }
}
```

Update the `adapt!` macro call to include `MockNetwork`:

```rust
adapt!(MockRegistry, MockFilesystem, MockLimiter, MockRuntime, MockNetwork);
```

Add `NetworkConfig` and `NetworkProvider` to the imports at top of mocks.rs.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p minibox mocks::tests::test_mock_network -- --nocapture`
Expected: 2 tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/minibox/src/domain.rs crates/minibox/src/adapters/mocks.rs
git commit -m "feat(domain): add DynNetworkProvider alias and MockNetwork adapter"
```

---

### Task 3: Implement `NoopNetwork` adapter

**Files:**

- Create: `crates/minibox/src/adapters/network/mod.rs`
- Create: `crates/minibox/src/adapters/network/none.rs`
- Modify: `crates/minibox/src/adapters/mod.rs`

- [ ] **Step 1: Write tests for NoopNetwork**

Create `crates/minibox/src/adapters/network/none.rs` with tests at bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_setup_returns_empty_path() {
        let net = NoopNetwork;
        let result = net.setup("test-container", &NetworkConfig::default()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn noop_attach_succeeds() {
        let net = NoopNetwork;
        assert!(net.attach("test-container", 1234).await.is_ok());
    }

    #[tokio::test]
    async fn noop_cleanup_succeeds() {
        let net = NoopNetwork;
        assert!(net.cleanup("test-container").await.is_ok());
    }

    #[tokio::test]
    async fn noop_stats_returns_zeroes() {
        let net = NoopNetwork;
        let stats = net.stats("test-container").await.unwrap();
        assert_eq!(stats.rx_bytes, 0);
        assert_eq!(stats.tx_bytes, 0);
    }
}
```

- [ ] **Step 2: Implement NoopNetwork**

Write the implementation above the tests in `none.rs`:

```rust
//! No-op network adapter — loopback only.
//!
//! Used when `NetworkMode::None` is selected. The container gets an isolated
//! network namespace with only the loopback interface. This is the current
//! default behavior.

use crate::adapt;
use crate::domain::{NetworkConfig, NetworkProvider, NetworkStats};
use anyhow::Result;
use async_trait::async_trait;

/// Network adapter that does nothing — container has loopback only.
pub struct NoopNetwork;

impl NoopNetwork {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NetworkProvider for NoopNetwork {
    async fn setup(&self, _container_id: &str, _config: &NetworkConfig) -> Result<String> {
        Ok(String::new())
    }

    async fn attach(&self, _container_id: &str, _pid: u32) -> Result<()> {
        Ok(())
    }

    async fn cleanup(&self, _container_id: &str) -> Result<()> {
        Ok(())
    }

    async fn stats(&self, _container_id: &str) -> Result<NetworkStats> {
        Ok(NetworkStats::default())
    }
}

adapt!(NoopNetwork);
```

- [ ] **Step 3: Create `network/mod.rs`**

```rust
//! Network adapter implementations.
//!
//! Each module implements [`NetworkProvider`] for a different [`NetworkMode`].

pub mod none;

pub use none::NoopNetwork;
```

- [ ] **Step 4: Wire into `adapters/mod.rs`**

Add `pub mod network;` to the module declarations and re-export `NoopNetwork`:

```rust
pub mod network;
pub use network::NoopNetwork;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p minibox adapters::network::none::tests -- --nocapture`
Expected: 4 tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/minibox/src/adapters/network/ crates/minibox/src/adapters/mod.rs
git commit -m "feat(adapters): add NoopNetwork adapter for NetworkMode::None"
```

---

### Task 4: Implement `HostNetwork` adapter

**Files:**

- Create: `crates/minibox/src/adapters/network/host.rs`
- Modify: `crates/minibox/src/adapters/network/mod.rs`
- Modify: `crates/minibox/src/adapters/mod.rs`

- [ ] **Step 1: Write tests for HostNetwork**

Create `crates/minibox/src/adapters/network/host.rs` with tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn host_setup_returns_host_marker() {
        let net = HostNetwork;
        let result = net.setup("test-container", &NetworkConfig::default()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "host");
    }

    #[tokio::test]
    async fn host_attach_succeeds() {
        let net = HostNetwork;
        assert!(net.attach("test-container", 1234).await.is_ok());
    }

    #[tokio::test]
    async fn host_cleanup_succeeds() {
        let net = HostNetwork;
        assert!(net.cleanup("test-container").await.is_ok());
    }
}
```

- [ ] **Step 2: Implement HostNetwork**

Write above the tests in `host.rs`:

```rust
//! Host network adapter — container shares host network namespace.
//!
//! Used when `NetworkMode::Host` is selected. The container skips
//! `CLONE_NEWNET` and shares the host's network stack. No veth pair
//! or bridge is created.

use crate::adapt;
use crate::domain::{NetworkConfig, NetworkProvider, NetworkStats};
use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

/// Network adapter that shares the host network namespace.
pub struct HostNetwork;

impl HostNetwork {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NetworkProvider for HostNetwork {
    async fn setup(&self, container_id: &str, _config: &NetworkConfig) -> Result<String> {
        info!(
            container_id = container_id,
            "network: using host networking (skip CLONE_NEWNET)"
        );
        Ok("host".to_string())
    }

    async fn attach(&self, _container_id: &str, _pid: u32) -> Result<()> {
        // No-op: container already shares host network namespace.
        Ok(())
    }

    async fn cleanup(&self, _container_id: &str) -> Result<()> {
        // No-op: nothing to clean up for host networking.
        Ok(())
    }

    async fn stats(&self, _container_id: &str) -> Result<NetworkStats> {
        // Host mode: stats would come from host interfaces, not meaningful per-container.
        Ok(NetworkStats::default())
    }
}

adapt!(HostNetwork);
```

- [ ] **Step 3: Wire into `network/mod.rs` and `adapters/mod.rs`**

Update `network/mod.rs`:

```rust
pub mod host;
pub mod none;

pub use host::HostNetwork;
pub use none::NoopNetwork;
```

Update `adapters/mod.rs` to re-export `HostNetwork`:

```rust
pub use network::{HostNetwork, NoopNetwork};
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p minibox adapters::network -- --nocapture`
Expected: 7 tests pass (4 noop + 3 host)

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/src/adapters/network/ crates/minibox/src/adapters/mod.rs
git commit -m "feat(adapters): add HostNetwork adapter for NetworkMode::Host"
```

---

### Task 5: Add `network` field to protocol

**Files:**

- Modify: `crates/minibox/src/protocol.rs:49-91`

- [ ] **Step 1: Write test for protocol round-trip with network field**

Add to the existing test module in `protocol.rs`:

```rust
#[test]
fn run_request_with_network_mode_roundtrip() {
    use crate::domain::{NetworkConfig, NetworkMode};

    let req = DaemonRequest::Run {
        image: "alpine".to_string(),
        tag: None,
        command: vec!["/bin/sh".to_string()],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: false,
        network: Some(NetworkMode::Host),
    };

    let encoded = encode_request(&req).expect("encode");
    let decoded = decode_request(&encoded).expect("decode");
    match decoded {
        DaemonRequest::Run { network, .. } => {
            assert_eq!(network, Some(NetworkMode::Host));
        }
        _ => panic!("wrong request type"),
    }
}

#[test]
fn run_request_without_network_defaults_to_none() {
    let json = r#"{"type":"Run","image":"alpine","command":["sh"],"memory_limit_bytes":null,"cpu_weight":null}"#;
    let req: DaemonRequest = serde_json::from_str(json).expect("parse");
    match req {
        DaemonRequest::Run { network, .. } => {
            assert_eq!(network, None);
        }
        _ => panic!("expected Run"),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p minibox protocol::tests::run_request_with_network --no-run 2>&1 | head -20`
Expected: compile error — no `network` field in `Run`

- [ ] **Step 3: Add `network` field to `DaemonRequest::Run`**

Add to the `Run` variant in `DaemonRequest`:

```rust
/// Network mode for the container. Defaults to `None` (loopback only)
/// when absent for backwards compatibility.
#[serde(default)]
network: Option<NetworkMode>,
```

Add `use crate::domain::NetworkMode;` to the imports at the top of `protocol.rs`.

- [ ] **Step 4: Fix all existing code that constructs/destructures `DaemonRequest::Run`**

Update all match arms and constructors across the codebase to include `network`:

- `crates/daemonbox/src/server.rs` — `dispatch()` function destructure
- `crates/daemonbox/src/handler.rs` — `handle_run()` signature and callers
- `crates/minibox-cli/src/commands/run.rs` — `execute()` request construction
- All existing tests in `protocol.rs` that construct `DaemonRequest::Run`

For all existing `Run` constructions, add `network: None`.

- [ ] **Step 5: Run full test suite**

Run: `cargo test -p minibox -p minibox-cli -p daemonbox -- --nocapture`
Expected: all tests pass including new network round-trip tests

- [ ] **Step 6: Commit**

```bash
git add crates/minibox/src/protocol.rs crates/daemonbox/src/server.rs crates/daemonbox/src/handler.rs crates/minibox-cli/src/commands/run.rs
git commit -m "feat(protocol): add network field to DaemonRequest::Run"
```

---

### Task 6: Add `network_provider` to `HandlerDependencies`

**Files:**

- Modify: `crates/daemonbox/src/handler.rs:56-70`
- Modify: `crates/miniboxd/src/main.rs:274-313`

- [ ] **Step 1: Write test that verifies HandlerDependencies includes network_provider**

The existing handler tests in `crates/daemonbox/tests/` (if any) or proptest fixtures should be updated. For now, verify compilation:

```bash
cargo check -p daemonbox
```

- [ ] **Step 2: Add `network_provider` to `HandlerDependencies`**

In `handler.rs`, add to the struct:

```rust
/// Network provider for container network setup/teardown.
pub network_provider: DynNetworkProvider,
```

Add to imports:

```rust
use minibox::domain::DynNetworkProvider;
```

- [ ] **Step 3: Update all `HandlerDependencies` construction sites**

In `miniboxd/src/main.rs`, add `network_provider` to each `HandlerDependencies` construction. All three adapter suites (Native, Gke, Colima) get `NoopNetwork` for now:

```rust
use minibox::adapters::NoopNetwork;

// In each HandlerDependencies { ... }:
network_provider: Arc::new(NoopNetwork::new()),
```

Also update the `macbox` crate if it constructs `HandlerDependencies`.

- [ ] **Step 4: Update any test fixtures that construct `HandlerDependencies`**

Search for `HandlerDependencies {` in test files and add `network_provider: Arc::new(MockNetwork::new())`.

- [ ] **Step 5: Run check to verify compilation**

Run: `cargo check --workspace`
Expected: compiles successfully

- [ ] **Step 6: Run all tests**

Run: `cargo xtask test-unit`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add crates/daemonbox/src/handler.rs crates/miniboxd/src/main.rs
git commit -m "feat(handler): add network_provider to HandlerDependencies"
```

---

### Task 7: Wire network lifecycle into handler

**Files:**

- Modify: `crates/daemonbox/src/handler.rs` — `run_inner_capture()` and `run_inner()`

- [ ] **Step 1: Write test for network setup/cleanup in handler**

Add to `crates/daemonbox/tests/` or inline in handler.rs if tests exist there. The key assertion: when a container is run, `network_provider.setup()` is called, and on stop/remove, `cleanup()` is called.

This is best verified via the mock's call counts in an integration-style test, but for now we verify the wiring compiles and the mock gets called correctly in the existing proptest/handler tests.

- [ ] **Step 2: Add network setup to `run_inner_capture()`**

After the cgroup is created (line ~322) and before the spawn config is built:

```rust
// ── Network setup ──────────────────────────────────────────────────
let network_mode = network.unwrap_or(NetworkMode::None);
let net_config = NetworkConfig {
    mode: network_mode,
    ..NetworkConfig::default()
};
let _net_ns = deps
    .network_provider
    .setup(&id, &net_config)
    .await
    .context("network setup")?;
```

Pass `network` through `run_inner_capture()` and `handle_run_streaming()` parameter lists.

- [ ] **Step 3: Add network attach after spawn**

After `spawn_process` returns with a PID:

```rust
deps.network_provider
    .attach(&id, pid)
    .await
    .context("network attach")?;
```

- [ ] **Step 4: Add network cleanup to stop/remove paths**

In `handle_remove()` and `handle_stop()`, call:

```rust
if let Err(e) = deps.network_provider.cleanup(&id).await {
    warn!(container_id = %id, error = %e, "network: cleanup failed");
}
```

For ephemeral containers in `handle_run_streaming()`, add cleanup before `state.remove_container()`:

```rust
if let Err(e) = deps.network_provider.cleanup(&container_id).await {
    warn!(container_id = %container_id, error = %e, "network: cleanup failed");
}
```

- [ ] **Step 5: Apply the same pattern to `run_inner()`**

Mirror the setup/attach/cleanup calls in the non-streaming `run_inner()` path.

- [ ] **Step 6: Handle Host mode — skip CLONE_NEWNET**

The `ContainerSpawnConfig` or the `NamespaceConfig` needs a way to skip `CLONE_NEWNET` when `NetworkMode::Host` is selected. Check how `LinuxNamespaceRuntime` builds its clone flags and add a field to `ContainerSpawnConfig`:

```rust
/// If true, skip CLONE_NEWNET (host networking mode).
pub skip_network_namespace: bool,
```

Set this to `true` when `network_mode == NetworkMode::Host`. Update `LinuxNamespaceRuntime::spawn_process()` to check this flag and omit `CLONE_NEWNET` from the clone flags.

- [ ] **Step 7: Run full test suite**

Run: `cargo xtask test-unit`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add crates/daemonbox/src/handler.rs crates/minibox/src/domain.rs crates/minibox/src/container/
git commit -m "feat(handler): wire network lifecycle (setup/attach/cleanup) into container run"
```

---

### Task 8: Add `--network` CLI flag

**Files:**

- Modify: `crates/minibox-cli/src/main.rs:52-71`
- Modify: `crates/minibox-cli/src/commands/run.rs:35-48`

- [ ] **Step 1: Write test for CLI network flag parsing**

Add to `crates/minibox-cli/src/main.rs` or a test module:

```rust
#[test]
fn cli_parses_network_none() {
    let cli = Cli::try_parse_from(["minibox", "run", "--network", "none", "alpine", "--", "/bin/sh"]);
    assert!(cli.is_ok());
}

#[test]
fn cli_parses_network_host() {
    let cli = Cli::try_parse_from(["minibox", "run", "--network", "host", "alpine", "--", "/bin/sh"]);
    assert!(cli.is_ok());
}

#[test]
fn cli_default_network_is_none() {
    let cli = Cli::try_parse_from(["minibox", "run", "alpine", "--", "/bin/sh"]).unwrap();
    match cli.command {
        Commands::Run { network, .. } => assert_eq!(network, "none"),
        _ => panic!("expected Run"),
    }
}
```

- [ ] **Step 2: Add `--network` flag to `Commands::Run`**

```rust
/// Network mode: none (default), bridge, host, tailnet
#[arg(long, default_value = "none")]
network: String,
```

- [ ] **Step 3: Update `execute()` to pass network mode**

In `commands/run.rs`, parse the string to `NetworkMode` and add to the request:

```rust
use minibox::domain::NetworkMode;

let network_mode = match network.as_str() {
    "none" => NetworkMode::None,
    "bridge" => NetworkMode::Bridge,
    "host" => NetworkMode::Host,
    "tailnet" => NetworkMode::Tailnet,
    other => anyhow::bail!("unknown network mode: {other} (expected: none, bridge, host, tailnet)"),
};
```

Add `network: Some(network_mode)` to the `DaemonRequest::Run` construction.

Update `execute()` signature to accept `network: String`.

- [ ] **Step 4: Update `main.rs` match arm to pass network through**

```rust
Commands::Run {
    image,
    command,
    memory,
    cpu_weight,
    tag,
    network,
} => commands::run::execute(image, tag, command, memory, cpu_weight, network).await,
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p minibox-cli -- --nocapture`
Expected: all tests pass including new CLI parsing tests

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-cli/src/main.rs crates/minibox-cli/src/commands/run.rs
git commit -m "feat(cli): add --network flag to minibox run"
```

---

### Task 9: Quality gates

**Files:** None (verification only)

- [ ] **Step 1: Run fmt check**

Run: `cargo fmt --all --check`
Expected: no formatting issues

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -p minibox -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -p minibox-llm -p minibox-secrets -- -D warnings`
Expected: no warnings

- [ ] **Step 3: Run full unit test suite**

Run: `cargo xtask test-unit`
Expected: all tests pass

- [ ] **Step 4: Fix any issues found in steps 1-3**

Address any lint, format, or test failures.

- [ ] **Step 5: Final commit if fixes were needed**

```bash
git add -A
git commit -m "chore: fix lint and formatting for networking phase 1"
```

---

## File Map

| Action | Path                                          | Purpose                                        |
| ------ | --------------------------------------------- | ---------------------------------------------- |
| Modify | `crates/minibox/src/domain/networking.rs`     | Add `NetworkMode` enum, update `NetworkConfig` |
| Modify | `crates/minibox/src/domain.rs`                | Add `DynNetworkProvider` type alias            |
| Create | `crates/minibox/src/adapters/network/mod.rs`  | Network adapter module dispatch                |
| Create | `crates/minibox/src/adapters/network/none.rs` | `NoopNetwork` adapter                          |
| Create | `crates/minibox/src/adapters/network/host.rs` | `HostNetwork` adapter                          |
| Modify | `crates/minibox/src/adapters/mod.rs`          | Re-export network adapters                     |
| Modify | `crates/minibox/src/adapters/mocks.rs`        | Add `MockNetwork`                              |
| Modify | `crates/minibox/src/protocol.rs`              | Add `network` field to `Run` request           |
| Modify | `crates/daemonbox/src/handler.rs`             | Add `network_provider` to deps, wire lifecycle |
| Modify | `crates/miniboxd/src/main.rs`                 | Inject `NoopNetwork` as default provider       |
| Modify | `crates/minibox-cli/src/main.rs`              | Add `--network` CLI flag                       |
| Modify | `crates/minibox-cli/src/commands/run.rs`      | Parse and send network mode                    |
| Modify | `crates/minibox/src/domain.rs`                | Add `skip_network_namespace` to spawn config   |
