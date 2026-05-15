//! cgroups v2 resource management for containers.
//!
//! Each container gets its own cgroup under the minibox slice, typically at
//! `/sys/fs/cgroup/minibox.slice/miniboxd.service/{id}/` when running under
//! systemd, or at `$MINIBOX_CGROUP_ROOT/{id}/` when overridden via the
//! environment variable. Only the unified cgroup v2 hierarchy is supported;
//! cgroup v1 is not.
//!
//! # cgroup v2 "no internal process" rule
//!
//! A cgroup cannot simultaneously hold processes **and** child cgroups. Tests
//! that need to write to cgroup limit files must therefore run in a dedicated
//! leaf cgroup (e.g. via `cargo xtask run-cgroup-tests`) rather than directly
//! in the service's own cgroup.

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
/// The cgroup path is `$MINIBOX_CGROUP_ROOT/{container_id}/`, which defaults
/// to `/sys/fs/cgroup/minibox/{container_id}/` when the environment variable
/// is not set. Under a standard systemd deployment the variable is set to
/// `/sys/fs/cgroup/minibox.slice/miniboxd.service`.
#[derive(Debug)]
pub struct CgroupManager {
    /// Container ID used to derive the cgroup path and for log fields.
    id: String,
    /// Absolute path to this container's cgroup directory.
    cgroup_path: PathBuf,
    config: CgroupConfig,
}

/// Default root of the minibox cgroup slice inside the cgroupfs mount point.
const DEFAULT_MINIBOX_CGROUP_ROOT: &str = "/sys/fs/cgroup/minibox";

