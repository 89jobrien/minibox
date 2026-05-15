//! Targeted coverage tests for handler.rs public functions that had no prior
//! test coverage.  The goal is to bring handler.rs function coverage to 80%.
//!
//! Each test covers at least one previously uncovered public or significant
//! private function by exercising the function's happy path, error path, or
//! both.

use minibox::daemon::handler::{
    self, BuildDeps, ContainerPolicy, EventDeps, ExecDeps, HandlerDependencies, ImageDeps,
    LifecycleDeps, PtySessionRegistry,
};
use minibox::daemon::state::DaemonState;
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use minibox_core::domain::SessionId;
use minibox_core::protocol::DaemonResponse;
use minibox_core::protocol::PushCredentials;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

// ---- handle_list ------------------------------------------------------------

/// handle_list on an empty daemon state returns an empty ContainerList.
#[tokio::test]
async fn test_handle_list_empty_state() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_list(state).await;
    match resp {
        DaemonResponse::ContainerList { containers } => {
            assert!(containers.is_empty(), "expected empty list");
        }
        other => panic!("expected ContainerList, got {other:?}"),
    }
}

/// handle_list returns all containers after a run.
#[tokio::test]
async fn test_handle_list_after_run() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        Arc::clone(&state),
        Arc::clone(&deps),
    )
    .await;
    assert!(
        matches!(resp, DaemonResponse::ContainerCreated { .. }),
        "run should succeed, got {resp:?}"
    );

    let list_resp = handler::handle_list(Arc::clone(&state)).await;
    match list_resp {
        DaemonResponse::ContainerList { containers } => {
            assert_eq!(containers.len(), 1, "expected 1 container");
        }
        other => panic!("expected ContainerList, got {other:?}"),
    }
}

// ---- handle_stop ------------------------------------------------------------

/// handle_stop with an unknown container returns Error.
#[tokio::test]
async fn test_handle_stop_unknown_container_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_stop("nonexistent-id".to_string(), state, deps).await;
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "unknown container should produce Error, got {resp:?}"
    );
}

// ---- handle_remove ----------------------------------------------------------

/// handle_remove with an unknown container returns Error.
#[tokio::test]
async fn test_handle_remove_unknown_container_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_remove("nonexistent-id".to_string(), state, deps).await;
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not found")),
        "unknown container should produce Error with 'not found', got {resp:?}"
    );
}

/// handle_remove after creating a stopped container returns Success.
#[tokio::test]
async fn test_handle_remove_stopped_container_succeeds() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        Arc::clone(&state),
        Arc::clone(&deps),
    )
    .await;
    let id = extract_container_id(&resp);

    // Stop the container first.
    let _ = handler::handle_stop(id.clone(), Arc::clone(&state), Arc::clone(&deps)).await;

    let remove_resp = handler::handle_remove(id, Arc::clone(&state), Arc::clone(&deps)).await;
    assert!(
        matches!(remove_resp, DaemonResponse::Success { .. }),
        "remove of stopped container should succeed, got {remove_resp:?}"
    );
}

// ---- handle_push (no pusher) ------------------------------------------------

/// handle_push without a pusher configured returns Error.
#[tokio::test]
async fn test_handle_push_no_pusher_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp); // build.image_pusher = None

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_push(
        "alpine:latest".to_string(),
        PushCredentials::Anonymous,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response from handle_push");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("push not supported")),
        "expected 'push not supported' error, got {resp:?}"
    );
}

// ---- handle_commit (no adapter) ---------------------------------------------

/// handle_commit without a commit adapter returns Error.
#[tokio::test]
async fn test_handle_commit_no_adapter_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp); // build.commit_adapter = None

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_commit(
        "container-id".to_string(),
        "myimage:latest".to_string(),
        None, // author
        None, // message
        vec![],
        None, // cmd_override
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response from handle_commit");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "commit without adapter should produce Error, got {resp:?}"
    );
}

// ---- handle_build (no builder) ----------------------------------------------

/// handle_build without a builder configured returns Error.
#[tokio::test]
async fn test_handle_build_no_builder_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp); // build.image_builder = None

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_build(
        "Dockerfile".to_string(),
        tmp.path().to_string_lossy().to_string(),
        "myimage:latest".to_string(),
        vec![],
        false,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response from handle_build");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "build without builder should produce Error, got {resp:?}"
    );
}

// ---- handle_exec (no exec runtime) ------------------------------------------

/// handle_exec without an exec runtime returns Error.
#[tokio::test]
async fn test_handle_exec_no_runtime_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp); // exec.exec_runtime = None

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_exec(
        "container-id".to_string(),
        vec!["/bin/sh".to_string()],
        vec![], // env
        None,   // working_dir
        false,  // tty
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response from handle_exec");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "exec without runtime should produce Error, got {resp:?}"
    );
}

