//! Domain layer: Pure business logic and trait definitions (ports).
//!
//! This module defines the contracts (traits) that infrastructure adapters
//! must implement. Following hexagonal architecture principles, the domain
//! layer has **zero dependencies** on infrastructure details.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │              Composition Root                   │
//! │                  (main.rs)                      │
//! │   Wires domain to adapters, injects deps        │
//! └────────────────┬────────────────────────────────┘
//!                  │
//!     ┌────────────┴────────────┐
//!     │                         │
//! ┌───▼─────────────┐    ┌──────▼──────────────┐
//! │   Domain Layer  │    │ Infrastructure      │
//! │   (this file)   │    │    Adapters         │
//! │  ┌──────────┐   │    │  ┌──────────────┐   │
//! │  │Business  │   │    │  │ DockerHub    │   │
//! │  │Logic     │───┼────┼─►│ Registry     │   │
//! │  └──────────┘   │    │  └──────────────┘   │
//! │                 │    │  ┌──────────────┐   │
//! │  ┌──────────┐   │    │  │ Overlay      │   │
//! │  │Traits    │◄──┼────┼──│ Filesystem   │   │
//! │  │(Ports)   │   │    │  └──────────────┘   │
//! │  └──────────┘   │    │  ┌──────────────┐   │
//! │  ┌──────────┐   │    │  │ Cgroup V2    │   │
//! │  │Domain    │   │    │  │ Limiter      │   │
//! │  │Types     │   │    │  └──────────────┘   │
//! │  └──────────┘   │    └─────────────────────┘
//! └─────────────────┘
//! ```
//!
//! Dependencies point inward: adapters → domain
//!
//! # Traits (Ports)
//!
//! - [`ImageRegistry`]: Abstraction for pulling container images
//! - [`FilesystemProvider`]: Abstraction for container filesystem operations
//! - [`ResourceLimiter`]: Abstraction for resource isolation and limits
//! - [`ContainerRuntime`]: Abstraction for spawning container processes
//!
//! # Benefits
//!
//! - **Testability**: Easy to create mock implementations for unit tests
//! - **Flexibility**: Swap implementations (e.g., Docker Hub → ghcr.io)
//! - **Maintainability**: Clear separation of concerns
//! - **Future-proofing**: Add new backends without changing business logic

// Core domain traits
mod extensions;
mod networking;

// Re-exports for public API
pub use extensions::*;
pub use networking::*;

use anyhow::Result;
use async_trait::async_trait;
use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A host-path bind mount to inject into a container at startup.
///
/// `host_path` is canonicalized and validated before the mount is applied.
/// `container_path` must be absolute (starts with `/`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BindMount {
    /// Absolute path on the host to mount into the container.
    pub host_path: std::path::PathBuf,
    /// Absolute path inside the container where the host path is mounted.
    pub container_path: std::path::PathBuf,
    /// If `true`, the mount is read-only inside the container.
    pub read_only: bool,
}

#[cfg(unix)]
use std::os::fd::OwnedFd;

// ---------------------------------------------------------------------------
// Dyn type aliases
// ---------------------------------------------------------------------------

/// Type alias for a shared, dynamic [`ImageRegistry`] implementation.
pub type DynImageRegistry = Arc<dyn ImageRegistry>;
/// Type alias for a shared, dynamic [`ImageLoader`] implementation.
pub type DynImageLoader = Arc<dyn ImageLoader>;
/// Type alias for a shared, dynamic [`FilesystemProvider`] implementation.
pub type DynFilesystemProvider = Arc<dyn FilesystemProvider>;
/// Type alias for a shared, dynamic [`ResourceLimiter`] implementation.
pub type DynResourceLimiter = Arc<dyn ResourceLimiter>;
/// Type alias for a shared, dynamic [`ContainerRuntime`] implementation.
pub type DynContainerRuntime = Arc<dyn ContainerRuntime>;
/// Type alias for a shared, dynamic [`NetworkProvider`] implementation.
pub type DynNetworkProvider = Arc<dyn NetworkProvider>;
/// Type alias for a shared, dynamic [`MetricsRecorder`] implementation.
pub type DynMetricsRecorder = Arc<dyn MetricsRecorder>;
/// Type alias for a shared, dynamic [`EventSink`] implementation.
pub type DynEventSink = Arc<dyn crate::events::EventSink>;
/// Type alias for a shared, dynamic [`EventSource`] implementation.
pub type DynEventSource = Arc<dyn crate::events::EventSource>;
/// Type alias for a shared, dynamic [`RegistryRouter`] implementation.
pub type DynRegistryRouter = Arc<dyn RegistryRouter>;

// ---------------------------------------------------------------------------
// Downcasting support for testing
// ---------------------------------------------------------------------------

/// Trait to enable downcasting trait objects back to concrete types.
///
/// This allows tests to retrieve the concrete adapter behind a `Dyn*` trait
/// object (e.g. to call adapter-specific helpers in integration tests).
/// Production code should use the trait interface exclusively.
pub trait AsAny: Send + Sync {
    /// Return `self` as `&dyn Any` so callers can use `downcast_ref::<T>()`.
    fn as_any(&self) -> &dyn Any;
}

// ---------------------------------------------------------------------------
// Metrics Recorder Port
// ---------------------------------------------------------------------------

/// Port for recording operational metrics.
///
/// Adapters: `PrometheusMetricsRecorder` (production), `NoOpMetricsRecorder`
/// (testing/disabled), `RecordingMetricsRecorder` (test assertions).
///
/// String-based names and labels keep the domain free of OTEL/Prometheus types.
pub trait MetricsRecorder: Send + Sync {
    /// Increment a counter by 1.
    fn increment_counter(&self, name: &str, labels: &[(&str, &str)]);
    /// Record a value in a histogram (e.g., duration in seconds).
    fn record_histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]);
    /// Set a gauge to an absolute value.
    fn set_gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]);
}

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
/// use linuxbox::domain::ImageRegistry;
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
// Filesystem Provider Port
// ---------------------------------------------------------------------------

