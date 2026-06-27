//! Conformance tests for the [`VmCheckpoint`] trait contract.
//!
//! All tests use `MockVmCheckpoint` — no real VM state is touched.

use minibox::testing::mocks::vm_checkpoint::MockVmCheckpoint;
use minibox_core::domain::VmCheckpoint;
use std::path::PathBuf;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn snap_path() -> PathBuf {
    PathBuf::from("/tmp/test-snapshot")
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// `save_snapshot` succeeds and returns `SnapshotInfo` with matching `container_id`.
pub struct SaveSnapshotReturnsInfo;
impl ConformanceTest for SaveSnapshotReturnsInfo {
    fn name(&self) -> &'static str {
        "save_snapshot_returns_info"
    }
    fn adapter(&self) -> &'static str {
        "vm_checkpoint"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockVmCheckpoint::new();
        let result = mock.save_snapshot("ctr-snap-001", &snap_path());
        if let Some(info) = ctx.assert_ok(result, "save_snapshot should succeed") {
            ctx.assert_eq(
                "ctr-snap-001".to_string(),
                info.container_id,
                "snapshot container_id must match",
            );
        }
        ctx.result()
    }
}

/// `save_snapshot` increments the stored snapshot count.
pub struct SaveSnapshotIncrementsCount;
impl ConformanceTest for SaveSnapshotIncrementsCount {
    fn name(&self) -> &'static str {
        "save_snapshot_increments_count"
    }
    fn adapter(&self) -> &'static str {
        "vm_checkpoint"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockVmCheckpoint::new();
        mock.save_snapshot("ctr-snap-002", &snap_path())
            .expect("save");
        ctx.assert_eq(1, mock.snapshot_count(), "snapshot_count after one save");
        mock.save_snapshot("ctr-snap-002", &snap_path())
            .expect("save");
        ctx.assert_eq(2, mock.snapshot_count(), "snapshot_count after two saves");
        ctx.result()
    }
}

/// `list_snapshots` returns only snapshots for the requested container.
pub struct ListSnapshotsFiltersByContainerId;
impl ConformanceTest for ListSnapshotsFiltersByContainerId {
    fn name(&self) -> &'static str {
        "list_snapshots_filters_by_container_id"
    }
    fn adapter(&self) -> &'static str {
        "vm_checkpoint"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockVmCheckpoint::new();
        mock.save_snapshot("ctr-a", &snap_path()).expect("save a");
        mock.save_snapshot("ctr-b", &snap_path()).expect("save b");
        let list = mock.list_snapshots("ctr-a").expect("list");
        ctx.assert_eq(
            1,
            list.len(),
            "list_snapshots should return only ctr-a snapshots",
        );
        ctx.assert_eq(
            "ctr-a".to_string(),
            list[0].container_id.clone(),
            "container_id in list",
        );
        ctx.result()
    }
}

/// `list_snapshots` returns empty for an unknown container.
pub struct ListSnapshotsEmptyForUnknown;
impl ConformanceTest for ListSnapshotsEmptyForUnknown {
    fn name(&self) -> &'static str {
        "list_snapshots_empty_for_unknown"
    }
    fn adapter(&self) -> &'static str {
        "vm_checkpoint"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockVmCheckpoint::new();
        let list = mock.list_snapshots("no-such-container").expect("list");
        ctx.assert_eq(
            0,
            list.len(),
            "list_snapshots must be empty for unknown container",
        );
        ctx.result()
    }
}

/// `restore_snapshot` succeeds after a prior save.
pub struct RestoreSnapshotSucceedsAfterSave;
impl ConformanceTest for RestoreSnapshotSucceedsAfterSave {
    fn name(&self) -> &'static str {
        "restore_snapshot_succeeds_after_save"
    }
    fn adapter(&self) -> &'static str {
        "vm_checkpoint"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockVmCheckpoint::new();
        mock.save_snapshot("ctr-snap-003", &snap_path())
            .expect("save");
        let result = mock.restore_snapshot("ctr-snap-003", &snap_path());
        ctx.assert_ok(result, "restore_snapshot should succeed after save");
        ctx.result()
    }
}

/// `restore_snapshot` returns Err when no snapshot exists.
pub struct RestoreSnapshotFailsWithoutSave;
impl ConformanceTest for RestoreSnapshotFailsWithoutSave {
    fn name(&self) -> &'static str {
        "restore_snapshot_fails_without_save"
    }
    fn adapter(&self) -> &'static str {
        "vm_checkpoint"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockVmCheckpoint::new();
        let result = mock.restore_snapshot("nonexistent", &snap_path());
        ctx.assert_err(
            result,
            "restore_snapshot without prior save must return Err",
        );
        ctx.result()
    }
}

/// Return all `vm_checkpoint` conformance tests.
#[must_use]
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(SaveSnapshotReturnsInfo),
        Box::new(SaveSnapshotIncrementsCount),
        Box::new(ListSnapshotsFiltersByContainerId),
        Box::new(ListSnapshotsEmptyForUnknown),
        Box::new(RestoreSnapshotSucceedsAfterSave),
        Box::new(RestoreSnapshotFailsWithoutSave),
    ]
}
