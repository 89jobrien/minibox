# Feature Matrix

Per-platform capability breakdown for minibox adapters.

Last updated: 2026-05-14

---

## Adapter Suites

| Adapter  | Platform                        | Status       | Crate   | Default?                    |
| -------- | ------------------------------- | ------------ | ------- | --------------------------- |
| `native` | Linux x86_64/arm64 (bare metal) | Production   | minibox | Yes on Linux                |
| `gke`    | Linux (GKE unprivileged pods)   | Production   | minibox | --                          |
| `colima` | macOS only (via Lima VM)        | Experimental | minibox | --                          |
| `smolvm` | macOS only (via SmolVM VM)      | Experimental | minibox | Yes on macOS (falls back to `krun`) |
| `krun`   | macOS only (via libkrun VM)     | Experimental | macbox  | macOS fallback when smolvm absent |
| `winbox` | Windows                         | Stub         | winbox  | --                          |

> `native` requires Linux kernel 5.0+, cgroups v2, and root. The `smolvm`/`krun`/`colima`
> adapters run on macOS by delegating container operations to a lightweight Linux VM — they do
> not provide native Linux namespace isolation on macOS.

---

## Capability Matrix

Cells marked **Linux only** require the `native` adapter on Linux. Cells marked **via VM**
indicate the capability is provided by the underlying VM, not by minibox namespace code.

| Feature                 | native          | gke  | colima  | smolvm | krun | winbox |
| ----------------------- | --------------- | ---- | ------- | ------ | ---- | ------ |
| **Container lifecycle** |                 |      |         |        |      |        |
| pull                    | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| run                     | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| stop                    | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| rm                      | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| ps                      | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| pause/resume            | Yes (Linux only)| No   | No      | No     | No   | No     |
| exec (-it)              | Yes (Linux only)| No   | Limited | No     | No   | No     |
| logs                    | Yes (Linux only)| No   | Limited | No     | No   | No     |
| events                  | Yes (Linux only)| Yes  | No      | No     | No   | No     |
| **Image management**    |                 |      |         |        |      |        |
| Docker Hub v2           | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| ghcr.io                 | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| Parallel layer pull     | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| prune / rmi             | Yes (Linux only)| No   | No      | No     | No   | No     |
| push (exp)              | Yes (Linux only)| Yes  | No      | No     | No   | No     |
| commit (exp)            | Yes (Linux only)| No   | No      | No     | No   | No     |
| build (exp)             | Yes (Linux only)| No   | No      | No     | No   | No     |
| **Isolation**           |                 |      |         |        |      |        |
| PID namespace           | Yes (Linux only)| No   | via VM  | via VM | via VM | No   |
| Mount namespace         | Yes (Linux only)| No   | via VM  | via VM | via VM | No   |
| Network namespace       | Yes (Linux only)| No   | via VM  | via VM | via VM | No   |
| UTS namespace           | Yes (Linux only)| No   | via VM  | via VM | via VM | No   |
| IPC namespace           | Yes (Linux only)| No   | via VM  | via VM | via VM | No   |
| cgroups v2              | Yes (Linux only)| No   | via VM  | via VM | No   | No     |
| Overlay FS              | Yes (Linux only)| Copy | nerdctl | No     | No   | No     |
| **Networking**          |                 |      |         |        |      |        |
| Bridge (exp)            | Yes (Linux only)| No   | No      | No     | No   | No     |
| Port forwarding         | No              | No   | No      | No     | No   | No     |
| DNS                     | No              | No   | No      | No     | No   | No     |
| **Mounts & Privileges** |                 |      |         |        |      |        |
| Bind mounts (`-v`)      | Yes (Linux only)| No   | No      | No     | No   | No     |
| Privileged mode         | Yes (Linux only)| No   | No      | No     | No   | No     |
| **Security**            |                 |      |         |        |      |        |
| SO_PEERCRED auth        | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| Tar path validation     | Yes             | Yes  | Yes     | Yes    | Yes  | Yes    |
| Setuid stripping        | Yes             | Yes  | Yes     | Yes    | Yes  | Yes    |
| Device node rejection   | Yes             | Yes  | Yes     | Yes    | Yes  | Yes    |
| **State persistence**   |                 |      |         |        |      |        |
| Records survive restart | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| PID reconciliation      | Yes (Linux only)| No   | No      | No     | No   | No     |
| **Observability**       |                 |      |         |        |      |        |
| Structured tracing      | Yes             | Yes  | Yes     | Yes    | Yes  | No     |
| OTLP export (opt-in)    | Yes             | Yes  | Yes     | Yes    | Yes  | No     |

> **SO_PEERCRED auth** applies to the minibox daemon's Unix socket on all adapters. The
> `colima`/`smolvm`/`krun` adapters still enforce UID 0 on the daemon socket; container
> operations are then forwarded into the VM over Lima/libkrun's own channel.

---

## Legend

- **Yes** — implemented and tested
- **Yes (Linux only)** — implemented; requires the `native` adapter on Linux with kernel 5.0+,
  cgroups v2, and root; not available on macOS or Windows adapters
