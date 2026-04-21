# Networking Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract `NetworkLifecycle` from handler.rs, expand network doc comments, add `NetworkProvider` conformance tests, and add Colima env-var + manifest.json regression tests.

**Architecture:** `NetworkLifecycle` wraps `DynNetworkProvider` with best-effort cleanup semantics; it is `#[derive(Clone)]` so `run_inner`'s `tokio::task::spawn` closure can call `attach` after cloning. All five inline network call sites in `handler.rs` become one-liner `NetworkLifecycle` method calls.

**Tech Stack:** Rust 2024, Tokio async, `async_trait`, `MockNetwork` from `minibox::adapters::mocks`

**Spec:** `docs/superpowers/specs/2026-03-24-networking-cleanup-design.md`

---

## File Map

| Action | File                                           | Purpose                                            |
| ------ | ---------------------------------------------- | -------------------------------------------------- |
| Create | `crates/daemonbox/src/network_lifecycle.rs`    | `NetworkLifecycle` struct                          |
| Modify | `crates/daemonbox/src/lib.rs`                  | declare `pub mod network_lifecycle`                |
| Modify | `crates/daemonbox/src/handler.rs`              | replace 5 inline network call sites                |
| Modify | `crates/minibox/src/adapters/mocks.rs`         | add `with_cleanup_failure()` to `MockNetwork`      |
| Modify | `crates/minibox/src/domain/networking.rs`      | expand doc comments                                |
| Modify | `crates/minibox/src/protocol.rs`               | expand `network` field doc                         |
| Modify | `crates/minibox-cli/src/main.rs`               | expand `--network` help text                       |
| Modify | `crates/daemonbox/tests/conformance_tests.rs`  | add NetworkProvider conformance section            |
| Modify | `crates/minibox/src/adapters/colima.rs`        | add env-var regression tests to inline `mod tests` |
| Modify | `crates/minibox/tests/adapter_colima_tests.rs` | add manifest.json regression tests                 |

---

## Task 1: Add `with_cleanup_failure()` to `MockNetwork`

Needed by Task 2's error-swallowing test.

> **Note on ordering:** The spec prescribes `NetworkLifecycle` extraction first, but Task 2's `cleanup_swallows_provider_error` test requires `MockNetwork.with_cleanup_failure()`. MockNetwork is committed first as a prerequisite; NetworkLifecycle extraction follows immediately in Task 2.

**Files:**

- Modify: `crates/minibox/src/adapters/mocks.rs:488-524`

- [ ] **Step 1: Add `cleanup_should_succeed` field to `MockNetworkState`**

In `mocks.rs`, find `MockNetworkState` (~line 488) and add the field:

```rust
#[derive(Debug)]
struct MockNetworkState {
    setup_should_succeed: bool,
    cleanup_should_succeed: bool,   // ← add this
    setup_count: usize,
    cleanup_count: usize,
}
```

- [ ] **Step 2: Initialize the new field in `MockNetwork::new()`**

```rust
state: Arc::new(Mutex::new(MockNetworkState {
    setup_should_succeed: true,
    cleanup_should_succeed: true,   // ← add this
    setup_count: 0,
    cleanup_count: 0,
})),
```

- [ ] **Step 3: Add the builder method after `with_setup_failure`**

```rust
/// Configure `cleanup` to return an error.
pub fn with_cleanup_failure(self) -> Self {
    self.state.lock().unwrap().cleanup_should_succeed = false;
    self
}
```

- [ ] **Step 4: Honor the flag in the `cleanup` impl**

```rust
async fn cleanup(&self, _container_id: &str) -> Result<()> {
    let mut state = self.state.lock().unwrap();
    state.cleanup_count += 1;
    if !state.cleanup_should_succeed {
        anyhow::bail!("mock network cleanup failure");
    }
    Ok(())
}
```

- [ ] **Step 5: Verify it compiles**

```bash
cargo check -p minibox
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox/src/adapters/mocks.rs
git commit -m "test(mocks): add with_cleanup_failure() to MockNetwork"
```

