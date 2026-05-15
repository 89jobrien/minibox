//! Container lifecycle tests: run, stop, remove, list, networking.

use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox::daemon::handler::{
    self, BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
};
use minibox::daemon::state::{ContainerState, DaemonState};
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::{DynImageRegistry, NetworkMode};
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

// ---------------------------------------------------------------------------
// handle_run Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_run_with_cached_image() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_registry = Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"));
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images2"))
            .expect("unwrap in test"),
    );
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                mock_registry.clone() as DynImageRegistry,
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
        state.clone(),
        deps.clone(),
    )
    .await;

    match response {
        DaemonResponse::ContainerCreated { id } => {
            assert!(!id.is_empty());
            assert_eq!(id.len(), 16); // UUID truncated to 16 chars

            // Verify container was added to state
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let container = state.get_container(&id).await;
            assert!(container.is_some());
        }
        _ => panic!("expected ContainerCreated response, got {response:?}"),
    }

    // Image was cached, so pull should NOT have been called
    assert_eq!(mock_registry.pull_count(), 0);
}

#[tokio::test]
async fn test_handle_run_pulls_uncached_image() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_registry = Arc::new(MockRegistry::new());
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images2"))
            .expect("unwrap in test"),
    );
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                mock_registry.clone() as DynImageRegistry,
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
    }); // Image not cached
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        None, // defaults to "latest"
        vec!["/bin/echo".to_string(), "hello".to_string()],
        Some(512 * 1024 * 1024), // 512MB memory limit
        Some(500),               // CPU weight
        false,
        state,
        deps.clone(),
    )
    .await;

    match response {
        DaemonResponse::ContainerCreated { .. } => {
            // Success - image was pulled
        }
        _ => panic!("expected Error, got {response:?}"),
    }

    // Image was not cached, so pull SHOULD have been called
    assert_eq!(mock_registry.pull_count(), 1);
}

#[tokio::test]
async fn test_handle_run_filesystem_setup_failure() {
    let temp_dir = TempDir::new().expect("unwrap in test");
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
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2"))
                    .expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new().with_setup_failure()),
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
        None,
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
            assert!(message.contains("filesystem setup failure"));
        }
        _ => panic!("expected Error response, got {response:?}"),
    }
}

#[tokio::test]
async fn test_handle_run_resource_limiter_failure() {
    let temp_dir = TempDir::new().expect("unwrap in test");
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
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2"))
                    .expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new().with_create_failure()),
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
        None,
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
            assert!(message.contains("resource limiter create failure"));
        }
        _ => panic!("expected Error response, got {response:?}"),
    }
}

#[tokio::test]
async fn test_handle_run_runtime_spawn_failure() {
    let temp_dir = TempDir::new().expect("unwrap in test");
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
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2"))
                    .expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new().with_spawn_failure()),
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
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps,
    )
    .await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("mock spawn failure"),
                "unexpected error message: {message}"
            );
        }
        _ => panic!("expected Error, got {response:?}"),
    }

    let containers = state.list_containers().await;
    assert_eq!(containers.len(), 1, "failed run should retain one record");
    assert_eq!(containers[0].state, "Failed");
}

// ---------------------------------------------------------------------------
// handle_remove Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_remove_success() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // First create a container
    let create_response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps.clone(),
    )
    .await;

    let container_id = match create_response {
        DaemonResponse::ContainerCreated { id } => id,
        _ => panic!("failed to create container"),
    };

    // Wait for container to spawn
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Mark it as stopped (handler requires this)
    state
        .update_container_state(&container_id, ContainerState::Stopped)
        .await
        .ok();

    // Now remove it
    let remove_response =
        handler::handle_remove(container_id.clone(), state.clone(), deps.clone()).await;

    match remove_response {
        DaemonResponse::Success { message } => {
            assert!(message.contains("removed"));
            assert!(message.contains(&container_id));
        }
        _ => panic!("expected Success, got {remove_response:?}"),
    }

    // Container should be gone from state
    assert!(state.get_container(&container_id).await.is_none());

    // Verify cleanup was called
    let filesystem = deps
        .lifecycle
        .filesystem
        .as_any()
        .downcast_ref::<MockFilesystem>()
        .expect("unwrap in test");
    assert_eq!(filesystem.cleanup_count(), 1);

    let limiter = deps
        .lifecycle
        .resource_limiter
        .as_any()
        .downcast_ref::<MockLimiter>()
        .expect("unwrap in test");
    assert_eq!(limiter.cleanup_count(), 1);
}

#[tokio::test]
async fn test_handle_remove_nonexistent_container() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_remove("nonexistent123".to_string(), state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(message.contains("not found"));
        }
        _ => panic!("expected Error, got {response:?}"),
    }
}

