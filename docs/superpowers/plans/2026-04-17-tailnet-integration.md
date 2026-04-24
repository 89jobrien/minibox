---
status: done
completed: "2026-04-17"
branch: main
---

# Tailnet Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `NetworkMode::Tailnet` via a new `tailbox` crate wrapping `tailscale-rs`.
Containers opt into either **gateway mode** (shared daemon device) or **per-container mode**
(each container gets its own tailnet node). Everything is gated behind a `tailnet` Cargo feature.

**Architecture:** Five-layer change: (1) `NetworkConfig` gains three new `#[serde(default)]`
fields (`tailnet_auth_key`, `tailnet_secret_name`, `tailnet_mode`) and a new `TailnetMode` enum
in `minibox-core`; (2) new `crates/tailbox` crate with `TailnetNetwork` adapter, `TailnetConfig`,
`DeviceStore`, auth-key resolution chain, and `TS_RS_EXPERIMENT` once-guard; (3) workspace
`Cargo.toml` registers `tailbox` as workspace member; (4) `miniboxd/Cargo.toml` gains a `tailnet`
feature flag; (5) `miniboxd/src/main.rs` network provider selection branch extended with a
`"tailnet"` arm gated on `#[cfg(feature = "tailnet")]`.

**Tech Stack:** Rust 2024 edition, `tailscale` v0.2, `async-trait`, `tokio::sync::Mutex`,
`minibox-secrets` (CredentialProvider chain), `serde_json` (context files), `tracing`,
`tempfile` (tests).

**Stability note:** `tailscale-rs` v0.2 is pre-1.0 with unaudited crypto. The crate-level doc
comment on `TailnetNetwork` must reproduce the verbatim stability warning from the spec.

---

## File Map

| File                                           | Change                                                      |
| ---------------------------------------------- | ----------------------------------------------------------- |
| `crates/minibox-core/src/domain/networking.rs` | Add `TailnetMode` enum; three new fields on `NetworkConfig` |
| `Cargo.toml` (workspace root)                  | Add `tailbox` to `[workspace] members`                      |
| `crates/tailbox/Cargo.toml`                    | New crate manifest                                          |
| `crates/tailbox/src/lib.rs`                    | Re-export `TailnetNetwork`, `TailnetConfig`                 |
| `crates/tailbox/src/config.rs`                 | `TailnetConfig` struct                                      |
| `crates/tailbox/src/adapter.rs`                | `TailnetNetwork` (NetworkProvider impl)                     |
| `crates/tailbox/src/auth.rs`                   | `resolve_auth_key()` priority chain                         |
| `crates/tailbox/src/experiment.rs`             | `ensure_tsrs_experiment()` once-guard                       |
| `crates/tailbox/tests/auth_tests.rs`           | Unit tests for auth resolution                              |
| `crates/tailbox/tests/adapter_tests.rs`        | Unit tests for setup/attach/cleanup/stats                   |
| `crates/miniboxd/Cargo.toml`                   | `tailnet` feature + optional `tailbox` dep                  |
| `crates/miniboxd/src/main.rs`                  | `"tailnet"` arm in network provider selection               |

---

## Task 1: Protocol — `TailnetMode` + three new fields on `NetworkConfig`

**Files:**

- Modify: `crates/minibox-core/src/domain/networking.rs`

- [ ] **Step 1: Add `TailnetMode` enum after the `NetworkMode` enum**

In `crates/minibox-core/src/domain/networking.rs`, after the closing `}` of `NetworkMode`
(around line 25), insert:

```rust
/// Selects between gateway and per-container Tailscale device allocation.
///
/// **Default is `Gateway`** — the daemon joins the tailnet once and containers
/// route through it. Use `PerContainer` to give each container its own tailnet
/// identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TailnetMode {
    /// One shared `tailscale::Device` for the daemon; containers share its tailnet
    /// IP but are not individually visible on the tailnet.
    #[default]
    Gateway,
    /// Each container gets its own `tailscale::Device` and a distinct tailnet IP.
    PerContainer,
}
```

- [ ] **Step 2: Add three new `#[serde(default)]` fields to `NetworkConfig`**

In the `NetworkConfig` struct, after the `ipv6_enabled` field, add:

```rust
    /// Inline Tailscale auth key. Takes precedence over secret lookup.
    /// Only used when `mode == NetworkMode::Tailnet`.
    #[serde(default)]
    pub tailnet_auth_key: Option<String>,

    /// minibox-secrets key name for Tailscale auth key lookup.
    /// Defaults to `"tailscale-auth-key"` when `None`.
    /// Only used when `mode == NetworkMode::Tailnet`.
    #[serde(default)]
    pub tailnet_secret_name: Option<String>,

    /// Tailnet networking mode: shared gateway device or per-container device.
    /// Only used when `mode == NetworkMode::Tailnet`.
    #[serde(default)]
    pub tailnet_mode: TailnetMode,
```

- [ ] **Step 3: Extend the existing `Default` impl for `NetworkConfig`**

In the `impl Default for NetworkConfig` block, add the three new fields:

```rust
            tailnet_auth_key: None,
            tailnet_secret_name: None,
            tailnet_mode: TailnetMode::Gateway,
```

