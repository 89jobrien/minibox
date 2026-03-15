//! In-memory container state tracking.
//!
//! `DaemonState` is the single shared data structure held behind an
//! `Arc<DaemonState>`.  All mutable access is gated behind a tokio
//! `RwLock` so many readers can proceed concurrently while writes are
//! exclusive.

use minibox_lib::image::ImageStore;
use minibox_lib::protocol::ContainerInfo;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use tracing::debug;

// SECURITY: Maximum concurrent container spawn operations to prevent fork bombs
const MAX_CONCURRENT_SPAWNS: usize = 100;

/// A complete record for a container tracked by the daemon.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields read through serialization and cleanup paths
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
}

impl DaemonState {
    /// Create a fresh `DaemonState` using the given image store.
    pub fn new(image_store: ImageStore) -> Self {
        Self {
            containers: Arc::new(RwLock::new(HashMap::new())),
            image_store: Arc::new(image_store),
            spawn_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_SPAWNS)),
        }
    }

    /// Register a new container record.
    pub async fn add_container(&self, record: ContainerRecord) {
        debug!("adding container {}", record.info.id);
        let mut map = self.containers.write().await;
        map.insert(record.info.id.clone(), record);
    }

    /// Remove and return a container record.
    pub async fn remove_container(&self, id: &str) -> Option<ContainerRecord> {
        debug!("removing container {}", id);
        let mut map = self.containers.write().await;
        map.remove(id)
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
    }
}
