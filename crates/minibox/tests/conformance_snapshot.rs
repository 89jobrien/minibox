//! Conformance tests for VM snapshot capability.
//!
//! Gated on [`BackendCapability::Checkpoint`]. Backends that do not declare
//! this capability will have all tests in this module skipped.

use minibox_core::domain::{
    BackendCapability, BackendCapabilitySet, NoopVmCheckpoint, SnapshotInfo, VmCheckpoint,
};
use std::path::Path;
use tempfile::TempDir;

/// Helper: returns true if the capability set includes Checkpoint.
fn has_checkpoint(caps: &BackendCapabilitySet) -> bool {
    caps.supports(BackendCapability::Checkpoint)
}

// ─── NoopVmCheckpoint tests ─────────────────────────────────────────────────

#[test]
fn noop_save_returns_error() {
    let noop = NoopVmCheckpoint;
    let result = noop.save_snapshot("ctr-1", Path::new("/tmp/snap"));
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("not supported"),
        "expected 'not supported' in error: {msg}"
    );
}

#[test]
fn noop_restore_returns_error() {
    let noop = NoopVmCheckpoint;
    let result = noop.restore_snapshot("ctr-1", Path::new("/tmp/snap"));
    assert!(result.is_err());
}

#[test]
fn noop_list_returns_error() {
    let noop = NoopVmCheckpoint;
    let result = noop.list_snapshots("ctr-1");
    assert!(result.is_err());
}

#[test]
fn checkpoint_capability_not_in_empty_set() {
    let caps = BackendCapabilitySet::new();
    assert!(!has_checkpoint(&caps));
}

#[test]
fn checkpoint_capability_in_set_when_added() {
    let caps = BackendCapabilitySet::new().with(BackendCapability::Checkpoint);
    assert!(has_checkpoint(&caps));
}

#[test]
fn snapshot_info_serializes_roundtrip() {
    let info = SnapshotInfo {
        container_id: "ctr-abc".to_string(),
        name: "after-setup".to_string(),
        created_at: "2026-04-27T00:00:00Z".to_string(),
        adapter: "smolvm".to_string(),
        image: "ubuntu:24.04".to_string(),
        size_bytes: 104_857_600,
    };
    let json = serde_json::to_string(&info).expect("serialize");
    let deser: SnapshotInfo = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.container_id, "ctr-abc");
    assert_eq!(deser.name, "after-setup");
    assert_eq!(deser.size_bytes, 104_857_600);
}