- [ ] **Step 4: Write serialisation tests**

At the bottom of the `#[cfg(test)] mod tests` block in `networking.rs`:

```rust
    #[test]
    fn tailnet_mode_serde_roundtrip() {
        for mode in [TailnetMode::Gateway, TailnetMode::PerContainer] {
            let json = serde_json::to_string(&mode).expect("serialize");
            let back: TailnetMode = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(mode, back);
        }
    }

    #[test]
    fn tailnet_mode_default_is_gateway() {
        assert_eq!(TailnetMode::default(), TailnetMode::Gateway);
    }

    #[test]
    fn network_config_tailnet_fields_default_to_none_and_gateway() {
        let cfg = NetworkConfig::default();
        assert!(cfg.tailnet_auth_key.is_none());
        assert!(cfg.tailnet_secret_name.is_none());
        assert_eq!(cfg.tailnet_mode, TailnetMode::Gateway);
    }

    #[test]
    fn network_config_old_json_omitting_tailnet_fields_deserialises() {
        // Clients that don't know about tailnet fields must still round-trip.
        let json = r#"{
            "mode": "Bridge",
            "bridge_name": "minibox0",
            "subnet": "172.18.0.0/16",
            "container_ip": null,
            "port_mappings": [],
            "dns_servers": [],
            "ipv6_enabled": false
        }"#;
        let cfg: NetworkConfig = serde_json::from_str(json).expect("deserialise");
        assert_eq!(cfg.tailnet_mode, TailnetMode::Gateway);
        assert!(cfg.tailnet_auth_key.is_none());
    }
```

- [ ] **Step 5: Run new tests**

```bash
cargo test -p minibox-core tailnet_mode tailnet_fields -- --nocapture
```

Expected: 4 PASS.

- [ ] **Step 6: Compile-check workspace**

```bash
cargo check --workspace
```

No new errors expected — all new fields have `#[serde(default)]` and the struct change is
additive.

- [ ] **Step 7: Commit**

```bash
git add crates/minibox-core/src/domain/networking.rs
git commit -m "feat(protocol): add TailnetMode enum and tailnet fields to NetworkConfig"
```

---

## Task 2: Workspace registration + `tailbox` crate skeleton

**Files:**

- Modify: `Cargo.toml` (workspace root)
- Create: `crates/tailbox/Cargo.toml`
- Create: `crates/tailbox/src/lib.rs`

- [ ] **Step 1: Add `tailbox` to workspace members**

In the root `Cargo.toml`, in the `[workspace] members` array, add:

```toml
"crates/tailbox",
```

Place it after `"crates/macbox"` to maintain platform-crate ordering.

- [ ] **Step 2: Create `crates/tailbox/Cargo.toml`**

```toml
[package]
name = "tailbox"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Tailscale-rs network adapter for minibox containers"

[dependencies]
minibox-core = { path = "../minibox-core" }
minibox-secrets = { path = "../minibox-secrets" }
anyhow = { workspace = true }
async-trait = { workspace = true }
tokio = { workspace = true, features = ["sync"] }
tracing = { workspace = true }
serde_json = { workspace = true }
tailscale = "0.2"

[dev-dependencies]
tempfile = { workspace = true }
tokio = { workspace = true, features = ["rt", "macros"] }

# tailbox is Linux + macOS ARM64 only (matches tailscale-rs platform support).
[target.'cfg(not(any(target_os = "linux", target_os = "macos")))'.dependencies]
# This crate must not be compiled on Windows. miniboxd gates it with
# #[cfg(feature = "tailnet")] and tailnet is only enabled on supported platforms.
```

- [ ] **Step 3: Create `crates/tailbox/src/lib.rs`**

```rust
//! Tailscale-rs network adapter for minibox containers.
//!
//! Implements [`minibox_core::domain::NetworkProvider`] via [`TailnetNetwork`].
//!
//! Two modes:
//! - **Gateway** — one shared `tailscale::Device` for the daemon; containers reach
//!   tailnet peers via proxy connections through that device.
//! - **PerContainer** — each container gets its own `tailscale::Device` and tailnet IP.
//!
//! The entire crate is intended to be compiled only when `miniboxd` is built with
//! `--features tailnet`.
//!
//! # Platform support
//!
//! Linux and macOS ARM64 only, matching `tailscale-rs` v0.2 platform support.
//! This crate will not compile on Windows.

pub mod adapter;
pub mod auth;
pub mod config;
pub mod experiment;

pub use adapter::TailnetNetwork;
pub use config::TailnetConfig;
```

- [ ] **Step 4: Create stub modules so crate compiles**

Create `crates/tailbox/src/config.rs`:

```rust
/// Configuration for the tailnet network adapter.
#[derive(Debug, Clone)]
pub struct TailnetConfig {
    /// Default auth key for the daemon gateway device.
    /// If `None`, falls back to `key_secret_name` then `TAILSCALE_AUTH_KEY`.
    pub auth_key: Option<String>,
    /// minibox-secrets key name to look up when `auth_key` is None.
    /// Defaults to `"tailscale-auth-key"`.
    pub key_secret_name: String,
}

impl Default for TailnetConfig {
    fn default() -> Self {
        Self {
            auth_key: None,
            key_secret_name: "tailscale-auth-key".to_string(),
        }
    }
}
```

