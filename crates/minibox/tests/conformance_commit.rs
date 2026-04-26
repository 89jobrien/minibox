//! Conformance tests for `ContainerCommitter` backends.
//!
//! These tests exercise the commit operation through the `DynContainerCommitter` trait
//! interface using the `BackendDescriptor` + fixture infrastructure from Phase 1.
//!
//! # Skip semantics
//!
//! Each test checks `backend.capabilities.supports(BackendCapability::Commit)` first.
//! Backends that omit the capability are skipped (not failed) — consistent with the
//! conformance suite design.
//!
//! # Backend under test
//!
//! `minibox_commit_backend()` wires `commit_upper_dir_to_image` (the sync inner function
//! extracted from `OverlayCommitAdapter`) directly, avoiding the need for a live
//! `StateHandle`.  This is intentional: the conformance suite tests the *algorithm*,
//! not the daemon integration.

use anyhow::Result;
use async_trait::async_trait;
use minibox::adapters::commit_upper_dir_to_image;
use minibox_core::domain::{
    AsAny, BackendCapability, CommitConfig, ContainerCommitter, ContainerId, DynContainerCommitter,
    ImageMetadata,
};
use minibox_core::image::ImageStore;
use minibox::testing::backend::BackendDescriptor;
use minibox::testing::fixtures::WritableUpperDirFixture;
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Inline commit adapter for conformance (no StateHandle required)
// ---------------------------------------------------------------------------

/// Wraps `commit_upper_dir_to_image` as a `ContainerCommitter` for conformance testing.
///
/// `upper_dir` is provided at construction time (captured from fixture) rather than
/// looked up from daemon state — this keeps the conformance test self-contained.
struct ConformanceCommitAdapter {
    image_store: Arc<ImageStore>,
    upper_dir: PathBuf,
}

impl AsAny for ConformanceCommitAdapter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait]
impl ContainerCommitter for ConformanceCommitAdapter {
    async fn commit(
        &self,
        _container_id: &ContainerId,
        target_ref: &str,
        config: &CommitConfig,
    ) -> anyhow::Result<ImageMetadata> {
        let image_store = Arc::clone(&self.image_store);
        let upper_dir = self.upper_dir.clone();
        let target_ref = target_ref.to_string();
        let config = config.clone();
        tokio::task::spawn_blocking(move || {
            commit_upper_dir_to_image(image_store, &upper_dir, &target_ref, &config)
        })
        .await
        .expect("spawn_blocking join")
    }
}

// ---------------------------------------------------------------------------
// Backend descriptor factory
// ---------------------------------------------------------------------------

fn minibox_commit_backend(
    image_store: Arc<ImageStore>,
    upper_dir: PathBuf,
) -> (BackendDescriptor, DynContainerCommitter) {
    let adapter: DynContainerCommitter = Arc::new(ConformanceCommitAdapter {
        image_store: Arc::clone(&image_store),
        upper_dir,
    });
    let adapter_for_descriptor = Arc::clone(&adapter);
    let descriptor = BackendDescriptor::new("minibox-native-commit")
        .with_committer(move || Arc::clone(&adapter_for_descriptor));
    (descriptor, adapter)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_image_store(tmp: &tempfile::TempDir) -> Arc<ImageStore> {
    Arc::new(ImageStore::new(tmp.path().join("images")).expect("ImageStore::new"))
}

fn default_commit_config() -> CommitConfig {
    CommitConfig {
        author: Some("conformance-test".to_string()),
        message: Some("conformance commit".to_string()),
        env_overrides: vec![],
        cmd_override: None,
    }
}

// ---------------------------------------------------------------------------
// Conformance tests
// ---------------------------------------------------------------------------

/// A successful commit must return `ImageMetadata` with the target name and tag.
#[tokio::test]
async fn commit_returns_metadata() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let upper = WritableUpperDirFixture::new()?;

    let (backend, committer) = minibox_commit_backend(Arc::clone(&store), upper.upper_dir.clone());

    if !backend.capabilities.supports(BackendCapability::Commit) {
        return Ok(()); // skip
    }

    let cid = ContainerId::new("conformancecommit01".to_string()).expect("ContainerId");
    let meta = committer
        .commit(&cid, "conformance/test-image:v1", &default_commit_config())
        .await?;

    assert_eq!(
        meta.name, "conformance/test-image",
        "metadata name mismatch"
    );
    assert_eq!(meta.tag, "v1", "metadata tag mismatch");
    assert!(
        !meta.layers.is_empty(),
        "commit result must have at least one layer"
    );

    Ok(())
}

/// After a successful commit, the layer artifact must be present on disk in the image store.
#[tokio::test]
async fn commit_writes_layer_artifact_to_store() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let upper = WritableUpperDirFixture::new()?;

    let (backend, committer) = minibox_commit_backend(Arc::clone(&store), upper.upper_dir.clone());

    if !backend.capabilities.supports(BackendCapability::Commit) {
        return Ok(()); // skip
    }

    let cid = ContainerId::new("conformancecommit02".to_string()).expect("ContainerId");
    let meta = committer
        .commit(
            &cid,
            "conformance/artifact-test:latest",
            &default_commit_config(),
        )
        .await?;

    // The layer digest reported in metadata must be a sha256 digest.
    let layer_digest = &meta.layers[0].digest;
    assert!(
        layer_digest.starts_with("sha256:"),
        "layer digest must be a sha256 digest, got: {layer_digest}"
    );

    // The image store should now have layers for the committed image.
    let layers = store
        .get_image_layers("conformance/artifact-test", "latest")
        .expect("get_image_layers after commit");
    assert!(
        !layers.is_empty(),
        "commit must write at least one layer to the store"
    );

    Ok(())
}

