# minibox-testers: Conformance Infrastructure Migration

**Date:** 2026-04-21
**Status:** Active
**Phase:** 1 of 3 — Migration only (expansion in subsequent specs)

---

## Overview

Extract all conformance and test infrastructure from `minibox-core` and `daemonbox` into a
dedicated `minibox-testers` crate. The migration keeps all existing conformance tests green and
establishes `minibox-testers` as the single source of truth for test doubles, fixtures, and
conformance report types. Subsequent phases (capability expansion, Queue-based registry) build on
this foundation.

---

## Goals

- One crate owns all test infrastructure: mocks, fixtures, conformance types, report emitter
- Zero duplicated test code across `minibox-core`, `minibox`, `daemonbox`
- All existing conformance tests pass against the migrated infra with no behavioral change
- `minibox-core`'s `test-utils` feature becomes a thin re-export pointing at `minibox-testers`
- `minibox-testers` is a `[dev-dependency]` only — never compiled into production binaries

---

## Non-Goals (Phase 1)

- No new test cases
- No Queue-based `ConformanceRegistry`
- No new capabilities (exec, GC, network, events)
- No changes to `ConformanceCapability` trait shape (that is Phase 2)
- No changes to domain trait definitions in `minibox-core`

---

## Crate Design

### Dependency Direction

```
minibox-core         (domain traits, protocol — unchanged)
    ▲
minibox-testers      (mocks, fixtures, conformance types — dev-only)
    ▲
minibox / daemonbox  (test files consume minibox-testers)
```

`minibox-testers` depends on `minibox-core` (and `minibox` for adapter types used in mocks).
Nothing in production depends on `minibox-testers`.

### Cargo.toml

```toml
[package]
name = "minibox-testers"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false          # dev-only, never published

[dependencies]
minibox-core = { path = "../minibox-core" }
minibox = { path = "../minibox" }
daemonbox = { path = "../daemonbox" }
anyhow.workspace = true
async-trait.workspace = true
serde.workspace = true
serde_json.workspace = true
tempfile = "3"
tokio = { workspace = true, features = ["rt", "macros"] }
```

### Module Layout

```
crates/minibox-testers/
  Cargo.toml
  src/
    lib.rs                    # pub mod declarations + top-level re-exports
    report.rs                 # ConformanceOutcome, ConformanceRow,
                              # ConformanceMatrixResult, write_conformance_reports
    fixtures/
      mod.rs
      image.rs                # MinimalStoredImageFixture
      upper_dir.rs            # WritableUpperDirFixture
      build_context.rs        # BuildContextFixture
      push_target.rs          # LocalPushTargetFixture
    mocks/
      mod.rs
      registry.rs             # MockRegistry
      filesystem.rs           # MockFilesystem
      limiter.rs              # MockLimiter
      runtime.rs              # MockRuntime
      network.rs              # MockNetwork
      commit.rs               # MockContainerCommitter
      build.rs                # MockImageBuilder
      push.rs                 # MockImagePusher
    backend/
      mod.rs
      descriptor.rs           # BackendDescriptor, BackendCapabilitySet (moved from conformance.rs)
    helpers/
      mod.rs
      daemon.rs               # make_mock_deps, make_mock_state (moved from daemonbox helpers)
      gc.rs                   # NoopImageGc
```

---

## Migration Sources → Destinations

| Source | Destination |
|--------|-------------|
| `minibox-core/src/adapters/conformance.rs` — report types | `minibox-testers/src/report.rs` |
| `minibox-core/src/adapters/conformance.rs` — `BackendDescriptor` | `minibox-testers/src/backend/descriptor.rs` |
| `minibox-core/src/adapters/conformance.rs` — fixtures | `minibox-testers/src/fixtures/` |
| `minibox-core/src/adapters/mocks.rs` | `minibox-testers/src/mocks/` |
| `minibox-core/src/adapters/test_fixtures.rs` | merged into `minibox-testers/src/fixtures/` |
| `daemonbox/tests/conformance_helpers.rs` | `minibox-testers/src/helpers/daemon.rs` |

### Backward-compatibility shims in `minibox-core`

The `test-utils` feature in `minibox-core` becomes a thin re-export module:

```rust
// minibox-core/src/adapters/conformance.rs (after migration)
#[cfg(feature = "test-utils")]
pub use minibox_testers::*;
```

This keeps existing `use minibox_core::adapters::conformance::*` call sites compiling without
changes during the migration. Remove the shim in Phase 2 once all call sites are updated.

---

## Conformance Test Role in Validating the Migration

The existing conformance test suite (`conformance_commit`, `conformance_build`, `conformance_push`,
`conformance_report`) is the migration's acceptance gate. The migration is complete when:

1. `cargo xtask test-conformance` exits 0
2. The emitted `report.md` matches the pre-migration baseline (same pass/skip counts, same rows)
3. `cargo nextest run --workspace` exits 0
4. `cargo clippy --workspace` exits 0 with no new warnings

No new test cases are added in Phase 1. The suite runs unmodified against the migrated types.

---

## Skip / Fail Semantics (unchanged)

The existing conventions are preserved verbatim:

- A test **skips** when `backend.capabilities.supports(cap)` returns `false` — return early, no
  assertion.
- A test **fails** when the backend declares the capability but the operation errors or produces
  incorrect output.
- `ConformanceOutcome::Fail` must be zero in any passing suite run.

---

## What `minibox-core` retains after Phase 1

- All domain trait definitions (`ImageRegistry`, `FilesystemProvider`, `ResourceLimiter`,
  `ContainerRuntime`, `ContainerCommitter`, `ImageBuilder`, `ImagePusher`, `NetworkProvider`)
- `BackendCapability` enum and `BackendCapabilitySet`
- Protocol types (`DaemonRequest`, `DaemonResponse`)
- `HostnameRegistryRouter`
- `ImageStore`, `ImageGarbageCollector`
- The `test-utils` feature (now a re-export shim pointing at `minibox-testers`)

---

## Acceptance Criteria

| # | Criterion |
|---|-----------|
| 1 | `minibox-testers` crate exists in workspace, compiles with `cargo check -p minibox-testers` |
| 2 | All types from migration sources exist at their new paths in `minibox-testers` |
| 3 | `minibox-core` `test-utils` re-exports compile cleanly |
| 4 | `cargo xtask test-conformance` passes with same pass/skip counts as pre-migration |
| 5 | `cargo nextest run --workspace` passes |
| 6 | `cargo clippy --workspace` — zero new warnings |
| 7 | No production crate depends on `minibox-testers` (verify with `cargo tree`) |