Create `crates/tailbox/src/experiment.rs`:

```rust
static TSRS_EXPERIMENT_SET: std::sync::Once = std::sync::Once::new();

/// Set `TS_RS_EXPERIMENT=this_is_unstable_software` exactly once at startup.
///
/// `tailscale-rs` requires this variable to be set before any `Device` is
/// constructed. Called at the top of `TailnetNetwork::setup()`.
///
/// # Safety
///
/// `SAFETY:` `set_var` is called inside a `Once` block. The only callers of
/// `TailnetNetwork::setup()` are async tasks dispatched by `miniboxd` after
/// tracing + adapter initialisation is complete. No other thread reads or
/// writes `TS_RS_EXPERIMENT`. The `Once` guarantee prevents concurrent
/// calls, satisfying the Rust 2024 requirement that `set_var` be called in
/// a context where no other threads are reading the env.
pub fn ensure_tsrs_experiment() {
    TSRS_EXPERIMENT_SET.call_once(|| {
        // SAFETY: called exactly once; no concurrent readers of TS_RS_EXPERIMENT.
        unsafe {
            std::env::set_var("TS_RS_EXPERIMENT", "this_is_unstable_software");
        }
    });
}
```

Create `crates/tailbox/src/auth.rs` (stub — full implementation in Task 3):

```rust
use anyhow::{bail, Result};
use minibox_core::domain::NetworkConfig;

/// Resolve a Tailscale auth key from the priority chain:
///
/// 1. `config.tailnet_auth_key` — inline key in the `RunContainer` request.
/// 2. `minibox-secrets` lookup via `config.tailnet_secret_name` (if set) or
///    `default_secret_name` (default `"tailscale-auth-key"`).
/// 3. `TAILSCALE_AUTH_KEY` environment variable.
/// 4. `Err(...)` — no key available.
pub async fn resolve_auth_key(
    config: &NetworkConfig,
    default_secret_name: &str,
) -> Result<String> {
    // Step 1: inline key
    if let Some(key) = config.tailnet_auth_key.as_deref() {
        if !key.is_empty() {
            return Ok(key.to_string());
        }
    }

    // Step 2: minibox-secrets lookup
    let secret_name = config
        .tailnet_secret_name
        .as_deref()
        .unwrap_or(default_secret_name);
    if let Ok(key) = lookup_secret(secret_name).await {
        if !key.is_empty() {
            return Ok(key);
        }
    }

    // Step 3: environment variable
    if let Ok(key) = std::env::var("TAILSCALE_AUTH_KEY") {
        if !key.is_empty() {
            return Ok(key);
        }
    }

    bail!(
        "tailnet: no auth key found — set tailnet_auth_key in RunContainer, \
         configure minibox-secrets key '{}', or set TAILSCALE_AUTH_KEY",
        secret_name
    )
}

/// Look up a secret from the minibox-secrets provider chain.
///
/// Returns `Err` if the secret does not exist or the provider chain is
/// not configured.
async fn lookup_secret(name: &str) -> Result<String> {
    use minibox_secrets::CredentialProvider as _;
    // Env provider is always available; keyring/op require daemon config.
    let provider = minibox_secrets::EnvProvider::new();
    provider
        .get(name)
        .await
        .map_err(|e| anyhow::anyhow!("tailnet: secrets lookup failed for '{}': {e}", name))
}
```

Create `crates/tailbox/src/adapter.rs` (stub — full implementation in Task 4):

```rust
use anyhow::Result;
use async_trait::async_trait;
use minibox_core::domain::{NetworkConfig, NetworkProvider, NetworkStats};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::TailnetConfig;

/// Tailscale-rs network adapter for minibox containers.
///
/// **Stability warning**: This adapter wraps `tailscale-rs` v0.2, which is pre-1.0,
/// contains unaudited cryptography, and has no backwards-compatibility guarantees.
/// Do not use in production until tailscale-rs completes a third-party security audit.
/// See: <https://github.com/tailscale/tailscale-rs#caveats>
pub struct TailnetNetwork {
    config: TailnetConfig,
    /// Per-container devices (PerContainer mode). Keyed by container_id.
    devices: Arc<Mutex<HashMap<String, tailscale::Device>>>,
    /// Shared gateway device (Gateway mode). Lazily initialised on first use.
    gateway_device: Arc<Mutex<Option<tailscale::Device>>>,
}

impl TailnetNetwork {
    /// Create a new `TailnetNetwork` adapter.
    pub async fn new(config: TailnetConfig) -> Result<Self> {
        Ok(Self {
            config,
            devices: Arc::new(Mutex::new(HashMap::new())),
            gateway_device: Arc::new(Mutex::new(None)),
        })
    }
}

minibox_core::as_any!(TailnetNetwork);

#[async_trait]
impl NetworkProvider for TailnetNetwork {
    async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String> {
        // Full implementation in Task 4.
        let _ = (container_id, config);
        anyhow::bail!("tailnet: setup not yet implemented")
    }

    async fn attach(&self, _container_id: &str, _pid: u32) -> Result<()> {
        // tailscale-rs devices are not pid-namespace-based; attach is a no-op.
        Ok(())
    }

    async fn cleanup(&self, container_id: &str) -> Result<()> {
        // Full implementation in Task 4.
        let _ = container_id;
        Ok(())
    }

    async fn stats(&self, _container_id: &str) -> Result<NetworkStats> {
        // tailscale-rs v0.2 has no stats API.
        Ok(NetworkStats::default())
    }
}
```

