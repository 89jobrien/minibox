//! Linux namespace container runtime adapter implementing the ContainerRuntime trait.
//!
//! This adapter wraps the existing container process spawning logic from
//! [`crate::container::process`] to implement the domain's
//! [`ContainerRuntime`] trait.

use crate::container::namespace::NamespaceConfig;
use crate::container::process::{ContainerConfig, spawn_container_process};
use anyhow::Result;
use async_trait::async_trait;
use minibox_core::adapt;
use minibox_core::domain::{
    ContainerRuntime, ContainerSpawnConfig, RuntimeCapabilities, SpawnResult,
};
use tracing::debug;

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
/// use linuxbox::adapters::LinuxNamespaceRuntime;
/// use linuxbox::domain::{ContainerRuntime, ContainerSpawnConfig};
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
#[derive(Debug, Clone, Copy)]
pub struct LinuxNamespaceRuntime;

impl LinuxNamespaceRuntime {
    /// Create a new Linux namespace container runtime adapter.
    ///
    /// This is a zero-sized type, so construction is trivial.
    pub fn new() -> Self {
        Self
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

    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        debug!(
            "spawning container process: command={}, rootfs={:?}",
            config.command, config.rootfs
        );

        let capture_output = config.capture_output;

        // Convert domain ContainerSpawnConfig to infrastructure ContainerConfig
        let container_config = ContainerConfig {
            rootfs: config.rootfs.clone(),
            command: config.command.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
            namespace_config: NamespaceConfig::all(), // All namespaces enabled
            cgroup_path: config.cgroup_path.clone(),
            hostname: config.hostname.clone(),
            capture_output,
            pre_exec_hooks: config.hooks.pre_exec.clone(),
        };

        // IMPORTANT: spawn_container_process uses blocking syscalls (clone/fork)
        // We must run it in a blocking thread to avoid blocking the async runtime
        let spawn_result =
            tokio::task::spawn_blocking(move || spawn_container_process(container_config))
                .await??; // First ? for join error, second ? for spawn error

        debug!("container process spawned with PID {}", spawn_result.pid);
        Ok(spawn_result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_creation() {
        let runtime = LinuxNamespaceRuntime::new();
        let _ = runtime;
    }

    #[test]
    fn test_runtime_default() {
        let runtime = LinuxNamespaceRuntime;
        let _ = runtime;
    }

    // Note: Actual spawn tests require Linux with root privileges
    // and a properly setup rootfs, so they belong in integration tests
}