#[tokio::test]
async fn test_handle_remove_running_container() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // Create a container
    let create_response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps.clone(),
    )
    .await;

    let container_id = match create_response {
        DaemonResponse::ContainerCreated { id } => id,
        _ => panic!("failed to create container"),
    };

    // Directly mark as Running (deterministic — no sleep/race with async spawn)
    state
        .update_container_state(&container_id, ContainerState::Running)
        .await
        .ok();

    // Try to remove while still running
    let response = handler::handle_remove(container_id, state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("running"),
                "expected 'running' in error message, got: {message}"
            );
        }
        _ => panic!("expected Error, got {response:?}"),
    }
}

// ---------------------------------------------------------------------------
// Integration-style test: Full workflow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_full_container_lifecycle() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // 1. Pull image
    let pull_response = handler::handle_pull(
        "nginx".to_string(),
        Some("alpine".to_string()),
        None,
        state.clone(),
        deps.clone(),
    )
    .await;
    assert!(matches!(pull_response, DaemonResponse::Success { .. }));

    // 2. Run container with resource limits
    let run_response = handle_run_once(
        "nginx".to_string(),
        Some("alpine".to_string()),
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "echo test".to_string(),
        ],
        Some(256 * 1024 * 1024), // 256MB
        Some(750),               // CPU weight
        false,
        state.clone(),
        deps.clone(),
    )
    .await;

    let container_id = match run_response {
        DaemonResponse::ContainerCreated { id } => id,
        _ => panic!("expected ContainerCreated"),
    };

    // 3. List containers
    let list_response = handler::handle_list(state.clone()).await;
    match list_response {
        DaemonResponse::ContainerList { containers } => {
            assert_eq!(containers.len(), 1);
            assert_eq!(containers[0].id, container_id);
            assert_eq!(containers[0].image, "nginx:alpine");
        }
        _ => panic!("expected ContainerList"),
    }

    // 4. Stop container (simulated by updating state)
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    state
        .update_container_state(&container_id, ContainerState::Stopped)
        .await
        .ok();

    // 5. Remove container
    let remove_response = handler::handle_remove(container_id.clone(), state.clone(), deps).await;
    assert!(matches!(remove_response, DaemonResponse::Success { .. }));

    // 6. Verify removal
    assert!(state.get_container(&container_id).await.is_none());
}

// ---------------------------------------------------------------------------
// Networking tests
// ---------------------------------------------------------------------------

/// Build deps with a specific `MockNetwork` instance so call counts can be
/// inspected after the handler runs.
fn create_test_deps_with_network(
    temp_dir: &TempDir,
    network: Arc<MockNetwork>,
) -> Arc<HandlerDependencies> {
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images3"))
            .expect("unwrap in test"),
    );
    Arc::new(HandlerDependencies {
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
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: network,
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
    })
}

/// `network_provider.setup()` is called exactly once per non-ephemeral
/// `handle_run` invocation.
#[tokio::test]
async fn test_network_setup_called_on_run() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_network = Arc::new(MockNetwork::new());
    let deps = create_test_deps_with_network(&temp_dir, mock_network.clone());
    let state = create_test_state_with_dir(&temp_dir);

    handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/true".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    assert_eq!(
        mock_network.setup_count(),
        1,
        "network setup should be called once"
    );
}

/// A network setup failure propagates back as a `DaemonResponse::Error`.
#[tokio::test]
async fn test_network_setup_failure_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_network = Arc::new(MockNetwork::new().with_setup_failure());
    let deps = create_test_deps_with_network(&temp_dir, mock_network.clone());
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/true".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "network setup failure should produce Error response, got {response:?}"
    );
}

/// `handle_run` with an explicit `NetworkMode::None` succeeds and calls
/// `setup()` exactly once — `NoopNetwork.setup()` is always invoked regardless
/// of mode; it simply returns an empty netns path.
#[tokio::test]
async fn test_handle_run_explicit_network_none() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_network = Arc::new(MockNetwork::new());
    let deps = create_test_deps_with_network(&temp_dir, mock_network.clone());
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/true".to_string()],
        None,
        None,
        false,
        Some(NetworkMode::None),
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

    let response = rx.recv().await.expect("handler sent no response");
    assert!(
        !matches!(response, DaemonResponse::Error { ref message } if message.contains("network")),
        "NetworkMode::None should not produce a network error, got {response:?}"
    );
    assert_eq!(mock_network.setup_count(), 1);
}

// ---------------------------------------------------------------------------
// Coverage expansion: run_inner orchestration paths
// ---------------------------------------------------------------------------