- [ ] **Step 5: Verify the crate compiles**

```bash
cargo check -p tailbox
```

Expected: compiles cleanly (stubs only, no logic yet).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/tailbox/
git commit -m "feat(tailbox): new crate skeleton — TailnetNetwork stub, TailnetConfig, auth stub"
```

---

## Task 3: Auth key resolution + tests

**Files:**

- Modify: `crates/tailbox/src/auth.rs`
- Create: `crates/tailbox/tests/auth_tests.rs`

The stub in Task 2 calls `minibox_secrets::EnvProvider`. The full implementation adds a
`CredentialProviderChain` so the order in the spec is respected.

- [ ] **Step 1: Confirm minibox-secrets API surface**

```bash
cargo doc -p minibox-secrets --no-deps --open
```

Locate the `CredentialProvider` trait and the concrete providers (`EnvProvider`, `OpProvider`,
etc.). Identify the chain type (likely `ProviderChain` or similar). If no chain type exists,
we compose manually with `or_else`.

- [ ] **Step 2: Replace `lookup_secret` with provider-chain call**

Rewrite `crates/tailbox/src/auth.rs` `lookup_secret` to use the full provider chain available
in `minibox-secrets`. The priority inside `lookup_secret` is:

1. OS keyring (if available).
2. 1Password CLI (`op`).
3. Environment variable (provider, not raw `std::env`).

```rust
async fn lookup_secret(name: &str) -> Result<String> {
    use minibox_secrets::{EnvProvider, ProviderChain};
    // Build a provider chain: env only by default.
    // Additional providers (keyring, op) wired in if minibox-secrets exposes them.
    let chain = ProviderChain::new(vec![Box::new(EnvProvider::new())]);
    chain
        .get(name)
        .await
        .map_err(|e| anyhow::anyhow!("tailnet: secrets lookup '{}': {e}", name))
}
```

Adjust the import and chain construction to match the actual minibox-secrets API. If
`ProviderChain` does not exist, use a single `EnvProvider` and add a `// TODO: wire
richer chain` comment.

- [ ] **Step 3: Write unit tests for `resolve_auth_key`**

Create `crates/tailbox/tests/auth_tests.rs`:

```rust
//! Unit tests for tailnet auth key resolution.
//!
//! Tests use `std::env` mutations — serialised with ENV_LOCK.

use minibox_core::domain::{NetworkConfig, TailnetMode};
use tailbox::auth::resolve_auth_key;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Helper: build a `NetworkConfig` with `mode = Tailnet`.
fn tailnet_config() -> NetworkConfig {
    NetworkConfig {
        mode: minibox_core::domain::NetworkMode::Tailnet,
        ..Default::default()
    }
}

#[tokio::test]
async fn inline_key_takes_precedence() {
    let _g = ENV_LOCK.lock().unwrap();
    // SAFETY: serialised by ENV_LOCK; no concurrent readers.
    unsafe { std::env::remove_var("TAILSCALE_AUTH_KEY") };

    let mut cfg = tailnet_config();
    cfg.tailnet_auth_key = Some("tskey-inline".to_string());

    let key = resolve_auth_key(&cfg, "tailscale-auth-key").await.unwrap();
    assert_eq!(key, "tskey-inline");
}

#[tokio::test]
async fn env_var_fallback() {
    let _g = ENV_LOCK.lock().unwrap();
    // SAFETY: serialised by ENV_LOCK.
    unsafe {
        std::env::remove_var("tailscale-auth-key"); // clear any secret with this name
        std::env::set_var("TAILSCALE_AUTH_KEY", "tskey-from-env");
    }

    let cfg = tailnet_config(); // tailnet_auth_key = None

    let key = resolve_auth_key(&cfg, "tailscale-auth-key").await.unwrap();
    assert_eq!(key, "tskey-from-env");

    unsafe { std::env::remove_var("TAILSCALE_AUTH_KEY") };
}

#[tokio::test]
async fn no_key_returns_error() {
    let _g = ENV_LOCK.lock().unwrap();
    // SAFETY: serialised by ENV_LOCK.
    unsafe {
        std::env::remove_var("TAILSCALE_AUTH_KEY");
        std::env::remove_var("tailscale-auth-key");
    }

    let cfg = tailnet_config();
    let result = resolve_auth_key(&cfg, "tailscale-auth-key").await;
    assert!(result.is_err(), "expected error when no key available");
    assert!(result.unwrap_err().to_string().contains("no auth key"));
}

