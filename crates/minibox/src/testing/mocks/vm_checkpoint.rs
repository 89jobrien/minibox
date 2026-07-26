//! Mock [`VmCheckpoint`] for conformance testing.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use anyhow::Result;
use minibox_core::domain::{SnapshotInfo, VmCheckpoint};
use std::path::Path;
use std::sync::Mutex;

/// Mock VM checkpoint that stores snapshots in memory.
///
/// Snapshots are keyed by container ID and accumulated across calls.
/// Useful for verifying the `VmCheckpoint` contract without real VM state.
#[derive(Debug)]
pub struct MockVmCheckpoint {
    snapshots: Mutex<Vec<SnapshotInfo>>,
}

impl MockVmCheckpoint {
    /// Create a fresh mock with no snapshots.
    pub const fn new() -> Self {
        Self {
            snapshots: Mutex::new(Vec::new()),
        }
    }

    /// Total number of snapshots stored across all container IDs.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.lock().expect("lock").len()
    }
}

impl Default for MockVmCheckpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl VmCheckpoint for MockVmCheckpoint {
    fn save_snapshot(&self, container_id: &str, path: &Path) -> Result<SnapshotInfo> {
        let info = SnapshotInfo {
            container_id: container_id.to_string(),
            name: path.file_name().map_or_else(
                || "snapshot".to_string(),
                |n| n.to_string_lossy().to_string(),
            ),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            adapter: "mock".to_string(),
            image: "alpine:3.18".to_string(),
            size_bytes: 0,
        };
        self.snapshots.lock().expect("lock").push(info.clone());
        Ok(info)
    }

    fn restore_snapshot(&self, container_id: &str, _path: &Path) -> Result<()> {
        let snaps = self.snapshots.lock().expect("lock");
        if snaps.iter().any(|s| s.container_id == container_id) {
            Ok(())
        } else {
            anyhow::bail!("mock: no snapshot found for container {container_id}")
        }
    }

    fn list_snapshots(&self, container_id: &str) -> Result<Vec<SnapshotInfo>> {
        let snaps = self.snapshots.lock().expect("lock");
        Ok(snaps
            .iter()
            .filter(|s| s.container_id == container_id)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn save_and_list_snapshot() {
        let mock = MockVmCheckpoint::new();
        let path = PathBuf::from("/tmp/snap1");
        mock.save_snapshot("ctr-1", &path)
            .expect("save should succeed");
        let list = mock.list_snapshots("ctr-1").expect("list should succeed");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].container_id, "ctr-1");
    }

    #[test]
    fn restore_fails_when_no_snapshot() {
        let mock = MockVmCheckpoint::new();
        let path = PathBuf::from("/tmp/snap1");
        assert!(mock.restore_snapshot("ctr-999", &path).is_err());
    }

    #[test]
    fn list_returns_empty_for_unknown_container() {
        let mock = MockVmCheckpoint::new();
        let list = mock.list_snapshots("unknown").expect("list should succeed");
        assert!(list.is_empty());
    }
}
