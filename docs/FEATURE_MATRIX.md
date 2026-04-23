# Feature Matrix

This document reflects the **actual implementation state** derived from source code inspection
(`crates/daemonbox/src/handler.rs`, `crates/miniboxd/src/main.rs`, `crates/minibox/src/adapters/`,
`crates/macbox/src/`, `crates/winbox/src/`). It supersedes any conflicting claims in CLAUDE.md
or README.md.

Last updated: 2026-04-19

---

## Legend

| Symbol | Meaning                                                        |
| ------ | -------------------------------------------------------------- |
| ✓      | Shipped — handler wired, adapter wired, tested                 |
| ~      | Experimental — wired but limited coverage or known gaps        |
| L      | Library only — types and adapters exist; not wired into daemon |
| S      | Stub — function exists but returns an error unconditionally    |
| —      | Not implemented                                                |

---

## Core Container Operations

| Feature                     | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                                                |
| --------------------------- | ------------ | --------- | ------------ | -------- | ------- | -------------------------------------------------------------------- |
| Pull image (Docker Hub)     | ✓            | ✓         | ✓            | ~        | —       | GKE/Colima delegate to respective runtimes                           |
| Pull image (ghcr.io)        | ✓            | ✓         | ~            | —        | —       | `GhcrRegistry` wired in native+GKE; Colima uses Colima registry only |
| Run container               | ✓            | ✓         | ✓            | ~        | —       | GKE uses proot; macOS VZ boots Alpine VM                             |
| Stop container              | ✓            | ✓         | ~            | ~        | —       |                                                                      |
| Remove container            | ✓            | ✓         | ~            | ~        | —       |                                                                      |
| List containers (`ps`)      | ✓            | ✓         | ✓            | ✓        | —       | In-memory state; survives until daemon restart                       |
| Exec in running container   | ✓            | —         | —            | —        | —       | Linux native only; `setns` + optional PTY (`-it`)                    |
| Named containers (`--name`) | ✓            | ✓         | ~            | ~        | —       | Name stored in daemon state                                          |

## Isolation and Resource Control

| Feature                 | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                             |
| ----------------------- | ------------ | --------- | ------------ | -------- | ------- | ------------------------------------------------- |
| PID namespace           | ✓            | ~         | —            | ✓        | —       | GKE via proot (limited isolation)                 |
| Network namespace       | ✓            | ~         | —            | ✓        | —       | Isolated but no veth/bridge by default            |
| Mount namespace         | ✓            | ~         | —            | ✓        | —       |                                                   |
| UTS / IPC namespaces    | ✓            | —         | —            | ✓        | —       |                                                   |
| cgroups v2 memory limit | ✓            | —         | —            | —        | —       | `CgroupV2Limiter`; requires kernel 5.0+           |
| cgroups v2 CPU weight   | ✓            | —         | —            | —        | —       |                                                   |
| Overlay filesystem      | ✓            | —         | —            | —        | —       | `OverlayFilesystem`; requires `CONFIG_OVERLAY_FS` |
| Copy filesystem (GKE)   | —            | ✓         | —            | —        | —       | `CopyFilesystem`; no overlay needed               |
| Bind mounts (`-v`)      | ✓            | —         | ~            | —        | —       | `--mount` flag on `run`                           |
| Privileged mode         | ✓            | —         | —            | —        | —       | `--privileged` on `run`; policy-gated             |

## Networking

| Feature                    | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                                             |
| -------------------------- | ------------ | --------- | ------------ | -------- | ------- | ----------------------------------------------------------------- |
| Isolated network namespace | ✓            | ~         | —            | ✓        | —       | No external connectivity by default                               |
| Bridge networking + NAT    | ~            | —         | —            | —        | —       | `MINIBOX_NETWORK_MODE=bridge`; veth + iptables DNAT; experimental |
| Host networking            | ~            | —         | —            | —        | —       | `MINIBOX_NETWORK_MODE=host`; wired, limited testing               |
| Tailnet (Tailscale)        | ~            | —         | —            | —        | —       | `MINIBOX_NETWORK_MODE=tailnet`; requires `tailnet` feature flag   |
| Port forwarding            | —            | —         | —            | —        | —       | Not implemented                                                   |
| DNS inside container       | —            | —         | —            | —        | —       | Not implemented                                                   |