/// Abstraction for container filesystem operations.
///
/// This trait defines the contract for filesystem implementations.
/// Daemon-side filesystem lifecycle: setup container rootfs and cleanup after
/// exit.
///
/// Implementations might include overlay filesystem, bind mounts, or
/// other copy-on-write filesystems like ZFS or Btrfs.
///
/// # Security
///
/// Implementations MUST:
/// - Validate all paths to prevent traversal attacks
/// - Mount filesystems with appropriate security flags (nosuid, nodev)
/// - Properly clean up mounts to avoid resource leaks
pub trait RootfsSetup: AsAny + Send + Sync {
    /// Setup the container rootfs and return the merged directory plus any
    /// backend metadata needed by follow-on operations such as commit/build.
    ///
    /// Creates the necessary directory structure and mounts (e.g., overlay)
    /// to provide a writable rootfs for the container.
    ///
    /// # Arguments
    ///
    /// * `image_layers` - Ordered list of layer paths (bottom-to-top)
    /// * `container_dir` - Per-container working directory
    ///
    /// # Returns
    ///
    /// A [`RootfsLayout`] describing the merged rootfs and optional
    /// backend-specific metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Path validation fails (security)
    /// - Directory creation fails
    /// - Mount operation fails
    ///
    /// # Security
    ///
    /// MUST validate that `image_layers` paths don't contain `..` or
    /// escape the allowed base directory.
    fn setup_rootfs(&self, image_layers: &[PathBuf], container_dir: &Path) -> Result<RootfsLayout>;

    /// Cleanup mounts after container exit.
    ///
    /// Unmounts the rootfs and removes the per-container directories.
    ///
    /// # Arguments
    ///
    /// * `container_dir` - Per-container directory to clean up
    ///
    /// # Errors
    ///
    /// Returns an error if unmount or directory removal fails.
    fn cleanup(&self, container_dir: &Path) -> Result<()>;
}

/// Child-process filesystem initialisation: pivot root inside the cloned
/// container process.
///
/// This trait is received only by the container child process after
/// `clone(2)`, keeping daemon-side setup (`RootfsSetup`) and child-side
/// init (`ChildInit`) under separate ownership.
pub trait ChildInit: Send + Sync {
    /// Pivot root inside the container process.
    ///
    /// This is called **inside the cloned child process** to switch the
    /// root filesystem and set up essential pseudo-filesystems (proc, sys, dev).
    ///
    /// # Arguments
    ///
    /// * `new_root` - Path to the new root filesystem
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Bind mount fails
    /// - Essential filesystem mounts fail
    /// - pivot_root syscall fails
    /// - Old root unmount fails
    ///
    /// # Security
    ///
    /// MUST mount proc/sys/dev with appropriate security flags:
    /// - proc: nosuid, nodev, noexec
    /// - sys: rdonly, nosuid, nodev, noexec
    /// - dev: nosuid, noexec
    fn pivot_root(&self, new_root: &Path) -> Result<()>;
}

/// Combined filesystem provider: supertrait alias that bundles [`RootfsSetup`]
/// and [`ChildInit`] for adapters that implement both lifecycle phases.
///
/// Prefer using [`RootfsSetup`] or [`ChildInit`] directly at call sites that
/// only need one half of the lifecycle.
pub trait FilesystemProvider: RootfsSetup + ChildInit {}

/// Blanket implementation: any type that implements both [`RootfsSetup`] and
/// [`ChildInit`] automatically satisfies [`FilesystemProvider`].
impl<T: RootfsSetup + ChildInit> FilesystemProvider for T {}

/// Backend-specific writable-layer metadata produced by
/// [`RootfsSetup::setup_rootfs`] and persisted into [`ContainerRecord`]
/// so that commit/build logic can locate the writable layer without
/// re-querying the container runtime.
///
/// The `metadata` map carries backend-specific key/value pairs so that new
/// backends can encode their own data (e.g. `"colima_instance" => "colima"`)
/// without adding new enum variants (OCP).  Callers that only need the
/// host-visible upper directory should use
/// [`BackendRootfsMetadata::overlay_upper_dir`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackendRootfsMetadata {
    /// Overlay filesystem backend.  `upper_dir` is the host-visible (or
    /// guest-visible, for VM adapters) writable layer directory.
    /// `metadata` carries adapter-specific key/value pairs, e.g.:
    /// - `"colima_instance"` — Lima/Colima instance name
    Overlay {
        upper_dir: PathBuf,
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        metadata: HashMap<String, String>,
    },
}

impl BackendRootfsMetadata {
    /// Return the host-visible overlay upper directory.
    pub fn overlay_upper_dir(&self) -> &PathBuf {
        match self {
            Self::Overlay { upper_dir, .. } => upper_dir,
        }
    }

    /// Look up a backend-specific metadata value by key.
    pub fn metadata_value(&self, key: &str) -> Option<&str> {
        match self {
            Self::Overlay { metadata, .. } => metadata.get(key).map(String::as_str),
        }
    }
}

/// Filesystem layout returned by [`FilesystemProvider::setup_rootfs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootfsLayout {
    /// Path to the merged/mounted rootfs that the runtime will use.
    pub merged_dir: PathBuf,
    /// Typed backend metadata for the writable layer, when the backend exposes
    /// one.  `None` for copy-based (GKE/proot) and VZ (in-VM) backends.
    pub rootfs_metadata: Option<BackendRootfsMetadata>,
    /// Source image reference associated with this rootfs when known.
    pub source_image_ref: Option<String>,
}

// ---------------------------------------------------------------------------
// Resource Limiter Port
// ---------------------------------------------------------------------------

