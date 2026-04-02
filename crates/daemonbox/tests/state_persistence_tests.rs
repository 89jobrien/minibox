//! Targeted coverage tests for `DaemonState` persistence and state-transition
//! edge cases that are not exercised by `daemon_recovery_tests.rs`.
//!
//! Lines targeted:
//!   104–118  load_from_disk: stale Running/Created containers → Stopped
//!   141–142  save_to_disk:   tmp-file write failure (data_dir does not exist)
//!   151–158  save_to_disk:   rename failure (read-only parent dir, unix only)
//!   223      update_container_state: container not found (no-op)
//!   231      update_container_state: Stopped → clears pid fields

use daemonbox::state::{ContainerRecord, ContainerState, DaemonState};
use minibox_core::image::ImageStore;
use minibox_core::protocol::ContainerInfo;
use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_image_store(tmp: &TempDir) -> ImageStore {
    ImageStore::new(tmp.path().join("images")).expect("ImageStore::new")
}

fn make_state(tmp: &TempDir) -> DaemonState {
    DaemonState::new(make_image_store(tmp), tmp.path())
}

fn make_record(id: &str, state: &str, pid: Option<u32>) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: state.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid,
        },
        pid,
        rootfs_path: PathBuf::from("/mock/rootfs"),
        cgroup_path: PathBuf::from("/mock/cgroup"),
        post_exit_hooks: vec![],
        overlay_upper: None,
        source_image_ref: None,
    }
}

/// Minimal state.json with one container in the given state.
fn write_state_json(dir: &TempDir, id: &str, state: &str, pid: Option<u32>) {
    let pid_json = match pid {
        Some(p) => p.to_string(),
        None => "null".to_string(),
    };
    let json = format!(
        r#"{{
  "{id}": {{
    "info": {{
      "id": "{id}",
      "image": "alpine:latest",
      "command": "/bin/sh",
      "state": "{state}",
      "created_at": "2026-01-01T00:00:00Z",
      "pid": {pid_json}
    }},
    "pid": {pid_json},
    "rootfs_path": "/mock/rootfs",
    "cgroup_path": "/mock/cgroup",
    "post_exit_hooks": []
  }}
}}"#
    );
    std::fs::write(dir.path().join("state.json"), json).expect("write state.json");
}

// ---------------------------------------------------------------------------
// load_from_disk — stale container recovery (lines 104–118)
// ---------------------------------------------------------------------------

/// A container persisted as "Running" must be loaded as "Stopped" with no pid.
#[tokio::test]
async fn test_load_from_disk_marks_stale_running_as_stopped() {
    let tmp = TempDir::new().unwrap();
    write_state_json(&tmp, "run-abc", "Running", Some(12345));

    let state = make_state(&tmp);
    state.load_from_disk().await;

    let containers = state.list_containers().await;
    assert_eq!(containers.len(), 1, "expected one container to be loaded");

    let c = &containers[0];
    assert_eq!(
        c.state, "Stopped",
        "stale Running container must be marked Stopped"
    );
    assert_eq!(c.pid, None, "stale Running container must have pid cleared");
}

/// A container persisted as "Created" must also be loaded as "Stopped" with no pid.
#[tokio::test]
async fn test_load_from_disk_marks_stale_created_as_stopped() {
    let tmp = TempDir::new().unwrap();
    write_state_json(&tmp, "cre-xyz", "Created", None);

    let state = make_state(&tmp);
    state.load_from_disk().await;

    let containers = state.list_containers().await;
    assert_eq!(containers.len(), 1, "expected one container to be loaded");

    let c = &containers[0];
    assert_eq!(
        c.state, "Stopped",
        "stale Created container must be marked Stopped"
    );
    assert_eq!(c.pid, None, "stale Created container must have pid cleared");
}

/// A container persisted as "Stopped" must be loaded unchanged.
#[tokio::test]
async fn test_load_from_disk_preserves_already_stopped() {
    let tmp = TempDir::new().unwrap();
    write_state_json(&tmp, "stop-zzz", "Stopped", None);

    let state = make_state(&tmp);
    state.load_from_disk().await;

    let containers = state.list_containers().await;
    assert_eq!(containers.len(), 1, "expected one container to be loaded");

    let c = &containers[0];
    assert_eq!(
        c.state, "Stopped",
        "already-Stopped container must remain Stopped"
    );
}

