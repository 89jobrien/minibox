# Container Networking Design Spec

**Date:** 2026-03-23
**Status:** Draft
**Crate:** linuxbox (domain + adapters), daemonbox (handler wiring)

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
linuxbox/src/adapters/
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

## Tailnet Mode: tailscaled Integration Strategies

Four strategies, ordered by complexity. Based on Tailscale's official userspace networking docs and the "four ways to put a service on your tailnet" architecture guide.

### Strategy A: Host-side SOCKS5/HTTP proxy (simplest, recommended first)

Run `tailscaled` on the **host** side in userspace networking mode. Container processes connect via proxy environment variables.

**Host side:**
```bash
tailscaled --tun=userspace-networking \
           --socks5-server=0.0.0.0:1055 \
           --outbound-http-proxy-listen=0.0.0.0:1055 &
tailscale up --login-server=https://<headscale> --authkey=<key>
```

**Container side** (injected as env vars by minibox):
```bash
ALL_PROXY=socks5://<host-ip>:1055/
HTTP_PROXY=http://<host-ip>:1055/
http_proxy=http://<host-ip>:1055/
```

**Pros:** No binary injection, no TUN device, no bind mounts needed. Works today.
**Cons:** TCP-only via SOCKS5 (no UDP, no ICMP/ping). All containers share one tailnet identity. Known issue: SOCKS5 proxy doesn't dial non-tailnet addresses in some configurations (tailscale/tailscale#1617). IPv6-only containers have DERP registration issues (#19069).

### Strategy B: Sidecar pattern (one tailscaled per container)

Run a dedicated `tailscaled` process per container in its network namespace. Each container gets its own tailnet IP and MagicDNS name. This is Tailscale's officially recommended approach for containerized services.

**Per container:**
```bash
# Inside the container's network namespace
tailscaled --tun=userspace-networking \
           --state=mem: \
           --socket=/tmp/tailscaled.sock \
           --socks5-server=localhost:1055 &
tailscale --socket=/tmp/tailscaled.sock up \
          --login-server=https://<headscale> \
          --authkey=<ephemeral-preauth-key> \
          --hostname=mbx-<container-id>
```

**Pros:** Each container is a distinct tailnet node with its own IP, ACLs, and DNS name. Clean deregistration on stop.
**Cons:** Requires `tailscaled`/`tailscale` binaries inside the container. Two approaches:
  - Bind-mount from host (requires bind mount support in minibox)
  - Use `tailscale/tailscale` base image or multi-stage image with tailscale pre-installed

### Strategy C: Bind-mount injection

Mount the host's `tailscaled` and `tailscale` binaries into the container rootfs read-only:
- `/usr/local/bin/tailscaled` → bind mount
- `/usr/local/bin/tailscale` → bind mount
- `/var/lib/tailscale/` → per-container tmpfs for state

Depends on bind mount support landing in minibox.

### Strategy D: Embed tsnet via libtailscale FFI (target architecture)

Embed Tailscale directly into the minibox daemon process via `libtailscale`, the official C library wrapping `tsnet`. Each container gets its own embedded tailnet node — no separate `tailscaled` binary needed anywhere.