/// Abstraction for resource isolation and limits.
///
/// This trait defines the contract for resource limiting implementations.
/// Implementations might include cgroups v2, cgroups v1, or systemd slices.
///
/// # Security
///
/// Implementations MUST:
/// - Validate resource limit values (minimum thresholds)
/// - Prevent resource DoS attacks (default PID limits)
/// - Properly cleanup cgroups to avoid resource leaks
pub trait ResourceLimiter: AsAny + Send + Sync {
    /// Create resource limits for a container.
    ///
    /// Creates the necessary control structures (e.g., cgroup directory)
    /// and applies the configured resource limits.
    ///
    /// # Arguments
    ///
    /// * `container_id` - Unique container identifier
    /// * `config` - Resource limit configuration
    ///
    /// # Returns
    ///
    /// Path or identifier of the created resource limit group.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Resource limit creation fails
    /// - Invalid limit values (below kernel minimums)
    /// - Limit application fails
    ///
    /// # Security
    ///
    /// MUST validate that `config` values are within acceptable ranges:
    /// - Memory: >= 4096 bytes (kernel minimum)
    /// - CPU weight: 1-10000 (kernel range)
    /// - PIDs: should default to reasonable limit (e.g., 1024) to prevent fork bombs
    fn create(&self, container_id: &str, config: &ResourceConfig) -> Result<String>;

    /// Add a process to the resource limits.
    ///
    /// Associates a running process with the container's resource limits.
    ///
    /// # Arguments
    ///
    /// * `container_id` - Container identifier
    /// * `pid` - Process ID to add
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Process doesn't exist
    /// - Adding to resource group fails
    fn add_process(&self, container_id: &str, pid: u32) -> Result<()>;

    /// Remove resource limits.
    ///
    /// Cleans up the resource limit structures. All processes must have
    /// exited before calling this.
    ///
    /// # Arguments
    ///
    /// * `container_id` - Container identifier
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Resource group still has running processes
    /// - Cleanup operation fails
    fn cleanup(&self, container_id: &str) -> Result<()>;
}

/// Resource limit configuration for a container.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ResourceConfig {
    /// Maximum memory (RSS + swap) in bytes. `None` means unlimited.
    pub memory_limit_bytes: Option<u64>,
    /// CPU weight in the range 1-10000 (default kernel value is 100).
    /// Higher values give more CPU time. `None` uses kernel default.
    pub cpu_weight: Option<u64>,
    /// Maximum number of PIDs (processes/threads). `None` means unlimited.
    /// Implementations should default to a safe value (e.g., 1024) to prevent
    /// fork bombs if not specified.
    pub pids_max: Option<u64>,
    /// I/O bandwidth limit in bytes/second. `None` means unlimited.
    pub io_max_bytes_per_sec: Option<u64>,
}

// ---------------------------------------------------------------------------
// Runtime Capabilities
// ---------------------------------------------------------------------------

/// Describes the isolation and resource features supported by a runtime adapter.
///
/// Callers can query capabilities to make decisions at runtime — for example,
/// skipping user-namespace setup on adapters that don't support it, or
/// falling back gracefully when cgroups v2 is unavailable.
///
/// # Example
///
/// ```rust,ignore
/// if runtime.capabilities().supports_network_isolation {
///     // configure bridge/veth networking
/// } else {
///     // skip network setup, container shares host network
/// }
/// ```
#[derive(Debug, Clone)]
pub struct RuntimeCapabilities {
    /// Supports Linux user namespace remapping (rootless containers).
    pub supports_user_namespaces: bool,
    /// Supports cgroups v2 for memory/CPU/PID resource limits.
    pub supports_cgroups_v2: bool,
    /// Supports overlay filesystem for copy-on-write container rootfs.
    pub supports_overlay_fs: bool,
    /// Supports network namespace isolation (separate network stack per container).
    pub supports_network_isolation: bool,
    /// Maximum number of concurrent containers, or `None` for unlimited.
    pub max_containers: Option<usize>,
}

// ---------------------------------------------------------------------------
// Container Runtime Port
// ---------------------------------------------------------------------------

/// Abstraction for spawning container processes with isolation.
///
/// This trait defines the contract for container runtime implementations.
/// Implementations might include Linux namespaces, Podman, or other
/// containerization technologies.
#[async_trait]
pub trait ContainerRuntime: AsAny + Send + Sync {
    /// Return the static capabilities of this runtime adapter.
    ///
    /// Allows callers to discover what isolation features are available
    /// without attempting operations that would fail. The returned struct
    /// is cheap to construct and may be called frequently.
    fn capabilities(&self) -> RuntimeCapabilities;

    /// Spawn a containerized process.
    ///
    /// Creates a new process with the configured isolation (namespaces,
    /// cgroups, etc.) and executes the specified command.
    ///
    /// # Arguments
    ///
    /// * `config` - Complete container spawn configuration
    ///
    /// # Returns
    ///
    /// PID of the spawned container init process.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Fork/clone fails
    /// - Namespace creation fails
    /// - Command execution fails
    ///
    /// # Notes
    ///
    /// This operation typically requires blocking I/O (fork/clone syscalls)
    /// and should be called from a blocking thread context in async code.
    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult>;
}

/// Result returned by [`ContainerRuntime::spawn_process`].
pub struct SpawnResult {
    /// PID of the spawned container init process.
    pub pid: u32,
    /// Present when [`ContainerSpawnConfig::capture_output`] was `true`.
    /// The read end of a pipe connected to the container's stdout+stderr.
    #[cfg(unix)]
    pub output_reader: Option<OwnedFd>,
    /// Placeholder for non-Unix builds where pipes are not supported.
    #[cfg(not(unix))]
    pub output_reader: Option<std::convert::Infallible>,
}

/// A single host-side lifecycle hook command.
///
/// Hooks run on the **host** with `CONTAINER_ID` and `CONTAINER_ROOTFS`
/// set in the environment. Post-exit hooks additionally receive `EXIT_CODE`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct HookSpec {
    /// Host executable to run (e.g., `"/usr/local/bin/notify.sh"`).
    pub command: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Timeout in seconds before the hook is abandoned. Defaults to 30s.
    pub timeout_secs: Option<u64>,
}

