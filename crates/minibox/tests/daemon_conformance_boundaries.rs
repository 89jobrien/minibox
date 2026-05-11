//! Conformance tests for the StateRepository/JsonFileRepository boundary.
//!
//! Tests the port contract between `DaemonState` and `StateRepository`, and verifies
//! that `JsonFileRepository` correctly implements atomic, transactional persistence.

use minibox::daemon::state::{
    ContainerRecord, ContainerState, DaemonState, JsonFileRepository, StateRepository,
};
use minibox_core::image::ImageStore;
use minibox_core::protocol::ContainerInfo;
use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_image_store(tmp: &TempDir) -> ImageStore {
    ImageStore::new(tmp.path().join("images")).expect("ImageStore::new")
}

fn make_container_record(id: &str, name: Option<&str>, state: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: name.map(|n| n.to_string()),
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: state.to_string(),
            created_at: "2026-04-11T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        rootfs_path: PathBuf::from("/tmp/rootfs"),
        cgroup_path: PathBuf::from("/tmp/cgroup"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: Some("alpine:latest".to_string()),
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None,
        manifest_path: None,
        workload_digest: None,
    }
}

// ---------------------------------------------------------------------------
// JsonFileRepository tests
// ---------------------------------------------------------------------------

/// conformance_json_repo_roundtrip: save_containers then load_containers
/// verifies data matches exactly.
#[test]
fn conformance_json_repo_roundtrip() {
    let tmp = TempDir::new().expect("TempDir::new");
    let repo = JsonFileRepository::new(tmp.path().join("state.json"));

    let mut original = HashMap::new();
    original.insert(
        "cont-1".to_string(),
        make_container_record("cont-1", Some("web"), "Running"),
    );
    original.insert(
        "cont-2".to_string(),
        make_container_record("cont-2", None, "Stopped"),
    );

    repo.save_containers(&original).expect("save_containers");
    let loaded = repo.load_containers().expect("load_containers");

    assert_eq!(loaded.len(), 2, "roundtrip must preserve container count");
    assert!(loaded.contains_key("cont-1"), "cont-1 must be loaded");
    assert!(loaded.contains_key("cont-2"), "cont-2 must be loaded");

    let cont1 = &loaded["cont-1"];
    assert_eq!(cont1.info.name, Some("web".to_string()), "name must match");
    assert_eq!(cont1.info.state, "Running", "state must match");
    assert_eq!(cont1.info.image, "alpine:latest", "image must match");

    let cont2 = &loaded["cont-2"];
    assert_eq!(
        cont2.info.name, None,
        "unnamed container must have None name"
    );
    assert_eq!(cont2.info.state, "Stopped", "state must match");
}

/// conformance_json_repo_missing_file_returns_empty: load from a nonexistent path
/// returns an empty HashMap without error.
#[test]
fn conformance_json_repo_missing_file_returns_empty() {
    let tmp = TempDir::new().expect("TempDir::new");
    let nonexistent_path = tmp.path().join("does-not-exist.json");

    let repo = JsonFileRepository::new(nonexistent_path);
    let containers = repo
        .load_containers()
        .expect("load_containers must not error");

    assert!(
        containers.is_empty(),
        "missing file must return empty HashMap, not error"
    );
}

/// conformance_json_repo_atomic_write: verify .json.tmp does NOT remain
/// after save_containers completes (atomic rename succeeded).
#[test]
fn conformance_json_repo_atomic_write() {
    let tmp = TempDir::new().expect("TempDir::new");
    let state_path = tmp.path().join("state.json");
    let repo = JsonFileRepository::new(state_path.clone());

    let mut data = HashMap::new();
    data.insert(
        "test".to_string(),
        make_container_record("test", None, "Created"),
    );

    repo.save_containers(&data).expect("save_containers");

    let tmp_path = state_path.with_extension("json.tmp");
    assert!(
        !tmp_path.exists(),
        "temporary .json.tmp file must NOT exist after save completes"
    );
    assert!(
        state_path.exists(),
        "final state.json file must exist after save completes"
    );
}

/// conformance_json_repo_overwrite_existing: save twice with different data,
/// second load returns second data.
#[test]
fn conformance_json_repo_overwrite_existing() {
    let tmp = TempDir::new().expect("TempDir::new");
    let repo = JsonFileRepository::new(tmp.path().join("state.json"));

    // First save: one container
    let mut first_data = HashMap::new();
    first_data.insert(
        "old".to_string(),
        make_container_record("old", None, "Stopped"),
    );
    repo.save_containers(&first_data)
        .expect("first save_containers");

    // Second save: different data (one new container, old one gone)
    let mut second_data = HashMap::new();
    second_data.insert(
        "new".to_string(),
        make_container_record("new", None, "Running"),
    );
    repo.save_containers(&second_data)
        .expect("second save_containers");

    let loaded = repo.load_containers().expect("load_containers");

    assert_eq!(loaded.len(), 1, "second save must overwrite, not append");
    assert!(
        loaded.contains_key("new"),
        "new container must be in second load"
    );
    assert!(
        !loaded.contains_key("old"),
        "old container must NOT be in second load"
    );
}

// ---------------------------------------------------------------------------
// DaemonState + StateRepository integration tests
// ---------------------------------------------------------------------------