## Image Management

| Feature                   | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                                        |
| ------------------------- | ------------ | --------- | ------------ | -------- | ------- | ------------------------------------------------------------ |
| Image store (disk)        | ✓            | ✓         | ~            | ~        | —       | `ImageStore`; layers at `MINIBOX_DATA_DIR/images/`           |
| Image GC (`prune`/`rmi`)  | ✓            | ✓         | ~            | ~        | —       | `ImageGarbageCollector` + `DiskLeaseService`                 |
| Load local OCI tarball    | ✓            | ✓         | ~            | ~        | —       | `handle_load_image`                                          |
| Push image                | ~            | —         | —            | —        | —       | `OciPushAdapter`; native only; experimental                  |
| Commit container to image | ~            | —         | —            | —        | —       | `overlay_commit_adapter`; native only; experimental          |
| Build image (Dockerfile)  | ~            | —         | —            | —        | —       | `MiniboxImageBuilder`; native only; no Dockerfile parser yet |

## Observability

| Feature                      | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                      |
| ---------------------------- | ------------ | --------- | ------------ | -------- | ------- | ------------------------------------------ |
| Structured tracing logs      | ✓            | ✓         | ✓            | ✓        | —       | `tracing` crate throughout                 |
| Container stdout/stderr logs | ✓            | ~         | —            | —        | —       | `handle_logs`; stored in daemon state      |
| Container lifecycle events   | ✓            | ✓         | ~            | ~        | —       | `BroadcastEventBroker`; `minibox events`   |
| Prometheus metrics           | ~            | —         | —            | —        | —       | `feature = "metrics"`; `/metrics` endpoint |
| OpenTelemetry OTLP traces    | ~            | —         | —            | —        | —       | `feature = "otel"`; compile-time opt-in    |

## Persistent State

| Feature                           | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                                              |
| --------------------------------- | ------------ | --------- | ------------ | -------- | ------- | ------------------------------------------------------------------ |
| Container records persisted       | ~            | ~         | ~            | ~        | —       | Saved to disk via `DaemonState::save_to_disk()`; loaded at startup |
| Container records across restarts | ~            | ~         | ~            | ~        | —       | Records survive restart; running containers are not reattached     |
| Image layer cache persisted       | ✓            | ✓         | ~            | ~        | —       | Disk-backed; survives restarts                                     |

> **Note on state persistence**: `DaemonState` saves container records to disk (loaded on
> startup). The daemon does not reattach to running processes after a restart — their PIDs are
> gone; records survive as `Stopped`. See `docs/STATE_MODEL.md` for the full persistence
> contract.

## Platform Daemon Status

| Platform             | Status         | Notes                                                                |
| -------------------- | -------------- | -------------------------------------------------------------------- |
| Linux                | ✓ Shipped      | Primary target; all core features                                    |
| macOS (Colima)       | ~ Experimental | Delegates to `limactl`/`nerdctl`; exec/logs limited                  |
| macOS (VZ.framework) | ~ Experimental | Requires `--features vz` + `cargo xtask build-vm-image`              |
| Windows              | S Stub         | `winbox::start()` returns error unconditionally; Phase 2 work needed |

## Adapter Wiring Summary (Linux daemon only)

| Adapter                                                           | `MINIBOX_ADAPTER` value | Wired | Status                  |
| ----------------------------------------------------------------- | ----------------------- | ----- | ----------------------- |
| `LinuxNamespaceRuntime` + `OverlayFilesystem` + `CgroupV2Limiter` | `native` (default)      | Yes   | Shipped                 |
| `ProotRuntime` + `CopyFilesystem` + `NoopLimiter`                 | `gke`                   | Yes   | Shipped                 |
| `ColimaRuntime` + `ColimaFilesystem` + `ColimaLimiter`            | `colima`                | Yes   | Experimental            |
| `docker_desktop` adapter                                          | not accepted            | No    | Library only            |
| `wsl2` adapter                                                    | not accepted            | No    | Library only            |
| `vf` adapter                                                      | not accepted (Linux)    | No    | macOS only via `macbox` |
| `hcs` adapter                                                     | not accepted            | No    | Library only            |

Passing an unrecognized value to `MINIBOX_ADAPTER` causes the daemon to exit at startup.
