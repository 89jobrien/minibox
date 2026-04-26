//! Conformance tests for the `ImageRegistry` trait contract.
//!
//! All tests use `MockRegistry` from `minibox::testing` — no network calls are made.
//! Each test creates a fresh mock to avoid shared state.

use minibox::testing::mocks::registry::MockRegistry;
use minibox_core::domain::ImageRegistry;
use minibox_core::image::reference::ImageRef;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn alpine_ref() -> ImageRef {
    ImageRef::parse("alpine:3.18").expect("parse alpine ref")
}

// ---------------------------------------------------------------------------
// Pull-count invariants
// ---------------------------------------------------------------------------

/// After a single successful pull, pull_count must be exactly 1.
#[tokio::test]
async fn registry_pull_increments_count() {
    let registry = MockRegistry::new();
    registry
        .pull_image(&alpine_ref())
        .await
        .expect("pull should succeed");
    assert_eq!(
        registry.pull_count(),
        1,
        "pull_count must be 1 after one pull"
    );
}

/// N successful pulls must result in pull_count == N.
#[tokio::test]
async fn registry_multiple_pulls_increment_count() {
    let registry = MockRegistry::new();
    let image = alpine_ref();
    for _ in 0..4 {
        registry
            .pull_image(&image)
            .await
            .expect("pull should succeed");
    }
    assert_eq!(
        registry.pull_count(),
        4,
        "pull_count must match number of pulls"
    );
}

// ---------------------------------------------------------------------------
// Cache invariants
// ---------------------------------------------------------------------------

/// After a successful pull, has_image must return true for that name/tag.
#[tokio::test]
async fn registry_pull_caches_image() {
    let registry = MockRegistry::new();
    let image = alpine_ref();
    registry
        .pull_image(&image)
        .await
        .expect("pull should succeed");
    let cached = registry.has_image(&image.cache_name(), &image.tag).await;
    assert!(cached, "has_image must return true after a successful pull");
}

/// A failed pull must not add the image to the cache.
#[tokio::test]
async fn registry_pull_failure_does_not_cache() {
    let registry = MockRegistry::new().with_pull_failure();
    let image = alpine_ref();
    let _ = registry.pull_image(&image).await; // expected to fail
    let cached = registry.has_image(&image.cache_name(), &image.tag).await;
    assert!(!cached, "has_image must return false after a failed pull");
}

/// `has_image` returns false for an image that was never seeded or pulled.
#[tokio::test]
async fn registry_has_image_false_for_uncached() {
    let registry = MockRegistry::new();
    let present = registry.has_image("library/alpine", "3.18").await;
    assert!(!present, "has_image must be false for an un-seeded image");
}

/// `with_cached_image` seeds an image; `has_image` must return true for it.
#[tokio::test]
async fn registry_has_image_true_for_seeded() {
    let registry = MockRegistry::new().with_cached_image("library/alpine", "3.18");
    let present = registry.has_image("library/alpine", "3.18").await;
    assert!(present, "has_image must return true for a seeded image");
}

// ---------------------------------------------------------------------------
// Failure-mode invariants
// ---------------------------------------------------------------------------

/// pull_image on a failure-configured mock must return Err.
#[tokio::test]
async fn registry_pull_failure_returns_err() {
    let registry = MockRegistry::new().with_pull_failure();
    let result = registry.pull_image(&alpine_ref()).await;
    assert!(
        result.is_err(),
        "pull_image must return Err on a failure-configured mock"
    );
}

// ---------------------------------------------------------------------------
// Layer invariants
// ---------------------------------------------------------------------------

/// `get_image_layers` must return Ok(Vec) — the mock always returns two paths.
#[test]
fn registry_get_layers_returns_vec() {
    let registry = MockRegistry::new();
    let layers = registry
        .get_image_layers("library/alpine", "3.18")
        .expect("get_image_layers must succeed on MockRegistry");
    assert!(
        !layers.is_empty(),
        "MockRegistry must return at least one mock layer path"
    );
}

/// `get_image_layers` returns a non-empty Vec for a different image too, confirming
/// the mock ignores the name/tag and always produces mock paths.
#[test]
fn registry_get_layers_returns_vec_for_nginx() {
    let registry = MockRegistry::new();
    let layers = registry
        .get_image_layers("library/nginx", "latest")
        .expect("get_image_layers must succeed on MockRegistry");
    assert!(
        !layers.is_empty(),
        "MockRegistry must return at least one mock layer path for nginx"
    );
    // Verify paths are absolute (mock returns /mock/layer*)
    for path in &layers {
        assert!(
            path.is_absolute(),
            "mock layer path must be absolute, got: {}",
            path.display()
        );
    }
}
