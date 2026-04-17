# Tailnet Integration Design

**Date:** 2026-04-17
**Status:** Approved

## Overview

Integrate `tailscale-rs` to implement `NetworkMode::Tailnet`, giving minibox containers first-class
Tailscale networking. Two modes compose:

- **Gateway mode** — the daemon joins the tailnet once; containers can reach tailnet peers via
  explicit TCP/UDP socket connections proxied through the daemon device (not transparent kernel
  routing — see Caveats).
- **Per-container mode** — each container gets its own `tailscale::Device` and tailnet identity
  (its own tailnet IP, visible as a node on the tailnet).

Both modes are supported simultaneously. Which mode a container uses is determined by
`NetworkConfig::tailnet_mode` at `RunContainer` time.

The entire feature is gated behind a `tailnet` Cargo feature flag so it is opt-in and does not
pull `tailscale-rs` (or its crypto stack) into default builds.

## Architecture

### New crate: `crates/tailbox`

Follows the `{platform}box` naming convention. Contains the `TailnetNetwork` adapter and all
tailscale-rs wiring. Compiles on Linux and macOS ARM64 (the platforms tailscale-rs supports).

```
miniboxd (main.rs)
  └── #[cfg(feature = "tailnet")] tailbox
        ├── TailnetNetwork     — NetworkProvider impl
        ├── DeviceStore        — Arc<Mutex<HashMap<container_id, Device>>>
        ├── TailnetConfig      — auth_key, mode, key_secret_name
        └── resolve_auth_key() — key resolution chain
```

### Dependency chain

`tailbox/Cargo.toml`:
```toml
[dependencies]
tailscale = "0.2"
minibox-core = { path = "../minibox-core" }
minibox-secrets = { path = "../minibox-secrets" }
anyhow = "1"
async-trait = "0.1"
tokio = { version = "1", features = ["sync"] }
tracing = "0.1"
serde_json = "1"
```

`miniboxd/Cargo.toml`:
```toml
[features]
tailnet = ["dep:tailbox"]

[dependencies]
tailbox = { path = "../tailbox", optional = true }
```

Adapter selection in `miniboxd/src/main.rs`:
```rust
#[cfg(feature = "tailnet")]
"tailnet" => Arc::new(tailbox::TailnetNetwork::new(tailnet_config).await?),
```

## TailnetNetwork Adapter

```rust
pub struct TailnetNetwork {
    config: TailnetConfig,
    devices: Arc<Mutex<HashMap<String, tailscale::Device>>>,
    gateway_device: Arc<Mutex<Option<tailscale::Device>>>,
}
```

### `TailnetConfig`

```rust
pub struct TailnetConfig {
    /// Default auth key for daemon gateway device.
    pub auth_key: Option<String>,
    /// minibox-secrets key name to look up if auth_key is None.
    /// Defaults to "tailscale-auth-key".
    pub key_secret_name: String,
}
```

### `setup(container_id, config) -> Result<String>`

1. Set `TS_RS_EXPERIMENT=this_is_unstable_software` via `Once`-guarded `unsafe set_var`.
2. Resolve auth key via `resolve_auth_key()` (see below).
3. **Gateway mode** (`config.tailnet_mode == TailnetMode::Gateway`):
   - Lazily start daemon `Device` if not already running (stored in `gateway_device`).
   - Allocate an internal routing entry (container_id → tailnet addr mapping).
   - Return JSON context: `{ "mode": "gateway", "tailnet_ip": "<daemon tailnet ipv4>",
     "container_id": "..." }`.
4. **Per-container mode** (`config.tailnet_mode == TailnetMode::PerContainer`):
   - Call `tailscale::Device::new(&tailscale::Config { key_state: load_key_file(...), ..Default::default() }, Some(auth_key)).await?`.
   - Key file path: `~/.mbx/tailnet/{container_id}.json` (created per-container).
   - Store device in `DeviceStore`.
   - Return JSON context: `{ "mode": "per_container", "tailnet_ip": "<device ipv4>",
     "container_id": "..." }`.
5. Write context to `/run/minibox/net/{container_id}.json` (same path as `BridgeNetwork`).

### `attach(container_id, pid) -> Result<()>`

No-op. Returns `Ok(())`. tailscale-rs devices are not pid-namespace-based.

### `cleanup(container_id) -> Result<()>`

- Per-container: remove device from `DeviceStore` (drop triggers teardown). Delete key file
  at `~/.mbx/tailnet/{container_id}.json`.
