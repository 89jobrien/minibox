# Feature Matrix

Per-platform capability breakdown for minibox adapters.

<!-- Last-updated: auto — run `git log -1 --format="%ad" -- docs/FEATURE_MATRIX.mbx.md` to check -->
<!-- Last-verified: 2026-06-13 — adapter source cross-checked against crates/minibox/src/adapters/ and crates/macbox/src/ -->

---

## Adapter Suites

| Adapter  | Platform             | Status       | Crate   | Default?                    |
| -------- | -------------------- | ------------ | ------- | --------------------------- |
| `native` | Linux (x86_64/arm64) | Production   | minibox | Fallback on Linux           | <!-- src: crates/minibox/src/adapters/runtime.rs -->
| `gke`    | Linux (GKE pods)     | Production   | minibox | --                          | <!-- src: crates/minibox/src/adapters/gke.rs -->
| `colima` | macOS/Linux (Colima) | Experimental | minibox | --                          | <!-- src: crates/minibox/src/adapters/colima.rs -->
| `smolvm` | macOS/Linux (SmolVM) | Experimental | minibox | Yes (all platforms)         | <!-- src: crates/minibox/src/adapters/smolvm.rs -->
| `krun`   | macOS/Linux (krun)   | Experimental | macbox  | Fallback on macOS           | <!-- src: crates/macbox/src/krun/ -->
| `winbox` | Windows              | Stub         | winbox  | --                          | <!-- src: crates/minibox/src/adapters/hcs.rs (stub) -->

---

## Capability Matrix

