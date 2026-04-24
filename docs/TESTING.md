# Testing Strategy for Minibox

This document describes the testing strategy for the minibox container runtime.

## Test Pyramid

```
                 ┌─────────────┐
                 │  E2E Tests  │  (Daemon + CLI binaries)
                 │  ~14 tests  │
                 └─────────────┘
            ┌─────────────────────┐
            │ Integration Tests   │  (Real infrastructure)
            │  ~24 tests          │
            └─────────────────────┘
       ┌──────────────────────────────┐
       │  Unit + Conformance + Prop   │  (Mocks, any platform)
       │       ~221 tests             │
       └──────────────────────────────┘
```

## Quick Reference

```bash
# Install just (task runner) if not already installed
cargo install just

# Check host capabilities
just doctor

# Run all tests (full pipeline with cleanup)
just test-all

# Individual test layers
just test-unit          # Mock-based, any platform
just test-integration   # Linux, root, cgroups v2
just test-e2e           # Linux, root, full lifecycle (integration test)
just test-e2e-suite     # Linux, root, built binaries

# Cleanup
just clean              # Full cargo clean
just clean-test         # Test artifacts only
just clean-stale        # Old artifacts (>7 days)
just nuke-test-state    # Kill orphans, remove cgroups/mounts
```

## Test Layers

### 1. Unit + Conformance + Property Tests (~221 tests)

**Requirements:** None (run anywhere)

**Files:**

- `crates/mbx/src/**` + `tests/` — 155 tests (unit, adapter, property)
- `crates/daemonbox/src/**` + `tests/` — 55 tests (handler, conformance, proptest, recovery)
- `crates/minibox-cli/src/**` — 11 tests
- `crates/minibox-llm/src/**` — 13 tests (provider unit tests)

**Run:** `just test-unit`

### 2. Integration Tests (~24 tests)

**Requirements:** Linux kernel 5.0+, root, cgroups v2, Docker Hub access

**Files:**

- `crates/miniboxd/tests/cgroup_tests.rs` — ResourceLimiter trait against real cgroupfs (16 tests)
- `crates/miniboxd/tests/integration_tests.rs` — handler-level tests with real infrastructure (8 tests)

**Run:** `just test-integration`

**Lifecycle e2e (integration test):**

- `test_complete_container_lifecycle` (in `crates/miniboxd/tests/integration_tests.rs`)
- **Run:** `just test-e2e`

**Architecture:** Tests exercise domain traits (hexagonal ports) and verify outcomes
by reading real infrastructure state (cgroupfs, procfs, mount table).

### 3. E2E Tests (~14 tests)

**Requirements:** Linux kernel 5.0+, root, cgroups v2, built binaries

**Files:**

- `crates/miniboxd/tests/e2e_tests.rs` — starts real miniboxd, exercises minibox CLI
- `crates/miniboxd/tests/helpers/mod.rs` — shared RAII fixture + test utilities

**Run:** `just test-e2e-suite`

**Architecture:**

- `DaemonFixture` starts an isolated daemon instance with per-test temp dirs, a unique
  cgroup root (UUID-scoped), and a dedicated Unix socket. RAII `Drop` impl sends SIGTERM,
  waits up to 5s, escalates to SIGKILL, then cleans up the cgroup tree.
- All tests are annotated `#[serial]` (`serial_test` crate) to prevent concurrent
  cgroup/overlayfs resource conflicts on Linux.
- Prerequisites use `pull_required()` (panics with full diagnostics on failure) rather
  than silent `run_cli(&["pull", ...])` calls.
- Container readiness uses `wait_for_running()` / `poll_until()` (100ms poll, 5s timeout)
  instead of fixed `sleep()` waits.

## Preflight / Doctor

The preflight module (`crates/mbx/src/preflight.rs`) probes the host for
capabilities needed by integration and e2e tests. Run `just doctor` to see a report.

Tests use `require_capability!` to skip gracefully when prerequisites are missing.