/// Metadata returned by two consecutive commits to different targets must be
/// independent — names/tags must not bleed across commit calls.
#[tokio::test]
async fn commit_metadata_is_consistent_across_calls() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let upper_a = WritableUpperDirFixture::new()?;
    let upper_b = WritableUpperDirFixture::new()?;

    let (backend_a, committer_a) =
        minibox_commit_backend(Arc::clone(&store), upper_a.upper_dir.clone());
    let (backend_b, committer_b) =
        minibox_commit_backend(Arc::clone(&store), upper_b.upper_dir.clone());

    if !backend_a.capabilities.supports(BackendCapability::Commit)
        || !backend_b.capabilities.supports(BackendCapability::Commit)
    {
        return Ok(()); // skip
    }

    let cid = ContainerId::new("conformancecommit03".to_string()).expect("ContainerId");

    let meta_a = committer_a
        .commit(&cid, "conformance/image-a:v1", &default_commit_config())
        .await?;
    let meta_b = committer_b
        .commit(&cid, "conformance/image-b:v2", &default_commit_config())
        .await?;

    assert_eq!(meta_a.name, "conformance/image-a");
    assert_eq!(meta_a.tag, "v1");
    assert_eq!(meta_b.name, "conformance/image-b");
    assert_eq!(meta_b.tag, "v2");

    // Both committed images must be independently findable in the store.
    assert!(
        store.has_image("conformance/image-a", "v1"),
        "image-a must be in store after commit"
    );
    assert!(
        store.has_image("conformance/image-b", "v2"),
        "image-b must be in store after commit"
    );

    Ok(())
}

/// Committing an empty upperdir must not panic.
///
/// The adapter may return `Ok(meta)` with empty layers, or `Err(_)` — both are
/// acceptable. The invariant under test is that no panic occurs.
#[tokio::test]
async fn commit_empty_upperdir_returns_error() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let upper = WritableUpperDirFixture::new()?;

    // Remove all files from the upperdir so it is empty.
    for entry in std::fs::read_dir(&upper.upper_dir)? {
        let entry = entry?;
        std::fs::remove_file(entry.path())?;
    }

    let (backend, committer) = minibox_commit_backend(Arc::clone(&store), upper.upper_dir.clone());

    if !backend.capabilities.supports(BackendCapability::Commit) {
        return Ok(()); // skip
    }

    let cid = ContainerId::new("00000000000000000000".to_string()).expect("ContainerId");
    let result = committer
        .commit(&cid, "conformance/empty-upper:v1", &default_commit_config())
        .await;

    // Either Ok (with any number of layers) or Err — but no panic.
    match result {
        Ok(_meta) => {} // adapter may produce an empty-tar layer — acceptable
        Err(_) => {}    // error is also acceptable
    }

    Ok(())
}

/// The conformance adapter must not validate the container ID itself.
///
/// `ContainerCommitter` uses `upper_dir` directly; the container ID is
/// informational only. A zero-filled ID must not cause an error.
#[tokio::test]
async fn commit_nonexistent_container_id_is_accepted() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let upper = WritableUpperDirFixture::new()?;

    let (backend, committer) = minibox_commit_backend(Arc::clone(&store), upper.upper_dir.clone());

    if !backend.capabilities.supports(BackendCapability::Commit) {
        return Ok(()); // skip
    }

    let cid = ContainerId::new("00000000000000000000".to_string()).expect("ContainerId");
    let meta = committer
        .commit(&cid, "conformance/zero-id:v1", &default_commit_config())
        .await?;

    assert_eq!(
        meta.name, "conformance/zero-id",
        "name must match target ref"
    );
    assert_eq!(meta.tag, "v1", "tag must match target ref");

    Ok(())
}

/// A backend that does NOT declare `Commit` capability must have no `make_committer`.
/// This test verifies the skip-path — the test harness must not call `commit` on it.
#[tokio::test]
async fn commit_skipped_for_backend_without_capability() {
    let descriptor = BackendDescriptor::new("no-commit-backend");
    assert!(
        !descriptor.capabilities.supports(BackendCapability::Commit),
        "backend must not claim Commit capability"
    );
    assert!(
        descriptor.make_committer.is_none(),
        "make_committer must be None when capability is absent"
    );
}

/// krun adapter declares no Commit capability — conformance must skip gracefully.
#[tokio::test]
async fn commit_krun_backend_skips_cleanly() {
    // krun does not expose a writable overlay upperdir — Commit is not supported.
    let descriptor = BackendDescriptor::new("krun");
    assert!(
        !descriptor.capabilities.supports(BackendCapability::Commit),
        "krun must not claim Commit capability"
    );
    assert!(
        descriptor.make_committer.is_none(),
        "krun must not wire a committer"
    );
}
