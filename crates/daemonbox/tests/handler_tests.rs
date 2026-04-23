//! Unit tests for daemon handlers using mock adapters.
//!
//! These tests demonstrate the testability benefits of hexagonal architecture.
//! All tests run without real infrastructure (no Docker Hub, cgroups, or Linux).

use daemonbox::handler::{
    self, BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
    handle_resize_pty, handle_send_input,
};
use daemonbox::state::{ContainerState, DaemonState};
use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::{DynImageRegistry, NetworkMode};
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

// `chrono` is used in manual ContainerRecord construction below.
#[allow(unused_imports)]
use chrono;

/// Helper that calls `handle_run` via a channel and returns the first response.
///
/// `handle_run` now sends responses via a channel rather than returning them,
/// so tests use this wrapper to recover the single response for non-ephemeral runs.
#[allow(clippy::too_many_arguments)]
async fn handle_run_once(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    ephemeral: bool,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral,
        None,
        vec![],
        false,
        vec![],
        None,
        state,
        deps,
        tx,
    )
    .await;
    rx.recv().await.expect("handler sent no response")
}

/// No-op image GC for tests.
struct NoopImageGc;

#[async_trait::async_trait]
impl minibox_core::image::gc::ImageGarbageCollector for NoopImageGc {
    async fn prune(
        &self,
        dry_run: bool,
        _in_use: &[String],
    ) -> anyhow::Result<minibox_core::image::gc::PruneReport> {
        Ok(minibox_core::image::gc::PruneReport {
            removed: vec![],
            freed_bytes: 0,
            dry_run,
        })
    }
}

/// Low-level builder: creates `HandlerDependencies` with the given registry_router,
/// image_store, and network_provider; all other fields use sensible mock defaults.
///
/// Used by tests that need to customise individual adapter components.
fn build_deps(
    registry_router: minibox_core::domain::DynRegistryRouter,
    image_store: Arc<minibox_core::image::ImageStore>,
    network_provider: minibox_core::domain::DynNetworkProvider,
    containers_base: std::path::PathBuf,
    run_containers_base: std::path::PathBuf,
) -> Arc<HandlerDependencies> {
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router,
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider,
            containers_base,
            run_containers_base,
        },
        exec: ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    })
}

/// Convenience wrapper: standard mock deps with a given registry and image_store.
fn build_deps_with_registry(
    registry_router: minibox_core::domain::DynRegistryRouter,
    image_store: Arc<minibox_core::image::ImageStore>,
    temp_dir: &TempDir,
) -> Arc<HandlerDependencies> {
    build_deps(
        registry_router,
        image_store,
        Arc::new(MockNetwork::new()),
        temp_dir.path().join("containers"),
        temp_dir.path().join("run"),
    )
}

/// Helper to create test dependencies with mocks.
fn create_test_deps_with_dir(temp_dir: &TempDir) -> Arc<HandlerDependencies> {
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images2")).unwrap());
    Arc::new(HandlerDependencies {
        image: daemonbox::handler::ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: daemonbox::handler::LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers"),
            run_containers_base: temp_dir.path().join("run"),
        },
        exec: daemonbox::handler::ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                daemonbox::handler::PtySessionRegistry::default(),
            )),
        },
        build: daemonbox::handler::BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: daemonbox::handler::EventDeps {
            event_sink: Arc::new(minibox_core::events::NoopEventSink),
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    })
}

/// Helper to create daemon state with a test image store.
fn create_test_state_with_dir(temp_dir: &TempDir) -> Arc<DaemonState> {
    let image_store = minibox::image::ImageStore::new(temp_dir.path().join("images")).unwrap();
    Arc::new(DaemonState::new(image_store, temp_dir.path()))
}

// ---------------------------------------------------------------------------
// handle_pull Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_pull_success() {
    let temp_dir = TempDir::new().unwrap();
    let mock_registry = Arc::new(MockRegistry::new());
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images2")).unwrap());
    let deps = build_deps_with_registry(
        Arc::new(HostnameRegistryRouter::new(
            mock_registry.clone() as DynImageRegistry,
            [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
        )),
        image_store,
        &temp_dir,
    );
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::Success { message } => {
            assert!(message.contains("pulled"));
            assert!(message.contains("alpine"));
        }
        _ => panic!("expected Success response, got {response:?}"),
    }

    // Verify pull was called via the locally retained mock reference
    assert_eq!(mock_registry.pull_count(), 1);
}

