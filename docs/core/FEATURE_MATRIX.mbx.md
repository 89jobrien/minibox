# Feature Matrix

Per-platform capability breakdown for minibox adapters.

Last updated: 2026-07-20

---

## Adapter Suites

| Adapter  | Platform                        | Status       | Crate   | Default?                          |
| -------- | ------------------------------- | ------------ | ------- | --------------------------------- |
| `native` | Linux only (x86_64/arm64) [^1]  | Production   | minibox | Fallback on Linux                 |
| `gke`    | Linux only (GKE pods) [^2]      | Production   | minibox | --                                |
| `colima` | Unix (macOS/Linux, Colima)      | Experimental | minibox | --                                |
| `smolvm` | Unix (macOS/Linux, SmolVM) [^3] | Experimental | minibox | Yes (Unix; not available on Win)  |
| `krun`   | Unix (macOS/Linux, krun)        | Experimental | macbox  | Fallback when smolvm absent [^4]  |
| `winbox` | Windows                         | Stub         | winbox  | --                                |

[^1]: `native` requires root (UID 0). Rejected at startup if non-root. Linux only
      (`cfg!(target_os = "linux")`). Cgroup v2 and overlay FS require kernel support.
[^2]: `gke` is Linux only (`cfg!(target_os = "linux")`). Unprivileged — no root required.
      Uses proot (ptrace) and copy-based filesystem instead of overlay.
[^3]: `smolvm` is compiled only on Unix (`cfg!(unix)`). Not available on Windows builds.
      Requires the `smolvm` binary on PATH at runtime.
[^4]: `krun` fallback platform-splits: `native` on Linux, `krun` on macOS, when
      `smolvm` binary is absent and `MINIBOX_ADAPTER` is unset.

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
| restart                 | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| exec (-it)              | Yes    | No   | Limited | No     | No   | No     |
| logs                    | Yes    | No   | Limited | No     | No   | No     |
| events                  | Yes    | Yes  | No      | No     | No   | No     |
| **Image management**    |        |      |         |        |      |        |
| Docker Hub v2           | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| ghcr.io                 | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| Parallel layer pull     | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| prune / rmi             | Yes    | No   | No      | No     | No   | No     |
| push (exp)              | Yes    | Yes  | Yes     | No     | No   | No     |
| commit (exp)            | Yes    | No   | Yes     | No     | No   | No     |
| build (exp)             | Yes    | No   | Yes     | Yes    | No   | No     |
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
| Layer digest verify     | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| Request frame limits    | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| Env redaction in logs   | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| **Execution integrity** |        |      |         |        |      |        |
| Execution manifest      | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| manifest get/verify     | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| Admission policy gate   | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| **State persistence**   |        |      |         |        |      |        |
| Records survive restart | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| PID reconciliation      | Yes    | No   | No      | No     | No   | No     |
| **Observability**       |        |      |         |        |      |        |
| Structured tracing      | Yes    | Yes  | Yes     | Yes    | Yes  | No     |
| OTLP export (opt-in)    | Yes    | Yes  | Yes     | Yes    | Yes  | No     |

---

## Source References for Capability Matrix

Key implementation sites backing the "Yes" entries above:

| Feature area | Source |
| --- | --- |
| Container lifecycle (run/stop/rm/ps/restart) | `crates/minibox/src/daemon/handler/lifecycle.rs`, `handler/run.rs`, `handler/stop.rs` |
| pause/resume (native, cgroup.freeze) | `crates/minibox/src/adapters/limiter.rs:CgroupV2Limiter` |
| exec | `crates/minibox/src/daemon/handler/exec.rs`, `crates/minibox-core/src/domain.rs:ExecRuntime` |
| logs | `crates/minibox/src/daemon/handler/logs.rs` |
| events | `crates/minibox-core/src/events.rs:EventSink`/`EventSource` |
| Image pull (Docker Hub v2 + parallel layers) | `crates/minibox-core/src/image/registry.rs:pull_image` |
| Image pull (ghcr.io) | `crates/minibox/src/adapters/ghcr.rs` |
| prune/rmi | `crates/minibox-core/src/image/gc.rs:ImageGarbageCollector` |
| push | `crates/minibox-core/src/domain.rs:ImagePusher` |
| commit | `crates/minibox-core/src/domain.rs:ContainerCommitter` |
| build | `crates/minibox-core/src/domain.rs:ImageBuilder` |
| PID/Mount/Net/UTS/IPC namespaces (native) | `crates/minibox/src/container/namespace.rs` |
| cgroups v2 | `crates/minibox/src/adapters/limiter.rs:CgroupV2Limiter` |
| Overlay FS | `crates/minibox/src/adapters/filesystem.rs:OverlayFilesystem` |
| Bridge networking | `crates/minibox/src/adapters/network/bridge.rs:BridgeNetwork` |
| Bind mounts / privileged mode | `crates/minibox/src/daemon/handler/run.rs` |
| SO_PEERCRED auth | `crates/minibox/src/daemon/server.rs:is_authorized` |
| Tar path validation | `crates/minibox-core/src/image/layer.rs:validate_tar_entry_path` |
| Setuid stripping | `crates/minibox-core/src/image/layer.rs` (mode & 0o777) |
| Device node rejection | `crates/minibox-core/src/image/layer.rs` (Block/Char check) |
| Layer digest verify | `crates/minibox-core/src/image/registry.rs` |
| Request frame limits | `crates/minibox/src/daemon/server.rs:MAX_REQUEST_SIZE` |
| Execution manifest + verify | `crates/minibox-core/src/domain/execution_manifest.rs` |
| Admission policy gate | `crates/minibox-core/src/domain/execution_policy.rs` |
| State persistence + PID reconciliation | `crates/minibox/src/daemon/state.rs:DaemonState` |
| Structured tracing | `crates/miniboxd/src/main.rs` (tracing subscriber init) |
| OTLP export | `crates/miniboxd/src/main.rs` (otel feature gate) |

