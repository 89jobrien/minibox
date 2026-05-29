//! Conformance tests for the [`ImagePusher`] trait contract.
//!
//! All tests use `MockImagePusher` — no network I/O is performed.

use minibox::testing::mocks::push::MockImagePusher;
use minibox_core::domain::{ImagePusher, RegistryCredentials};
use minibox_core::image::reference::ImageRef;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn alpine() -> ImageRef {
    ImageRef::parse("alpine:3.18").expect("parse alpine ref")
}

fn anon() -> RegistryCredentials {
    RegistryCredentials::Anonymous
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// push_image succeeds and returns a PushResult with a non-empty digest.
pub struct PushImageReturnsDigest;
impl ConformanceTest for PushImageReturnsDigest {
    fn name(&self) -> &str {
        "push_image_returns_digest"
    }
    fn adapter(&self) -> &str {
        "image_pusher"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImagePusher::new();
        let result = rt().block_on(mock.push_image(&alpine(), &anon(), None));
        if let Some(r) = ctx.assert_ok(result, "push_image should succeed") {
            ctx.assert_true(!r.digest.is_empty(), "digest must be non-empty");
        }
        ctx.result()
    }
}

/// push_image records the tag in the mock.
pub struct PushImageRecordsTag;
impl ConformanceTest for PushImageRecordsTag {
    fn name(&self) -> &str {
        "push_image_records_tag"
    }
    fn adapter(&self) -> &str {
        "image_pusher"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImagePusher::new();
        rt().block_on(mock.push_image(&alpine(), &anon(), None))
            .expect("push");
        ctx.assert_true(
            mock.last_pushed_digest().is_some(),
            "last_pushed_digest should be set after push",
        );
        ctx.result()
    }
}

/// push_image returns Err when configured to fail.
pub struct PushImageFailureReturnsErr;
impl ConformanceTest for PushImageFailureReturnsErr {
    fn name(&self) -> &str {
        "push_image_failure_returns_err"
    }
    fn adapter(&self) -> &str {
        "image_pusher"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImagePusher::new().with_failure();
        let result = rt().block_on(mock.push_image(&alpine(), &anon(), None));
        ctx.assert_err(result, "push_image with failure configured must return Err");
        ctx.result()
    }
}

/// push_image sends progress when a channel is provided.
pub struct PushImageSendsProgress;
impl ConformanceTest for PushImageSendsProgress {
    fn name(&self) -> &str {
        "push_image_sends_progress"
    }
    fn adapter(&self) -> &str {
        "image_pusher"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockImagePusher::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let result =
            rt().block_on(mock.push_image(&alpine(), &anon(), Some(std::sync::Arc::new(tx))));
        ctx.assert_ok(result, "push_image with progress channel should succeed");
        let got = rt().block_on(rx.recv());
        ctx.assert_true(got.is_some(), "at least one progress event should be sent");
        ctx.result()
    }
}

/// Return all image_pusher conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(PushImageReturnsDigest),
        Box::new(PushImageRecordsTag),
        Box::new(PushImageFailureReturnsErr),
        Box::new(PushImageSendsProgress),
    ]
}
