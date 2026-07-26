//! Conformance tests for the `ImageRegistry` trait contract.
//!
//! All tests use `MockRegistry` -- no network calls are made.

use minibox::testing::mocks::registry::MockRegistry;
use minibox_core::domain::ImageRegistry;
use minibox_core::image::reference::ImageRef;

fn alpine() -> ImageRef {
    ImageRef::parse("alpine:3.18").expect("parse alpine ref")
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

crate::conformance_test! {
    name: "pull_increments_count",
    adapter: "registry",
    category: Unit,
    |ctx| {
        let registry = MockRegistry::new();
        rt().block_on(registry.pull_image(&alpine())).expect("pull");
        ctx.assert_eq(1, registry.pull_count(), "pull_count after one pull");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "multiple_pulls_increment_count",
    adapter: "registry",
    category: Unit,
    |ctx| {
        let registry = MockRegistry::new();
        let image = alpine();
        for _ in 0..4 {
            rt().block_on(registry.pull_image(&image)).expect("pull");
        }
        ctx.assert_eq(4, registry.pull_count(), "pull_count after 4 pulls");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "has_image_after_pull",
    adapter: "registry",
    category: Unit,
    |ctx| {
        let registry = MockRegistry::new();
        let r = alpine();
        rt().block_on(registry.pull_image(&r)).expect("pull");
        ctx.assert_true(
            rt().block_on(registry.has_image(&r.cache_name(), &r.tag)),
            "has_image after pull",
        );
        ctx.result()
    }
}

crate::conformance_test! {
    name: "fresh_registry_has_no_images",
    adapter: "registry",
    category: EdgeCase,
    |ctx| {
        let registry = MockRegistry::new();
        ctx.assert_false(
            rt().block_on(registry.has_image("alpine", "3.18")),
            "no images before pull",
        );
        ctx.assert_eq(0, registry.pull_count(), "pull_count starts at zero");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "pull_failure_registry_returns_err",
    adapter: "registry",
    category: EdgeCase,
    |ctx| {
        let registry = MockRegistry::new().with_pull_failure();
        let result = rt().block_on(registry.pull_image(&alpine()));
        ctx.assert_err(result, "pull_failure registry must return Err");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "pull_count_incremented_on_failure",
    adapter: "registry",
    category: EdgeCase,
    |ctx| {
        let registry = MockRegistry::new().with_pull_failure();
        let _ = rt().block_on(registry.pull_image(&alpine()));
        ctx.assert_eq(
            1,
            registry.pull_count(),
            "pull_count incremented even on failure",
        );
        ctx.result()
    }
}
