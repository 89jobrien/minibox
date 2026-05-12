//! Daemon state persistence and creation_params tests.

use minibox::daemon::state::DaemonState;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

/// add_container triggers save_to_disk; a new DaemonState loaded from the same
/// directory via load_from_disk must contain the same container record.
///
/// Guards the persistence contract documented in docs/STATE_MODEL.md.
#[tokio::test]
async fn test_daemon_state_persistence_survives_restart() {
    let tmp = TempDir::new().unwrap();

    let container_id = "persist-test-00001a";
    {
        let image_store = minibox_core::image::ImageStore::new(tmp.path().join("images")).unwrap();
        let state = DaemonState::new(image_store, tmp.path());
        let record = minibox::daemon::state::ContainerRecord {
            info: minibox_core::protocol::ContainerInfo {
                id: container_id.to_string(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Stopped".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                pid: None,
            },
            pid: None,
            rootfs_path: std::path::PathBuf::from("/tmp/fake-rootfs"),
            cgroup_path: std::path::PathBuf::from("/tmp/fake-cgroup"),
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
        };
        state.add_container(record).await;
    }

    let image_store2 = minibox_core::image::ImageStore::new(tmp.path().join("images2")).unwrap();
    let state2 = DaemonState::new(image_store2, tmp.path());
    state2.load_from_disk().await;

    let container = state2.get_container(container_id).await;
    assert!(
        container.is_some(),
        "container record must survive daemon restart"
    );
    assert_eq!(
        container.unwrap().info.id,
        container_id,
        "restored container must have the same id"
    );
}

/// remove_container triggers save_to_disk; a new DaemonState loaded from the
/// same directory must not contain the removed record.
#[tokio::test]
async fn test_daemon_state_remove_persists_to_disk() {
    let tmp = TempDir::new().unwrap();

    let container_id = "remove-persist-0001";
    {
        let image_store = minibox_core::image::ImageStore::new(tmp.path().join("images")).unwrap();
        let state = DaemonState::new(image_store, tmp.path());
        let record = minibox::daemon::state::ContainerRecord {
            info: minibox_core::protocol::ContainerInfo {
                id: container_id.to_string(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Stopped".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                pid: None,
            },
            pid: None,
            rootfs_path: std::path::PathBuf::from("/tmp/fake-rootfs"),
            cgroup_path: std::path::PathBuf::from("/tmp/fake-cgroup"),
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
        };
        state.add_container(record).await;
        state.remove_container(container_id).await;
    }

    let image_store2 = minibox_core::image::ImageStore::new(tmp.path().join("images2")).unwrap();
    let state2 = DaemonState::new(image_store2, tmp.path());
    state2.load_from_disk().await;

    assert!(
        state2.get_container(container_id).await.is_none(),
        "removed container must not appear after restart"
    );
}

// ---------------------------------------------------------------------------
// Name-resolution and additional error-path coverage (#158)

// ---------------------------------------------------------------------------
// restart-3: creation_params population
// ---------------------------------------------------------------------------

#[tokio::test]
async fn handle_run_stores_creation_params() {
    let temp_dir = tempfile::TempDir::new().expect("TempDir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/echo".to_string(), "hello".to_string()],
        Some(67_108_864),
        Some(256),
        false,
        state.clone(),
        deps,
    )
    .await;

    match response {
        minibox_core::protocol::DaemonResponse::ContainerCreated { id, .. } => {
            let record = state
                .get_container(&id)
                .await
                .expect("container must be in state");
            let cp = record
                .creation_params
                .expect("creation_params must be populated by handle_run");
            assert_eq!(cp.image, "alpine");
            assert_eq!(cp.tag.as_deref(), Some("latest"));
            assert_eq!(cp.command, vec!["/bin/echo", "hello"]);
            assert_eq!(cp.memory_limit_bytes, Some(67_108_864));
            assert_eq!(cp.cpu_weight, Some(256));
        }
        other => panic!("expected ContainerCreated, got {other:?}"),
    }
}