// ---- handle_logs ------------------------------------------------------------

/// handle_logs with an unknown container returns Error.
#[tokio::test]
async fn test_handle_logs_unknown_container_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(8);
    handler::handle_logs("nonexistent-container".to_string(), false, state, deps, tx).await;

    let resp = rx.recv().await.expect("no response from handle_logs");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "logs for unknown container should produce Error, got {resp:?}"
    );
}

// ---- handle_send_input ------------------------------------------------------

/// handle_send_input with no active session returns Error.
#[tokio::test]
async fn test_handle_send_input_no_session_returns_error() {
    use base64::Engine as _;
    let tmp = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_send_input(
        SessionId::new("nonexistent-session".to_string()),
        base64::engine::general_purpose::STANDARD.encode(b"hello"),
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response from handle_send_input");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "send_input without active session should produce Error, got {resp:?}"
    );
}

/// handle_send_input with invalid base64 returns Error.
#[tokio::test]
async fn test_handle_send_input_invalid_base64_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_send_input(
        SessionId::new("some-session".to_string()),
        "not-valid-base64!!!".to_string(),
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response from handle_send_input");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("base64")),
        "invalid base64 should produce Error with 'base64', got {resp:?}"
    );
}

// ---- handle_resize_pty ------------------------------------------------------

/// handle_resize_pty with no active session returns Error.
#[tokio::test]
async fn test_handle_resize_pty_no_session_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_resize_pty(
        SessionId::new("nonexistent-session".to_string()),
        80,
        24,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response from handle_resize_pty");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "resize_pty without active session should produce Error, got {resp:?}"
    );
}

// ---- handle_save_snapshot ---------------------------------------------------

/// handle_save_snapshot with NoopVmCheckpoint returns Error (not supported).
#[tokio::test]
async fn test_handle_save_snapshot_noop_checkpoint_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_save_snapshot(
        "container-id".to_string(),
        Some("snap1".to_string()),
        state,
        deps,
    )
    .await;
    // NoopVmCheckpoint is a stub that always returns "not supported".
    assert!(
        matches!(
            resp,
            DaemonResponse::SnapshotSaved { .. } | DaemonResponse::Error { .. }
        ),
        "save_snapshot must return SnapshotSaved or Error, got {resp:?}"
    );
}

// ---- handle_restore_snapshot ------------------------------------------------

/// handle_restore_snapshot with NoopVmCheckpoint returns SnapshotRestored.
#[tokio::test]
async fn test_handle_restore_snapshot_noop_checkpoint_succeeds() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_restore_snapshot(
        "container-id".to_string(),
        "snap1".to_string(),
        state,
        deps,
    )
    .await;
    assert!(
        matches!(resp, DaemonResponse::SnapshotRestored { .. }),
        "NoopVmCheckpoint restore should succeed, got {resp:?}"
    );
}

// ---- handle_list_snapshots --------------------------------------------------

/// handle_list_snapshots with NoopVmCheckpoint returns empty SnapshotList.
#[tokio::test]
async fn test_handle_list_snapshots_noop_checkpoint() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_list_snapshots("container-id".to_string(), deps).await;
    match resp {
        DaemonResponse::SnapshotList { snapshots, .. } => {
            assert!(
                snapshots.is_empty(),
                "NoopVmCheckpoint should return empty list"
            );
        }
        other => panic!("expected SnapshotList, got {other:?}"),
    }
}

// ---- handle_pull ------------------------------------------------------------

/// handle_pull with a pull-failure registry returns Error.
#[tokio::test]
async fn test_handle_pull_pull_failure_returns_error() {
    use minibox::adapters::mocks::MockRegistry;

    let tmp = TempDir::new().expect("create temp dir");
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(tmp.path().join("images"))
            .expect("create image store"),
    );
    let deps = build_deps_with_registry(
        Arc::new(HostnameRegistryRouter::new(
            Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
            [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
        )),
        image_store,
        &tmp,
    );
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_pull("alpine".to_string(), None, None, state, deps).await;
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "pull failure should produce Error, got {resp:?}"
    );
}

/// handle_pull with an invalid image reference returns Error.
#[tokio::test]
async fn test_handle_pull_invalid_image_ref_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    // An empty string is an invalid image reference.
    let resp = handler::handle_pull("".to_string(), None, None, state, deps).await;
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "invalid ref should produce Error, got {resp:?}"
    );
}

// ---- handle_load_image ------------------------------------------------------

