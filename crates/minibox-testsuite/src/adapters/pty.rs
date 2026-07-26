//! Conformance tests for the [`PtyAllocator`] trait contract.
//!
//! All tests use `MockPtyAllocator` — no real OS PTY pairs are allocated.
//!
//! Note: the plan originally included a separate `tty.rs` module for a
//! `TtyProvider` trait. That trait does not exist in domain.rs; the real
//! PTY abstraction is `PtyAllocator`, so both Task 14 and Task 15 from the
//! plan are covered here.

use minibox::testing::mocks::pty::MockPtyAllocator;
use minibox_core::domain::{PtyAllocator, PtyConfig};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const fn default_config() -> PtyConfig {
    PtyConfig {
        enabled: true,
        cols: 80,
        rows: 24,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// allocate succeeds and returns a `PtyHandle` with non-negative fds.
crate::conformance_test! {
    name: "allocate_returns_handle",
    adapter: "pty",
    capability: Pty,
    category: Unit,
    |ctx| {
        let mock = MockPtyAllocator::new();
        let result = mock.allocate(&default_config());
        if let Some(handle) = ctx.assert_ok(result, "allocate should succeed") {
            ctx.assert_true(handle.master_fd >= 0, "master_fd must be non-negative");
            ctx.assert_true(handle.slave_fd >= 0, "slave_fd must be non-negative");
        }
        ctx.result()
    }
}

// allocate increments the call count.
crate::conformance_test! {
    name: "allocate_increments_count",
    adapter: "pty",
    capability: Pty,
    category: Unit,
    |ctx| {
        let mock = MockPtyAllocator::new();
        let _ = mock.allocate(&default_config());
        ctx.assert_eq(1, mock.allocate_count(), "allocate_count after one call");
        let _ = mock.allocate(&default_config());
        ctx.assert_eq(2, mock.allocate_count(), "allocate_count after two calls");
        ctx.result()
    }
}

// allocate returns Err when configured to fail.
crate::conformance_test! {
    name: "allocate_failure_returns_err",
    adapter: "pty",
    capability: Pty,
    category: EdgeCase,
    |ctx| {
        let mock = MockPtyAllocator::failing();
        let result = mock.allocate(&default_config());
        ctx.assert_err(result, "allocate with failure configured must return Err");
        ctx.result()
    }
}

// allocate with disabled PTY config still calls through.
crate::conformance_test! {
    name: "allocate_with_disabled_config",
    adapter: "pty",
    capability: Pty,
    category: EdgeCase,
    |ctx| {
        let mock = MockPtyAllocator::new();
        let cfg = PtyConfig {
            enabled: false,
            cols: 0,
            rows: 0,
        };
        // The mock ignores enabled — callers control whether they call allocate.
        // This test verifies the mock does not panic on disabled config.
        let result = mock.allocate(&cfg);
        ctx.assert_ok(result, "allocate with disabled config should not panic");
        ctx.result()
    }
}
