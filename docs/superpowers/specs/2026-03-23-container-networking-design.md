# Container Networking Design Spec

**Date:** 2026-03-23
**Status:** Draft
**Crate:** minibox-lib (domain + adapters), daemonbox (handler wiring)

## Problem

Minibox containers get `CLONE_NEWNET` isolation but no actual connectivity. The network namespace is empty — no veth, no bridge, no routes. This blocks:

- Container-to-container communication
- Container-to-host communication
- Cross-host container mesh (tailnet)
- Port forwarding / service exposure
- DNS resolution inside containers

The existing `NetworkProvider` trait in `domain/networking.rs` assumes bridge-only networking. It needs to support multiple network modes including WireGuard mesh via Headscale.

## Goals

1. Extend `NetworkProvider` and `NetworkConfig` to support multiple network modes
2. Define a `NetworkMode` enum: `None`, `Bridge`, `Host`, `Tailnet`
3. Add Headscale REST API client for tailnet container registration
4. Wire networking into the container lifecycle (setup before exec, cleanup on stop)
5. Support ephemeral tailnet nodes that auto-deregister

## Non-Goals

- CNI plugin compatibility (future work)
- Multi-network attachment (one network per container for now)
- IPv6 tailnet (Headscale supports it, but defer)
- DERP relay server embedding
- Slirp4netns / rootless networking

## Network Modes

### `None` (current behavior)

Empty network namespace. Container has loopback only. No changes needed.

### `Bridge` (local veth + bridge)

Traditional Docker-style networking:
- Create `minibox0` bridge on first use
- Allocate IP from `172.18.0.0/16` subnet
- Create veth pair: host end attached to bridge, container end in namespace
- Configure iptables MASQUERADE for outbound NAT
- Optional port mappings via iptables DNAT

This is what the existing `NetworkProvider` trait already describes.

### `Host` (shared namespace)

Container shares the host's network namespace. Skip `CLONE_NEWNET` flag. No veth, no bridge. Container sees all host interfaces.

Requires approval gate in minibox-orch (security escalation).

### `Tailnet` (WireGuard mesh via Headscale)

Container joins a Headscale-coordinated tailnet:
- Daemon creates ephemeral pre-auth key via Headscale REST API
- Container runs `tailscaled` in userspace networking mode (no TUN required)
- Container registers with `tailscale up --login-server=<url> --authkey=<key>`
- Container gets a stable 100.x.y.z IP and MagicDNS hostname (`mbx-<id>.<tailnet>`)
- On container stop, daemon calls `DELETE /api/v1/node/<id>` to deregister
- Ephemeral nodes auto-expire after 30min inactivity as fallback

## Domain Changes

### `NetworkMode` enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkMode {
    /// No networking (loopback only)
    None,
    /// Linux bridge with veth pair
    Bridge,
    /// Share host network namespace
    Host,
    /// WireGuard mesh via Headscale/Tailscale
    Tailnet,
}
```

### `NetworkConfig` changes

```rust
pub struct NetworkConfig {
    pub mode: NetworkMode,

    // Bridge mode fields (existing)
    pub bridge_name: String,
    pub subnet: String,
    pub container_ip: Option<String>,
    pub port_mappings: Vec<PortMapping>,
    pub dns_servers: Vec<String>,
    pub ipv6_enabled: bool,

    // Tailnet mode fields (new)
    pub tailnet: Option<TailnetConfig>,
}

pub struct TailnetConfig {
    /// Headscale server URL (e.g., "https://hs.example.com")
    pub headscale_url: String,

    /// API key for Headscale REST API (from minibox-secrets)
    pub api_key: String,

    /// Headscale user to register containers under
    pub user: String,

    /// Tags to apply to container node (e.g., ["tag:minibox", "tag:web"])
    pub tags: Vec<String>,

    /// Hostname prefix (container ID appended)
    pub hostname_prefix: String,

    /// Use ephemeral pre-auth keys (auto-expire on disconnect)
    pub ephemeral: bool,

    /// Path to tailscaled binary (injected into container or on host)
    pub tailscaled_path: Option<String>,