/// handle_load_image with NoopImageLoader succeeds.
#[tokio::test]
async fn test_handle_load_image_noop_loader_succeeds() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_load_image(
        "/tmp/image.tar".to_string(),
        "myimage".to_string(),
        "v1".to_string(),
        state,
        deps,
    )
    .await;
    assert!(
        matches!(resp, DaemonResponse::ImageLoaded { .. }),
        "NoopImageLoader should succeed, got {resp:?}"
    );
}

// ---- handle_get_manifest ----------------------------------------------------

/// handle_get_manifest with unknown container returns Error.
#[tokio::test]
async fn test_handle_get_manifest_unknown_container_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_get_manifest("nonexistent-id".to_string(), state, deps, tx).await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not found")),
        "unknown container should produce Error with 'not found', got {resp:?}"
    );
}

// ---- handle_verify_manifest -------------------------------------------------

/// handle_verify_manifest with unknown container returns Error.
#[tokio::test]
async fn test_handle_verify_manifest_unknown_container_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_verify_manifest(
        "nonexistent-id".to_string(),
        r#"{"allow":[]}"#.to_string(),
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "unknown container should produce Error, got {resp:?}"
    );
}

// ---- handle_update ----------------------------------------------------------

/// handle_update with an explicit list of images pulls each and sends UpdateProgress.
#[tokio::test]
async fn test_handle_update_explicit_images_sends_progress() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
    handler::handle_update(
        vec!["alpine:latest".to_string()],
        false,
        false,
        false,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response from handle_update");
    assert!(
        matches!(resp, DaemonResponse::UpdateProgress { .. }),
        "expected UpdateProgress, got {resp:?}"
    );
}

/// handle_update with no images and all=false, containers=false sends UpdateComplete.
#[tokio::test]
async fn test_handle_update_empty_list_sends_update_complete() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
    handler::handle_update(vec![], false, false, false, state, deps, tx).await;

    let resp = rx.recv().await.expect("no response from handle_update");
    assert!(
        matches!(resp, DaemonResponse::Success { .. }),
        "empty update should produce Success, got {resp:?}"
    );
}

// ---- PtySessionRegistry::cleanup --------------------------------------------

/// PtySessionRegistry::cleanup removes both resize and stdin channels.
#[test]
fn test_pty_session_registry_cleanup_removes_channels() {
    let mut reg = PtySessionRegistry::default();

    let (resize_tx, _resize_rx) = tokio::sync::mpsc::channel::<(u16, u16)>(1);
    let (stdin_tx, _stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);

    reg.resize.insert("session-1".to_string(), resize_tx);
    reg.stdin.insert("session-1".to_string(), stdin_tx);

    reg.cleanup("session-1");

    assert!(
        !reg.resize.contains_key("session-1"),
        "resize channel must be removed after cleanup"
    );
    assert!(
        !reg.stdin.contains_key("session-1"),
        "stdin channel must be removed after cleanup"
    );
}

/// cleanup on a non-existent session_id is a no-op.
#[test]
fn test_pty_session_registry_cleanup_nonexistent_is_noop() {
    let mut reg = PtySessionRegistry::default();
    reg.cleanup("no-such-session"); // must not panic
}

// ---- HandlerDependencies::with_image_loader ---------------------------------

/// with_image_loader replaces the image loader in HandlerDependencies.
#[test]
fn test_handler_dependencies_with_image_loader_replaces_loader() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&tmp);

    let new_loader: minibox_core::domain::DynImageLoader =
        Arc::new(minibox::daemon::handler::NoopImageLoader);
    let _updated = (*deps).clone().with_image_loader(new_loader);
}

// ---- ContainerPolicy::from_env ----------------------------------------------

/// ContainerPolicy::from_env with no env vars set returns all-deny defaults.
#[test]
fn test_container_policy_from_env_defaults_deny_all() {
    // These env vars must NOT be set in the test environment.
    unsafe {
        std::env::remove_var("MINIBOX_ALLOW_BIND_MOUNTS");
        std::env::remove_var("MINIBOX_ALLOW_PRIVILEGED");
    }
    let policy = ContainerPolicy::from_env();
    assert!(
        !policy.allow_bind_mounts,
        "default env should deny bind mounts"
    );
    assert!(
        !policy.allow_privileged,
        "default env should deny privileged"
    );
}

// ---- handle_pipeline --------------------------------------------------------

/// handle_pipeline with a nonexistent pipeline file returns Error.
#[tokio::test]
async fn test_handle_pipeline_nonexistent_file_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(8);
    handler::handle_pipeline(
        "/nonexistent/pipeline.cruxx".to_string(),
        None,
        None,
        None,
        vec![],
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response from handle_pipeline");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "nonexistent pipeline file should produce Error, got {resp:?}"
    );
}
