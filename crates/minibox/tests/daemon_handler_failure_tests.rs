//! Failure, policy, and error-path tests for daemon handler.

use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox::daemon::handler::{
    self, BuildDeps, ContainerPolicy, EventDeps, ExecDeps, HandlerDependencies, ImageDeps,
    LifecycleDeps,
};
use minibox::daemon::state::DaemonState;
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::{BindMount, DynImageRegistry};
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

#[tokio::test]
async fn test_policy_denies_bind_mount_by_default() {
    let temp_dir = TempDir::new().unwrap();
    let deps = make_deps_with_policy(&temp_dir, ContainerPolicy::default());
    let state = create_test_state_with_dir(&temp_dir);

    let bind_mount = BindMount {
        host_path: std::path::PathBuf::from("/tmp/host"),
        container_path: std::path::PathBuf::from("/mnt/host"),
        read_only: false,
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        None,
        vec![bind_mount],
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
    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("bind mount") || message.contains("policy"),
                "expected policy error, got: {message}"
            );
        }
        _ => panic!("expected Error, got {resp:?}"),
    }
}

/// Default policy denies privileged containers.
#[tokio::test]
async fn test_policy_denies_privileged_by_default() {
    let temp_dir = TempDir::new().unwrap();
    let deps = make_deps_with_policy(&temp_dir, ContainerPolicy::default());
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        None,
        vec![],
        true, // privileged
        vec![],
        None,
        None,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("privileged") || message.contains("policy"),
                "expected policy error, got: {message}"
            );
        }
        _ => panic!("expected Error, got {resp:?}"),
    }
}

/// Default policy allows plain containers (no mounts, not privileged).
#[tokio::test]
async fn test_policy_allows_plain_container() {
    let temp_dir = TempDir::new().unwrap();
    let deps = make_deps_with_policy(&temp_dir, ContainerPolicy::default());
    let state = create_test_state_with_dir(&temp_dir);

    let resp = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    assert!(
        matches!(resp, DaemonResponse::ContainerCreated { .. }),
        "plain container should pass policy, got {resp:?}"
    );
}

/// Policy configured with allow_bind_mounts=true permits bind mounts.
#[tokio::test]
async fn test_policy_can_be_configured_to_allow_mounts() {
    let temp_dir = TempDir::new().unwrap();
    let policy = ContainerPolicy {
        allow_bind_mounts: true,
        allow_privileged: false,
    };
    let deps = make_deps_with_policy(&temp_dir, policy);
    let state = create_test_state_with_dir(&temp_dir);

    let bind_mount = BindMount {
        host_path: std::path::PathBuf::from("/tmp/host"),
        container_path: std::path::PathBuf::from("/mnt/host"),
        read_only: false,
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        None,
        vec![bind_mount],
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
        matches!(resp, DaemonResponse::ContainerCreated { .. }),
        "policy with allow_bind_mounts should permit bind mounts, got {resp:?}"
    );
}