/// conformance_daemon_state_add_remove_persists: add container, verify listed,
/// remove, verify gone.
#[tokio::test]
async fn conformance_daemon_state_add_remove_persists() {
    let tmp = TempDir::new().expect("TempDir::new");
    let state = DaemonState::new(make_image_store(&tmp), tmp.path());

    let record = make_container_record("test-id", Some("myapp"), "Created");

    // Add container
    state.add_container(record.clone()).await;

    // Verify it's listed
    let listed = state.list_containers().await;
    assert_eq!(listed.len(), 1, "container must be listed after add");
    assert_eq!(
        listed[0].id, "test-id",
        "listed container must have correct id"
    );
    assert_eq!(
        listed[0].name,
        Some("myapp".to_string()),
        "listed container must have correct name"
    );

    // Remove container
    let removed = state.remove_container("test-id").await;
    assert!(
        removed.is_some(),
        "remove_container must return the removed record"
    );
    assert_eq!(
        removed.unwrap().info.id,
        "test-id",
        "removed record must have correct id"
    );

    // Verify it's gone
    let listed_after = state.list_containers().await;
    assert!(
        listed_after.is_empty(),
        "container must be gone after remove"
    );
}

/// conformance_daemon_state_invalid_transition_errors: attempting an invalid
/// state transition returns an error.
#[tokio::test]
async fn conformance_daemon_state_invalid_transition_errors() {
    let tmp = TempDir::new().expect("TempDir::new");
    let state = DaemonState::new(make_image_store(&tmp), tmp.path());

    let record = make_container_record("test", None, "Stopped");
    state.add_container(record).await;

    // Try invalid transition: Stopped → Running
    let result = state
        .update_container_state("test", ContainerState::Running)
        .await;

    assert!(result.is_err(), "Stopped → Running must be invalid");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("invalid transition"),
        "error message must indicate invalid transition"
    );

    // Verify state unchanged
    let record = state.get_container("test").await.expect("get_container");
    assert_eq!(
        record.info.state, "Stopped",
        "state must not change after failed transition"
    );
}

/// conformance_daemon_state_valid_transitions: Created→Running, Running→Paused,
/// Paused→Running, Running→Stopped all succeed.
#[tokio::test]
async fn conformance_daemon_state_valid_transitions() {
    let tmp = TempDir::new().expect("TempDir::new");
    let state = DaemonState::new(make_image_store(&tmp), tmp.path());

    let record = make_container_record("test", None, "Created");
    state.add_container(record).await;

    // Created → Running
    let res = state
        .update_container_state("test", ContainerState::Running)
        .await;
    assert!(res.is_ok(), "Created → Running must be valid");
    let record = state.get_container("test").await.unwrap();
    assert_eq!(record.info.state, "Running", "state must be Running");

    // Running → Paused
    let res = state
        .update_container_state("test", ContainerState::Paused)
        .await;
    assert!(res.is_ok(), "Running → Paused must be valid");
    let record = state.get_container("test").await.unwrap();
    assert_eq!(record.info.state, "Paused", "state must be Paused");

    // Paused → Running
    let res = state
        .update_container_state("test", ContainerState::Running)
        .await;
    assert!(res.is_ok(), "Paused → Running must be valid");
    let record = state.get_container("test").await.unwrap();
    assert_eq!(record.info.state, "Running", "state must be Running");

    // Running → Stopped
    let res = state
        .update_container_state("test", ContainerState::Stopped)
        .await;
    assert!(res.is_ok(), "Running → Stopped must be valid");
    let record = state.get_container("test").await.unwrap();
    assert_eq!(record.info.state, "Stopped", "state must be Stopped");
    assert_eq!(record.info.pid, None, "pid must be cleared on Stopped");
    assert_eq!(record.pid, None, "record.pid must be cleared on Stopped");
}

/// conformance_daemon_state_resolve_id_by_name: add named container,
/// resolve by name returns correct id.
#[tokio::test]
async fn conformance_daemon_state_resolve_id_by_name() {
    let tmp = TempDir::new().expect("TempDir::new");
    let state = DaemonState::new(make_image_store(&tmp), tmp.path());

    let record1 = make_container_record("id-1", Some("api"), "Created");
    let record2 = make_container_record("id-2", Some("db"), "Created");

    state.add_container(record1).await;
    state.add_container(record2).await;

    // Resolve by exact ID
    let resolved = state.resolve_id("id-1").await;
    assert_eq!(resolved, Some("id-1".to_string()), "exact ID match");

    // Resolve by name
    let resolved = state.resolve_id("api").await;
    assert_eq!(
        resolved,
        Some("id-1".to_string()),
        "name match must return id-1"
    );

    let resolved = state.resolve_id("db").await;
    assert_eq!(
        resolved,
        Some("id-2".to_string()),
        "name match must return id-2"
    );

    // Nonexistent
    let resolved = state.resolve_id("nonexistent").await;
    assert_eq!(resolved, None, "nonexistent name/id must return None");
}

/// conformance_daemon_state_name_collision: verify name_in_use returns true
/// for existing name.
#[tokio::test]
async fn conformance_daemon_state_name_collision() {
    let tmp = TempDir::new().expect("TempDir::new");
    let state = DaemonState::new(make_image_store(&tmp), tmp.path());

    let record = make_container_record("test", Some("myname"), "Created");
    state.add_container(record).await;

    // Check that name is in use
    let in_use = state.name_in_use("myname").await;
    assert!(in_use, "existing name must be reported as in use");

    // Check that different name is not in use
    let in_use = state.name_in_use("othername").await;
    assert!(!in_use, "nonexistent name must not be in use");

    // Unnamed container
    let record_unnamed = make_container_record("test2", None, "Created");
    state.add_container(record_unnamed).await;

    let in_use = state.name_in_use("").await;
    assert!(!in_use, "empty string must not match unnamed containers");
}
