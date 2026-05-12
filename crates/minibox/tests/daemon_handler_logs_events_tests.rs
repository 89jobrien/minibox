//! Log streaming, event handling, pause/resume, and serde round-trip tests.

use minibox::daemon::handler::{
    self, handle_resize_pty, handle_send_input,
};
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;


// ---------------------------------------------------------------------------
// handle_pause / handle_resume Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_pause_nonexistent_container_returns_error() {
    let tmp = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_pause(
        "doesnotexist".to_string(),
        state,
        Arc::new(minibox_core::events::NoopEventSink),
    )
    .await;
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "got: {resp:?}"
    );
}

#[tokio::test]
async fn test_resume_nonexistent_container_returns_error() {
    let tmp = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&tmp);

    let resp = handler::handle_resume(
        "doesnotexist".to_string(),
        state,
        Arc::new(minibox_core::events::NoopEventSink),
    )
    .await;
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "got: {resp:?}"
    );
}



// ---------------------------------------------------------------------------
// handle_logs Tests
// ---------------------------------------------------------------------------

/// handle_logs for unknown container_id → Error response.
#[tokio::test]
async fn test_handle_logs_unknown_container() {
    let temp_dir = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
    handler::handle_logs("nonexistent-id".to_string(), false, state, deps, tx).await;

    let resp = rx.recv().await.expect("handle_logs sent no response");
    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("not found") || message.contains("nonexistent"),
                "expected not-found error, got: {message}"
            );
        }
        _ => panic!("expected Error, got {resp:?}"),
    }
}

/// handle_logs for known container with log files → LogLine + Success.
#[tokio::test]
async fn test_handle_logs_reads_log_files() {
    let temp_dir = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // Register a container in state.
    let container_id = "logtest001logtest001";
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
        rootfs_path: std::path::PathBuf::from("/tmp/fake"),
        cgroup_path: std::path::PathBuf::from("/tmp/fake"),
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

    // Write log files to the expected path.
    let container_dir = temp_dir.path().join("containers").join(container_id);
    std::fs::create_dir_all(&container_dir).unwrap();
    std::fs::write(container_dir.join("stdout.log"), "line one\nline two\n").unwrap();
    std::fs::write(container_dir.join("stderr.log"), "err line\n").unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(32);
    handler::handle_logs(container_id.to_string(), false, state, deps, tx).await;

    let mut log_lines = Vec::new();
    let mut got_success = false;
    while let Some(resp) = rx.recv().await {
        match resp {
            DaemonResponse::LogLine { stream: _, line } => log_lines.push(line),
            DaemonResponse::Success { .. } => {
                got_success = true;
                break;
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    assert!(got_success, "expected terminal Success");
    assert!(log_lines.contains(&"line one".to_string()));
    assert!(log_lines.contains(&"line two".to_string()));
    assert!(log_lines.contains(&"err line".to_string()));
}

// ---------------------------------------------------------------------------
// PtySessionRegistry — SendInput / ResizePty handler tests
// ---------------------------------------------------------------------------



#[tokio::test]
async fn send_input_unknown_session_returns_error() {
    use base64::Engine as _;
    let tmp = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&tmp);
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let data = base64::engine::general_purpose::STANDARD.encode(b"hello");
    handle_send_input(
        minibox_core::domain::SessionId::from("no-such-session"),
        data,
        deps,
        tx,
    )
    .await;
    let resp = rx.recv().await.unwrap();
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "expected Error for unknown session, got {resp:?}"
    );
}

#[tokio::test]
async fn resize_pty_unknown_session_returns_error() {
    let tmp = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&tmp);
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    handle_resize_pty(
        minibox_core::domain::SessionId::from("no-such-session"),
        80,
        24,
        deps,
        tx,
    )
    .await;
    let resp = rx.recv().await.unwrap();
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "expected Error for unknown session, got {resp:?}"
    );
}



// ---------------------------------------------------------------------------
// Additional error-path tests for handle_pause / handle_resume (#116)
// ---------------------------------------------------------------------------

/// Pausing a stopped container returns an Error — the container must be Running.
#[tokio::test]
async fn test_pause_stopped_container_returns_error() {
    let tmp = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&tmp);

    let container_id = "pausetest001abc01";
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
        rootfs_path: std::path::PathBuf::from("/tmp/fake"),
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

    let resp = handler::handle_pause(
        container_id.to_string(),
        state,
        Arc::new(minibox_core::events::NoopEventSink),
    )
    .await;

    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("not running") || message.contains("Stopped"),
                "expected 'not running' error, got: {message}"
            );
        }
        _ => panic!("expected Error for stopped container, got {resp:?}"),
    }
}