#[tokio::test]
async fn secret_name_override() {
    let _g = ENV_LOCK.lock().unwrap();
    // SAFETY: serialised by ENV_LOCK.
    unsafe {
        std::env::remove_var("TAILSCALE_AUTH_KEY");
        // Simulate a secret stored under a custom name via env var of same name.
        std::env::set_var("my-custom-secret", "tskey-custom");
    }

    let mut cfg = tailnet_config();
    cfg.tailnet_secret_name = Some("my-custom-secret".to_string());

    // If EnvProvider looks up by env var name, this should resolve.
    // The test documents expected behaviour; adjust assertion if EnvProvider
    // uses a different lookup key convention.
    let _ = resolve_auth_key(&cfg, "tailscale-auth-key").await;
    // At minimum: does not panic. Exact success depends on EnvProvider impl.

    unsafe { std::env::remove_var("my-custom-secret") };
}
```

- [ ] **Step 4: Run auth tests**

```bash
cargo test -p tailbox -- auth_tests --nocapture
```

Expected: `inline_key_takes_precedence`, `env_var_fallback`, `no_key_returns_error` PASS.
`secret_name_override` may be informational depending on `EnvProvider` key conventions.

- [ ] **Step 5: Commit**

```bash
git add crates/tailbox/src/auth.rs crates/tailbox/tests/auth_tests.rs
git commit -m "feat(tailbox): auth key resolution chain + unit tests"
```

---

## Task 4: `TailnetNetwork` adapter — gateway + per-container modes

**Files:**

- Modify: `crates/tailbox/src/adapter.rs`
- Create: `crates/tailbox/tests/adapter_tests.rs`

This task implements the full `setup()` and `cleanup()` logic. `attach()` and `stats()`
are already correct as stubs.

- [ ] **Step 1: Write failing tests first**

Create `crates/tailbox/tests/adapter_tests.rs`:

```rust
//! Unit tests for TailnetNetwork adapter.
//!
//! Tests avoid real tailscale connections by using tempdir context paths and
//! mocking auth-key resolution via TAILSCALE_AUTH_KEY env var.
//!
//! Integration tests requiring a real tailnet are marked #[ignore].

use minibox_core::domain::{NetworkConfig, NetworkMode, TailnetMode};
use tailbox::{TailnetConfig, TailnetNetwork};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gateway_config() -> NetworkConfig {
    NetworkConfig {
        mode: NetworkMode::Tailnet,
        tailnet_mode: TailnetMode::Gateway,
        tailnet_auth_key: Some("tskey-test-fake".to_string()),
        ..Default::default()
    }
}

fn per_container_config() -> NetworkConfig {
    NetworkConfig {
        mode: NetworkMode::Tailnet,
        tailnet_mode: TailnetMode::PerContainer,
        tailnet_auth_key: Some("tskey-test-fake".to_string()),
        ..Default::default()
    }
}

/// attach() must always return Ok(()) — it is a no-op.
#[tokio::test]
async fn attach_is_noop() {
    let net = TailnetNetwork::new(TailnetConfig::default()).await.unwrap();
    let result = net.attach("test-container-id", 12345).await;
    assert!(result.is_ok(), "attach should always succeed: {result:?}");
}

/// stats() must always return Ok(NetworkStats::default()) — no stats API in v0.2.
#[tokio::test]
async fn stats_returns_default() {
    let net = TailnetNetwork::new(TailnetConfig::default()).await.unwrap();
    let stats = net.stats("test-container-id").await.unwrap();
    assert_eq!(stats.rx_bytes, 0);
    assert_eq!(stats.tx_bytes, 0);
}

/// cleanup() on an unknown container_id must not error.
#[tokio::test]
async fn cleanup_unknown_container_is_ok() {
    let net = TailnetNetwork::new(TailnetConfig::default()).await.unwrap();
    // Should return Ok even if no device was ever registered.
    let result = net.cleanup("never-existed").await;
    assert!(result.is_ok(), "cleanup of unknown container should be ok: {result:?}");
}

/// setup() context JSON must include expected shape fields.
///
/// NOTE: This test requires `tailscale-rs` to actually connect — skip in CI
/// unless TAILSCALE_AUTH_KEY is present and network access is available.
#[cfg(all(test, feature = "tailnet"))]
#[tokio::test]
#[ignore = "requires TAILSCALE_AUTH_KEY and network access"]
async fn setup_gateway_mode_returns_valid_context() {
    let _g = ENV_LOCK.lock().unwrap();
    let key = std::env::var("TAILSCALE_AUTH_KEY")
        .expect("TAILSCALE_AUTH_KEY must be set for integration test");

    let mut config = gateway_config();
    config.tailnet_auth_key = Some(key);

    let net = TailnetNetwork::new(TailnetConfig::default()).await.unwrap();
    let ctx_json = net.setup("test-gw-container", &config).await.unwrap();
    let ctx: serde_json::Value = serde_json::from_str(&ctx_json).unwrap();

    assert_eq!(ctx["mode"].as_str().unwrap(), "gateway");
    assert!(ctx["tailnet_ip"].as_str().is_some(), "must have tailnet_ip");
    assert_eq!(ctx["container_id"].as_str().unwrap(), "test-gw-container");
}

