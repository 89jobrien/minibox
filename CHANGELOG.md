# Changelog

All notable changes to minibox are documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versions follow [Semantic Versioning](https://semver.org/).

---

## [v0.24.0] - 2026-05-07

First public release.

### Added

**Conformance suite:**
- `minibox-conformance` crate — 28 backend-agnostic tests covering `ImageRegistry`,
  `ResourceLimiter`, `ContainerRuntime`, and `DaemonState` contracts
- `cargo xtask test-conformance` — runs suite and emits Markdown + JSON reports
- `BackendDescriptor` and `BackendCapability` flags — structured adapter self-description

**crux plugin:**
- `minibox-crux-plugin` binary — exposes minibox ops (run/stop/ps/exec/pause/resume/
  image-ls/image-rm) over JSON-RPC stdio for agent integration
- 10-test integration suite for the plugin

**macOS adapter improvements:**
- smolvm set as default macOS adapter; krun as automatic fallback when smolvm binary absent
- ghcr.io, event broker, and metrics wired into krun and smolvm adapter suites
- `MINIBOX_ADAPTER` env var: unrecognized values produce structured error with valid options
- smolvm hello-world agent demo script

**Container restart:**
- `RunCreationParams` stored in `ContainerRecord` — enables restart without re-specifying flags
- `handle_update` Wave 3: restart phase implemented

**Daemon hardening:**
- Exec input validation hardened; socket auth tightened
- `fork()` in exec path: regression guard added (`spawn_blocking` invariant enforced)
- `ImageList` added to `is_terminal_response`; exhaustiveness guard added

**CI:**
- Gitea CI: `GITEA_URL` moved to repository secret (no hardcoded addresses)
- Permissions blocks added to workflow files
- Reviewdog for inline PR lint comments

**Developer tooling:**
- `cargo xtask doctor` extended with preflight checks and `check-protocol-sites` subcommand
- `mbx diagnose <id>` subcommand — structured container diagnostic output
- `just` recipes aligned with xtask; stale crate names corrected throughout

### Fixed

- `fork()` UB in pty exec path replaced with `nsenter` + `Command`
- `push_auth_from_credentials` scope error in `push.rs`
- `RegistryCredentials::Token` now sent as Bearer auth in `OciPushAdapter`
- `panic!(IPv6 not supported)` replaced with `bail!` in `bridge.rs`
- Layer digest propagated correctly in task failure paths
- Unused `_label` stub field removed from registry router tests
- macOS socket bind/chmod/signal boilerplate extracted to helper

### Changed

- Workspace version: `0.23.0` → `0.24.0`
- Crate count: 9 → 10 (added `minibox-conformance`)
- CI split: `ci.yml` → `pr.yml` + `merge.yml`
- Default macOS adapter: `krun` → `smolvm` (krun remains automatic fallback)

---

## [v0.23.0] - 2026-04-28

### Added

**Workspace consolidation:**
- 13-crate workspace reduced to 9 crates (7-phase consolidation)
- `minibox-oci` + `minibox-client` absorbed into `minibox-core`
- `daemonbox` + `linuxbox` merged into unified `minibox` crate
- `minibox::testing` module — unified mock and fixture infrastructure

**Adapter registry:**
- `miniboxd::adapter_registry` — typed `AdapterSuite` enum, `AdapterInfo` metadata,
  structured `AdapterSelectionError`
- Startup logs: selected adapter and available options emitted as structured fields

**State management:**
- Container state reconciliation on daemon restart — stale Running containers marked Orphaned
- `ProcessChecker` trait + `KillProcessChecker` (unix-gated)
- Disk-persisted state survives daemon restarts

**macOS adapters:**
- krun fully wired: `KrunRuntime`, `KrunRegistry`, `KrunFilesystem`, `KrunLimiter`
- `SmolVM` adapter suite wired into miniboxd
- QEMU cross-platform VM runner — `HostPlatform` detection, `VmRunner`/`VmHandle`
- `cargo xtask build-vm-image` — platform-aware cross-compilation + Alpine kernel assembly

**OCI image push (GKE):**
- `OciPushAdapter` wired into GKE adapter suite via `ImagePusher` port

**Testing:**
- Security regression suite: tar traversal, symlink escape, path validation, socket auth
- Handler error-path coverage raised to 80%+
- Proptest expansion: all protocol variants covered
- Cross-platform protocol e2e tests

**Infrastructure:**
- `cargo xtask pre-commit` — fmt-check + clippy + release build (macOS-safe gate)
- Protocol-drift detection workflow
- Three-tier git workflow: `main` → `next` (auto) → `stable` (manual)

### Fixed

- IPv6 panic replaced with `bail!` in `IpAllocator`
- `KillProcessChecker` gated behind `cfg(unix)` for macOS compatibility
- `ContainerState` unified in `minibox-core` (no local duplicates)
- `HandlerDependencies` decomposed into ISP sub-structs
- Stale `linuxbox::` refs cleaned up across crate boundaries

### Changed

- `linuxbox` crate renamed to `minibox` (2026-04-21)
- `minibox-cli` renamed to `mbx`
- `FilesystemProvider` split into `RootfsSetup` + `ChildInit` (ISP)

---

## [v0.2.0] - 2026-04-14

### Added

**Container features:**
- `exec` — run commands in existing containers via `setns` + `NativeExecRuntime`
- Named containers — `--name` on `run`; name column in `ps`; `exec` by name
- Log capture — `minibox logs <id>`; stdout/stderr stored per container
- PTY/interactive mode — `-it` on `run` and `exec`; raw terminal, stdin relay, SIGWINCH
- Container events — `minibox events`; `SubscribeEvents` protocol; lifecycle event emission
- Image GC and leases — `minibox prune` / `minibox rmi`; `ImageGc`, `DiskLeaseService` traits
- Bridge networking — veth pairs, NAT via iptables DNAT; `MINIBOX_NETWORK_MODE=bridge`
- Bind mounts — `-v`/`--volume`/`--mount`; path validation against traversal
- Privileged mode — `--privileged`; curated capability whitelist
- OCI image push — `OciPushAdapter` implementing OCI Distribution Spec push
- Container commit — `ContainerCommitter` trait; overlay upperdir snapshot to new image
- Image build — `ImageBuilder` trait; `DockerfileParser`; `MiniboxImageBuilder`
- Container pause/resume — cgroup freeze/thaw via SIGSTOP/SIGCONT

**Observability:**
- OpenTelemetry tracing — OTLP exporter; handlers instrumented with spans
- Prometheus metrics — `/metrics` HTTP endpoint; `MetricsRecorder` domain port
- Structured tracing contract — canonical `key = value` fields, severity rules

**macOS / VZ.framework:**
- VM image pipeline — `cargo xtask build-vm-image`; Alpine aarch64 + cross-compiled agent
- `VzAdapter` — domain traits via JSON-over-newline over vsock
- virtiofs host-path mounts for OCI layers and bind mounts
- macOS Tahoe GCD main-queue dispatch fix for VZ.framework

**Infrastructure:**
- `minibox-macros` proc macros — `as_any!`, `default_new!`, `adapt!`
- `cargo xtask bump` — workspace version bump
- `cargo xtask bench-vps` — VPS bench with explicit `--commit`/`--push` opt-in
- Dual MIT/Apache-2.0 license

**Testing:**
- Backend-agnostic conformance suite with `BackendDescriptor` and `BackendCapability`
- Proptest suite — 33 property-based tests (DaemonState invariants, protocol codec, digest)
- Sandbox tests — 15 shell/Python scenario tests
- DinD integration test — nested miniboxd inside a minibox container

### Fixed

- Absolute symlink rewriting relative to their own directory
- Mount namespace made private before `pivot_root`
- FD collection before close to avoid mid-iteration close in `close_extra_fds`
- `fork()` inside active Tokio runtime gated behind `spawn_blocking`
- Stdin relay fd exhaustion, exec registry leak, SIGWINCH reliability
- Colima: `has_image` uses `docker images --filter`; strips `library/` prefix for nerdctl
- VZ: GCD main-queue dispatch for `connectToPort:completionHandler:`

---

## [v0.1.0] - 2026-03-17

### Added

- Parallel OCI layer pulls — concurrent `tokio::spawn` per layer with progress tracking
- `GhcrRegistry` adapter — ghcr.io client with `WWW-Authenticate` challenge/response
- `ImageRef` type — parses `[REGISTRY/]NAMESPACE/NAME[:TAG]`, routes to correct adapter
- Local image store (`LocalStore`) — reads already-extracted layers without re-pulling
- `macbox` crate — macOS daemon entry point, Colima preflight, adapter wiring
- `winbox` crate — Windows daemon stub
- Platform dispatch in `miniboxd` — Linux → native, macOS → `macbox::start()`,
  Windows → `winbox::start()`
- `minibox-macros` crate — `as_any!`, `default_new!`, `adapt!` proc macros
- Colima adapter wired into daemon
- Architecture diagrams — crate dependency graph, hexagonal architecture, lifecycle flow
- Streaming protocol: `ContainerOutput`/`ContainerStopped`; `ephemeral` flag on run
- `minibox run` exits with the container's exit code
- `xtask` task runner: `pre-commit`, `prepush`, `test-unit`, `test-property`, `test-e2e`
- Benchmark tooling — codec and adapter suites, JSON report schema
- Structured tracing contract

### Fixed

- `daemonbox` gates `nix` on `cfg(unix)`; Windows-compatible stubs added
- `io.max` PID 0 validation; absolute symlink rewriting corrected

---

## [v0.0.2] - 2026-03-15

### Security

- Fixed Zip Slip path traversal in tar extraction
- `SO_PEERCRED` Unix socket authentication (root-only access)
- Manifest and layer size limits; setuid/setgid stripping; device node rejection

---

## [v0.0.1] - 2026-03-15

Initial release.

### Added

- `miniboxd` daemon + `minibox` CLI over Unix socket JSON protocol
- OCI image pulling from Docker Hub with anonymous token auth
- Linux namespace isolation (PID, mount, UTS, IPC, network)
- cgroups v2 resource limits (`memory.max`, `cpu.weight`)
- Overlay filesystem — stacked read-only layers + per-container read-write upper dir
- Container lifecycle: `pull`, `run`, `ps`, `stop`, `rm`
- Container state machine: Created → Running → Stopped
- Background reaper task

[v0.24.0]: https://github.com/89jobrien/minibox/releases/tag/v0.24.0
[v0.23.0]: https://github.com/89jobrien/minibox/releases/tag/v0.23.0
[v0.2.0]: https://github.com/89jobrien/minibox/releases/tag/v0.2.0
[v0.1.0]: https://github.com/89jobrien/minibox/releases/tag/v0.1.0
[v0.0.2]: https://github.com/89jobrien/minibox/releases/tag/v0.0.2
[v0.0.1]: https://github.com/89jobrien/minibox/releases/tag/v0.0.1
