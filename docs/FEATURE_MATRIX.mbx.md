# Feature Matrix

Per-platform capability breakdown for minibox adapters.

Last updated: 2026-05-08

---

## Adapter Suites

| Adapter  | Platform             | Status       | Crate   | Default?                    |
| -------- | -------------------- | ------------ | ------- | --------------------------- |
| `native` | Linux (x86_64/arm64) | Production   | minibox | --                          |
| `gke`    | Linux (GKE pods)     | Production   | minibox | --                          |
| `colima` | macOS/Linux (Colima) | Experimental | minibox | --                          |
| `smolvm` | macOS/Linux (SmolVM) | Experimental | minibox | Yes (falls back to `krun`)  |
| `krun`   | macOS/Linux (krun)   | Experimental | macbox  | Fallback when smolvm absent |
| `winbox` | Windows              | Stub         | winbox  | --                          |

---

## Capability Matrix

| Feature                 | native | gke  | colima  | smolvm | krun | winbox |
| ----------------------- | ------ | ---- | ------- | ------ | ---- | ------ |
| **Container lifecycle** |        |      |         |        |      |        |
| pull                    | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| run                     | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| stop                    | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| rm                      | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| ps                      | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| pause/resume            | Yes    | No   | No      | No     | No   | No     |
| exec (-it)              | Yes    | No   | Limited | No     | No   | No     |
| logs                    | Yes    | No   | Limited | No     | No   | No     |
| events                  | Yes    | Yes  | No      | No     | No   | No     |
| **Image management**    |        |      |         |        |      |        |
| Docker Hub v2           | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| ghcr.io                 | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| Parallel layer pull     | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| prune / rmi             | Yes    | No   | No      | No     | No   | No     |
| push (exp)              | Yes    | Yes  | No      | No     | No   | No     |
| commit (exp)            | Yes    | No   | No      | No     | No   | No     |
| build (exp)             | Yes    | No   | No      | No     | No   | No     |
| **Isolation**           |        |      |         |        |      |        |
| PID namespace           | Yes    | No   | Lima VM | VM     | VM   | No     |
| Mount namespace         | Yes    | No   | Lima VM | VM     | VM   | No     |
| Network namespace       | Yes    | No   | Lima VM | VM     | VM   | No     |
| UTS namespace           | Yes    | No   | Lima VM | VM     | VM   | No     |
| IPC namespace           | Yes    | No   | Lima VM | VM     | VM   | No     |
| cgroups v2              | Yes    | No   | Lima VM | VM     | No   | No     |
| Overlay FS              | Yes    | Copy | nerdctl | No     | No   | No     |
| **Networking**          |        |      |         |        |      |        |
| Bridge (exp)            | Yes    | No   | No      | No     | No   | No     |
| Port forwarding         | No     | No   | No      | No     | No   | No     |
| DNS                     | No     | No   | No      | No     | No   | No     |
| **Mounts & Privileges** |        |      |         |        |      |        |
| Bind mounts (`-v`)      | Yes    | No   | No      | No     | No   | No     |
| Privileged mode         | Yes    | No   | No      | No     | No   | No     |
| **Security**            |        |      |         |        |      |        |
| SO_PEERCRED auth        | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| Tar path validation     | Yes    | Yes  | Yes     | Yes    | Yes  | Yes    |
| Setuid stripping        | Yes    | Yes  | Yes     | Yes    | Yes  | Yes    |
| Device node rejection   | Yes    | Yes  | Yes     | Yes    | Yes  | Yes    |
| **State persistence**   |        |      |         |        |      |        |
| Records survive restart | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| PID reconciliation      | Yes    | No   | No      | No     | No   | No     |
| **Observability**       |        |      |         |        |      |        |
| Structured tracing      | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| OTLP export (opt-in)    | Yes    | Yes  | Yes     | Yes    | Yes  | No     |

---

## Legend

- **Yes** — implemented and tested
- **No** — not implemented for this adapter
- **Limited** — partially working, known gaps
- **WIP** — actively being developed
- **Copy** — uses copy-based filesystem instead of overlay
- **VM** — isolation provided by the underlying VM, not minibox namespaces

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

- **`gke` adapter** uses proot for filesystem isolation and a no-op resource
  limiter. Designed for running inside unprivileged GKE pods where
  namespaces and cgroups are unavailable.
- **`colima` adapter** delegates to `nerdctl`/`limactl` inside a Lima VM.
  Exec and logs are limited because they go through Lima's SSH tunnel.
- **`smolvm` adapter** is the **default** when `MINIBOX_ADAPTER` is unset and
  the `smolvm` binary is present on PATH. Automatically falls back to `krun`
  when the binary is absent. Lightweight Linux VMs with subsecond boot.
- **`krun` adapter** uses libkrun to run containers in lightweight VMs.
  All four adapter ports (runtime, registry, filesystem, limiter) are wired
  into the daemon and pass 31 conformance tests. Acts as the fallback when
  `smolvm` is unavailable.
- **`docker_desktop` adapter** (`DockerDesktopRuntime`/`Filesystem`/`Limiter`)
  exists in `crates/minibox/src/adapters/docker_desktop.rs` and is publicly
  exported, but is not registered in `AdapterSuite` or wired into the daemon.
  Not included in the matrix above.
- **`winbox`** returns an error unconditionally. Phase 2 (Named Pipe
  server, HCS/WSL2 wiring) has not started.
- **Observability env vars** (daemon startup):
  - `MINIBOX_OTLP_ENDPOINT` — OTLP trace export endpoint (`otel` feature required).
  - `MINIBOX_METRICS_ADDR` — Prometheus metrics bind address (e.g. `0.0.0.0:9090`);
    `metrics` feature required.