/// Policy configured with allow_privileged=true permits privileged containers.
#[tokio::test]
async fn test_policy_can_be_configured_to_allow_privileged() {
    let temp_dir = TempDir::new().unwrap();
    let policy = ContainerPolicy {
        allow_bind_mounts: false,
        allow_privileged: true,
    };
    let deps = make_deps_with_policy(&temp_dir, policy);
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        None,
        vec![],
        true, // privileged
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
        matches!(resp, DaemonResponse::ContainerCreated { .. }),
        "policy with allow_privileged should permit privileged, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// validate_policy unit tests (#116, #123)
// ---------------------------------------------------------------------------

/// Plain container (no mounts, not privileged) passes any policy configuration.
#[test]
fn test_validate_policy_plain_container_always_allowed() {
    use minibox::daemon::handler::{ContainerPolicy, validate_policy};

    let policy = ContainerPolicy::default(); // deny-all defaults
    assert!(
        validate_policy(&[], false, &policy).is_ok(),
        "plain container must always pass policy"
    );
}

/// `validate_policy` rejects bind mounts when `allow_bind_mounts` is false.
#[test]
fn test_validate_policy_denies_bind_mount() {
    use minibox::daemon::handler::{ContainerPolicy, validate_policy};
    use minibox_core::domain::BindMount;

    let policy = ContainerPolicy {
        allow_bind_mounts: false,
        allow_privileged: false,
    };
    let mounts = vec![BindMount {
        host_path: std::path::PathBuf::from("/tmp/data"),
        container_path: std::path::PathBuf::from("/data"),
        read_only: false,
    }];
    let err = validate_policy(&mounts, false, &policy).unwrap_err();
    assert!(
        err.contains("bind mount") || err.contains("policy"),
        "expected bind-mount policy error, got: {err}"
    );
}

/// `validate_policy` rejects privileged=true when `allow_privileged` is false.
#[test]
fn test_validate_policy_denies_privileged() {
    use minibox::daemon::handler::{ContainerPolicy, validate_policy};

    let policy = ContainerPolicy {
        allow_bind_mounts: false,
        allow_privileged: false,
    };
    let err = validate_policy(&[], true, &policy).unwrap_err();
    assert!(
        err.contains("privileged") || err.contains("policy"),
        "expected privileged policy error, got: {err}"
    );
}

/// handle_run rejects a second container that tries to claim an already-used name.
#[tokio::test]
async fn test_handle_run_duplicate_container_name_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // First run — claims name "mybox".
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
        Some("mybox".to_string()),
        None,
        Arc::clone(&state),
        Arc::clone(&deps),
        tx1,
    )
    .await;
    let first = rx1.recv().await.expect("first run: no response");
    assert!(
        matches!(first, DaemonResponse::ContainerCreated { .. }),
        "first run should succeed, got {first:?}"
    );

    // Second run with the same name must be rejected.
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
        Some("mybox".to_string()),
        None,
        Arc::clone(&state),
        Arc::clone(&deps),
        tx2,
    )
    .await;
    let second = rx2.recv().await.expect("second run: no response");
    assert!(
        matches!(second, DaemonResponse::Error { .. }),
        "duplicate name should produce Error, got {second:?}"
    );
}
#[tokio::test]
async fn test_handle_run_filesystem_setup_failure_v2() {
    let temp_dir = TempDir::new().unwrap();
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images_fs")).unwrap());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new().with_setup_failure()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers_fs"),
            run_containers_base: temp_dir.path().join("run_fs"),
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
            allow_bind_mounts: false,
            allow_privileged: false,
        },
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    });
    let state = create_test_state_with_dir(&temp_dir);

    let resp = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "filesystem setup failure should produce Error, got {resp:?}"
    );
}

/// handle_run with a resource limiter that fails create → Error response.
#[tokio::test]
async fn test_handle_run_limiter_create_failure() {
    let temp_dir = TempDir::new().unwrap();
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images_lc")).unwrap());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new().with_create_failure()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers_lc"),
            run_containers_base: temp_dir.path().join("run_lc"),
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
            allow_bind_mounts: false,
            allow_privileged: false,
        },
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    });
    let state = create_test_state_with_dir(&temp_dir);

    let resp = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "limiter create failure should produce Error, got {resp:?}"
    );
}

/// handle_run — bind-mount denied by default policy → Error response.
#[tokio::test]
async fn test_handle_run_bind_mount_denied_by_policy() {
    let temp_dir = TempDir::new().unwrap();
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("images_bm")).unwrap(),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers_bm"),
            run_containers_base: temp_dir.path().join("run_bm"),
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
            allow_bind_mounts: false,
            allow_privileged: false,
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
        false,
        None,
        vec![BindMount {
            host_path: std::path::PathBuf::from("/tmp/data"),
            container_path: std::path::PathBuf::from("/data"),
            read_only: false,
        }],
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
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("bind mount")),
        "bind mount policy denial should produce Error, got {resp:?}"
    );
}

/// handle_run — privileged mode denied by default policy → Error response.
#[tokio::test]
async fn test_handle_run_privileged_denied_by_policy() {
    let temp_dir = TempDir::new().unwrap();
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("images_pr")).unwrap(),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers_pr"),
            run_containers_base: temp_dir.path().join("run_pr"),
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
            allow_bind_mounts: false,
            allow_privileged: false,
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
        false,
        None,
        vec![],
        true, // privileged=true, policy denies it
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
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("privileged")),
        "privileged policy denial should produce Error, got {resp:?}"
    );
}

