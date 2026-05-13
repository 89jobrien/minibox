//! Exec, PTY, send_input, and resize_pty tests.

use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox::daemon::handler::{
    self, BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
    handle_resize_pty, handle_send_input,
};
use minibox_core::domain::{DynImageRegistry, NetworkMode};
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

// ---------------------------------------------------------------------------

/// handle_exec with no exec_runtime wired → Error "not supported".
#[tokio::test]
async fn test_handle_exec_no_runtime_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir); // exec_runtime: None
    let state = create_test_state_with_dir(&temp_dir);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_exec(
        "abc123def456abcd".to_string(),
        vec!["/bin/sh".to_string()],
        vec![],
        None,
        false,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not supported")),
        "expected 'not supported' error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------

// handle_send_input happy path — channel registered, bytes delivered (#116/#129)
// ---------------------------------------------------------------------------

/// Registering a stdin channel then calling handle_send_input delivers
/// decoded bytes to the channel and returns Success.
///
/// Covers the `reg.stdin.get(...)` Some arm and final Success send in
/// handle_send_input — both previously uncovered.
#[tokio::test]
async fn test_handle_send_input_delivers_bytes_and_returns_success() {
    use base64::Engine as _;
    let tmp = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&tmp);

    let session_id = "test-session-send-01";
    let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
    {
        let mut reg = deps.exec.pty_sessions.lock().await;
        reg.stdin.insert(session_id.to_string(), stdin_tx);
    }

    let payload = b"hello world";
    let encoded = base64::engine::general_purpose::STANDARD.encode(payload);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handle_send_input(
        minibox_core::domain::SessionId::from(session_id),
        encoded,
        Arc::clone(&deps),
        tx,
    )
    .await;

    let received = stdin_rx
        .recv()
        .await
        .expect("stdin channel must receive bytes");
    assert_eq!(&received[..], payload, "delivered bytes must match payload");

    let resp = rx.recv().await.expect("no response from handle_send_input");
    assert!(
        matches!(resp, DaemonResponse::Success { ref message } if message.contains("input")),
        "expected Success with 'input' in message, got {resp:?}"
    );
}

