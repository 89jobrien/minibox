//! Docker Hub registry adapter implementing the [`ImageRegistry`] domain trait.
//!
//! This adapter wraps the existing [`RegistryClient`] and [`ImageStore`]
//! infrastructure to implement [`ImageRegistry`] following hexagonal
//! architecture principles: the domain layer depends only on the trait; this
//! adapter wires the trait to the real Docker Hub HTTP API.
//!
//! Selected by `MINIBOX_ADAPTER=native` (the default). Requires network
//! access to `registry-1.docker.io`.

use crate::as_any;
use crate::domain::{ImageMetadata, ImageRegistry, LayerInfo};
use crate::image::ImageStore;
use crate::image::registry::RegistryClient;
use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

/// Docker Hub registry implementation of the [`ImageRegistry`] trait.
///
/// Provides access to Docker Hub's public registry API, supporting anonymous
/// pulls of public images. Delegates HTTP operations to [`RegistryClient`] and
/// local layer caching to [`ImageStore`].
///
/// # Example
///
/// ```rust,ignore
/// use linuxbox::adapters::DockerHubRegistry;
/// use linuxbox::domain::ImageRegistry;
/// use linuxbox::image::ImageStore;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let store = Arc::new(ImageStore::new("/var/lib/minibox/images")?);
///     let registry = DockerHubRegistry::new(store.clone())?;
///
///     let metadata = registry.pull_image("library/alpine", "latest").await?;
///     println!("Pulled image with {} layers", metadata.layers.len());
///
///     Ok(())
/// }
/// ```
///
/// # Thread Safety
///
/// `Send + Sync` — safe to share across threads behind an `Arc`.
#[derive(Debug, Clone)]
pub struct DockerHubRegistry {
    /// HTTP client for Docker Hub v2 API calls (token auth, manifest/blob fetch).
    client: RegistryClient,
    /// Local image storage — extracted layer directories and manifests on disk.
    store: Arc<ImageStore>,
}

impl DockerHubRegistry {
    /// Create a new Docker Hub registry adapter.
    ///
    /// # Arguments
    ///
    /// * `store` - Shared reference to the local image store used for caching layers.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be initialised
    /// (e.g. TLS initialisation failure).
    pub fn new(store: Arc<ImageStore>) -> Result<Self> {
        let client = RegistryClient::new()?;
        Ok(Self { client, store })
    }

    /// Return a reference to the underlying image store.
    ///
    /// Useful for callers that need direct store access (e.g. checking disk
    /// usage or performing manual cache cleanup) without going through the
    /// registry abstraction.
    pub fn store(&self) -> &Arc<ImageStore> {
        &self.store
    }
}

as_any!(DockerHubRegistry);

#[async_trait]
impl ImageRegistry for DockerHubRegistry {
    /// Return `true` if the image is already present in the local cache.
    ///
    /// Does not make any network requests.
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        debug!(
            image_name = name,
            image_tag = tag,
            "registry: checking local cache"
        );
        self.store.has_image(name, tag)
    }

    /// Pull the image from Docker Hub and return its metadata.
    ///
    /// Downloads all missing layers to the local [`ImageStore`]. If the image
    /// is already cached, layers are not re-downloaded.
    ///
    /// The `size` field in each returned [`LayerInfo`] is `0` because the
    /// layer size is not readily available without re-reading the manifest
    /// after the pull; the digest is derived from the on-disk directory name.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`RegistryClient::pull_image`] (network, auth,
    /// manifest parse) or from [`ImageStore::get_image_layers`] if the layer
    /// directories cannot be located after a successful pull.
    async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata> {
        debug!(
            image_name = name,
            image_tag = tag,
            "registry: pulling from Docker Hub"
        );

        // Delegate network I/O and tar extraction to RegistryClient.
        self.client.pull_image(name, tag, &self.store).await?;

        // Locate the on-disk layer directories written by RegistryClient.
        let layer_paths = self.store.get_image_layers(name, tag)?;

        // Build LayerInfo from layer directory names.
        // Directory names encode the digest with ':' replaced by '_' for
        // filesystem compatibility; reverse that substitution here.
        let layers: Vec<LayerInfo> = layer_paths
            .iter()
            .map(|path| {
                let digest = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .replace('_', ":");

                LayerInfo {
                    digest,
                    // Size is not readily available without re-parsing the manifest.
                    size: 0,
                }
            })
            .collect();

        Ok(ImageMetadata {
            name: name.to_string(),
            tag: tag.to_string(),
            layers,
        })
    }

    /// Return the ordered list of layer directory paths for a locally cached image.
    ///
    /// Paths are suitable for use as overlay `lowerdir` entries when setting
    /// up a container rootfs. The image must have been pulled first.
    ///
    /// # Errors
    ///
    /// Returns an error if the image is not in the local cache.
    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>> {
        debug!(
            image_name = name,
            image_tag = tag,
            "registry: resolving layer paths"
        );
        self.store.get_image_layers(name, tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_registry_creation() {
        let temp_dir = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp_dir.path()).unwrap());

        let registry = DockerHubRegistry::new(store.clone());
        assert!(registry.is_ok());
    }

    #[tokio::test]
    async fn test_has_image_for_nonexistent_image() {
        let temp_dir = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp_dir.path()).unwrap());
        let registry = DockerHubRegistry::new(store).unwrap();

        // Non-existent image should return false
        let exists = registry.has_image("library/nonexistent", "latest").await;
        assert!(!exists);
    }

    #[test]
    fn test_get_image_layers_for_nonexistent_image() {
        let temp_dir = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp_dir.path()).unwrap());
        let registry = DockerHubRegistry::new(store).unwrap();

        // Non-existent image should return error
        let result = registry.get_image_layers("library/nonexistent", "latest");
        assert!(result.is_err());
    }
}