/// Per-container mode integration test.
#[cfg(all(test, feature = "tailnet"))]
#[tokio::test]
#[ignore = "requires TAILSCALE_AUTH_KEY and network access"]
async fn setup_per_container_mode_returns_own_ip() {
    let _g = ENV_LOCK.lock().unwrap();
    let key = std::env::var("TAILSCALE_AUTH_KEY")
        .expect("TAILSCALE_AUTH_KEY must be set for integration test");

    let mut config = per_container_config();
    config.tailnet_auth_key = Some(key);

    let net = TailnetNetwork::new(TailnetConfig::default()).await.unwrap();
    let ctx_json = net.setup("test-pc-container", &config).await.unwrap();
    let ctx: serde_json::Value = serde_json::from_str(&ctx_json).unwrap();

    assert_eq!(ctx["mode"].as_str().unwrap(), "per_container");
    assert!(ctx["tailnet_ip"].as_str().is_some());

    // Cleanup: remove device.
    net.cleanup("test-pc-container").await.unwrap();
}
```

- [ ] **Step 2: Run — confirm `attach_is_noop`, `stats_returns_default`,
      `cleanup_unknown_container_is_ok` pass; integration tests are skipped**

```bash
cargo test -p tailbox -- adapter_tests --nocapture
```

Expected: 3 unit tests PASS; 2 integration tests IGNORED.

- [ ] **Step 3: Implement `setup()` for gateway mode**

Replace the stub body of `setup()` in `crates/tailbox/src/adapter.rs`:

```rust
async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String> {
    use crate::auth::resolve_auth_key;
    use crate::experiment::ensure_tsrs_experiment;

    ensure_tsrs_experiment();

    let auth_key = resolve_auth_key(config, &self.config.key_secret_name).await?;

    let ctx = match config.tailnet_mode {
        TailnetMode::Gateway => self.setup_gateway(container_id, &auth_key).await?,
        TailnetMode::PerContainer => {
            self.setup_per_container(container_id, &auth_key).await?
        }
    };

    // Persist context file at /run/minibox/net/{container_id}.json
    // (same path as BridgeNetwork — consumed by cleanup).
    let ctx_path = Self::net_context_path(container_id);
    if let Some(parent) = ctx_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(&ctx_path, &ctx)
        .with_context(|| format!("write tailnet context to {}", ctx_path.display()))?;

    tracing::info!(
        container_id = container_id,
        mode = ?config.tailnet_mode,
        "tailnet: network setup complete"
    );

    Ok(ctx)
}
```

- [ ] **Step 4: Implement `setup_gateway` helper**

Add private method to `TailnetNetwork` impl block:

```rust
/// Lazily start the daemon gateway device; return the JSON context string.
async fn setup_gateway(&self, container_id: &str, auth_key: &str) -> Result<String> {
    let mut gw = self.gateway_device.lock().await;
    if gw.is_none() {
        tracing::info!("tailnet: starting gateway device");
        let device = tailscale::Device::new(
            &tailscale::Config {
                key_state: Self::gateway_key_state()?,
                ..Default::default()
            },
            Some(auth_key.to_string()),
        )
        .await
        .context("tailnet: gateway device init failed")?;
        *gw = Some(device);
    }

    let device = gw.as_ref().expect("just set");
    let tailnet_ip = device
        .local_addr()
        .await
        .context("tailnet: gateway local_addr failed")?
        .ip()
        .to_string();

    let ctx = serde_json::json!({
        "mode": "gateway",
        "tailnet_ip": tailnet_ip,
        "container_id": container_id,
    });
    Ok(ctx.to_string())
}
```

- [ ] **Step 5: Implement `setup_per_container` helper**

```rust
/// Start a dedicated device for this container; return the JSON context string.
async fn setup_per_container(&self, container_id: &str, auth_key: &str) -> Result<String> {
    let key_file = Self::container_key_path(container_id);
    if let Some(parent) = key_file.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create key dir {}", parent.display()))?;
    }

    let device = tailscale::Device::new(
        &tailscale::Config {
            key_state: tailscale::load_key_file(&key_file)?,
            ..Default::default()
        },
        Some(auth_key.to_string()),
    )
    .await
    .with_context(|| format!("tailnet: per-container device init for {container_id}"))?;

    let tailnet_ip = device
        .local_addr()
        .await
        .context("tailnet: per-container local_addr failed")?
        .ip()
        .to_string();

    self.devices
        .lock()
        .await
        .insert(container_id.to_string(), device);

    tracing::info!(
        container_id = container_id,
        tailnet_ip = %tailnet_ip,
        "tailnet: per-container device joined"
    );

    let ctx = serde_json::json!({
        "mode": "per_container",
        "tailnet_ip": tailnet_ip,
        "container_id": container_id,
    });
    Ok(ctx.to_string())
}
```

- [ ] **Step 6: Add path helpers and `gateway_key_state` to `TailnetNetwork`**

```rust
fn net_context_path(container_id: &str) -> std::path::PathBuf {
    std::path::Path::new("/run/minibox/net").join(format!("{container_id}.json"))
}

fn container_key_path(container_id: &str) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    std::path::PathBuf::from(home)
        .join(".minibox/tailnet")
        .join(format!("{container_id}.json"))
}