/// handle_send_input with invalid base64 returns Error.
#[tokio::test]
async fn test_handle_send_input_invalid_base64_returns_error() {
    let tmp = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&tmp);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handle_send_input(
        minibox_core::domain::SessionId::from("any-session"),
        "!!!not-base64!!!".to_string(),
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "expected Error for invalid base64, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_resize_pty happy path — channel registered, resize delivered (#116/#129)

// ---------------------------------------------------------------------------

/// Registering a resize channel then calling handle_resize_pty delivers
/// (cols, rows) and returns Success — both previously uncovered.
#[tokio::test]
async fn test_handle_resize_pty_delivers_size_and_returns_success() {
    let tmp = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&tmp);

    let session_id = "test-session-resize-01";
    let (resize_tx, mut resize_rx) = tokio::sync::mpsc::channel::<(u16, u16)>(8);
    {
        let mut reg = deps.exec.pty_sessions.lock().await;
        reg.resize.insert(session_id.to_string(), resize_tx);
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handle_resize_pty(
        minibox_core::domain::SessionId::from(session_id),
        132,
        50,
        Arc::clone(&deps),
        tx,
    )
    .await;

    let (cols, rows) = resize_rx
        .recv()
        .await
        .expect("resize channel must receive size");
    assert_eq!((cols, rows), (132, 50), "resize dimensions must match");

    let resp = rx.recv().await.expect("no response from handle_resize_pty");
    assert!(
        matches!(resp, DaemonResponse::Success { ref message } if message.contains("resize")),
        "expected Success with 'resize' in message, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_pause / handle_resume success paths (#116/#129)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------

/// cleanup removes both the resize and stdin channels for the target session
/// while leaving all other sessions intact.
#[tokio::test]
async fn test_pty_session_registry_cleanup_removes_only_target_session() {
    use minibox::daemon::handler::PtySessionRegistry;

    let mut registry = PtySessionRegistry::default();

    let (resize_tx_a, _) = tokio::sync::mpsc::channel::<(u16, u16)>(1);
    let (stdin_tx_a, _) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
    let (resize_tx_b, _) = tokio::sync::mpsc::channel::<(u16, u16)>(1);
    let (stdin_tx_b, _) = tokio::sync::mpsc::channel::<Vec<u8>>(1);

    registry.resize.insert("session-a".to_string(), resize_tx_a);
    registry.stdin.insert("session-a".to_string(), stdin_tx_a);
    registry.resize.insert("session-b".to_string(), resize_tx_b);
    registry.stdin.insert("session-b".to_string(), stdin_tx_b);

    registry.cleanup("session-a");

    assert!(
        !registry.resize.contains_key("session-a"),
        "session-a resize must be removed"
    );
    assert!(
        !registry.stdin.contains_key("session-a"),
        "session-a stdin must be removed"
    );
    assert!(
        registry.resize.contains_key("session-b"),
        "session-b resize must remain"
    );
    assert!(
        registry.stdin.contains_key("session-b"),
        "session-b stdin must remain"
    );
}

// ---------------------------------------------------------------------------
// send_error dropped-receiver path (#116/#129)
// ---------------------------------------------------------------------------

/// When the response channel receiver is dropped before handle_run sends its
/// terminal error response, the `if tx.send(...).await.is_err()` warn path
/// must not panic.

#[tokio::test]
async fn test_handler_with_dropped_receiver_does_not_panic() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&tmp);

    let deps_fail = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(minibox_core::adapters::HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(tmp.path().join("img_dr")).expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: tmp.path().join("containers_dr"),
            run_containers_base: tmp.path().join("run_dr"),
        },
        exec: ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                minibox::daemon::handler::PtySessionRegistry::default(),
            )),
        },
        build: BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: EventDeps {
            event_sink: Arc::new(minibox_core::events::NoopEventSink),
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: minibox::daemon::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    });

    let (tx, rx) = tokio::sync::mpsc::channel::<DaemonResponse>(1);
    drop(rx); // closed — any send returns Err, triggering the warn path

    handler::handle_run(
        "alpine".to_string(),
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
        deps_fail,
        tx,
    )
    .await;
    // No panic = warn path exercised correctly.
}

// ---------------------------------------------------------------------------
// Error-path tests: handle_run — infrastructure adapter failures
// ---------------------------------------------------------------------------

/// handle_run with a filesystem that fails setup_rootfs → Error response (variant 2).

// --- handle_resize_pty: unknown session ---

#[tokio::test]
async fn test_handle_resize_pty_unknown_session_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handle_resize_pty(
        minibox_core::domain::SessionId::new("nonexistent-session".to_string()),
        80,
        24,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("no active tty session")),
        "expected unknown session error, got {resp:?}"
    );
}

// --- handle_send_input: closed channel still succeeds ---

#[tokio::test]
async fn test_handle_send_input_closed_channel_still_succeeds() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let d = (*create_test_deps_with_dir(&temp_dir)).clone();
    let (stdin_tx, stdin_rx_dropped) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
    drop(stdin_rx_dropped);
    {
        let mut reg = d.exec.pty_sessions.lock().await;
        reg.stdin.insert("test-session".to_string(), stdin_tx);
    }
    let deps = Arc::new(d);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handle_send_input(
        minibox_core::domain::SessionId::new("test-session".to_string()),
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"hello"),
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Success { .. }),
        "expected Success (channel closed is non-fatal), got {resp:?}"
    );
}

// --- handle_resize_pty: closed channel still succeeds ---

#[tokio::test]
async fn test_handle_resize_pty_closed_channel_still_succeeds() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let d = (*create_test_deps_with_dir(&temp_dir)).clone();
    let (resize_tx, resize_rx_dropped) = tokio::sync::mpsc::channel::<(u16, u16)>(1);
    drop(resize_rx_dropped);
    {
        let mut reg = d.exec.pty_sessions.lock().await;
        reg.resize.insert("test-session".to_string(), resize_tx);
    }
    let deps = Arc::new(d);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handle_resize_pty(
        minibox_core::domain::SessionId::new("test-session".to_string()),
        120,
        40,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Success { .. }),
        "expected Success (channel closed is non-fatal), got {resp:?}"
    );
}

