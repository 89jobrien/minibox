//! Persistent container state tracking.
//!
//! `DaemonState` is the single shared data structure held behind an
//! `Arc<DaemonState>`.  All mutable access is gated behind a tokio
//! `RwLock` so many readers can proceed concurrently while writes are
//! exclusive.
//!
//! State is persisted to a JSON file after every mutation so that
//! container records survive daemon restarts.

use minibox_lib::image::ImageStore;
use minibox_lib::protocol::ContainerInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use tracing::{debug, warn};

// SECURITY: Maximum concurrent container spawn operations to prevent fork bombs
const MAX_CONCURRENT_SPAWNS: usize = 100;

/// Default state file name within the data directory.
const STATE_FILENAME: &str = "state.json";

/// A complete record for a container tracked by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerRecord {
    /// Serialisable snapshot shared with the CLI.
    pub info: ContainerInfo,
    /// Host-namespace PID, or `None` if the process has not started yet
    /// or has exited.
    pub pid: Option<u32>,
    /// Path to the merged overlay directory used as the container rootfs.
    pub rootfs_path: PathBuf,
    /// Path to the container's cgroup directory.
    pub cgroup_path: PathBuf,
}

/// Shared daemon state, cheap to clone because it wraps `Arc`s internally.
#[derive(Clone)]
pub struct DaemonState {
    /// All containers known to the daemon.
    containers: Arc<RwLock<HashMap<String, ContainerRecord>>>,
    /// Image cache / pull facility.
    pub image_store: Arc<ImageStore>,
    /// SECURITY: Semaphore limiting concurrent container spawn operations
    pub spawn_semaphore: Arc<Semaphore>,
    /// Path to the state file on disk.
    state_file: PathBuf,
}

impl DaemonState {
    /// Create a fresh `DaemonState` using the given image store.
    ///
    /// `data_dir` is the base directory where `state.json` will be written
    /// (e.g. `/var/lib/minibox`).
    pub fn new(image_store: ImageStore, data_dir: &Path) -> Self {
        Self {
            containers: Arc::new(RwLock::new(HashMap::new())),
            image_store: Arc::new(image_store),
            spawn_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_SPAWNS)),
            state_file: data_dir.join(STATE_FILENAME),
        }
    }

    /// Load previously persisted state from disk.
    ///
    /// Any containers that were "Running" when the daemon last exited are
    /// marked "Stopped" since the processes are no longer alive.
    ///
    /// Returns silently if the state file does not exist or is unreadable.
    pub async fn load_from_disk(&self) {
        let path = &self.state_file;
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!("no state file at {}, starting fresh", path.display());
                return;
            }
            Err(e) => {
                warn!("failed to read state file {}: {}", path.display(), e);
                return;
            }
        };

        let mut records: HashMap<String, ContainerRecord> = match serde_json::from_str(&data) {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "failed to parse state file {} (starting fresh): {}",
                    path.display(),
                    e
                );
                return;
            }
        };

        // Processes from the previous daemon session are gone.
        for record in records.values_mut() {
            if record.info.state == "Running" || record.info.state == "Created" {
                debug!(
                    "marking stale container {} as Stopped (was {})",
                    record.info.id, record.info.state
                );
                record.info.state = "Stopped".to_string();
                record.info.pid = None;
                record.pid = None;
            }
        }

        let count = records.len();
        *self.containers.write().await = records;
        debug!("loaded {} container records from disk", count);
    }

    /// Persist the current state to disk (atomic write via temp file + rename).
    async fn save_to_disk(&self) {
        let map = self.containers.read().await;
        let json = match serde_json::to_string_pretty(&*map) {
            Ok(j) => j,
            Err(e) => {
                warn!("failed to serialise state: {}", e);
                return;
            }
        };
        drop(map); // release lock before I/O

        let tmp_path = self.state_file.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp_path, &json) {
            warn!("failed to write state file {}: {}", tmp_path.display(), e);
            return;
        }
        if let Err(e) = std::fs::rename(&tmp_path, &self.state_file) {
            warn!(
                "failed to rename {} → {}: {}",
                tmp_path.display(),
                self.state_file.display(),
                e
            );
        }
    }

    /// Register a new container record.
    pub async fn add_container(&self, record: ContainerRecord) {
        debug!("adding container {}", record.info.id);
        let mut map = self.containers.write().await;
        map.insert(record.info.id.clone(), record);
        drop(map);
        self.save_to_disk().await;
    }

    /// Remove and return a container record.
    pub async fn remove_container(&self, id: &str) -> Option<ContainerRecord> {
        debug!("removing container {}", id);
        let mut map = self.containers.write().await;
        let removed = map.remove(id);
        drop(map);
        self.save_to_disk().await;
        removed
    }

    /// Look up a container by its short ID.
    pub async fn get_container(&self, id: &str) -> Option<ContainerRecord> {
        let map = self.containers.read().await;
        map.get(id).cloned()
    }

    /// Return `ContainerInfo` snapshots for every container.
    pub async fn list_containers(&self) -> Vec<ContainerInfo> {
        let map = self.containers.read().await;
        map.values().map(|r| r.info.clone()).collect()
    }

    /// Change the `state` field of a container (e.g. "Running" → "Stopped").
    pub async fn update_container_state(&self, id: &str, new_state: &str) {
        let mut map = self.containers.write().await;
        if let Some(record) = map.get_mut(id) {
            debug!(
                "updating container {} state {} → {}",
                id, record.info.state, new_state
            );
            record.info.state = new_state.to_string();
            if new_state == "Stopped" {
                record.info.pid = None;
                record.pid = None;
            }
        }
        drop(map);
        self.save_to_disk().await;
    }

    /// Set the PID for a container (called after the container process is
    /// successfully forked).
    pub async fn set_container_pid(&self, id: &str, pid: u32) {
        let mut map = self.containers.write().await;
        if let Some(record) = map.get_mut(id) {
            record.pid = Some(pid);
            record.info.pid = Some(pid);
            record.info.state = "Running".to_string();
        }
        drop(map);
        self.save_to_disk().await;
    }
}