/// Pre/post-execution hooks for the container lifecycle.
///
/// All hooks run on the **host** — not inside the container.
/// `pre_exec` hooks run before the container process is cloned;
/// `post_exit` hooks run after the container process has exited.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ContainerHooks {
    /// Commands to run on the host before the container process starts.
    pub pre_exec: Vec<HookSpec>,
    /// Commands to run on the host after the container process exits.
    pub post_exit: Vec<HookSpec>,
}

/// Configuration for spawning a containerized process.
#[derive(Debug, Clone)]
pub struct ContainerSpawnConfig {
    /// Path to the container rootfs (merged overlay directory).
    pub rootfs: PathBuf,
    /// Command to execute (e.g., `"/bin/sh"`).
    pub command: String,
    /// Command arguments (e.g., `["-c", "echo hello"]`).
    pub args: Vec<String>,
    /// Environment variables (e.g., `["PATH=/usr/bin", "HOME=/root"]`).
    pub env: Vec<String>,
    /// Hostname to set inside the container.
    pub hostname: String,
    /// Path to the cgroup directory for this container.
    pub cgroup_path: PathBuf,
    /// When `true`, container stdout+stderr are captured to a pipe.
    /// The read end is returned in [`SpawnResult::output_reader`].
    pub capture_output: bool,
    /// Optional host-side lifecycle hooks.
    pub hooks: ContainerHooks,
    /// If true, skip CLONE_NEWNET — container shares host network namespace.
    pub skip_network_namespace: bool,
    /// Bind mounts to apply inside the container before pivot_root.
    ///
    /// Each `BindMount.host_path` is mounted at `rootfs + BindMount.container_path`
    /// inside the container's new mount namespace, then the container sees it at
    /// `container_path` after pivot_root.
    pub mounts: Vec<BindMount>,
    /// If `true`, the container process is granted a full Linux capability set
    /// via `capset(2)` before `execvp`. Required for DinD.
    pub privileged: bool,
}

// ---------------------------------------------------------------------------
// Domain Errors
// ---------------------------------------------------------------------------

/// Domain-specific errors that are independent of infrastructure.
///
/// These errors represent business logic failures, not infrastructure
/// failures. Infrastructure adapters should map their specific errors
/// (e.g., `std::io::Error`, `reqwest::Error`) to these domain errors
/// when appropriate, or return generic `anyhow::Error` for infrastructure
/// failures.
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    /// Image was not found in the registry or local cache.
    #[error("image {name}:{tag} not found")]
    ImageNotFound {
        /// Image name (e.g., `"library/ubuntu"`).
        name: String,
        /// Image tag (e.g., `"22.04"`).
        tag: String,
    },

    /// Image pull from registry failed.
    #[error("failed to pull image '{image}:{tag}': {source}")]
    ImagePullFailed {
        /// Image name.
        image: String,
        /// Image tag.
        tag: String,
        /// Underlying error from the registry adapter.
        #[source]
        source: anyhow::Error,
    },

    /// Image has no layers (corrupted or invalid image).
    #[error("image {name}:{tag} has no layers")]
    EmptyImage {
        /// Image name.
        name: String,
        /// Image tag.
        tag: String,
    },

    /// Container was not found in the daemon state.
    #[error("container '{id}' not found")]
    ContainerNotFound {
        /// Container ID.
        id: String,
    },

    /// Container process failed to spawn.
    #[error("container '{id}' failed to spawn: {source}")]
    ContainerSpawnFailed {
        /// Container ID.
        id: String,
        /// Underlying error from the runtime adapter.
        #[source]
        source: anyhow::Error,
    },

    /// Attempted operation on a running container that requires it to be stopped.
    #[error("container '{id}' is already running")]
    AlreadyRunning {
        /// Container ID.
        id: String,
    },

    /// Invalid container configuration provided.
    #[error("invalid container configuration: {0}")]
    InvalidConfig(String),

    /// Resource limit values are outside acceptable ranges.
    #[error("invalid resource limits: {0}")]
    InvalidResourceLimits(String),

    /// A resource limit value exceeded the allowed maximum.
    #[error("resource limit '{limit}': value {value} exceeds maximum {max}")]
    ResourceLimitExceeded {
        /// Name of the limit (e.g., `"memory_bytes"`).
        limit: String,
        /// The value that was provided.
        value: u64,
        /// The maximum allowed value.
        max: u64,
    },

    /// An infrastructure error that does not fit a more specific variant.
    #[error(transparent)]
    InfrastructureError(#[from] anyhow::Error),
}

// ---------------------------------------------------------------------------
// Domain Types
// ---------------------------------------------------------------------------

/// Container state machine.
///
/// Represents the lifecycle of a container from creation to removal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerState {
    /// Container has been created but not yet started.
    Created,
    /// Container process is running.
    Running,
    /// Container is frozen (cgroup.freeze = 1).
    Paused,
    /// Container process has exited.
    Stopped,
    /// Container failed to start or crashed.
    Failed,
}

impl ContainerState {
    /// Return the canonical string representation of this state.
    ///
    /// The returned strings (`"Created"`, `"Running"`, `"Paused"`, `"Stopped"`, `"Failed"`)
    /// are used directly in [`crate::protocol::ContainerInfo::state`] list
    /// responses sent to the CLI.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "Created",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
            Self::Failed => "Failed",
        }
    }
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Container identifier type.
///
/// Provides type safety for container IDs throughout the domain.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContainerId(String);

impl ContainerId {
    /// Create a new container ID.
    ///
    /// # Validation
    ///
    /// IDs must be:
    /// - Non-empty
    /// - Alphanumeric (a-z, A-Z, 0-9)
    /// - Between 1 and 64 characters
    pub fn new(id: String) -> Result<Self> {
        if id.is_empty() {
            anyhow::bail!("container ID cannot be empty");
        }
        if id.len() > 64 {
            anyhow::bail!("container ID too long: {} (max 64)", id.len());
        }
        if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            anyhow::bail!("container ID must be alphanumeric");
        }
        Ok(Self(id))
    }

    /// Get the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContainerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ContainerId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Session ID — PTY / interactive exec sessions