- **No** — not implemented for this adapter
- **Limited** — partially working, known gaps
- **via VM** — isolation provided by the underlying VM (Lima, SmolVM, or libkrun), not by
  minibox Linux namespace/cgroup code; not available natively on macOS
- **Copy** — uses copy-based filesystem instead of overlay
- **nerdctl** — delegated to nerdctl running inside the Lima VM

---

## Code Citations

Each row in the capability matrix that claims Yes or Limited maps to one primary entry point
below. Citations use repo-relative paths. Line numbers are approximate and may drift as code
evolves — use the function name to anchor your search.

### Container lifecycle

| Feature      | Entry point                                                                    |
| ------------ | ------------------------------------------------------------------------------ |
| pull         | `crates/minibox/src/daemon/handler.rs:1793` — `handle_pull`                   |
| run          | `crates/minibox/src/daemon/handler.rs:340` — `handle_run`                     |
| stop         | `crates/minibox/src/daemon/handler.rs:1335` — `handle_stop`                   |
| rm           | `crates/minibox/src/daemon/handler.rs:1562` — `handle_remove`                 |
| ps           | `crates/minibox/src/daemon/handler.rs:1653` — `handle_list`                   |
| pause/resume | `crates/minibox/src/daemon/handler.rs:1471` — `handle_pause` (cgroup.freeze); |
|              | `crates/minibox/src/container/cgroups.rs:67` — `CgroupManager` implementation |
| exec (-it)   | `crates/minibox/src/daemon/handler.rs:1935` — `handle_exec`;                  |
|              | `crates/minibox/src/adapters/exec.rs:127` — `build_nsenter_command`           |
| logs         | `crates/minibox/src/daemon/handler.rs:1667` — `handle_logs`                   |
| events       | `crates/minibox/src/daemon/handler.rs:2455` — `handle_subscribe_events`;      |
|              | `crates/minibox-core/src/events.rs:76` — `BroadcastEventBroker`               |

### Image management

| Feature             | Entry point                                                                              |
| ------------------- | ---------------------------------------------------------------------------------------- |
| Docker Hub v2       | `crates/minibox-core/src/adapters/registry_router.rs:26` — `HostnameRegistryRouter`     |
|                     | (routes non-GHCR refs to the Docker Hub registry by default)                            |
| ghcr.io             | `crates/minibox/src/adapters/ghcr.rs:384` — `pull_image` on `GhcrRegistry`              |
| Parallel layer pull | `crates/minibox-core/src/image/registry.rs:516` — `pull_image`                          |
|                     | (see comment at line 509: "Download all layers in parallel")                             |
| prune / rmi         | `crates/minibox-core/src/image/gc.rs:45` — `ImageGc::prune`;                            |
|                     | `crates/minibox/src/daemon/handler.rs:1793` — wired via `handle_pull` / prune path      |
| push (exp)          | `crates/minibox/src/adapters/push.rs:50` — `push_image`;                                |
|                     | `crates/minibox/src/daemon/handler.rs:2134` — `handle_push`                             |
| commit (exp)        | `crates/minibox/src/adapters/commit.rs:58` — `commit_upper_dir_to_image`;               |
|                     | `crates/minibox/src/daemon/handler.rs:2232` — `handle_commit`                           |
| build (exp)         | `crates/minibox/src/adapters/builder.rs:86` — `build_image`;                            |
|                     | `crates/minibox/src/daemon/handler.rs:2326` — `handle_build`                            |

### Isolation (native adapter)

All five Linux namespace types are configured through a single struct and applied via a single
`clone(2)` call. The GKE adapter uses proot instead (`crates/minibox/src/adapters/gke.rs:260`
— `ProotRuntime`).

| Feature         | Entry point                                                                       |
| --------------- | --------------------------------------------------------------------------------- |
| PID namespace   | `crates/minibox/src/container/namespace.rs:26` — `NamespaceConfig::pid` field;   |
|                 | `crates/minibox/src/container/namespace.rs:52` — `to_clone_flags` sets           |
|                 | `CLONE_NEWPID`                                                                    |
| Mount namespace | Same `to_clone_flags` (line 58) — sets `CLONE_NEWNS`;                            |
|                 | `crates/minibox/src/container/filesystem.rs:103` — `setup_overlay` mounts        |
|                 | overlayfs inside the new mount namespace                                          |
| Network ns.     | Same `to_clone_flags` (line 67) — sets `CLONE_NEWNET`                            |
| UTS namespace   | Same `to_clone_flags` (line 61) — sets `CLONE_NEWUTS`                            |
| IPC namespace   | Same `to_clone_flags` (line 64) — sets `CLONE_NEWIPC`                            |
| cgroups v2      | `crates/minibox/src/container/cgroups.rs:44` — `CgroupManager`                   |
| Overlay FS      | `crates/minibox/src/container/filesystem.rs:103` — `setup_overlay`               |

### Networking

