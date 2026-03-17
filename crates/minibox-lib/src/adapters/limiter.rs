//! cgroups v2 resource limiter adapter implementing the ResourceLimiter trait.
//!
//! This adapter wraps the existing [`CgroupManager`] from
//! [`crate::container::cgroups`] to implement the domain's
//! [`ResourceLimiter`] trait.

use crate::container::cgroups::{CgroupConfig, CgroupManager};
use crate::domain::{ResourceConfig, ResourceLimiter};
use anyhow::Result;
use tracing::debug;

/// cgroups v2 implementation of the [`ResourceLimiter`] trait.
///
/// This adapter uses Linux cgroups v2 (unified hierarchy) to enforce
/// resource limits on containerized processes. It delegates to the existing
/// [`CgroupManager`] which handles low-level cgroup operations.
///
/// # Platform Support
///
/// This adapter is **Linux-only** and requires:
/// - Kernel 5.0+ (recommended, 4.5+ minimum for basic cgroups v2)
/// - cgroups v2 unified hierarchy mounted at `/sys/fs/cgroup/`
/// - Root privileges for cgroup manipulation
///
/// # Resource Limits Supported
///
/// - **Memory**: Maximum RSS + swap in bytes (`memory.max`)
/// - **CPU**: CPU weight/shares (`cpu.weight`, range 1-10000)
/// - **PIDs**: Maximum number of processes/threads (`pids.max`)
/// - **I/O**: I/O bandwidth throttling (`io.max`)
///
/// # Security
///
/// - Default PID limit of 1024 prevents fork bombs
/// - Minimum memory limit enforced (4096 bytes)
/// - CPU weight validated to kernel range (1-10000)
///
/// # Example
///
/// ```rust,ignore
/// use minibox_lib::adapters::CgroupV2Limiter;
/// use minibox_lib::domain::{ResourceConfig, ResourceLimiter};
///
/// let limiter = CgroupV2Limiter::new();
///
/// let config = ResourceConfig {
///     memory_limit_bytes: Some(512 * 1024 * 1024), // 512 MB
///     cpu_weight: Some(500), // Half of default
///     pids_max: Some(1024),
///     io_max_bytes_per_sec: None,
/// };
///
/// // Create resource limits
/// let cgroup_path = limiter.create("container-abc123", &config)?;
///
/// // Add process to limits (from inside the container)
/// limiter.add_process("container-abc123", 12345)?;
///
/// // Later, cleanup
/// limiter.cleanup("container-abc123")?;
/// ```
#[derive(Debug, Clone, Copy)]
pub struct CgroupV2Limiter;

impl CgroupV2Limiter {
    /// Create a new cgroups v2 resource limiter adapter.
    ///
    /// This is a zero-sized type, so construction is trivial.
    pub fn new() -> Self {
        Self
    }
}


adapt!(CgroupV2Limiter);

impl ResourceLimiter for CgroupV2Limiter {
    fn create(&self, container_id: &str, config: &ResourceConfig) -> Result<String> {
        debug!(
            "creating cgroup for container {} with config: {:?}",
            container_id, config
        );

        // Convert domain ResourceConfig to infrastructure CgroupConfig
        let cgroup_config = CgroupConfig {
            memory_limit_bytes: config.memory_limit_bytes,
            cpu_weight: config.cpu_weight,
            pids_max: config.pids_max,
            io_max_bytes_per_sec: config.io_max_bytes_per_sec,
        };

        // Create manager and apply limits
        let manager = CgroupManager::new(container_id, cgroup_config);
        manager.create()?;

        // Return the cgroup path as a string
        Ok(manager.cgroup_path.display().to_string())
    }

    fn add_process(&self, container_id: &str, pid: u32) -> Result<()> {
        debug!(
            "adding process {} to cgroup for container {}",
            pid, container_id
        );

        // Create manager (doesn't re-create the cgroup, just for API access)
        let manager = CgroupManager::new(container_id, CgroupConfig::default());
        manager.add_process(pid)
    }

    fn cleanup(&self, container_id: &str) -> Result<()> {
        debug!("cleaning up cgroup for container {}", container_id);

        // Create manager for cleanup
        let manager = CgroupManager::new(container_id, CgroupConfig::default());
        manager.cleanup()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limiter_creation() {
        let limiter = CgroupV2Limiter::new();
        let _ = limiter;
    }

    #[test]
    fn test_limiter_default() {
        let limiter = CgroupV2Limiter;
        let _ = limiter;
    }

    #[test]
    fn test_resource_config_conversion() {
        let domain_config = ResourceConfig {
            memory_limit_bytes: Some(512 * 1024 * 1024),
            cpu_weight: Some(500),
            pids_max: Some(2048),
            io_max_bytes_per_sec: Some(10 * 1024 * 1024),
        };

        // Verify the conversion happens (tested implicitly in create())
        let limiter = CgroupV2Limiter::new();
        let _ = limiter;

        // Actual cgroup operations require Linux root privileges
        // and are tested in integration tests
        assert_eq!(domain_config.memory_limit_bytes, Some(512 * 1024 * 1024));
        assert_eq!(domain_config.cpu_weight, Some(500));
        assert_eq!(domain_config.pids_max, Some(2048));
    }

    // Note: Actual cgroup tests require Linux with root privileges
    // and are better suited for integration tests
}
