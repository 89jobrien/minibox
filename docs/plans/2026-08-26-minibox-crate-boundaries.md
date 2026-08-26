---
source_sha: 045070e8926941810fbe1c48663b9ea3640cffd0
sources:
  - docs/designs/2026-08-26-minibox-crate-boundaries-design.md
  - crates/minibox-core/src/
  - crates/minibox/src/
  - crates/minibox-core/Cargo.toml
  - crates/minibox/Cargo.toml
  - xtask/src/
generated: 2026-08-26
---

# Plan: Minibox Crate Boundaries

## Goal

Extract a pure `minibox-domain` inner ring, preserve compatible `minibox-core` paths, remove
duplicate `minibox` implementations, and enforce the resulting dependency direction.

## Context Map

### Files to Modify

| File                                         | Purpose                       | Change                                                  |
| -------------------------------------------- | ----------------------------- | ------------------------------------------------------- |
| `Cargo.toml`                                 | Workspace graph               | Register `minibox-domain` and its workspace dependency  |
| `crates/minibox-domain/Cargo.toml`           | Domain package                | Declare only domain-safe dependencies                   |
| `crates/minibox-domain/src/lib.rs`           | Domain API                    | Export values, ports, events, and path types            |
| `crates/minibox-domain/src/*.rs`             | Domain implementation         | Receive extracted modules from `minibox-core`           |
| `crates/minibox-core/Cargo.toml`             | Shared infrastructure package | Depend on `minibox-domain`                              |
| `crates/minibox-core/src/lib.rs`             | Compatibility facade          | Re-export migrated domain APIs                          |
| `crates/minibox-core/src/domain/mod.rs`      | Legacy path facade            | Re-export `minibox-domain` instead of owning modules    |
| `crates/minibox-core/src/events.rs`          | Event adapter                 | Keep Tokio broker; consume domain event ports/types     |
| `crates/minibox-core/src/image/reference.rs` | Legacy image path             | Re-export domain `ImageRef` parsing API                 |
| `crates/minibox-core/src/adapters/mod.rs`    | Shared adapters               | Remain canonical owner of registry/router adapters      |
| `crates/minibox/src/lib.rs`                  | Runtime facade                | Re-export canonical core/domain APIs only               |
| `crates/minibox/src/adapters/mod.rs`         | Runtime adapters              | Re-export canonical `DockerHubRegistry`                 |
| `crates/minibox/src/error.rs`                | Runtime errors                | Retain runtime-specific errors; re-export shared errors |
| `crates/minibox/src/container/mod.rs`        | Native container state        | Privatize or rename non-domain state                    |
| `crates/minibox/src/daemon/handler/exec.rs`  | Protocol boundary             | Map domain `ExecOutput` to `DaemonResponse`             |
| `crates/minibox/src/adapters/exec.rs`        | Exec adapter                  | Emit domain `ExecOutput` through a sink adapter         |
| `xtask/src/main.rs`                          | Quality gate entrypoint       | Register architecture command                           |
| `xtask/src/architecture.rs`                  | Architecture enforcement      | Validate dependency rings and canonical paths           |

### Files to Delete

| File                                      | Reason                                        |
| ----------------------------------------- | --------------------------------------------- |
| `crates/minibox/src/preflight.rs`         | Undeclared shadow of canonical core preflight |
| `crates/minibox/src/domain/networking.rs` | Undeclared shadow of canonical domain module  |
| `crates/minibox/src/domain/extensions.rs` | Undeclared shadow of canonical domain module  |
| `crates/minibox/src/adapters/registry.rs` | Duplicate active Docker Hub adapter           |

### Dependencies

- `minibox-domain` has no workspace-crate dependency except `minibox-macros` if macro expansion
  cannot be removed during extraction.
- `minibox-core` depends on `minibox-domain`; compatibility paths prevent immediate consumer
  migration.
- `minibox` depends on both crates and remains the outer runtime ring.
- `macbox`, `smolbox`, `winbox`, `miniboxd`, `mbx`, `mcp`, `minibox-tui`, tests, benches, and fuzz
  targets compile through compatibility re-exports during this change.

### Test Coverage

- Existing inline tests move with domain modules.
- Existing adapter conformance suites remain in `minibox-core::adapters::conformance`.
- Existing protocol fuzz targets retain protocol imports from `minibox-core`.
- New architecture tests cover forbidden edges, shadow paths, and canonical owners.
- New exec integration tests cover `ExecOutput` to `DaemonResponse` mapping.

### Risk

- Public type identity must remain singular; compatibility modules must re-export, never wrap.
- Serde representations must remain unchanged when types move crates.
- Macro expansions currently expect `crate::domain`; the compatibility facade must remain until
  macro paths are made crate-independent.
- Tokio's foreign type cannot receive an implementation of a foreign domain trait from
  `minibox-core`; use a local `TokioProgressSink<T>` wrapper.
