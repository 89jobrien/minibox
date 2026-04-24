# Quick Wins Design Spec

# Container Freeze, Events, Image GC + Leases, Bridge Networking

**Date:** 2026-04-02
**Status:** approved

---

## Overview

Four independent improvements derived from a containerd gap analysis. Each is
self-contained and can be committed individually. Ordered from smallest to
largest:

1. **Freeze/Pause** — write `cgroup.freeze`, add `Paused` state, CLI commands
2. **Container Events** — pub/sub broker, dashbox integration, `minibox events` tail
3. **Image GC + Leases** — reference-counted prune, `minibox prune`, lease protection
4. **Bridge Networking** — veth pair + NAT, `BridgeNetwork` adapter, port mappings

---

## Phase 1: Container Freeze / Pause (XS)

### Problem

No way to pause a running container. `cgroup.freeze` exists in kernel but is unwired.

### Design

Add `pause()` / `resume()` to `CgroupManager` (`crates/minibox/src/container/cgroups.rs`).
Write `"1"` to `{cgroup_path}/cgroup.freeze` for pause, `"0"` for resume.

New `ContainerState::Paused` variant (between `Running` and `Stopped`).

New protocol variants:

- `DaemonRequest::PauseContainer { id: String }`
- `DaemonRequest::ResumeContainer { id: String }`
- `DaemonResponse::ContainerPaused { id: String }`
- `DaemonResponse::ContainerResumed { id: String }`

New handler arms: `handle_pause`, `handle_resume` — look up container, validate
state (`Running` → `Paused`, `Paused` → `Running`), call `CgroupManager`,
update state.

New CLI subcommands: `minibox pause <id>`, `minibox resume <id>`.

**macOS:** `CgroupManager` is Linux-only. `handle_pause`/`handle_resume`
return `DomainError::InvalidConfig("pause not supported on this platform")` when
no cgroup path is available. CLI prints error and exits 1.

### State machine addition

```
Created → Running → Paused → Running → Stopped
```

`update_container_state` must allow `Running → Paused` and `Paused → Running`
transitions.

---

## Phase 2: Container Events (S)

### Problem

No observability. Handler fires and forgets. Dashbox polls state; can't react.

### Design

New file: `crates/minibox-core/src/events.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContainerEvent {
    Created   { id: String, image: String, timestamp: SystemTime },
    Started   { id: String, pid: u32,      timestamp: SystemTime },
    Stopped   { id: String, exit_code: i32, timestamp: SystemTime },
    Paused    { id: String,                 timestamp: SystemTime },
    Resumed   { id: String,                 timestamp: SystemTime },
    OomKilled { id: String,                 timestamp: SystemTime },
    ImagePulled  { image: String, size_bytes: u64, timestamp: SystemTime },
    ImageRemoved { image: String,                  timestamp: SystemTime },
    ImagePruned  { count: usize,  freed_bytes: u64, timestamp: SystemTime },
}

/// Port: emit events (handlers depend on this)
pub trait EventSink: Send + Sync {
    fn emit(&self, event: ContainerEvent);
}

/// Port: subscribe to event stream (dashbox / CLI depend on this)
pub trait EventSource: Send + Sync {
    fn subscribe(&self) -> broadcast::Receiver<ContainerEvent>;
}

/// Adapter: tokio broadcast channel, implements both ports
pub struct BroadcastEventBroker {
    tx: broadcast::Sender<ContainerEvent>,
}
```

ISP: `HandlerDependencies` gains `event_sink: Arc<dyn EventSink>`.
Dashbox and CLI only see `Arc<dyn EventSource>`.
`BroadcastEventBroker` implements both — injected at composition root.

New protocol variant: `DaemonRequest::SubscribeEvents`
New protocol variant: `DaemonResponse::Event { event: ContainerEvent }`
(non-terminal, like `ContainerOutput`)

New CLI: `minibox events` — streams JSON-lines to stdout until Ctrl+C.

### Event emission points

