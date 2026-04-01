//! Tests for DaemonState resilience and handler error recovery.
//!
//! Verifies that state operations are safe under failure conditions,
//! concurrent access, and invalid inputs.

use daemonbox::handler;
use daemonbox::state::{ContainerRecord, DaemonState};
use mbx::adapters::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime};
use minibox_core::protocol::{ContainerInfo, DaemonResponse};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_state(temp_dir: &TempDir) -> Arc<DaemonState> {
    let image_store = mbx::image::ImageStore::new(temp_dir.path().join("images")).unwrap();
    Arc::new(DaemonState::new(image_store, temp_dir.path()))
}

fn make_deps(temp_dir: &TempDir) -> Arc<daemonbox::handler::HandlerDependencies> {
    Arc::new(daemonbox::handler::HandlerDependencies {
        registry: Arc::new(MockRegistry::new()),
        ghcr_registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: Arc::new(MockNetwork::new()),
        containers_base: temp_dir.path().join("containers"),
        run_containers_base: temp_dir.path().join("run"),
        metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
        exec_runtime: None,
        image_pusher: None,
        commit_adapter: None,
        image_builder: None,
    })
}

fn make_record(id: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Created".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        rootfs_path: PathBuf::from("/tmp/fake-rootfs"),
        cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/fake"),
        post_exit_hooks: vec![],
    }
}

// ---------------------------------------------------------------------------
// DaemonState resilience tests
// ---------------------------------------------------------------------------

/// Concurrent add/remove operations must not corrupt state or panic.
#[tokio::test]
async fn test_concurrent_add_remove_no_corruption() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let mut handles = vec![];

    // Spawn 10 tasks, each adding and removing a unique container.
    for i in 0..10u32 {
        let state_clone = Arc::clone(&state);
        let id = format!("concurrent-container-{i:04}");
        let handle = tokio::spawn(async move {
            let record = make_record(&id);
            state_clone.add_container(record).await;
            state_clone.remove_container(&id).await;
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.expect("task must not panic");
    }

    // After all tasks complete, state should be empty.
    let containers = state.list_containers().await;
    assert!(
        containers.is_empty(),
        "expected empty state after all removes, got {} containers",
        containers.len()
    );
}

/// `remove_container` on a non-existent ID returns `None` without panicking.
#[tokio::test]
async fn test_remove_nonexistent_returns_none() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let result = state.remove_container("nonexistent-id").await;
    assert!(
        result.is_none(),
        "expected None when removing non-existent container"
    );
}

/// Removing the same container twice: first returns `Some`, second returns `None`.
#[tokio::test]
async fn test_double_remove_second_is_none() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let id = "double-remove-test";
    state.add_container(make_record(id)).await;

    let first = state.remove_container(id).await;
    assert!(first.is_some(), "first remove should return the record");

    let second = state.remove_container(id).await;
    assert!(
        second.is_none(),
        "second remove of same ID should return None"
    );
}

/// After a mix of successful and failed removes, `list_containers` only contains
/// containers that were actually added and not removed.
#[tokio::test]
async fn test_list_consistent_after_partial_removes() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let ids = ["keep-1", "keep-2", "remove-1", "remove-2"];
    for id in &ids {
        state.add_container(make_record(id)).await;
    }

    // Remove two containers, attempt to remove a phantom (should be silent).
    state.remove_container("remove-1").await;
    state.remove_container("remove-2").await;
    state.remove_container("never-existed").await; // should not panic

    let mut remaining: Vec<String> = state
        .list_containers()
        .await
        .into_iter()
        .map(|c| c.id)
        .collect();
    remaining.sort();

    assert_eq!(
        remaining,
        vec!["keep-1".to_string(), "keep-2".to_string()],
        "list should contain exactly the two containers that were not removed"
    );
}

/// `load_from_disk` with no state file present succeeds and yields empty state.
#[tokio::test]
async fn test_load_from_disk_missing_file_is_empty() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    // No state.json exists — load_from_disk must not panic or error.
    state.load_from_disk().await;

    let containers = state.list_containers().await;
    assert!(
        containers.is_empty(),
        "expected empty state when no state file exists"
    );
}

/// `load_from_disk` with a corrupt/empty JSON file succeeds and yields empty state.
#[tokio::test]
async fn test_load_from_disk_corrupt_file_is_empty() {
    let temp_dir = TempDir::new().unwrap();
    // Write garbage into the state file before creating DaemonState.
    std::fs::write(temp_dir.path().join("state.json"), b"not valid json at all").unwrap();

    let state = make_state(&temp_dir);
    state.load_from_disk().await;

    let containers = state.list_containers().await;
    assert!(
        containers.is_empty(),
        "corrupt state file should result in empty state, not a panic"
    );
}

// ---------------------------------------------------------------------------
// Handler error recovery tests
// ---------------------------------------------------------------------------

/// `handle_stop` with an empty string ID returns an Error response.
#[tokio::test]
async fn test_handle_stop_empty_id_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let deps = make_deps(&temp_dir);
    let response = handler::handle_stop("".to_string(), state, deps).await;

    match response {
        DaemonResponse::Error { .. } => {} // expected
        other => panic!("expected Error response for empty ID, got {other:?}"),
    }
}

/// `handle_remove` with an empty string ID returns an Error response.
#[tokio::test]
async fn test_handle_remove_empty_id_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);
    let deps = make_deps(&temp_dir);

    let response = handler::handle_remove("".to_string(), state, deps).await;

    match response {
        DaemonResponse::Error { .. } => {} // expected
        other => panic!("expected Error response for empty ID, got {other:?}"),
    }
}

/// `handle_list` on a fresh DaemonState returns an empty ContainerList.
#[tokio::test]
async fn test_handle_list_fresh_state_is_empty() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    let response = handler::handle_list(state).await;

    match response {
        DaemonResponse::ContainerList { containers } => {
            assert!(
                containers.is_empty(),
                "fresh state should yield an empty list"
            );
        }
        other => panic!("expected ContainerList, got {other:?}"),
    }
}

/// `handle_list` is idempotent: consecutive calls return the same contents.
#[tokio::test]
async fn test_handle_list_idempotent() {
    let temp_dir = TempDir::new().unwrap();
    let state = make_state(&temp_dir);

    // Seed some containers directly.
    for i in 0..3u32 {
        state
            .add_container(make_record(&format!("list-idempotent-{i}")))
            .await;
    }

    let first = match handler::handle_list(Arc::clone(&state)).await {
        DaemonResponse::ContainerList { containers } => containers,
        other => panic!("expected ContainerList, got {other:?}"),
    };

    let second = match handler::handle_list(Arc::clone(&state)).await {
        DaemonResponse::ContainerList { containers } => containers,
        other => panic!("expected ContainerList, got {other:?}"),
    };

    assert_eq!(
        first.len(),
        second.len(),
        "consecutive list calls should return the same count"
    );

    // Both calls must return the same set of IDs (order may differ).
    let mut ids_first: Vec<String> = first.into_iter().map(|c| c.id).collect();
    let mut ids_second: Vec<String> = second.into_iter().map(|c| c.id).collect();
    ids_first.sort();
    ids_second.sort();
    assert_eq!(
        ids_first, ids_second,
        "consecutive list calls must return identical container IDs"
    );
}
