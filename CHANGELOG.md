# Changelog

All notable changes to minibox are documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versions follow [Semantic Versioning](https://semver.org/).

---

## [Unreleased]

### Security

- CNI plugin lookup now rejects plugin types that are not a single normal path component,
  closing a path-traversal vector in `find_plugin_binary`.
- Colima `commit`/`save` archive import now resolves `config`/`layers` entries with a
  canonicalization check against the extraction root, rejecting archive members that
  escape via `..` or absolute paths.

### Changed

- `xtask coverage-check`'s handler function-coverage threshold raised from 61% to the
  documented 80% target; enforced in CI on Linux via a new `handler-coverage` job in
  `stability-gates.yml`.
- CI's main pipeline job now runs `cargo xtask verify` directly instead of
  `cargo xtask ci --fail-fast`.
- `stability-gates.yml` now also runs on pushes to `develop`.
- Stability checklist consolidated from 7 to 6 mandatory gates (former advisory-adjacent
  gate 7 folded into the advisory list); `STABILITY_CHECKLIST.mbx.md`, `SUPPORT_TIERS.mbx.md`,
  and `CONTRIBUTING.md` updated to reflect gate 2 (handler coverage) now passing at 92.41%.
- `SmolVmRegistry::vm_exec` and `SmolVmRuntime::vm_exec` deduplicated into a shared
  `run_vm_exec` free function in `crates/minibox/src/adapters/smolvm.rs`.

### Fixed

- `crates/macbox/src/vz/adapter.rs`'s trait-bound compile check for `FilesystemProvider`
  was missing its import, silently disabling clippy's `--all-targets` pass on `macbox`;
  fixing it surfaced 28 previously-masked clippy violations across `vz/agent_init.rs`,
  `vz/proxy.rs`, and `vz/vm.rs`, now resolved.

### Removed

- `crates/minibox/src/domain.rs` (984 lines) deleted: an orphaned, never-`mod`-declared
  duplicate of `minibox-core`'s domain capability types, shadowed at compile time by
  `lib.rs`'s `pub use minibox_core::domain;` and already stale relative to it (missing
  `ContainerState::Paused`, `DomainError::ContainerNotStopped`, and several
  `ContainerSpawnConfig` fields).

---

## [v0.31.0] - 2026-05-26

### Added

**MCP control surface:**

- `minibox-mcp` stdio server for agent workflows, with tools for doctor, container listing,
  image listing, logs, manifests, image pulls, runs, stops, and removals.
- Policy gates for higher-risk MCP actions. Mutations, bind mounts, privileged runs, and host
  networking stay disabled unless explicitly enabled with `MINIBOX_MCP_ALLOW_*` environment
  variables.

**SmolVM local image loading:**

- SmolVM can load local OCI image tarballs into the VM image cache, making locally built images
  available without first pushing them to a registry.

**Benchmark and regression tooling:**

- Expanded hot-path benchmark coverage and regression checks.
- `just bench`, `just bench-check`, and `just bench-baseline` workflows for local performance
  runs and baseline comparison.

**Showcase and schemas:**

- End-to-end showcase suite and narrated demo workflow for container lifecycle behavior.
- Generated CLI schema for tool integration and validation.

**Testing documentation:**

- `TESTING.md` now provides a full test strategy reference, including platform/root/CI matrices,
  per-category commands, helper guidance, coverage maps, and writing conventions.

**Backup workflow:**

- SOPS-backed rustic backup workflow for maintainers, including `just` recipes to materialize,
  initialize, and verify the local backup repository without committing plaintext secrets.

### Changed

- README and core docs now reflect current adapter support, command names, feature coverage,
  and known limitations.
- Platform capability docs now include the MCP control surface and current Native, GKE, Colima,
  SmolVM, Krun, and Winbox support.
- CLI command paths now use richer diagnostics for daemon errors, missing responses, and
  unexpected responses.

### Fixed

- `mbx diagnose` now includes recent container logs, giving users more context while debugging
  failed or unexpected container states.
- `mbx pause` and `mbx resume` terminal response handling is more reliable.
- `mbx ps` polling output parsing is more robust.
- Paused container state now persists across daemon restarts.
- SmolVM command execution avoids blocking the async runtime during VM operations.
- SmolVM local image loading now tags loaded images and makes them available inside the VM cache.
- CI and quality gates are more reliable across protocol drift checks, clippy behavior, SARIF
  output, rustqual reporting, and stability validation.
- `cargo xtask doctor` now folds in the tool, secret-manager auth, and smolvm checks previously
  only available via `scripts/preflight.nu`, giving a single canonical environment validation path.
- Image registry pull now returns typed `ManifestTooLarge` and `LayerTooLarge` errors instead of
  a generic failure, giving clearer diagnostics when pull size limits are exceeded.
- The GKE proot adapter's filesystem-copy and process-spawn error messages now render paths with
  `Display` instead of `Debug` formatting, matching this repo's tracing/error conventions.

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
