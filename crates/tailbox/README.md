# tailbox

Tailscale-rs network adapter for minibox containers.

Implements `minibox_core::domain::NetworkProvider` via `TailnetNetwork`, giving containers
access to Tailscale networks (tailnets) without a full Tailscale install inside the container.

## Modes

| Mode           | Description                                                                   |
| -------------- | ----------------------------------------------------------------------------- |
| `Gateway`      | One shared `tailscale::Device` for the daemon. Containers reach tailnet peers |
|                | via proxy connections through that device.                                    |
| `PerContainer` | Each container gets its own `tailscale::Device` and tailnet IP.               |

## Platform support

Linux and macOS ARM64 only, matching `tailscale-rs` v0.2 platform support. The
`tailscale` dependency is gated with `cfg(any(target_os = "linux", target_os = "macos"))`.

## Configuration

```rust
use tailbox::TailnetConfig;

let config = TailnetConfig {
    // Inline auth key — if None, looks up `key_secret_name` in minibox-secrets,
    // then falls back to the `TAILSCALE_AUTH_KEY` env var.
    auth_key: None,
    key_secret_name: "tailscale-auth-key".to_string(),
};
```

## Usage in miniboxd

Wire `TailnetNetwork` into `HandlerDependencies` behind the `tailnet` feature flag:

```toml
[features]
tailnet = ["tailbox"]
```

```rust
#[cfg(feature = "tailnet")]
let network = tailbox::TailnetNetwork::new(TailnetConfig::default()).await?;
```

## Modules

| Module       | Description                                         |
| ------------ | --------------------------------------------------- |
| `adapter`    | `TailnetNetwork` — the `NetworkProvider` adapter    |
| `auth`       | Auth key resolution and device authentication       |
| `config`     | `TailnetConfig` — runtime configuration             |
| `experiment` | Feature-gated experimental networking extensions    |
