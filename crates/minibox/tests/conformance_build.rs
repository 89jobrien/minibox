//! Conformance tests for `ImageBuilder` backends.
//!
//! Tests exercise the build operation through the `DynImageBuilder` trait interface
//! using `BuildContextFixture` from Phase 1 and `MiniboxImageBuilder` as the backend
//! under test.
//!
//! # Skip semantics
//!
//! Each test checks `backend.capabilities.supports(BackendCapability::BuildFromContext)`
//! first.  Backends that omit the capability are skipped, not failed.
//!
//! # What this tests
//!
//! `MiniboxImageBuilder` is an MVP implementation: RUN steps are no-ops but ENV/CMD
//! metadata is captured and the final image is committed to the store.  The conformance
//! suite verifies the observable contract — result present, metadata preserved — without
//! asserting on RUN side-effects.

use anyhow::Result;
use minibox::adapters::MiniboxImageBuilder;
use minibox_core::domain::{BackendCapability, BuildConfig, BuildContext, DynImageBuilder};
use minibox_core::image::ImageStore;
use minibox::testing::backend::BackendDescriptor;
use minibox::testing::fixtures::BuildContextFixture;
use std::sync::Arc;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Backend descriptor factory
// ---------------------------------------------------------------------------

fn minibox_build_backend(
    image_store: Arc<ImageStore>,
    data_dir: std::path::PathBuf,
) -> (BackendDescriptor, DynImageBuilder) {
    let builder: DynImageBuilder =
        Arc::new(MiniboxImageBuilder::new(Arc::clone(&image_store), data_dir));
    let builder_for_descriptor = Arc::clone(&builder);
    let descriptor = BackendDescriptor::new("minibox-native-build")
        .with_builder(move || Arc::clone(&builder_for_descriptor));
    (descriptor, builder)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_image_store(tmp: &tempfile::TempDir) -> Arc<ImageStore> {
    Arc::new(ImageStore::new(tmp.path().join("images")).expect("ImageStore::new"))
}

fn build_config(tag: &str) -> BuildConfig {
    BuildConfig {
        tag: tag.to_string(),
        build_args: vec![],
        no_cache: false,
    }
}

// ---------------------------------------------------------------------------
// Conformance tests
// ---------------------------------------------------------------------------

/// A minimal `FROM scratch` Dockerfile build must succeed and return `ImageMetadata`.
#[tokio::test]
async fn build_minimal_dockerfile_succeeds() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let ctx_fixture = BuildContextFixture::new()?;

    let (backend, builder) = minibox_build_backend(Arc::clone(&store), tmp.path().join("data"));

    if !backend
        .capabilities
        .supports(BackendCapability::BuildFromContext)
    {
        return Ok(()); // skip
    }

    let (tx, mut rx) = mpsc::channel(64);

    let context = BuildContext {
        directory: ctx_fixture.context_dir.clone(),
        dockerfile: std::path::PathBuf::from("Dockerfile"),
    };
    let config = build_config("conformance/build-test:latest");

    let meta = builder.build_image(&context, &config, tx).await?;

    // Drain progress messages (non-empty channel means progress was emitted).
    let mut progress_count = 0;
    rx.close();
    while rx.try_recv().is_ok() {
        progress_count += 1;
    }

    assert!(
        progress_count > 0,
        "builder must emit at least one progress update"
    );
    assert!(
        !meta.name.is_empty(),
        "build result must have a non-empty image name"
    );

    Ok(())
}

/// After a successful build the resulting image must be stored and retrievable.
#[tokio::test]
async fn build_result_is_present_in_store() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let ctx_fixture = BuildContextFixture::new()?;

    let (backend, builder) = minibox_build_backend(Arc::clone(&store), tmp.path().join("data"));

    if !backend
        .capabilities
        .supports(BackendCapability::BuildFromContext)
    {
        return Ok(()); // skip
    }

    let (tx, _rx) = mpsc::channel(64);
    let context = BuildContext {
        directory: ctx_fixture.context_dir.clone(),
        dockerfile: std::path::PathBuf::from("Dockerfile"),
    };
    let config = build_config("conformance/stored-image:v1");

    builder.build_image(&context, &config, tx).await?;

    // The image must be findable in the store after build.
    assert!(
        store.has_image("conformance/stored-image", "v1"),
        "built image must be present in the store"
    );

    Ok(())
}