**Crate:** [`libtailscale`](https://crates.io/crates/libtailscale) v0.2.2 (Feb 2026) — Rust FFI bindings by messense. Wraps the [official `tailscale/libtailscale`](https://github.com/tailscale/libtailscale) C API (BSD-3-Clause, actively maintained, last commit Feb 27 2026).

**Build requirement:** Go 1.20+ toolchain at compile time. `libtailscale-sys` invokes CGo to build the Go source into a static archive (`libtailscale.a`), which is then linked into the Rust binary.

**How it works:**
```rust
// In the daemon, per container:
use libtailscale::Tailscale;

let ts = Tailscale::new();
ts.set_hostname(&format!("mbx-{}", container_id));
ts.set_auth_key(&preauth_key);           // ephemeral key from Headscale API
ts.set_control_url(&headscale_url);       // point to Headscale, not Tailscale SaaS
ts.set_dir(&format!("/tmp/ts-{}", container_id)); // per-container state
ts.start();

// ts now has a tailnet IP; use ts.listen() / ts.dial() for connections
// or get the SOCKS5 proxy fd for the container to use
let listener = ts.listen("tcp", ":80")?;  // listen on tailnet
let conn = ts.dial("tcp", "other-node:5432")?; // dial tailnet peers
```

**Architecture:** The daemon manages a pool of `Tailscale` instances (one per tailnet-mode container). Each instance is a lightweight embedded tsnet server with its own identity, IP, and MagicDNS name. No TUN device, no sidecar process, no bind mounts.

**Pros:**
- Zero external dependencies at runtime (no `tailscaled` binary)
- Per-container tailnet identity with full `listen()`/`dial()` API
- MagicDNS and ACLs work natively
- Clean lifecycle: `start()` on container create, `close()` on container stop
- Headscale-compatible via `set_control_url()`

**Cons:**
- Requires Go toolchain at build time (CGo)
- Adds ~15MB to daemon binary (embedded Go runtime)
- `libtailscale` Rust crate fails to build on docs.rs (CGo dependency) — must vendor or build locally
- All `Tailscale` instances share the daemon process; crash isolation is weaker than sidecar
- Go runtime GC runs inside the Rust process (memory accounting, potential latency spikes)

**Container-side integration options:**
1. **Proxy injection** — daemon runs SOCKS5 proxy per container, injects `ALL_PROXY` env var
2. **FD passing** — daemon `dial()`s tailnet peers on behalf of the container, passes connected fds
3. **Network namespace wiring** — daemon's embedded tsnet writes to a socketpair or veth that terminates in the container's netns

Option 1 is simplest. Option 3 is the cleanest for transparent networking.

**Recommendation:** Strategy D is the target architecture. Start with Strategy A (host-side proxy) for prototyping, skip B/C, go directly to D once the `libtailscale` build pipeline is sorted. The Rust crate exists and is current.

### Known Limitations (from web research)

- **SOCKS5 proxy scope** — In userspace mode, the SOCKS5 proxy only dials tailnet addresses by default. Non-tailnet (internet) traffic may not route through it depending on exit node configuration (tailscale/tailscale#1617).
- **No UDP/ICMP** — Userspace SOCKS5 is TCP-only. `ping` won't work. DNS queries must go through the HTTP proxy or a configured resolver.
- **IPv6-only** — Containers with only IPv6 connectivity have issues with DERP peer registration in SOCKS5 mode (tailscale/tailscale#19069).
- **State management** — `--state=mem:` means tailscale state is lost on restart. For ephemeral containers this is fine. For long-lived containers, mount a state directory.

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

### Phase 3: Headscale client + host-side proxy (Strategy A)

- Implement `HeadscaleClient` REST wrapper (pre-auth keys, node CRUD, health)
- Credential flow via minibox-secrets (`op://minibox/headscale/api-key`)
- Implement `TailnetNetwork` adapter using host-side `tailscaled` SOCKS5 proxy
- Inject `ALL_PROXY`/`HTTP_PROXY` env vars into container processes
- CLI `--network tailnet --tag web`
- Integration tests against live Headscale instance

### Phase 4: Embedded tsnet via libtailscale (Strategy D — target)

- Add `libtailscale` + `libtailscale-sys` to workspace deps (requires Go toolchain in CI)
- Build pipeline: `mise` task to ensure Go is available, CGo builds `libtailscale.a`
- Implement `EmbeddedTailnetNetwork` adapter: per-container `Tailscale` instance in daemon
- SOCKS5 proxy per container (Option 1) or netns wiring (Option 3)
- Replace host-side proxy with embedded instances
- Per-container MagicDNS hostname (`mbx-<id>.<tailnet>`)
- ACL template management via `PUT /api/v1/policy`
- Lifecycle: `ts.start()` on create, `ts.close()` on stop, Headscale deregistration as fallback
- Bench: measure connection setup latency, Go GC impact on daemon

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
- `libtailscale` v0.2.2 + `libtailscale-sys` v0.2.2 — Rust FFI to embedded tsnet (Phase 4)
- Go 1.20+ toolchain — build-time dependency for CGo compilation of `libtailscale.a` (Phase 4)
- `mise` Go toolchain task — ensure Go is available in CI and local dev

## Open Questions

1. Should `NetworkMode` be per-adapter-suite or global? (e.g., Colima adapter suite might handle networking differently via limactl)
2. Should the bridge be per-daemon or per-network? (Docker supports multiple bridge networks)
3. Should tailnet containers get a dedicated Headscale user (`minibox@`) or inherit the daemon's user?
4. How should network stats integrate with the bench harness?
5. For Strategy A (shared proxy), how do we handle the SOCKS5 non-tailnet routing limitation? Force exit node config?
6. Should minibox manage `tailscaled` lifecycle on the host, or require it to be pre-running?

## References

- [Tailscale userspace networking docs](https://tailscale.com/docs/concepts/userspace-networking) — `--tun=userspace-networking`, SOCKS5/HTTP proxy setup, env vars
- [Tailscale ephemeral nodes](https://tailscale.com/docs/features/ephemeral-nodes) — auto-cleanup for short-lived containers
- [Four ways to put a service on your tailnet](https://tailscale.com/blog/four-ways-tailscale-service) — sidecar pattern (Strategy B), tsnet (Strategy D)
- [Headscale REST API](https://headscale.net/stable/ref/api/) — `/api/v1` endpoints, Bearer auth, Swagger docs
- [Headscale ACLs](https://headscale.net/stable/ref/acls/) — huJSON policy format, autogroups, tag ownership
- [Headscale pre-auth keys](https://headscale.net/stable/usage/getting-started/) — `headscale preauthkeys create --user <USER> --ephemeral`
- [tailscale/tailscale#1617](https://github.com/tailscale/tailscale/issues/1617) — SOCKS5 proxy doesn't dial non-tailnet addresses
- [tailscale/tailscale#19069](https://github.com/tailscale/tailscale/issues/19069) — SOCKS5 fails in IPv6-only containers
- [Docker networking internals (veth + bridge)](https://oneuptime.com/blog/post/2026-02-08-how-to-understand-docker-networking-internals-veth-pairs-bridges/view) — reference for Bridge mode implementation
- [Rust container networking with namespaces](https://www.kungfudev.com/blog/2023/12/21/simplified-networking-crafting-isolated-echo-server-in-rust) — netlink + veth in Rust
- [`libtailscale` Rust crate](https://crates.io/crates/libtailscale) v0.2.2 — Rust FFI bindings to `tailscale/libtailscale` C API
- [`tailscale/libtailscale`](https://github.com/tailscale/libtailscale) — Official C library wrapping tsnet (BSD-3-Clause)
- [`messense/libtailscale-rs`](https://github.com/messense/libtailscale-rs) — Rust crate source (Go 1.20+ build dep)
- `.firecrawl/headscale-research.json` — full structured Headscale API extraction (local)
