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
            │  ~28 tests          │
            └─────────────────────┘
       ┌──────────────────────────────┐
       │     Unit + Conformance      │  (Mocks, any platform)
       │        ~52 tests            │
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
just test-e2e           # Linux, root, built binaries

# Cleanup
just clean              # Full cargo clean
just clean-test         # Test artifacts only
just clean-stale        # Old artifacts (>7 days)
just nuke-test-state    # Kill orphans, remove cgroups/mounts
```

## Test Layers

### 1. Unit + Conformance Tests (~52 tests)

**Requirements:** None (run anywhere)

**Files:**
- `crates/miniboxd/tests/handler_tests.rs` — handler logic with mock adapters
- `crates/miniboxd/tests/conformance_tests.rs` — trait contract verification with mocks
- `crates/minibox-lib/src/protocol.rs` — protocol serialization
- `crates/minibox-lib/src/preflight.rs` — kernel version parsing

**Run:** `just test-unit`

### 2. Integration Tests (~28 tests)

**Requirements:** Linux kernel 5.0+, root, cgroups v2, Docker Hub access

**Files:**
- `crates/miniboxd/tests/cgroup_tests.rs` — ResourceLimiter trait against real cgroupfs
- `crates/miniboxd/tests/integration_tests.rs` — handler-level tests with real infrastructure

**Run:** `just test-integration`

**Architecture:** Tests exercise domain traits (hexagonal ports) and verify outcomes
by reading real infrastructure state (cgroupfs, procfs, mount table).

### 3. E2E Tests (~14 tests)

**Requirements:** Linux kernel 5.0+, root, cgroups v2, built binaries

**Files:**
- `crates/miniboxd/tests/e2e_tests.rs` — starts real miniboxd, exercises minibox CLI

**Run:** `just test-e2e`

**Architecture:** `DaemonFixture` starts an isolated daemon instance with temp dirs,
then runs CLI commands as subprocesses. RAII cleanup on drop.

## Preflight / Doctor

The preflight module (`crates/minibox-lib/src/preflight.rs`) probes the host for
capabilities needed by integration and e2e tests. Run `just doctor` to see a report.

Tests use `require_capability!` to skip gracefully when prerequisites are missing.
