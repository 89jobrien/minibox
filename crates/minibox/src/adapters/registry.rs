//! Docker Hub registry adapter implementing the [`ImageRegistry`] domain trait.
//!
//! This adapter wraps the existing [`RegistryClient`] and [`ImageStore`]
//! infrastructure to implement [`ImageRegistry`] following hexagonal
//! architecture principles: the domain layer depends only on the trait; this
//! adapter wires the trait to the real Docker Hub HTTP API.
//!
//! Selected by `MINIBOX_ADAPTER=native` (the default). Requires network
//! access to `registry-1.docker.io`.

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{ImageMetadata, ImageRegistry, LayerInfo};
use minibox_core::image::ImageStore;
use minibox_core::image::registry::RegistryClient;
use minibox_macros::denormalize_digest;
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
/// use minibox::adapters::DockerHubRegistry;
/// use minibox::domain::ImageRegistry;
/// use minibox::image::ImageStore;
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
    async fn pull_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
    ) -> Result<ImageMetadata> {
        let api_name = image_ref.repository(); // "library/alpine" or "org/image"
        let store_name = image_ref.cache_name(); // "library/alpine" or "ghcr.io/org/image"
        let tag = &image_ref.tag;

        debug!(
            image_name = %store_name,
            image_tag = tag,
            "registry: pulling from Docker Hub"
        );

        // Delegate network I/O and tar extraction to RegistryClient.
        self.client.pull_image(&api_name, tag, &self.store).await?;

        // Locate the on-disk layer directories written by RegistryClient.
        let layer_paths = self.store.get_image_layers(&store_name, tag)?;

        // Build LayerInfo from layer directory names.
        // Directory names encode the digest with ':' replaced by '_' for
        // filesystem compatibility; reverse that substitution here.
        let layers: Vec<LayerInfo> = layer_paths
            .iter()
            .map(|path| {
                let digest = denormalize_digest!(
                    path.file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                );

                LayerInfo {
                    digest,
                    // Size is not readily available without re-parsing the manifest.
                    size: 0,
                }
            })
            .collect();

        Ok(ImageMetadata {
            name: store_name,
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
    use minibox_core::image::manifest::{Descriptor, OciManifest};
    use tempfile::TempDir;

    fn sample_manifest(layer_digests: &[&str]) -> OciManifest {
        OciManifest {
            schema_version: 2,
            media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
            config: Descriptor {
                media_type: "application/vnd.oci.image.config.v1+json".to_string(),
                size: 100,
                digest: "sha256:config123".to_string(),
                platform: None,
            },
            layers: layer_digests
                .iter()
                .map(|d| Descriptor {
                    media_type: "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
                    size: 1000,
                    digest: d.to_string(),
                    platform: None,
                })
                .collect(),
        }
    }

    #[test]
    fn test_registry_creation() {
        let temp_dir = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp_dir.path()).unwrap());

        let registry = DockerHubRegistry::new(store.clone());
        assert!(registry.is_ok());
    }

    #[test]
    fn test_store_accessor_returns_same_arc() {
        let temp_dir = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp_dir.path()).unwrap());
        let registry = DockerHubRegistry::new(store.clone()).unwrap();

        // store() should return an Arc pointing to the same allocation
        assert!(Arc::ptr_eq(registry.store(), &store));
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

    #[tokio::test]
    async fn test_has_image_true_after_store_seeded() {
        let temp_dir = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp_dir.path()).unwrap());

        // Seed the store with a manifest so has_image returns true
        let manifest = sample_manifest(&["sha256:layer1abc"]);
        store
            .store_manifest("library/alpine", "latest", &manifest)
            .unwrap();

        let registry = DockerHubRegistry::new(store).unwrap();
        assert!(registry.has_image("library/alpine", "latest").await);
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

    #[test]
    fn test_get_image_layers_success_with_seeded_store() {
        let temp_dir = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp_dir.path()).unwrap());

        // Seed the store: store a manifest with two layers
        let digests = ["sha256:aaa111", "sha256:bbb222"];
        let manifest = sample_manifest(&digests);
        store
            .store_manifest("library/busybox", "stable", &manifest)
            .unwrap();

        let registry = DockerHubRegistry::new(store).unwrap();
        let layers = registry
            .get_image_layers("library/busybox", "stable")
            .expect("get_image_layers should succeed for seeded image");

        assert_eq!(layers.len(), 2);
        // ImageStore encodes digest as sha256_<hex> in the directory name
        assert!(
            layers[0].to_string_lossy().contains("sha256_aaa111"),
            "first layer path should contain sha256_aaa111, got: {:?}",
            layers[0]
        );
        assert!(
            layers[1].to_string_lossy().contains("sha256_bbb222"),
            "second layer path should contain sha256_bbb222, got: {:?}",
            layers[1]
        );
    }

    #[tokio::test]
    async fn test_pull_image_fails_for_invalid_image_ref() {
        let temp_dir = TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp_dir.path()).unwrap());
        let registry = DockerHubRegistry::new(store).unwrap();

        // An ImageRef with a clearly bogus name will fail at the RegistryClient
        // network layer (token auth or manifest fetch) — exercises the pull_image
        // error return path without requiring a real Docker Hub connection to succeed.
        let image_ref = crate::image::reference::ImageRef {
            registry: "docker.io".to_string(),
            namespace: "minibox-test-nonexistent-ns-xyz".to_string(),
            name: "image-does-not-exist-abc999".to_string(),
            tag: "nosuchtagXYZ".to_string(),
        };

        let result = registry.pull_image(&image_ref).await;
        assert!(
            result.is_err(),
            "pull_image should return an error for a non-existent image"
        );
    }
}
