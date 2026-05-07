# Testing Guide

Full test strategy for minibox. See `DEVELOPMENT.md` for the developer workflow and
`docs/TEST_INFRASTRUCTURE.md` for the generated codebase analysis this document summarises.

## Test Categories

| Category              | Count (est.) | Platform    | Root? | CI?         |
| --------------------- | ------------ | ----------- | ----- | ----------- |
| Unit (inline lib)     | ~728         | any         | no    | yes         |
| Handler + conformance | ~209         | any         | no    | partial     |
| minibox-core conform. | 126          | any         | no    | yes         |
| Adapter isolation     | ~66          | varies      | varies| partial     |
| Property (proptest)   | ~46          | any         | no    | no          |
| Security regression   | ~19          | any         | no    | yes         |
| CLI subprocess        | 30           | any         | no    | no          |
| krun conformance      | ~29          | macOS/Linux | no    | no          |
| Sandbox               | ~17          | Linux       | yes   | no          |
| E2E daemon+CLI        | 15           | Linux       | yes   | next/stable |
| Cgroup integration    | 16           | Linux       | yes   | next/stable |
| VZ smoke              | ~1           | macOS       | no    | no          |
| Protocol evolution    | 11           | any         | no    | yes         |

`cargo nextest` on macOS reports ~506 tests — that is the cross-platform subset. Linux-only,
feature-gated, and root-required tests add ~700 more. Total estimate: ~1,467.

## Quick Reference

```bash
# Cross-platform unit tests (any machine, no root)
cargo xtask test-unit

# Property tests
cargo xtask test-property

# Integration tests — Linux + root
just test-integration

# E2E single lifecycle test — Linux + root
just test-e2e

# Full E2E suite — Linux + root
just test-e2e-suite

# Run E2E suite on VPS over SSH
just test-e2e-vps

# Adapter swap tests (Colima + handler)
just test-adapters

# CLI subprocess integration tests
just test-cli-subprocess

# macOS VZ isolation (requires VM image)
just test-vz-isolation

# Coverage HTML report
just coverage

# Full pipeline: nuke -> doctor -> unit + integration + e2e -> nuke
just test-all
```

## Running Tests by Category

### Unit tests

Run on any platform, no root required. Use these during local development.

```bash
cargo xtask test-unit          # canonical: lib + select integration + conformance
just test-unit                 # equivalent shorthand
```

### Property tests

Fuzz-style tests using `proptest`. Not in CI; run manually before pushing to `next`.

```bash
cargo xtask test-property
```

### Integration tests (Linux + root)

Cgroup resource limit tests and native adapter isolation. Require a Linux host with root.

```bash
just test-integration
```

### E2E tests (Linux + root)

Full daemon + CLI round-trip tests. Use the VPS target for Linux when developing on macOS.

```bash
just test-e2e               # single lifecycle scenario
just test-e2e-suite         # full suite (15 scenarios)
just test-e2e-vps           # run suite on VPS via SSH
```

### Adapter tests

Colima and handler adapter swap tests. Cross-platform; no root required.

```bash
just test-adapters
just test-cli-subprocess    # 30 CLI subprocess tests
just test-vz-isolation      # macOS VZ (requires VM image from cargo xtask build-vm-image)
```

### Conformance suite

Backend-agnostic capability tests tracked against a Markdown/JSON report.

```bash
cargo xtask test-conformance          # run + report
cargo xtask test-krun-conformance     # krun-specific variant
```

### Security regression

Tar extraction, path traversal, overlay escape, and socket auth tests. Cross-platform.
These run inside `test-unit` via the security regression test files; no separate command needed.

### Coverage

```bash
just coverage                # HTML report at target/llvm-cov/html/
cargo xtask coverage-check   # gate: handler.rs function coverage >= 80%
```

## Test Helpers

All helpers live behind the `test-utils` feature flag.

**`minibox::testing`** — enabled with `--features test-utils`:

- `mocks/` — `MockRegistry`, `MockFilesystem`, `MockLimiter`, `MockRuntime`, `MockNetwork`,
  `MockExecRuntime`, `MockImagePusher`, `MockContainerCommitter`, `MockImageBuilder`
- `fixtures/` — `ContainerFixture`, `ImageFixture`, `BuildContextFixture`,
  `PushTargetFixture`, `UpperDirFixture`
- `helpers/` — `create_test_deps_with_dir`, GC helpers, daemon helpers
- `backend/` — `BackendCapability`, `BackendDescriptor` (conformance)

**`minibox-core::adapters`** — enabled with `--features test-utils`:

- `mocks.rs` — cross-platform mock adapters
- `test_fixtures.rs` — shared fixtures
- `conformance.rs` — conformance harness

## CI Coverage

Eight workflows run in GitHub Actions. Tests relevant to each:

| Workflow              | Tests run                                             | Branches           |
| --------------------- | ----------------------------------------------------- | ------------------ |
| `ci.yml`              | lint, unit, archive integration, audit/deny           | all                |
| `stability-gates.yml` | doc-sync, adapter integration, no-unwrap, compile     | all                |
| `conformance.yml`     | `cargo xtask test-conformance`                        | next/stable        |
| `bench-regression.yml`| criterion + 10% regression gate                       | next/stable        |
| `protocol-drift.yml`  | variant count + handler coverage (protocol.rs changes)| all                |
| `nightly.yml`         | `cargo geiger` unsafe audit (informational)           | daily cron         |
| `release.yml`         | musl cross-compile + publish                          | `v*` tags          |

### CI gaps (not covered by any workflow)

The following require manual runs before merging to `next`:

- Property tests: `cargo xtask test-property`
- Sandbox tests: `cargo xtask test-sandbox`
- CLI subprocess: `just test-cli-subprocess`
- krun conformance: `cargo xtask test-krun-conformance`
- VZ smoke: `just test-vz-isolation`
- Coverage gate: `cargo xtask coverage-check`

## Writing Tests

### Platform gating

Gate Linux-only tests explicitly. macOS `cargo check` does not validate
`#[cfg(target_os = "linux")]` paths.

```rust
#[cfg(target_os = "linux")]
#[test]
fn test_cgroup_limits() { ... }
```

### Environment mutation

`std::env::set_var` and `remove_var` are `unsafe` in Rust 2024. Serialize any test that mutates
the environment with a shared mutex.

```rust
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_env_var() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var("KEY", "val") };
    // ...
}
```

### CLI subprocess tests

Do not use `Command::cargo_bin()`. Use the `find_minibox()` helper or set `MINIBOX_TEST_BIN_DIR`.

### Protocol changes

Changes to `crates/minibox-core/src/protocol.rs` must be accompanied by updates to handlers,
CLI paths, and snapshot tests in the same commit. New request fields use `#[serde(default)]`
for wire compatibility.

### Snapshot tests

After `cargo nextest` runs, check for `.snap.new` files. Review and accept with:

```bash
cargo insta review
```

## Cleaning Test State

```bash
cargo xtask nuke-test-state   # kill orphans, unmount overlays, clean cgroups, remove temp state
just clean-test               # remove test binaries
```