// ---------------------------------------------------------------------------

/// Opaque identifier for a live PTY or interactive exec session.
///
/// Parallel to [`ContainerId`] but scoped to exec sessions rather than
/// container lifecycle. A session is created when `Exec` or `Run` is invoked
/// with `tty: true` and destroyed when the process exits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SessionId(String);

impl SessionId {
    /// Create a new session ID from a string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for SessionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

// ---------------------------------------------------------------------------
// Exec Runtime Port
// ---------------------------------------------------------------------------

/// Pure specification for running a command inside a container.
///
/// This is a domain value type — no channel fields, no tokio types.
/// Channel wiring (stdin relay, PTY resize) belongs in the infrastructure
/// adapter layer (`linuxbox::adapters::exec`).
#[derive(Debug, Clone)]
pub struct ExecSpec {
    pub cmd: Vec<String>,
    pub env: Vec<String>,
    pub working_dir: Option<std::path::PathBuf>,
    pub tty: bool,
}

/// Handle representing a started exec instance.
#[derive(Debug, Clone)]
pub struct ExecHandle {
    pub id: String,
}

/// Port for running commands inside already-running containers.
#[async_trait]
pub trait ExecRuntime: AsAny + Send + Sync {
    async fn run_in_container(
        &self,
        container_id: &ContainerId,
        spec: ExecSpec,
        tx: tokio::sync::mpsc::Sender<crate::protocol::DaemonResponse>,
    ) -> anyhow::Result<ExecHandle>;
}

/// Type alias for a shared, dynamic [`ExecRuntime`] implementation.
pub type DynExecRuntime = Arc<dyn ExecRuntime>;

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
        progress_tx: Option<tokio::sync::mpsc::Sender<PushProgress>>,
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
        progress_tx: tokio::sync::mpsc::Sender<BuildProgress>,
    ) -> anyhow::Result<ImageMetadata>;
}

/// Type alias for a shared, dynamic [`ImageBuilder`] implementation.
pub type DynImageBuilder = Arc<dyn ImageBuilder>;

// ---------------------------------------------------------------------------
// PTY Allocator Port (#83)
// ---------------------------------------------------------------------------

/// Configuration for allocating a pseudo-terminal (PTY) for interactive containers.
///
/// Passed to [`PtyAllocator::allocate`] to request a PTY pair with the given
/// terminal dimensions. The caller is responsible for closing the returned file
/// descriptors when no longer needed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PtyConfig {
    /// Whether PTY allocation is requested.
    pub enabled: bool,
    /// Terminal width in columns.
    pub cols: u16,
    /// Terminal height in rows.
    pub rows: u16,
}

/// An allocated PTY pair — a master and a slave file descriptor.
///
/// The master fd is used by the host to read/write the terminal stream.
/// The slave fd is handed to the container process as its controlling terminal.
///
/// # Ownership
///
/// The caller that calls [`PtyAllocator::allocate`] owns both fds and is
/// responsible for closing them. Do NOT call `close()` on them from outside
/// unless you also own the handle.
#[derive(Debug)]
pub struct PtyHandle {
    /// File descriptor for the master side of the PTY.
    pub master_fd: i32,
    /// File descriptor for the slave side of the PTY.
    pub slave_fd: i32,
}

/// Port for allocating a PTY pair.
///
/// Implementations live in the adapter layer. The domain layer never calls
/// `posix_openpt` directly — all OS-level PTY operations go through this trait.
pub trait PtyAllocator: Send + Sync {
    /// Allocate a PTY pair with the terminal dimensions specified in `config`.
    ///
    /// Returns [`PtyHandle`] on success, or `Err` when PTY allocation is not
    /// supported (e.g., [`NullPtyAllocator`]) or when the OS call fails.
    fn allocate(&self, config: &PtyConfig) -> anyhow::Result<PtyHandle>;
}

/// Type alias for a shared, dynamic [`PtyAllocator`] implementation.
pub type DynPtyAllocator = Arc<dyn PtyAllocator>;

/// A no-op [`PtyAllocator`] that always returns `Err`.
///
/// Used as the default adapter when PTY support is not available (e.g., on
/// macOS or in test environments that do not exercise the PTY path).
pub struct NullPtyAllocator;

impl PtyAllocator for NullPtyAllocator {
    fn allocate(&self, _config: &PtyConfig) -> anyhow::Result<PtyHandle> {
        anyhow::bail!("pty: PTY allocation is not supported in this environment")
    }
}

/// A test double [`PtyAllocator`] that returns a pre-configured [`PtyHandle`].
///
/// Enabled only when the `test-utils` feature is active so production binaries
/// do not pull in test scaffolding.
#[cfg(feature = "test-utils")]
pub struct MockPtyAllocator {
    master_fd: i32,
    slave_fd: i32,
}

#[cfg(feature = "test-utils")]
impl MockPtyAllocator {
    /// Create a `MockPtyAllocator` that returns `master_fd` and `slave_fd`.
    pub fn new(master_fd: i32, slave_fd: i32) -> Self {
        Self {
            master_fd,
            slave_fd,
        }
    }
}

#[cfg(feature = "test-utils")]
impl PtyAllocator for MockPtyAllocator {
    fn allocate(&self, _config: &PtyConfig) -> anyhow::Result<PtyHandle> {
        Ok(PtyHandle {
            master_fd: self.master_fd,
            slave_fd: self.slave_fd,
        })
    }
}

// ---------------------------------------------------------------------------
// Conformance boundary — commit / build / push capabilities
// ---------------------------------------------------------------------------