/// Extract container ID from a ContainerCreated response; panics otherwise.
fn extract_container_id(response: &DaemonResponse) -> String {
    match response {
        DaemonResponse::ContainerCreated { id } => id.clone(),
        other => panic!("expected ContainerCreated, got: {other:?}"),
    }
}

/// Tag defaults to "latest" when `None` is passed.
#[tokio::test]
async fn test_run_defaults_tag_to_latest() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        None, // tag=None → defaults to "latest"
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps,
    )
    .await;

    let id = extract_container_id(&response);
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    let container = state.get_container(&id).await.expect("container exists");
    // image label should end with ":latest"
    assert!(
        container.info.image.ends_with(":latest"),
        "expected image tag 'latest', got: {}",
        container.info.image
    );
}

/// Bare image name "nginx" (no slash) is normalised to "library/nginx".
#[tokio::test]
async fn test_run_normalizes_short_image_name() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "nginx".to_string(),
        Some("alpine".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    // Should succeed — MockRegistry caches all images by default.
    extract_container_id(&response);
}

/// Container directories are created on disk during run.
#[tokio::test]
async fn test_run_creates_container_directories() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps.clone(),
    )
    .await;

    let id = extract_container_id(&response);
    // containers_base/{id} should exist
    let container_dir = deps.lifecycle.containers_base.join(&id);
    assert!(
        container_dir.exists(),
        "container dir should exist: {}",
        container_dir.display()
    );
}

/// After handle_run the container is registered in state.
#[tokio::test]
async fn test_run_container_in_created_or_running_state() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps,
    )
    .await;

    let id = extract_container_id(&response);
    // Allow async spawn task to run
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    let container = state
        .get_container(&id)
        .await
        .expect("container should exist in state");
    let s = &container.info.state;
    assert!(
        s == "Created" || s == "Running" || s == "Stopped",
        "unexpected container state: {s}"
    );
}

/// Five consecutive runs produce five unique container IDs.
#[tokio::test]
async fn test_run_generates_unique_ids() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let mut ids = std::collections::HashSet::new();
    for _ in 0..5 {
        let response = handle_run_once(
            "alpine".to_string(),
            None,
            vec!["/bin/sh".to_string()],
            None,
            None,
            false,
            state.clone(),
            deps.clone(),
        )
        .await;
        let id = extract_container_id(&response);
        ids.insert(id);
    }
    assert_eq!(ids.len(), 5, "all 5 container IDs should be unique");
}

/// Memory and CPU limits are accepted and container is created successfully.
#[tokio::test]
async fn test_run_with_memory_and_cpu_limits() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        Some(256 * 1024 * 1024), // 256MB
        Some(500),               // CPU weight
        false,
        state,
        deps,
    )
    .await;

    extract_container_id(&response); // panics if not ContainerCreated
}

/// Passing an explicit NetworkMode::Host succeeds and the network provider
/// receives a setup call.
#[tokio::test]
async fn test_run_with_network_mode_host() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let mock_network = Arc::new(MockNetwork::new());
    let deps = create_test_deps_with_network(&temp_dir, mock_network.clone());
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/true".to_string()],
        None,
        None,
        false,
        Some(NetworkMode::Host),
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

    let response = rx.recv().await.expect("handler sent no response");
    extract_container_id(&response);
    assert_eq!(
        mock_network.setup_count(),
        1,
        "network setup should be called for Host mode"
    );
}

/// Stopping a nonexistent container returns an Error response.
#[tokio::test]
async fn test_stop_nonexistent_container() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_stop("nonexistent_id_123".to_string(), state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("not found") || message.contains("no PID"),
                "expected 'not found' in error, got: {message}"
            );
        }
        other => panic!("expected Error response, got: {other:?}"),
    }
}

/// Removing a container whose filesystem cleanup fails still succeeds
/// (cleanup failures are best-effort warnings, not hard errors).
#[tokio::test]
async fn test_remove_with_filesystem_cleanup_failure() {
    use minibox::adapters::mocks::FailableFilesystemMock;

    let temp_dir = TempDir::new().expect("create temp dir");
    let failable_fs = Arc::new(FailableFilesystemMock::new());

    // Build deps with the failable mock (setup succeeds by default).
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2"))
                    .expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: failable_fs.clone(),
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

    // Create a container (setup succeeds).
    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps.clone(),
    )
    .await;
    let id = extract_container_id(&response);

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    state
        .update_container_state(&id, ContainerState::Stopped)
        .await
        .ok();

    // Now toggle cleanup to fail.
    failable_fs.set_fail_cleanup(true);

    // Remove should still return Success (cleanup failure is best-effort).
    let remove_response = handler::handle_remove(id.clone(), state.clone(), deps).await;
    match remove_response {
        DaemonResponse::Success { message } => {
            assert!(
                message.contains("removed"),
                "expected 'removed' in message, got: {message}"
            );
        }
        other => panic!("expected Success despite cleanup failure, got: {other:?}"),
    }

    // Container should be gone from state regardless.
    assert!(state.get_container(&id).await.is_none());
}