/// Multiple containers with mixed states: Running and Created become Stopped,
/// already-Stopped containers are unchanged.
#[tokio::test]
async fn test_load_from_disk_mixed_states_all_stale_become_stopped() {
    let tmp = TempDir::new().unwrap();

    // Build a JSON with three containers.
    let json = r#"{
  "run-1": {
    "info": { "id": "run-1", "image": "alpine:latest", "command": "/bin/sh",
              "state": "Running", "created_at": "2026-01-01T00:00:00Z", "pid": 100 },
    "pid": 100, "rootfs_path": "/mock", "cgroup_path": "/mock", "post_exit_hooks": []
  },
  "cre-1": {
    "info": { "id": "cre-1", "image": "alpine:latest", "command": "/bin/sh",
              "state": "Created", "created_at": "2026-01-01T00:00:00Z", "pid": null },
    "pid": null, "rootfs_path": "/mock", "cgroup_path": "/mock", "post_exit_hooks": []
  },
  "stp-1": {
    "info": { "id": "stp-1", "image": "alpine:latest", "command": "/bin/sh",
              "state": "Stopped", "created_at": "2026-01-01T00:00:00Z", "pid": null },
    "pid": null, "rootfs_path": "/mock", "cgroup_path": "/mock", "post_exit_hooks": []
  }
}"#;
    std::fs::write(tmp.path().join("state.json"), json).unwrap();

    let state = make_state(&tmp);
    state.load_from_disk().await;

    let containers = state.list_containers().await;
    assert_eq!(containers.len(), 3);

    for c in &containers {
        assert_eq!(
            c.state, "Stopped",
            "container {} should be Stopped after load, got {}",
            c.id, c.state
        );
        assert_eq!(
            c.pid, None,
            "container {} should have no pid after load",
            c.id
        );
    }
}

// ---------------------------------------------------------------------------
// save_to_disk — write failure (lines 141–142)
// ---------------------------------------------------------------------------

/// If the data_dir does not exist, add_container must complete without panicking.
/// The failure is best-effort (warn! log only) — state is held in memory.
#[tokio::test]
async fn test_save_to_disk_write_failure_does_not_panic() {
    // Use a path that will never exist so the write fails.
    let nonexistent = PathBuf::from("/tmp/minibox-test-nonexistent-dir-98765");
    let tmp = TempDir::new().unwrap();
    let image_store = make_image_store(&tmp);
    let state = DaemonState::new(image_store, &nonexistent);

    // Must not panic — failure is silently logged.
    let record = make_record("write-fail-1", "Created", None);
    state.add_container(record).await;
}

// ---------------------------------------------------------------------------
// save_to_disk — rename failure (lines 151–158, unix only)
// ---------------------------------------------------------------------------

/// If the tmp file is written but the rename fails (e.g. because the target
/// directory has been made read-only after the write), the daemon must not panic.
#[cfg(unix)]
#[tokio::test]
async fn test_save_to_disk_rename_failure_does_not_panic() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();

    // Create state.json so the target exists (the write path itself will succeed
    // for the .tmp file, but we make the directory read-only so rename fails).
    std::fs::write(tmp.path().join("state.json"), b"{}").unwrap();

    let state = make_state(&tmp);

    // Make the directory read-only — the .tmp write and subsequent rename will
    // fail with EACCES.
    let perms = std::fs::Permissions::from_mode(0o555);
    std::fs::set_permissions(tmp.path(), perms).unwrap();

    // Must not panic — both failures are best-effort warn! logs.
    let record = make_record("rename-fail-1", "Created", None);
    state.add_container(record).await;

    // Restore permissions so TempDir can clean up.
    let perms = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(tmp.path(), perms).unwrap();
}

// ---------------------------------------------------------------------------
// update_container_state — container not found (line 223)
// ---------------------------------------------------------------------------

/// Calling update_container_state on a nonexistent ID must be a silent no-op.
#[tokio::test]
async fn test_update_container_state_nonexistent_is_noop() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    // No containers have been added — this must not panic.
    state
        .update_container_state("nonexistent-id", ContainerState::Stopped)
        .await
        .ok();

    let containers = state.list_containers().await;
    assert!(
        containers.is_empty(),
        "state must remain empty after noop update"
    );
}

/// update_container_state on a nonexistent ID while other containers exist
/// must leave existing containers untouched.
#[tokio::test]
async fn test_update_container_state_nonexistent_does_not_affect_others() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    state
        .add_container(make_record("existing-1", "Running", Some(42)))
        .await;

    state
        .update_container_state("no-such-container", ContainerState::Stopped)
        .await
        .ok();

    let c = state
        .get_container("existing-1")
        .await
        .expect("existing container must still be present");
    assert_eq!(
        c.info.state, "Running",
        "existing container state must be unchanged"
    );
    assert_eq!(
        c.info.pid,
        Some(42),
        "existing container pid must be unchanged"
    );
}

// ---------------------------------------------------------------------------
// update_container_state — Stopped clears pid (line 231)
// ---------------------------------------------------------------------------

