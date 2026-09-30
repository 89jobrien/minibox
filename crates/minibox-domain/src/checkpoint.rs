//! VM checkpoint metadata and persistence port definitions.

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// VM Checkpoint Port
// ---------------------------------------------------------------------------

// TODO(feature-idea-31): define versioned immutable snapshot IDs, parent lineage, payload
// lengths/digests, and root-last atomic publication before implementing backend storage.

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
/// The port is defined but **unimplemented**: no adapter in this workspace
/// satisfies it, and none declares [`BackendCapability::Checkpoint`]. Every
/// production wiring site therefore installs [`NoopVmCheckpoint`], so snapshot
/// requests fail with an explicit "not supported" error rather than reporting
/// state that was never persisted. An adapter that gains real checkpointing
/// implements this trait and declares the capability in the same change.
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
/// This is the production default for every adapter, because no adapter
/// implements checkpointing. All three methods fail with the same reason and
/// none touch the filesystem, so a failed request leaves no snapshot behind.
/// Note that `list_snapshots` also errors rather than returning an empty
/// vector: an empty list would assert that no snapshots exist, which this
/// adapter cannot know.
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
