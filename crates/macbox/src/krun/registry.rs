//! KrunRegistry — ImageRegistry adapter for the krun VM backend.
//!
//! `KrunRegistry` is a thin newtype wrapper over [`DockerHubRegistry`] from
//! `minibox-core`. The krun adapter uses standard Docker Hub image pulls; this
//! module wires the [`ImageRegistry`] port so the krun adapter suite is
//! self-contained and its registry implementation is explicit.

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::adapters::DockerHubRegistry;
use minibox_core::domain::{AsAny, ImageMetadata, ImageRegistry};
use minibox_core::image::ImageStore;
use std::path::PathBuf;
use std::sync::Arc;

/// Registry adapter for the krun microVM backend.
///
/// Delegates all operations to [`DockerHubRegistry`]. The krun backend
/// has no registry-specific logic — it reuses standard Docker Hub pulls.
///
/// # Manifest size limit
///
/// Inherited from the underlying [`DockerHubRegistry`]: 10 MiB
/// (`MAX_MANIFEST_SIZE = 10 * 1024 * 1024`).
#[derive(Debug, Clone)]
pub struct KrunRegistry {
    inner: DockerHubRegistry,
}

impl KrunRegistry {
    /// Create a new `KrunRegistry` backed by the given image store.
    pub fn new(store: Arc<ImageStore>) -> Result<Self> {
        let inner = DockerHubRegistry::new(store)?;
        Ok(Self { inner })
    }

    /// Return the manifest size cap enforced by the underlying registry client.
    ///
    /// Used by K-I-05 to verify the size limit configuration is present.
    pub const fn manifest_size_limit_bytes() -> u64 {
        10 * 1024 * 1024 // 10 MiB — matches MAX_MANIFEST_SIZE in minibox-oci
    }
}

impl AsAny for KrunRegistry {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait]
impl ImageRegistry for KrunRegistry {
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        self.inner.has_image(name, tag).await
    }

    async fn pull_image(
        &self,
        image_ref: &minibox_core::image::reference::ImageRef,
    ) -> Result<ImageMetadata> {
        self.inner.pull_image(image_ref).await
    }

    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>> {
        self.inner.get_image_layers(name, tag)
    }
}