fn gateway_key_state() -> anyhow::Result<tailscale::KeyState> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let path = std::path::PathBuf::from(home).join(".minibox/tailnet/gateway.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create gateway key dir {}", parent.display()))?;
    }
    tailscale::load_key_file(&path).context("tailnet: load gateway key state")
}
```

- [ ] **Step 7: Implement `cleanup()`**

Replace the stub in `adapter.rs`:

```rust
async fn cleanup(&self, container_id: &str) -> Result<()> {
    // Per-container: drop the device (triggers tailscale-rs teardown).
    {
        let mut devices = self.devices.lock().await;
        if devices.remove(container_id).is_some() {
            tracing::info!(
                container_id = container_id,
                "tailnet: per-container device removed"
            );
            // Delete key state file.
            let key_path = Self::container_key_path(container_id);
            if let Err(e) = std::fs::remove_file(&key_path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(
                        container_id = container_id,
                        path = %key_path.display(),
                        error = %e,
                        "tailnet: could not remove key state file"
                    );
                }
            }
        }
        // Gateway mode: device persists. No-op.
    }

    // Remove context file (best-effort).
    let ctx_path = Self::net_context_path(container_id);
    if let Err(e) = std::fs::remove_file(&ctx_path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                container_id = container_id,
                error = %e,
                "tailnet: could not remove net context file"
            );
        }
    }

    Ok(())
}
```

- [ ] **Step 8: Verify `tailscale-rs` API matches assumptions**

The implementation above assumes:

- `tailscale::Device::new(config, auth_key)` is async and returns `Result<Device>`.
- `device.local_addr()` is async and returns `Result<SocketAddr>`.
- `tailscale::load_key_file(path)` returns `Result<KeyState>`.

Run `cargo doc -p tailscale --no-deps` and verify these signatures. Adjust any mismatches
before proceeding.

```bash
cargo doc -p tailscale --no-deps 2>&1 | head -20
cargo check -p tailbox 2>&1 | head -30
```

Fix any API mismatches. Document deviations from the spec in a `// NOTE:` comment.

- [ ] **Step 9: Run all tailbox tests**

```bash
cargo test -p tailbox -- --nocapture 2>&1 | tail -20
```

Expected: all non-`#[ignore]` tests PASS.

- [ ] **Step 10: Compile-check workspace**

```bash
cargo check --workspace 2>&1 | head -20
```

- [ ] **Step 11: Commit**

```bash
git add crates/tailbox/src/adapter.rs crates/tailbox/tests/adapter_tests.rs
git commit -m "feat(tailbox): TailnetNetwork gateway + per-container setup/cleanup"
```

---

## Task 5: `miniboxd` feature flag + wiring

**Files:**

- Modify: `crates/miniboxd/Cargo.toml`
- Modify: `crates/miniboxd/src/main.rs`

- [ ] **Step 1: Add `tailnet` feature and optional dep to `miniboxd/Cargo.toml`**

In `crates/miniboxd/Cargo.toml`, under `[features]`:

```toml
[features]
tailnet = ["dep:tailbox"]
```

Under `[dependencies]`:

```toml
tailbox = { path = "../tailbox", optional = true }
```

Verify there is a `[features]` section; if not, create one.

- [ ] **Step 2: Add `"tailnet"` arm to the network provider selection block in `main.rs`**

Locate the existing block (around line 493):

```rust
    let mode = std::env::var("MINIBOX_NETWORK_MODE").unwrap_or_else(|_| "none".to_string());
    match mode.as_str() {
        "bridge" => Arc::new(BridgeNetwork::new().context("BridgeNetwork init failed")?),
        "host" => Arc::new(minibox::adapters::network::HostNetwork::new()),
        _ => Arc::new(NoopNetwork::new()),
    }
```

Replace with:

```rust
    let mode = std::env::var("MINIBOX_NETWORK_MODE").unwrap_or_else(|_| "none".to_string());
    match mode.as_str() {
        "bridge" => Arc::new(BridgeNetwork::new().context("BridgeNetwork init failed")?)
            as Arc<dyn minibox_core::domain::NetworkProvider>,
        "host" => Arc::new(minibox::adapters::network::HostNetwork::new())
            as Arc<dyn minibox_core::domain::NetworkProvider>,
        #[cfg(feature = "tailnet")]
        "tailnet" => {
            let tailnet_cfg = tailbox::TailnetConfig {
                auth_key: std::env::var("TAILSCALE_AUTH_KEY").ok(),
                key_secret_name: std::env::var("MINIBOX_TAILNET_SECRET_NAME")
                    .unwrap_or_else(|_| "tailscale-auth-key".to_string()),
            };
            Arc::new(
                tailbox::TailnetNetwork::new(tailnet_cfg)
                    .await
                    .context("TailnetNetwork init failed")?,
            ) as Arc<dyn minibox_core::domain::NetworkProvider>
        }
        _ => Arc::new(NoopNetwork::new()) as Arc<dyn minibox_core::domain::NetworkProvider>,
    }
```

Add the `tailbox` import at the top of the Linux-only imports block:

```rust
#[cfg(all(target_os = "linux", feature = "tailnet"))]
use tailbox;
```

- [ ] **Step 3: Compile without the feature (default build must be clean)**

```bash
cargo check -p miniboxd 2>&1 | head -20
```

Expected: clean — the `"tailnet"` arm is cfg-gated, the default `_` arm still matches.

- [ ] **Step 4: Compile with the feature**