/// Build metadata must reflect the tag supplied in `BuildConfig`.
#[tokio::test]
async fn build_metadata_reflects_config_tag() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let ctx_fixture = BuildContextFixture::new()?;

    let (backend, builder) = minibox_build_backend(Arc::clone(&store), tmp.path().join("data"));

    if !backend
        .capabilities
        .supports(BackendCapability::BuildFromContext)
    {
        return Ok(()); // skip
    }

    let (tx, _rx) = mpsc::channel(64);
    let context = BuildContext {
        directory: ctx_fixture.context_dir.clone(),
        dockerfile: std::path::PathBuf::from("Dockerfile"),
    };
    let config = build_config("conformance/meta-test:v42");

    let meta = builder.build_image(&context, &config, tx).await?;

    assert_eq!(meta.tag, "v42", "returned tag must match BuildConfig.tag");
    assert!(
        !meta.layers.is_empty(),
        "built image must have at least one layer"
    );
    assert!(
        meta.layers[0].digest.starts_with("sha256:"),
        "layer digest must be sha256-prefixed"
    );

    Ok(())
}

/// An empty Dockerfile must not cause a panic.
///
/// The builder may return `Ok(meta)` with empty layers or `Err(_)` — both are
/// acceptable. The invariant under test is that no panic occurs.
#[tokio::test]
async fn build_empty_dockerfile_returns_error_or_empty() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);

    // Create a context dir with a zero-byte Dockerfile.
    let ctx_dir = tempfile::TempDir::new()?;
    std::fs::write(ctx_dir.path().join("Dockerfile"), b"")?;

    let (backend, builder) = minibox_build_backend(Arc::clone(&store), tmp.path().join("data"));

    if !backend
        .capabilities
        .supports(BackendCapability::BuildFromContext)
    {
        return Ok(()); // skip
    }

    let (tx, _rx) = mpsc::channel(64);
    let context = BuildContext {
        directory: ctx_dir.path().to_path_buf(),
        dockerfile: std::path::PathBuf::from("Dockerfile"),
    };
    let config = build_config("conformance/empty-dockerfile:v1");

    let result = builder.build_image(&context, &config, tx).await;

    // Either empty layers or an error — but no panic.
    match result {
        Ok(meta) => assert!(
            meta.layers.is_empty(),
            "empty Dockerfile must produce empty layers, got: {:?}",
            meta.layers
        ),
        Err(_) => {} // also acceptable
    }

    Ok(())
}

/// A tag without a registry prefix must be parsed and reflected in metadata.
///
/// `BuildConfig.tag = "myimage:v1"` (no registry host) must result in
/// `meta.name == "myimage"` and `meta.tag == "v1"`.
#[tokio::test]
async fn build_tag_without_registry_prefix_is_accepted() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let ctx_fixture = BuildContextFixture::new()?;

    let (backend, builder) = minibox_build_backend(Arc::clone(&store), tmp.path().join("data"));

    if !backend
        .capabilities
        .supports(BackendCapability::BuildFromContext)
    {
        return Ok(()); // skip
    }

    let (tx, _rx) = mpsc::channel(64);
    let context = BuildContext {
        directory: ctx_fixture.context_dir.clone(),
        dockerfile: std::path::PathBuf::from("Dockerfile"),
    };
    let config = build_config("myimage:v1");

    let meta = builder.build_image(&context, &config, tx).await?;

    assert_eq!(meta.name, "myimage", "name must be 'myimage'");
    assert_eq!(meta.tag, "v1", "tag must be 'v1'");

    Ok(())
}

/// A backend that does NOT declare `BuildFromContext` must have no `make_builder`.
#[tokio::test]
async fn build_skipped_for_backend_without_capability() {
    let descriptor = BackendDescriptor::new("no-build-backend");
    assert!(
        !descriptor
            .capabilities
            .supports(BackendCapability::BuildFromContext),
        "backend must not claim BuildFromContext capability"
    );
    assert!(
        descriptor.make_builder.is_none(),
        "make_builder must be None when capability is absent"
    );
}

/// krun adapter declares no BuildFromContext capability — conformance must skip gracefully.
#[tokio::test]
async fn build_krun_backend_skips_cleanly() {
    let descriptor = BackendDescriptor::new("krun");
    assert!(
        !descriptor
            .capabilities
            .supports(BackendCapability::BuildFromContext),
        "krun must not claim BuildFromContext capability"
    );
    assert!(
        descriptor.make_builder.is_none(),
        "krun must not wire a builder"
    );
}
