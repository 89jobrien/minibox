# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

## [v0.0.14] - 2026-03-19

### Added

- `macbox` crate: macOS daemon entry point, Colima preflight, paths, adapter wiring, `start()` entry point.
- `winbox` crate: Windows daemon stub with `start()` entry point and Named Pipe path helpers.
- `ServerListener` + `PeerCreds` traits in `daemonbox`: generic `run_server` / `handle_connection`
  accepting any listener, enabling `UnixServerListener` (Linux/macOS) and future `NamedPipeListener` (Windows).
- Platform dispatch in `miniboxd`: Linux calls native handler, macOS calls `macbox::start()`, Windows calls `winbox::start()`.
- Platform-aware default socket/pipe path in `minibox-cli`.
- Architecture diagrams: crate-dependency-graph, hexagonal-architecture, platform-adapter-selection, container-lifecycle.

### Changed

- `miniboxd` no longer requires a `compile_error!` Linux guard — the dispatch is conditional compilation in `main()`.
- `daemonbox` gates `nix` on `cfg(unix)` and adds Windows-compatible stubs for stop/wait operations.

## [v0.0.13] - 2026-03-19

### Added

- `GhcrRegistry` adapter: ghcr.io OCI registry client with `WWW-Authenticate` challenge/response auth flow.
- `ImageRef` type in `minibox-lib/src/image/reference.rs`: parses `[REGISTRY/]NAMESPACE/NAME[:TAG]`
  and routes to the correct registry adapter (Docker Hub or ghcr.io).
- Local image store (`LocalStore`) for reading already-extracted layers without re-pulling.

## [v0.0.12] - 2026-03-18

### Added

- `ContainerOutput` / `ContainerStopped` streaming protocol messages.
- `ephemeral` flag on `RunContainer` requests: CLI streams stdout/stderr in real time and exits with the container's exit code.
- `minibox run` exits with the container exit code.

### Changed

- Container stdout/stderr piped back to client via socket rather than inherited by daemon.

## [v0.0.11] - 2026-03-18

### Added

- `daemonbox` crate extracted from `miniboxd`: `handler.rs`, `state.rs`, `server.rs` moved out of the daemon binary into a standalone macOS-safe library crate.
- `miniboxd/src/lib.rs` re-export shim for backward compatibility; all existing tests continue to compile.
- 9 manifest unit tests: single/list/index manifest parsing, `find_linux_amd64`, error paths.
- macOS CI job in `.github/workflows/ci.yml` running clippy + tests for the four cross-platform crates.

### Changed

- Handler and conformance tests moved from `crates/miniboxd/tests/` to `crates/daemonbox/tests/`.
- `just test-unit` uses per-crate `-p` flags; no longer uses `--workspace --lib` (which fails on macOS).

## [v0.0.10] - 2026-03-17

### Added

- GitHub Actions workflows for CI, release, and integration testing.
- Security-critical tests for path validation (Zip Slip prevention) and tar extraction safety.

### Fixed

- Resolved all clippy warnings blocking CI, including Linux-only lints.
- Narrowed security clippy lints to the `suspicious` group to reduce false positives.
- Fixed test module placement, unit struct defaults, and e2e process kill capture.
- Switched reqwest to `rustls-tls` for static musl cross-compilation support.

## [v0.0.9] - 2026-03-16

### Added

- Preflight host capability probing module to detect cgroups v2, overlay FS, and kernel version at startup.
- Justfile task runner with `just test-unit`, `just test-integration`, `just test-e2e`, and `just doctor` recipes.
- Cgroup v2 integration tests exercising the `ResourceLimiter` trait against real kernel interfaces.
- Daemon+CLI e2e tests using a `DaemonFixture` harness that starts/stops a real daemon subprocess.
- Configurable image base and runner selection for e2e/integration test suites.
- `MINIBOX_SOCKET_PATH` environment variable to override the Unix socket path.

### Fixed

- Added `SAFETY` comments to all unsafe blocks; deepened cgroup cleanup on container exit.

## [v0.0.8] - 2026-03-16