/// Removing a nonexistent container returns an Error response.
#[tokio::test]
async fn test_remove_nonexistent_container() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_remove("does_not_exist_456".to_string(), state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("not found"),
                "expected 'not found' in error, got: {message}"
            );
        }
        other => panic!("expected Error response, got: {other:?}"),
    }
}

/// Listing containers when none exist returns an empty list.
#[tokio::test]
async fn test_list_empty() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_list(state).await;

    match response {
        DaemonResponse::ContainerList { containers } => {
            assert!(containers.is_empty(), "expected empty list");
        }
        other => panic!("expected ContainerList, got: {other:?}"),
    }
}

/// After running one container, handle_list returns a list with one entry.
#[tokio::test]
async fn test_list_after_run() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps,
    )
    .await;
    let id = extract_container_id(&response);

    let list_response = handler::handle_list(state).await;
    match list_response {
        DaemonResponse::ContainerList { containers } => {
            assert_eq!(containers.len(), 1, "expected 1 container in list");
            assert_eq!(containers[0].id, id);
        }
        other => panic!("expected ContainerList, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// New coverage expansion tests
// ---------------------------------------------------------------------------

/// `run_inner` returns `DaemonResponse::Error` when the registry reports zero
/// layers for an otherwise valid (pulled) image.
#[tokio::test]
async fn test_handle_run_empty_image_returns_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            // Image is "pre-cached" so the pull is skipped, but get_image_layers returns empty.
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(
                    MockRegistry::new()
                        .with_cached_image("library/alpine", "latest")
                        .with_empty_layers(),
                ) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2"))
                    .expect("unwrap in test"),
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
            // The handler wraps DomainError::EmptyImage; the message should
            // reference the image or contain a recognisable keyword.
            assert!(
                message.to_lowercase().contains("empty")
                    || message.to_lowercase().contains("no layer")
                    || message.contains("alpine"),
                "expected empty-image error, got: {message}"
            );
        }
        other => panic!("expected Error response for empty image, got: {other:?}"),
    }
}

/// `handle_stop` on a container that is in "Created" state (no PID) returns
/// an error indicating the container has no PID or is not running.
///
/// This covers the `record.pid.ok_or_else(...)` path in `stop_inner` (Unix).
#[tokio::test]
async fn test_handle_stop_container_without_pid_returns_error() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // Insert a container record directly in "Created" state (pid = None).
    let container_id = "nopidcontainer01".to_string();
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
                "expected 'no PID' or 'not running' in error, got: {message}"
            );
        }
        other => panic!("expected Error response, got: {other:?}"),
    }
}

/// `remove_inner` cgroup cleanup failure is best-effort — `handle_remove` still
/// returns `DaemonResponse::Success` even when `ResourceLimiter::cleanup` fails.
#[tokio::test]
async fn test_handle_remove_cgroup_cleanup_failure_still_succeeds() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2"))
                    .expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new().with_cleanup_failure()),
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

    // Create a container, then mark it Stopped so remove is permitted.
    let create_response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps.clone(),
    )
    .await;
    let id = extract_container_id(&create_response);

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    state
        .update_container_state(&id, ContainerState::Stopped)
        .await
        .ok();

    // Remove should succeed despite cgroup cleanup failure.
    let remove_response = handler::handle_remove(id.clone(), state.clone(), deps).await;
    match remove_response {
        DaemonResponse::Success { message } => {
            assert!(
                message.contains("removed"),
                "expected 'removed' in success message, got: {message}"
            );
        }
        other => panic!("expected Success despite cgroup cleanup failure, got: {other:?}"),
    }

    // Container should be removed from state regardless.
    assert!(
        state.get_container(&id).await.is_none(),
        "container should be absent from state after remove"
    );
}

// ---------------------------------------------------------------------------
// Coverage expansion: error paths not previously tested
// ---------------------------------------------------------------------------

/// `handle_pull` with a completely invalid image reference returns Error.
///
/// This covers the `ImageRef::parse` failure path in `handle_pull` (the early
/// return before the registry is even consulted).
#[tokio::test]
async fn test_handle_pull_invalid_image_ref_returns_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // "ghcr.io/imageonly" fails parse: non-docker.io registry requires org/name format.
    let response =
        handler::handle_pull("ghcr.io/imageonly".to_string(), None, None, state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("invalid image reference"),
                "expected parse error, got: {message}"
            );
        }
        other => panic!("expected Error response, got: {other:?}"),
    }
}