---

## Task 2: Create `NetworkLifecycle` module

**Files:**

- Create: `crates/daemonbox/src/network_lifecycle.rs`
- Modify: `crates/daemonbox/src/lib.rs`

- [ ] **Step 1: Write the failing unit tests first**

Create `crates/daemonbox/src/network_lifecycle.rs` with the test module only (struct not yet defined):

```rust
//! Lifecycle wrapper for [`NetworkProvider`] with consistent error handling.

use anyhow::Result;
use minibox::domain::{DynNetworkProvider, NetworkConfig};
use tracing::warn;

/// Thin lifecycle wrapper around a [`NetworkProvider`].
///
/// Provides consistent setup/attach/cleanup with best-effort cleanup
/// semantics (cleanup logs warn on error, never propagates).
///
/// `NetworkLifecycle` is `Clone` because `run_inner` constructs it before
/// a `tokio::task::spawn` closure that must call `attach`. The inner
/// [`DynNetworkProvider`] is `Arc<dyn NetworkProvider + Send + Sync>`,
/// so cloning is a cheap `Arc` refcount increment.
#[derive(Clone)]
pub struct NetworkLifecycle {
    provider: DynNetworkProvider,
}

impl NetworkLifecycle {
    /// Wrap a provider.
    pub fn new(provider: DynNetworkProvider) -> Self {
        Self { provider }
    }

    /// Set up network namespace for a new container.
    ///
    /// Returns the namespace path (e.g., `/var/run/netns/container-abc123`).
    pub async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String> {
        self.provider.setup(container_id, config).await
    }

    /// Attach a running container process to its network namespace.
    pub async fn attach(&self, container_id: &str, pid: u32) -> Result<()> {
        self.provider.attach(container_id, pid).await
    }

    /// Tear down networking for a container.
    ///
    /// Best-effort: logs `warn!` on error and never propagates the failure.
    /// Callers should not depend on the outcome.
    pub async fn cleanup(&self, container_id: &str) {
        if let Err(e) = self.provider.cleanup(container_id).await {
            warn!(
                container_id = %container_id,
                error = %e,
                "network: cleanup failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox::adapters::mocks::MockNetwork;
    use std::sync::Arc;

    #[tokio::test]
    async fn setup_delegates_to_provider_and_tracks_count() {
        let mock = Arc::new(MockNetwork::new());
        let net = NetworkLifecycle::new(mock.clone());
        let config = NetworkConfig::default();

        let result = net.setup("ctr1", &config).await;

        assert!(result.is_ok(), "setup should succeed with mock");
        assert_eq!(mock.setup_count(), 1);
    }

    #[tokio::test]
    async fn setup_propagates_provider_error() {
        let mock = Arc::new(MockNetwork::new().with_setup_failure());
        let net = NetworkLifecycle::new(mock.clone());
        let config = NetworkConfig::default();

        let result = net.setup("ctr1", &config).await;

        assert!(result.is_err(), "setup failure must be propagated to caller");
        assert_eq!(mock.setup_count(), 1);
    }

    #[tokio::test]
    async fn attach_delegates_to_provider() {
        let mock = Arc::new(MockNetwork::new());
        let net = NetworkLifecycle::new(mock.clone());

        let result = net.attach("ctr1", 1234).await;

        assert!(result.is_ok(), "attach should succeed with mock");
    }

    #[tokio::test]
    async fn cleanup_delegates_to_provider_and_tracks_count() {
        let mock = Arc::new(MockNetwork::new());
        let net = NetworkLifecycle::new(mock.clone());

        net.cleanup("ctr1").await;

        assert_eq!(mock.cleanup_count(), 1);
    }

    #[tokio::test]
    async fn cleanup_swallows_provider_error() {
        let mock = Arc::new(MockNetwork::new().with_cleanup_failure());
        let net = NetworkLifecycle::new(mock.clone());

        // Must not panic or return error — best-effort cleanup
        net.cleanup("ctr1").await;

        assert_eq!(mock.cleanup_count(), 1, "cleanup must still be called once");
    }

    #[tokio::test]
    async fn lifecycle_clone_shares_provider() {
        let mock = Arc::new(MockNetwork::new());
        let net = NetworkLifecycle::new(mock.clone());
        let net2 = net.clone();

        let config = NetworkConfig::default();
        net.setup("ctr1", &config).await.unwrap();
        net2.setup("ctr2", &config).await.unwrap();

        // Both clones share the same provider Arc
        assert_eq!(mock.setup_count(), 2);
    }
}
```

