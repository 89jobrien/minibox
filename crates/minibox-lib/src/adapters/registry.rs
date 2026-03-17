//! Docker Hub registry adapter implementing the ImageRegistry trait.
//!
//! This adapter wraps the existing [`RegistryClient`] and [`ImageStore`]
//! infrastructure code to implement the domain's [`ImageRegistry`] trait,
//! following hexagonal architecture principles.

use crate::domain::{AsAny, ImageMetadata, ImageRegistry, LayerInfo};
use crate::image::ImageStore;
use crate::image::registry::RegistryClient;
use anyhow::Result;
use async_trait::async_trait;
use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

/// Docker Hub registry implementation of the [`ImageRegistry`] trait.
///
/// This adapter provides access to Docker Hub's public registry API,
/// supporting anonymous pulls of public images. It delegates to the
/// existing [`RegistryClient`] for HTTP operations and [`ImageStore`]
/// for local caching.
///
/// # Example
///
/// ```rust,ignore
/// use minibox_lib::adapters::DockerHubRegistry;
/// use minibox_lib::domain::ImageRegistry;
/// use minibox_lib::image::ImageStore;
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
/// This adapter is `Send + Sync` and can be safely shared across threads
/// using `Arc<DockerHubRegistry>`.
#[derive(Debug, Clone)]
pub struct DockerHubRegistry {
    /// HTTP client for Docker Hub API.
    client: RegistryClient,
    /// Local image storage.
    store: Arc<ImageStore>,
}

impl DockerHubRegistry {
    /// Create a new Docker Hub registry adapter.
    ///
    /// # Arguments
    ///
    /// * `store` - Shared reference to the local image store
    ///
    /// # Returns
    ///
    /// A new adapter instance ready to pull images from Docker Hub.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be initialized (e.g.,
    /// TLS initialization failure).
    pub fn new(store: Arc<ImageStore>) -> Result<Self> {
        let client = RegistryClient::new()?;
        Ok(Self { client, store })
    }

    /// Get the underlying image store.
    ///
    /// Useful for operations that need direct access to the store,
    /// such as checking disk usage or manual cleanup.
    pub fn store(&self) -> &Arc<ImageStore> {
        &self.store
    }
}

impl AsAny for DockerHubRegistry {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[async_trait]
impl ImageRegistry for DockerHubRegistry {
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        debug!("checking if image {}:{} exists locally", name, tag);
        self.store.has_image(name, tag)
    }

    async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata> {
        debug!("pulling image {}:{} from Docker Hub", name, tag);

        // Delegate to existing RegistryClient
        self.client.pull_image(name, tag, &self.store).await?;

        // Extract layer paths to build metadata
        let layer_paths = self.store.get_image_layers(name, tag)?;

        // Build LayerInfo from layer paths
        // Note: We don't have direct access to digest/size without reading manifest,
        // but we can extract digest from directory name
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
                    size: 0, // Size not readily available without manifest access
                }
            })
            .collect();

        Ok(ImageMetadata {
            name: name.to_string(),
            tag: tag.to_string(),
            layers,
        })
    }

    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>> {
        debug!("getting layer paths for image {}:{}", name, tag);
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
