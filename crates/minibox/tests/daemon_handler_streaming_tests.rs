//! Streaming, ephemeral run, and wait-for-exit tests.

use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox::daemon::handler::{
    self, BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
};
use minibox::daemon::state::DaemonState;
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

// ---------------------------------------------------------------------------
// daemon_wait_for_exit coverage
//
// The non-streaming run path calls daemon_wait_for_exit inside spawn_blocking.
// MockRuntime hands out fake PIDs (10000+) that don't correspond to real
// processes, so waitpid returns ECHILD → the Err arm fires, exit code -1 is
// recorded, and update_container_state("Stopped") is called.
//
// Existing tests use a fixed 100–200 ms sleep which is often not enough for
// the spawn_blocking thread to complete before the test exits.  These tests
// poll until the container reaches "Stopped" (up to 2s) so the blocking thread
// definitely finishes, covering lines 731–768 of handler.rs.
// ---------------------------------------------------------------------------

/// Helper: poll DaemonState until the container reaches the expected state or
/// the deadline passes.  Returns the final state string.
async fn wait_for_container_state(
    state: &Arc<DaemonState>,
    id: &str,
    expected: &str,
    timeout_ms: u64,
) -> String {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
    loop {
        if let Some(record) = state.get_container(id).await
            && record.info.state == expected
        {
            return record.info.state;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
    state
        .get_container(id)
        .await
        .map(|r| r.info.state)
        .unwrap_or_else(|| "GONE".to_string())
}

/// Non-streaming run with a successful mock spawn: daemon_wait_for_exit is
/// invoked via spawn_blocking with a non-existent PID, waitpid returns an
/// error (ECHILD), and the container is eventually marked "Stopped".
///
/// This covers the waitpid Err arm (handler.rs 743–746) and the state-update
/// path (lines 758–768) inside daemon_wait_for_exit.
#[tokio::test]
#[cfg(unix)]
async fn test_daemon_wait_for_exit_covers_waitpid_error_arm() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/true".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps,
    )
    .await;

    let id = extract_container_id(&response);

    // Poll until Stopped (daemon_wait_for_exit must complete for this to happen).
    let final_state = wait_for_container_state(&state, &id, "Stopped", 2000).await;

    assert_eq!(
        final_state, "Stopped",
        "container should reach Stopped after daemon_wait_for_exit completes"
    );
}

/// Multiple successive non-streaming runs all reach "Stopped" — verifies
/// daemon_wait_for_exit handles each container's spawn_blocking call
/// independently and doesn't interfere between containers.
#[tokio::test]
#[cfg(unix)]
async fn test_daemon_wait_for_exit_multiple_containers() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let mut ids = Vec::new();
    for _ in 0..3 {
        let response = handle_run_once(
            "alpine".to_string(),
            None,
            vec!["/bin/true".to_string()],
            None,
            None,
            false,
            state.clone(),
            deps.clone(),
        )
        .await;
        ids.push(extract_container_id(&response));
    }

    for id in &ids {
        let final_state = wait_for_container_state(&state, id, "Stopped", 2000).await;
        assert_eq!(
            final_state, "Stopped",
            "container {id} should reach Stopped"
        );
    }
}

// ---------------------------------------------------------------------------
// handle_run ephemeral=true path (lines 119–131 of handler.rs)
//
// On Unix, handle_run with ephemeral=true dispatches to handle_run_streaming
// which calls run_inner_capture.  MockRuntime.spawn_process() returns
// output_reader=None, so run_inner_capture returns an error at the
// "capture_output=true but runtime returned no output_reader" check.
// handle_run_streaming catches this and sends DaemonResponse::Error.
//
// This covers the #[cfg(unix)] ephemeral branch at lines 119–131 and the
// error-return path in handle_run_streaming (lines 194–203).
// ---------------------------------------------------------------------------

