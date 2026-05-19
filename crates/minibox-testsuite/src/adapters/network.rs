//! Conformance tests for the [`NetworkProvider`] trait contract.
//!
//! All tests use `MockNetwork` — no real network namespaces are created.

use minibox::testing::mocks::network::MockNetwork;
use minibox_core::domain::{NetworkConfig, NetworkProvider};

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn default_config() -> NetworkConfig {
    NetworkConfig::default()
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// setup returns a non-empty netns path.
pub struct SetupReturnsNetnsPath;
impl ConformanceTest for SetupReturnsNetnsPath {
    fn name(&self) -> &str {
        "setup_returns_netns_path"
    }
    fn adapter(&self) -> &str {
        "network"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockNetwork::new();
        let result = rt().block_on(mock.setup("ctr-net-001", &default_config()));
        if let Some(path) = ctx.assert_ok(result, "network setup should succeed") {
            ctx.assert_true(!path.is_empty(), "returned netns path must be non-empty");
        }
        ctx.result()
    }
}

/// setup increments the call count.
pub struct SetupIncrementsCount;
impl ConformanceTest for SetupIncrementsCount {
    fn name(&self) -> &str {
        "setup_increments_count"
    }
    fn adapter(&self) -> &str {
        "network"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockNetwork::new();
        let _ = rt().block_on(mock.setup("ctr-net-002", &default_config()));
        ctx.assert_eq(1, mock.setup_count(), "setup_count after one call");
        ctx.result()
    }
}

/// cleanup increments the cleanup count.
pub struct CleanupIncrementsCount;
impl ConformanceTest for CleanupIncrementsCount {
    fn name(&self) -> &str {
        "cleanup_increments_count"
    }
    fn adapter(&self) -> &str {
        "network"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockNetwork::new();
        let _ = rt().block_on(mock.cleanup("ctr-net-003"));
        ctx.assert_eq(1, mock.cleanup_count(), "cleanup_count after one call");
        ctx.result()
    }
}

/// setup returns Err when configured to fail.
pub struct SetupFailureReturnsErr;
impl ConformanceTest for SetupFailureReturnsErr {
    fn name(&self) -> &str {
        "setup_failure_returns_err"
    }
    fn adapter(&self) -> &str {
        "network"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockNetwork::new().with_setup_failure();
        let result = rt().block_on(mock.setup("ctr-net-004", &default_config()));
        ctx.assert_err(
            result,
            "network setup with failure configured must return Err",
        );
        ctx.result()
    }
}

/// cleanup returns Err when configured to fail.
pub struct CleanupFailureReturnsErr;
impl ConformanceTest for CleanupFailureReturnsErr {
    fn name(&self) -> &str {
        "cleanup_failure_returns_err"
    }
    fn adapter(&self) -> &str {
        "network"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockNetwork::new().with_cleanup_failure();
        let result = rt().block_on(mock.cleanup("ctr-net-005"));
        ctx.assert_err(
            result,
            "network cleanup with failure configured must return Err",
        );
        ctx.result()
    }
}

/// Return all network conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(SetupReturnsNetnsPath),
        Box::new(SetupIncrementsCount),
        Box::new(CleanupIncrementsCount),
        Box::new(SetupFailureReturnsErr),
        Box::new(CleanupFailureReturnsErr),
    ]
}
