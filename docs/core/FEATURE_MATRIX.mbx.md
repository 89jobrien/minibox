---
source_sha: f5481a9482fbb04690db6b7a52ee8eca9c5fe5e9
sources:
  - crates/minibox/src/daemon/handler
  - crates/minibox/src/adapters/limiter.rs
  - crates/minibox-domain/src/exec.rs
  - crates/minibox-core/src/events.rs
  - crates/minibox-domain/src/events.rs
  - crates/minibox-core/src/image/registry.rs
  - crates/minibox/src/adapters/ghcr.rs
  - crates/minibox-core/src/image/gc.rs
  - crates/minibox-domain/src/image.rs
  - crates/minibox/src/container/namespace.rs
  - crates/minibox/src/adapters/filesystem.rs
  - crates/minibox/src/adapters/network/bridge.rs
  - crates/minibox/src/daemon/server.rs
  - crates/minibox-core/src/image/layer.rs
  - crates/minibox-domain/src/execution_manifest.rs
  - crates/minibox-domain/src/execution_policy.rs
  - crates/minibox/src/daemon/state.rs
  - crates/miniboxd/src/main.rs
  - crates/minibox/src/adapters/gke.rs
  - crates/minibox/src/adapters/colima.rs
  - crates/minibox/src/adapters/smolvm.rs
  - crates/macbox/src/krun
  - crates/macbox/tests/krun_conformance_tests.rs
  - crates/macbox/tests/krun_adapter_conformance.rs
  - crates/macbox/src/vz
  - crates/minibox/src/adapters/docker_desktop.rs
  - crates/mcp
  - crates/minibox-tui
  - crates/mbx
generated: 2026-09-16
---

# Feature Matrix

Per-platform capability breakdown for minibox adapters.

Last updated: 2026-09-18

---

## Adapter Suites

<!-- BEGIN GENERATED: adapter-suites -->
| Adapter | Platforms | Maturity | Default roles |
| --- | --- | --- | --- |
| `colima` | `linux`, `macos` | Experimental | -- |
| `gke` | `linux` | Production | -- |
| `krun` | `linux`, `macos` | Experimental | `macos_fallback` |
| `native` | `linux` | Production | `linux_fallback` |
| `smolvm` | `linux`, `macos` | Experimental | `unix_default` |
| `vz` | `macos` | Blocked | -- |
| `winbox` | `windows` | Stub | -- |
<!-- END GENERATED: adapter-suites -->

[^1]: `native` requires root (UID 0); daemon startup rejects non-root native selection. Linux only
      (`cfg!(target_os = "linux")`). Cgroup v2 and overlay FS require kernel support.
[^2]: `gke` is Linux only (`cfg!(target_os = "linux")`). Unprivileged — no root required.
      Uses proot (ptrace) and copy-based filesystem instead of overlay.
[^3]: `smolvm` is compiled only on Unix (`cfg!(unix)`). Not available on Windows builds.
      Requires the `smolvm` binary on PATH at runtime.
[^4]: `krun` fallback platform-splits: `native` on Linux, `krun` on macOS, when
      `smolvm` binary is absent and `MINIBOX_ADAPTER` is unset.