| Feature     | Entry point                                                                             |
| ----------- | --------------------------------------------------------------------------------------- |
| Bridge      | `crates/minibox/src/adapters/network/bridge.rs:80` — `BridgeNetwork`;                  |
|             | `crates/minibox/src/adapters/network/bridge.rs:12` — `IpAllocator` (DNAT/IP tracking) |
| Port fwd.   | Not implemented — no port-forwarding code present                                       |
| DNS         | Not implemented — no container DNS resolver present                                     |

### Mounts and privileges

| Feature         | Entry point                                                                         |
| --------------- | ----------------------------------------------------------------------------------- |
| Bind mounts     | `crates/minibox/src/container/filesystem.rs:327` — `apply_bind_mounts`;            |
|                 | `crates/minibox/src/daemon/handler.rs:298` — `validate_policy` enforces            |
|                 | `allow_bind_mounts` gate before the handler proceeds                                |
| Privileged mode | `crates/minibox/src/daemon/handler.rs:252` — `HandlerDependencies::allow_privileged`; |
|                 | `crates/minibox/src/daemon/handler.rs:298` — `validate_policy` enforces the gate   |

### Security

All four security controls are adapter-agnostic and applied before any container operation.

| Feature               | Entry point                                                                          |
| --------------------- | ------------------------------------------------------------------------------------ |
| SO_PEERCRED auth      | `crates/minibox/src/daemon/server.rs:75` — `is_authorized` (UID == 0 check)         |
| Tar path validation   | `crates/minibox-core/src/image/layer.rs:321` — `validate_layer_path`;               |
|                       | symlink/device rejection at lines 181 and 214                                        |
| Setuid stripping      | `crates/minibox-core/src/image/layer.rs:269` — strips setuid/setgid/sticky bits     |
|                       | (mask removes 04000, 02000, 01000) during tar extraction                             |
| Device node rejection | `crates/minibox-core/src/image/layer.rs:181` — rejects block/char devices in        |
|                       | tar entries before any filesystem write                                              |

### State persistence

| Feature               | Entry point                                                                         |
| --------------------- | ----------------------------------------------------------------------------------- |
| Records survive restart | `crates/minibox/src/daemon/state.rs:295` — `DaemonState::load_from_disk`;        |
|                         | `crates/minibox/src/daemon/state.rs:411` — `DaemonState::save_to_disk`           |
| PID reconciliation    | `crates/minibox/src/daemon/state.rs:359` — `DaemonState::reconcile_on_startup`;   |
|                       | marks stale Running containers Orphaned if the PID is gone                          |

### Observability

| Feature            | Entry point                                                                             |
| ------------------ | --------------------------------------------------------------------------------------- |
| Structured tracing | All handlers use `tracing::info!/warn!/error!` with key=value fields — see              |
|                    | `crates/minibox/src/daemon/handler.rs` throughout                                      |
| OTLP export        | `crates/minibox/src/daemon/telemetry/traces.rs:23` — `init_tracing(otlp_endpoint)`;   |
|                    | configures an optional OTLP span exporter when `MINIBOX_OTLP_ENDPOINT` is set          |

---

## Notes

- **`native` adapter** runs directly on Linux using kernel namespaces, cgroups v2, and
  overlayfs. It requires Linux kernel 5.0+, root privileges, and cgroups v2 mounted at
  `/sys/fs/cgroup`. Features marked "Linux only" are unavailable through any other adapter.
- **`gke` adapter** uses proot for filesystem isolation and a no-op resource limiter. Designed
  for running inside unprivileged GKE pods where namespaces and cgroups are unavailable.
- **`colima` adapter** is macOS-only. It delegates container operations to `nerdctl`/`limactl`
  inside a Lima VM. Exec and logs are limited because they route through Lima's SSH tunnel.
  Colima is not a supported Linux deployment path; use `native` on Linux.
- **`smolvm` adapter** is the **default on macOS** when `MINIBOX_ADAPTER` is unset and the
  `smolvm` binary is present on PATH. Automatically falls back to `krun` when the binary is
  absent. Runs lightweight Linux VMs with subsecond boot. macOS-only; not intended for Linux.
- **`krun` adapter** uses libkrun to run containers in lightweight VMs. All four adapter ports
  (runtime, registry, filesystem, limiter) are wired into the daemon and pass 31 conformance
  tests. Acts as the fallback when `smolvm` is unavailable. macOS-only.
- **`docker_desktop` adapter** (`DockerDesktopRuntime`/`Filesystem`/`Limiter`) exists in
  `crates/minibox/src/adapters/docker_desktop.rs` and is publicly exported, but is not
  registered in `AdapterSuite` or wired into the daemon. Not included in the matrix above.
- **`winbox`** returns an error unconditionally. Phase 2 (Named Pipe server, HCS/WSL2 wiring)
  has not started.
- **Observability env vars** (daemon startup):
  - `MINIBOX_OTLP_ENDPOINT` — OTLP trace export endpoint (`otel` feature required).
  - `MINIBOX_METRICS_ADDR` — Prometheus metrics bind address (e.g. `0.0.0.0:9090`);
    `metrics` feature required.