/// handle_run with ephemeral=true on Unix dispatches to handle_run_streaming.
/// MockRuntime returns output_reader=None, which causes run_inner_capture to
/// fail, and the channel receives a DaemonResponse::Error.
#[tokio::test]
#[cfg(unix)]
async fn test_handle_run_ephemeral_dispatches_streaming_path() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"))
                    as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()), // returns output_reader=None
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
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
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        true, // ephemeral=true → streaming path
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

    let response = rx.recv().await.expect("handler must send a response");
    // MockRuntime returns output_reader=None, so run_inner_capture fails with
    // "capture_output=true but runtime returned no output_reader".
    match response {
        DaemonResponse::Error { ref message } => {
            assert!(
                message.contains("output_reader") || message.contains("capture"),
                "expected output_reader error, got: {message}"
            );
        }
        // ContainerStopped with exit_code=-1 is also acceptable if the mock
        // runtime path somehow returns a PID without an output_reader.
        DaemonResponse::ContainerStopped { .. } => {}
        other => panic!("expected Error or ContainerStopped from ephemeral path, got: {other:?}"),
    }
}

/// handle_run with ephemeral=true and a pull failure sends Error via channel.
/// Covers the error-return at the top of handle_run_streaming (line ~195).
#[tokio::test]
#[cfg(unix)]
async fn test_handle_run_ephemeral_pull_failure_sends_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
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
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        true, // ephemeral=true
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

    let response = rx.recv().await.expect("handler must send a response");
    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "ephemeral run with pull failure must produce Error, got: {response:?}"
    );
}

// ---------------------------------------------------------------------------
// Error path tests for missing coverage
// ---------------------------------------------------------------------------

/// When registry has no layers for an image, handle_run produces an Error.
#[tokio::test]
async fn test_run_empty_image_no_layers() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_registry = Arc::new(MockRegistry::new().with_empty_layers());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::clone(&mock_registry) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
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
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::Error { message } => {
            // The error should mention no layers or empty image
            assert!(
                message.contains("no layers")
                    || message.contains("empty")
                    || message.contains("Empty"),
                "expected empty image error, got: {message}"
            );
        }
        other => panic!("expected Error response for empty image, got {other:?}"),
    }
}

/// Pulling an image that fails at the registry produces an Error.
#[tokio::test]
async fn test_pull_registry_failure_with_tag() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_registry = Arc::new(MockRegistry::new().with_pull_failure());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::clone(&mock_registry) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
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
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_pull(
        "testimage".to_string(),
        Some("v1.0".to_string()),
        None,
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("pull") || message.contains("failed"),
                "expected pull failure, got: {message}"
            );
        }
        _ => panic!("expected Error response for pull failure, got {response:?}"),
    }
}

// ---------------------------------------------------------------------------

// Regression: minibox run streaming protocol order (2026-03-27)
//
// Root causes that were fixed:
// 1. is_terminal_response returned true for ContainerCreated → connection closed early
// 2. handle_run_streaming never emitted ContainerCreated before streaming output
// 3. CLI run.rs returned Ok(()) on ContainerCreated instead of continuing
//
// This test verifies the full streaming sequence when the runtime successfully
// spawns a container:  ContainerCreated → ContainerOutput → ContainerStopped
// ---------------------------------------------------------------------------

/// A test-only runtime that returns a real Unix pipe so the streaming path
/// in handle_run_streaming can be exercised end-to-end with known output.
///
/// `spawn_process` writes `payload` bytes to the write end of the pipe and
/// closes it, simulating a container that produces output and then exits.
/// `waitpid` on the fake PID will return ECHILD → exit_code = -1.
#[cfg(unix)]
struct PipedMockRuntime {
    payload: Vec<u8>,
}