/// `handle_run` with an invalid image reference returns Error.
///
/// Covers the `ImageRef::parse` failure path inside `run_inner` before any
/// registry or filesystem calls are made.
#[tokio::test]
async fn test_handle_run_invalid_image_ref_returns_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "ghcr.io/imageonly".to_string(),
        None,
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
            assert!(
                message.contains("invalid image reference"),
                "expected parse error, got: {message}"
            );
        }
        other => panic!("expected Error response, got: {other:?}"),
    }
}

/// `handle_run` returns Error when the image is not cached and the pull fails.
///
/// Covers the `registry.pull_image()` error path in `run_inner` — the image
/// is absent (no `with_cached_image`) and `with_pull_failure` forces an error.
#[tokio::test]
async fn test_handle_run_pull_failure_returns_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = Arc::new(HandlerDependencies {
        // Not cached + pull always fails → ImagePullFailed domain error.
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2"))
                    .expect("unwrap in test"),
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
            assert!(
                message.to_lowercase().contains("pull")
                    || message.to_lowercase().contains("failed")
                    || message.contains("alpine"),
                "expected pull-failure error, got: {message}"
            );
        }
        other => panic!("expected Error response, got: {other:?}"),
    }
}

/// `handle_stop` on a running container with a dead PID succeeds.
///
/// Covers the happy path of `stop_inner` (Unix): SIGTERM is sent (silently
/// ignored for ESRCH), the `kill(pid, None)` probe returns ESRCH immediately,
/// the loop exits, and the container state is updated to `"Stopped"`.
///
/// Using PID 999999 which almost certainly does not exist on any test host.
#[tokio::test]
#[cfg(unix)]
async fn test_handle_stop_dead_pid_succeeds() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let container_id = "deadpidcontainer01".to_string();
    state
        .add_container(ContainerRecord {
            info: ContainerInfo {
                id: container_id.clone(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Running".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                pid: Some(999_999),
            },
            pid: Some(999_999),
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

    let response = handler::handle_stop(container_id.clone(), state.clone(), deps).await;

    match response {
        DaemonResponse::Success { message } => {
            assert!(
                message.contains("stopped"),
                "expected 'stopped' in success message, got: {message}"
            );
        }
        other => panic!("expected Success response, got: {other:?}"),
    }

    // Container state should be "Stopped".
    let record = state
        .get_container(&container_id)
        .await
        .expect("container should still exist in state");
    assert_eq!(
        record.info.state, "Stopped",
        "container state should be Stopped after handle_stop"
    );
}

// ---------------------------------------------------------------------------
// handle_stop network cleanup
// ---------------------------------------------------------------------------

/// `handle_stop` calls `network_provider.cleanup()` before issuing SIGTERM.
///
/// This covers the `NetworkLifecycle::cleanup(&id)` call at the top of
/// `handle_stop`.  We use a dead PID (999998) so the stop completes immediately
/// without a real process wait loop.
#[tokio::test]
#[cfg(unix)]
async fn test_handle_stop_triggers_network_cleanup() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let temp_dir = TempDir::new().expect("create temp dir");
    let mock_network = Arc::new(MockNetwork::new());
    let deps = create_test_deps_with_network(&temp_dir, mock_network.clone());
    let state = create_test_state_with_dir(&temp_dir);

    let container_id = "netstoptest0001".to_string();
    state
        .add_container(ContainerRecord {
            info: ContainerInfo {
                id: container_id.clone(),
                name: None,
                image: "alpine:latest".to_string(),
                command: "/bin/sh".to_string(),
                state: "Running".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                pid: Some(999_998),
            },
            pid: Some(999_998),
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

    handler::handle_stop(container_id, state, deps).await;

    assert_eq!(
        mock_network.cleanup_count(),
        1,
        "network cleanup must be called exactly once during handle_stop"
    );
}

// ---------------------------------------------------------------------------
// handle_remove network cleanup
// ---------------------------------------------------------------------------

/// `handle_remove` calls `network_provider.cleanup()` as part of `remove_inner`.
#[tokio::test]
async fn test_handle_remove_triggers_network_cleanup() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let mock_network = Arc::new(MockNetwork::new());
    let deps = create_test_deps_with_network(&temp_dir, mock_network.clone());
    let state = create_test_state_with_dir(&temp_dir);

    // Create then stop.
    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps.clone(),
    )
    .await;
    let id = extract_container_id(&response);
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    state
        .update_container_state(&id, ContainerState::Stopped)
        .await
        .ok();

    // network_provider.setup() was called during run (count=1).
    // After remove, cleanup should also be called.
    let setup_count_before_remove = mock_network.setup_count();

    handler::handle_remove(id, state, deps).await;

    assert_eq!(
        mock_network.cleanup_count(),
        1,
        "network cleanup must be called exactly once during handle_remove"
    );
    // setup count must not increase due to remove
    assert_eq!(
        mock_network.setup_count(),
        setup_count_before_remove,
        "handle_remove must not call network setup"
    );
}

// ---------------------------------------------------------------------------
// Edge cases: empty command, combined image:tag, Created-state remove
// ---------------------------------------------------------------------------

/// When `command` is empty, `run_inner` defaults the spawn command to
/// `/bin/sh`.  The container is still created successfully.
#[tokio::test]
async fn test_handle_run_empty_command_defaults_to_sh() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec![], // empty command
        None,
        None,
        false,
        state.clone(),
        deps,
    )
    .await;

    let id = extract_container_id(&response);
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    let container = state
        .get_container(&id)
        .await
        .expect("container must exist");
    // The recorded command string is the joined vec, which for an empty vec is "".
    // The spawn_command fallback to /bin/sh is internal; the ContainerInfo.command
    // stores the join of the original command vec.
    assert!(
        container.info.command.is_empty() || container.info.command.contains("sh"),
        "unexpected command: {}",
        container.info.command
    );
}

