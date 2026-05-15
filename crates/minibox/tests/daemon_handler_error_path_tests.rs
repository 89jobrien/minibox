//! Error-path coverage for daemon handler lifecycle operations (GH #158).
//!
//! Covers branches that were not exercised by the existing failure/lifecycle
//! test suites:
//!
//! - `handle_remove` on a Running container returns `AlreadyRunning` error.
//! - `handle_resume` on a Running (non-paused) container returns an error.
//! - `handle_logs` on an unknown container sends Error via the channel.
//! - `handle_logs` with a dropped receiver does not panic.
//! - `handle_run` with an invalid image reference returns an Error response.
//! - `handle_stop` via name (not ID) for an unknown container returns Error.

use minibox::daemon::handler;
use minibox::daemon::state::{ContainerRecord, DaemonState};
use minibox_core::protocol::{ContainerInfo, DaemonResponse};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_state(temp_dir: &TempDir) -> Arc<DaemonState> {
    let image_store =
        minibox::image::ImageStore::new(temp_dir.path().join("images")).expect("unwrap in test");
    Arc::new(DaemonState::new(image_store, temp_dir.path()))
}

/// Build a container record in a specific state.
fn make_record_with_state(id: &str, state_str: &str) -> ContainerRecord {
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: state_str.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            pid: Some(99999),
        },
        pid: Some(99999),
        rootfs_path: PathBuf::from("/tmp/fake-rootfs"),
        cgroup_path: PathBuf::from("/tmp/fake-cgroup"),
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
// handle_remove: running container must be rejected
// ---------------------------------------------------------------------------

/// `handle_remove` on a container that is still `"Running"` must return
/// `DaemonResponse::Error` containing "running" (the `AlreadyRunning` domain
/// error text).
#[tokio::test]
async fn test_handle_remove_running_container_returns_error() {
    let tmp = TempDir::new().expect("create TempDir");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let id = "remove-running-test-001";
    state
        .add_container(make_record_with_state(id, "Running"))
        .await;

    let resp = handler::handle_remove(id.to_string(), state, deps).await;

    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "removing a running container must return Error, got {resp:?}"
    );
    if let DaemonResponse::Error { message } = resp {
        assert!(
            message.to_lowercase().contains("running") || message.contains("AlreadyRunning"),
            "error message must mention running state, got: {message}"
        );
    }
}

// ---------------------------------------------------------------------------
// handle_resume: non-paused container must be rejected
// ---------------------------------------------------------------------------

/// `handle_resume` on a container that is `"Running"` (not `"Paused"`) must
/// return `DaemonResponse::Error` containing "not paused".
#[tokio::test]
async fn test_handle_resume_running_container_returns_not_paused_error() {
    let tmp = TempDir::new().expect("create TempDir");
    let state = make_state(&tmp);

    let id = "resume-running-test-001";
    state
        .add_container(make_record_with_state(id, "Running"))
        .await;

    let event_sink: Arc<dyn minibox_core::events::EventSink> =
        Arc::new(minibox_core::events::NoopEventSink);
    let resp = handler::handle_resume(id.to_string(), state, event_sink).await;

    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "resuming a non-paused container must return Error, got {resp:?}"
    );
    if let DaemonResponse::Error { message } = resp {
        assert!(
            message.contains("not paused"),
            "error message must say 'not paused', got: {message}"
        );
    }
}

// ---------------------------------------------------------------------------
// handle_logs: unknown container sends Error via channel
// ---------------------------------------------------------------------------

/// `handle_logs` on an unknown container sends `DaemonResponse::Error` with
/// "not found" as the first (and only) message on the channel.
#[tokio::test]
async fn test_handle_logs_unknown_container_sends_error() {
    let tmp = TempDir::new().expect("create TempDir");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_logs(
        "nonexistent-container-xyz".to_string(),
        false,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("handler must send a response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not found")),
        "unknown container in handle_logs must send Error with 'not found', got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_logs: dropped receiver does not panic (send_error warn path)
// ---------------------------------------------------------------------------

/// When the receiver is dropped before `handle_logs` sends its error,
/// the handler must not panic — it should log a warning and return cleanly.
#[tokio::test]
async fn test_handle_logs_dropped_receiver_does_not_panic() {
    let tmp = TempDir::new().expect("create TempDir");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, rx) = tokio::sync::mpsc::channel::<DaemonResponse>(1);
    drop(rx); // Receiver dropped before handler runs.

    // This must complete without panic even though the send will fail.
    handler::handle_logs(
        "nonexistent-container-dropped-rx".to_string(),
        false,
        state,
        deps,
        tx,
    )
    .await;
    // No assertion needed — the test passes if it doesn't panic.
}

// ---------------------------------------------------------------------------
// handle_run: invalid image reference returns Error (not panic)
// ---------------------------------------------------------------------------

/// An image reference that cannot be parsed (empty string) must produce
/// `DaemonResponse::Error`, not a panic.
#[tokio::test]
async fn test_handle_run_invalid_image_ref_returns_error() {
    let tmp = TempDir::new().expect("create TempDir");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "".to_string(), // invalid — empty image name
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        None,
        vec![],
        false,
        vec![],
        None,
        None,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("handler must send a response");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "invalid image reference must produce Error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_stop: unknown container by name returns Error
// ---------------------------------------------------------------------------

/// `handle_stop` with a name that doesn't match any container returns
/// `DaemonResponse::Error` with "not found".
#[tokio::test]
async fn test_handle_stop_unknown_name_returns_error() {
    let tmp = TempDir::new().expect("create TempDir");
    let state = make_state(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let resp = handler::handle_stop("no-such-name-xyz".to_string(), state, deps).await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not found")),
        "stop on unknown name must return Error with 'not found', got {resp:?}"
    );
}
