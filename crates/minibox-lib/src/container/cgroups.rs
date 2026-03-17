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
    /// Maximum number of PIDs (processes/threads). `None` means unlimited.
    /// Default: 1024 to prevent fork bombs.
    pub pids_max: Option<u64>,
    /// I/O bandwidth limit in bytes/second. `None` means unlimited.
    pub io_max_bytes_per_sec: Option<u64>,
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

/// Default root of the minibox cgroup slice inside the cgroupfs mount point.
const DEFAULT_MINIBOX_CGROUP_ROOT: &str = "/sys/fs/cgroup/minibox";

fn cgroup_root() -> PathBuf {
    if let Ok(root) = std::env::var("MINIBOX_CGROUP_ROOT") {
        PathBuf::from(root)
    } else {
        PathBuf::from(DEFAULT_MINIBOX_CGROUP_ROOT)
    }
}

impl CgroupManager {
    /// Create a new manager for `container_id` with the given resource limits.
    ///
    /// This only builds the struct -- call [`create`](Self::create) to actually
    /// create the directory and write the limits.
    pub fn new(container_id: &str, config: CgroupConfig) -> Self {
        let cgroup_path = cgroup_root().join(container_id);
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

        // Enable controllers on the parent cgroup so child cgroups can use them.
        // Without this, writing pids.max/memory.max/etc. in the child fails with
        // Permission denied because the controllers aren't delegated.
        let root = cgroup_root();
        enable_subtree_controllers(&root)?;

        // Memory limit
        if let Some(mem) = self.config.memory_limit_bytes {
            // SECURITY: Validate minimum memory (kernel minimum is typically 4KB)
            if mem < 4096 {
                anyhow::bail!("memory limit must be >= 4096 bytes, got {}", mem);
            }
            self.write_file("memory.max", &mem.to_string())?;
            debug!("set memory.max={}", mem);
        }

        // CPU weight
        if let Some(cpu) = self.config.cpu_weight {
            // SECURITY: Validate range (kernel range is 1-10000)
            if !(1..=10000).contains(&cpu) {
                anyhow::bail!("cpu_weight must be 1-10000, got {}", cpu);
            }
            self.write_file("cpu.weight", &cpu.to_string())?;
            debug!("set cpu.weight={}", cpu);
        }

        // SECURITY: PID limit to prevent fork bombs
        let pids_limit = self.config.pids_max.unwrap_or(1024);
        self.write_file("pids.max", &pids_limit.to_string())?;
        debug!("set pids.max={}", pids_limit);

        // SECURITY: I/O throttling to prevent disk DoS
        if let Some(io_limit) = self.config.io_max_bytes_per_sec {
            // Format: "major:minor rbps=<bytes> wbps=<bytes>"
            // We'll use 8:0 (sda) as a default - in production, this should be configurable
            let io_max_line = format!("8:0 rbps={} wbps={}", io_limit, io_limit);
            self.write_file("io.max", &io_max_line)?;
            debug!("set io.max={} bytes/sec", io_limit);
        }

        info!("cgroup created at {:?}", self.cgroup_path);
        Ok(())
    }

    /// Add a running process to this cgroup by writing its PID to
    /// `cgroup.procs`.
    pub fn add_process(&self, pid: u32) -> anyhow::Result<()> {
        debug!("adding PID {} to cgroup {:?}", pid, self.cgroup_path);
        let path = self.cgroup_path.join("cgroup.procs");
        fs::write(&path, format!("{}\n", pid)).map_err(|source| CgroupError::AddProcessFailed {
            pid,
            path: path.display().to_string(),
            source,
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
            warn!(
                "cgroup {:?} already gone, skipping cleanup",
                self.cgroup_path
            );
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

/// Enable cgroup controllers (`pids`, `memory`, `cpu`, `io`) in
/// `cgroup.subtree_control` of the given directory so that child cgroups
/// can use the corresponding resource-limit files.
///
/// Idempotent: controllers that are already enabled are silently skipped.
fn enable_subtree_controllers(dir: &std::path::Path) -> anyhow::Result<()> {
    let subtree_control = dir.join("cgroup.subtree_control");
    let current = fs::read_to_string(&subtree_control).unwrap_or_default();

    for controller in &["pids", "memory", "cpu", "io"] {
        if !current.split_whitespace().any(|c| c == *controller) {
            let value = format!("+{controller}");
            debug!("enabling controller {value} in {:?}", subtree_control);
            if let Err(e) = fs::write(&subtree_control, &value) {
                // Non-fatal: the controller may not be available on this host.
                warn!(
                    "could not enable {controller} in {}: {e}",
                    subtree_control.display()
                );
            }
        }
    }
    Ok(())
}

/// Build the cgroup path for a container without constructing a full manager.
///
/// Useful when only the *path* is needed (e.g., stored in [`Container`]).
pub fn cgroup_path_for(container_id: &str) -> PathBuf {
    cgroup_root().join(container_id)
}