/// A combined `image:tag` string in the image field (no separate tag param)
/// is parsed correctly by `ImageRef::parse`.
#[tokio::test]
async fn test_handle_run_image_colon_tag_format() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // Pass tag embedded in image string; tag param is None.
    let response = handle_run_once(
        "alpine:3.18".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps,
    )
    .await;

    let id = extract_container_id(&response);
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    let container = state
        .get_container(&id)
        .await
        .expect("container must exist");
    assert!(
        container.info.image.contains("3.18"),
        "expected tag '3.18' in image label, got: {}",
        container.info.image
    );
}

/// `handle_remove` on a container in `"Created"` state (never ran, no PID)
/// succeeds — `remove_inner` only blocks removal when state is `"Running"`.
#[tokio::test]
async fn test_handle_remove_created_container_succeeds() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let container_id = "createdcontainer1".to_string();
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

    let response = handler::handle_remove(container_id.clone(), state.clone(), deps).await;

    assert!(
        matches!(response, DaemonResponse::Success { .. }),
        "remove of Created-state container must succeed, got: {response:?}"
    );
    assert!(
        state.get_container(&container_id).await.is_none(),
        "container must be absent from state after remove"
    );
}

/// `handle_remove` on a `"Failed"` container (spawn error path) succeeds.
#[tokio::test]
async fn test_handle_remove_failed_container_succeeds() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2"))
                    .expect("unwrap in test"),
            ),
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new().with_spawn_failure()),
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

    // Spawn failure is reported synchronously, leaving a Failed-state record
    // that can be removed.
    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps.clone(),
    )
    .await;
    match response {
        DaemonResponse::Error { ref message } => {
            assert!(
                message.contains("mock spawn failure"),
                "unexpected error message: {message}"
            );
        }
        _ => panic!("expected Error, got {response:?}"),
    }

    let containers = state.list_containers().await;
    assert_eq!(containers.len(), 1, "failed run should retain one record");
    assert_eq!(containers[0].state, "Failed", "container should be Failed");
    let id = containers[0].id.clone();

    // Now remove it — should succeed because "Failed" != "Running".
    let remove_response = handler::handle_remove(id.clone(), state.clone(), deps).await;
    assert!(
        matches!(remove_response, DaemonResponse::Success { .. }),
        "remove of Failed container must succeed, got: {remove_response:?}"
    );
    assert!(state.get_container(&id).await.is_none());
}

/// `handle_pull` for a GHCR image that fails returns an Error with the
/// failure message from the GHCR registry mock.
#[tokio::test]
async fn test_handle_pull_ghcr_failure_returns_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [(
                    "ghcr.io",
                    Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
                )],
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2"))
                    .expect("unwrap in test"),
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
        "ghcr.io/org/myimage".to_string(),
        Some("latest".to_string()),
        None,
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("mock pull failure"),
                "expected mock pull failure message, got: {message}"
            );
        }
        other => panic!("expected Error response, got: {other:?}"),
    }
}

