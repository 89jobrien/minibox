//! Conformance tests for the [`ImageLoader`] trait contract.
//!
//! All tests use `MockImageLoader` — no real filesystem operations occur.

use minibox::testing::mocks::image_loader::MockImageLoader;
use minibox_core::domain::ImageLoader;
use std::path::PathBuf;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn tarball() -> PathBuf {
    PathBuf::from("/tmp/test-image.tar")
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// load_image succeeds and increments the call count.
pub struct LoadImageSucceedsAndIncrementsCount;
impl ConformanceTest for LoadImageSucceedsAndIncrementsCount {
    fn name(&self) -> &str {
        "load_image_succeeds_and_increments_count"
    }
    fn adapter(&self) -> &str {
        "image_loader"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImageLoader::new();
        let result = rt().block_on(mock.load_image(&tarball(), "alpine", "3.18"));
        ctx.assert_ok(result, "load_image should succeed");
        ctx.assert_eq(1, mock.load_count(), "load_count after one call");
        ctx.result()
    }
}

/// load_image accumulates the count across multiple calls.
pub struct LoadImageCountAccumulates;
impl ConformanceTest for LoadImageCountAccumulates {
    fn name(&self) -> &str {
        "load_image_count_accumulates"
    }
    fn adapter(&self) -> &str {
        "image_loader"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImageLoader::new();
        for i in 0..3_u32 {
            rt().block_on(mock.load_image(&tarball(), &format!("img{i}"), "latest"))
                .expect("load");
        }
        ctx.assert_eq(3, mock.load_count(), "load_count after three calls");
        ctx.result()
    }
}

/// load_image returns Err when configured to fail.
pub struct LoadImageFailureReturnsErr;
impl ConformanceTest for LoadImageFailureReturnsErr {
    fn name(&self) -> &str {
        "load_image_failure_returns_err"
    }
    fn adapter(&self) -> &str {
        "image_loader"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImageLoader::failing();
        let result = rt().block_on(mock.load_image(&tarball(), "alpine", "3.18"));
        ctx.assert_err(result, "load_image with failure configured must return Err");
        ctx.result()
    }
}

/// load_image failure still increments call count.
pub struct LoadImageFailureIncrementsCount;
impl ConformanceTest for LoadImageFailureIncrementsCount {
    fn name(&self) -> &str {
        "load_image_failure_increments_count"
    }
    fn adapter(&self) -> &str {
        "image_loader"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImageLoader::failing();
        let _ = rt().block_on(mock.load_image(&tarball(), "alpine", "3.18"));
        ctx.assert_eq(
            1,
            mock.load_count(),
            "load_count incremented even on failure",
        );
        ctx.result()
    }
}

/// Return all image_loader conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(LoadImageSucceedsAndIncrementsCount),
        Box::new(LoadImageCountAccumulates),
        Box::new(LoadImageFailureReturnsErr),
        Box::new(LoadImageFailureIncrementsCount),
    ]
}
