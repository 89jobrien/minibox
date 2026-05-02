//! Conformance tests for the `ImageRegistry` trait contract.
//!
//! All tests use `MockRegistry` — no network calls are made.

use minibox::testing::mocks::registry::MockRegistry;
use minibox_core::domain::ImageRegistry;
use minibox_core::image::reference::ImageRef;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn alpine() -> ImageRef {
    ImageRef::parse("alpine:3.18").expect("parse alpine ref")
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

pub struct PullIncrementsCount;
impl ConformanceTest for PullIncrementsCount {
    fn name(&self) -> &str { "pull_increments_count" }
    fn adapter(&self) -> &str { "registry" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let registry = MockRegistry::new();
        rt().block_on(registry.pull_image(&alpine())).expect("pull");
        ctx.assert_eq(1, registry.pull_count(), "pull_count after one pull");
        ctx.result()
    }
}

pub struct MultiplePullsIncrementCount;
impl ConformanceTest for MultiplePullsIncrementCount {
    fn name(&self) -> &str { "multiple_pulls_increment_count" }
    fn adapter(&self) -> &str { "registry" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let registry = MockRegistry::new();
        let image = alpine();
        for _ in 0..4 {
            rt().block_on(registry.pull_image(&image)).expect("pull");
        }
        ctx.assert_eq(4, registry.pull_count(), "pull_count after 4 pulls");
        ctx.result()
    }
}

pub struct HasImageAfterPull;
impl ConformanceTest for HasImageAfterPull {
    fn name(&self) -> &str { "has_image_after_pull" }
    fn adapter(&self) -> &str { "registry" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let registry = MockRegistry::new();
        let r = alpine();
        rt().block_on(registry.pull_image(&r)).expect("pull");
        // has_image uses the cache_name() and tag stored by pull_image.
        ctx.assert_true(
            rt().block_on(registry.has_image(&r.cache_name(), &r.tag)),
            "has_image after pull",
        );
        ctx.result()
    }
}

pub struct FreshRegistryHasNoImages;
impl ConformanceTest for FreshRegistryHasNoImages {
    fn name(&self) -> &str { "fresh_registry_has_no_images" }
    fn adapter(&self) -> &str { "registry" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let registry = MockRegistry::new();
        ctx.assert_false(rt().block_on(registry.has_image("alpine", "3.18")), "no images before pull");
        ctx.assert_eq(0, registry.pull_count(), "pull_count starts at zero");
        ctx.result()
    }
}

pub struct PullFailureRegistryReturnsErr;
impl ConformanceTest for PullFailureRegistryReturnsErr {
    fn name(&self) -> &str { "pull_failure_registry_returns_err" }
    fn adapter(&self) -> &str { "registry" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let registry = MockRegistry::new().with_pull_failure();
        let result = rt().block_on(registry.pull_image(&alpine()));
        ctx.assert_err(result, "pull_failure registry must return Err");
        ctx.result()
    }
}

pub struct PullCountIncrementedOnFailure;
impl ConformanceTest for PullCountIncrementedOnFailure {
    fn name(&self) -> &str { "pull_count_incremented_on_failure" }
    fn adapter(&self) -> &str { "registry" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        // The mock increments pull_count before checking pull_should_succeed,
        // so failed attempts are counted — this is the documented contract.
        let registry = MockRegistry::new().with_pull_failure();
        let _ = rt().block_on(registry.pull_image(&alpine()));
        ctx.assert_eq(1, registry.pull_count(), "pull_count incremented even on failure");
        ctx.result()
    }
}

/// Return all registry conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(PullIncrementsCount),
        Box::new(MultiplePullsIncrementCount),
        Box::new(HasImageAfterPull),
        Box::new(FreshRegistryHasNoImages),
        Box::new(PullFailureRegistryReturnsErr),
        Box::new(PullCountIncrementedOnFailure),
    ]
}
