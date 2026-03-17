# CHANGELOG Design Spec

**Date:** 2026-03-17
**Status:** Approved

## Goal

Create a `CHANGELOG.md` at the repo root, backfilled from the initial commit, following the [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) format.

## Format

- Standard Keep a Changelog structure
- Sections per version: `Added`, `Changed`, `Fixed`, `Security` (only include sections with entries)
- Newest version at top, oldest at bottom
- `[Unreleased]` placeholder at top for future work
- Human-readable entries (not raw commit messages)

## Versions

| Version | Theme | Date |
|---------|-------|------|
| `[Unreleased]` | Future work placeholder | — |
| `v0.0.10` | CI pipelines, clippy fixes, rustls | 2026-03-17 |
| `v0.0.9` | Test pyramid: e2e, integration, preflight | 2026-03-16 |
| `v0.0.8` | Benchmark tooling | 2026-03-16 |
| `v0.0.7` | Cgroup v2 delegation fixes | 2026-03-16 |
| `v0.0.6` | Ops/systemd deployment | 2026-03-16 |
| `v0.0.5` | GKE adapter & DomainError patterns | 2026-03-16 |
| `v0.0.4` | Cross-platform adapters & trait expansion | 2026-03-16 |
| `v0.0.3` | Hexagonal architecture refactor | 2026-03-16 |
| `v0.0.2` | Security hardening | 2026-03-15 |
| `v0.0.1` | Initial runtime | 2026-03-15 |

## Commit-to-Version Mapping

### v0.0.1 — Initial runtime
- cc51b1b — Initial commit

### v0.0.2 — Security hardening
- 8ea4f73 — Fix critical vulnerabilities (CVSS 7.5-9.8)
- 2fc7036 — High-priority hardening measures
- abb1981 — Security fixes documentation

### v0.0.3 — Hexagonal architecture refactor
- 9056f19 — Domain layer (Phase 1/5)
- 66a60cd — Infrastructure adapters (Phase 2/5)
- 1fd1638 — Dependency injection to handlers (Phase 3/5)
- 12305de — Unit tests with mock implementations (Phase 5/5)
- 71c8c3f — Integration tests with real infrastructure
- 1822f53 — Clippy lints and license declarations

### v0.0.4 — Cross-platform adapters & trait expansion
- 26d4bed — Cross-platform adapters (Windows/macOS)
- ed70022 — Networking, TTY, exec, logs, state traits + benchmarks
- c26061b — Cross-platform build support and trait overhead validation
- 8aa50a9 — Cross-platform conformance test suite
- 6e3127e — Comprehensive security framework (P1)
- 3dcbecc — Colima/Lima adapter for native macOS support

### v0.0.5 — GKE adapter & DomainError patterns
- d57b1b3 — GKE unprivileged adapter suite (proot, copy FS, no-op limiter)
- 29c17f1 — Dyn type aliases and structured DomainError variants
- cf73152 — RuntimeCapabilities and capabilities() to ContainerRuntime trait
- 96d6b09 — Persist container state across daemon restarts
- 9be71a8 — Replace runtime cfg gate with compile_error! for Linux-only enforcement
- 5a52591 — Extract miniboxd lib crate, modernize format strings

### v0.0.6 — Ops/systemd deployment
- d738fcf — Justfile with sync, build, smoke, and test recipes
- 43a9ee3 — systemd unit for miniboxd
- cd613dc — tmpfiles config for runtime socket dir
- febecdb — install script for systemd setup
- 3bcda7f — minibox CLI with systemd setup
- 3ff8f43 — systemd slice and safe absolute symlinks
- a30159e — cgroup controller delegation via systemd
- 33e606c — Remove unsupported DelegateControllers

### v0.0.7 — Cgroup v2 delegation fixes
- 922272b — Enable cgroup subtree controllers before writing limits
- a93fcdd — DelegateSubgroup for miniboxd
- 30dcb73 — Point cgroup root at delegated subgroup
- 8e97d3f — Cgroup v2 delegation via supervisor leaf cgroup

### v0.0.8 — Benchmark tooling
- e4dae86 through 58a262e — Benchmark skeleton, schema, stats, config, runner, suites, writers, dry-run, reporting

### v0.0.9 — Test pyramid: e2e, integration, preflight
- 72f77c8 — CLI socket path configurable via MINIBOX_SOCKET_PATH
- b76f4c6 — Preflight host capability probing module
- 21f3b9a — Justfile task runner for test workflows
- 0c9aa06 — Cgroup v2 integration tests for ResourceLimiter trait
- 027afd2 — Daemon+CLI e2e tests with DaemonFixture harness
- 6bc2397 — E2e/integration runners and configurable image base

### v0.0.10 — CI, clippy, rustls
- 6cb130f — CI, release, and integration workflows
- 1a4c283 — Security-critical path validation and tar extraction tests
- 18e0472 through 949ccfa — Clippy warnings resolved, rustls-tls for static musl cross-compilation