[^5]: `vz` requires macOS + the `vz` Cargo feature (off by default). Removed
      2026-05-07 (commit `00ee4427`, issue #305) after a Tahoe-beta VZ.framework
      regression (`VZErrorInternal(1)`); code restored 2026-08-15, but the
      adapter is **currently non-functional** — a follow-up minimal repro
      showed `VZLinuxBootLoader` still fails with `VZErrorDomain code=1` on
      macOS 26.4, confirmed against two independent kernel images. See the
      status update at the top of
      `docs/designs/2026-08-15-vz-adapter-revival-design.md`. Bypasses the
      shared `run_daemon()`/`AdapterSuite` dispatch entirely — selected in
      `main()` before the tokio runtime starts, because VM boot needs the OS main
      thread for GCD completion-handler callbacks.

---

## Capability Matrix

<!-- BEGIN GENERATED: adapter-capabilities -->
| Capability | `colima` | `gke` | `krun` | `native` | `smolvm` | `vz` | `winbox` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `admission_policy_gate` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `bind_mounts` | No | No | No | Yes | No | No | No |
| `bridge_network` | No | No | No | Yes | No | No | No |
| `build` | Yes | No | No | Yes | Yes | No | No |
| `cgroups_v2` | Yes | No | No | Yes | Yes | Blocked | No |
| `commit` | Yes | No | No | Yes | No | No | No |
| `device_node_rejection` | Yes | Yes | Yes | Yes | Yes | Blocked | Yes |
| `dns` | No | No | No | Yes | No | Blocked | No |
| `docker_hub_v2` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `environment_redaction` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `events` | No | Yes | No | Yes | No | No | No |
| `exec` | Limited | No | No | Yes | No | No | No |
| `execution_manifest` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `ghcr_io` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `ipc_namespace` | Yes | No | Yes | Yes | Yes | Blocked | No |
| `layer_digest_verification` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `logs` | Limited | No | No | Yes | No | No | No |
| `manifest_get_verify` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `mount_namespace` | Yes | No | Yes | Yes | Yes | Blocked | No |
| `network_namespace` | Yes | No | Yes | Yes | Yes | Blocked | No |
| `otlp_export` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `overlay_filesystem` | Yes | Limited | No | Yes | No | Blocked | No |
| `parallel_layer_pull` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `pause_resume` | No | No | No | Yes | No | No | No |
| `peer_credential_auth` | No | No | No | Yes | No | No | No |
| `pid_namespace` | Yes | No | Yes | Yes | Yes | Blocked | No |
| `pid_reconciliation` | No | No | No | Yes | No | No | No |
| `port_forwarding` | No | No | No | Yes | No | Blocked | No |
| `privileged_mode` | No | No | No | Yes | No | No | No |
| `prune_rmi` | No | No | No | Yes | No | No | No |
| `ps` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `pull` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `push` | Yes | Yes | No | Yes | No | No | No |
| `request_frame_limits` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `restart` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `rm` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `run` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `setuid_stripping` | Yes | Yes | Yes | Yes | Yes | Blocked | Yes |
| `state_persistence` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `stop` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `structured_tracing` | Yes | Yes | Yes | Yes | Yes | Blocked | No |
| `tar_path_validation` | Yes | Yes | Yes | Yes | Yes | Blocked | Yes |
| `uts_namespace` | Yes | No | Yes | Yes | Yes | Blocked | No |
<!-- END GENERATED: adapter-capabilities -->

---

## Source References for Capability Matrix

Key implementation sites backing the "Yes" entries above:

| Feature area | Source |
| --- | --- |
| Container lifecycle (run/stop/rm/ps/restart) | `crates/minibox/src/daemon/handler/lifecycle.rs`, `handler/run.rs`, `handler/stop.rs` |
| pause/resume (native, cgroup.freeze) | `crates/minibox/src/adapters/limiter.rs:CgroupV2Limiter` |
| exec | `crates/minibox/src/daemon/handler/exec.rs`, `crates/minibox-domain/src/exec.rs:ExecRuntime` |
| logs | `crates/minibox/src/daemon/handler/logs.rs` |
| events | `crates/minibox-domain/src/events.rs:EventSink`/`EventSource`; broker adapter in `minibox-core` |
| Image pull (Docker Hub v2 + parallel layers) | `crates/minibox-core/src/image/registry.rs:pull_image` |
| Image pull (ghcr.io) | `crates/minibox/src/adapters/ghcr.rs` |
| prune/rmi | `crates/minibox-core/src/image/gc.rs:ImageGarbageCollector` |
| push | `crates/minibox-domain/src/image.rs:ImagePusher` |
| commit | `crates/minibox-domain/src/image.rs:ContainerCommitter` |
| build | `crates/minibox-domain/src/image.rs:ImageBuilder` |
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
| Execution manifest + verify | `crates/minibox-domain/src/execution_manifest.rs` |
| Admission policy gate | `crates/minibox-domain/src/execution_policy.rs` |
| State persistence + PID reconciliation | `crates/minibox/src/daemon/state.rs:DaemonState` |
| Structured tracing | `crates/miniboxd/src/main.rs` (tracing subscriber init) |
| OTLP export | `crates/miniboxd/src/main.rs` (otel feature gate) |

---

## Control Surfaces

- `mbx` is the primary CLI and connects directly to the daemon Unix socket.
- `minibox-crux-plugin` exposes a JSON-RPC stdio bridge for Crux workflows.
- `minibox-mcp` exposes an MCP stdio server for agent workflows. Its first tool set wraps existing daemon protocol requests for doctor, ps, images, logs, manifest, pull, run, stop, and rm; mutating or higher-risk run options are gated by MCP-specific policy environment variables.
- `minibox-cli` optionally exposes `mbx tui` with `cargo build -p minibox-cli --features tui`;
  the read-only dashboard implementation lives in `minibox-tui`.

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
  `ColimaImagePusher`, `ColimaContainerCommitter`, and
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
  and pass 29 krun-specific conformance tests. Acts as the fallback when
  `smolvm` is unavailable.
- **`vz` adapter** uses Apple's Virtualization.framework directly
  (see `crates/macbox/src/vz/`), communicating with the in-VM
  `minibox-agent` over vsock
  (`crates/macbox/src/vz/vsock.rs`, `proxy.rs`). Opt-in only via
  `MINIBOX_ADAPTER=vz` and the `vz` Cargo feature — not wired into
  `AdapterSuite`'s `build_handler_deps` dispatch like the other
  adapters, because `VZVirtualMachine` construction and its
  completion-handler callbacks must run on the GCD main queue, which
  requires bypassing `#[tokio::main]` entirely
  (see `vz_main()`/`start_vz()` in `crates/miniboxd/src/main.rs` and
  `crates/macbox/src/lib.rs`). Removed 2026-05-07 (issue #305) after
  a macOS 26 Tahoe-beta regression (`VZErrorInternal(1)`); code
  restored 2026-08-15, but **currently non-functional** — a follow-up
  minimal repro (isolated from all minibox configuration, tested
  against two independent kernel images) showed `VZLinuxBootLoader`
  still fails with `VZErrorDomain code=1` on macOS 26.4. The earlier
  belief that the regression was fixed was based on a Lima repro that
  used `VZEFIBootLoader` — a different boot mechanism than this
  adapter needs. See the status update in
  `docs/designs/2026-08-15-vz-adapter-revival-design.md`.
  `exec`/`logs`/`push`/`commit`/`build` are unimplemented, same gaps
  as `krun`.
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
  (see `crates/minibox-domain/src/execution_policy.rs:ExecutionPolicy`).
  Environment variable values are stored as SHA-256 digests --
  never plaintext -- in `execution-manifest.json`
  (see `crates/minibox-domain/src/execution_manifest.rs:ExecutionManifest::seal`).
- **Observability env vars** (daemon startup,
  see `crates/miniboxd/src/main.rs`):
    - `MINIBOX_OTLP_ENDPOINT` -- OTLP trace export endpoint
      (`otel` feature required).
    - `MINIBOX_METRICS_ADDR` -- Prometheus metrics bind address
      (e.g. `0.0.0.0:9090`); `metrics` feature required.