#[cfg(unix)]
impl minibox_core::domain::AsAny for PipedMockRuntime {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(unix)]
#[async_trait::async_trait]
impl minibox_core::domain::ContainerRuntime for PipedMockRuntime {
    fn capabilities(&self) -> minibox_core::domain::RuntimeCapabilities {
        minibox_core::domain::RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: false,
            supports_overlay_fs: false,
            supports_network_isolation: false,
            max_containers: None,
        }
    }

    async fn spawn_process(
        &self,
        _config: &minibox_core::domain::ContainerSpawnConfig,
    ) -> anyhow::Result<minibox_core::domain::SpawnResult> {
        use std::io::Write;
        use std::os::unix::io::{FromRawFd, IntoRawFd, OwnedFd};

        let (read_fd, write_fd) = nix::unistd::pipe().expect("pipe");
        // Write the payload into the pipe synchronously, then close the write
        // end so the drain loop sees EOF immediately.
        let write_raw = write_fd.into_raw_fd();
        {
            // SAFETY: write_raw is the write end of our pipe, valid until close.
            let mut w = unsafe { std::fs::File::from_raw_fd(write_raw) };
            w.write_all(&self.payload).expect("write payload");
            // File::drop closes write_raw here.
        }
        let read_raw = read_fd.into_raw_fd();
        // SAFETY: read_raw is the read end of our pipe, transferred to OwnedFd.
        let output_reader = unsafe { OwnedFd::from_raw_fd(read_raw) };
        Ok(minibox_core::domain::SpawnResult {
            pid: u32::MAX, // fake PID; waitpid will return ECHILD → exit_code -1
            output_reader: Some(output_reader),
            runtime_id: None,
        })
    }
}