- Gateway: no-op — daemon device persists across container lifecycle.
- Remove `/run/minibox/net/{container_id}.json` (best-effort, warn on error).

### `stats(container_id) -> Result<NetworkStats>`

Returns `NetworkStats::default()`. tailscale-rs has no stats API in v0.2.

## Auth Key Resolution

`TailnetNetwork::resolve_auth_key(config: &NetworkConfig) -> Result<String>`:

1. `config.tailnet_auth_key` — inline key passed in the `RunContainer` request.
2. minibox-secrets lookup by `config.tailnet_secret_name` (if set) or
   `TailnetConfig::key_secret_name` (default: `"tailscale-auth-key"`).
3. `TAILSCALE_AUTH_KEY` environment variable.
4. `Err(...)` — no key available; surface a clear error to the caller.

## Protocol Changes

Two new `#[serde(default)]` fields on `NetworkConfig` in
`crates/minibox-core/src/domain/networking.rs`:

```rust
/// Inline Tailscale auth key (per-container mode). Takes precedence over secret lookup.
#[serde(default)]
pub tailnet_auth_key: Option<String>,

/// minibox-secrets key name for Tailscale auth key lookup.
/// Defaults to "tailscale-auth-key" if None.
#[serde(default)]
pub tailnet_secret_name: Option<String>,

/// Tailnet networking mode (gateway or per-container).
#[serde(default)]
pub tailnet_mode: TailnetMode,
```

New enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TailnetMode {
    #[default]
    Gateway,
    PerContainer,
}
```

`NetworkMode::Tailnet` already exists — no enum change needed.

## `TS_RS_EXPERIMENT` Handling

```rust
static TSRS_EXPERIMENT_SET: std::sync::Once = std::sync::Once::new();

fn ensure_tsrs_experiment() {
    TSRS_EXPERIMENT_SET.call_once(|| {
        // SAFETY: called once at daemon startup before any threads read this var.
        // tailscale-rs requires this env var to acknowledge its unstable status.
        unsafe {
            std::env::set_var("TS_RS_EXPERIMENT", "this_is_unstable_software");
        }
    });
}
```

Called at the top of `TailnetNetwork::setup()`.

## Feature Flag Build

Default build (no tailnet):
```bash
cargo build --release
```

With tailnet support:
```bash
cargo build --release --features tailnet
```

CI: `tailnet` feature is NOT included in `cargo xtask pre-commit` or `cargo xtask test-unit` by
default. A separate CI job or manual test covers it:
```bash
cargo build -p tailbox
cargo clippy -p tailbox -- -D warnings
```

## Testing Strategy

**Unit tests** (no tailscale auth required, no network):
- `TailnetNetwork::attach()` is a no-op — trivially passes.
- `resolve_auth_key()` priority chain — tested with mock env vars and a mock secrets provider.
- `setup()` in gateway mode with a mock `Device` (behind a test trait shim, or skipped/ignored).
- Context JSON written to tempdir — verified structure.

**Integration tests** (require auth key, marked `#[ignore]`):
```rust
#[cfg(all(test, feature = "tailnet"))]
#[tokio::test]
#[ignore = "requires TAILSCALE_AUTH_KEY and network access"]
async fn test_per_container_device_joins_tailnet() { ... }
```

**Excluded from `cargo xtask test-unit`** — integration tests are not run in default CI.

## Stability Caveat (doc comment on `TailnetNetwork`)

```
/// **Stability warning**: This adapter wraps `tailscale-rs` v0.2, which is pre-1.0,
/// contains unaudited cryptography, and has no backwards-compatibility guarantees.
/// Do not use in production until tailscale-rs completes a third-party security audit.
/// See: https://github.com/tailscale/tailscale-rs#caveats
```

## Caveats and Non-Goals

- **No direct connections**: tailscale-rs v0.2 routes all traffic via DERP relays. NAT
  traversal is on their roadmap.
- **No MagicDNS**: peer lookup by hostname is unsupported in v0.2.
- **No stats**: `NetworkStats` returns zeros — tailscale-rs exposes no counters yet.
- **Gateway routing not kernel-level**: gateway mode uses tailscale-rs TCP/UDP socket APIs,
  not a TUN interface — containers reach tailnet peers via explicit proxy connections, not
  transparent kernel routing. Full TUN-based transparent routing is a future enhancement
  (`ts_transport_tun` crate exists upstream but is not yet stable).
- **Windows not supported**: tailscale-rs does not support Windows; `tailbox` is
  `#[cfg(any(target_os = "linux", target_os = "macos"))]`.