- The working tree contains unrelated documentation changes; implementation must not stage,
  rewrite, or revert them.

## Architecture

- Crates affected directly: `minibox-domain`, `minibox-core`, `minibox`, `xtask`.
- New public types: `ExecOutput`, `ProgressClosed`, `TokioProgressSink<T>`.
- Migrated public types: all current `minibox_core::domain::*`, `ContainerEvent`, `EventSink`,
  `InternalPath`, and `ImageRef`.
- Data flow: transport request (`minibox-core`) -> domain values/ports (`minibox-domain`) ->
  concrete adapter (`minibox`) -> domain output -> protocol response (`minibox`).

## Tech Stack

- Rust 2024 workspace.
- `serde`, `thiserror`, `async-trait`, `anyhow`, and `slashcrux` are allowed in the domain crate.
- Tokio, Reqwest, tracing subscribers, filesystem mutation, sockets, and process execution are
  forbidden in the domain crate.
- `cargo metadata` JSON powers architecture dependency checks without a new third-party crate.

## Tasks

### Task 1: Pin architecture failures

**Crate**: `xtask`
**Files**: `xtask/src/architecture.rs`, `xtask/src/main.rs`, `Cargo.toml`
**Run**: `cargo nextest run -p xtask architecture`

1. Add tests that currently fail for the missing `minibox-domain` ring, duplicate registry owner,
   and undeclared shadow paths.
2. Represent rings as exact crate-name sets and validate every internal Cargo edge.
3. Add canonical-path assertions for domain, preflight, registry, and shared errors.
4. Verify the tests fail for the expected current-tree violations rather than parser errors.

### Task 2: Create the domain crate and move independent values

**Crate**: `minibox-domain`
**Files**: `crates/minibox-domain/Cargo.toml`, `crates/minibox-domain/src/lib.rs`,
`crates/minibox-domain/src/capability.rs`, `crates/minibox-domain/src/checkpoint.rs`,
`crates/minibox-domain/src/error.rs`, `crates/minibox-domain/src/execution_manifest.rs`,
`crates/minibox-domain/src/execution_policy.rs`, `crates/minibox-domain/src/extensions.rs`,
`crates/minibox-domain/src/ids.rs`, `crates/minibox-domain/src/metrics.rs`,
`crates/minibox-domain/src/networking.rs`, `crates/minibox-domain/src/pty.rs`,
`crates/minibox-domain/src/state.rs`, `crates/minibox-domain/src/workflow.rs`
**Run**: `cargo nextest run -p minibox-domain`

1. Move each module with its inline unit/property tests and preserve visibility, derives, Serde
   attributes, and public names.
2. Remove infrastructure-only test imports; keep workflow protocol conversion tests in core.
3. Confirm the crate has no Tokio, Reqwest, tracing-subscriber, filesystem-I/O, socket, or process
   dependency.
4. Run clippy with warnings denied.

### Task 3: Move path and filesystem/runtime contracts

**Crate**: `minibox-domain`
**Files**: `crates/minibox-domain/src/path.rs`, `crates/minibox-domain/src/filesystem.rs`,
`crates/minibox-domain/src/runtime.rs`, `crates/minibox-core/src/path.rs`,
`crates/minibox-core/src/domain/mod.rs`
**Run**: `cargo nextest run -p minibox-domain -p minibox-core`

1. Move `InternalPath`, rootfs value types, and runtime port contracts into the domain crate.
2. Replace core source modules with direct public re-exports so old and new paths name the same
   Rust types.
3. Add compile-time identity tests accepting a new-path value where an old-path value is required.
4. Keep all filesystem mutation and runtime implementations outside the domain crate.

### Task 4: Split event ports from the Tokio broker

**Crate**: `minibox-domain`, `minibox-core`
**Files**: `crates/minibox-domain/src/events.rs`, `crates/minibox-core/src/events.rs`,
`crates/minibox-domain/src/runtime.rs`
**Run**: `cargo nextest run -p minibox-domain -p minibox-core events`

1. Move `ContainerEvent` and `EventSink` to `minibox-domain` with unchanged Serde representation.
2. Keep `EventSource`, `BroadcastEventBroker`, `NoopEventSink`, and Tokio receiver types in core.
3. Add serialization snapshot/round-trip tests in domain and broker behavior tests in core.
4. Remove Tokio from all domain signatures.

### Task 5: Move image identity and ports

**Crate**: `minibox-domain`, `minibox-core`
**Files**: `crates/minibox-domain/src/image.rs`, `crates/minibox-domain/src/image_reference.rs`,
`crates/minibox-core/src/image/reference.rs`, `crates/minibox-core/src/domain/mod.rs`
**Run**: `cargo nextest run -p minibox-domain -p minibox-core image`

