//! Init-path regression tests for startup reconciliation (`MoA` review H6).
//!
//! `run_daemon` must not only load persisted state but also call
//! `reconcile_on_startup` with the production checkers so that records left
//! `"Running"`/`"Paused"` by a previous daemon are probed against live host
//! PIDs. These tests drive the same load+reconcile sequence the daemon init
//! path (`load_state` in `main.rs`) uses, with the real `KillProcessChecker`
//! and `FsCgroupFreezeChecker` adapters.

#![cfg(unix)]

use minibox_core::image::ImageStore;
use miniboxd::state::{DaemonState, FsCgroupFreezeChecker, KillProcessChecker};
use tempfile::TempDir;

/// PID far above any platform `pid_max` (Linux default 4194304, macOS ~99999)
/// so `kill(pid, 0)` deterministically reports ESRCH.
const DEAD_PID: u32 = 999_999_999;

/// Minimal persisted state.json with one container record. Omitted
/// `ContainerRecord` fields deserialize via `#[serde(default)]`.
fn write_state_json(data_dir: &std::path::Path, id: &str, state: &str, pid: u32) {
    let json = format!(
        r#"{{
  "{id}": {{
    "info": {{
      "id": "{id}",
      "image": "alpine:latest",
      "command": "/bin/sh",
      "state": "{state}",
      "created_at": "2026-01-01T00:00:00Z",
      "pid": {pid}
    }},
    "pid": {pid},
    "rootfs_path": "/mock/rootfs",
    "cgroup_path": "/mock/cgroup",
    "post_exit_hooks": []
  }}
}}"#
    );
    std::fs::write(data_dir.join("state.json"), json).expect("write state.json");
}

/// The same init sequence `run_daemon` uses: new + `load_from_disk` +
/// `reconcile_on_startup` with the production checker adapters.
async fn load_and_reconcile(data_dir: &std::path::Path) -> DaemonState {
    let image_store = ImageStore::new(data_dir.join("images")).expect("ImageStore::new");
    let state = DaemonState::new(image_store, data_dir);
    state.load_from_disk().await;
    state
        .reconcile_on_startup(&KillProcessChecker, &FsCgroupFreezeChecker)
        .await;
    state
}

#[tokio::test]
async fn startup_reconciles_running_record_with_dead_pid() {
    let tmp = TempDir::new().expect("tempdir");
    write_state_json(tmp.path(), "run-dead", "Running", DEAD_PID);

    let state = load_and_reconcile(tmp.path()).await;

    let containers = state.list_containers().await;
    assert_eq!(containers.len(), 1, "expected one loaded record");
    assert_eq!(
        containers[0].state, "Orphaned",
        "dead-PID Running record must be reconciled to Orphaned"
    );
    assert_eq!(containers[0].pid, None, "stale PID must be cleared");
}

#[tokio::test]
async fn startup_reconciles_running_record_with_alive_unmonitored_pid() {
    // The test process itself is alive, but after a daemon restart no reaper
    // is attached — reconcile must still mark the record Orphaned.
    let tmp = TempDir::new().expect("tempdir");
    write_state_json(tmp.path(), "run-alive", "Running", std::process::id());

    let state = load_and_reconcile(tmp.path()).await;

    let containers = state.list_containers().await;
    assert_eq!(containers.len(), 1, "expected one loaded record");
    assert_eq!(
        containers[0].state, "Orphaned",
        "alive-but-unmonitored Running record must be reconciled to Orphaned"
    );
}

#[tokio::test]
async fn startup_reconciles_paused_record_with_dead_pid() {
    let tmp = TempDir::new().expect("tempdir");
    write_state_json(tmp.path(), "run-paused", "Paused", DEAD_PID);

    let state = load_and_reconcile(tmp.path()).await;

    let containers = state.list_containers().await;
    assert_eq!(containers.len(), 1, "expected one loaded record");
    assert_eq!(
        containers[0].state, "Orphaned",
        "dead-PID Paused record must be reconciled to Orphaned"
    );
}

#[tokio::test]
async fn startup_leaves_stopped_record_untouched() {
    let tmp = TempDir::new().expect("tempdir");
    write_state_json(tmp.path(), "run-stopped", "Stopped", 0);

    let state = load_and_reconcile(tmp.path()).await;

    let containers = state.list_containers().await;
    assert_eq!(containers.len(), 1, "expected one loaded record");
    assert_eq!(
        containers[0].state, "Stopped",
        "Stopped records must not be touched by reconciliation"
    );
}

#[tokio::test]
async fn startup_persists_reconciled_state_back_to_disk() {
    // reconcile_on_startup must save orphaned transitions so a subsequent
    // crash does not resurrect the stale "Running" record.
    let tmp = TempDir::new().expect("tempdir");
    write_state_json(tmp.path(), "run-dead", "Running", DEAD_PID);

    let _state = load_and_reconcile(tmp.path()).await;

    let persisted =
        std::fs::read_to_string(tmp.path().join("state.json")).expect("read state.json");
    assert!(
        persisted.contains("Orphaned"),
        "reconciled Orphaned state must be persisted to disk, got: {persisted}"
    );
    assert!(
        !persisted.contains("\"Running\""),
        "stale Running state must not survive reconciliation on disk"
    );
}