#[tokio::test]
async fn test_handle_pull_with_library_prefix() {
    let temp_dir = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // Bare image name should get "library/" prefix
    let response = handler::handle_pull("ubuntu".to_string(), None, state, deps).await;

    match response {
        DaemonResponse::Success { .. } => {
            // Success - library prefix was added internally
        }
        _ => panic!("expected Success response"),
    }
}

#[tokio::test]
async fn test_handle_pull_failure() {
    let temp_dir = TempDir::new().unwrap();
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    });
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_pull("alpine".to_string(), None, state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(message.contains("mock pull failure"));
        }
        _ => panic!("expected Error response, got {response:?}"),
    }
}

// ---------------------------------------------------------------------------
// handle_run Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_run_with_cached_image() {
    let temp_dir = TempDir::new().unwrap();
    let mock_registry = Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"));
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images2")).unwrap());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                mock_registry.clone() as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
    let temp_dir = TempDir::new().unwrap();
    let mock_registry = Arc::new(MockRegistry::new());
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images2")).unwrap());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                mock_registry.clone() as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
        _ => panic!("expected ContainerCreated, got {response:?}"),
    }

    // Image was not cached, so pull SHOULD have been called
    assert_eq!(mock_registry.pull_count(), 1);
}

#[tokio::test]
async fn test_handle_run_filesystem_setup_failure() {
    let temp_dir = TempDir::new().unwrap();
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"))
                    as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
    let temp_dir = TempDir::new().unwrap();
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"))
                    as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
    let temp_dir = TempDir::new().unwrap();
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest"))
                    as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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

    // Container creation succeeds (spawn happens asynchronously)
    match response {
        DaemonResponse::ContainerCreated { id } => {
            // Wait for async spawn to fail
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

            // Container should be marked as Failed
            let container = state.get_container(&id).await;
            assert!(container.is_some());
            assert_eq!(container.unwrap().info.state, "Failed");
        }
        _ => panic!("expected ContainerCreated, got {response:?}"),
    }
}

// ---------------------------------------------------------------------------
// handle_remove Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_remove_success() {
    let temp_dir = TempDir::new().unwrap();
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
        .unwrap();
    assert_eq!(filesystem.cleanup_count(), 1);

    let limiter = deps
        .lifecycle
        .resource_limiter
        .as_any()
        .downcast_ref::<MockLimiter>()
        .unwrap();
    assert_eq!(limiter.cleanup_count(), 1);
}

#[tokio::test]
async fn test_handle_remove_nonexistent_container() {
    let temp_dir = TempDir::new().unwrap();
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
    let temp_dir = TempDir::new().unwrap();
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
    let temp_dir = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // 1. Pull image
    let pull_response = handler::handle_pull(
        "nginx".to_string(),
        Some("alpine".to_string()),
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
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images3")).unwrap());
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    })
}

/// `network_provider.setup()` is called exactly once per non-ephemeral
/// `handle_run` invocation.
#[tokio::test]
async fn test_network_setup_called_on_run() {
    let temp_dir = TempDir::new().unwrap();
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
    let temp_dir = TempDir::new().unwrap();
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
    let temp_dir = TempDir::new().unwrap();
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
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
    use daemonbox::state::ContainerRecord;
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
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
    let response = handler::handle_pull("ghcr.io/imageonly".to_string(), None, state, deps).await;

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
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
    use daemonbox::state::ContainerRecord;
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
// GHCR routing tests
// ---------------------------------------------------------------------------

/// `handle_pull` for a `ghcr.io/…` image routes to `ghcr_registry`, not
/// `registry`.  The Docker Hub mock should see zero pulls; the GHCR mock
/// should see exactly one.
#[tokio::test]
async fn test_handle_pull_routes_to_ghcr_registry() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let docker_registry = Arc::new(MockRegistry::new());
    let ghcr_registry = Arc::new(MockRegistry::new());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                docker_registry.clone() as DynImageRegistry,
                [("ghcr.io", ghcr_registry.clone() as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    });
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_pull(
        "ghcr.io/org/myimage".to_string(),
        Some("v1.0".to_string()),
        state,
        deps,
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::Success { .. }),
        "expected Success, got: {response:?}"
    );
    assert_eq!(
        docker_registry.pull_count(),
        0,
        "Docker Hub registry must not be called for ghcr.io image"
    );
    assert_eq!(
        ghcr_registry.pull_count(),
        1,
        "GHCR registry must be called exactly once"
    );
}

