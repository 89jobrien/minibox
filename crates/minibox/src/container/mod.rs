//! Container struct, state machine, and lifecycle management.
//!
//! A [`Container`] is the central object managed by the daemon. It tracks
//! identity, runtime state, and the paths needed to clean up after exit.

pub mod cgroups;
pub mod filesystem;
pub mod namespace;
pub mod process;

use crate::container::cgroups::{CgroupConfig, CgroupManager, cgroup_path_for};
use crate::container::filesystem::{cleanup_mounts, setup_overlay};
use crate::container::namespace::NamespaceConfig;
use crate::container::process::{ContainerConfig, spawn_container_process, wait_for_exit};
use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Lifecycle state of a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerState {
    /// Created but not yet started.
    Created,
    /// The container process is running.
    Running,
    /// The container process has exited.
    Stopped,
    /// Resources have been cleaned up; the container record can be discarded.
    Removed,
    /// Container was running in a previous daemon session but its PID is gone.
    Orphaned,
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerState::Created => write!(f, "created"),
            ContainerState::Running => write!(f, "running"),
            ContainerState::Stopped => write!(f, "stopped"),
            ContainerState::Removed => write!(f, "removed"),
            ContainerState::Orphaned => write!(f, "orphaned"),
        }
    }
}

// ---------------------------------------------------------------------------
// Container
// ---------------------------------------------------------------------------

/// A container managed by the Minibox runtime.
#[derive(Debug)]
pub struct Container {
    /// Short UUID used as the container's identifier.
    pub id: String,
    /// Image name (e.g. `"ubuntu"`).
    pub image: String,
    /// Command and arguments to run inside the container.
    pub command: Vec<String>,
    /// PID of the container init process (set after [`start`](Self::start)).
    pub pid: Option<u32>,
    /// Current lifecycle state.
    pub state: ContainerState,
    /// Timestamp when the container was created.
    pub created_at: DateTime<Utc>,
    /// Path to the overlay `merged/` directory used as the container rootfs.
    pub rootfs_path: PathBuf,
    /// Path to the container's cgroup directory.
    pub cgroup_path: PathBuf,
}

impl Container {
    /// Create a new container record (does **not** start the process).
    ///
    /// # Arguments
    ///
    /// * `image` -- Image name.
    /// * `command` -- Command + args vector.
    /// * `base_dir` -- Per-container working directory (e.g.
    ///   `/var/lib/minibox/containers/{id}`).
    /// * `cgroup_config` -- Resource limits.
    pub fn new(
        image: impl Into<String>,
        command: Vec<String>,
        base_dir: &Path,
        _cgroup_config: CgroupConfig,
    ) -> anyhow::Result<Self> {
        let id = Uuid::new_v4()
            .to_string()
            .chars()
            .take(12)
            .collect::<String>();

        let cgroup_path = cgroup_path_for(&id);
        let rootfs_path = base_dir.join("merged");

        debug!("creating container id={}", id);

        Ok(Self {
            id,
            image: image.into(),
            command,
            pid: None,
            state: ContainerState::Created,
            created_at: Utc::now(),
            rootfs_path,
            cgroup_path,
        })
    }

