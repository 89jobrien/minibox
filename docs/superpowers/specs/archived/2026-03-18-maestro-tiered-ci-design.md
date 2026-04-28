# Maestro Tiered CI Design

**Date:** 2026-03-18
**Status:** Approved

## Overview

Apply the devloop-based tiered CI model to Maestro — an internal Rust workspace with ~12 core devs on macOS/Linux, PRs with required status checks, and Windows product consumers who receive pre-built artifacts. GHA is the enforcement gate. Local hooks are a developer speed boost installed via onboarding.

---

## Context

| Property                 | Value                                                             |
| ------------------------ | ----------------------------------------------------------------- |
| Team                     | ~12 core devs (macOS + Linux), Windows product consumers          |
| Workflow                 | PRs with required status checks                                   |
| Windows                  | Pre-built artifacts only — not a dev/CI platform                  |
| Platform-specific crates | Some; Linux-only crates excluded from macOS jobs via `-p` scoping |
| Enforcement              | GHA gates PRs; hooks are convenience only                         |

---

## CI Structure

### `ci.yml` — Five parallel jobs, two required tiers

```
Tier 1 (fast, required):
  fmt          ubuntu-latest   ~30s    cargo fmt --all --check
  clippy-linux ubuntu-latest   ~2min   cargo clippy --workspace --all-targets -- -D warnings
  clippy-macos macos-latest    ~2min   cargo clippy -p <cross-platform crates> -- -D warnings

Tier 2 (test, ~8 min, required, parallel with Tier 1):
  test-linux   ubuntu-latest           cargo nextest run --workspace --lib
  test-macos   macos-latest            cargo nextest run -p <cross-platform crates>
```

**Cross-platform crate list:** The `-p` flags for macOS jobs are determined at implementation time by auditing the workspace for crates with Linux-only guards (e.g. `compile_error!()` on non-Linux targets). This list is maintained in the CI files and updated as new platform-specific crates are added. The Maestro equivalent of minibox's `miniboxd` (any crate that wraps Linux-only syscalls) is excluded.

All five jobs required to merge via branch protection rules on `main`.

### `nightly.yml` — Scheduled 03:00 UTC, not blocking

```
security      ubuntu-latest    cargo audit + cargo deny check
bench         ubuntu-latest    cargo build --release + run benchmarks
integration   self-hosted      full e2e test suite (if privileged ops required)
                               or ubuntu-latest if no root/cgroup dependency
```

**Integration runner:** Use `self-hosted` if Maestro's integration tests require root, cgroups, or privileged kernel operations. Use `ubuntu-latest` if they are container/mock-based. This is determined at implementation time.

### `release.yml` — Triggered on tag push (`v*`)

Cross-compile release binaries using a `windows-latest` GHA runner for the Windows target (avoids the MSVC sysroot complexity of cross-compiling from Linux):

```
ubuntu-latest    x86_64-unknown-linux-gnu
ubuntu-latest    aarch64-unknown-linux-gnu   (via cross)
macos-latest     x86_64-apple-darwin
macos-latest     aarch64-apple-darwin
windows-latest   x86_64-pc-windows-msvc
```

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

`lld` is Linux-only — setting it at workflow level breaks macOS jobs. macOS jobs omit `RUSTFLAGS`.

### Toolchain

`rust-toolchain.toml` pins edition 2024 and minimum Rust version. `dtolnay/rust-toolchain@stable` reads it automatically. All devs and CI use the same toolchain without per-workflow overrides.

### Caching

`Swatinem/rust-cache@v2` on all jobs. `taiki-e/install-action@nextest` for pre-built nextest binary.

---

## Local Hooks

Hooks ship as `scripts/install-hooks.sh`, run as part of developer onboarding. Not enforced — GHA is the gate.

| Hook       | Runs                                                            |
| ---------- | --------------------------------------------------------------- |
| pre-commit | `fmt-check` + `lint` + `build-release`                          |
| pre-push   | `nextest --release` (reuses pre-commit artifacts, no recompile) |
| commit-msg | conventional commit warning (non-blocking)                      |

### Justfile targets (standard across all devs)

```
fmt           cargo fmt --all
fmt-check     cargo fmt --all -- --check
lint          cargo clippy <cross-platform crates> -- -D warnings
build-release cargo build --release <all crates>
nextest       cargo nextest run --release <cross-platform crates>
pre-commit    fmt-check + lint + build-release
prepush       nextest
commit msg    git add -A && git commit -m "{{msg}}"
push *args    git push {{args}} && clean-artifacts
```

Coverage and flamegraph (`just coverage`, `just flamegraph`) are available as standalone targets but are not part of the `prepush` gate — on a 12-dev team, flamegraph tooling (`samply`, DTrace) is not uniformly available and would cause hook failures for some devs.

---

## Onboarding

New devs run:

```bash
git clone <maestro>
./scripts/install-hooks.sh
```

No other setup required for CI contribution. Hooks give fast local feedback; PRs are gated by GHA.

---

## Branch Protection Rules

Required status checks on `main`:

- `fmt`
- `clippy-linux`
- `clippy-macos`
- `test-linux`
- `test-macos`

Nightly jobs are not required checks — they feed async signal to the team.

---

## Relationship to Minibox

Maestro and minibox share the same CI tier pattern and Justfile conventions. As minibox features are integrated into Maestro (e.g. minibox as a `ContainerProvider`), the cross-platform crate list for macOS CI jobs will evolve. When a Maestro crate gains a minibox-backed Linux-only implementation, it is added to the Linux-only exclusion list and removed from the macOS `-p` scope. The CI spec is updated in the same PR that introduces the platform guard.

---

## Success Criteria

- fmt failure visible within ~30s of push
- Clippy failure visible within ~2 min of push
- All five CI jobs required for merge
- `RUSTFLAGS` with `lld` does not appear in macOS job environments
- Nightly covers security, benchmarks, and e2e without blocking PRs
- Windows release artifacts produced on tag push via `windows-latest` runner
- New dev productive within one `./scripts/install-hooks.sh` invocation
