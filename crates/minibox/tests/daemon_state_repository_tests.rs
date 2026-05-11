//! Tests for the StateRepository port and JsonFileRepository adapter.
//!
//! Verifies that `DaemonState` can be constructed with a `StateRepository`
//! dependency, and that `JsonFileRepository` implements it correctly.

use minibox::daemon::state::{DaemonState, JsonFileRepository, StateRepository};
use minibox_core::image::ImageStore;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

fn make_image_store(tmp: &TempDir) -> ImageStore {
    ImageStore::new(tmp.path().join("images")).expect("ImageStore::new")
}

/// `JsonFileRepository::load_containers` on a missing file returns an empty map.
#[test]
fn json_file_repository_load_missing_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let repo = JsonFileRepository::new(tmp.path().join("state.json"));
    let containers = repo.load_containers().expect("load must succeed");
    assert!(containers.is_empty(), "empty file should yield empty map");
}

/// `JsonFileRepository::save_containers` then `load_containers` round-trips.
#[tokio::test]
async fn json_file_repository_round_trips() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;
    use std::path::PathBuf;

    let tmp = TempDir::new().unwrap();
    let repo = JsonFileRepository::new(tmp.path().join("state.json"));

    let mut map = HashMap::new();
    map.insert(
        "test-id".to_string(),
        ContainerRecord {
            info: ContainerInfo {
                id: "test-id".to_string(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Stopped".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
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
        },
    );

    repo.save_containers(&map).expect("save must succeed");
    let loaded = repo.load_containers().expect("load must succeed");
    assert!(
        loaded.contains_key("test-id"),
        "saved container must round-trip"
    );
}

/// `DaemonState` can be constructed with an `Arc<dyn StateRepository>`.
#[tokio::test]
async fn daemon_state_accepts_repository_port() {
    let tmp = TempDir::new().unwrap();
    let image_store = make_image_store(&tmp);
    let repo: Arc<dyn StateRepository> =
        Arc::new(JsonFileRepository::new(tmp.path().join("state.json")));

    // This will fail to compile until DaemonState::with_repository exists.
    let state = DaemonState::with_repository(image_store, repo);
    let containers = state.list_containers().await;
    assert!(containers.is_empty());
}