- `handle_run` after spawn → `Started`
- `daemon_wait_for_exit` on exit → `Stopped` or `OomKilled`
- `handle_run` on create → `Created`
- `handle_pause` → `Paused`
- `handle_resume` → `Resumed`
- `handle_pull` on completion → `ImagePulled`
- `handle_prune` on completion → `ImagePruned`

---

## Phase 3: Image GC + Leases (M)

### Problem

Disk fills up. No way to remove unused images. No protection for in-flight pulls.

### Design

#### Leases

New file: `crates/minibox-core/src/image/lease.rs`

```rust
pub struct LeaseRecord {
    pub id:         String,
    pub created_at: SystemTime,
    pub expire_at:  SystemTime,
    pub image_refs: HashSet<String>,  // "name:tag" strings protected by this lease
}

pub trait ImageLeaseService: Send + Sync {
    async fn acquire(&self, image_ref: &str, ttl: Duration) -> Result<String>; // returns lease id
    async fn release(&self, lease_id: &str) -> Result<()>;
    async fn extend(&self, lease_id: &str, ttl: Duration) -> Result<()>;
    async fn list(&self) -> Result<Vec<LeaseRecord>>;
}

pub struct DiskLeaseService {
    leases:    Arc<RwLock<HashMap<String, LeaseRecord>>>,
    leases_path: PathBuf,  // {data_dir}/leases.json
}
```

Leases persisted to `leases.json` (same dir as `state.json`). Atomic write
(temp + rename). Loaded on daemon start.

`handle_pull` acquires a lease before pulling, releases on success. On failure,
lease expires naturally (default TTL: 1 hour).

#### GC

New file: `crates/minibox-core/src/image/gc.rs`

```rust
pub trait ImageGarbageCollector: Send + Sync {
    async fn prune(&self, dry_run: bool) -> Result<PruneReport>;
}

pub struct PruneReport {
    pub removed:    Vec<String>,  // image refs deleted
    pub freed_bytes: u64,
    pub dry_run:    bool,
}

pub struct ImageGc {
    store:         Arc<ImageStore>,
    lease_service: Arc<dyn ImageLeaseService>,
    state:         Arc<DaemonState>,
}
```

GC algorithm:

1. List all `name:tag` in `ImageStore` (walk `{data_dir}/images/`)
2. Collect `in_use`: all `source_image_ref` from running/paused containers
3. Collect `leased`: all `image_refs` from non-expired leases
4. `candidates = all - in_use - leased`
5. For each candidate: sum layer dir sizes, delete manifest + layers dir
6. Emit `ImagePruned` event

New protocol variants:

- `DaemonRequest::Prune { dry_run: bool }`
- `DaemonResponse::Pruned { report: PruneReport }`

New CLI: `minibox prune [--dry-run]`

Also: `minibox rmi <image>` — remove a specific image (if not in use).

---

## Phase 4: Bridge Networking (M)

### Problem

`NetworkMode::Bridge` is defined but no adapter exists. Containers get isolated
network namespace with no connectivity.

### Design

New file: `crates/minibox/src/adapters/network/bridge.rs` (Linux-only)

```rust
#[cfg(target_os = "linux")]
pub struct BridgeNetwork {
    bridge_name: String,        // default: "minibox0"
    subnet:      ipnet::IpNet,  // default: 172.20.0.0/16
    ip_alloc:    Arc<Mutex<IpAllocator>>,
    dns_servers: Vec<IpAddr>,
}
```

#### setup() flow

1. Ensure `minibox0` bridge exists (`ip link add minibox0 type bridge` via `rtnetlink` crate or subprocess fallback)
2. Ensure bridge has an IP on the subnet (`ip addr add 172.20.0.1/16 dev minibox0`)
3. Ensure bridge is up (`ip link set minibox0 up`)
4. Enable IP forwarding (`/proc/sys/net/ipv4/ip_forward = 1`)
5. Allocate container IP from `IpAllocator`
6. Create veth pair: `veth-{id_prefix}` (host) + `eth0` (peer, goes into container)
7. Attach host veth to bridge
8. Return namespace path (written by caller after `clone(CLONE_NEWNET)`)

#### attach() flow (called after container PID is known)

