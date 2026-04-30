# Feature Matrix

Per-platform capability breakdown for minibox adapters.

Last updated: 2026-04-27

---

## Adapter Suites

| Adapter  | Platform             | Status       | Crate   |
| -------- | -------------------- | ------------ | ------- |
| `native` | Linux (x86_64/arm64) | Production   | minibox |
| `gke`    | Linux (GKE pods)     | Production   | minibox |
| `colima` | macOS (Colima/Lima)  | Experimental | macbox  |
| `krun`   | macOS (libkrun)      | In progress  | macbox  |
| `vz`     | macOS (VZ.framework) | Blocked      | macbox  |
| `winbox` | Windows              | Stub         | winbox  |

---

## Capability Matrix

| Feature                 | native | gke  | colima  | krun | vz      | winbox |
| ----------------------- | ------ | ---- | ------- | ---- | ------- | ------ |
| **Container lifecycle** |        |      |         |      |         |        |
| pull                    | Yes    | Yes  | Yes     | Yes  | Blocked | No     |
| run                     | Yes    | Yes  | Yes     | WIP  | Blocked | No     |
| stop                    | Yes    | Yes  | Yes     | WIP  | Blocked | No     |
| rm                      | Yes    | Yes  | Yes     | WIP  | Blocked | No     |
| ps                      | Yes    | Yes  | Yes     | WIP  | Blocked | No     |
| pause/resume            | Yes    | No   | No      | No   | Blocked | No     |
| exec (-it)              | Yes    | No   | Limited | No   | Blocked | No     |
| logs                    | Yes    | No   | Limited | No   | Blocked | No     |
| events                  | Yes    | No   | No      | No   | Blocked | No     |
| **Image management**    |        |      |         |      |         |        |
| Docker Hub v2           | Yes    | Yes  | Yes     | Yes  | Blocked | No     |
| ghcr.io                 | Yes    | Yes  | Yes     | Yes  | Blocked | No     |
| Parallel layer pull     | Yes    | Yes  | Yes     | Yes  | Blocked | No     |
| prune / rmi             | Yes    | No   | No      | No   | Blocked | No     |
| push (exp)              | Yes    | No   | No      | No   | Blocked | No     |
| commit (exp)            | Yes    | No   | No      | No   | Blocked | No     |
| build (exp)             | Yes    | No   | No      | No   | Blocked | No     |
| **Isolation**           |        |      |         |      |         |        |
| PID namespace           | Yes    | No   | Lima VM | VM   | Blocked | No     |
| Mount namespace         | Yes    | No   | Lima VM | VM   | Blocked | No     |
| Network namespace       | Yes    | No   | Lima VM | VM   | Blocked | No     |
| UTS namespace           | Yes    | No   | Lima VM | VM   | Blocked | No     |
| IPC namespace           | Yes    | No   | Lima VM | VM   | Blocked | No     |
| cgroups v2              | Yes    | No   | No      | No   | Blocked | No     |
| Overlay FS              | Yes    | Copy | nerdctl | No   | Blocked | No     |
| **Networking**          |        |      |         |      |         |        |
| Bridge (exp)            | Yes    | No   | No      | No   | Blocked | No     |
| Port forwarding         | No     | No   | No      | No   | No      | No     |
| DNS                     | No     | No   | No      | No   | No      | No     |
| **Mounts & Privileges** |        |      |         |      |         |        |
| Bind mounts (`-v`)      | Yes    | No   | No      | No   | Blocked | No     |
| Privileged mode         | Yes    | No   | No      | No   | Blocked | No     |
| **Security**            |        |      |         |      |         |        |
| SO_PEERCRED auth        | Yes    | Yes  | Yes     | Yes  | Blocked | No     |
| Tar path validation     | Yes    | Yes  | Yes     | Yes  | Yes     | Yes    |
| Setuid stripping        | Yes    | Yes  | Yes     | Yes  | Yes     | Yes    |
| Device node rejection   | Yes    | Yes  | Yes     | Yes  | Yes     | Yes    |
| **State persistence**   |        |      |         |      |         |        |
| Records survive restart | Yes    | Yes  | Yes     | Yes  | Blocked | No     |
| PID reconciliation      | Yes    | No   | No      | No   | Blocked | No     |
| **Observability**       |        |      |         |      |         |        |
| Structured tracing      | Yes    | Yes  | Yes     | Yes  | Blocked | No     |
| OTLP export (opt-in)    | Yes    | Yes  | Yes     | Yes  | Blocked | No     |

---

## Legend

- **Yes** — implemented and tested
- **No** — not implemented for this adapter
- **Limited** — partially working, known gaps
- **WIP** — actively being developed
- **Blocked** — VZ.framework blocked by Apple bug (GH #61)
- **Copy** — uses copy-based filesystem instead of overlay
- **VM** — isolation provided by the underlying VM, not minibox namespaces

---

## Notes

- **`gke` adapter** uses proot for filesystem isolation and a no-op resource
  limiter. Designed for running inside unprivileged GKE pods where
  namespaces and cgroups are unavailable.
- **`colima` adapter** delegates to `nerdctl`/`limactl` inside a Lima VM.
  Exec and logs are limited because they go through Lima's SSH tunnel.
- **`krun` adapter** uses libkrun to run containers in lightweight VMs.
  Phases 1-3 (runtime, registry, filesystem, limiter adapters) are
  complete with 31 conformance tests. Daemon wiring is in progress.
- **`vz` adapter** targets Apple's Virtualization.framework directly.
  Blocked by `VZErrorInternal(code=1)` on macOS 26 ARM64
  ([GH #61](https://github.com/89jobrien/minibox/issues/61)).
- **`winbox`** returns an error unconditionally. Phase 2 (Named Pipe
  server, HCS/WSL2 wiring) has not started.
