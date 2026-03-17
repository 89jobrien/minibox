# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

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
