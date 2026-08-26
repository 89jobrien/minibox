//! VM checkpoint metadata and persistence port definitions.

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// VM Checkpoint Port
// ---------------------------------------------------------------------------

/// Metadata describing a saved VM snapshot.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotInfo {
    /// Container/VM ID this snapshot belongs to.
    pub container_id: String,
    /// Human-readable snapshot name.
    pub name: String,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// Adapter that created this snapshot.
    pub adapter: String,
    /// Image the container was running when snapshotted.
    pub image: String,
    /// Snapshot size in bytes (0 if unknown).
    pub size_bytes: u64,
}

/// Port for saving and restoring VM state checkpoints.
///
/// Adapters that support checkpointing (smolvm, krun, vz) implement this
/// trait. Adapters that do not support it return an error from every method
/// and omit [`BackendCapability::Checkpoint`] from their capability set.
pub trait VmCheckpoint: Send + Sync {
    /// Persist the current VM/container state to `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if checkpointing is unsupported or the OS call fails.
    fn save_snapshot(&self, container_id: &str, path: &Path) -> Result<SnapshotInfo>;

    /// Restore VM/container state from a previously saved snapshot at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot cannot be found or restored.
    fn restore_snapshot(&self, container_id: &str, path: &Path) -> Result<()>;

    /// List all snapshots for `container_id`.
    ///
    /// # Errors
    ///
    /// Returns an error if listing snapshots fails.
    fn list_snapshots(&self, container_id: &str) -> Result<Vec<SnapshotInfo>>;
}

/// Type alias for a shared, dynamic [`VmCheckpoint`] implementation.
pub type DynVmCheckpoint = Arc<dyn VmCheckpoint>;

/// A no-op [`VmCheckpoint`] that always returns "not supported".
///
/// Used as the default adapter for backends without checkpoint support.
pub struct NoopVmCheckpoint;

impl VmCheckpoint for NoopVmCheckpoint {
    fn save_snapshot(&self, _container_id: &str, _path: &Path) -> Result<SnapshotInfo> {
        anyhow::bail!("checkpoint: not supported by this adapter")
    }

    fn restore_snapshot(&self, _container_id: &str, _path: &Path) -> Result<()> {
        anyhow::bail!("checkpoint: not supported by this adapter")
    }

    fn list_snapshots(&self, _container_id: &str) -> Result<Vec<SnapshotInfo>> {
        anyhow::bail!("checkpoint: not supported by this adapter")
    }
}