1. Move `ImageRef`, its parser/formatter, image metadata, credentials, and image port traits.
2. Keep OCI manifests, HTTP registry clients, layer extraction, image store, leases, and GC in core.
3. Preserve `minibox_core::image::reference::ImageRef` through a direct re-export.
4. Move existing property tests and fuzz-target imports without changing their invariants/corpus.

### Task 6: Decouple exec ports from protocol responses

**Crate**: `minibox-domain`, `minibox-core`, `minibox`
**Files**: `crates/minibox-domain/src/exec.rs`, `crates/minibox-core/src/progress.rs`,
`crates/minibox/src/adapters/exec.rs`, `crates/minibox/src/daemon/handler/exec.rs`
**Run**: `cargo nextest run -p minibox-domain -p minibox exec`

1. Add `ExecOutput`, `ExecOutputStream`, and `ExecSession` domain contracts.
2. Add bounded `TokioExecOutputStream` and generic `TokioProgressSink<T>` adapters in core.
3. Change every `ExecRuntime` implementation and conformance double to return the handle and
   bounded output stream together.
4. Map stdout, stderr, exit, and post-acceptance error events to `DaemonResponse` only in the
   daemon handler.
5. Add an integration test proving stream order and terminal-response behavior are unchanged.

### Task 7: Install the core compatibility facade

**Crate**: `minibox-core`
**Files**: `crates/minibox-core/Cargo.toml`, `crates/minibox-core/src/lib.rs`,
`crates/minibox-core/src/domain/mod.rs`
**Run**: `cargo nextest run -p minibox-core`

1. Depend on `minibox-domain` and replace migrated implementations with direct re-exports.
2. Preserve existing paths for `domain`, events, path, and image-reference consumers.
3. Add compile-only compatibility tests for representative old paths and direct new paths.
4. Verify all downstream crates still resolve one type identity.

### Task 8: Canonicalize minibox ownership

**Crate**: `minibox`
**Files**: `crates/minibox/src/lib.rs`, `crates/minibox/src/adapters/mod.rs`,
`crates/minibox/src/error.rs`, `crates/minibox/src/container/mod.rs`,
`crates/minibox/src/testing/fixtures/container.rs`,
`crates/minibox/src/testing/fixtures/push_target.rs`
**Delete**: `crates/minibox/src/preflight.rs`, `crates/minibox/src/domain/networking.rs`,
`crates/minibox/src/domain/extensions.rs`, `crates/minibox/src/adapters/registry.rs`
**Run**: `cargo nextest run -p minibox --all-features`

1. Prove deleted files are undeclared or replaced by canonical re-exports.
2. Re-export core `DockerHubRegistry` and run its existing conformance/integration tests.
3. Re-export shared errors and fixtures; retain only runtime-specific implementations locally.
4. Privatize or rename native-only state so `ContainerState` has one public canonical owner.
5. Run Dupehound with tests excluded and verify cross-crate registry/preflight/fixture clusters are
   removed without suppressions.

### Task 9: Enable the architecture gate

**Crate**: `xtask`
**Files**: `xtask/src/architecture.rs`, `xtask/src/main.rs`, `xtask/src/precommit.rs`,
`.github/workflows/merge.yml`
**Run**: `cargo xtask architecture`

1. Make the Task 1 tests pass against the migrated workspace graph.
2. Fail on reverse ring dependencies, forbidden infrastructure dependencies in
   `minibox-domain`, and canonical shadow paths.
3. Add the command to local pre-commit and merge CI without disabling existing gates.
4. Verify a temporary fixture graph containing `minibox-domain -> minibox-core` is rejected.

### Task 10: Full verification and documentation sync

**Crate**: workspace
**Files**: `docs/core/ARCHITECTURE.mbx.md`, `docs/core/CRATE_TIERS.mbx.md`,
`.ctx/memory-bank/systemPatterns.mbx.md`, `.ctx/rustqual-baseline.json`
**Run**: `cargo xtask pre-commit`

1. Update architecture docs from the implemented graph and correct the stale domain source path.
2. Run `cargo check`, targeted clippy, workspace nextest through the repository's supported xtask
   gates, architecture check, protocol drift, rustqual compare, and Dupehound production scan.
3. Run adapter conformance suites and protocol/image-reference fuzz smoke tests.
4. Refresh the rustqual baseline only after the score does not regress.
5. Run the docs audit and report unrelated pre-existing collisions/staleness separately.

## Exit Criteria

- `minibox-domain` has no infrastructure dependencies or transport types.
- Cargo dependencies point inward and the architecture command enforces them.
- `minibox-core::domain::*` compatibility paths resolve to `minibox-domain` type identities.
- `minibox` has no orphan domain/preflight source and no duplicate Docker Hub adapter.
- Existing adapter conformance, protocol compatibility, and daemon streaming tests pass.
- Rustqual does not regress and Dupehound no longer reports the targeted cross-crate duplicates.
- No unrelated dirty-tree changes are modified, staged, or reverted.
