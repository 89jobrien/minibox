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
  into the daemon and pass 29 conformance tests. Acts as the fallback when
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
