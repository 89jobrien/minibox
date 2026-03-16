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
//! │                 │    │                     │
//! │  ┌──────────┐   │    │  ┌──────────────┐   │
//! │  │Business  │   │    │  │ DockerHub    │   │
//! │  │Logic     │───┼────┼─►│ Registry     │   │
//! │  │          │   │    │  └──────────────┘   │
//! │  └──────────┘   │    │                     │
//! │                 │    │  ┌──────────────┐   │
//! │  ┌──────────┐   │    │  │ Overlay      │   │
//! │  │Traits    │◄──┼────┼──│ Filesystem   │   │
//! │  │(Ports)   │   │    │  └──────────────┘   │
//! │  └──────────┘   │    │                     │
//! │                 │    │  ┌──────────────┐   │
//! │  ┌──────────┐   │    │  │ Cgroup V2    │   │
//! │  │Domain    │   │    │  │ Limiter      │   │
//! │  │Types     │   │    │  └──────────────┘   │
//! └─────────────────┘    └─────────────────────┘
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
mod networking;
mod extensions;

// Re-exports for public API
pub use networking::*;
pub use extensions::*;

use anyhow::Result;
use async_trait::async_trait;
use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Dyn type aliases
// ---------------------------------------------------------------------------

/// Type alias for a shared, dynamic [`ImageRegistry`] implementation.
pub type DynImageRegistry = Arc<dyn ImageRegistry>;
/// Type alias for a shared, dynamic [`FilesystemProvider`] implementation.
pub type DynFilesystemProvider = Arc<dyn FilesystemProvider>;
/// Type alias for a shared, dynamic [`ResourceLimiter`] implementation.
pub type DynResourceLimiter = Arc<dyn ResourceLimiter>;
/// Type alias for a shared, dynamic [`ContainerRuntime`] implementation.
pub type DynContainerRuntime = Arc<dyn ContainerRuntime>;

// ---------------------------------------------------------------------------
// Downcasting support for testing
// ---------------------------------------------------------------------------

/// Trait to enable downcasting trait objects back to concrete types.
///
/// This allows tests to access adapter-specific methods.
pub trait AsAny: Send + Sync {
    fn as_any(&self) -> &dyn Any;
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
/// use minibox_lib::domain::ImageRegistry;
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
    async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata>;

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
/// Implementations might include overlay filesystem, bind mounts, or
/// other copy-on-write filesystems like ZFS or Btrfs.
///
/// # Security
///
/// Implementations MUST:
/// - Validate all paths to prevent traversal attacks
/// - Mount filesystems with appropriate security flags (nosuid, nodev)
/// - Properly clean up mounts to avoid resource leaks
pub trait FilesystemProvider: AsAny + Send + Sync {
    /// Setup the container rootfs and return the merged directory path.
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
    /// Path to the merged/mounted rootfs that the container will use.
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
    fn setup_rootfs(
        &self,
        image_layers: &[PathBuf],
        container_dir: &Path,
    ) -> Result<PathBuf>;

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
// Container Runtime Port
// ---------------------------------------------------------------------------

/// Abstraction for spawning container processes with isolation.
///
/// This trait defines the contract for container runtime implementations.
/// Implementations might include Linux namespaces, Podman, or other
/// containerization technologies.
#[async_trait]
pub trait ContainerRuntime: AsAny + Send + Sync {
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
    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<u32>;
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
    /// Container process has exited.
    Stopped,
    /// Container failed to start or crashed.
    Failed,
}

impl ContainerState {
    /// Convert state to string representation for protocol/display.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "Created",
            Self::Running => "Running",
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
