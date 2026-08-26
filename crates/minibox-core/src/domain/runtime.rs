//! Container runtime and resource-limiting domain ports.

use anyhow::Result;
use async_trait::async_trait;
use std::any::Any;
use std::os::fd::OwnedFd;
use std::sync::Arc;

use super::{
    BindMount, FilesystemProvider, ImageLoader, ImageRegistry, MetricsRecorder, NetworkProvider,
    RegistryRouter,
};

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
/// - Prevent resource `DoS` attacks (default PID limits)
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
#[allow(clippy::struct_excessive_bools)]
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

    /// Wait for a container process to exit and return its exit code.
    ///
    /// There is no default implementation — each adapter must override this
    /// with platform-specific wait logic (e.g. `waitpid` on Linux, VM-level
    /// wait on krun/smolvm, Docker API on Colima).
    async fn wait_for_exit(&self, _runtime_id: Option<&str>, _pid: u32) -> Result<i32> {
        anyhow::bail!(
            "wait_for_exit: no default implementation — \
             adapter must override with platform-specific wait logic"
        )
    }
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
    /// Runtime-internal container ID, used by adapters that manage their own
    /// process tree (e.g. krun/smolvm). Passed back to `wait_for_exit`.
    /// `None` for native adapters where `waitpid(pid)` suffices.
    pub runtime_id: Option<String>,
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
    pub rootfs: crate::path::InternalPath,
    /// Command to execute (e.g., `"/bin/sh"`).
    pub command: String,
    /// Command arguments (e.g., `["-c", "echo hello"]`).
    pub args: Vec<String>,
    /// Environment variables (e.g., `["PATH=/usr/bin", "HOME=/root"]`).
    pub env: Vec<String>,
    /// Hostname to set inside the container.
    pub hostname: String,
    /// Path to the cgroup directory for this container.
    pub cgroup_path: crate::path::InternalPath,
    /// When `true`, container stdout+stderr are captured to a pipe.
    /// The read end is returned in [`SpawnResult::output_reader`].
    pub capture_output: bool,
    /// Optional host-side lifecycle hooks.
    pub hooks: ContainerHooks,
    /// If true, skip `CLONE_NEWNET` — container shares host network namespace.
    pub skip_network_namespace: bool,
    /// Bind mounts to apply inside the container before `pivot_root`.
    ///
    /// Each `BindMount.host_path` is mounted at `rootfs + BindMount.container_path`
    /// inside the container's new mount namespace, then the container sees it at
    /// `container_path` after `pivot_root`.
    pub mounts: Vec<BindMount>,
    /// If `true`, the container process is granted a full Linux capability set
    /// via `capset(2)` before `execvp`. Required for `DinD`.
    pub privileged: bool,
    /// OCI image reference that produced this container's rootfs
    /// (e.g. `"alpine:latest"`, `"ghcr.io/org/img:v1"`).
    ///
    /// VM-based backends (krun/smolvm) use this to pass `--image` to the
    /// hypervisor instead of re-deriving the ref from the rootfs path.
    /// `None` for Linux-native backends that operate on the extracted rootfs
    /// directly.
    pub image_ref: Option<String>,
}