// --- handle_pause: stopped container returns error ---

#[tokio::test]
async fn test_handle_pause_stopped_container_returns_not_running() {
    let temp_dir = TempDir::new().unwrap();
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
    let container_id = match rx_run.recv().await.unwrap() {
        DaemonResponse::ContainerCreated { id } => id,
        other => panic!("expected ContainerCreated, got {other:?}"),
    };

    let _ = handler::handle_stop(container_id.clone(), state.clone(), deps).await;

    let event_sink: Arc<dyn minibox_core::events::EventSink> =
        Arc::new(minibox_core::events::NoopEventSink);
    let resp = handler::handle_pause(container_id, state, event_sink).await;
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not running")),
        "expected 'not running' error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// Issue #158: Error-path and protocol compatibility coverage
// ---------------------------------------------------------------------------

/// Client drops the receiver before `handle_run` (ephemeral, pull-failure path)
/// can send its error.  The `warn` path in `send_error` must fire without panic.
///
/// Covers the `tx.send(...).await.is_err()` → warn path in `handle_run_streaming`
/// and `send_error` for the dropped-receiver case.
#[tokio::test]
#[cfg(unix)]
async fn test_handle_run_streaming_client_disconnect_does_not_panic() {
    let tmp = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&tmp);
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(tmp.path().join("img_disc")).unwrap(),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: tmp.path().join("containers_disc"),
            run_containers_base: tmp.path().join("run_disc"),
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
    // Drop the receiver immediately — streaming error path must not panic.
    drop(rx);

    handler::handle_run(
        "alpine".to_string(),
        None,
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
    // No panic = warn path exercised correctly.
}

/// Containers persisted with `state = "Running"` are NOT reattached after
/// daemon restart — their PID is stale and the record is visible but no live
/// process exists.
///
/// This covers the "persisted-but-not-reattached" behaviour described in the
/// CLAUDE.md limitations section and acceptance criterion (g) of issue #158.
#[tokio::test]
async fn test_persisted_running_container_not_reattached_after_restart() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let tmp = TempDir::new().unwrap();
    let container_id = "running-on-restart1".to_string();

    // Simulate first daemon: container was Running at shutdown.
    {
        let image_store = minibox_core::image::ImageStore::new(tmp.path().join("images")).unwrap();
        let state = DaemonState::new(image_store, tmp.path());
        state
            .add_container(ContainerRecord {
                info: ContainerInfo {
                    id: container_id.clone(),
                    name: None,
                    image: "alpine:latest".to_string(),
                    command: "/bin/sh".to_string(),
                    state: "Running".to_string(),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    pid: Some(999_997), // stale PID — process does not exist
                },
                pid: Some(999_997),
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
            })
            .await;
    }

    // Simulate second daemon: load state from disk.
    let image_store2 = minibox_core::image::ImageStore::new(tmp.path().join("images2")).unwrap();
    let state2 = DaemonState::new(image_store2, tmp.path());
    state2.load_from_disk().await;

    // Record is present — the daemon remembers the container.
    let record = state2
        .get_container(&container_id)
        .await
        .expect("persisted container must be visible after restart");

    // The record retains the stale state — reattachment is NOT performed.
    assert_eq!(
        record.info.id, container_id,
        "container ID must be preserved"
    );
    // PID is still present in the record (not cleared by load).
    // The process at that PID almost certainly doesn't exist, but the daemon
    // does not attempt to reattach — this is the documented limitation.
    assert_eq!(
        record.pid,
        Some(999_997),
        "stale PID must be preserved as-is (no reattach)"
    );
    // State is still "Running" from the previous daemon instance; it is NOT
    // automatically corrected to "Stopped" on load (no reattach = no correction).
    assert_eq!(
        record.info.state, "Running",
        "state must remain as persisted (no auto-correction on load)"
    );
}

// ---------------------------------------------------------------------------
// ContainerPolicy gate tests
// ---------------------------------------------------------------------------