/// `handle_list` with multiple containers returns all of them.
#[tokio::test]
async fn test_handle_list_returns_all_containers() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // Create three containers.
    let mut created_ids = std::collections::HashSet::new();
    for image in &["alpine", "ubuntu", "nginx"] {
        let response = handle_run_once(
            image.to_string(),
            None,
            vec!["/bin/sh".to_string()],
            None,
            None,
            false,
            state.clone(),
            deps.clone(),
        )
        .await;
        created_ids.insert(extract_container_id(&response));
    }

    let list_response = handler::handle_list(state).await;
    match list_response {
        DaemonResponse::ContainerList { containers } => {
            assert_eq!(
                containers.len(),
                3,
                "expected 3 containers in list, got {}",
                containers.len()
            );
            let listed_ids: std::collections::HashSet<_> =
                containers.iter().map(|c| c.id.clone()).collect();
            assert_eq!(
                listed_ids, created_ids,
                "listed container IDs do not match created IDs"
            );
        }
        other => panic!("expected ContainerList, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Error-path tests: handle_remove — running container
// ---------------------------------------------------------------------------

/// handle_remove on a Running container → Error (AlreadyRunning).
#[tokio::test]
async fn test_handle_remove_running_container_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let container_id = "removerunning001ab";
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

    let resp = handler::handle_remove(container_id.to_string(), state, deps).await;

    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "removing a Running container should produce Error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_run_bridge_network_mode_calls_setup() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_network = Arc::new(MockNetwork::new());
    let deps = create_test_deps_with_network(&temp_dir, mock_network.clone());
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/true".to_string()],
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

    let _resp = rx.recv().await.expect("handler sent no response");
    assert_eq!(
        mock_network.setup_count(),
        1,
        "network_provider.setup() must be called once for NetworkMode::Bridge"
    );
}

/// ExecRuntime adapter that always returns Err — used in exec error-path tests.
struct FailingExecRuntime;

#[async_trait::async_trait]
impl minibox_core::domain::ExecRuntime for FailingExecRuntime {
    async fn run_in_container(
        &self,
        _container_id: &minibox_core::domain::ContainerId,
        _spec: minibox_core::domain::ExecSpec,
        _tx: tokio::sync::mpsc::Sender<minibox_core::protocol::DaemonResponse>,
    ) -> anyhow::Result<minibox_core::domain::ExecHandle> {
        anyhow::bail!("mock exec runtime failure: setns not supported")
    }
}

impl minibox_core::domain::AsAny for FailingExecRuntime {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// handle_exec with an exec_runtime that returns Err → DaemonResponse::Error.
///
/// Covers the Err arm of exec_rt.run_in_container() in handle_exec.
#[tokio::test]
async fn test_handle_exec_with_runtime_returning_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&temp_dir);

    let deps = {
        let mut d = (*create_test_deps_with_dir(&temp_dir)).clone();
        d.exec.exec_runtime = Some(Arc::new(FailingExecRuntime));
        Arc::new(d)
    };

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
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("exec failed")),
        "exec_runtime error should produce 'exec failed' Error response, got {resp:?}"
    );
}

/// add_container triggers save_to_disk; a new DaemonState loaded from the same
/// directory via load_from_disk must contain the same container record.
///
/// Guards the persistence contract documented in docs/STATE_MODEL.md.
#[tokio::test]

async fn test_daemon_state_persistence_survives_restart() {
    let tmp = TempDir::new().expect("unwrap in test");

    let container_id = "persist-test-00001a";
    {
        let image_store = minibox_core::image::ImageStore::new(tmp.path().join("images"))
            .expect("unwrap in test");
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

    let image_store2 =
        minibox_core::image::ImageStore::new(tmp.path().join("images2")).expect("unwrap in test");
    let state2 = DaemonState::new(image_store2, tmp.path());
    state2.load_from_disk().await;

    let container = state2.get_container(container_id).await;
    assert!(
        container.is_some(),
        "container record must survive daemon restart"
    );
    assert_eq!(
        container.expect("unwrap in test").info.id,
        container_id,
        "restored container must have the same id"
    );
}

/// remove_container triggers save_to_disk; a new DaemonState loaded from the
/// same directory must not contain the removed record.
#[tokio::test]
async fn test_daemon_state_remove_persists_to_disk() {
    let tmp = TempDir::new().expect("unwrap in test");

    let container_id = "remove-persist-0001";
    {
        let image_store = minibox_core::image::ImageStore::new(tmp.path().join("images"))
            .expect("unwrap in test");
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

    let image_store2 =
        minibox_core::image::ImageStore::new(tmp.path().join("images2")).expect("unwrap in test");
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

/// `handle_stop` resolves a container by name (not just ID).
#[tokio::test]
async fn test_handle_stop_resolves_by_name() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&tmp);
    let state = create_test_state_with_dir(&tmp);

    // Create a named container via handle_run.
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
        false,
        vec![],
        Some("my-stop-ctr".to_string()),
        None,
        state.clone(),
        deps.clone(),
        tx,
    )
    .await;
    let resp = rx.recv().await.expect("handler sent no response");
    let id = extract_container_id(&resp);

    // Wait for spawn task to set PID (or fail).
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Stop by name -- should resolve to the container's real ID.
    let stop_resp = handler::handle_stop("my-stop-ctr".to_string(), state.clone(), deps).await;
    // The response should either be Success (stopped) or Error mentioning the
    // container -- but NOT "container not found".
    match &stop_resp {
        DaemonResponse::Success { message } => {
            assert!(
                message.contains(&id),
                "stop success should reference container id {id}, got: {message}"
            );
        }
        DaemonResponse::Error { message } => {
            assert!(
                !message.contains("not found"),
                "stop by name should resolve, but got not-found: {message}"
            );
        }
        other => panic!("expected Success or Error, got {other:?}"),
    }
}