/// Return the cgroup root directory.
///
/// Reads `MINIBOX_CGROUP_ROOT` from the environment, falling back to
/// [`DEFAULT_MINIBOX_CGROUP_ROOT`] when the variable is absent.
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
            id: container_id.to_string(),
            cgroup_path,
            config,
        }
    }

    /// Create the cgroup directory and apply the configured resource limits.
    ///
    /// Idempotent: if the directory already exists the limits are (re-)written.
    pub fn create(&self) -> anyhow::Result<()> {
        debug!(cgroup_path = %self.cgroup_path.display(), "cgroup: creating directory");

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
            debug!(memory_max = mem, "cgroup: set memory.max");
        }

        // CPU weight
        if let Some(cpu) = self.config.cpu_weight {
            // SECURITY: Validate range (kernel range is 1-10000)
            if !(1..=10000).contains(&cpu) {
                anyhow::bail!("cpu_weight must be 1-10000, got {}", cpu);
            }
            self.write_file("cpu.weight", &cpu.to_string())?;
            debug!(cpu_weight = cpu, "cgroup: set cpu.weight");
        }

        // SECURITY: PID limit to prevent fork bombs
        let pids_limit = self.config.pids_max.unwrap_or(1024);
        self.write_file("pids.max", &pids_limit.to_string())?;
        debug!(pids_max = pids_limit, "cgroup: set pids.max");

        // SECURITY: I/O throttling to prevent disk DoS
        if let Some(io_limit) = self.config.io_max_bytes_per_sec {
            // Format: "major:minor rbps=<bytes> wbps=<bytes>"
            // Detect the first available block device dynamically so this works
            // on VMs using virtio (vda/253:0) as well as bare-metal (sda/8:0).
            match find_first_block_device() {
                Some(dev) => {
                    let io_max_line = format!("{} rbps={} wbps={}", dev, io_limit, io_limit);
                    self.write_file("io.max", &io_max_line)?;
                    debug!(io_max_bytes_per_sec = io_limit, device = %dev, "cgroup: set io.max");
                }
                None => {
                    warn!("cgroup: no block device found in /sys/block, skipping io.max");
                }
            }
        }

        info!(cgroup_path = %self.cgroup_path.display(), "cgroup: created");
        Ok(())
    }

    /// Add a running process to this cgroup by writing its PID to
    /// `cgroup.procs`.
    pub fn add_process(&self, pid: u32) -> anyhow::Result<()> {
        if pid == 0 {
            // PID 0 is silently accepted by some kernel versions but is never
            // valid as an explicit process ID. The child uses the write-"0"
            // convention in add_self_to_cgroup instead.
            anyhow::bail!("PID 0 is not a valid process ID");
        }
        debug!(pid = pid, cgroup_path = %self.cgroup_path.display(), "cgroup: adding process");
        let path = self.cgroup_path.join("cgroup.procs");
        fs::write(&path, format!("{}\n", pid)).map_err(|source| CgroupError::AddProcessFailed {
            pid,
            path: path.display().to_string(),
            source,
        })?;
        info!(pid = pid, "cgroup: process added");
        Ok(())
    }

    /// Remove the cgroup directory.
    ///
    /// All processes must have exited (or been migrated) before this is called;
    /// the kernel will refuse to remove a cgroup that still contains tasks.
    pub fn cleanup(&self) -> anyhow::Result<()> {
        debug!(cgroup_path = %self.cgroup_path.display(), "cgroup: removing directory");
        if !self.cgroup_path.exists() {
            warn!(
                cgroup_path = %self.cgroup_path.display(),
                "cgroup: directory already gone, skipping cleanup"
            );
            return Ok(());
        }
        fs::remove_dir(&self.cgroup_path).map_err(|source| CgroupError::CleanupFailed {
            path: self.cgroup_path.display().to_string(),
            source,
        })?;
        info!(cgroup_path = %self.cgroup_path.display(), "cgroup: removed");
        Ok(())
    }

    /// Returns the cgroup path for this container.
    pub fn cgroup_path(&self) -> &std::path::Path {
        &self.cgroup_path
    }

    /// Freeze all processes in this container's cgroup.
    ///
    /// Writes `"1"` to `cgroup.freeze`. Requires cgroups v2.
    pub async fn pause(&self) -> anyhow::Result<()> {
        let freeze_path = self.cgroup_path.join("cgroup.freeze");
        tokio::fs::write(&freeze_path, "1\n")
            .await
            .with_context(|| format!("cgroup: write 1 to {}", freeze_path.display()))?;
        info!(container_id = %self.id, "cgroup: container paused");
        Ok(())
    }

    /// Thaw all processes in this container's cgroup.
    ///
    /// Writes `"0"` to `cgroup.freeze`. Requires cgroups v2.
    pub async fn resume(&self) -> anyhow::Result<()> {
        let freeze_path = self.cgroup_path.join("cgroup.freeze");
        tokio::fs::write(&freeze_path, "0\n")
            .await
            .with_context(|| format!("cgroup: write 0 to {}", freeze_path.display()))?;
        info!(container_id = %self.id, "cgroup: container resumed");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Write `value` to `{cgroup_path}/{filename}`.
    ///
    /// Returns a [`CgroupError::WriteFailed`] if the write fails, which
    /// typically indicates the controller is not enabled or the kernel rejected
    /// the value.
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

/// Return the `MAJOR:MINOR` string for the first block device found under
/// `/sys/block/`, or `None` if no block devices are present.
///
/// Used to build a valid `io.max` entry; the exact device does not matter
/// for resource-limit purposes — the kernel applies the limit to every
/// device the cgroup's processes access.
fn find_first_block_device() -> Option<String> {
    let sys_block = std::path::Path::new("/sys/block");
    let entries = fs::read_dir(sys_block).ok()?;
    for entry in entries.flatten() {
        let dev_path = entry.path().join("dev");
        if let Ok(dev) = fs::read_to_string(&dev_path) {
            let dev = dev.trim();
            if !dev.is_empty() {
                return Some(dev.to_string());
            }
        }
    }
    None
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
            debug!(controller = controller, subtree_control = %subtree_control.display(), "cgroup: enabling controller");
            if let Err(e) = fs::write(&subtree_control, &value) {
                // Non-fatal: the controller may not be available on this host.
                warn!(
                    controller = controller,
                    subtree_control = %subtree_control.display(),
                    error = %e,
                    "cgroup: could not enable controller"
                );
            }
        }
    }
    Ok(())
}

/// Build the cgroup path for a container without constructing a full manager.
///
/// Useful when only the path is needed (e.g., to pass to `ContainerConfig`
/// without creating a [`CgroupManager`]). Applies the same `MINIBOX_CGROUP_ROOT`
/// override logic as [`CgroupManager::new`].
pub fn cgroup_path_for(container_id: &str) -> PathBuf {
    cgroup_root().join(container_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn test_cgroup_pause_resume_methods_exist() {
        // Verify the methods compile. Real behavior requires root + cgroup mount.
        let mgr = CgroupManager::new("test-pause-id", CgroupConfig::default());
        let _ = mgr.cgroup_path();
        // pause/resume are async; confirm they exist by taking a reference
        let _pause = CgroupManager::pause;
        let _resume = CgroupManager::resume;
    }
}
