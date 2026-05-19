//! Conformance tests for the [`RootfsSetup`] + [`ChildInit`] trait contract.
//!
//! All tests use `MockFilesystem` — no real mounts or syscalls are made.

use minibox::testing::mocks::filesystem::MockFilesystem;
use minibox_core::domain::RootfsSetup;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("create tempdir")
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// setup_rootfs succeeds with an empty layer list.
pub struct SetupRootfsSucceedsWithNoLayers;
impl ConformanceTest for SetupRootfsSucceedsWithNoLayers {
    fn name(&self) -> &str {
        "setup_rootfs_succeeds_with_no_layers"
    }
    fn adapter(&self) -> &str {
        "filesystem"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let fs = MockFilesystem::new();
        let dir = tmp();
        let result = fs.setup_rootfs(&[], dir.path());
        ctx.assert_ok(result, "setup_rootfs with empty layers should succeed");
        ctx.result()
    }
}

/// setup_rootfs increments the call count.
pub struct SetupRootfsIncrementsCount;
impl ConformanceTest for SetupRootfsIncrementsCount {
    fn name(&self) -> &str {
        "setup_rootfs_increments_count"
    }
    fn adapter(&self) -> &str {
        "filesystem"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let fs = MockFilesystem::new();
        let dir = tmp();
        let _ = fs.setup_rootfs(&[], dir.path());
        ctx.assert_eq(1, fs.setup_count(), "setup_count after one call");
        let _ = fs.setup_rootfs(&[], dir.path());
        ctx.assert_eq(2, fs.setup_count(), "setup_count after two calls");
        ctx.result()
    }
}

/// cleanup increments the cleanup call count.
pub struct CleanupIncrementsCount;
impl ConformanceTest for CleanupIncrementsCount {
    fn name(&self) -> &str {
        "cleanup_increments_count"
    }
    fn adapter(&self) -> &str {
        "filesystem"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let fs = MockFilesystem::new();
        let dir = tmp();
        let _ = fs.cleanup(dir.path());
        ctx.assert_eq(1, fs.cleanup_count(), "cleanup_count after one call");
        ctx.result()
    }
}

/// setup_rootfs returns Err when configured to fail.
pub struct SetupRootfsFailureReturnsErr;
impl ConformanceTest for SetupRootfsFailureReturnsErr {
    fn name(&self) -> &str {
        "setup_rootfs_failure_returns_err"
    }
    fn adapter(&self) -> &str {
        "filesystem"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let fs = MockFilesystem::new().with_setup_failure();
        let dir = tmp();
        let result = fs.setup_rootfs(&[], dir.path());
        ctx.assert_err(
            result,
            "setup_rootfs with failure configured must return Err",
        );
        ctx.result()
    }
}

/// Return all filesystem conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(SetupRootfsSucceedsWithNoLayers),
        Box::new(SetupRootfsIncrementsCount),
        Box::new(CleanupIncrementsCount),
        Box::new(SetupRootfsFailureReturnsErr),
    ]
}
