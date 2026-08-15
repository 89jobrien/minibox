use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

use super::{AsAny, ContainerId, DynProgressSink};

// ---------------------------------------------------------------------------
// Image Registry Port
// ---------------------------------------------------------------------------

/// Abstraction for pulling container images from a registry.
///
/// This trait defines the contract for image registry implementations.
/// Implementations might include Docker Hub, GitHub Container Registry,
/// Quay.io, or private registries.
///
/// # Examples
///
/// ```rust,ignore
/// use minibox::domain::ImageRegistry;
///
/// struct DockerHubRegistry {
///     client: RegistryClient,
///     store: ImageStore,
/// }
///
/// #[async_trait]
/// impl ImageRegistry for DockerHubRegistry {
///     async fn has_image(&self, name: &str, tag: &str) -> bool {
///         self.store.has_image(name, tag)
///     }
///     // ... implement other methods
/// }
/// ```
#[async_trait]
pub trait ImageRegistry: AsAny + Send + Sync {
    /// Check if an image exists locally in the store.
    ///
    /// Returns `true` if the image has been pulled and cached locally,
    /// `false` otherwise.
    async fn has_image(&self, name: &str, tag: &str) -> bool;

    /// Pull an image from the registry and store it locally.
    ///
    /// Downloads all layers, verifies their digests, and extracts them
    /// to the local image store.
    ///
    /// # Arguments
    ///
    /// * `name` - Image name (e.g., `"library/ubuntu"`)
    /// * `tag` - Image tag (e.g., `"22.04"`)
    ///
    /// # Returns
    ///
    /// Metadata about the pulled image including layer information.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Authentication fails
    /// - Network request fails
    /// - Manifest is invalid
    /// - Layer download fails
    /// - Digest verification fails
    async fn pull_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
    ) -> Result<ImageMetadata>;

    /// Get the layer paths for a cached image.
    ///
    /// Returns an ordered list of layer directories (bottom-to-top) that
    /// can be used to construct an overlay filesystem.
    ///
    /// # Arguments
    ///
    /// * `name` - Image name
    /// * `tag` - Image tag
    ///
    /// # Returns
    ///
    /// Vector of absolute paths to extracted layer directories.
    ///
    /// # Errors
    ///
    /// Returns an error if the image is not cached locally.
    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>>;
}

// ---------------------------------------------------------------------------
// Registry Router Port
// ---------------------------------------------------------------------------

/// Port for routing an image reference to the appropriate [`ImageRegistry`] adapter.
///
/// Implementations select the registry based on the image's hostname (or any
/// other criteria) and return a reference to the corresponding adapter.
///
/// # Implementations
///
/// - [`minibox_core::adapters::HostnameRegistryRouter`]: routes by lowercase hostname;
///   falls back to a default registry for unrecognised hostnames.
///
/// # Example
///
/// ```rust,ignore
/// use minibox_core::domain::{DynRegistryRouter, RegistryRouter};
///
/// let router: DynRegistryRouter = Arc::new(HostnameRegistryRouter::new(
///     docker_hub_registry,
///     [("ghcr.io", ghcr_registry)],
/// ));
/// let registry = router.route(&image_ref);
/// ```
pub trait RegistryRouter: Send + Sync {
    /// Return the registry adapter that should handle `image_ref`.
    fn route(&self, image_ref: &crate::image::reference::ImageRef) -> &dyn ImageRegistry;
}

/// Port for loading a local OCI image tarball into the image store.
///
/// Implementations:
/// - `NativeImageLoader`: extracts tarball directly into `ImageStore`
/// - `ColimaRegistry`: delegates to `nerdctl load -i <path>` in the Lima VM
#[async_trait]
pub trait ImageLoader: Send + Sync {
    /// Load the OCI tarball at `path` and register it as `name:tag`.
    async fn load_image(&self, path: &std::path::Path, name: &str, tag: &str)
    -> anyhow::Result<()>;
}

/// Metadata about a pulled container image.
#[derive(Debug, Clone)]
pub struct ImageMetadata {
    /// Fully qualified image name (e.g., `"library/ubuntu"`).
    pub name: String,
    /// Image tag (e.g., `"22.04"`).
    pub tag: String,
    /// List of layers in bottom-to-top order.
    pub layers: Vec<LayerInfo>,
}

