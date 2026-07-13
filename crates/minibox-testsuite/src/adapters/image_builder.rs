//! Conformance tests for the [`ImageBuilder`] trait contract.
//!
//! All tests use `MockImageBuilder` — no real build operations occur.

use minibox::testing::mocks::build::MockImageBuilder;
use minibox_core::domain::{BuildConfig, BuildContext, ImageBuilder};

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
// Tests
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "build_image_returns_metadata",
    adapter: "image_builder",
    capability: BuildFromContext,
    category: Unit,
    |ctx| {
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

crate::conformance_test! {
    name: "build_image_increments_count",
    adapter: "image_builder",
    capability: BuildFromContext,
    category: Unit,
    |ctx| {
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

crate::conformance_test! {
    name: "build_image_sends_progress",
    adapter: "image_builder",
    capability: BuildFromContext,
    category: Unit,
    |ctx| {
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

crate::conformance_test! {
    name: "build_image_failure_returns_err",
    adapter: "image_builder",
    capability: BuildFromContext,
    category: EdgeCase,
    |ctx| {
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