    /// Start the container:
    /// 1. Create and configure the cgroup.
    /// 2. Mount the overlay rootfs.
    /// 3. Clone a child process with all namespaces.
    /// 4. Store the child PID and transition to [`Running`](ContainerState::Running).
    pub fn start(
        &mut self,
        base_dir: &Path,
        image_layers: &[PathBuf],
        cgroup_config: CgroupConfig,
    ) -> anyhow::Result<()> {
        if self.state != ContainerState::Created {
            bail!("container {} is not in Created state", self.id);
        }

        info!("starting container {}", self.id);

        // 1. Set up cgroup.
        let cgroup_manager = CgroupManager::new(&self.id, cgroup_config);
        cgroup_manager
            .create()
            .with_context(|| format!("failed to create cgroup for container {}", self.id))?;

        // 2. Mount overlay rootfs.
        let merged = setup_overlay(image_layers, base_dir)
            .with_context(|| format!("failed to set up overlay for container {}", self.id))?;
        self.rootfs_path = merged.clone();

        // 3. Spawn child process.
        let process_config = ContainerConfig {
            rootfs: merged,
            command: self.command.first().cloned().unwrap_or_default(),
            args: self.command.iter().skip(1).cloned().collect(),
            env: vec![
                "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
                "TERM=xterm".into(),
            ],
            namespace_config: NamespaceConfig::all(),
            cgroup_path: cgroup_manager.cgroup_path().to_path_buf(),
            hostname: format!("minibox-{}", &self.id[..8]),
            capture_output: false,
            pre_exec_hooks: vec![],
            mounts: vec![],
            privileged: false,
            pty: None,
        };

        let spawn = spawn_container_process(process_config)
            .with_context(|| format!("failed to spawn container process for {}", self.id))?;

        self.pid = Some(spawn.pid);
        self.state = ContainerState::Running;
        info!("container {} running with PID={}", self.id, spawn.pid);
        Ok(())
    }

    /// Stop the container:
    /// 1. Send SIGTERM to the container process.
    /// 2. Wait up to 5 seconds for a graceful exit.
    /// 3. Send SIGKILL if still running.
    /// 4. Remove the cgroup.
    pub fn stop(&mut self) -> anyhow::Result<()> {
        if self.state != ContainerState::Running {
            bail!("container {} is not running", self.id);
        }

        let pid = self
            .pid
            .ok_or_else(|| anyhow::anyhow!("container {} has no PID", self.id))?;

        info!("stopping container {} (PID={})", self.id, pid);

        let nix_pid = Pid::from_raw(pid as i32);

        // Send SIGTERM first.
        if let Err(e) = kill(nix_pid, Signal::SIGTERM) {
            warn!("SIGTERM to PID {} failed: {}", pid, e);
        }

        // Wait up to 5 seconds for the process to exit.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut exited = false;
        while std::time::Instant::now() < deadline {
            match nix::sys::wait::waitpid(nix_pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)) {
                Ok(nix::sys::wait::WaitStatus::StillAlive) => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Ok(_) => {
                    exited = true;
                    break;
                }
                Err(nix::errno::Errno::ECHILD) => {
                    // Process already reaped.
                    exited = true;
                    break;
                }
                Err(e) => {
                    warn!("waitpid for PID {} failed: {}", pid, e);
                    break;
                }
            }
        }

        if !exited {
            info!(
                "container {} did not exit gracefully, sending SIGKILL",
                self.id
            );
            if let Err(e) = kill(nix_pid, Signal::SIGKILL) {
                warn!("SIGKILL to PID {} failed: {}", pid, e);
            }
            // Reap the process.
            let _ = wait_for_exit(pid);
        }

        // Clean up the cgroup.
        let cgroup_manager = CgroupManager::new(&self.id, CgroupConfig::default());
        if let Err(e) = cgroup_manager.cleanup() {
            warn!("cgroup cleanup for container {} failed: {}", self.id, e);
        }

        self.state = ContainerState::Stopped;
        info!("container {} stopped", self.id);
        Ok(())
    }

    /// Remove the container: clean up the overlay mounts and mark as removed.
    ///
    /// The container must be in the [`Stopped`](ContainerState::Stopped) state.
    pub fn remove(&mut self, base_dir: &Path) -> anyhow::Result<()> {
        if self.state == ContainerState::Running {
            bail!("container {} is still running; stop it first", self.id);
        }
        if self.state == ContainerState::Removed {
            return Ok(());
        }

        info!("removing container {}", self.id);

        if let Err(e) = cleanup_mounts(base_dir) {
            warn!("mount cleanup for container {} failed: {}", self.id, e);
        }

        self.state = ContainerState::Removed;
        info!("container {} removed", self.id);
        Ok(())
    }
}