/// Helper: call `handle_run` with specific mounts, privileged flag, and policy.
async fn handle_run_with_policy(
    mounts: Vec<minibox_core::domain::BindMount>,
    privileged: bool,
    policy: ContainerPolicy,
) -> DaemonResponse {
    let temp_dir = TempDir::new().expect("create temp dir");
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images"))
            .expect("create image store"),
    );
    let mock_registry = Arc::new(MockRegistry::new());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                mock_registry as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
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
            pty_sessions: Arc::new(tokio::sync::Mutex::new(
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
        policy,
        checkpoint: Arc::new(minibox_core::domain::NoopVmCheckpoint),
    });
    let state = Arc::new(DaemonState::new(
        minibox::image::ImageStore::new(temp_dir.path().join("images2")).expect("image store"),
        temp_dir.path(),
    ));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        None,
        mounts,
        privileged,
        vec![],
        None,
        None,
        state,
        deps,
        tx,
    )
    .await;
    rx.recv().await.expect("handler sent no response")
}

fn sample_bind_mount() -> minibox_core::domain::BindMount {
    minibox_core::domain::BindMount {
        host_path: std::path::PathBuf::from("/tmp/host"),
        container_path: std::path::PathBuf::from("/mnt/data"),
        read_only: false,
    }
}

#[tokio::test]
async fn test_policy_denies_privileged_by_default_via_helper() {
    let response = handle_run_with_policy(vec![], true, ContainerPolicy::default()).await;
    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("privileged"),
                "expected privileged policy error, got: {message}"
            );
        }
        other => panic!("expected Error response for denied privileged, got {other:?}"),
    }
}

#[tokio::test]
async fn test_policy_allows_bind_mount_when_permitted() {
    let policy = ContainerPolicy {
        allow_bind_mounts: true,
        allow_privileged: false,
    };
    let response = handle_run_with_policy(vec![sample_bind_mount()], false, policy).await;
    // Should NOT be a policy error (may be ContainerCreated or other non-policy error).
    if let DaemonResponse::Error { message } = &response {
        assert!(
            !message.contains("policy violation"),
            "bind mount should be allowed but got policy error: {message}"
        );
    }
}

#[tokio::test]
async fn test_policy_allows_privileged_when_permitted() {
    let policy = ContainerPolicy {
        allow_bind_mounts: false,
        allow_privileged: true,
    };
    let response = handle_run_with_policy(vec![], true, policy).await;
    if let DaemonResponse::Error { message } = &response {
        assert!(
            !message.contains("policy violation"),
            "privileged should be allowed but got policy error: {message}"
        );
    }
}

#[tokio::test]
async fn test_policy_denies_both_bind_mount_and_privileged() {
    let response =
        handle_run_with_policy(vec![sample_bind_mount()], true, ContainerPolicy::default()).await;
    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("policy violation"),
                "expected policy violation error, got: {message}"
            );
        }
        other => panic!("expected Error response, got {other:?}"),
    }
}

#[tokio::test]
async fn test_policy_empty_mounts_privileged_true_denied() {
    let response = handle_run_with_policy(vec![], true, ContainerPolicy::default()).await;
    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("privileged"),
                "expected privileged policy error, got: {message}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_policy_from_env_defaults() {
    let policy = ContainerPolicy::from_env();
    assert!(!policy.allow_bind_mounts, "default should deny bind mounts");
    assert!(!policy.allow_privileged, "default should deny privileged");
}

// ---------------------------------------------------------------------------
// Error-path tests: handle_pause / handle_resume
// ---------------------------------------------------------------------------

/// handle_pause with an unknown container ID returns Error with "not found".
#[tokio::test]
async fn test_handle_pause_container_not_found() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&temp_dir);
    let event_sink: Arc<dyn minibox_core::events::EventSink> =
        Arc::new(minibox_core::events::NoopEventSink);

    let resp =
        handler::handle_pause("nonexistent-container-id".to_string(), state, event_sink).await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not found")),
        "unknown container should produce Error with 'not found', got {resp:?}"
    );
}

/// handle_resume with an unknown container ID returns Error with "not found".
#[tokio::test]
async fn test_handle_resume_container_not_found() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&temp_dir);
    let event_sink: Arc<dyn minibox_core::events::EventSink> =
        Arc::new(minibox_core::events::NoopEventSink);

    let resp =
        handler::handle_resume("nonexistent-container-id".to_string(), state, event_sink).await;

    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("not found")),
        "unknown container should produce Error with 'not found', got {resp:?}"
    );
}
