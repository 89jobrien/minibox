# CHANGELOG Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create `CHANGELOG.md` at the repo root, backfilled from the initial commit using Keep a Changelog format, with 10 version entries grouped by feature milestone.

**Architecture:** Single markdown file at repo root. Each version section contains categorized entries (Added/Changed/Fixed/Security) derived from the commit-to-version mapping in the spec. Entries are human-readable prose, not raw commit messages. Newest version first.

**Tech Stack:** Markdown, Keep a Changelog format (https://keepachangelog.com/en/1.0.0/), git log for commit details.

**Spec:** `docs/superpowers/specs/2026-03-17-changelog-design.md`

---

## File Structure

- **Create:** `CHANGELOG.md` — changelog at repo root
- **Modify:** `.gitignore` — add `!CHANGELOG.md` exception (\*.md is currently ignored)

---

### Task 1: Add CHANGELOG.md exception to .gitignore

`.gitignore` has `*.md` with explicit exceptions. CHANGELOG.md needs to be added.

**Files:**

- Modify: `.gitignore`

- [ ] **Step 1: Add the exception**

In `.gitignore`, after the existing `!TESTING.md` line, add:

```
!CHANGELOG.md
```

- [ ] **Step 2: Commit**

```bash
git add .gitignore
git commit -m "chore: allow CHANGELOG.md in gitignore"
```

---

### Task 2: Create CHANGELOG.md with [Unreleased] and v0.0.10

Start from the top of the file and work downward, one version at a time.

**Files:**

- Create: `CHANGELOG.md`

- [ ] **Step 1: Create the file with header and [Unreleased]**

```markdown
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
```

- [ ] **Step 2: Verify file exists and renders correctly**

```bash
cat CHANGELOG.md
```

Expected: header + [Unreleased] + v0.0.10 section visible

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add CHANGELOG with v0.0.10 (CI, clippy, rustls)"
```

---

### Task 3: Add v0.0.9 — Test pyramid

**Files:**

- Modify: `CHANGELOG.md`

- [ ] **Step 1: Append v0.0.9 section after v0.0.10**

```markdown
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
```

- [ ] **Step 2: Verify**

```bash
grep -c "^## \[" CHANGELOG.md
```

Expected: `3` (Unreleased + v0.0.10 + v0.0.9)

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add CHANGELOG v0.0.9 (test pyramid)"
```

---

### Task 4: Add v0.0.8 — Benchmark tooling

**Files:**

- Modify: `CHANGELOG.md`

- [ ] **Step 1: Append v0.0.8 section**

```markdown
## [v0.0.8] - 2026-03-16

### Added

- Benchmark tooling (`bench/`) with CLI config, command runner, test suites, report writers, stats helper, and dry-run mode.
- Benchmark report schema for structured JSON output.
- Suite selection and per-suite reporting; skip stats on failed runs.
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add CHANGELOG v0.0.8 (benchmark tooling)"
```

---

### Task 5: Add v0.0.7 — Cgroup v2 delegation fixes

**Files:**

- Modify: `CHANGELOG.md`

- [ ] **Step 1: Append v0.0.7 section**

```markdown
## [v0.0.7] - 2026-03-16

### Fixed

- Enabled cgroup subtree controllers before writing resource limits, fixing permission errors on cgroups v2.
- Introduced a supervisor leaf cgroup so the daemon can delegate controllers to container sub-cgroups.
- Pointed the cgroup root at the delegated subgroup; enabled `DelegateSubgroup` in the systemd unit.
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add CHANGELOG v0.0.7 (cgroup v2 fixes)"
```

---

### Task 6: Add v0.0.6 — Ops/systemd deployment

**Files:**

- Modify: `CHANGELOG.md`

- [ ] **Step 1: Append v0.0.6 section**

```markdown
## [v0.0.6] - 2026-03-16

### Added

- Justfile with `sync`, `build`, `smoke`, and `test` recipes for common development workflows.
- systemd unit file for `miniboxd` with cgroup delegation and `DelegateSubgroup` support.
- `tmpfiles.d` config to create the runtime socket directory at `/run/minibox/` on boot.
- Install script to deploy the daemon and CLI binaries with systemd setup.
- systemd slice (`minibox.slice`) for resource isolation; allow safe absolute symlinks in the slice.
- systemd cgroup controller delegation; removed unsupported `DelegateControllers` option.
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add CHANGELOG v0.0.6 (ops/systemd)"
```

---

### Task 7: Add v0.0.4 and v0.0.5 — GKE adapter & DomainError patterns

Note: v0.0.5 has no independent commits (themes merged into v0.0.4 split at GKE adapter commit). Use a single combined section labeled v0.0.4, omit v0.0.5 as a standalone heading.

**Files:**

- Modify: `CHANGELOG.md`

- [ ] **Step 1: Append combined v0.0.4/v0.0.5 section**

```markdown
## [v0.0.4] - 2026-03-16

### Added

- GKE unprivileged adapter suite using `proot`, copy-FS, and a no-op resource limiter for rootless/GKE environments. Selected via `MINIBOX_ADAPTER=gke`.
- `RuntimeCapabilities` struct and `capabilities()` method on `ContainerRuntime` trait for runtime feature detection.
- In-memory container state tracking that survives handler restarts within a daemon session (note: state is still lost on daemon process exit).

### Changed

- Adopted `Dyn` type aliases and structured `DomainError` variants for cleaner error handling across adapters.
- Enforced Linux-only compilation via `compile_error!` macro instead of a runtime cfg gate.
- Extracted `miniboxd` as its own lib crate; modernized format strings throughout.
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add CHANGELOG v0.0.4 (GKE adapter, DomainError)"
```

---

### Task 8: Add v0.0.3 — Hexagonal architecture & cross-platform adapters

**Files:**

- Modify: `CHANGELOG.md`

- [ ] **Step 1: Append v0.0.3 section**

```markdown
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
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add CHANGELOG v0.0.3 (hexagonal arch, adapters)"
```

---

### Task 9: Add v0.0.2 and v0.0.1

**Files:**

- Modify: `CHANGELOG.md`

- [ ] **Step 1: Append v0.0.2 section**

```markdown
## [v0.0.2] - 2026-03-15

### Security

- Fixed critical vulnerabilities (CVSS 7.5–9.8): Zip Slip path traversal in tar extraction, symlink escape in overlay filesystem setup.
- Implemented high-priority hardening: `SO_PEERCRED` Unix socket authentication (root-only), manifest/layer size limits (10 MB / 1 GB / 5 GB total), setuid/setgid bit stripping, device node rejection.
```

- [ ] **Step 2: Append v0.0.1 section**

```markdown
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
```

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add CHANGELOG v0.0.2 and v0.0.1 (security hardening, initial runtime)"
```

---

### Task 10: Final validation

- [ ] **Step 1: Count all version sections**

```bash
grep "^## \[" CHANGELOG.md
```

Expected output:

```
## [Unreleased]
## [v0.0.10] - 2026-03-17
## [v0.0.9] - 2026-03-16
## [v0.0.8] - 2026-03-16
## [v0.0.7] - 2026-03-16
## [v0.0.6] - 2026-03-16
## [v0.0.4] - 2026-03-16
## [v0.0.3] - 2026-03-16
## [v0.0.2] - 2026-03-15
## [v0.0.1] - 2026-03-15
```

- [ ] **Step 2: Verify no raw commit messages leaked in**

```bash
grep -E "^- [0-9a-f]{7,}" CHANGELOG.md
```

Expected: no output

- [ ] **Step 3: Verify file is tracked by git**

```bash
git status CHANGELOG.md
```

Expected: `nothing to commit` (all changes committed)

- [ ] **Step 4: Check .gitignore doesn't block it**

```bash
git check-ignore -v CHANGELOG.md
```

Expected: no output (file is not ignored; it is tracked)

```bash
git ls-files CHANGELOG.md
```

Expected: `CHANGELOG.md`