    /// Use userspace networking (no TUN device required)
    pub userspace_networking: bool,
}
```

### `NetworkProvider` trait update

The existing trait methods (`setup`, `attach`, `cleanup`, `stats`) are sufficient. The change is in what adapters implement them, selected by `NetworkMode`.

## Adapter Architecture

```
minibox-lib/src/adapters/
├── network/
│   ├── mod.rs          # NetworkMode dispatch
│   ├── none.rs         # NoopNetwork (current behavior)
│   ├── bridge.rs       # BridgeNetwork (veth + iptables)
│   ├── host.rs         # HostNetwork (skip CLONE_NEWNET)
│   └── tailnet.rs      # TailnetNetwork (Headscale + tailscaled)
├── headscale.rs        # HeadscaleClient (REST API wrapper)
```

### `HeadscaleClient`

Wraps the Headscale `/api/v1` REST API. Bearer token auth.

```rust
pub struct HeadscaleClient {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

impl HeadscaleClient {
    /// Health check
    pub async fn health(&self) -> Result<()>;

    /// Create ephemeral pre-auth key for a user
    pub async fn create_preauth_key(&self, user: &str, ephemeral: bool, tags: &[String]) -> Result<String>;

    /// List nodes
    pub async fn list_nodes(&self) -> Result<Vec<HeadscaleNode>>;

    /// Delete a node by ID
    pub async fn delete_node(&self, node_id: u64) -> Result<()>;

    /// Get node by hostname
    pub async fn get_node_by_hostname(&self, hostname: &str) -> Result<Option<HeadscaleNode>>;

    /// Set tags on a node
    pub async fn set_tags(&self, node_id: u64, tags: &[String]) -> Result<()>;

    /// Expire a pre-auth key
    pub async fn expire_preauth_key(&self, key_id: &str) -> Result<()>;
}
```

Key endpoints used:
- `POST /api/v1/preauthkey` — create ephemeral key
- `GET /api/v1/node` — list/find nodes
- `DELETE /api/v1/node/{id}` — deregister on stop
- `POST /api/v1/node/{id}/tags` — tag containers
- `GET /api/v1/health` — preflight
- `PUT /api/v1/policy` — optional ACL push

### `TailnetNetwork` adapter

```rust
pub struct TailnetNetwork {
    client: HeadscaleClient,
    config: TailnetConfig,
}

#[async_trait]
impl NetworkProvider for TailnetNetwork {
    async fn setup(&self, container_id: &str, config: &NetworkConfig) -> Result<String> {
        // 1. Create ephemeral pre-auth key via HeadscaleClient
        // 2. Return pre-auth key as the "namespace path"
        //    (container init will use it to join tailnet)
    }

    async fn attach(&self, container_id: &str, pid: u32) -> Result<()> {
        // 1. Start tailscaled inside the container's network namespace
        //    (userspace mode: --tun=userspace-networking)
        // 2. Run: tailscale up --login-server=<url> --authkey=<key>
        //         --hostname=mbx-<container_id>
        // 3. Wait for tailscale to report "connected"
    }

    async fn cleanup(&self, container_id: &str) -> Result<()> {
        // 1. Find node by hostname mbx-<container_id>
        // 2. DELETE /api/v1/node/{id}
        // 3. Ephemeral fallback: node auto-expires after 30min
    }