/// Regression: handle_run_streaming emits ContainerCreated as the FIRST message.
///
/// Before the fix, handle_run_streaming never emitted ContainerCreated at all —
/// the CLI had no way to learn the container ID until ContainerStopped.
/// This test verifies the correct streaming sequence:
///   ContainerCreated → ContainerOutput(s) → ContainerStopped
#[tokio::test]
#[cfg(unix)]
async fn test_handle_run_streaming_emits_container_created_first() {
    let payload = b"hello from container\n";
    let temp_dir = TempDir::new().expect("tempdir");
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"))
                    as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(PipedMockRuntime {
                payload: payload.to_vec(),
            }),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
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
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
    handler::handle_run(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/true".to_string()],
        None,
        None,
        true, // ephemeral — triggers streaming path
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

    // Collect all responses until channel closes.
    let mut responses = Vec::new();
    while let Some(r) = rx.recv().await {
        responses.push(r);
    }

    // Regression (1): ContainerCreated must be the very first message.
    assert!(
        !responses.is_empty(),
        "streaming run must send at least one response"
    );
    match &responses[0] {
        DaemonResponse::ContainerCreated { id } => {
            assert!(!id.is_empty(), "ContainerCreated id must not be empty");
        }
        other => {
            panic!("regression: first streaming message must be ContainerCreated, got: {other:?}")
        }
    }

    // Regression (2): ContainerStopped must be the last message (terminal).
    let last = responses.last().expect("at least one response");
    assert!(
        matches!(last, DaemonResponse::ContainerStopped { .. }),
        "last streaming message must be ContainerStopped, got: {last:?}"
    );

    // The output from the pipe must appear as ContainerOutput between the two.
    let output_bytes: Vec<u8> = responses
        .iter()
        .filter_map(|r| {
            if let DaemonResponse::ContainerOutput { data, .. } = r {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(data).ok()
            } else {
                None
            }
        })
        .flatten()
        .collect();
    assert_eq!(
        output_bytes, payload,
        "streaming output must match the pipe payload"
    );
}

// ---------------------------------------------------------------------------
// SIGTERM / process-group signal contract tests
// ---------------------------------------------------------------------------

/// `handle_stop` on a container whose PID is already dead succeeds immediately.
///
/// Documents the fast path: both the SIGTERM to the process group and the
/// `kill(pid, None)` probe return `ESRCH`, so the poll loop exits on the first
/// iteration and the container state transitions to "Stopped".
///
/// This is the common case for short-lived containers (e.g. `/bin/true`) that
/// finish before `stop` is called.
#[tokio::test]
#[cfg(unix)]
async fn test_stop_dead_pid_exits_immediately() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // PID 999_997 is virtually certain not to exist on any test host.
    let container_id = "sigterm_dead_pid01".to_string();
    state
        .add_container(ContainerRecord {
            info: ContainerInfo {
                id: container_id.clone(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/true".to_string(),
                state: "Running".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                pid: Some(999_997),
            },
            pid: Some(999_997),
            rootfs_path: std::path::PathBuf::from("/mock/rootfs"),
            cgroup_path: std::path::PathBuf::from("/mock/cgroup"),
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

    let before = tokio::time::Instant::now();
    let response = handler::handle_stop(container_id.clone(), state.clone(), deps).await;
    let elapsed = before.elapsed();

    // Must succeed.
    match response {
        DaemonResponse::Success { message } => {
            assert!(
                message.contains("stopped"),
                "expected 'stopped' in: {message}"
            );
        }
        other => panic!("expected Success, got: {other:?}"),
    }

    // Must be fast — dead PID exits the poll loop on the first probe (≤250 ms
    // sleep interval + small epsilon).  If this takes >1 s something is wrong.
    assert!(
        elapsed.as_secs() < 1,
        "stop of dead PID took {}ms, expected <1s",
        elapsed.as_millis()
    );

    let record = state
        .get_container(&container_id)
        .await
        .expect("container still in state");
    assert_eq!(record.info.state, "Stopped");
}

/// `handle_stop` on a container with no PID returns an error.
///
/// Documents the `record.pid.ok_or_else(...)` branch: a container in "Created"
/// state that was never started has no PID and cannot be signalled.
#[tokio::test]
#[cfg(unix)]
async fn test_stop_container_no_pid_returns_error() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let container_id = "sigterm_nopid0001".to_string();
    state
        .add_container(ContainerRecord {
            info: ContainerInfo {
                id: container_id.clone(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Created".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                pid: None,
            },
            pid: None,
            rootfs_path: std::path::PathBuf::from("/mock/rootfs"),
            cgroup_path: std::path::PathBuf::from("/mock/cgroup"),
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

    let response = handler::handle_stop(container_id, state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("no PID")
                    || message.contains("not running")
                    || message.contains("no pid"),
                "expected a 'no PID' / 'not running' error, got: {message}"
            );
        }
        other => panic!("expected Error, got: {other:?}"),
    }
}

/// Stopping a non-existent container returns an error.
///
/// Documents the `ContainerNotFound` path in `stop_inner`: if the container ID
/// has no record in state, the handler returns an `Error` response.
#[tokio::test]
#[cfg(unix)]
async fn test_stop_unknown_container_returns_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_stop("does_not_exist_xyz".to_string(), state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("not found")
                    || message.contains("NotFound")
                    || message.contains("does_not_exist"),
                "expected a 'not found' error, got: {message}"
            );
        }
        other => panic!("expected Error, got: {other:?}"),
    }
}

///
/// After `handle_run` returns `ContainerCreated`, the background
/// `daemon_wait_for_exit` task fires: `waitpid` on the mock PID returns an
/// error (ECHILD — process never existed), which triggers the abnormal-exit
/// branch that updates the container state to `"Stopped"` with exit_code = -1.
///
/// This test polls until the transition completes, covering the state-machine
/// path: Created → Running (inside spawn_blocking) → Stopped (on waitpid err).
#[tokio::test]
#[cfg(unix)]
async fn test_container_state_transitions_running_to_stopped_on_abnormal_exit() {
    let tmp = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&tmp);
    let deps = create_test_deps_with_dir(&tmp);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/true".to_string()],
        None,
        None,
        false, // non-ephemeral
        state.clone(),
        deps,
    )
    .await;

    let id = extract_container_id(&response);

    // Poll until the background wait task transitions the container to Stopped.
    let final_state = wait_for_container_state(&state, &id, "Stopped", 3000).await;

    assert_eq!(
        final_state, "Stopped",
        "container must reach Stopped state after abnormal exit (no real PID)"
    );
}
