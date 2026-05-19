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

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_config() -> PtyConfig {
    PtyConfig {
        enabled: true,
        cols: 80,
        rows: 24,
    }
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// allocate succeeds and returns a PtyHandle with non-negative fds.
pub struct AllocateReturnsHandle;
impl ConformanceTest for AllocateReturnsHandle {
    fn name(&self) -> &str {
        "allocate_returns_handle"
    }
    fn adapter(&self) -> &str {
        "pty"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockPtyAllocator::new();
        let result = mock.allocate(&default_config());
        if let Some(handle) = ctx.assert_ok(result, "allocate should succeed") {
            ctx.assert_true(handle.master_fd >= 0, "master_fd must be non-negative");
            ctx.assert_true(handle.slave_fd >= 0, "slave_fd must be non-negative");
        }
        ctx.result()
    }
}

/// allocate increments the call count.
pub struct AllocateIncrementsCount;
impl ConformanceTest for AllocateIncrementsCount {
    fn name(&self) -> &str {
        "allocate_increments_count"
    }
    fn adapter(&self) -> &str {
        "pty"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockPtyAllocator::new();
        let _ = mock.allocate(&default_config());
        ctx.assert_eq(1, mock.allocate_count(), "allocate_count after one call");
        let _ = mock.allocate(&default_config());
        ctx.assert_eq(2, mock.allocate_count(), "allocate_count after two calls");
        ctx.result()
    }
}

/// allocate returns Err when configured to fail.
pub struct AllocateFailureReturnsErr;
impl ConformanceTest for AllocateFailureReturnsErr {
    fn name(&self) -> &str {
        "allocate_failure_returns_err"
    }
    fn adapter(&self) -> &str {
        "pty"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockPtyAllocator::failing();
        let result = mock.allocate(&default_config());
        ctx.assert_err(result, "allocate with failure configured must return Err");
        ctx.result()
    }
}

/// allocate with disabled PTY config still calls through.
pub struct AllocateWithDisabledConfig;
impl ConformanceTest for AllocateWithDisabledConfig {
    fn name(&self) -> &str {
        "allocate_with_disabled_config"
    }
    fn adapter(&self) -> &str {
        "pty"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
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

/// Return all pty conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(AllocateReturnsHandle),
        Box::new(AllocateIncrementsCount),
        Box::new(AllocateFailureReturnsErr),
        Box::new(AllocateWithDisabledConfig),
    ]
}