    async fn stats(&self, container_id: &str) -> Result<NetworkStats> {
        // Query tailscale0 interface stats from /proc/net/dev
        // inside the container's network namespace
    }
}
```

## Container Lifecycle Integration

### Current flow (no networking)

```
RunContainer → create overlay → clone(CLONE_NEWNET|...) → pivot_root → exec
```

### New flow

```
RunContainer
  → create overlay
  → network.setup(id, config)     // create pre-auth key or veth pair
  → clone(flags)                  // CLONE_NEWNET omitted for Host mode
  → pivot_root
  → network.attach(id, pid)       // move veth or start tailscaled
  → exec

StopContainer / reaper
  → network.cleanup(id)           // delete node or remove veth
  → destroy overlay
```

### Handler changes (daemonbox)

`handle_run_streaming` in `handler.rs` needs to:
1. Accept `NetworkConfig` from `RunContainer` request
2. Call `network_provider.setup()` before spawn
3. Call `network_provider.attach()` after spawn returns PID
4. Call `network_provider.cleanup()` in the reaper/stop path

### Protocol changes

Add `network` field to `RunContainer`:

```rust
pub struct RunContainer {
    // ... existing fields ...
    pub network: Option<NetworkConfig>,
}
```

CLI surface:

```
minibox run --network none alpine -- /bin/sh
minibox run --network bridge -p 8080:80 nginx
minibox run --network host alpine -- /bin/sh
minibox run --network tailnet --tag web alpine -- /bin/sh
```

## Tailnet Mode: tailscaled Injection

Two strategies for getting `tailscaled` into the container:

### Strategy A: Bind-mount from host (recommended first)

Mount the host's `tailscaled` and `tailscale` binaries into the container rootfs:
- `/usr/local/bin/tailscaled` → bind mount read-only
- `/usr/local/bin/tailscale` → bind mount read-only

Requires: bind mount support in minibox (prerequisite feature).

### Strategy B: Pre-baked images

Use images that already include tailscale. The `tailscale/tailscale` Docker image works.

### Strategy C: Userspace SOCKS5 proxy (simplest)

Run `tailscaled` on the **host** side with `--socks5-server` and configure the container to use the SOCKS5 proxy. No binary injection needed. Limited to TCP.

**Recommendation:** Start with Strategy C (host-side proxy) for the prototype. Move to Strategy A once bind mounts land.

## Headscale Credential Flow

```
minibox-secrets (CredentialProvider)
  → op:// ref or env var → Headscale API key
  → HeadscaleClient::new(url, key)
  → create_preauth_key(user, ephemeral=true, tags)
  → pass key to container init
```

Environment variables:
- `MINIBOX_HEADSCALE_URL` — Headscale server URL
- `MINIBOX_HEADSCALE_API_KEY` — API key (or `op://minibox/headscale/api-key`)
- `MINIBOX_HEADSCALE_USER` — default user for container registration

## ACL Integration (optional, phase 2)

Headscale ACLs use huJSON. Minibox could manage a policy template:

```json
{
  "tagOwners": {
    "tag:minibox": ["minibox-admin@"]
  },
  "acls": [
    {
      "action": "accept",
      "src": ["tag:minibox"],
      "dst": ["tag:minibox:*"]
    }
  ]
}
```

Push via `PUT /api/v1/policy` on daemon startup. Container tags control access.

## Implementation Phases

### Phase 1: NetworkMode + None/Host adapters

- Add `NetworkMode` enum to domain
- Add `network` field to `RunContainer` protocol
- Implement `NoopNetwork` adapter (current behavior, explicit)
- Implement `HostNetwork` adapter (skip `CLONE_NEWNET`)
- Wire into handler lifecycle
- CLI `--network none|host`
- Tests: unit tests with mock adapters

### Phase 2: Bridge adapter

- Implement `BridgeNetwork` (veth + bridge + iptables)
- IP allocation from subnet
- Port mapping via iptables DNAT
- CLI `--network bridge -p 8080:80`
- Integration tests (Linux + root)

### Phase 3: Headscale client + tailnet adapter

- Implement `HeadscaleClient` REST wrapper
- Implement `TailnetNetwork` adapter (Strategy C: host-side proxy first)
- Credential flow via minibox-secrets
- CLI `--network tailnet --tag web`
- Integration tests against real Headscale instance

### Phase 4: Full tailnet (bind mount injection)

- Bind-mount `tailscaled`/`tailscale` into container
- Container-native tailnet (no SOCKS5 proxy)
- MagicDNS resolution inside containers
- ACL template management

## Security Considerations

- **Headscale API key** must be stored via minibox-secrets, never in config files or CLI args
- **Pre-auth keys** are single-use and ephemeral — leak window is narrow
- **Host mode** is a security escalation — requires approval gate in minibox-orch
- **Bridge mode** iptables rules must be container-scoped, cleaned up on stop
- **Tailnet mode** containers can see other tailnet nodes — ACLs are the access boundary

## Dependencies

- `reqwest` — HTTP client for Headscale API (already in workspace for image pulls)
- `iptables` crate or raw command — bridge mode NAT rules
- minibox-secrets — credential storage for Headscale API key
- Bind mount support — Phase 4 tailscaled injection

## Open Questions

1. Should `NetworkMode` be per-adapter-suite or global? (e.g., Colima adapter suite might handle networking differently)
2. Should the bridge be per-daemon or per-network? (Docker supports multiple bridge networks)
3. Should tailnet containers get a dedicated Headscale user (`minibox@`) or inherit the daemon's user?
4. How should network stats integrate with the bench harness?