1. Move peer veth into container's network namespace: `ip link set eth0 netns {pid}`
2. Inside namespace (via `nsenter` or `setns`):
   - Assign IP to `eth0`
   - Set `eth0` up
   - Add default route via bridge IP
   - Write `/etc/resolv.conf` with DNS servers

#### cleanup() flow

1. Delete veth pair (bridge stays — shared resource)
2. Delete iptables DNAT rules for port mappings
3. Release IP back to allocator

#### Port mapping

For each `PortMapping` in `NetworkConfig`:

```
iptables -t nat -A PREROUTING -p {proto} --dport {host_port} \
  -j DNAT --to-destination {container_ip}:{container_port}
iptables -t nat -A POSTROUTING -s 172.20.0.0/16 -j MASQUERADE
```

#### IpAllocator

Sequential allocation from subnet, skipping `.0` (network) and `.1` (gateway).
Persisted in `DaemonState` (new field: `allocated_ips: HashMap<String, IpAddr>`).
Released on container stop/cleanup.

#### Dependencies

- `rtnetlink` crate for netlink socket operations (or `nix` raw socket + subprocess fallback)
- `ipnet` crate for CIDR parsing
- `iptables` subprocess for NAT rules (spawn `iptables` binary)

**macOS / non-Linux:** `BridgeNetwork` gated with `#[cfg(target_os = "linux")]`.
`NetworkMode::Bridge` on macOS → `DomainError::InvalidConfig("bridge networking requires Linux")`.

### Wiring

`miniboxd` native suite: if `config.network.mode == Bridge`, construct `BridgeNetwork`.
`daemonbox/src/handler.rs`: pass `network_provider` through to `spawn_process`.

---

## Testing Strategy

| Feature | Unit test                  | Integration test             | Platform                   |
| ------- | -------------------------- | ---------------------------- | -------------------------- |
| Freeze  | Mock cgroup writes         | Real cgroup (Linux+root)     | Linux only for integration |
| Events  | MockEventSink counts calls | CLI `minibox events` streams | any                        |
| GC      | In-memory store + lease    | Real image store on tmpfs    | any                        |
| Bridge  | Subprocess mock            | Real veth/bridge creation    | Linux+root                 |

All unit tests: `cargo xtask test-unit`
Integration: `just test-integration` (Linux+root)

---

## Files touched

| File                                            | Change                                           |
| ----------------------------------------------- | ------------------------------------------------ |
| `crates/minibox-core/src/events.rs`             | **CREATE** — event types + traits                |
| `crates/minibox-core/src/image/lease.rs`        | **CREATE** — lease service                       |
| `crates/minibox-core/src/image/gc.rs`           | **CREATE** — GC + prune                          |
| `crates/minibox/src/adapters/network/bridge.rs` | **CREATE** — bridge adapter                      |
| `crates/minibox-core/src/lib.rs`                | mod events                                       |
| `crates/minibox-core/src/domain.rs`             | EventSink/EventSource type aliases               |
| `crates/minibox-core/src/image/mod.rs`          | list_all_images(), delete_image()                |
| `crates/minibox-core/src/protocol.rs`           | Pause/Resume/Prune/Events variants               |
| `crates/minibox/src/protocol.rs`                | mirror changes                                   |
| `crates/minibox/src/container/cgroups.rs`       | pause(), resume()                                |
| `crates/minibox/src/adapters/network/mod.rs`    | re-export BridgeNetwork                          |
| `crates/daemonbox/src/handler.rs`               | handle_pause/resume/prune/events, event_sink dep |
| `crates/daemonbox/src/server.rs`                | dispatch arms, Event as non-terminal             |
| `crates/daemonbox/src/state.rs`                 | Paused state, allocated_ips                      |
| `crates/miniboxd/src/main.rs`                   | wire BroadcastEventBroker, BridgeNetwork         |
| `crates/minibox-cli/src/commands/`              | pause.rs, resume.rs, events.rs, prune.rs, rmi.rs |
| `crates/minibox-cli/src/main.rs`                | register new subcommands                         |