// --- handle_send_input: unknown session ---

#[tokio::test]
async fn test_handle_send_input_unknown_session_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handle_send_input(
        minibox_core::domain::SessionId::new("nonexistent-session".to_string()),
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"data"),
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("no active tty session")),
        "expected unknown session error, got {resp:?}"
    );
}

// --- handle_logs: missing log dir sends Success ---

#[tokio::test]
async fn test_handle_logs_missing_log_dir_sends_success() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&temp_dir);
    let deps = create_test_deps_with_dir(&temp_dir);

    let (tx_run, mut rx_run) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
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
        state.clone(),
        deps.clone(),
        tx_run,
    )
    .await;
    let container_id = match rx_run.recv().await.expect("unwrap in test") {
        DaemonResponse::ContainerCreated { id } => id,
        other => panic!("expected ContainerCreated, got {other:?}"),
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_logs(container_id, false, state, deps, tx).await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Success { ref message } if message.contains("end of log")),
        "expected end-of-log Success, got {resp:?}"
    );
}

// --- handle_run: duplicate container name ---

#[tokio::test]
async fn test_handle_run_duplicate_name_second_attempt_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&temp_dir);
    let deps = create_test_deps_with_dir(&temp_dir);

    let (tx1, mut rx1) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        None,
        vec![],
        false,
        vec![],
        Some("mycontainer".to_string()),
        None,
        state.clone(),
        deps.clone(),
        tx1,
    )
    .await;
    let resp1 = rx1.recv().await.expect("unwrap in test");
    assert!(
        matches!(resp1, DaemonResponse::ContainerCreated { .. }),
        "first run should succeed, got {resp1:?}"
    );

    let (tx2, mut rx2) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        None,
        vec![],
        false,
        vec![],
        Some("mycontainer".to_string()),
        None,
        state,
        deps,
        tx2,
    )
    .await;
    let resp2 = rx2.recv().await.expect("unwrap in test");
    assert!(
        matches!(resp2, DaemonResponse::Error { ref message } if message.contains("already in use")),
        "duplicate name should return error, got {resp2:?}"
    );
}

// --- handle_run: network setup failure (bridge mode) ---

#[tokio::test]
async fn test_handle_run_network_setup_failure_bridge_mode() {
    use minibox::adapters::mocks::MockNetwork;

    let temp_dir = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&temp_dir);

    let mock_registry = Arc::new(MockRegistry::new());
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images2")).expect("unwrap in test"));
    let failing_network = Arc::new(MockNetwork::new().with_setup_failure());
    let deps = build_deps(
        Arc::new(minibox_core::adapters::HostnameRegistryRouter::new(
            Arc::clone(&mock_registry) as minibox_core::domain::DynImageRegistry,
            std::iter::empty::<(&str, minibox_core::domain::DynImageRegistry)>(),
        )),
        image_store,
        failing_network,
        temp_dir.path().join("containers"),
        temp_dir.path().join("run"),
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        Some(NetworkMode::Bridge),
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

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("network")),
        "expected network setup error, got {resp:?}"
    );
}

// --- handle_exec: failure with tty=true cleans up PTY registry ---

#[tokio::test]
async fn test_handle_exec_failure_with_tty_cleans_up_pty_registry() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&temp_dir);

    let mock_exec = minibox::testing::mocks::MockExecRuntime::new().with_failure();
    let deps = {
        let mut d = (*create_test_deps_with_dir(&temp_dir)).clone();
        d.exec.exec_runtime = Some(Arc::new(mock_exec) as minibox_core::domain::DynExecRuntime);
        Arc::new(d)
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_exec(
        "abc123def456abcd".to_string(),
        vec!["sh".to_string()],
        vec![],
        None,
        true,
        state,
        deps.clone(),
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("exec failed")),
        "expected exec failure error, got {resp:?}"
    );

    let reg = deps.exec.pty_sessions.lock().await;
    assert!(
        reg.resize.is_empty(),
        "PTY resize registry should be empty after exec failure"
    );
    assert!(
        reg.stdin.is_empty(),
        "PTY stdin registry should be empty after exec failure"
    );
}