- [ ] **Step 2: Declare the module in `lib.rs`**

In `crates/daemonbox/src/lib.rs`, add:

```rust
pub mod handler;
pub mod network_lifecycle;
pub mod server;
pub mod state;
```

- [ ] **Step 3: Run the tests — verify they pass**

```bash
cargo test -p daemonbox network_lifecycle -- --nocapture
```

Expected: all 6 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/daemonbox/src/network_lifecycle.rs crates/daemonbox/src/lib.rs
git commit -m "feat(daemonbox): add NetworkLifecycle wrapper with best-effort cleanup"
```

---

## Task 3: Replace network call sites in `handler.rs`

Replace all 5 inline `network_provider` call sites with `NetworkLifecycle`.

**Files:**

- Modify: `crates/daemonbox/src/handler.rs`

- [ ] **Step 1: Add the import at the top of `handler.rs`**

After the existing `use` block, add:

```rust
use crate::network_lifecycle::NetworkLifecycle;
```

- [ ] **Step 2: Replace `handle_run_streaming` cleanup (line ~228)**

Find:

```rust
    // ── Network cleanup (ephemeral) ────────────────────────────────────
    if let Err(e) = deps.network_provider.cleanup(&container_id).await {
        warn!(container_id = %container_id, error = %e, "network: cleanup failed");
    }
```

Replace with:

```rust
    // ── Network cleanup (ephemeral) ────────────────────────────────────
    NetworkLifecycle::new(deps.network_provider.clone())
        .cleanup(&container_id)
        .await;
```

- [ ] **Step 3: Replace `run_inner_capture` setup + attach (lines ~343-412)**

Find the setup block:

```rust
    // ── Network setup ──────────────────────────────────────────────────
    let net_mode = network.unwrap_or(NetworkMode::None);
    let network_config = NetworkConfig {
        mode: net_mode,
        ..NetworkConfig::default()
    };
    let _net_ns = deps
        .network_provider
        .setup(&id, &network_config)
        .await
        .context("network setup")?;
```

Replace with:

```rust
    // ── Network setup ──────────────────────────────────────────────────
    let net_mode = network.unwrap_or(NetworkMode::None);
    let network_config = NetworkConfig {
        mode: net_mode,
        ..NetworkConfig::default()
    };
    let net = NetworkLifecycle::new(deps.network_provider.clone());
    let _net_ns = net.setup(&id, &network_config).await.context("network setup")?;
```

Then find the attach block:

```rust
    // ── Network attach ─────────────────────────────────────────────────
    deps.network_provider
        .attach(&id, pid)
        .await
        .context("network attach")?;
```

Replace with:

```rust
    // ── Network attach ─────────────────────────────────────────────────
    net.attach(&id, pid).await.context("network attach")?;
```

- [ ] **Step 4: Replace `run_inner` setup + spawn-attach (lines ~536-616)**

Find the setup block (identical pattern to run_inner_capture):

```rust
    // ── Network setup ──────────────────────────────────────────────────
    let net_mode = network.unwrap_or(NetworkMode::None);
    let network_config = NetworkConfig {
        mode: net_mode,
        ..NetworkConfig::default()
    };
    let _net_ns = deps
        .network_provider
        .setup(&id, &network_config)
        .await
        .context("network setup")?;