/// `handle_run` for a `ghcr.io/…` image routes to `ghcr_registry`.
/// The Docker Hub mock sees zero pulls; the GHCR mock sees exactly one.
#[tokio::test]
async fn test_handle_run_routes_to_ghcr_registry() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let docker_registry = Arc::new(MockRegistry::new());
    let ghcr_registry = Arc::new(MockRegistry::new());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                docker_registry.clone() as DynImageRegistry,
                [("ghcr.io", ghcr_registry.clone() as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    });
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "ghcr.io/org/myimage".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    extract_container_id(&response);
    assert_eq!(
        docker_registry.pull_count(),
        0,
        "Docker Hub registry must not be called for ghcr.io image"
    );
    assert_eq!(
        ghcr_registry.pull_count(),
        1,
        "GHCR registry must be called exactly once"
    );
}

/// `handle_run` for a cached GHCR image skips the pull entirely.
#[tokio::test]
async fn test_handle_run_ghcr_cached_skips_pull() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let ghcr_registry =
        Arc::new(MockRegistry::new().with_cached_image("ghcr.io/org/myimage", "latest"));
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", ghcr_registry.clone() as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    });
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "ghcr.io/org/myimage".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    extract_container_id(&response);
    assert_eq!(
        ghcr_registry.pull_count(),
        0,
        "cached GHCR image must not trigger a pull"
    );
}

/// `handle_run` for a GHCR image where the pull fails returns an Error.
#[tokio::test]
async fn test_handle_run_ghcr_pull_failure_returns_error() {
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
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    });
    let state = create_test_state_with_dir(&temp_dir);

    let response = handle_run_once(
        "ghcr.io/org/myimage".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state,
        deps,
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "GHCR pull failure must produce Error response, got: {response:?}"
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
    use daemonbox::state::ContainerRecord;
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
    use daemonbox::state::ContainerRecord;
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
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    });
    let state = create_test_state_with_dir(&temp_dir);

    // Run will return ContainerCreated immediately; async spawn fails → "Failed".
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

    // Wait for async spawn task to mark container as Failed.
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    let container = state.get_container(&id).await.expect("container exists");
    assert_eq!(container.info.state, "Failed", "container should be Failed");

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
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    });
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_pull(
        "ghcr.io/org/myimage".to_string(),
        Some("latest".to_string()),
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
        if let Some(record) = state.get_container(id).await {
            if record.info.state == expected {
                return record.info.state;
            }
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
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
    let temp_dir = TempDir::new().unwrap();
    let mock_registry = Arc::new(MockRegistry::new().with_empty_layers());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::clone(&mock_registry) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
    let temp_dir = TempDir::new().unwrap();
    let mock_registry = Arc::new(MockRegistry::new().with_pull_failure());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::clone(&mock_registry) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
    });
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_pull(
        "testimage".to_string(),
        Some("v1.0".to_string()),
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
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store: Arc::new(
                minibox_core::image::ImageStore::new(temp_dir.path().join("img2")).unwrap(),
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
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: daemonbox::handler::ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: true,
        },
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
    use daemonbox::state::ContainerRecord;
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
    use daemonbox::state::ContainerRecord;
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

// ---------------------------------------------------------------------------
// handle_load_image Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_load_image_success() {
    let tmp = TempDir::new().unwrap();

    struct OkLoader;
    #[async_trait::async_trait]
    impl minibox_core::domain::ImageLoader for OkLoader {
        async fn load_image(&self, _p: &std::path::Path, _n: &str, _t: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    let deps = Arc::try_unwrap(create_test_deps_with_dir(&tmp))
        .unwrap_or_else(|_| panic!("Arc had other refs"))
        .with_image_loader(Arc::new(OkLoader) as minibox_core::domain::DynImageLoader);
    let state = create_test_state_with_dir(&tmp);

    let response = handler::handle_load_image(
        "fake.tar".to_string(),
        "minibox-tester".to_string(),
        "latest".to_string(),
        state,
        Arc::new(deps),
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::ImageLoaded { .. }),
        "expected ImageLoaded, got: {response:?}"
    );
}

#[tokio::test]
async fn test_load_image_failure() {
    let tmp = TempDir::new().unwrap();

    struct FailLoader;
    #[async_trait::async_trait]
    impl minibox_core::domain::ImageLoader for FailLoader {
        async fn load_image(&self, _p: &std::path::Path, _n: &str, _t: &str) -> anyhow::Result<()> {
            anyhow::bail!("file not found")
        }
    }

    let deps = Arc::try_unwrap(create_test_deps_with_dir(&tmp))
        .unwrap_or_else(|_| panic!("Arc had other refs"))
        .with_image_loader(Arc::new(FailLoader) as minibox_core::domain::DynImageLoader);
    let state = create_test_state_with_dir(&tmp);

    let response = handler::handle_load_image(
        "/nonexistent/fake.tar".to_string(),
        "minibox-tester".to_string(),
        "latest".to_string(),
        state,
        Arc::new(deps),
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "expected Error, got: {response:?}"
    );
}

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
// Task 1: ContainerPolicy tests (auth-policy-gate)
// ---------------------------------------------------------------------------

use daemonbox::handler::ContainerPolicy;
use minibox_core::domain::BindMount;

fn make_deps_with_policy(temp_dir: &TempDir, policy: ContainerPolicy) -> Arc<HandlerDependencies> {
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images_pol")).unwrap());
    Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers_pol"),
            run_containers_base: temp_dir.path().join("run_pol"),
        },
        exec: ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy,
    })
}

