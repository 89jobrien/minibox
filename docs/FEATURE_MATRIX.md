# Feature Matrix

This document reflects the **actual implementation state** derived from source code inspection.
It supersedes any conflicting claims in CLAUDE.md or README.md.

**Primary sources inspected:**

- Handlers: `crates/daemonbox/src/handler.rs` (`handle_run` L317, `handle_exec` L1819,
  `handle_logs` L1605, `handle_load_image` L1755, `handle_push` L2018, `handle_commit` L2116,
  `handle_build` L2210)
- Adapter structs: `crates/minibox/src/adapters/` — `runtime.rs` (`LinuxNamespaceRuntime`),
  `filesystem.rs` (`OverlayFilesystem`), `limiter.rs` (`CgroupV2Limiter`), `gke.rs`
  (`ProotRuntime`, `CopyFilesystem`, `NoopLimiter`), `colima.rs` (`ColimaRuntime`)
- Image management: `crates/minibox-oci/src/image/gc.rs` (`ImageGc`),
  `crates/minibox-oci/src/image/lease.rs` (`DiskLeaseService`),
  `crates/minibox/src/adapters/push.rs` (`OciPushAdapter`),
  `crates/minibox/src/adapters/builder.rs` (`MiniboxImageBuilder`)
- Events: `crates/minibox-core/src/events.rs` (`BroadcastEventBroker`)
- Daemon entry: `crates/miniboxd/src/main.rs`; platform dispatch: `crates/macbox/src/`,
  `crates/winbox/src/`

For the authoritative last-modified date, run: `git log -1 --format="%ci" -- docs/FEATURE_MATRIX.md`

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
| Exec in running container   | ✓            | —         | —            | —        | —       | Linux native only; `setns` + optional PTY (`-it`) — `handle_exec` L1819 |
| Named containers (`--name`) | ✓            | ✓         | ~            | ~        | —       | Name stored in daemon state                                          |

## Isolation and Resource Control

| Feature                 | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                             |
| ----------------------- | ------------ | --------- | ------------ | -------- | ------- | ------------------------------------------------- |
| PID namespace           | ✓            | ~         | —            | ✓        | —       | GKE via proot (limited isolation)                 |
| Network namespace       | ✓            | ~         | —            | ✓        | —       | Isolated but no veth/bridge by default            |
| Mount namespace         | ✓            | ~         | —            | ✓        | —       |                                                   |
| UTS / IPC namespaces    | ✓            | —         | —            | ✓        | —       |                                                   |
| cgroups v2 memory limit | ✓            | —         | —            | —        | —       | `CgroupV2Limiter` (`adapters/limiter.rs`); requires kernel 5.0+ |
| cgroups v2 CPU weight   | ✓            | —         | —            | —        | —       | `CgroupV2Limiter` (`adapters/limiter.rs`)                       |
| Overlay filesystem      | ✓            | —         | —            | —        | —       | `OverlayFilesystem` (`adapters/filesystem.rs`); requires `CONFIG_OVERLAY_FS` |
| Copy filesystem (GKE)   | —            | ✓         | —            | —        | —       | `CopyFilesystem` (`adapters/gke.rs`); no overlay needed         |
| Bind mounts (`-v`)      | ✓            | —         | ~            | —        | —       | `--mount` flag on `run`                           |
| Privileged mode         | ✓            | —         | —            | —        | —       | `--privileged` on `run`; policy-gated             |

## Networking

| Feature                    | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                                             |
| -------------------------- | ------------ | --------- | ------------ | -------- | ------- | ----------------------------------------------------------------- |
| Isolated network namespace | ✓            | ~         | —            | ✓        | —       | No external connectivity by default                               |
| Bridge networking + NAT    | ~            | —         | —            | —        | —       | `MINIBOX_NETWORK_MODE=bridge`; veth + iptables DNAT; `adapters/network/bridge.rs`; experimental |
| Host networking            | ~            | —         | —            | —        | —       | `MINIBOX_NETWORK_MODE=host`; wired, limited testing               |
| Tailnet (Tailscale)        | ~            | —         | —            | —        | —       | `MINIBOX_NETWORK_MODE=tailnet`; requires `tailnet` feature flag   |
| Port forwarding            | —            | —         | —            | —        | —       | Not implemented                                                   |
| DNS inside container       | —            | —         | —            | —        | —       | Not implemented                                                   |

