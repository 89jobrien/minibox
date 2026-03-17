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

### Entry Style

Write sentences from the user/developer perspective. Example:

> Added GKE unprivileged adapter suite using proot, copy-FS, and no-op limiter for rootless environments.

Not: `feat: Add GKE unprivileged adapter suite (proot, copy FS, no-op limiter)`

### Commit Inclusion Policy

- **doc-only and chore commits may be omitted** from the changelog unless they reflect a meaningful change visible to a developer using the project (e.g., adding CLAUDE.md or TESTING.md is worth a brief entry; fixing `.worktrees` gitignore is not).
- Merge commits are omitted.
- The spec commit itself (`25acbe3`) is omitted (internal tooling).

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

All commits listed. Omittable commits (doc/chore/merge with no user-visible impact) are marked with `[omit]`.

### v0.0.1 — Initial runtime
- cc51b1b — Initial commit

### v0.0.2 — Security hardening
- 8ea4f73 — Fix critical vulnerabilities (CVSS 7.5-9.8)
- 2fc7036 — High-priority hardening measures
- abb1981 — `[omit]` Security fixes documentation

### v0.0.3 — Hexagonal architecture refactor
- 9056f19 — Domain layer (Phase 1/5)
- 66a60cd — Infrastructure adapters (Phase 2/5)
- 1fd1638 — Dependency injection to handlers (Phase 3/5)
- 12305de — Unit tests with mock implementations (Phase 5/5)
- 9adc6a6 — `[omit]` Add CLAUDE.md project documentation
- 71c8c3f — Integration tests with real infrastructure
- 26d4bed — Cross-platform adapters (Windows/macOS stubs)
- ed70022 — Networking, TTY, exec, logs, state traits + performance benchmarks
- c26061b — Cross-platform build support and trait overhead validation
- 955a6d3 — `[omit]` Remove emojis from documentation
- 08d9852 — `[omit]` README update reflecting hexagonal architecture
- 8aa50a9 — Cross-platform conformance test suite
- 6e3127e — Comprehensive security framework (path validation, tar safety)
- 3dcbecc — Colima/Lima adapter for native macOS support
- 1822f53 — Fix clippy lints and add license declarations
- 5e832ed — `[omit]` Add test results and validation report
- 6b9f9b7 — `[omit]` Zombienet-SDK architectural pattern analysis
- 08ac7d1 — `[omit]` README improvements

### v0.0.4 — Cross-platform adapters & trait expansion

Note: commits 9056f19–08ac7d1 above are grouped into v0.0.3 as part of the same arch push; v0.0.4 continues from here.

- d57b1b3 — GKE unprivileged adapter suite (proot, copy FS, no-op limiter)
- d457ac3 — `[omit]` Add GKE adapter to README
- f66fffd — `[omit]` Trim README acknowledgment
- 29c17f1 — Adopt Dyn type aliases and structured DomainError variants
- cf73152 — Add RuntimeCapabilities and `capabilities()` to ContainerRuntime trait
- 96d6b09 — Add in-memory container state persistence across daemon restarts (note: this does NOT persist across process restarts; state is lost on daemon exit)
- 9be71a8 — Enforce Linux-only compilation via `compile_error!` (replaces runtime cfg gate)
- 5a52591 — Extract miniboxd as a lib crate; modernize format strings

### v0.0.5 — GKE adapter & DomainError patterns

(See mapping note above — v0.0.4 and v0.0.5 themes merged; commits are split at d57b1b3.)

### v0.0.6 — Ops/systemd deployment
- d738fcf — Add Justfile with sync, build, smoke, and test recipes
- beca3d0 — `[omit]` Add ops runtime plan
- 43a9ee3 — Add systemd unit for miniboxd
- cd613dc — Add tmpfiles config for runtime socket dir
- febecdb — Add install script for systemd setup
- 04c47ab — `[omit]` Add ops runtime instructions
- 3bcda7f — Install minibox CLI with systemd setup
- 740566d — `[omit]` Add usage guide
- 7731e96 — `[omit]` Allow USAGE.md in gitignore
- d3ad3ef — `[omit]` Add VPS usage guide
- 3ff8f43 — Add systemd slice and allow safe absolute symlinks
- a30159e — Delegate cgroup controllers via systemd
- 33e606c — Remove unsupported DelegateControllers option