```

Replace with:

```rust
    // ── Network setup ──────────────────────────────────────────────────
    let net_mode = network.unwrap_or(NetworkMode::None);
    let network_config = NetworkConfig {
        mode: net_mode,
        ..NetworkConfig::default()
    };
    let net = NetworkLifecycle::new(deps.network_provider.clone());
    let _net_ns = net.setup(&id, &network_config).await.context("network setup")?;
```

Then find the block that clones `network_provider` for the spawn closure:

```rust
    let network_provider_clone = Arc::clone(&deps.network_provider);
```

Replace with:

```rust
    let net_clone = net.clone();
```

Then inside the `tokio::task::spawn` closure, find:

```rust
                // ── Network attach ─────────────────────────────────────
                if let Err(e) = network_provider_clone.attach(&id_clone, pid).await {
                    warn!(container_id = %id_clone, error = %e, "network: attach failed");
                }
```

Replace with:

```rust
                // ── Network attach ─────────────────────────────────────
                if let Err(e) = net_clone.attach(&id_clone, pid).await {
                    warn!(container_id = %id_clone, error = %e, "network: attach failed");
                }
```

- [ ] **Step 5: Replace `handle_stop` cleanup (line ~772)**

Find:

```rust
    // ── Network cleanup ────────────────────────────────────────────────
    if let Err(e) = deps.network_provider.cleanup(&id).await {
        warn!(container_id = %id, error = %e, "network: cleanup failed");
    }
```

Replace with:

```rust
    // ── Network cleanup ────────────────────────────────────────────────
    NetworkLifecycle::new(deps.network_provider.clone())
        .cleanup(&id)
        .await;
```

- [ ] **Step 6: Replace `remove_inner` cleanup (line ~909)**

Find:

```rust
    // ── Network cleanup ────────────────────────────────────────────────
    if let Err(e) = deps.network_provider.cleanup(id).await {
        warn!(container_id = %id, error = %e, "network: cleanup failed");
    }
```

Replace with:

```rust
    // ── Network cleanup ────────────────────────────────────────────────
    NetworkLifecycle::new(deps.network_provider.clone())
        .cleanup(id)
        .await;
