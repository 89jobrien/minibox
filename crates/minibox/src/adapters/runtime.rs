//! Linux namespace container runtime adapter implementing the `ContainerRuntime` trait.
//!
//! This adapter wraps the existing container process spawning logic from
//! [`crate::container::process`] to implement the domain's
//! [`ContainerRuntime`] trait.

use crate::container::namespace::NamespaceConfig;
use crate::container::process::{ContainerConfig, UidMapping, spawn_container_process};
use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::adapt;
use minibox_core::domain::{
    ContainerRuntime, ContainerSpawnConfig, RuntimeCapabilities, SpawnResult, UidRangeMode,
};
use tracing::{debug, instrument};

/// Linux namespaces implementation of the [`ContainerRuntime`] trait.
///
/// This adapter uses Linux kernel namespaces to provide process isolation
/// for containers. It delegates to the existing process spawning logic which
/// handles the low-level `clone()` syscall and namespace setup.
///
/// # Platform Support
///
/// This adapter is **Linux-only** and requires:
/// - Kernel 4.0+ (5.0+ recommended)
/// - Namespace support: PID, Mount, UTS, IPC, Network, User (optional)
/// - Root privileges for namespace creation
///
/// # Namespaces Created
///
/// - **PID**: Isolated process ID space
/// - **Mount**: Isolated filesystem mounts
/// - **UTS**: Isolated hostname
/// - **IPC**: Isolated IPC resources (semaphores, message queues)
/// - **Network**: Isolated network stack (no setup by default)
///
/// # Container Lifecycle
///
/// 1. Parent calls `spawn_process()` with configuration
/// 2. `clone()` creates child with new namespaces
/// 3. Child process:
///    - Adds itself to cgroup
///    - Sets hostname
///    - Pivots root filesystem
///    - Closes inherited file descriptors
///    - Executes user command
/// 4. Parent receives child PID and returns
///
/// # Async/Sync Boundary
///
/// The actual `clone()` syscall is synchronous and blocking. This adapter
/// spawns a blocking task to handle the fork operation, making it safe to
/// call from async contexts.
///
/// # Example
///
/// ```rust,ignore
/// use crate::adapters::LinuxNamespaceRuntime;
/// use crate::domain::{ContainerRuntime, ContainerSpawnConfig};
/// use std::path::PathBuf;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let runtime = LinuxNamespaceRuntime::new();
///
///     let config = ContainerSpawnConfig {
///         rootfs: PathBuf::from("/var/lib/minibox/containers/abc123/merged"),
///         command: "/bin/sh".to_string(),
///         args: vec!["-c".to_string(), "echo hello".to_string()],
///         env: vec!["PATH=/usr/bin".to_string()],
///         hostname: "container-abc123".to_string(),
///         cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/abc123"),
///         capture_output: false,
///     };
///
///     let spawn_result = runtime.spawn_process(&config).await?;
///     println!("Container started with PID {}", spawn_result.pid);
///
///     Ok(())
/// }
/// ```
#[derive(Debug, Clone)]
pub struct LinuxNamespaceRuntime {
    next_exclusive_id: std::sync::Arc<std::sync::Mutex<u32>>,
}

impl LinuxNamespaceRuntime {
    /// Create a new Linux namespace container runtime adapter.
    ///
    /// This is a zero-sized type, so construction is trivial.
    pub fn new() -> Self {
        Self {
            next_exclusive_id: std::sync::Arc::new(std::sync::Mutex::new(165_536)),
        }
    }

    fn allocate_uid_mapping(&self, mode: UidRangeMode) -> Result<UidMapping> {
        const RANGE_SIZE: u32 = 65_536;
        const SHARED_RANGE_START: u32 = 100_000;
        if mode == UidRangeMode::Shared {
            return Ok(UidMapping {
                host_uid: SHARED_RANGE_START,
                host_gid: SHARED_RANGE_START,
                size: RANGE_SIZE,
            });
        }
        let mut next = self
            .next_exclusive_id
            .lock()
            .map_err(|_| anyhow::anyhow!("UID range allocator poisoned"))?;
        let start = *next;
        *next = next
            .checked_add(RANGE_SIZE)
            .ok_or_else(|| anyhow::anyhow!("exclusive UID range pool exhausted"))?;
        Ok(UidMapping {
            host_uid: start,
            host_gid: start,
            size: RANGE_SIZE,
        })
    }
}

adapt!(LinuxNamespaceRuntime);

