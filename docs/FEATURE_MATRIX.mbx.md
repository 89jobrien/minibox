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
