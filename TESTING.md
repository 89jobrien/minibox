# Testing Strategy for Minibox

This document describes the comprehensive testing strategy for the minibox container runtime.

## Test Pyramid

```
                 ┌─────────────┐
                 │   E2E Tests │  (Manual, daemon + CLI)
                 │    (TODO)   │
                 └─────────────┘
            ┌─────────────────────┐
            │ Integration Tests   │  (Linux only, real infrastructure)
            │    11 tests         │
            └─────────────────────┘
       ┌──────────────────────────────┐
       │     Unit Tests              │  (Platform-agnostic, mocks)
       │ 13 handler + 24 protocol    │
       └──────────────────────────────┘
```

## Test Categories

### 1. Unit Tests (37 tests)

**Location:** `crates/miniboxd/tests/handler_tests.rs`, `crates/minibox-lib/src/protocol.rs`

**Requirements:** None (run anywhere)

**Purpose:** Test business logic in isolation using mock implementations

**Run:**
```bash
cargo test -p miniboxd --test handler_tests
cargo test -p minibox-lib protocol::tests
```

### 2. Integration Tests (11 tests)

**Location:** `crates/miniboxd/tests/integration_tests.rs`

**Requirements:** Linux kernel 5.0+, root, Docker Hub access

**Run:**
```bash
sudo -E cargo test -p miniboxd --test integration_tests -- --test-threads=1 --ignored
```

### 3. End-to-End Tests (TODO)

Manual testing with daemon + CLI.

## Running Tests

### All Unit Tests
```bash
cargo test --workspace
```

### Integration Tests (Linux)
```bash
sudo -E cargo test -p miniboxd --test integration_tests -- --test-threads=1 --ignored
```

For complete testing documentation, see full TESTING.md in repository.
