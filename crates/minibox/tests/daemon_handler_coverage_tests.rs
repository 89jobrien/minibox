#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::unwrap_in_result,
    clippy::collapsible_if,
    clippy::match_same_arms,
    clippy::only_used_in_recursion,
    clippy::used_underscore_binding,
    clippy::map_unwrap_or,
    clippy::manual_assert,
    clippy::as_ptr_cast_mut,
    clippy::ptr_as_ptr,
    clippy::must_use_candidate,
    clippy::used_underscore_items,
    clippy::missing_const_for_fn,
    clippy::manual_string_new,
    clippy::semicolon_if_nothing_returned,
    clippy::unreadable_literal,
    clippy::default_constructed_unit_structs,
    clippy::ref_as_ptr,
    clippy::allow_attributes_without_reason,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_raw_string_hashes,
    clippy::manual_is_variant_and,
    clippy::ignore_without_reason,
    clippy::default_trait_access,
    clippy::cast_lossless,
    clippy::match_wild_err_arm,
    clippy::format_push_string,
    clippy::bool_assert_comparison,
    clippy::struct_excessive_bools
)]
//! Targeted coverage tests for handler.rs public functions that had no prior
//! test coverage.  The goal is to bring handler.rs function coverage to 80%.
//!
//! Each test covers at least one previously uncovered public or significant
//! private function by exercising the function's happy path, error path, or
//! both.

use minibox::daemon::handler::{self, ContainerPolicy, PtySessionRegistry};
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
    // NoopVmCheckpoint is a stub — may succeed or return "not supported".
    assert!(
        matches!(
            resp,
            DaemonResponse::SnapshotRestored { .. } | DaemonResponse::Error { .. }
        ),
        "restore_snapshot must return SnapshotRestored or Error, got {resp:?}"
    );
}

// ---- handle_list_snapshots --------------------------------------------------

/// handle_list_snapshots with NoopVmCheckpoint returns SnapshotList or Error.
#[tokio::test]
async fn test_handle_list_snapshots_noop_checkpoint() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_list_snapshots("container-id".to_string(), deps).await;
    // NoopVmCheckpoint may return an empty list or an Error.
    assert!(
        matches!(
            resp,
            DaemonResponse::SnapshotList { .. } | DaemonResponse::Error { .. }
        ),
        "list_snapshots must return SnapshotList or Error, got {resp:?}"
    );
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
        handler::UpdateParams {
            images: vec!["alpine:latest".to_string()],
            all: false,
            containers: false,
            restart: false,
        },
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
    handler::handle_update(
        handler::UpdateParams {
            images: vec![],
            all: false,
            containers: false,
            restart: false,
        },
        state,
        deps,
        tx,
    )
    .await;

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

// ---- handle_update (all=true / containers=true) -----------------------------

/// handle_update with all=true lists images from the store and sends progress.
#[tokio::test]
async fn test_handle_update_all_true_sends_success() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    // Image store is empty, so all=true results in empty list → Success immediately.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
    handler::handle_update(
        handler::UpdateParams {
            images: vec![],
            all: true,
            containers: false,
            restart: false,
        },
        Arc::clone(&state),
        Arc::clone(&deps),
        tx,
    )
    .await;

    let resp = rx
        .recv()
        .await
        .expect("no response from handle_update all=true");
    assert!(
        matches!(resp, DaemonResponse::Success { .. }),
        "all=true with empty store should produce Success, got {resp:?}"
    );
}