/// Information about a single image layer.
#[derive(Debug, Clone)]
pub struct LayerInfo {
    /// Digest of the layer (e.g., `"sha256:abc123..."`).
    pub digest: String,
    /// Size of the layer in bytes.
    pub size: u64,
}

// ---------------------------------------------------------------------------
// Image Pusher Port
// ---------------------------------------------------------------------------

/// Credentials for authenticating to a registry.
#[derive(Debug, Clone)]
pub enum RegistryCredentials {
    Anonymous,
    Basic { username: String, password: String },
    Token(String),
}

/// Result of a successful image push.
#[derive(Debug, Clone)]
pub struct PushResult {
    pub digest: String,
    pub size_bytes: u64,
}

/// Push progress update.
#[derive(Debug, Clone)]
pub struct PushProgress {
    pub layer_digest: String,
    pub bytes_uploaded: u64,
    pub total_bytes: u64,
}

/// Port for pushing images to OCI-compliant registries.
#[async_trait]
pub trait ImagePusher: AsAny + Send + Sync {
    async fn push_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
        credentials: &RegistryCredentials,
        progress_tx: Option<DynProgressSink<PushProgress>>,
    ) -> anyhow::Result<PushResult>;
}

/// Type alias for a shared, dynamic [`ImagePusher`] implementation.
pub type DynImagePusher = Arc<dyn ImagePusher>;

// ---------------------------------------------------------------------------
// Container Committer Port
// ---------------------------------------------------------------------------

/// Configuration for committing a container to a new image.
#[derive(Debug, Clone)]
pub struct CommitConfig {
    pub author: Option<String>,
    pub message: Option<String>,
    pub env_overrides: Vec<String>,
    pub cmd_override: Option<Vec<String>>,
}

/// Port for snapshotting a container's filesystem diff into a new image.
#[async_trait]
pub trait ContainerCommitter: AsAny + Send + Sync {
    async fn commit(
        &self,
        container_id: &ContainerId,
        target_ref: &str,
        config: &CommitConfig,
    ) -> anyhow::Result<ImageMetadata>;
}

/// Type alias for a shared, dynamic [`ContainerCommitter`] implementation.
pub type DynContainerCommitter = Arc<dyn ContainerCommitter>;

// ---------------------------------------------------------------------------
// Image Builder Port
// ---------------------------------------------------------------------------

/// Context directory and Dockerfile location for a build.
#[derive(Debug, Clone)]
pub struct BuildContext {
    /// Directory that serves as the build context (files available to COPY/ADD).
    pub directory: std::path::PathBuf,
    /// Path to the Dockerfile, relative to `directory`.
    pub dockerfile: std::path::PathBuf,
}

/// Configuration for an image build operation.
#[derive(Debug, Clone)]
pub struct BuildConfig {
    /// Target image tag (e.g. `"myapp:latest"`).
    pub tag: String,
    /// Build-time argument overrides (ARG key=value).
    pub build_args: Vec<(String, String)>,
    /// When `true`, skip any cached layers and rebuild from scratch.
    pub no_cache: bool,
}

/// A progress update emitted while a build is running.
#[derive(Debug, Clone)]
pub struct BuildProgress {
    /// 1-based index of the current step.
    pub step: u32,
    /// Total number of steps in the Dockerfile.
    pub total_steps: u32,
    /// Human-readable description of the current step.
    pub message: String,
}

/// Port for building container images from a Dockerfile.
#[async_trait]
pub trait ImageBuilder: AsAny + Send + Sync {
    /// Build an image from the given context and config, streaming progress via `progress_tx`.
    ///
    /// Returns [`ImageMetadata`] for the newly built image on success.
    async fn build_image(
        &self,
        context: &BuildContext,
        config: &BuildConfig,
        progress_tx: DynProgressSink<BuildProgress>,
    ) -> anyhow::Result<ImageMetadata>;
}

/// Type alias for a shared, dynamic [`ImageBuilder`] implementation.
pub type DynImageBuilder = Arc<dyn ImageBuilder>;
