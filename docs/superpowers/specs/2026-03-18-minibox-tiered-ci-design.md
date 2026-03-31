# Minibox Tiered CI Design

**Date:** 2026-03-18
**Status:** Approved

## Overview

Replace the current single-job `ci.yml` with a two-tier parallel model based on devloop's CI pattern. Tier 1 (fmt + clippy) fails fast in ~2 min; Tier 2 (nextest) runs in parallel and gates merge at ~8 min. A new `nightly.yml` handles expensive work (security audit, benchmarks, integration tests) off the critical path.

Local hooks remain unchanged — they are a speed boost for the developer, not the enforcement gate.

---

## Current State

`ci.yml` has one `linux` job: clippy → nextest. No macOS test job. No nightly. GHA is the gate but doesn't fail fast — a clippy error makes you wait for the full test run to start before you find out.

---

## Target State

### `ci.yml` — Five parallel jobs, two required tiers

```
Tier 1 (fast, required):
  fmt          ubuntu-latest   ~30s    cargo fmt --all --check
  clippy-linux ubuntu-latest   ~2min   cargo clippy --workspace --all-targets -- -D warnings
  clippy-macos macos-latest    ~2min   cargo clippy -p mbx -p minibox-macros
                                                    -p minibox-cli -p daemonbox -- -D warnings

Tier 2 (test, ~8 min, required, parallel with Tier 1):
  test-linux   ubuntu-latest           cargo nextest run --workspace --lib
                                       cargo nextest run -p daemonbox
                                           --test handler_tests --test conformance_tests
  test-macos   macos-latest            cargo nextest run -p mbx -p minibox-macros
                                                        -p minibox-cli -p daemonbox
```

All five jobs required to merge. Tier 1 and Tier 2 start at the same time — a fmt failure is visible in ~30s while tests are still compiling.

**Note on `test-linux` structure:** `cargo nextest run --workspace --lib` covers library unit tests for all crates. The `daemonbox` integration tests (`handler_tests`, `conformance_tests`) are not lib tests — they live under `tests/` and require an explicit `-p daemonbox --test` invocation. This is not redundant; it is two distinct test surfaces.

### `nightly.yml` — Scheduled 03:00 UTC, not blocking

Three independent jobs on the nightly schedule:

```
security      ubuntu-latest   cargo audit + cargo deny check
bench         ubuntu-latest   cargo build --release -p minibox-bench
                              ./target/release/minibox-bench
integration   self-hosted     just test-integration (cgroup tests, Linux + root)
                              just test-e2e-suite (daemon + CLI e2e, Linux + root)
```

`integration` requires a `self-hosted` runner tagged `[self-hosted, linux, privileged]` — the same runner used by the existing `integration.yml`. The nightly runs these as standalone steps (not via `workflow_call`) to avoid coupling to the integration workflow's trigger conditions. `security.yml` continues to run on its own daily schedule independently; nightly does not call it.

### Performance env vars

Applied at the workflow level (all jobs):

```yaml
env:
  CARGO_INCREMENTAL: "0"
  CARGO_REGISTRIES_CRATES_IO_PROTOCOL: sparse
```

Applied at the job level for Linux jobs only (`clippy-linux`, `test-linux`, `bench`, `integration`):

```yaml
env:
  RUSTFLAGS: "-C link-arg=-fuse-ld=lld"
```

`lld` is Linux-only — setting it at workflow level would break macOS jobs. macOS jobs omit `RUSTFLAGS`.

`CARGO_INCREMENTAL=0`: CI caches are near-full rebuilds; incremental adds cache bloat without benefit. Sparse protocol cuts registry fetch from ~30s to ~3s. `lld` cuts link time 2-4x on Linux.

### Toolchain

`rust-toolchain.toml` pins the edition (2024) and minimum Rust version. `dtolnay/rust-toolchain@stable` reads it automatically — no `with: toolchain:` override needed in workflow files. This ensures CI and local builds use the same toolchain.

### Caching

`Swatinem/rust-cache@v2` on all jobs (separate cache per OS + job name). `taiki-e/install-action@nextest` for pre-built nextest binary (~5s install vs 3min `cargo install`).

### Workspace note

`minibox-bench` is a member of the workspace (`crates/minibox-bench/`) used in the nightly bench step.

---

## Local Hooks (unchanged)

| Hook       | Runs                                   | Purpose                                               |
| ---------- | -------------------------------------- | ----------------------------------------------------- |
| pre-commit | `fmt-check` + `lint` + `build-release` | Fast local feedback, optimized binary ready for bench |
| pre-push   | `nextest --release`                    | Reuses pre-commit artifacts, no recompile             |
| commit-msg | conventional commit warning            | Non-blocking format hint                              |

Hooks are a developer convenience. GHA is the gate.

---

## Implementation Scope

**Files to change:**

- `.github/workflows/ci.yml` — split into 5-job tiered structure; add `CARGO_INCREMENTAL`, sparse protocol; scope `RUSTFLAGS`/`lld` to Linux jobs only

**Files to create:**

- `.github/workflows/nightly.yml` — security + bench + integration on schedule

**Files unchanged:**

- `.github/workflows/integration.yml` — self-hosted integration tests (still triggerable manually)
- `.github/workflows/security.yml` — daily security scans (runs independently)
- `.github/workflows/release.yml` — release workflow unchanged
- `Justfile`, `scripts/install-hooks.sh` — local workflow unchanged

---

## Success Criteria

- fmt failure visible within ~30s of push
- Clippy failure visible within ~2 min of push
- All five CI jobs required for merge (branch protection)
- `RUSTFLAGS` with `lld` does not appear in macOS job environments
- Nightly runs integration + security + benchmarks without blocking PRs
- No change to local hook behavior
