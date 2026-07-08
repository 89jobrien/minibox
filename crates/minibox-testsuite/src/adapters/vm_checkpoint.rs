//! Conformance tests for the [`VmCheckpoint`] trait contract.
//!
//! All tests use `MockVmCheckpoint` — no real VM state is touched.

use minibox::testing::mocks::vm_checkpoint::MockVmCheckpoint;
use minibox_core::domain::VmCheckpoint;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn snap_path() -> PathBuf {
    PathBuf::from("/tmp/test-snapshot")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "save_snapshot_returns_info",
    adapter: "vm_checkpoint",
    capability: Checkpoint,
    category: Unit,
    |ctx| {
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

crate::conformance_test! {
    name: "save_snapshot_increments_count",
    adapter: "vm_checkpoint",
    capability: Checkpoint,
    category: Unit,
    |ctx| {
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

crate::conformance_test! {
    name: "list_snapshots_filters_by_container_id",
    adapter: "vm_checkpoint",
    capability: Checkpoint,
    category: Unit,
    |ctx| {
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

crate::conformance_test! {
    name: "list_snapshots_empty_for_unknown",
    adapter: "vm_checkpoint",
    capability: Checkpoint,
    category: EdgeCase,
    |ctx| {
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

crate::conformance_test! {
    name: "restore_snapshot_succeeds_after_save",
    adapter: "vm_checkpoint",
    capability: Checkpoint,
    category: Unit,
    |ctx| {
        let mock = MockVmCheckpoint::new();
        mock.save_snapshot("ctr-snap-003", &snap_path())
            .expect("save");
        let result = mock.restore_snapshot("ctr-snap-003", &snap_path());
        ctx.assert_ok(result, "restore_snapshot should succeed after save");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "restore_snapshot_fails_without_save",
    adapter: "vm_checkpoint",
    capability: Checkpoint,
    category: EdgeCase,
    |ctx| {
        let mock = MockVmCheckpoint::new();
        let result = mock.restore_snapshot("nonexistent", &snap_path());
        ctx.assert_err(
            result,
            "restore_snapshot without prior save must return Err",
        );
        ctx.result()
    }
}
