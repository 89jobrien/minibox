//! Conformance tests for `DaemonState` persistence contract.
//!
//! Verifies:
//! - `add_container` followed by `get_container` round-trips.
//! - `remove_container` returns the removed record.
//! - `remove_container` on a non-existent id returns None.
//! - `list_containers` returns all added containers.
//! - `save_to_disk` + `load_from_disk` round-trips (persistence).
//! - `update_container_state` changes status correctly.
//! - `resolve_id` matches by full id and name.
//! - `name_in_use` detects name collisions.
//!
//! Uses temporary directories — no shared state between tests.

use minibox::daemon::state::{ContainerRecord, DaemonState};
use minibox_core::domain::ContainerState;
use minibox_core::image::ImageStore;
use minibox_core::protocol::ContainerInfo;
use std::path::PathBuf;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_state(tmp: &TempDir) -> DaemonState {
    let image_store = ImageStore::new(tmp.path().join("images")).expect("ImageStore::new");
    DaemonState::new(image_store, tmp.path())
}

fn make_record(id: &str, name: Option<&str>, image: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: name.map(|s| s.to_string()),
            image: image.to_string(),
            command: "/bin/sh".to_string(),
            state: "Created".to_string(),
            created_at: "2026-04-27T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        rootfs_path: PathBuf::from("/tmp/rootfs"),
        cgroup_path: PathBuf::from("/tmp/cgroup"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: None,
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
// add + get round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_add_then_get_round_trips() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    let record = make_record("aabbccdd11223344", None, "alpine:latest");
    state.add_container(record).await;

    let retrieved = state.get_container("aabbccdd11223344").await;
    assert!(retrieved.is_some(), "get must return the added container");
    assert_eq!(retrieved.unwrap().info.image, "alpine:latest");
}

// ---------------------------------------------------------------------------
// remove
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_remove_returns_record() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    state
        .add_container(make_record("remove01abcdef12", None, "nginx"))
        .await;

    let removed = state.remove_container("remove01abcdef12").await;
    assert!(removed.is_some(), "remove must return the record");
    assert_eq!(removed.unwrap().info.image, "nginx");

    let gone = state.get_container("remove01abcdef12").await;
    assert!(gone.is_none(), "container must be gone after remove");
}

#[tokio::test]
async fn state_remove_nonexistent_returns_none() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    let result = state.remove_container("doesnotexist1234").await;
    assert!(result.is_none());
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_list_returns_all_containers() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    state
        .add_container(make_record("list01abcdef1234", None, "alpine"))
        .await;
    state
        .add_container(make_record("list02abcdef1234", None, "ubuntu"))
        .await;
    state
        .add_container(make_record("list03abcdef1234", None, "nginx"))
        .await;

    let list = state.list_containers().await;
    assert_eq!(list.len(), 3, "list must return all 3 containers");
}

#[tokio::test]
async fn state_list_empty_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    let list = state.list_containers().await;
    assert!(list.is_empty());
}

// ---------------------------------------------------------------------------
// Persistence: save + load round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_save_load_round_trips() {
    let tmp = TempDir::new().unwrap();

    // Create state, add containers, state auto-saves on add.
    {
        let state = make_state(&tmp);
        state
            .add_container(make_record("persist01abcdef1", None, "alpine"))
            .await;
        state
            .add_container(make_record("persist02abcdef1", Some("web"), "nginx"))
            .await;
    }

    // Create a new state from the same directory — load_from_disk.
    {
        let state = make_state(&tmp);
        state.load_from_disk().await;

        let c1 = state.get_container("persist01abcdef1").await;
        assert!(c1.is_some(), "container 1 must survive save/load");
        assert_eq!(c1.unwrap().info.image, "alpine");

        let c2 = state.get_container("persist02abcdef1").await;
        assert!(c2.is_some(), "container 2 must survive save/load");
        assert_eq!(c2.unwrap().info.name.as_deref(), Some("web"));
    }
}

// ---------------------------------------------------------------------------
// update_container_state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_update_status() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    state
        .add_container(make_record("update01abcdef12", None, "alpine"))
        .await;

    state
        .update_container_state("update01abcdef12", ContainerState::Running)
        .await
        .expect("update must succeed for existing container");

    let record = state.get_container("update01abcdef12").await.unwrap();
    assert_eq!(record.info.state, "Running");
}

#[tokio::test]
async fn state_update_nonexistent_does_not_panic() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    // Must return Err for nonexistent container.
    let result = state
        .update_container_state("ghost123abcdef12", ContainerState::Stopped)
        .await;
    assert!(result.is_err(), "update on nonexistent id must return Err");
}

// ---------------------------------------------------------------------------
// resolve_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_resolve_full_id() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    state
        .add_container(make_record("resolve1abcdef12", None, "alpine"))
        .await;

    let resolved = state.resolve_id("resolve1abcdef12").await;
    assert_eq!(resolved.as_deref(), Some("resolve1abcdef12"));
}

#[tokio::test]
async fn state_resolve_by_name() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    state
        .add_container(make_record("named01abcdefgh1", Some("myapp"), "alpine"))
        .await;

    let resolved = state.resolve_id("myapp").await;
    assert_eq!(resolved.as_deref(), Some("named01abcdefgh1"));
}

#[tokio::test]
async fn state_resolve_unknown_returns_none() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    let resolved = state.resolve_id("nope").await;
    assert!(resolved.is_none());
}

// ---------------------------------------------------------------------------
// name_in_use
// ---------------------------------------------------------------------------

#[tokio::test]
async fn state_name_in_use_detects_collision() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    state
        .add_container(make_record("namecol1abcdef12", Some("web"), "nginx"))
        .await;

    assert!(state.name_in_use("web").await);
    assert!(!state.name_in_use("api").await);
}