---

## Legend

- **Yes** -- implemented and tested
- **No** -- not implemented for this adapter
- **Limited** -- partially working, known gaps
- **WIP** -- actively being developed
- **Copy** -- uses copy-based filesystem instead of overlay
- **VM** -- isolation provided by the underlying VM, not
  minibox namespaces

---

## Notes

- **`gke` adapter** uses proot for filesystem isolation
  (see `crates/minibox/src/adapters/gke.rs:ProotRuntime`) and a
  no-op resource limiter (`crates/minibox/src/adapters/gke.rs:NoopLimiter`).
  Designed for running inside unprivileged GKE pods where namespaces and
  cgroups are unavailable.
- **`colima` adapter** delegates to `nerdctl`/`limactl` inside a
  Lima VM
  (see `crates/minibox/src/adapters/colima.rs:ColimaRuntime`).
  Exec and logs are limited because they go through Lima's SSH
  tunnel. Push, commit, and build are wired via
  `ColimaImagePusher`, `OverlayCommitAdapter`, and
  `MiniboxImageBuilder`.
- **`smolvm` adapter** is the **default on Unix** when
  `MINIBOX_ADAPTER` is unset and the `smolvm` binary is present on
  PATH (see `crates/miniboxd/src/adapter_registry.rs`). Falls back
  to `native` on Linux or `krun` on macOS when the binary is
  absent. Not available on Windows (`cfg!(unix)`). Lightweight Linux
  VMs with subsecond boot
  (see `crates/minibox/src/adapters/smolvm.rs:SmolVmRuntime`).
- **`krun` adapter** uses libkrun to run containers in
  lightweight VMs
  (see `crates/macbox/src/krun/runtime.rs:KrunRuntime`).
  All four adapter ports (runtime, registry, filesystem, limiter)
  are wired into the daemon
  (see `crates/miniboxd/src/main.rs:build_krun_handler_dependencies`)
  and pass 31 conformance tests. Acts as the fallback when
  `smolvm` is unavailable.
- **`docker_desktop` adapter**
  (`DockerDesktopRuntime`/`Filesystem`/`Limiter`) exists in
  `crates/minibox/src/adapters/docker_desktop.rs` and is publicly
  exported, but is not registered in `AdapterSuite` or wired into
  the daemon. Not included in the matrix above.
  <!--joe:note::docker_desktop adapter logic lives in crates/minibox/src/adapters/docker_desktop.rs-->
- **`winbox`** returns an error unconditionally. Phase 2 (Named Pipe
  server, HCS/WSL2 wiring) has not started.
- **Execution integrity** is implemented at the daemon handler
  layer, not inside individual adapters
  (see `crates/minibox/src/daemon/handler/run.rs:prepare_run`
  and `crates/minibox/src/daemon/handler/manifest.rs`). All
  adapters that support `run` inherit manifest persist,
  `mbx manifest`, `mbx verify`, and admission-policy gating
  (see `crates/minibox-core/src/domain/execution_policy.rs:ExecutionPolicy`).
  Environment variable values are stored as SHA-256 digests --
  never plaintext -- in `execution-manifest.json`
  (see `crates/minibox-core/src/domain/execution_manifest.rs:ExecutionManifest::seal`).
- **Observability env vars** (daemon startup,
  see `crates/miniboxd/src/main.rs`):
    - `MINIBOX_OTLP_ENDPOINT` -- OTLP trace export endpoint
      (`otel` feature required).
    - `MINIBOX_METRICS_ADDR` -- Prometheus metrics bind address
      (e.g. `0.0.0.0:9090`); `metrics` feature required.