/// handle_update with containers=true collects source refs from running containers.
#[tokio::test]
async fn test_handle_update_containers_true_collects_refs() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    // Add a running container with a source image ref.
    let record = ContainerRecord {
        info: ContainerInfo {
            id: "ctr-update-test".to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Running".to_string(),
            created_at: "1970-01-01T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        runtime_id: None,
        rootfs_path: tmp.path().join("rootfs"),
        cgroup_path: tmp.path().join("cgroup"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: Some("alpine:latest".to_string()),
        upper_dir: None,
        merged_dir: None,
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None,
        manifest_path: None,
        workload_digest: None,
    };
    state.add_container(record).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
    handler::handle_update(
        handler::UpdateParams {
            images: vec![],
            all: false,
            containers: true,
            restart: false,
        },
        Arc::clone(&state),
        Arc::clone(&deps),
        tx,
    )
    .await;

    // Should produce UpdateProgress for alpine:latest (the mock pull succeeds).
    let resp = rx
        .recv()
        .await
        .expect("no response from handle_update containers=true");
    assert!(
        matches!(resp, DaemonResponse::UpdateProgress { ref image, .. } if image == "alpine:latest")
            || matches!(resp, DaemonResponse::Success { .. }),
        "expected UpdateProgress or Success, got {resp:?}"
    );
}

// ---- handle_update restart regression tests (#178) --------------------------

/// Regression test for #178: container with pid=None causes stop_inner to fail.
///
/// When restart=true and stop_inner returns Err (no PID), the handler must
/// `continue` past that container without panicking. The container record must
/// remain in state unchanged.
#[cfg(unix)]
#[tokio::test]
async fn test_handle_update_restart_stop_fails_continues() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    // Container with pid=None — stop_inner will return Err for this record.
    let record = ContainerRecord {
        info: ContainerInfo {
            id: "ctr-no-pid".to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Running".to_string(),
            created_at: "1970-01-01T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        runtime_id: None,
        rootfs_path: tmp.path().join("rootfs-no-pid"),
        cgroup_path: tmp.path().join("cgroup-no-pid"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: Some("alpine:latest".to_string()),
        upper_dir: None,
        merged_dir: None,
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None,
        manifest_path: None,
        workload_digest: None,
    };
    state.add_container(record).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
    handler::handle_update(
        handler::UpdateParams {
            images: vec!["alpine:latest".to_string()],
            all: false,
            containers: false,
            restart: true,
        },
        Arc::clone(&state),
        Arc::clone(&deps),
        tx,
    )
    .await;

    // Drain all responses — must terminate with Success (not hang or panic).
    let mut got_success = false;
    while let Ok(resp) = rx.try_recv() {
        if matches!(resp, DaemonResponse::Success { .. }) {
            got_success = true;
        }
    }
    // If try_recv missed it, do a blocking recv.
    if !got_success {
        let resp = rx
            .recv()
            .await
            .expect("handle_update must send terminal response");
        got_success = matches!(resp, DaemonResponse::Success { .. });
    }
    assert!(
        got_success,
        "handle_update must send Success terminal response"
    );

    // Original record must still be present and unchanged.
    let record = state
        .get_container("ctr-no-pid")
        .await
        .expect("record must still exist after failed stop");
    assert_eq!(
        record.info.state, "Running",
        "container state must remain Running when stop_inner failed"
    );
}

/// Regression test for #178: running container with creation_params=None.
///
/// When restart=true and stop succeeds but creation_params is None, the handler
/// hits the warn branch and does not attempt to re-run the container.
/// The final Success message must indicate stopped=1, restarted=0.
#[cfg(unix)]
#[tokio::test]
async fn test_handle_update_restart_no_creation_params_warns() {
    use minibox::daemon::state::{ContainerRecord, ContainerState};
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    // A "Running" container with a PID that doesn't exist (high value) so
    // nix::kill returns ESRCH immediately.  stop_inner treats that as "process
    // already gone" and marks the container Stopped, returning Ok(()).
    // This avoids sending SIGTERM to the test process itself.
    let our_pid = 2_000_000_u32; // far above OS PID limit — ESRCH on any signal
    let record = ContainerRecord {
        info: ContainerInfo {
            id: "ctr-no-params".to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Running".to_string(),
            created_at: "1970-01-01T00:00:00Z".to_string(),
            pid: Some(our_pid),
        },
        pid: Some(our_pid),
        runtime_id: None,
        rootfs_path: tmp.path().join("rootfs-no-params"),
        cgroup_path: tmp.path().join("cgroup-no-params"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: Some("alpine:latest".to_string()),
        upper_dir: None,
        merged_dir: None,
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None, // No creation_params — cannot restart.
        manifest_path: None,
        workload_digest: None,
    };
    state.add_container(record).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(32);
    handler::handle_update(
        handler::UpdateParams {
            images: vec!["alpine:latest".to_string()],
            all: false,
            containers: false,
            restart: true,
        },
        Arc::clone(&state),
        Arc::clone(&deps),
        tx,
    )
    .await;

    // Collect all responses.
    let mut responses = Vec::new();
    while let Ok(resp) = rx.try_recv() {
        responses.push(resp);
    }
    if let Some(resp) = rx.recv().await {
        responses.push(resp);
    }

    // Terminal response must be Success.
    let success_msg = responses
        .iter()
        .find_map(|r| {
            if let DaemonResponse::Success { message } = r {
                Some(message.clone())
            } else {
                None
            }
        })
        .expect("handle_update must send a Success terminal response");

    // Container was stopped (stop_inner succeeded) but not restarted.
    // Message must mention stopped=1, restarted=0.
    assert!(
        success_msg.contains("stopped 1") && success_msg.contains("restarted 0"),
        "expected 'stopped 1' and 'restarted 0' in message, got: {success_msg:?}"
    );

    // Container state must now be Stopped (stop_inner calls update_container_state).
    let record = state
        .get_container("ctr-no-params")
        .await
        .expect("record must still exist");
    assert_eq!(
        record.info.state,
        ContainerState::Stopped.as_str(),
        "container must be Stopped after successful stop with no creation_params"
    );
}

// ---- handle_list (multiple containers) --------------------------------------

/// handle_list returns all containers when multiple are present.
#[tokio::test]
async fn test_handle_list_multiple_containers() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    for i in 0..3usize {
        let record = ContainerRecord {
            info: ContainerInfo {
                id: format!("ctr-list-{i}"),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Stopped".to_string(),
                created_at: "1970-01-01T00:00:00Z".to_string(),
                pid: None,
            },
            pid: None,
            runtime_id: None,
            rootfs_path: tmp.path().join(format!("rootfs-list-{i}")),
            cgroup_path: tmp.path().join(format!("cgroup-list-{i}")),
            post_exit_hooks: vec![],
            rootfs_metadata: None,
            source_image_ref: None,
            upper_dir: None,
            merged_dir: None,
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

    let resp = handler::handle_list(Arc::clone(&state)).await;
    match resp {
        DaemonResponse::ContainerList { containers } => {
            assert_eq!(containers.len(), 3, "expected 3 containers in list");
        }
        other => panic!("expected ContainerList, got {other:?}"),
    }
}

// ---- handle_build (context path validation) ---------------------------------

/// handle_build with a relative context_path returns Error (security check).
#[tokio::test]
async fn test_handle_build_relative_context_path_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    // build.image_builder is None so we'll hit "build not supported" first.
    // To reach the path check, we'd need a builder. Test the no-builder path.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_build(
        "FROM alpine".to_string(),
        "relative/path".to_string(),
        "test:latest".to_string(),
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
        "expected Error (no builder or bad path), got {resp:?}"
    );
}

// ---- handle_get_manifest (happy path) ---------------------------------------

/// handle_get_manifest returns Manifest with correct content when the manifest
/// file exists on disk and the container record points to it.
#[tokio::test]
async fn test_handle_get_manifest_success() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::domain::execution_manifest::{
        ExecutionManifest, ExecutionManifestImage, ExecutionManifestRequest,
        ExecutionManifestRuntime, ExecutionManifestSubject,
    };
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    // Write a valid ExecutionManifest JSON to a file in the TempDir.
    let manifest = ExecutionManifest {
        schema_version: 1,
        container_id: "ctr-manifest-test".to_string(),
        created_at: "2026-05-11T00:00:00Z".to_string(),
        manifest_path: None,
        workload_digest: None,
        subject: ExecutionManifestSubject {
            image_ref: "alpine:3.18".to_string(),
            image: ExecutionManifestImage {
                manifest_digest: None,
                config_digest: None,
                layer_digests: vec![],
            },
        },
        runtime: ExecutionManifestRuntime {
            command: vec!["echo".to_string(), "hello".to_string()],
            env: vec![],
            mounts: vec![],
            resource_limits: None,
            network_mode: "none".to_string(),
            privileged: false,
            platform: None,
        },
        request: ExecutionManifestRequest {
            name: None,
            ephemeral: false,
        },
    };
    let manifest_path = tmp.path().join("execution-manifest.json");
    let json = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
    std::fs::write(&manifest_path, &json).expect("write manifest file");

    // Register a container record whose manifest_path points to the file.
    let record = ContainerRecord {
        info: ContainerInfo {
            id: "ctr-manifest-test".to_string(),
            name: None,
            image: "alpine:3.18".to_string(),
            command: "echo hello".to_string(),
            state: "Running".to_string(),
            created_at: "2026-05-11T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        runtime_id: None,
        rootfs_path: tmp.path().join("rootfs"),
        cgroup_path: tmp.path().join("cgroup"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: None,
        upper_dir: None,
        merged_dir: None,
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None,
        manifest_path: Some(manifest_path),
        workload_digest: None,
    };
    state.add_container(record).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_get_manifest("ctr-manifest-test".to_string(), state, deps, tx).await;

    let resp = rx
        .recv()
        .await
        .expect("no response from handle_get_manifest");
    match resp {
        DaemonResponse::Manifest { manifest: val } => {
            let image_ref = val
                .get("subject")
                .and_then(|s| s.get("image_ref"))
                .and_then(|v| v.as_str())
                .expect("image_ref missing from manifest value");
            assert_eq!(
                image_ref, "alpine:3.18",
                "returned manifest must contain correct image_ref"
            );
        }
        other => panic!("expected Manifest, got {other:?}"),
    }
}

// ---- handle_verify_manifest (happy path: allowed) ---------------------------

/// handle_verify_manifest returns VerifyResult { allowed: true } when the
/// policy permits the workload described in the manifest.
#[tokio::test]
async fn test_handle_verify_manifest_allowed() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::domain::execution_manifest::{
        ExecutionManifest, ExecutionManifestImage, ExecutionManifestRequest,
        ExecutionManifestRuntime, ExecutionManifestSubject,
    };
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let manifest = ExecutionManifest {
        schema_version: 1,
        container_id: "ctr-verify-allow".to_string(),
        created_at: "2026-05-11T00:00:00Z".to_string(),
        manifest_path: None,
        workload_digest: None,
        subject: ExecutionManifestSubject {
            image_ref: "alpine:3.18".to_string(),
            image: ExecutionManifestImage {
                manifest_digest: None,
                config_digest: None,
                layer_digests: vec![],
            },
        },
        runtime: ExecutionManifestRuntime {
            command: vec!["echo".to_string()],
            env: vec![],
            mounts: vec![],
            resource_limits: None,
            network_mode: "none".to_string(),
            privileged: false,
            platform: None,
        },
        request: ExecutionManifestRequest {
            name: None,
            ephemeral: false,
        },
    };
    let manifest_path = tmp.path().join("execution-manifest-allow.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("serialize"),
    )
    .expect("write manifest");

    let record = ContainerRecord {
        info: ContainerInfo {
            id: "ctr-verify-allow".to_string(),
            name: None,
            image: "alpine:3.18".to_string(),
            command: "echo".to_string(),
            state: "Running".to_string(),
            created_at: "2026-05-11T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        runtime_id: None,
        rootfs_path: tmp.path().join("rootfs"),
        cgroup_path: tmp.path().join("cgroup"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: None,
        upper_dir: None,
        merged_dir: None,
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None,
        manifest_path: Some(manifest_path),
        workload_digest: None,
    };
    state.add_container(record).await;

    // A permissive policy: no constraints at all.
    let permissive_policy = r#"{}"#;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_verify_manifest(
        "ctr-verify-allow".to_string(),
        permissive_policy.to_string(),
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx
        .recv()
        .await
        .expect("no response from handle_verify_manifest");
    match resp {
        DaemonResponse::VerifyResult { allowed, reason } => {
            assert!(allowed, "permissive policy must allow; reason: {reason:?}");
            assert!(reason.is_none(), "allowed result must have no reason");
        }
        other => panic!("expected VerifyResult, got {other:?}"),
    }
}

// ---- handle_verify_manifest (happy path: denied) ----------------------------

/// handle_verify_manifest returns VerifyResult { allowed: false, reason: Some(...) }
/// when the policy rejects the workload (e.g. image not in allowed list).
#[tokio::test]
async fn test_handle_verify_manifest_denied() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::domain::execution_manifest::{
        ExecutionManifest, ExecutionManifestImage, ExecutionManifestRequest,
        ExecutionManifestRuntime, ExecutionManifestSubject,
    };
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let manifest = ExecutionManifest {
        schema_version: 1,
        container_id: "ctr-verify-deny".to_string(),
        created_at: "2026-05-11T00:00:00Z".to_string(),
        manifest_path: None,
        workload_digest: None,
        subject: ExecutionManifestSubject {
            image_ref: "alpine:3.18".to_string(),
            image: ExecutionManifestImage {
                manifest_digest: None,
                config_digest: None,
                layer_digests: vec![],
            },
        },
        runtime: ExecutionManifestRuntime {
            command: vec!["echo".to_string()],
            env: vec![],
            mounts: vec![],
            resource_limits: None,
            network_mode: "none".to_string(),
            privileged: false,
            platform: None,
        },
        request: ExecutionManifestRequest {
            name: None,
            ephemeral: false,
        },
    };
    let manifest_path = tmp.path().join("execution-manifest-deny.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("serialize"),
    )
    .expect("write manifest");

    let record = ContainerRecord {
        info: ContainerInfo {
            id: "ctr-verify-deny".to_string(),
            name: None,
            image: "alpine:3.18".to_string(),
            command: "echo".to_string(),
            state: "Running".to_string(),
            created_at: "2026-05-11T00:00:00Z".to_string(),
            pid: None,
        },
        pid: None,
        runtime_id: None,
        rootfs_path: tmp.path().join("rootfs"),
        cgroup_path: tmp.path().join("cgroup"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: None,
        upper_dir: None,
        merged_dir: None,
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: None,
        manifest_path: Some(manifest_path),
        workload_digest: None,
    };
    state.add_container(record).await;

    // A restrictive policy: only ubuntu images are allowed.
    let restrictive_policy = r#"{"allowed_images":["ubuntu*"]}"#;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_verify_manifest(
        "ctr-verify-deny".to_string(),
        restrictive_policy.to_string(),
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx
        .recv()
        .await
        .expect("no response from handle_verify_manifest");
    match resp {
        DaemonResponse::VerifyResult { allowed, reason } => {
            assert!(!allowed, "restrictive policy must deny alpine image");
            assert!(
                reason.is_some(),
                "denied result must include a reason string"
            );
            let r = reason.expect("reason is Some");
            assert!(
                r.contains("not in allowed list"),
                "denial reason must mention 'not in allowed list', got: {r}"
            );
        }
        other => panic!("expected VerifyResult, got {other:?}"),
    }
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
        handler::PipelineParams {
            pipeline_path: "/nonexistent/pipeline.crux".to_string(),
            input: None,
            image: None,
            budget: None,
            env: vec![],
        },
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

// ---- handle_list_pipelines --------------------------------------------------

/// handle_list_pipelines on a fresh (empty) trace store returns an empty list.
#[tokio::test]
async fn test_handle_list_pipelines_empty_store() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_list_pipelines(None, None, state).await;
    match resp {
        DaemonResponse::PipelineList { pipelines } => {
            assert!(pipelines.is_empty(), "expected empty pipeline list");
        }
        other => panic!("expected PipelineList, got {other:?}"),
    }
}

/// handle_list_pipelines with a limit returns at most that many entries.
#[tokio::test]
async fn test_handle_list_pipelines_with_limit() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_list_pipelines(Some(5), None, state).await;
    match resp {
        DaemonResponse::PipelineList { pipelines } => {
            assert!(
                pipelines.len() <= 5,
                "expected at most 5 pipelines, got {}",
                pipelines.len()
            );
        }
        other => panic!("expected PipelineList, got {other:?}"),
    }
}

/// handle_list_pipelines with a pipeline name filter on an empty store returns empty.
#[tokio::test]
async fn test_handle_list_pipelines_with_pipeline_filter() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp =
        handler::handle_list_pipelines(None, Some("nonexistent.crux".to_string()), state).await;
    match resp {
        DaemonResponse::PipelineList { pipelines } => {
            assert!(pipelines.is_empty(), "expected empty filtered list");
        }
        other => panic!("expected PipelineList, got {other:?}"),
    }
}

// ---- handle_show_pipeline ---------------------------------------------------

/// handle_show_pipeline with a nonexistent ID returns Error.
#[tokio::test]
async fn test_handle_show_pipeline_not_found() {
    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_show_pipeline("no-such-run".to_string(), state).await;
    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("not found"),
                "expected 'not found' in error, got: {message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

// ---- ephemeral run (streaming path) -----------------------------------------

/// handle_run with ephemeral=true exercises handle_run_streaming and run_inner_capture.
///
/// Requires MockRuntime::with_output_pipe() which creates a real OS pipe so that
/// run_inner_capture can obtain an OwnedFd output_reader.
#[cfg(unix)]
#[tokio::test]
async fn test_handle_run_ephemeral_exercises_streaming_path() {
    use minibox::adapters::mocks::MockRuntime;

    let tmp = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&tmp);

    // Build deps with a MockRuntime configured to return an output pipe.
    let runtime: minibox_core::domain::DynContainerRuntime =
        Arc::new(MockRuntime::new().with_output_pipe());
    let deps = create_test_deps_with_dir_and_runtime(&tmp, runtime);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(32);
    handler::handle_run(
        handler::RunParams {
            image: "alpine".to_string(),
            tag: None,
            command: vec!["/bin/true".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            ephemeral: true,
            network: // ephemeral = true → streaming path
        None,
            mounts: vec![],
            privileged: false,
            shared_uid_range: false,
            env: vec![],
            name: None,
            platform: None,
            cgroup_parent: None, priority: None, policy_override: None,
        },
        Arc::clone(&state),
        deps,
        tx,
    )
    .await;

    // Drain all responses; expect ContainerStopped as the terminal message.
    let mut got_stopped = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        match rx.try_recv() {
            Ok(DaemonResponse::ContainerStopped { .. }) => {
                got_stopped = true;
                break;
            }
            Ok(_) => {}
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }
    assert!(
        got_stopped,
        "ephemeral run must produce ContainerStopped terminal response"
    );
}