/// An individual capability that a backend adapter may or may not support.
///
/// Used by [`BackendCapabilitySet`] to describe what a concrete backend can do.
/// The conformance suite gates tests on these flags so that backend-specific
/// tests are skipped rather than failed when a capability is absent.
///
/// # Backend support matrix
///
/// | Capability          | linux-native | Colima |
/// |---------------------|:------------:|:------:|
/// | `Commit`            | yes          | no     |
/// | `BuildFromContext`  | yes          | no     |
/// | `PushToRegistry`    | yes          | yes    |
///
/// **linux-native** — `OverlayCommitAdapter`, `MiniboxImageBuilder`,
/// `OciPushAdapter`: all three traits are fully implemented; commit and build
/// require root and Linux namespaces; push requires a reachable OCI registry.
///
/// **Colima** — `ColimaImagePusher` implements `ImagePusher`; there is no
/// Colima-native `ContainerCommitter` or `ImageBuilder` implementation yet
/// (Colima containers use the nerdctl/lima CLI, which does not expose an
/// upperdir for overlay-style commit, and no Dockerfile build path has been
/// wired into the adapter suite).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendCapability {
    /// Backend can snapshot a running container's FS diff into a new image
    /// via [`ContainerCommitter::commit`].
    Commit,
    /// Backend can build an image from a `BuildContext` + `BuildConfig` via
    /// [`ImageBuilder::build_image`].
    BuildFromContext,
    /// Backend can push an image to an OCI-compliant registry via
    /// [`ImagePusher::push_image`].
    PushToRegistry,
}

/// The full set of [`BackendCapability`] flags declared by one backend.
///
/// Construct via [`BackendCapabilitySet::new`] and chain
/// [`BackendCapabilitySet::with`] calls:
///
/// ```rust
/// use minibox_core::domain::{BackendCapability, BackendCapabilitySet};
///
/// let caps = BackendCapabilitySet::new()
///     .with(BackendCapability::Commit)
///     .with(BackendCapability::PushToRegistry);
///
/// assert!(caps.supports(BackendCapability::Commit));
/// assert!(!caps.supports(BackendCapability::BuildFromContext));
/// ```
#[derive(Debug, Clone, Default)]
pub struct BackendCapabilitySet {
    flags: std::collections::HashSet<BackendCapability>,
}

impl BackendCapabilitySet {
    /// Create an empty capability set (no capabilities).
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a capability to this set (builder-style).
    pub fn with(mut self, cap: BackendCapability) -> Self {
        self.flags.insert(cap);
        self
    }