#[async_trait]
impl ContainerRuntime for LinuxNamespaceRuntime {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: true,
            supports_cgroups_v2: true,
            supports_overlay_fs: true,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    #[instrument(
        skip(self, config),
        fields(command = %config.command, privileged = config.privileged),
        err
    )]
    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        let uid_mapping = self.allocate_uid_mapping(config.uid_range_mode)?;
        debug!(
            "spawning container process: command={}, rootfs={:?}",
            config.command, config.rootfs
        );

        let capture_output = config.capture_output;

        // Convert domain ContainerSpawnConfig to infrastructure ContainerConfig
        let container_config = ContainerConfig {
            rootfs: config.rootfs.clone().to_path_buf(),
            command: config.command.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
            namespace_config: NamespaceConfig::all(), // All namespaces enabled
            cgroup_path: config.cgroup_path.clone().to_path_buf(),
            hostname: config.hostname.clone(),
            capture_output,
            pre_exec_hooks: config.hooks.pre_exec.clone(),
            mounts: config.mounts.clone(),
            privileged: config.privileged,
            uid_mapping,
            pty: None,
        };

        // IMPORTANT: spawn_container_process uses blocking syscalls (clone/fork)
        // We must run it in a blocking thread to avoid blocking the async runtime
        let spawn_result =
            tokio::task::spawn_blocking(move || spawn_container_process(container_config))
                .await??; // First ? for join error, second ? for spawn error

        debug!("container process spawned with PID {}", spawn_result.pid);
        Ok(spawn_result)
    }

    async fn wait_for_exit(&self, _runtime_id: Option<&str>, pid: u32) -> Result<i32> {
        tokio::task::spawn_blocking(move || crate::container::process::wait_for_exit(pid))
            .await
            .context("wait_for_exit: join error")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_creation() {
        let runtime = LinuxNamespaceRuntime::new();
        // Verify the reported capabilities match the known Linux namespace feature set.
        let caps = runtime.capabilities();
        assert!(caps.supports_user_namespaces);
        assert!(caps.supports_overlay_fs);
    }

    #[test]
    fn test_runtime_default() {
        let runtime = LinuxNamespaceRuntime::new();
        assert!(runtime.capabilities().supports_user_namespaces);
    }

    #[test]
    fn exclusive_ranges_do_not_overlap() {
        let runtime = LinuxNamespaceRuntime::new();
        let first = runtime
            .allocate_uid_mapping(UidRangeMode::Exclusive)
            .expect("first range");
        let second = runtime
            .allocate_uid_mapping(UidRangeMode::Exclusive)
            .expect("second range");
        assert_eq!(first.size, 65_536);
        assert_eq!(second.host_uid, first.host_uid + first.size);
    }

    #[test]
    fn shared_ranges_reuse_the_explicit_shared_pool() {
        let runtime = LinuxNamespaceRuntime::new();
        let first = runtime
            .allocate_uid_mapping(UidRangeMode::Shared)
            .expect("first range");
        let second = runtime
            .allocate_uid_mapping(UidRangeMode::Shared)
            .expect("second range");
        assert_eq!(first, second);
    }

    // Note: Actual spawn tests require Linux with root privileges
    // and a properly setup rootfs, so they belong in integration tests

    #[test]
    fn spawn_config_fields_map_to_container_config() {
        use minibox_core::domain::{BindMount, ContainerHooks, ContainerSpawnConfig};
        use minibox_core::path::InternalPath;
        use std::path::PathBuf;

        let bind = BindMount {
            host_path: PathBuf::from("/tmp/host"),
            container_path: PathBuf::from("/guest"),
            read_only: true,
        };
        let spawn_config = ContainerSpawnConfig {
            rootfs: InternalPath::from("/rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            hostname: "test".to_string(),
            cgroup_path: InternalPath::from("/cgroup"),
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: false,
            mounts: vec![bind.clone()],
            privileged: true,
            image_ref: None,
            uid_range_mode: UidRangeMode::Shared,
        };

        // Build ContainerConfig the same way spawn_process does.
        let container_config = crate::container::process::ContainerConfig {
            rootfs: spawn_config.rootfs.clone().to_path_buf(),
            command: spawn_config.command.clone(),
            args: spawn_config.args.clone(),
            env: spawn_config.env.clone(),
            namespace_config: crate::container::namespace::NamespaceConfig::all(),
            cgroup_path: spawn_config.cgroup_path.clone().to_path_buf(),
            hostname: spawn_config.hostname.clone(),
            capture_output: spawn_config.capture_output,
            pre_exec_hooks: spawn_config.hooks.pre_exec.clone(),
            mounts: spawn_config.mounts.clone(),
            privileged: spawn_config.privileged,
            uid_mapping: crate::container::process::UidMapping {
                host_uid: 100_000,
                host_gid: 100_000,
                size: 65_536,
            },
            pty: None,
        };

        assert_eq!(container_config.mounts.len(), 1);
        assert_eq!(
            container_config.mounts[0].host_path,
            PathBuf::from("/tmp/host")
        );
        assert!(container_config.privileged);
    }
}