/// Transitioning a container to "Stopped" must clear both pid fields.
#[tokio::test]
async fn test_update_container_state_stopped_clears_pid() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    let id = "pid-clear-test";
    state.add_container(make_record(id, "Created", None)).await;

    // Simulate fork: record PID and advance to Running.
    state.set_container_pid(id, 1234).await;

    // Verify pid is set.
    let running = state.get_container(id).await.expect("container must exist");
    assert_eq!(running.pid, Some(1234));
    assert_eq!(running.info.pid, Some(1234));
    assert_eq!(running.info.state, "Running");

    // Transition to Stopped — pid fields must be cleared.
    state
        .update_container_state(id, ContainerState::Stopped)
        .await
        .expect("Stopped transition");

    let stopped = state
        .get_container(id)
        .await
        .expect("container must still exist");
    assert_eq!(stopped.info.state, "Stopped", "state must be Stopped");
    assert_eq!(stopped.pid, None, "ContainerRecord.pid must be cleared");
    assert_eq!(stopped.info.pid, None, "ContainerInfo.pid must be cleared");
}

/// Transitioning to a non-"Stopped" state must NOT clear the pid fields.
#[tokio::test]
async fn test_update_container_state_non_stopped_preserves_pid() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    let id = "pid-preserve-test";
    state.add_container(make_record(id, "Created", None)).await;
    state.set_container_pid(id, 5678).await;

    // Transition to "Failed" (not "Stopped") — pid must be preserved.
    state
        .update_container_state(id, ContainerState::Failed)
        .await
        .expect("Failed transition");

    let record = state.get_container(id).await.expect("container must exist");
    assert_eq!(record.info.state, "Failed");
    assert_eq!(
        record.pid,
        Some(5678),
        "pid must not be cleared for non-Stopped transition"
    );
    assert_eq!(
        record.info.pid,
        Some(5678),
        "info.pid must not be cleared for non-Stopped transition"
    );
}

// ---------------------------------------------------------------------------
// Concurrent saves — atomicity under contention
// ---------------------------------------------------------------------------

/// Spawn N tasks that concurrently add/remove containers and verify that
/// the final state is consistent — no panics, no lost containers, and
/// the on-disk JSON is valid.
///
/// Validates the temp-write-and-rename atomicity guarantee under contention.
#[tokio::test]
async fn test_concurrent_add_remove_is_consistent() {
    use std::sync::Arc;

    let tmp = TempDir::new().unwrap();
    let state = Arc::new(make_state(&tmp));

    let n = 20usize;
    let mut handles = Vec::with_capacity(n);

    for i in 0..n {
        let s = Arc::clone(&state);
        handles.push(tokio::spawn(async move {
            let id = format!("container-{i:04}");
            let record = make_record(&id, "Created", None);
            s.add_container(record).await;
            s.set_container_pid(&id, (1000 + i) as u32).await;
            s.update_container_state(&id, ContainerState::Stopped)
                .await
                .expect("Stopped transition");
            s.remove_container(&id).await;
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }

    // All containers were added then removed — list must be empty.
    let containers = state.list_containers().await;
    assert!(
        containers.is_empty(),
        "expected 0 containers after concurrent add/remove, got {}",
        containers.len()
    );

    // state.json must be valid JSON (not a torn write).
    let json = std::fs::read_to_string(tmp.path().join("state.json"))
        .expect("state.json must exist after saves");
    let parsed: serde_json::Value =
        serde_json::from_str(&json).expect("state.json must be valid JSON after concurrent saves");
    assert!(parsed.is_object(), "state.json root must be an object");
}

/// Concurrent readers see a consistent snapshot even while writers are active.
#[tokio::test]
async fn test_concurrent_read_during_writes_does_not_panic() {
    use std::sync::Arc;

    let tmp = TempDir::new().unwrap();
    let state = Arc::new(make_state(&tmp));

    // Pre-seed a container so readers have something to find.
    let seed = make_record("seed-container", "Running", Some(42));
    state.add_container(seed).await;

    let n = 10usize;
    let mut handles = Vec::with_capacity(n * 2);

    // Writers: add and immediately remove transient containers.
    for i in 0..n {
        let s = Arc::clone(&state);
        handles.push(tokio::spawn(async move {
            let id = format!("transient-{i}");
            s.add_container(make_record(&id, "Created", None)).await;
            s.remove_container(&id).await;
        }));
    }

    // Readers: list containers concurrently with writes.
    for _ in 0..n {
        let s = Arc::clone(&state);
        handles.push(tokio::spawn(async move {
            let _ = s.list_containers().await;
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }

    // Seed container must still be present.
    assert!(
        state.get_container("seed-container").await.is_some(),
        "seed container must survive concurrent writes"
    );
}