| Feature                 | native | gke  | colima  | smolvm | krun | winbox |
| ----------------------- | ------ | ---- | ------- | ------ | ---- | ------ |
| **Container lifecycle** |        |      |         |        |      |        |
| pull                    | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox-core/src/image/registry.rs -->
| run                     | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/daemon/handler/run.rs -->
| stop                    | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/daemon/handler/lifecycle.rs -->
| rm                      | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/daemon/handler/lifecycle.rs -->
| ps                      | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/daemon/handler/lifecycle.rs -->
| pause/resume            | Yes    | No   | No      | No     | No   | No     | <!-- src: crates/minibox/src/adapters/limiter.rs (CgroupV2Limiter) -->
| restart                 | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/daemon/handler/lifecycle.rs -->
| exec (-it)              | Yes    | No   | Limited | No     | No   | No     | <!-- src: crates/minibox/src/adapters/exec.rs (NativeExecRuntime); colima via limactl SSH tunnel -->
| logs                    | Yes    | No   | Limited | No     | No   | No     | <!-- src: crates/minibox/src/daemon/handler/logs.rs; colima via limactl -->
| events                  | Yes    | Yes  | No      | No     | No   | No     | <!-- src: crates/minibox-core/src/events.rs (EventSink/EventSource) -->
| **Image management**    |        |      |         |        |      |        |
| Docker Hub v2           | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox-core/src/image/registry.rs -->
| ghcr.io                 | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/adapters/ghcr.rs -->
| Parallel layer pull     | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox-core/src/image/registry.rs (pull_image, parallel layer fetch) -->
| prune / rmi             | Yes    | No   | No      | No     | No   | No     | <!-- src: crates/minibox-core/src/image/gc.rs (ImageGarbageCollector) -->
| push (exp)              | Yes    | Yes  | Yes     | No     | No   | No     | <!-- src: crates/minibox/src/adapters/push.rs; colima: crates/minibox/src/adapters/colima_push.rs -->
| commit (exp)            | Yes    | No   | Yes     | No     | No   | No     | <!-- src: crates/minibox/src/adapters/commit.rs -->
| build (exp)             | Yes    | No   | Yes     | Yes    | No   | No     | <!-- src: crates/minibox-core/src/domain.rs (ImageBuilder) -->
| **Isolation**           |        |      |         |        |      |        |
| PID namespace           | Yes    | No   | Lima VM | VM     | VM   | No     | <!-- src: crates/minibox/src/container/namespace.rs (native); provided by Lima/smolvm/krun VM -->
| Mount namespace         | Yes    | No   | Lima VM | VM     | VM   | No     | <!-- src: crates/minibox/src/container/namespace.rs -->
| Network namespace       | Yes    | No   | Lima VM | VM     | VM   | No     | <!-- src: crates/minibox/src/container/namespace.rs -->
| UTS namespace           | Yes    | No   | Lima VM | VM     | VM   | No     | <!-- src: crates/minibox/src/container/namespace.rs -->
| IPC namespace           | Yes    | No   | Lima VM | VM     | VM   | No     | <!-- src: crates/minibox/src/container/namespace.rs -->
| cgroups v2              | Yes    | No   | Lima VM | VM     | No   | No     | <!-- src: crates/minibox/src/adapters/limiter.rs (CgroupV2Limiter) -->
| Overlay FS              | Yes    | Copy | nerdctl | No     | No   | No     | <!-- src: crates/minibox/src/adapters/filesystem.rs (OverlayFilesystem) -->
| **Networking**          |        |      |         |        |      |        |
| Bridge (exp)            | Yes    | No   | No      | No     | No   | No     | <!-- src: crates/minibox/src/adapters/network/bridge.rs (BridgeNetwork) -->
| Port forwarding         | No     | No   | No      | No     | No   | No     |
| DNS                     | No     | No   | No      | No     | No   | No     |
| **Mounts & Privileges** |        |      |         |        |      |        |
| Bind mounts (`-v`)      | Yes    | No   | No      | No     | No   | No     | <!-- src: crates/minibox/src/daemon/handler/run.rs -->
| Privileged mode         | Yes    | No   | No      | No     | No   | No     | <!-- src: crates/minibox/src/daemon/handler/run.rs -->
| **Security**            |        |      |         |        |      |        |
| SO_PEERCRED auth        | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/daemon/server.rs (is_authorized) -->
| Tar path validation     | Yes    | Yes  | Yes     | Yes    | Yes  | Yes    | <!-- src: crates/minibox-core/src/image/layer.rs (validate_tar_entry_path) -->
| Setuid stripping        | Yes    | Yes  | Yes     | Yes    | Yes  | Yes    | <!-- src: crates/minibox-core/src/image/layer.rs (mode & 0o777) -->
| Device node rejection   | Yes    | Yes  | Yes     | Yes    | Yes  | Yes    | <!-- src: crates/minibox-core/src/image/layer.rs (Block/Char check) -->
| Layer digest verify     | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox-core/src/image/registry.rs -->
| Request frame limits    | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/daemon/server.rs (MAX_REQUEST_SIZE) -->
| Env redaction in logs   | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/daemon/telemetry/traces.rs -->
| **Execution integrity** |        |      |         |        |      |        |
| Execution manifest      | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/daemon/handler/manifest.rs -->
| manifest get/verify     | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox-core/src/domain/execution_manifest.rs -->
| Admission policy gate   | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox-core/src/domain/execution_policy.rs -->
| **State persistence**   |        |      |         |        |      |        |
| Records survive restart | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/minibox/src/daemon/state.rs (DaemonState) -->
| PID reconciliation      | Yes    | No   | No      | No     | No   | No     | <!-- src: crates/minibox/src/daemon/state.rs -->
| **Observability**       |        |      |         |        |      |        |
| Structured tracing      | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/miniboxd/src/main.rs (tracing subscriber init) -->
| OTLP export (opt-in)    | Yes    | Yes  | Yes     | Yes    | Yes  | No     | <!-- src: crates/miniboxd/src/main.rs (otel feature gate) -->

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
  (see `crates/minibox/src/adapters/gke.rs:GkeRuntime`) and a
  no-op resource limiter. Designed for running inside unprivileged
  GKE pods where namespaces and cgroups are unavailable.
- **`colima` adapter** delegates to `nerdctl`/`limactl` inside a
  Lima VM
  (see `crates/minibox/src/adapters/colima.rs:ColimaRuntime`).
  Exec and logs are limited because they go through Lima's SSH
  tunnel. Push, commit, and build are wired via
  `ColimaImagePusher`, `OverlayCommitAdapter`, and
  `MiniboxImageBuilder`.
- **`smolvm` adapter** is the **default** when `MINIBOX_ADAPTER`
  is unset and the `smolvm` binary is present on PATH
  (see `crates/miniboxd/src/main.rs:select_adapter`). Falls back
  to `native` on Linux or `krun` on macOS when the binary is
  absent. Lightweight Linux VMs with subsecond boot
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