## Image Management

| Feature                   | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                                        |
| ------------------------- | ------------ | --------- | ------------ | -------- | ------- | ------------------------------------------------------------ |
| Image store (disk)        | ✓            | ✓         | ~            | ~        | —       | `ImageStore`; layers at `MINIBOX_DATA_DIR/images/`           |
| Image GC (`prune`/`rmi`)  | ✓            | ✓         | ~            | ~        | —       | `ImageGc` + `DiskLeaseService` (`minibox-oci/src/image/gc.rs`, `lease.rs`) |
| Load local OCI tarball    | ✓            | ✓         | ~            | ~        | —       | `handle_load_image` (`daemonbox/src/handler.rs` L1755)              |
| Push image                | ~            | —         | —            | —        | —       | `OciPushAdapter` (`adapters/push.rs`); native only; experimental    |
| Commit container to image | ~            | —         | —            | —        | —       | `adapters/commit.rs`; native only; experimental                     |
| Build image (Dockerfile)  | ~            | —         | —            | —        | —       | `MiniboxImageBuilder` (`adapters/builder.rs`); native only; no Dockerfile parser yet |

## Observability

| Feature                      | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                      |
| ---------------------------- | ------------ | --------- | ------------ | -------- | ------- | ------------------------------------------ |
| Structured tracing logs      | ✓            | ✓         | ✓            | ✓        | —       | `tracing` crate throughout                 |
| Container stdout/stderr logs | ✓            | ~         | —            | —        | —       | `handle_logs` (`daemonbox/src/handler.rs` L1605); stored in `DaemonState` |
| Container lifecycle events   | ✓            | ✓         | ~            | ~        | —       | `BroadcastEventBroker` (`minibox-core/src/events.rs`); `minibox events` |
| Prometheus metrics           | ~            | —         | —            | —        | —       | `feature = "metrics"`; `/metrics` endpoint |
| OpenTelemetry OTLP traces    | ~            | —         | —            | —        | —       | `feature = "otel"`; compile-time opt-in    |

## Persistent State

| Feature                           | Linux native | Linux GKE | macOS Colima | macOS VZ | Windows | Notes                                                              |
| --------------------------------- | ------------ | --------- | ------------ | -------- | ------- | ------------------------------------------------------------------ |
| Container records persisted       | ~            | ~         | ~            | ~        | —       | Saved to disk via `DaemonState::save_to_disk()`; loaded at startup |
| Container records across restarts | ~            | ~         | ~            | ~        | —       | Records survive restart; running containers are not reattached     |
| Image layer cache persisted       | ✓            | ✓         | ~            | ~        | —       | Disk-backed; survives restarts                                     |

> **Note on state persistence**: CLAUDE.md previously said "no persistent state". This is now
> partially incorrect. `DaemonState` saves container records to disk (loaded on startup), but
> the daemon does not reattach to processes from a previous run.

## Platform Daemon Status

| Platform             | Status         | Notes                                                                |
| -------------------- | -------------- | -------------------------------------------------------------------- |
| Linux                | ✓ Shipped      | Primary target; all core features                                    |
| macOS (Colima)       | ~ Experimental | Delegates to `limactl`/`nerdctl`; exec/logs limited                  |
| macOS (VZ.framework) | ~ Experimental | Requires `--features vz` + `cargo xtask build-vm-image`              |
| Windows              | S Stub         | `winbox::start()` returns error unconditionally (`crates/winbox/src/lib.rs`); Phase 2 not started |

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