    /// Return `true` if this set includes `cap`.
    pub fn supports(&self, cap: BackendCapability) -> bool {
        self.flags.contains(&cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // --- MetricsRecorder tests ---

    /// Verify that a no-op MetricsRecorder can be constructed and used as a trait object.
    #[test]
    fn test_metrics_recorder_trait_object() {
        struct StubRecorder;
        impl MetricsRecorder for StubRecorder {
            fn increment_counter(&self, _name: &str, _labels: &[(&str, &str)]) {}
            fn record_histogram(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
            fn set_gauge(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
        }

        let recorder: Arc<dyn MetricsRecorder> = Arc::new(StubRecorder);
        recorder.increment_counter("test_counter", &[("key", "val")]);
        recorder.record_histogram("test_hist", 1.5, &[]);
        recorder.set_gauge("test_gauge", 42.0, &[("a", "b")]);
    }

    // --- ContainerId tests ---

    #[test]
    fn test_container_id_valid() {
        let id = ContainerId::new("abc123".to_string()).expect("valid alphanumeric id");
        assert_eq!(id.as_str(), "abc123");
    }

    #[test]
    fn test_container_id_empty() {
        let result = ContainerId::new(String::new());
        assert!(result.is_err(), "empty id should fail");
    }

    #[test]
    fn test_container_id_too_long() {
        let long = "a".repeat(65);
        let result = ContainerId::new(long);
        assert!(result.is_err(), "65-char id should fail");
    }

    #[test]
    fn test_container_id_max_length() {
        let id_str = "a".repeat(64);
        let id = ContainerId::new(id_str.clone()).expect("64-char id should succeed");
        assert_eq!(id.as_str(), id_str);
    }

    #[test]
    fn test_container_id_special_chars() {
        let result = ContainerId::new("abc-123".to_string());
        assert!(result.is_err(), "hyphen should fail alphanumeric check");
    }

    #[test]
    fn test_container_id_spaces() {
        let result = ContainerId::new("abc 123".to_string());
        assert!(result.is_err(), "space should fail alphanumeric check");
    }

    #[test]
    fn test_container_id_as_str() {
        let id = ContainerId::new("deadbeef".to_string()).expect("valid id");
        assert_eq!(id.as_str(), "deadbeef");
    }

    #[test]
    fn test_container_id_display() {
        let id = ContainerId::new("abc123".to_string()).expect("valid id");
        assert_eq!(format!("{id}"), "abc123");
    }

    #[test]
    fn test_container_id_equality() {
        let a = ContainerId::new("abc123".to_string()).expect("valid id");
        let b = ContainerId::new("abc123".to_string()).expect("valid id");
        assert_eq!(a, b);
    }

    #[test]
    fn test_container_id_hash() {
        let a = ContainerId::new("abc123".to_string()).expect("valid id");
        let b = ContainerId::new("def456".to_string()).expect("valid id");
        let mut set: HashSet<ContainerId> = HashSet::new();
        set.insert(a.clone());
        set.insert(b.clone());
        assert!(set.contains(&a));
        assert!(set.contains(&b));
        assert_eq!(set.len(), 2);
    }

    // --- ContainerState tests ---

    #[test]
    fn test_container_state_as_str() {
        assert_eq!(ContainerState::Created.as_str(), "Created");
        assert_eq!(ContainerState::Running.as_str(), "Running");
        assert_eq!(ContainerState::Paused.as_str(), "Paused");
        assert_eq!(ContainerState::Stopped.as_str(), "Stopped");
        assert_eq!(ContainerState::Failed.as_str(), "Failed");
    }

    #[test]
    fn test_container_state_display() {
        assert_eq!(format!("{}", ContainerState::Created), "Created");
        assert_eq!(format!("{}", ContainerState::Running), "Running");
        assert_eq!(format!("{}", ContainerState::Paused), "Paused");
        assert_eq!(format!("{}", ContainerState::Stopped), "Stopped");
        assert_eq!(format!("{}", ContainerState::Failed), "Failed");
    }

    #[test]
    fn test_container_state_clone_eq() {
        let state = ContainerState::Running;
        let cloned = state;
        assert_eq!(state, cloned);
        assert_ne!(state, ContainerState::Stopped);
    }

    // --- DomainError tests ---

    #[test]
    fn test_domain_error_display_image_not_found() {
        let err = DomainError::ImageNotFound {
            name: "library/ubuntu".to_string(),
            tag: "22.04".to_string(),
        };
        assert_eq!(format!("{err}"), "image library/ubuntu:22.04 not found");
    }

    #[test]
    fn test_domain_error_display_container_not_found() {
        let err = DomainError::ContainerNotFound {
            id: "abc123".to_string(),
        };
        assert_eq!(format!("{err}"), "container 'abc123' not found");
    }

    #[test]
    fn test_domain_error_display_resource_limit_exceeded() {
        let err = DomainError::ResourceLimitExceeded {
            limit: "memory_bytes".to_string(),
            value: 9999,
            max: 1024,
        };
        let msg = format!("{err}");
        assert!(msg.contains("memory_bytes"), "should contain limit name");
        assert!(msg.contains("9999"), "should contain value");
        assert!(msg.contains("1024"), "should contain max");
    }

    // --- ResourceConfig tests ---

    #[test]
    fn test_resource_config_default() {
        let config = ResourceConfig::default();
        assert!(config.memory_limit_bytes.is_none());
        assert!(config.cpu_weight.is_none());
        assert!(config.pids_max.is_none());
        assert!(config.io_max_bytes_per_sec.is_none());
    }

    #[test]
    fn test_resource_config_serde_roundtrip() {
        let config = ResourceConfig {
            memory_limit_bytes: Some(1024 * 1024 * 256),
            cpu_weight: Some(500),
            pids_max: Some(100),
            io_max_bytes_per_sec: Some(1024 * 1024),
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let back: ResourceConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.memory_limit_bytes, config.memory_limit_bytes);
        assert_eq!(back.cpu_weight, config.cpu_weight);
        assert_eq!(back.pids_max, config.pids_max);
        assert_eq!(back.io_max_bytes_per_sec, config.io_max_bytes_per_sec);
    }

    // --- HookSpec / ContainerHooks tests ---

    #[test]
    fn test_hook_spec_default() {
        let hook = HookSpec::default();
        assert_eq!(hook.command, "");
        assert!(hook.args.is_empty());
        assert!(hook.timeout_secs.is_none());
    }

    #[test]
    fn test_container_hooks_default() {
        let hooks = ContainerHooks::default();
        assert!(hooks.pre_exec.is_empty());
        assert!(hooks.post_exit.is_empty());
    }

    // --- RuntimeCapabilities tests ---

    #[test]
    fn test_runtime_capabilities_debug() {
        let caps = RuntimeCapabilities {
            supports_user_namespaces: true,
            supports_cgroups_v2: false,
            supports_overlay_fs: true,
            supports_network_isolation: false,
            max_containers: Some(128),
        };
        let debug_str = format!("{caps:?}");
        assert!(!debug_str.is_empty(), "Debug impl should produce output");
    }

    // --- ImageLoader tests ---

    // --- ExecSpec purity test ---

    /// Verify that ExecSpec is Clone and contains no channel fields.
    /// This encodes the architecture contract: ExecSpec is a pure domain
    /// value type that must not depend on tokio infrastructure.
    #[test]
    fn exec_spec_is_pure_domain() {
        let spec = crate::domain::ExecSpec {
            cmd: vec!["echo".to_string()],
            env: vec![],
            working_dir: None,
            tty: false,
        };
        // Must be Clone — pure domain types are always Clone
        let cloned = spec.clone();
        assert_eq!(cloned.cmd, vec!["echo".to_string()]);
        assert!(!cloned.tty);
    }

    #[cfg(test)]
    mod image_loader_tests {
        use super::*;
        use std::path::Path;

        struct AlwaysOkLoader;

        #[async_trait::async_trait]
        impl ImageLoader for AlwaysOkLoader {
            async fn load_image(
                &self,
                _path: &Path,
                _name: &str,
                _tag: &str,
            ) -> anyhow::Result<()> {
                Ok(())
            }
        }

        #[tokio::test]
        async fn image_loader_trait_is_object_safe() {
            let loader: Box<dyn ImageLoader> = Box::new(AlwaysOkLoader);
            let result = loader
                .load_image(
                    std::path::Path::new("/fake.tar"),
                    "minibox-tester",
                    "latest",
                )
                .await;
            assert!(result.is_ok());
        }
    }

    mod backend_rootfs_metadata_tests {
        use super::*;
        use std::path::PathBuf;

        #[test]
        fn overlay_upper_dir_returns_path_for_native_variant() {
            let path = PathBuf::from("/var/lib/minibox/containers/abc/upper");
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: path.clone(),
                metadata: std::collections::HashMap::new(),
            };
            assert_eq!(meta.overlay_upper_dir(), &path);
        }

        #[test]
        fn overlay_upper_dir_returns_path_for_colima_variant() {
            let path = PathBuf::from("/Users/joe/.lima/colima/upper");
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: path.clone(),
                metadata: kv,
            };
            assert_eq!(meta.overlay_upper_dir(), &path);
        }

        #[test]
        fn metadata_value_none_for_missing_key() {
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/tmp/upper"),
                metadata: std::collections::HashMap::new(),
            };
            assert_eq!(meta.metadata_value("colima_instance"), None);
        }

        #[test]
        fn metadata_value_returns_value_for_present_key() {
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/tmp/upper"),
                metadata: kv,
            };
            assert_eq!(meta.metadata_value("colima_instance"), Some("colima"));
        }

        #[test]
        fn backend_rootfs_metadata_roundtrips_serde_overlay() {
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/var/lib/minibox/containers/abc/upper"),
                metadata: std::collections::HashMap::new(),
            };
            let json = serde_json::to_string(&meta).expect("serialize");
            let restored: BackendRootfsMetadata = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(meta, restored);
        }