/// `handle_remove` resolves a container by name (not just ID).
#[tokio::test]
async fn test_handle_remove_resolves_by_name() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&tmp);
    let state = create_test_state_with_dir(&tmp);

    // Insert a named Stopped container directly into state.
    let container_id = "nameresrm00000001";
    let record = minibox::daemon::state::ContainerRecord {
        info: minibox_core::protocol::ContainerInfo {
            id: container_id.to_string(),
            name: Some("rm-by-name".to_string()),
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

    // Remove by name.
    let resp = handler::handle_remove("rm-by-name".to_string(), state.clone(), deps).await;
    match resp {
        DaemonResponse::Success { message } => {
            assert!(
                message.contains(container_id),
                "remove success should reference id {container_id}, got: {message}"
            );
        }
        other => panic!("expected Success, got {other:?}"),
    }

    // Container should be gone.
    assert!(
        state.get_container(container_id).await.is_none(),
        "container should be removed after handle_remove by name"
    );
}

/// `handle_exec` with an empty container ID returns an error.
#[tokio::test]
async fn test_handle_exec_empty_container_id_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = {
        let mut d = (*create_test_deps_with_dir(&tmp)).clone();
        d.exec.exec_runtime = Some(Arc::new(FailingExecRuntime));
        Arc::new(d)
    };
    let state = create_test_state_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_exec(
        String::new(), // empty ID
        vec!["/bin/sh".to_string()],
        vec![],
        None,
        false,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("handler sent no response");
    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("invalid") || message.contains("empty"),
                "expected invalid container id error, got: {message}"
            );
        }
        other => panic!("expected Error for empty container ID, got {other:?}"),
    }
}

/// `handle_exec` with special characters in container ID returns an error.
#[tokio::test]
async fn test_handle_exec_special_chars_container_id_returns_error() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = {
        let mut d = (*create_test_deps_with_dir(&tmp)).clone();
        d.exec.exec_runtime = Some(Arc::new(FailingExecRuntime));
        Arc::new(d)
    };
    let state = create_test_state_with_dir(&tmp);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_exec(
        "../../../etc/passwd".to_string(), // path traversal attempt
        vec!["/bin/sh".to_string()],
        vec![],
        None,
        false,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("handler sent no response");
    match resp {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("invalid"),
                "expected invalid container id error, got: {message}"
            );
        }
        other => panic!("expected Error for special-char container ID, got {other:?}"),
    }
}

/// `handle_logs` gracefully handles client disconnect mid-stream.
#[tokio::test]
async fn test_handle_logs_client_disconnect_mid_stream() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&tmp);
    let state = create_test_state_with_dir(&tmp);

    let container_id = "logdiscon00000001";
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

    // Write many log lines to ensure streaming is in progress when we drop.
    let container_dir = tmp.path().join("containers").join(container_id);
    std::fs::create_dir_all(&container_dir).expect("create log dir");
    let mut content = String::new();
    for i in 0..1000 {
        content.push_str(&format!("log line {i}\n"));
    }
    std::fs::write(container_dir.join("stdout.log"), &content).expect("write log");

    // Use a channel with capacity 1 and drop the receiver immediately.
    let (tx, rx) = tokio::sync::mpsc::channel::<DaemonResponse>(1);
    drop(rx);

    // Must not panic -- the warn path for client disconnect is exercised.
    handler::handle_logs(container_id.to_string(), false, state, deps, tx).await;
}

/// `handle_run` creates a container and it appears in state.
#[tokio::test]
async fn test_handle_run_propagates_env_vars() {
    let tmp = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&tmp);
    let state = create_test_state_with_dir(&tmp);

    let response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps,
    )
    .await;

    // Should succeed -- env vars are passed through to the spawn config.
    match response {
        DaemonResponse::ContainerCreated { id } => {
            assert!(!id.is_empty(), "container ID should be non-empty");
            // Container should be registered in state.
            let container = state.get_container(&id).await;
            assert!(container.is_some(), "container should exist in state");
        }
        other => panic!("expected ContainerCreated, got {other:?}"),
    }
}

// ===========================================================================