/// Default policy denies bind mounts.
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
// Task 2: handler coverage gap tests
// ---------------------------------------------------------------------------

/// handle_run with a registry that fails pull → Error response.
#[tokio::test]
async fn test_handle_run_image_pull_failure() {
    let temp_dir = TempDir::new().unwrap();
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images_pf")).unwrap());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers_pf"),
            run_containers_base: temp_dir.path().join("run_pf"),
        },
        exec: ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: ContainerPolicy::default(),
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
        "pull failure should produce Error, got {resp:?}"
    );
}

/// handle_run with an image that has no layers → Error response.
#[tokio::test]
async fn test_handle_run_empty_layers() {
    let temp_dir = TempDir::new().unwrap();
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images_el")).unwrap());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_empty_layers()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers_el"),
            run_containers_base: temp_dir.path().join("run_el"),
        },
        exec: ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: ContainerPolicy::default(),
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
        "empty layers should produce Error, got {resp:?}"
    );
}

/// handle_pull for a non-existent image (pull failure) → Error response.
#[tokio::test]
async fn test_handle_pull_nonexistent_image() {
    let temp_dir = TempDir::new().unwrap();
    let image_store =
        Arc::new(minibox_core::image::ImageStore::new(temp_dir.path().join("images_ni")).unwrap());
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
                [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
            )),
            image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
            image_gc: Arc::new(NoopImageGc),
            image_store,
        },
        lifecycle: LifecycleDeps {
            filesystem: Arc::new(MockFilesystem::new()),
            resource_limiter: Arc::new(MockLimiter::new()),
            runtime: Arc::new(MockRuntime::new()),
            network_provider: Arc::new(MockNetwork::new()),
            containers_base: temp_dir.path().join("containers_ni"),
            run_containers_base: temp_dir.path().join("run_ni"),
        },
        exec: ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                daemonbox::handler::PtySessionRegistry::default(),
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
            metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: ContainerPolicy::default(),
    });
    let state = create_test_state_with_dir(&temp_dir);

    let resp = handler::handle_pull(
        "does-not-exist".to_string(),
        Some("latest".to_string()),
        state,
        deps,
    )
    .await;

    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "nonexistent image pull should return Error, got {resp:?}"
    );
}

