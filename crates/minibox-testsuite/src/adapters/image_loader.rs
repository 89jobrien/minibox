//! Conformance tests for the [`ImageLoader`] trait contract.
//!
//! All tests use `MockImageLoader` — no real filesystem operations occur.

use minibox::testing::mocks::image_loader::MockImageLoader;
use minibox_core::domain::ImageLoader;
use std::path::PathBuf;

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
// Tests
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "load_image_succeeds_and_increments_count",
    adapter: "image_loader",
    capability: ImageLoader,
    category: Unit,
    |ctx| {
        let mock = MockImageLoader::new();
        let result = rt().block_on(mock.load_image(&tarball(), "alpine", "3.18"));
        ctx.assert_ok(result, "load_image should succeed");
        ctx.assert_eq(1, mock.load_count(), "load_count after one call");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "load_image_count_accumulates",
    adapter: "image_loader",
    capability: ImageLoader,
    category: Unit,
    |ctx| {
        let mock = MockImageLoader::new();
        for i in 0..3_u32 {
            rt().block_on(mock.load_image(&tarball(), &format!("img{i}"), "latest"))
                .expect("load");
        }
        ctx.assert_eq(3, mock.load_count(), "load_count after three calls");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "load_image_failure_returns_err",
    adapter: "image_loader",
    capability: ImageLoader,
    category: EdgeCase,
    |ctx| {
        let mock = MockImageLoader::failing();
        let result = rt().block_on(mock.load_image(&tarball(), "alpine", "3.18"));
        ctx.assert_err(result, "load_image with failure configured must return Err");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "load_image_failure_increments_count",
    adapter: "image_loader",
    capability: ImageLoader,
    category: EdgeCase,
    |ctx| {
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
