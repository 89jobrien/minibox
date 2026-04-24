//! Mock implementation of [`ImageRegistry`].

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::domain::{AsAny, ImageMetadata, ImageRegistry, LayerInfo};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// MockRegistry
// ---------------------------------------------------------------------------

/// Mock implementation of [`ImageRegistry`] for testing.
///
/// Simulates an image registry without making network requests. Configure
/// pre-cached images and failure behaviour via builder methods before use.
///
/// All state is stored in a shared `Arc<Mutex<…>>` so the mock can be cloned
/// and observed from the test after injection.
#[derive(Debug, Clone)]
pub struct MockRegistry {
    state: Arc<Mutex<MockRegistryState>>,
}

#[derive(Debug)]
pub struct MockRegistryState {
    /// Images that are already "cached" locally (checked by `has_image`).
    cached_images: Vec<(String, String)>, // (name, tag)
    /// Whether `pull_image` calls should succeed (`true`) or fail (`false`).
    pull_should_succeed: bool,
    /// Running count of `pull_image` invocations.
    pull_count: usize,
}

impl MockRegistry {
    /// Create a new mock registry with no cached images and pull success enabled.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockRegistryState {
                cached_images: Vec::new(),
                pull_should_succeed: true,
                pull_count: 0,
            })),
        }
    }

    /// Configure the registry to report `name:tag` as already cached locally.
    ///
    /// May be called multiple times to seed multiple images.
    pub fn with_cached_image(self, name: &str, tag: &str) -> Self {
        self.state
            .lock()
            .unwrap()
            .cached_images
            .push((name.to_string(), tag.to_string()));
        self
    }

    /// Configure all subsequent `pull_image` calls to return an error.
    pub fn with_pull_failure(self) -> Self {
        self.state.lock().unwrap().pull_should_succeed = false;
        self
    }

    /// Return the number of times `pull_image` has been called.
    pub fn pull_count(&self) -> usize {
        self.state.lock().unwrap().pull_count
    }

    /// Synchronous variant of `has_image` — bypasses async machinery.
    ///
    /// Useful in benchmarks and synchronous test helpers where an async
    /// executor is not available.
    pub fn has_image_sync(&self, image: &str, tag: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .cached_images
            .iter()
            .any(|(n, t)| n == image && t == tag)
    }
}

#[async_trait]
impl ImageRegistry for MockRegistry {
    /// Return `true` if the image was seeded via [`with_cached_image`] or pulled successfully.
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .cached_images
            .iter()
            .any(|(n, t)| n == name && t == tag)
    }

    /// Simulate an image pull.
    ///
    /// Increments the pull counter. On success adds the image to the local
    /// cache and returns [`ImageMetadata`] with two fixed mock layers. On
    /// failure (configured via [`with_pull_failure`]) returns an error
    /// without modifying the cache.
    async fn pull_image(
        &self,
        image_ref: &minibox_core::image::reference::ImageRef,
    ) -> Result<ImageMetadata> {
        let name = image_ref.cache_name();
        let tag = image_ref.tag.clone();
        let mut state = self.state.lock().unwrap();
        state.pull_count += 1;

        if !state.pull_should_succeed {
            anyhow::bail!("mock pull failure");
        }

        // Simulate a successful pull by adding the image to the local cache.
        state.cached_images.push((name.clone(), tag.clone()));

        Ok(ImageMetadata {
            name,
            tag,
            layers: vec![
                LayerInfo {
                    digest: "sha256:mock-layer-1".to_string(),
                    size: 1024,
                },
                LayerInfo {
                    digest: "sha256:mock-layer-2".to_string(),
                    size: 2048,
                },
            ],
        })
    }

    /// Return two fixed mock layer paths regardless of the image name or tag.
    fn get_image_layers(&self, _name: &str, _tag: &str) -> Result<Vec<PathBuf>> {
        Ok(vec![
            PathBuf::from("/mock/layer1"),
            PathBuf::from("/mock/layer2"),
        ])
    }
}

impl AsAny for MockRegistry {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

impl Default for MockRegistry {
    fn default() -> Self {
        Self::new()
    }
}