/// handle_stop with an unknown container ID → Error response.
#[tokio::test]
async fn test_handle_stop_nonexistent_container() {
    let temp_dir = TempDir::new().unwrap();
    let deps = make_deps_with_policy(&temp_dir, ContainerPolicy::default());
    let state = create_test_state_with_dir(&temp_dir);

    let resp = handler::handle_stop("nonexistent-id-xyz".to_string(), state, deps).await;

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

/// handle_remove with an unknown container ID → Error response.
#[tokio::test]
async fn test_handle_rm_nonexistent_container() {
    let temp_dir = TempDir::new().unwrap();
    let deps = make_deps_with_policy(&temp_dir, ContainerPolicy::default());
    let state = create_test_state_with_dir(&temp_dir);

    let resp = handler::handle_remove("nonexistent-id-xyz".to_string(), state, deps).await;

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
    let record = daemonbox::state::ContainerRecord {
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
    let record = daemonbox::state::ContainerRecord {
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
    let record = daemonbox::state::ContainerRecord {
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
    let record = daemonbox::state::ContainerRecord {
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
    let record = daemonbox::state::ContainerRecord {
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

// ---------------------------------------------------------------------------
// validate_policy unit tests (#116, #123)
// ---------------------------------------------------------------------------

/// Plain container (no mounts, not privileged) passes any policy configuration.
#[test]
fn test_validate_policy_plain_container_always_allowed() {
    use daemonbox::handler::{ContainerPolicy, validate_policy};

    let policy = ContainerPolicy::default(); // deny-all defaults
    assert!(
        validate_policy(&[], false, &policy).is_ok(),
        "plain container must always pass policy"
    );
}

/// `validate_policy` rejects bind mounts when `allow_bind_mounts` is false.
#[test]
fn test_validate_policy_denies_bind_mount() {
    use daemonbox::handler::{ContainerPolicy, validate_policy};
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
    use daemonbox::handler::{ContainerPolicy, validate_policy};

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

// ---------------------------------------------------------------------------
// handle_push Tests
// ---------------------------------------------------------------------------

/// handle_push with no image_pusher wired → Error "not supported".
#[tokio::test]
async fn test_handle_push_no_pusher_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_push(
        "docker.io/library/alpine:latest".to_string(),
        minibox_core::protocol::PushCredentials::Anonymous,
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

/// handle_push with an invalid image ref → Error.
#[tokio::test]
async fn test_handle_push_invalid_image_ref_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&temp_dir);

    let mock_pusher = Arc::new(minibox_core::adapters::mocks::MockImagePusher::new());
    let deps = {
        let mut d = (*create_test_deps_with_dir(&temp_dir)).clone();
        d.build.image_pusher =
            Some(Arc::clone(&mock_pusher) as minibox_core::domain::DynImagePusher);
        Arc::new(d)
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_push(
        "".to_string(),
        minibox_core::protocol::PushCredentials::Anonymous,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "expected Error for invalid ref, got {resp:?}"
    );
}

/// handle_push with a valid pusher and valid image ref → Success.
#[tokio::test]
async fn test_handle_push_success() {
    let temp_dir = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&temp_dir);

    let mock_pusher = Arc::new(minibox_core::adapters::mocks::MockImagePusher::new());
    let deps = {
        let mut d = (*create_test_deps_with_dir(&temp_dir)).clone();
        d.build.image_pusher =
            Some(Arc::clone(&mock_pusher) as minibox_core::domain::DynImagePusher);
        Arc::new(d)
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(8);

    handler::handle_push(
        "docker.io/library/alpine:latest".to_string(),
        minibox_core::protocol::PushCredentials::Anonymous,
        state,
        deps,
        tx,
    )
    .await;

    // Drain until terminal (Success or Error).
    let mut terminal = None;
    while let Ok(Some(r)) =
        tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
    {
        if !matches!(r, DaemonResponse::PushProgress { .. }) {
            terminal = Some(r);
            break;
        }
    }
    assert!(
        matches!(terminal, Some(DaemonResponse::Success { .. })),
        "expected Success, got {terminal:?}"
    );
    assert!(mock_pusher.has_tag("docker.io/library/alpine:latest"));
}

// ---------------------------------------------------------------------------
// handle_commit Tests
// ---------------------------------------------------------------------------

/// handle_commit with no commit_adapter wired → Error "not supported".
#[tokio::test]
async fn test_handle_commit_no_adapter_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_commit(
        "abc123def456abcd".to_string(),
        "myimage:v1".to_string(),
        None,
        None,
        vec![],
        None,
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

/// handle_commit with invalid container id → Error.
#[tokio::test]
async fn test_handle_commit_invalid_container_id_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&temp_dir);

    let mock_committer = Arc::new(minibox_core::adapters::mocks::MockContainerCommitter::new());
    let deps = {
        let mut d = (*create_test_deps_with_dir(&temp_dir)).clone();
        d.build.commit_adapter =
            Some(Arc::clone(&mock_committer) as minibox_core::domain::DynContainerCommitter);
        Arc::new(d)
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_commit(
        "".to_string(), // empty → invalid ContainerId
        "myimage:v1".to_string(),
        None,
        None,
        vec![],
        None,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "expected Error for invalid container id, got {resp:?}"
    );
}

/// handle_commit with valid adapter and valid container id → Success.
#[tokio::test]
async fn test_handle_commit_success() {
    let temp_dir = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&temp_dir);

    let mock_committer = Arc::new(minibox_core::adapters::mocks::MockContainerCommitter::new());
    let deps = {
        let mut d = (*create_test_deps_with_dir(&temp_dir)).clone();
        d.build.commit_adapter =
            Some(Arc::clone(&mock_committer) as minibox_core::domain::DynContainerCommitter);
        Arc::new(d)
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_commit(
        "abc123def456abcd".to_string(),
        "myimage:v1".to_string(),
        Some("test-author".to_string()),
        Some("test commit".to_string()),
        vec![],
        None,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Success { ref message } if message.contains("myimage:v1")),
        "expected Success with target image, got {resp:?}"
    );
    assert_eq!(mock_committer.call_count(), 1);
}

// ---------------------------------------------------------------------------
// handle_build Tests
// ---------------------------------------------------------------------------

/// handle_build with no image_builder wired → Error "not supported".
#[tokio::test]
async fn test_handle_build_no_builder_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_build(
        "FROM alpine\nRUN echo hi".to_string(),
        temp_dir.path().to_str().unwrap().to_string(),
        "myimage:latest".to_string(),
        vec![],
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

/// handle_build with a relative context_path → Error.
#[tokio::test]
async fn test_handle_build_relative_context_path_returns_error() {
    let temp_dir = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&temp_dir);

    let mock_builder = Arc::new(minibox_core::adapters::mocks::MockImageBuilder::new());
    let deps = {
        let mut d = (*create_test_deps_with_dir(&temp_dir)).clone();
        d.build.image_builder =
            Some(Arc::clone(&mock_builder) as minibox_core::domain::DynImageBuilder);
        Arc::new(d)
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_build(
        "FROM alpine".to_string(),
        "relative/path".to_string(),
        "myimage:latest".to_string(),
        vec![],
        false,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("absolute")),
        "expected 'absolute' path error, got {resp:?}"
    );
}

/// handle_build with valid builder and absolute context path → BuildComplete.
#[tokio::test]
async fn test_handle_build_success() {
    let temp_dir = TempDir::new().unwrap();
    let state = create_test_state_with_dir(&temp_dir);

    let mock_builder = Arc::new(minibox_core::adapters::mocks::MockImageBuilder::new());
    let deps = {
        let mut d = (*create_test_deps_with_dir(&temp_dir)).clone();
        d.build.image_builder =
            Some(Arc::clone(&mock_builder) as minibox_core::domain::DynImageBuilder);
        Arc::new(d)
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);

    handler::handle_build(
        "FROM alpine\nRUN echo hi".to_string(),
        temp_dir.path().to_str().unwrap().to_string(),
        "myimage:latest".to_string(),
        vec![],
        false,
        state,
        deps,
        tx,
    )
    .await;

    // Drain until terminal response.
    let mut terminal = None;
    while let Ok(Some(r)) =
        tokio::time::timeout(std::time::Duration::from_millis(300), rx.recv()).await
    {
        if !matches!(r, DaemonResponse::BuildOutput { .. }) {
            terminal = Some(r);
            break;
        }
    }
    assert!(
        matches!(
            terminal,
            Some(DaemonResponse::BuildComplete { ref tag, .. }) if tag == "myimage:latest"
        ),
        "expected BuildComplete, got {terminal:?}"
    );
    assert_eq!(mock_builder.call_count(), 1);
}

// ---------------------------------------------------------------------------
// handle_exec Tests
// ---------------------------------------------------------------------------

/// handle_exec with no exec_runtime wired → Error "not supported".
#[tokio::test]
async fn test_handle_exec_no_runtime_returns_error() {
    let temp_dir = TempDir::new().unwrap();
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
