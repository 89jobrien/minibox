//! Conformance tests for the [`ImageBuilder`] trait contract.
//!
//! All tests use `MockImageBuilder` — no real build operations occur.

use minibox::testing::mocks::build::MockImageBuilder;
use minibox_core::domain::{BuildConfig, BuildContext, ImageBuilder};

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn empty_context() -> BuildContext {
    BuildContext {
        directory: std::path::PathBuf::from("/tmp/build-ctx"),
        dockerfile: std::path::PathBuf::from("Dockerfile"),
    }
}

fn build_config(tag: &str) -> BuildConfig {
    BuildConfig {
        tag: tag.to_string(),
        build_args: vec![],
        no_cache: false,
    }
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// `build_image` succeeds and returns `ImageMetadata`.
pub struct BuildImageReturnsMetadata;
impl ConformanceTest for BuildImageReturnsMetadata {
    fn name(&self) -> &'static str {
        "build_image_returns_metadata"
    }
    fn adapter(&self) -> &'static str {
        "image_builder"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImageBuilder::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let result = rt().block_on(mock.build_image(
            &empty_context(),
            &build_config("myapp:v1"),
            std::sync::Arc::new(tx),
        ));
        if let Some(meta) = ctx.assert_ok(result, "build_image should succeed") {
            ctx.assert_eq("myapp".to_string(), meta.name, "image name");
            ctx.assert_eq("v1".to_string(), meta.tag, "image tag");
        }
        ctx.result()
    }
}

/// `build_image` increments the call count.
pub struct BuildImageIncrementsCount;
impl ConformanceTest for BuildImageIncrementsCount {
    fn name(&self) -> &'static str {
        "build_image_increments_count"
    }
    fn adapter(&self) -> &'static str {
        "image_builder"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImageBuilder::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let _ = rt().block_on(mock.build_image(
            &empty_context(),
            &build_config("img:tag"),
            std::sync::Arc::new(tx),
        ));
        ctx.assert_eq(1, mock.call_count(), "call_count after one build");
        ctx.result()
    }
}

/// `build_image` sends at least one progress event.
pub struct BuildImageSendsProgress;
impl ConformanceTest for BuildImageSendsProgress {
    fn name(&self) -> &'static str {
        "build_image_sends_progress"
    }
    fn adapter(&self) -> &'static str {
        "image_builder"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImageBuilder::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let _ = rt().block_on(mock.build_image(
            &empty_context(),
            &build_config("img:tag"),
            std::sync::Arc::new(tx),
        ));
        let event = rt().block_on(rx.recv());
        ctx.assert_true(
            event.is_some(),
            "at least one build progress event expected",
        );
        ctx.result()
    }
}

/// `build_image` returns Err when configured to fail.
pub struct BuildImageFailureReturnsErr;
impl ConformanceTest for BuildImageFailureReturnsErr {
    fn name(&self) -> &'static str {
        "build_image_failure_returns_err"
    }
    fn adapter(&self) -> &'static str {
        "image_builder"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImageBuilder::new().with_failure();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let result = rt().block_on(mock.build_image(
            &empty_context(),
            &build_config("img:tag"),
            std::sync::Arc::new(tx),
        ));
        ctx.assert_err(
            result,
            "build_image with failure configured must return Err",
        );
        ctx.result()
    }
}

/// Return all `image_builder` conformance tests.
#[must_use]
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(BuildImageReturnsMetadata),
        Box::new(BuildImageIncrementsCount),
        Box::new(BuildImageSendsProgress),
        Box::new(BuildImageFailureReturnsErr),
    ]
}