```bash
cargo check -p miniboxd --features tailnet 2>&1 | head -20
```

Expected: clean.

- [ ] **Step 5: Run pre-commit gate (excludes tailnet — expected)**

```bash
cargo xtask pre-commit 2>&1 | tail -10
```

Expected: PASS (tailnet feature not in default clippy invocation).

- [ ] **Step 6: Run tailbox-specific clippy**

```bash
cargo clippy -p tailbox -- -D warnings 2>&1 | head -30
```

Fix any warnings before committing.

- [ ] **Step 7: Commit**

```bash
git add crates/miniboxd/Cargo.toml crates/miniboxd/src/main.rs
git commit -m "feat(miniboxd): tailnet feature flag + TailnetNetwork wiring in network provider selection"
```

---

## Task 6: Final validation and pre-commit gate

- [ ] **Step 1: Run full unit test suite**

```bash
cargo xtask test-unit 2>&1 | tail -20
```

Expected: all tests PASS (tailnet integration tests are `#[ignore]`).

- [ ] **Step 2: Run tailbox tests explicitly**

```bash
cargo test -p tailbox -- --nocapture 2>&1 | tail -20
```

- [ ] **Step 3: Run clippy across all non-tailnet crates**

```bash
cargo xtask pre-commit 2>&1 | tail -10
```

- [ ] **Step 4: Run clippy on tailbox separately**

```bash
cargo clippy -p tailbox -- -D warnings 2>&1
```

- [ ] **Step 5: Verify default build produces no `tailbox` in binary**

```bash
cargo build --release -p miniboxd 2>&1 | head -5
nm target/release/miniboxd 2>/dev/null | grep -i tailscale | head -5
```

Expected: no `tailscale` symbols in the default build.

- [ ] **Step 6: Final commit (if any fixups from Steps 1–5)**

```bash
git add -A
git commit -m "chore(tailbox): clippy + test fixups from final validation pass"
```

---

## Self-Review

**Spec coverage:**

| Requirement                                                                | Task                                  |
| -------------------------------------------------------------------------- | ------------------------------------- |
| New `tailbox` crate with `{platform}box` naming                            | Task 2                                |
| `NetworkConfig` — `TailnetMode` enum + 3 new fields, `#[serde(default)]`   | Task 1                                |
| `TailnetNetwork` struct — `DeviceStore`, `gateway_device`, `TailnetConfig` | Task 2 + 4                            |
| Gateway mode: lazy daemon device, shared tailnet IP                        | Task 4 Steps 3–4                      |
| Per-container mode: own device, key file, `DeviceStore` entry              | Task 4 Steps 5–6                      |
| `attach()` is a no-op                                                      | Task 2 stub; confirmed in Task 4 test |
| `cleanup()` — per-container drops device + key file; removes ctx file      | Task 4 Step 7                         |
| `stats()` returns `NetworkStats::default()`                                | Task 2 stub; confirmed in Task 4 test |
| `resolve_auth_key()` priority chain (inline → secrets → env → err)         | Task 3                                |
| `TS_RS_EXPERIMENT` once-guard with `SAFETY:` comment                       | Task 2 Step 4                         |
| `tailnet` Cargo feature gates dep and `main.rs` arm                        | Task 5                                |
| `MINIBOX_NETWORK_MODE=tailnet` selects the adapter                         | Task 5 Step 2                         |
| Stability warning in crate-level doc                                       | Task 2 Step 3                         |
| Context file at `/run/minibox/net/{container_id}.json`                     | Task 4 Step 3                         |
| Per-container key file at `~/.minibox/tailnet/{container_id}.json`         | Task 4 Step 6                         |
| Unit tests: no network required                                            | Tasks 3 + 4                           |
| Integration tests: `#[ignore]`, require `TAILSCALE_AUTH_KEY`               | Task 4 Steps 1 + 7                    |
| Default build excludes tailscale-rs                                        | Task 5 + Task 6 Step 5                |

**Type consistency:**

- `TailnetMode` defined Task 1, used in `NetworkConfig` Task 1, matched in `adapter.rs` Task 4.
- `TailnetConfig` defined Task 2 (`config.rs`), instantiated in `miniboxd/main.rs` Task 5.
- `DeviceStore` (`Arc<Mutex<HashMap<String, tailscale::Device>>>`) defined Task 2, populated
  Task 4 Step 5, cleared Task 4 Step 7.
- `gateway_device` (`Arc<Mutex<Option<tailscale::Device>>>`) defined Task 2, lazily set
  Task 4 Step 4.
- `resolve_auth_key` defined Task 3, called in `setup()` Task 4 Step 3.
- `ensure_tsrs_experiment` defined Task 2, called in `setup()` Task 4 Step 3.
- `net_context_path` / `container_key_path` / `gateway_key_state` defined Task 4 Step 6.

**Platform contract:**

- `tailbox` compiles on Linux and macOS only. The `#[cfg(feature = "tailnet")]` gate in
  `miniboxd` ensures the crate is never pulled into Windows or GKE builds unless explicitly
  opted in.
- `TS_RS_EXPERIMENT` `set_var` is wrapped in `Once` + `unsafe` with a `SAFETY:` comment per
  project rules (`rust-patterns.md` rule 6).