/// Resuming a running (non-paused) container returns an Error.
///
/// Only containers in state `Paused` can be resumed.
#[tokio::test]
async fn test_resume_running_container_returns_error() {
    let tmp = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&tmp);

    let container_id = "resumetest001abc0";
    let record = minibox::daemon::state::ContainerRecord {
        info: minibox_core::protocol::ContainerInfo {
            id: container_id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Running".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: Some(12345),
        },
        pid: Some(12345),
        rootfs_path: std::path::PathBuf::from("/tmp/fake"),
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

    let resp = handler::handle_resume(
        container_id.to_string(),
        state,
        Arc::new(minibox_core::events::NoopEventSink),
    )
    .await;

    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("not paused") || message.contains("Running"),
                "expected 'not paused' error, got: {message}"
            );
        }
        _ => panic!("expected Error for non-paused container, got {resp:?}"),
    }
}

/// Pausing a Running container whose cgroup directory does not exist returns an Error.
///
/// `handle_pause` writes `1` to `{cgroup_path}/cgroup.freeze`. If the cgroup
/// dir is absent the write must fail gracefully rather than panic.
#[tokio::test]
async fn test_pause_missing_cgroup_returns_error() {
    let tmp = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&tmp);

    // Point cgroup_path at a directory that does not exist.
    let nonexistent_cgroup = tmp.path().join("no-such-cgroup-dir");

    let container_id = "pausecgrouptest001";
    let record = minibox::daemon::state::ContainerRecord {
        info: minibox_core::protocol::ContainerInfo {
            id: container_id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Running".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: Some(99999),
        },
        pid: Some(99999),
        rootfs_path: std::path::PathBuf::from("/tmp/fake"),
        cgroup_path: nonexistent_cgroup,
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

    let resp = handler::handle_pause(
        container_id.to_string(),
        state,
        Arc::new(minibox_core::events::NoopEventSink),
    )
    .await;

    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("pause failed") || message.contains("No such file"),
                "expected cgroup write error, got: {message}"
            );
        }
        _ => panic!("expected Error when cgroup path is absent, got {resp:?}"),
    }
}

/// Resuming a Paused container whose cgroup directory does not exist returns an Error.
///
/// Mirrors `test_pause_missing_cgroup_returns_error` for the resume path.
#[tokio::test]
async fn test_resume_missing_cgroup_returns_error() {
    let tmp = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&tmp);

    let nonexistent_cgroup = tmp.path().join("no-such-cgroup-resume");

    let container_id = "resumecgrouptest01";
    let record = minibox::daemon::state::ContainerRecord {
        info: minibox_core::protocol::ContainerInfo {
            id: container_id.to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Paused".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: Some(99998),
        },
        pid: Some(99998),
        rootfs_path: std::path::PathBuf::from("/tmp/fake"),
        cgroup_path: nonexistent_cgroup,
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

    let resp = handler::handle_resume(
        container_id.to_string(),
        state,
        Arc::new(minibox_core::events::NoopEventSink),
    )
    .await;

    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("resume failed") || message.contains("No such file"),
                "expected cgroup write error, got: {message}"
            );
        }
        _ => panic!("expected Error when cgroup path is absent, got {resp:?}"),
    }
}



/// ContainerPaused and transitions state to Paused.
#[tokio::test]
async fn test_handle_pause_running_container_succeeds() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&tmp);

    let cgroup_dir = tmp.path().join("fake-cgroup-pause");
    std::fs::create_dir_all(&cgroup_dir).unwrap();

    let container_id = "pausesuccesstest01";
    state
        .add_container(ContainerRecord {
            info: ContainerInfo {
                id: container_id.to_string(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Running".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                pid: Some(99997),
            },
            pid: Some(99997),
            rootfs_path: tmp.path().join("fake-rootfs"),
            cgroup_path: cgroup_dir.clone(),
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
        })
        .await;

    let resp = handler::handle_pause(
        container_id.to_string(),
        Arc::clone(&state),
        Arc::new(minibox_core::events::NoopEventSink),
    )
    .await;

    assert!(
        matches!(resp, DaemonResponse::ContainerPaused { ref id } if id == container_id),
        "expected ContainerPaused, got {resp:?}"
    );

    let record = state
        .get_container(container_id)
        .await
        .expect("container must still exist");
    assert_eq!(
        record.info.state, "Paused",
        "state should be Paused after handle_pause"
    );
}