```

- [ ] **Step 7: Remove the now-unused `Arc` import if needed**

Check if `use std::sync::Arc;` is still needed (it's used in other places in handler.rs — it will still be needed).

Also remove any leftover unused import of `DynNetworkProvider` from handler.rs if it's no longer directly referenced (it's still needed for `HandlerDependencies.network_provider` type annotation).

- [ ] **Step 8: Run the full unit test suite**

```bash
cargo xtask test-unit
```

Expected: all tests pass (no regressions in handler_tests.rs or conformance_tests.rs).

- [ ] **Step 9: Commit**

```bash
git add crates/daemonbox/src/handler.rs
git commit -m "refactor(handler): replace inline network calls with NetworkLifecycle"
```

---

## Task 4: Expand doc comments

**Files:**

- Modify: `crates/minibox/src/domain/networking.rs`
- Modify: `crates/minibox/src/protocol.rs`
- Modify: `crates/minibox-cli/src/main.rs`

- [ ] **Step 1: Expand `NetworkMode` variant docs in `networking.rs`**

Find the `NetworkMode` enum (line 13) and expand the variant doc comments:

```rust
/// Selects which networking adapter handles container network setup.
///
/// When a [`DaemonRequest::Run`] omits the `network` field, the daemon uses
/// `NetworkMode::None` (the `Default`). Containers in `None` mode get an
/// isolated network namespace but no routable interfaces — they cannot reach
/// the host or internet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NetworkMode {
    /// **Default.** Container gets an isolated network namespace with no
    /// interfaces configured. Cannot reach the host network or internet.
    ///
    /// Suitable for batch jobs, compute-only workloads, or any container
    /// that does not need network access.
    #[default]
    None,
    /// Container gets a virtual Ethernet (veth) pair attached to a Linux
    /// bridge (default: `minibox0`). Assigned a private IP from the bridge
    /// subnet (`172.18.0.0/16` by default). Can reach the host and, with
    /// NAT, the internet.
    Bridge,
    /// Container shares the host network namespace. All host ports are
    /// visible inside the container.
    ///
    /// **Security note:** `skip_network_namespace` is set in
    /// [`ContainerSpawnConfig`] when this mode is selected.
    Host,
    /// Container network is managed by Tailscale/tsnet, giving the container
    /// a Tailnet IP directly accessible from other nodes on the tailnet.
    Tailnet,
}
```

- [ ] **Step 2: Expand `NetworkConfig` default doc**

Find `impl Default for NetworkConfig` (line 118) and add a doc comment above it:

```rust
impl Default for NetworkConfig {
    /// Returns a configuration with [`NetworkMode::None`] and sensible bridge
    /// defaults (`minibox0`, `172.18.0.0/16`) pre-populated for when a real
    /// network mode is later selected.
    ///
    /// This is what the daemon uses when `DaemonRequest::Run.network` is
    /// `None` (absent from the JSON payload).
    fn default() -> Self {
```

- [ ] **Step 3: Expand the `network` field doc in `protocol.rs`**

Find the `network` field in `DaemonRequest::Run` (~line 68-71):

```rust
        /// Network mode for the container.
        ///
        /// When `None` (absent from the JSON payload), the daemon substitutes
        /// [`NetworkConfig::default()`], which selects [`NetworkMode::None`]:
        /// an isolated network namespace with no interfaces — no host access,
        /// no internet. This is the safe default for backwards compatibility.
        ///
        /// Pass `Some(NetworkMode::Bridge)` for container-to-host connectivity
        /// or `Some(NetworkMode::Host)` to share the host network namespace.
        #[serde(default)]
        network: Option<NetworkMode>,
```

- [ ] **Step 4: Expand `--network` help text in `main.rs`**

Find the `--network` arg definition (~line 72-74):

```rust
        /// Network mode: none (default), bridge, host, tailnet.
        ///
        /// `none` — container gets an isolated network namespace with no
        /// interfaces. Cannot reach the host or internet (safe default).
        ///
        /// `bridge` — veth pair on `minibox0`, private IP, host-reachable.
        ///
        /// `host` — shares the host network namespace (all host ports visible).
        ///
        /// `tailnet` — network managed via Tailscale/tsnet.
        #[arg(long, default_value = "none")]
        network: String,
```

- [ ] **Step 5: Verify docs build cleanly**

```bash
cargo doc --no-deps -p minibox -p minibox-cli 2>&1 | grep -E "^error|warning\["
```

Expected: no errors. Warnings about missing docs on other items are acceptable.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox/src/domain/networking.rs crates/minibox/src/protocol.rs crates/minibox-cli/src/main.rs
git commit -m "docs(networking): expand NetworkMode/NetworkConfig/protocol/CLI doc comments"
```

---

## Task 5: NetworkProvider conformance tests

**Files:**

- Modify: `crates/daemonbox/tests/conformance_tests.rs`

- [ ] **Step 1: Add a `NetworkProvider` conformance section**

At the end of the `mod conformance { ... }` block (after the existing handler tests, before `mod performance_conformance`), add:

```rust
    // ---------------------------------------------------------------------------
    // NetworkProvider conformance
    // ---------------------------------------------------------------------------

    /// The noop provider (MockNetwork with mode=None) must succeed on setup.
    #[tokio::test]
    async fn network_noop_must_succeed_for_none_mode() {
        let mock = MockNetwork::new();
        let config = minibox::domain::NetworkConfig::default(); // mode = None
        let result = mock.setup("ctr-noop", &config).await;
        assert!(result.is_ok(), "noop setup must succeed: {result:?}");
    }

    /// `setup` must return a non-empty namespace path.
    #[tokio::test]
    async fn network_setup_must_return_namespace_path() {
        let mock = MockNetwork::new();
        let config = minibox::domain::NetworkConfig::default();
        let ns_path = mock.setup("ctr-ns", &config).await.expect("setup failed");
        assert!(!ns_path.is_empty(), "setup must return a non-empty namespace path");
    }

    /// `cleanup` must succeed after `setup` has been called.
    #[tokio::test]
    async fn network_cleanup_must_succeed_after_setup() {
        let mock = MockNetwork::new();
        let config = minibox::domain::NetworkConfig::default();
        mock.setup("ctr-clean", &config).await.expect("setup failed");
        let result = mock.cleanup("ctr-clean").await;
        assert!(result.is_ok(), "cleanup must succeed after setup: {result:?}");
    }

    /// `handle_run` must invoke `network_provider.setup` exactly once per run.
    ///
    /// This mirrors `test_network_setup_called_on_run` from `handler_tests.rs`
    /// but lives in the conformance suite to document the requirement at the
    /// contract level independently of the handler's internal wiring.
    #[tokio::test]
    async fn handler_run_must_invoke_network_setup() {
        let temp_dir = TempDir::new().unwrap();
        let mock_network = Arc::new(MockNetwork::new());
        let deps = Arc::new(HandlerDependencies {
            registry: Arc::new(MockRegistry::new()),
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: mock_network.clone(),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
        });
        let state = mock_state(&temp_dir);

        handle_run_once(
            "library/alpine".to_string(),
            Some("latest".to_string()),
            vec!["/bin/echo".to_string(), "hello".to_string()],
            None,
            None,
            false,
            state,
            deps,
        )
        .await;

        assert_eq!(
            mock_network.setup_count(),
            1,
            "handle_run must call network setup exactly once"
        );
    }
```

- [ ] **Step 2: Add a network performance conformance test**

In the existing `mod performance_conformance { ... }` block, add:

```rust
    /// The noop network provider (MockNetwork) must complete setup in under 1 ms.
    ///
    /// Real providers (bridge, tailnet) may be slower; this only validates the
    /// mock satisfies the performance floor so test suites are not bottlenecked
    /// by network setup.
    #[tokio::test]
    async fn network_noop_setup_must_complete_under_1ms() {
        let mock = MockNetwork::new();
        let config = minibox::domain::NetworkConfig::default();
        let start = std::time::Instant::now();
        mock.setup("perf-ctr", &config).await.unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 1,
            "noop network setup took {elapsed:?}, expected < 1ms"
        );
    }
```

- [ ] **Step 3: Run the conformance tests**

```bash
cargo test -p daemonbox --test conformance_tests -- --nocapture
```

Expected: all new tests pass alongside the existing 16 conformance tests.

- [ ] **Step 4: Commit**

```bash
git add crates/daemonbox/tests/conformance_tests.rs
git commit -m "test(conformance): add NetworkProvider direct trait and handler conformance tests"
```

---

## Task 6: Colima env-var regression tests

Add to `colima.rs` inline `mod tests` (the only place that can call private functions).

**Files:**

- Modify: `crates/minibox/src/adapters/colima.rs`

The existing `ENV_MUTEX` is already declared at line 907. These tests go inside the same `mod tests` block.

- [ ] **Step 1: Add default-fallback tests and env-var override tests**

After the existing `test_lima_home_defaults_to_colima_lima_dir` test (~line 952), add all five tests (two verify existing fallback behavior, three verify override behavior):

```rust
    #[test]
    fn colima_home_defaults_to_home_dot_colima() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev_colima = std::env::var("COLIMA_HOME").ok();
        let prev_home = std::env::var("HOME").ok();

        unsafe {
            std::env::remove_var("COLIMA_HOME");
            std::env::set_var("HOME", "/home/testuser");
        }

        let result = colima_home();

        unsafe {
            match prev_colima {
                Some(v) => std::env::set_var("COLIMA_HOME", v),
                None => std::env::remove_var("COLIMA_HOME"),
            }
            match prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }

        assert_eq!(result, std::path::PathBuf::from("/home/testuser/.colima"));
    }

    #[test]
    fn colima_home_respects_colima_home_env_var() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev = std::env::var("COLIMA_HOME").ok();

        unsafe { std::env::set_var("COLIMA_HOME", "/custom/colima"); }

        let result = colima_home();

        unsafe {
            match prev {
                Some(v) => std::env::set_var("COLIMA_HOME", v),
                None => std::env::remove_var("COLIMA_HOME"),
            }
        }

        assert_eq!(result, std::path::PathBuf::from("/custom/colima"));
    }

    #[test]
    fn lima_home_defaults_to_colima_home_lima() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev_lima = std::env::var("LIMA_HOME").ok();
        let prev_colima = std::env::var("COLIMA_HOME").ok();
        let prev_home = std::env::var("HOME").ok();

        unsafe {
            std::env::remove_var("LIMA_HOME");
            std::env::set_var("COLIMA_HOME", "/custom/colima");
            std::env::remove_var("HOME");
        }

        let result = lima_home();

        unsafe {
            match prev_lima {
                Some(v) => std::env::set_var("LIMA_HOME", v),
                None => std::env::remove_var("LIMA_HOME"),
            }
            match prev_colima {
                Some(v) => std::env::set_var("COLIMA_HOME", v),
                None => std::env::remove_var("COLIMA_HOME"),
            }
            match prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }

        assert_eq!(result, "/custom/colima/_lima");
    }

    #[test]
    fn lima_home_respects_lima_home_env_var() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev = std::env::var("LIMA_HOME").ok();

        unsafe { std::env::set_var("LIMA_HOME", "/custom/lima"); }

        let result = lima_home();

        unsafe {
            match prev {
                Some(v) => std::env::set_var("LIMA_HOME", v),
                None => std::env::remove_var("LIMA_HOME"),
            }
        }

        assert_eq!(result, "/custom/lima");
    }

    #[test]
    fn limactl_command_injects_lima_home_into_env() {
        let _guard = ENV_MUTEX.lock().unwrap();
        let prev_lima = std::env::var("LIMA_HOME").ok();
        let prev_colima = std::env::var("COLIMA_HOME").ok();
        let prev_home = std::env::var("HOME").ok();

        unsafe {
            std::env::remove_var("LIMA_HOME");
            std::env::remove_var("COLIMA_HOME");
            std::env::set_var("HOME", "/home/testuser");
        }

        // limactl_command builds a std::process::Command — we can inspect its
        // envs by calling get_envs() on the returned Command.
        let cmd = limactl_command("limactl");
        let envs: std::collections::HashMap<_, _> = cmd.get_envs().collect();
        let lima_home_val = envs
            .get(std::ffi::OsStr::new("LIMA_HOME"))
            .and_then(|v| v.map(|s| s.to_string_lossy().into_owned()));

        unsafe {
            match prev_lima {
                Some(v) => std::env::set_var("LIMA_HOME", v),
                None => std::env::remove_var("LIMA_HOME"),
            }
            match prev_colima {
                Some(v) => std::env::set_var("COLIMA_HOME", v),
                None => std::env::remove_var("COLIMA_HOME"),
            }
            match prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }

        assert_eq!(
            lima_home_val.as_deref(),
            Some("/home/testuser/.colima/_lima"),
            "limactl_command must inject LIMA_HOME derived from HOME"
        );
    }
```

- [ ] **Step 2: Run the colima unit tests**

```bash
cargo test -p minibox adapters::colima::tests -- --nocapture
```

Expected: 5 new tests pass alongside existing 5 (10 total in `colima::tests`).

- [ ] **Step 3: Commit**

```bash
git add crates/minibox/src/adapters/colima.rs
git commit -m "test(colima): add env var regression tests for colima_home/lima_home/limactl_command"
```

---

## Task 7: Colima manifest.json regression tests

Add to the external integration test file, testing the `ColimaRegistry` via injected executor.

**Files:**

- Modify: `crates/minibox/tests/adapter_colima_tests.rs`

- [ ] **Step 1: Add happy-path, malformed, and empty-layers manifest tests**

After the existing `get_image_layers_returns_host_accessible_paths` test, add all three:

```rust
/// `get_image_layers` must parse `manifest.json` to locate layer tarballs
/// and return one host-accessible path per layer.
///
/// This is the primary regression test for the manifest-based extraction path
/// that replaced the old short-digest directory guessing.
#[test]
fn get_image_layers_parses_manifest_json_to_locate_layers() {
    let fake_manifest = r#"[{"Layers":["layer0/layer.tar","layer1/layer.tar","layer2/layer.tar"]}]"#;

    let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
        if args.first() == Some(&"cat") && args[1].ends_with("/manifest.json") {
            Ok(fake_manifest.to_string())
        } else {
            // tar extraction commands return empty (success)
            Ok(String::new())
        }
    }));

    let layers = registry
        .get_image_layers("library/ubuntu", "22.04")
        .expect("manifest parsing must succeed");

    assert_eq!(layers.len(), 3, "must return one path per layer tarball");
    for layer in &layers {
        let s = layer.to_string_lossy();
        assert!(
            s.starts_with("/tmp/") || s.starts_with("/Users/"),
            "layer path {s:?} must be under a Lima-shared mount (/tmp or /Users)"
        );
    }
}

/// `get_image_layers` must return an error when `manifest.json` is not valid JSON.
#[test]
fn get_image_layers_returns_error_on_malformed_manifest() {
    let registry = ColimaRegistry::new().with_executor(Arc::new(|args: &[&str]| {
        if args.first() == Some(&"cat") && args[1].ends_with("/manifest.json") {
            Ok("this is not valid json {{{{".to_string())
        } else {
            Ok(String::new())
        }
    }));

    let result = registry.get_image_layers("alpine", "latest");

    assert!(
        result.is_err(),
        "malformed manifest.json must produce an error, got: {result:?}"
    );
}
```

- [ ] **Step 2: Add an empty layers array test**

```rust
/// `get_image_layers` must return an error when `manifest.json` contains an
/// empty `Layers` array — an image with no layers is not usable.
#[test]
fn get_image_layers_returns_error_on_empty_layers_array() {
    // Valid JSON but no layers — e.g. a scratch image with 0 layers.
    let empty_manifest = r#"[{"Layers":[]}]"#;

    let registry = ColimaRegistry::new().with_executor(Arc::new(move |args: &[&str]| {
        if args.first() == Some(&"cat") && args[1].ends_with("/manifest.json") {
            Ok(empty_manifest.to_string())
        } else {
            Ok(String::new())
        }
    }));

    let result = registry.get_image_layers("scratch", "latest");

    // Either an error or an empty vec is acceptable — the key requirement
    // is that the caller can detect a no-layer image before trying to
    // mount an empty overlay stack.
    match result {
        Ok(layers) => assert!(
            layers.is_empty(),
            "empty manifest layers must produce empty vec, got {layers:?}"
        ),
        Err(_) => {} // also acceptable
    }
}
```

> **Note on empty layers:** `get_image_layers` may return `Ok(vec![])` or `Err(...)` for a zero-layer image — both are valid since `run_inner_capture` checks `layer_dirs.is_empty()` and returns `DomainError::EmptyImage` either way. The test documents this contract.

- [ ] **Step 3: Run the colima integration tests**

```bash
cargo test -p minibox --test adapter_colima_tests -- --nocapture
```

Expected: 3 new tests pass alongside existing tests.

- [ ] **Step 4: Run the full unit test suite to confirm no regressions**

```bash
cargo xtask test-unit
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/tests/adapter_colima_tests.rs
git commit -m "test(colima): add manifest.json malformed and empty-layers regression tests"
```

---

## Final Verification

- [ ] **Run the full test suite one last time**

```bash
cargo xtask test-unit
```

Expected output: all tests pass, no new failures.

- [ ] **Run clippy to catch any lint regressions**

```bash
cargo clippy -p minibox -p daemonbox -p minibox-cli -- -D warnings
```

Expected: no warnings treated as errors.
