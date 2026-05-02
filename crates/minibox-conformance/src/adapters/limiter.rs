//! Conformance tests for the `ResourceLimiter` trait contract.
//!
//! All tests use `MockLimiter` — no kernel/cgroup interaction.

use minibox::testing::mocks::limiter::MockLimiter;
use minibox_core::domain::{ResourceConfig, ResourceLimiter};

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn default_config() -> ResourceConfig {
    ResourceConfig {
        memory_limit_bytes: Some(128 * 1024 * 1024),
        cpu_weight: Some(512),
        pids_max: None,
        io_max_bytes_per_sec: None,
    }
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

pub struct CreateReturnsCgroupPath;
impl ConformanceTest for CreateReturnsCgroupPath {
    fn name(&self) -> &str { "create_returns_cgroup_path" }
    fn adapter(&self) -> &str { "limiter" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let limiter = MockLimiter::new();
        let path = ctx.assert_ok(limiter.create("testcontainer01", &default_config()), "create");
        if let Some(p) = path {
            ctx.assert_contains(&p, "testcontainer01", "path contains container_id");
        }
        ctx.result()
    }
}

pub struct CreateIncrementsCount;
impl ConformanceTest for CreateIncrementsCount {
    fn name(&self) -> &str { "create_increments_count" }
    fn adapter(&self) -> &str { "limiter" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let limiter = MockLimiter::new();
        limiter.create("counttest01", &default_config()).expect("create");
        ctx.assert_eq(1, limiter.create_count(), "create_count after one create");
        ctx.result()
    }
}

pub struct CreateFailureReturnsErr;
impl ConformanceTest for CreateFailureReturnsErr {
    fn name(&self) -> &str { "create_failure_returns_err" }
    fn adapter(&self) -> &str { "limiter" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let limiter = MockLimiter::new().with_create_failure();
        ctx.assert_err(limiter.create("failtest01", &default_config()), "failure mock returns Err");
        ctx.result()
    }
}

pub struct CreateFailureIncrementsCount;
impl ConformanceTest for CreateFailureIncrementsCount {
    fn name(&self) -> &str { "create_failure_increments_count" }
    fn adapter(&self) -> &str { "limiter" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let limiter = MockLimiter::new().with_create_failure();
        let _ = limiter.create("failtest02", &default_config());
        ctx.assert_eq(1, limiter.create_count(), "failed create still counted");
        ctx.result()
    }
}

pub struct AddProcessSucceedsByDefault;
impl ConformanceTest for AddProcessSucceedsByDefault {
    fn name(&self) -> &str { "add_process_succeeds_by_default" }
    fn adapter(&self) -> &str { "limiter" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let limiter = MockLimiter::new();
        ctx.assert_ok(limiter.add_process("aptest01", 12345), "add_process succeeds");
        ctx.result()
    }
}

pub struct CleanupIncrementsCount;
impl ConformanceTest for CleanupIncrementsCount {
    fn name(&self) -> &str { "cleanup_increments_count" }
    fn adapter(&self) -> &str { "limiter" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let limiter = MockLimiter::new();
        limiter.cleanup("cleanuptest01").expect("cleanup");
        ctx.assert_eq(1, limiter.cleanup_count(), "cleanup_count after one cleanup");
        ctx.result()
    }
}

pub struct CreateThenCleanupRoundTrip;
impl ConformanceTest for CreateThenCleanupRoundTrip {
    fn name(&self) -> &str { "create_then_cleanup_round_trip" }
    fn adapter(&self) -> &str { "limiter" }
    fn category(&self) -> TestCategory { TestCategory::Integration }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let limiter = MockLimiter::new();
        ctx.assert_ok(limiter.create("roundtrip01", &default_config()), "create");
        ctx.assert_ok(limiter.cleanup("roundtrip01"), "cleanup");
        ctx.assert_eq(1, limiter.create_count(), "create_count == 1");
        ctx.assert_eq(1, limiter.cleanup_count(), "cleanup_count == 1");
        ctx.result()
    }
}

/// Return all limiter conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(CreateReturnsCgroupPath),
        Box::new(CreateIncrementsCount),
        Box::new(CreateFailureReturnsErr),
        Box::new(CreateFailureIncrementsCount),
        Box::new(AddProcessSucceedsByDefault),
        Box::new(CleanupIncrementsCount),
        Box::new(CreateThenCleanupRoundTrip),
    ]
}