### Added

- Benchmark tooling (`bench/`) with CLI config, command runner, test suites, report writers, stats helper, and dry-run mode.
- Benchmark report schema for structured JSON output.
- Suite selection and per-suite reporting; skip stats on failed runs.

## [v0.0.7] - 2026-03-16

### Fixed

- Enabled cgroup subtree controllers before writing resource limits, fixing permission errors on cgroups v2.
- Introduced a supervisor leaf cgroup so the daemon can delegate controllers to container sub-cgroups.
- Pointed the cgroup root at the delegated subgroup; enabled `DelegateSubgroup` in the systemd unit.

## [v0.0.6] - 2026-03-16

### Added

- Justfile with `sync`, `build`, `smoke`, and `test` recipes for common development workflows.
- systemd unit file for `miniboxd` with cgroup delegation and `DelegateSubgroup` support.
- `tmpfiles.d` config to create the runtime socket directory at `/run/minibox/` on boot.
- Install script to deploy the daemon and CLI binaries with systemd setup.
- systemd slice (`minibox.slice`) for resource isolation; allow safe absolute symlinks in the slice.
- systemd cgroup controller delegation; removed unsupported `DelegateControllers` option.

## [v0.0.4] - 2026-03-16

### Added

- GKE unprivileged adapter suite using `proot`, copy-FS, and a no-op resource limiter for rootless/GKE environments. Selected via `MINIBOX_ADAPTER=gke`.
- `RuntimeCapabilities` struct and `capabilities()` method on `ContainerRuntime` trait for runtime feature detection.
- In-memory container state tracking that survives handler restarts within a daemon session (note: state is still lost on daemon process exit).

### Changed

- Adopted `Dyn` type aliases and structured `DomainError` variants for cleaner error handling across adapters.
- Enforced Linux-only compilation via `compile_error!` macro instead of a runtime cfg gate.
- Extracted `miniboxd` as its own lib crate; modernized format strings throughout.

## [v0.0.3] - 2026-03-16

### Added

- Hexagonal architecture domain layer: `ResourceLimiter`, `FilesystemProvider`, `ContainerRuntime`, and `ImageRegistry` traits in `minibox-lib/src/domain.rs`.
- Infrastructure adapters implementing domain traits for native Linux (namespaces, overlay FS, cgroups v2) and cross-platform stubs (Windows/macOS).
- Dependency injection wired into daemon handlers; mock adapter implementations for unit tests.
- Comprehensive unit tests using mock adapters; integration tests against real Linux infrastructure.
- Cross-platform conformance test suite validating adapter contracts.
- Colima/Lima adapter for running containers on macOS via a Linux VM.
- Comprehensive security framework: path canonicalization, `..` rejection, symlink validation, device node blocking, and setuid/setgid stripping.

### Fixed

- Clippy lints resolved; license declarations added to all crates.

## [v0.0.2] - 2026-03-15

### Security

- Fixed critical vulnerabilities (CVSS 7.5–9.8): Zip Slip path traversal in tar extraction, symlink escape in overlay filesystem setup.
- Implemented high-priority hardening: `SO_PEERCRED` Unix socket authentication (root-only), manifest/layer size limits (10 MB / 1 GB / 5 GB total), setuid/setgid bit stripping, device node rejection.

## [v0.0.1] - 2026-03-15

### Added

- Initial Docker-like container runtime in Rust with daemon (`miniboxd`) and CLI (`minibox`) binaries.
- OCI image pulling from Docker Hub using anonymous token auth and v2 manifest/blob API.
- Linux namespace isolation: PID, mount, UTS, IPC, and network namespaces via `clone(2)`.
- cgroups v2 resource limits: `memory.max` and `cpu.weight` per container.
- Overlay filesystem support: stacked read-only layers plus per-container read-write upper dir.
- Container lifecycle: `pull`, `run`, `ps`, `stop`, `rm` commands over a Unix socket JSON protocol.
- In-memory container state machine: Created → Running → Stopped.
- Background reaper task to detect container exit and update state.
