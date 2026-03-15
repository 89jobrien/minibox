//! cgroups v2 resource management for containers.
//!
//! Each container gets its own cgroup under `/sys/fs/cgroup/minibox/{id}/`.
//! We only write to the unified v2 hierarchy - cgroupv1 is not supported.

use crate::error::CgroupError;
use anyhow::Context;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// Resource limits to apply to a container's cgroup.
#[derive(Debug, Clone, Default)]
pub struct CgroupConfig {
    /// Maximum RSS + swap in bytes. `None` means unlimited.
    pub memory_limit_bytes: Option<u64>,
    /// CPU weight in the range 1-10000 (default kernel value is 100).
    /// `None` leaves the default.
    pub cpu_weight: Option<u64>,
}

/// Manages a single container cgroup under the minibox slice.
///
/// The cgroup path is `/sys/fs/cgroup/minibox/{container_id}/`.
#[derive(Debug)]
pub struct CgroupManager {
    /// Absolute path to this container's cgroup directory.
    pub cgroup_path: PathBuf,
    config: CgroupConfig,
}

/// Root of the minibox cgroup slice inside the cgroupfs mount point.
const MINIBOX_CGROUP_ROOT: &str = "/sys/fs/cgroup/minibox";

impl CgroupManager {
    /// Create a new manager for `container_id` with the given resource limits.
    ///
    /// This only builds the struct -- call [`create`](Self::create) to actually
    /// create the directory and write the limits.
    pub fn new(container_id: &str, config: CgroupConfig) -> Self {
        let cgroup_path = PathBuf::from(MINIBOX_CGROUP_ROOT).join(container_id);
        Self {
            cgroup_path,
            config,
        }
    }

    /// Create the cgroup directory and apply the configured resource limits.
    ///
    /// Idempotent: if the directory already exists the limits are (re-)written.
    pub fn create(&self) -> anyhow::Result<()> {
        debug!("creating cgroup at {:?}", self.cgroup_path);

        fs::create_dir_all(&self.cgroup_path).map_err(|source| CgroupError::CreateFailed {
            path: self.cgroup_path.display().to_string(),
            source,
        })?;

        if let Some(mem) = self.config.memory_limit_bytes {
            self.write_file("memory.max", &mem.to_string())?;
            debug!("set memory.max={}", mem);
        }

        if let Some(cpu) = self.config.cpu_weight {
            self.write_file("cpu.weight", &cpu.to_string())?;
            debug!("set cpu.weight={}", cpu);
        }

        info!("cgroup created at {:?}", self.cgroup_path);
        Ok(())
    }

    /// Add a running process to this cgroup by writing its PID to
    /// `cgroup.procs`.
    pub fn add_process(&self, pid: u32) -> anyhow::Result<()> {
        debug!("adding PID {} to cgroup {:?}", pid, self.cgroup_path);
        let path = self.cgroup_path.join("cgroup.procs");
        fs::write(&path, format!("{}\n", pid)).map_err(|source| {
            CgroupError::AddProcessFailed {
                pid,
                path: path.display().to_string(),
                source,
            }
        })?;
        info!("added PID {} to cgroup", pid);
        Ok(())
    }

    /// Remove the cgroup directory.
    ///
    /// All processes must have exited (or been migrated) before this is called;
    /// the kernel will refuse to remove a cgroup that still contains tasks.
    pub fn cleanup(&self) -> anyhow::Result<()> {
        debug!("removing cgroup {:?}", self.cgroup_path);
        if !self.cgroup_path.exists() {
            warn!("cgroup {:?} already gone, skipping cleanup", self.cgroup_path);
            return Ok(());
        }
        fs::remove_dir(&self.cgroup_path).map_err(|source| CgroupError::CleanupFailed {
            path: self.cgroup_path.display().to_string(),
            source,
        })?;
        info!("cgroup removed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn write_file(&self, filename: &str, value: &str) -> anyhow::Result<()> {
        let path = self.cgroup_path.join(filename);
        fs::write(&path, value)
            .map_err(|source| CgroupError::WriteFailed {
                path: path.display().to_string(),
                source,
            })
            .with_context(|| format!("writing cgroup file {filename}"))?;
        Ok(())
    }
}

/// Build the cgroup path for a container without constructing a full manager.
///
/// Useful when only the *path* is needed (e.g., stored in [`Container`]).
pub fn cgroup_path_for(container_id: &str) -> PathBuf {
    PathBuf::from(MINIBOX_CGROUP_ROOT).join(container_id)
}