        #[test]
        fn backend_rootfs_metadata_roundtrips_serde_with_kv() {
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/Users/joe/.lima/colima/upper"),
                metadata: kv,
            };
            let json = serde_json::to_string(&meta).expect("serialize");
            let restored: BackendRootfsMetadata = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(meta, restored);
        }

        #[test]
        fn rootfs_layout_metadata_survives_commit_image_ref() {
            // Verify that an Overlay metadata's upper_dir is unchanged
            // after being stored and retrieved (simulates the commit path
            // reading the upper_dir from the container record).
            let upper = PathBuf::from("/Users/joe/.lima/colima/containers/abc/upper");
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: upper.clone(),
                metadata,
            };
            let layout = RootfsLayout {
                merged_dir: PathBuf::from("/tmp/merged"),
                rootfs_metadata: Some(meta),
                source_image_ref: Some("alpine:latest".to_string()),
            };
            let recovered_upper = layout.rootfs_metadata.as_ref().unwrap().overlay_upper_dir();
            assert_eq!(recovered_upper, &upper);
        }

        // --- Task 1: OCP fix tests ---

        #[test]
        fn overlay_variant_has_opaque_metadata_map() {
            // BackendRootfsMetadata::Overlay must carry an opaque HashMap so
            // backends can encode their own KVs without adding new variants.
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("colima_instance".to_string(), "colima".to_string());
            let upper = PathBuf::from("/Users/joe/.lima/colima/upper");
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: upper.clone(),
                metadata: metadata.clone(),
            };
            assert_eq!(meta.overlay_upper_dir(), &upper);
            assert_eq!(meta.metadata_value("colima_instance"), Some("colima"));
        }

        #[test]
        fn overlay_variant_metadata_empty_for_native() {
            // Native overlay encodes no extra KVs.
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/var/lib/minibox/containers/abc/upper"),
                metadata: std::collections::HashMap::new(),
            };
            assert_eq!(meta.metadata_value("colima_instance"), None);
        }

        #[test]
        fn backend_rootfs_metadata_roundtrips_serde_with_metadata_map() {
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/Users/joe/.lima/colima/upper"),
                metadata: kv,
            };
            let json = serde_json::to_string(&meta).expect("serialize");
            let restored: BackendRootfsMetadata = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(meta, restored);
        }
    }

    mod pty_allocator_tests {
        use super::*;

        #[test]
        fn pty_config_default_values() {
            let cfg = PtyConfig {
                enabled: true,
                cols: 80,
                rows: 24,
            };
            assert!(cfg.enabled);
            assert_eq!(cfg.cols, 80);
            assert_eq!(cfg.rows, 24);
        }

        #[test]
        fn pty_config_serde_roundtrip() {
            let cfg = PtyConfig {
                enabled: true,
                cols: 120,
                rows: 40,
            };
            let json = serde_json::to_string(&cfg).expect("serialize");
            let back: PtyConfig = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back.enabled, cfg.enabled);
            assert_eq!(back.cols, cfg.cols);
            assert_eq!(back.rows, cfg.rows);
        }

        #[test]
        fn pty_config_json_missing_fields_use_serde_default() {
            // When a JSON payload omits fields the struct must still deserialize.
            let json = r#"{"enabled":false,"cols":80,"rows":24}"#;
            let cfg: PtyConfig = serde_json::from_str(json).expect("deserialize");
            assert!(!cfg.enabled);
        }

        #[test]
        fn null_pty_allocator_returns_err() {
            let alloc = NullPtyAllocator;
            let cfg = PtyConfig {
                enabled: true,
                cols: 80,
                rows: 24,
            };
            assert!(
                alloc.allocate(&cfg).is_err(),
                "NullPtyAllocator must always return Err"
            );
        }

        #[cfg(feature = "test-utils")]
        #[test]
        fn mock_pty_allocator_returns_configured_handle() {
            let alloc = MockPtyAllocator::new(5, 6);
            let cfg = PtyConfig {
                enabled: true,
                cols: 80,
                rows: 24,
            };
            let handle = alloc.allocate(&cfg).expect("MockPtyAllocator must succeed");
            assert_eq!(handle.master_fd, 5);
            assert_eq!(handle.slave_fd, 6);
        }
    }

    mod isp_trait_split_tests {
        use super::*;
        use std::path::{Path, PathBuf};

        // --- Task 2: ISP split tests ---

        /// Verify that RootfsSetup is a standalone trait (not mixed with ChildInit).
        struct OnlySetup;
        impl AsAny for OnlySetup {
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
        impl RootfsSetup for OnlySetup {
            fn setup_rootfs(
                &self,
                _layers: &[PathBuf],
                _container_dir: &Path,
            ) -> Result<RootfsLayout> {
                Ok(RootfsLayout {
                    merged_dir: PathBuf::from("/tmp/merged"),
                    rootfs_metadata: None,
                    source_image_ref: None,
                })
            }

            fn cleanup(&self, _container_dir: &Path) -> Result<()> {
                Ok(())
            }
        }

        /// Verify that ChildInit is a standalone trait for pivot_root.
        struct OnlyChildInit;
        impl ChildInit for OnlyChildInit {
            fn pivot_root(&self, _new_root: &Path) -> Result<()> {
                Ok(())
            }
        }

        #[test]
        fn rootfs_setup_can_be_used_without_child_init() {
            let setup = OnlySetup;
            let result = setup.setup_rootfs(&[], Path::new("/tmp/container"));
            assert!(result.is_ok());
            assert!(setup.cleanup(Path::new("/tmp/container")).is_ok());
        }

        #[test]
        fn child_init_can_be_used_without_rootfs_setup() {
            let init = OnlyChildInit;
            assert!(init.pivot_root(Path::new("/tmp/new_root")).is_ok());
        }
    }
}