### v0.0.7 — Cgroup v2 delegation fixes
- 6c08f14 — `[omit]` Document cgroup debug findings
- 922272b — Enable cgroup subtree controllers before writing limits
- a93fcdd — Enable DelegateSubgroup for miniboxd
- 30dcb73 — Point cgroup root at delegated subgroup
- d89fc6c — `[omit]` Update cgroup findings docs
- 8e97d3f — Fix cgroup v2 delegation via supervisor leaf cgroup
- e46e39f — `[omit]` Update cgroup findings with root cause and fix
- 48f1bf2 — `[omit]` Record cgroup fix success
- 14ef5f9 — `[omit]` Merge cgroup findings update

### v0.0.8 — Benchmark tooling
- d0d4517 — `[omit]` Add benchmark design doc
- ba1f9f3 — `[omit]` Ignore .worktrees
- 10b42a3 — `[omit]` Add e2e test infrastructure design spec
- 0c6309c — `[omit]` Add e2e test infrastructure implementation plan
- 6e29159 — `[omit]` Normalize formatting in docs
- e6be36e — `[omit]` Rustfmt cleanup in miniboxd main.rs
- 1920aba — `[omit]` Note macOS test limitation in CLAUDE.md
- b0ac570 — `[omit]` Improve CLAUDE.md
- f14cba1 — `[omit]` Ignore .worktrees (duplicate)
- e4dae86 — Add benchmark skeleton
- b32feb3 — Add benchmark report schema
- a25bdeb — Add benchmark stats helper
- 9a0355f — Add benchmark CLI config
- e8a7f5c — Add benchmark command runner
- 145515a — Add benchmark test suites
- 4c240df — Add benchmark report writers
- 1521657 — Add benchmark dry-run and main entrypoint
- c29e5a7 — `[omit]` Add benchmark usage docs
- c4d49e7 — Fix benchmark suite selection and reporting
- 58a262e — Skip stats on failed benchmark runs
- 3bf42ee — `[omit]` Update benchmark plan
- b58b06f — `[omit]` Merge commit

### v0.0.9 — Test pyramid: e2e, integration, preflight
- 72f77c8 — Make CLI socket path configurable via `MINIBOX_SOCKET_PATH`
- b76f4c6 — Add preflight host capability probing module
- 21f3b9a — Add Justfile task runner for test workflows (`just test-unit`, `just test-e2e`, etc.)
- 0c9aa06 — Add cgroup v2 integration tests exercising `ResourceLimiter` trait
- 027afd2 — Add daemon+CLI e2e tests with `DaemonFixture` harness
- 41e4204 — `[omit]` Update TESTING.md with full test pyramid
- e5f454e — Add SAFETY comments to unsafe blocks; deepen cgroup cleanup
- 6bc2397 — Add e2e/integration runners and configurable image base
- 2330985 — `[omit]` Update CLAUDE.md

### v0.0.10 — CI, clippy, rustls
- 6cb130f — Add CI, release, and integration GitHub Actions workflows
- 1a4c283 — Add security-critical path validation and tar extraction tests
- 18e0472 — Resolve all clippy warnings for CI
- d346c80 — Resolve Linux-only clippy warnings
- a797629 — Fix collapsible_if in handler.rs
- ecc48dd — Fix test module placement, unit struct defaults, e2e kill capture
- eaa4137 — Narrow security clippy lints to suspicious group only
- a823910 — Suppress dead_code on DaemonFixture fields/methods
- d381eef — Fix unused variable and needless borrow in cgroup tests
- 949ccfa — Switch reqwest to rustls-tls for static musl cross-compilation