/// handle_resume on a Paused container with a writable cgroup dir returns
/// ContainerResumed and transitions state to Running.
#[tokio::test]
async fn test_handle_resume_paused_container_succeeds() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&tmp);

    let cgroup_dir = tmp.path().join("fake-cgroup-resume");
    std::fs::create_dir_all(&cgroup_dir).unwrap();
    std::fs::write(cgroup_dir.join("cgroup.freeze"), "1\n").unwrap();

    let container_id = "resumesuccesstest1";
    state
        .add_container(ContainerRecord {
            info: ContainerInfo {
                id: container_id.to_string(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Paused".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                pid: Some(99996),
            },
            pid: Some(99996),
            rootfs_path: tmp.path().join("fake-rootfs"),
            cgroup_path: cgroup_dir.clone(),
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
        })
        .await;

    let resp = handler::handle_resume(
        container_id.to_string(),
        Arc::clone(&state),
        Arc::new(minibox_core::events::NoopEventSink),
    )
    .await;

    assert!(
        matches!(resp, DaemonResponse::ContainerResumed { ref id } if id == container_id),
        "expected ContainerResumed, got {resp:?}"
    );

    let record = state
        .get_container(container_id)
        .await
        .expect("container must still exist");
    assert_eq!(
        record.info.state, "Running",
        "state should be Running after handle_resume"
    );
}

// ---------------------------------------------------------------------------
// PtySessionRegistry::cleanup unit test (#116/#129)


/// All `DaemonResponse` variants round-trip through JSON serialisation.
///
/// This ensures every variant is serialisable and deserialisable without data
/// loss — a prerequisite for the JSON-over-newline wire protocol.
#[test]
fn test_daemon_response_serde_round_trip_all_variants() {
    use minibox_core::domain::SnapshotInfo;
    use minibox_core::events::ContainerEvent;
    use minibox_core::protocol::{ContainerInfo, OutputStreamKind};

    let now = "2026-01-01T00:00:00Z".to_string();

    let variants: Vec<DaemonResponse> = vec![
        DaemonResponse::ContainerCreated {
            id: "abc123".to_string(),
        },
        DaemonResponse::Success {
            message: "ok".to_string(),
        },
        DaemonResponse::ContainerPaused {
            id: "abc123".to_string(),
        },
        DaemonResponse::ContainerResumed {
            id: "abc123".to_string(),
        },
        DaemonResponse::ContainerList {
            containers: vec![ContainerInfo {
                id: "abc123".to_string(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Running".to_string(),
                created_at: now.clone(),
                pid: Some(1234),
            }],
        },
        DaemonResponse::ImageLoaded {
            image: "myimage:latest".to_string(),
        },
        DaemonResponse::Error {
            message: "something went wrong".to_string(),
        },
        DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: "aGVsbG8=".to_string(), // base64 "hello"
        },
        DaemonResponse::ContainerStopped { exit_code: 0 },
        DaemonResponse::ExecStarted {
            exec_id: "exec-001".to_string(),
        },
        DaemonResponse::PushProgress {
            layer_digest: "sha256:abc".to_string(),
            bytes_uploaded: 1024,
            total_bytes: 4096,
        },
        DaemonResponse::BuildOutput {
            step: 1,
            total_steps: 3,
            message: "RUN echo hi".to_string(),
        },
        DaemonResponse::BuildComplete {
            image_id: "sha256:deadbeef".to_string(),
            tag: "myapp:latest".to_string(),
        },
        DaemonResponse::Event {
            event: ContainerEvent::Created {
                id: "abc123".to_string(),
                image: "alpine:latest".to_string(),
                timestamp: std::time::SystemTime::UNIX_EPOCH,
            },
        },
        DaemonResponse::Pruned {
            removed: vec!["alpine:old".to_string()],
            freed_bytes: 1024 * 1024,
            dry_run: false,
        },
        DaemonResponse::LogLine {
            stream: OutputStreamKind::Stderr,
            line: "error: something".to_string(),
        },
        DaemonResponse::PipelineComplete {
            trace: serde_json::json!({"steps": []}),
            container_id: "abc123".to_string(),
            exit_code: 0,
        },
        DaemonResponse::SnapshotSaved {
            info: SnapshotInfo {
                container_id: "abc123".to_string(),
                name: "snap1".to_string(),
                created_at: now.clone(),
                adapter: "smolvm".to_string(),
                image: "alpine:latest".to_string(),
                size_bytes: 0,
            },
        },
        DaemonResponse::SnapshotRestored {
            id: "abc123".to_string(),
            name: "snap1".to_string(),
        },
        DaemonResponse::SnapshotList {
            id: "abc123".to_string(),
            snapshots: vec![],
        },
        DaemonResponse::UpdateProgress {
            image: "alpine:latest".to_string(),
            status: "up to date".to_string(),
        },
    ];

    for variant in &variants {
        let json = serde_json::to_string(variant)
            .unwrap_or_else(|e| panic!("serialise {variant:?} failed: {e}"));
        let restored: DaemonResponse = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("deserialise {variant:?} failed: {e}\njson: {json}"));
        // Re-serialise to compare — direct PartialEq not derived.
        let json2 = serde_json::to_string(&restored)
            .unwrap_or_else(|e| panic!("re-serialise {restored:?} failed: {e}"));
        assert_eq!(json, json2, "round-trip mismatch for variant: {variant:?}");
    }
}

