//! Image management tests: pull, push, build, commit, load, update, platform routing.

use minibox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox::daemon::handler::{
    self, BuildDeps, ContainerPolicy, EventDeps, ExecDeps, HandlerDependencies, ImageDeps,
    LifecycleDeps,
};
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::DynImageRegistry;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

mod daemon_handler_common;
use daemon_handler_common::*;

// ---------------------------------------------------------------------------
// handle_pull Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_pull_success() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_registry = Arc::new(MockRegistry::new());
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images2"))
            .expect("unwrap in test"),
    );
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
        None,
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
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // Bare image name should get "library/" prefix
    let response = handler::handle_pull("ubuntu".to_string(), None, None, state, deps).await;

    match response {
        DaemonResponse::Success { .. } => {
            // Success - library prefix was added internally
        }
        _ => panic!("expected Success response"),
    }
}

#[tokio::test]
async fn test_handle_pull_failure() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = Arc::new(HandlerDependencies {
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

    let response = handler::handle_pull("alpine".to_string(), None, None, state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(message.contains("mock pull failure"));
        }
        _ => panic!("expected Error response, got {response:?}"),
    }
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
        Some("v1.0".to_string()),
        None,
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
// handle_load_image Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_load_image_success() {
    let tmp = TempDir::new().expect("unwrap in test");

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
    let tmp = TempDir::new().expect("unwrap in test");

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
// Task 2: handler coverage gap tests
// ---------------------------------------------------------------------------

/// handle_run with a registry that fails pull → Error response.
#[tokio::test]
async fn test_handle_run_image_pull_failure() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images_pf"))
            .expect("unwrap in test"),
    );
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
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
            containers_base: temp_dir.path().join("containers_pf"),
            run_containers_base: temp_dir.path().join("run_pf"),
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
        policy: ContainerPolicy::default(),
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
        "pull failure should produce Error, got {resp:?}"
    );
}

/// handle_run with an image that has no layers → Error response.
#[tokio::test]
async fn test_handle_run_empty_layers() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images_el"))
            .expect("unwrap in test"),
    );
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_empty_layers()) as DynImageRegistry,
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
            containers_base: temp_dir.path().join("containers_el"),
            run_containers_base: temp_dir.path().join("run_el"),
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
        policy: ContainerPolicy::default(),
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
        "empty layers should produce Error, got {resp:?}"
    );
}

/// handle_pull for a non-existent image (pull failure) → Error response.
#[tokio::test]
async fn test_handle_pull_nonexistent_image() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images_ni"))
            .expect("unwrap in test"),
    );
    let deps = Arc::new(HandlerDependencies {
        image: ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new().with_pull_failure()) as DynImageRegistry,
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
            containers_base: temp_dir.path().join("containers_ni"),
            run_containers_base: temp_dir.path().join("run_ni"),
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
        policy: ContainerPolicy::default(),
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    });
    let state = create_test_state_with_dir(&temp_dir);

    let resp = handler::handle_pull(
        "does-not-exist".to_string(),
        Some("latest".to_string()),
        None,
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
    let temp_dir = TempDir::new().expect("unwrap in test");
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
    let temp_dir = TempDir::new().expect("unwrap in test");
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
// handle_push Tests
// ---------------------------------------------------------------------------

/// handle_push with no image_pusher wired → Error "not supported".
#[tokio::test]
async fn test_handle_push_no_pusher_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
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
    let temp_dir = TempDir::new().expect("unwrap in test");
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
    let temp_dir = TempDir::new().expect("unwrap in test");
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
    let temp_dir = TempDir::new().expect("unwrap in test");
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
    let temp_dir = TempDir::new().expect("unwrap in test");
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
    let temp_dir = TempDir::new().expect("unwrap in test");
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
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);

    handler::handle_build(
        "FROM alpine\nRUN echo hi".to_string(),
        temp_dir
            .path()
            .to_str()
            .expect("unwrap in test")
            .to_string(),
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
    let temp_dir = TempDir::new().expect("unwrap in test");
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
    let temp_dir = TempDir::new().expect("unwrap in test");
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
        temp_dir
            .path()
            .to_str()
            .expect("unwrap in test")
            .to_string(),
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
// Error-path tests: handle_pull — invalid image reference
// ---------------------------------------------------------------------------

/// handle_pull with an empty image string (invalid ref) → Error response (variant 2).
#[tokio::test]
async fn test_handle_pull_invalid_image_ref_returns_error_v2() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    // An empty string is not a valid image reference.
    let resp = handler::handle_pull("".to_string(), None, None, state, deps).await;

    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "invalid image ref should produce Error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// Error-path tests: handle_load_image — loader failure
// ---------------------------------------------------------------------------

/// Failing image loader used in load_image error-path tests.
struct FailingImageLoader;

#[async_trait::async_trait]
impl minibox_core::domain::ImageLoader for FailingImageLoader {
    async fn load_image(
        &self,
        _path: &std::path::Path,
        _name: &str,
        _tag: &str,
    ) -> anyhow::Result<()> {
        anyhow::bail!("mock image loader failure")
    }
}

/// handle_load_image with a loader that always fails → Error response.
#[tokio::test]
async fn test_handle_load_image_loader_failure() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&temp_dir);

    let deps = {
        let mut d = (*create_test_deps_with_dir(&temp_dir)).clone();
        d.image.image_loader = Arc::new(FailingImageLoader);
        Arc::new(d)
    };

    let resp = handler::handle_load_image(
        "/tmp/nonexistent.tar".to_string(),
        "myimage".to_string(),
        "latest".to_string(),
        state,
        deps,
    )
    .await;

    assert!(
        matches!(resp, DaemonResponse::Error { .. }),
        "image loader failure should produce Error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// Issue #134 — bridge networking, exec error path, persistence semantics
// ---------------------------------------------------------------------------

/// handle_run with NetworkMode::Bridge calls network_provider.setup() once.
///
/// Guards that the bridge network code path is exercised end-to-end through
/// the handler without requiring any Linux-only syscalls.
// Error-path coverage tests (#158, #129, #116)
// ===========================================================================

// --- handle_push: pusher adapter failure ---

#[tokio::test]
async fn test_handle_push_adapter_failure_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&temp_dir);

    let mock_pusher =
        Arc::new(minibox_core::adapters::mocks::MockImagePusher::new().with_failure());
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

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("mock push failure")),
        "expected push failure error, got {resp:?}"
    );
}

// --- handle_commit: committer adapter failure ---

#[tokio::test]
async fn test_handle_commit_adapter_failure_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&temp_dir);

    let mock_committer =
        Arc::new(minibox_core::adapters::mocks::MockContainerCommitter::new().with_failure());
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
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("mock commit failure")),
        "expected commit failure error, got {resp:?}"
    );
}

// --- handle_build: builder adapter failure ---

#[tokio::test]
async fn test_handle_build_adapter_failure_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let state = create_test_state_with_dir(&temp_dir);

    let mock_builder =
        Arc::new(minibox_core::adapters::mocks::MockImageBuilder::new().with_failure());
    let deps = {
        let mut d = (*create_test_deps_with_dir(&temp_dir)).clone();
        d.build.image_builder =
            Some(Arc::clone(&mock_builder) as minibox_core::domain::DynImageBuilder);
        Arc::new(d)
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(8);

    handler::handle_build(
        "FROM alpine\nRUN true".to_string(),
        temp_dir
            .path()
            .to_str()
            .expect("unwrap in test")
            .to_string(),
        "testimage:v1".to_string(),
        vec![],
        false,
        state,
        deps,
        tx,
    )
    .await;

    let mut terminal = None;
    while let Ok(Some(r)) =
        tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
    {
        if !matches!(r, DaemonResponse::BuildOutput { .. }) {
            terminal = Some(r);
            break;
        }
    }
    assert!(
        matches!(terminal, Some(DaemonResponse::Error { ref message }) if message.contains("mock build failure")),
        "expected build failure error, got {terminal:?}"
    );
}

// --- handle_build: canonicalize failure (nonexistent path) ---

#[tokio::test]
async fn test_handle_build_canonicalize_failure_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
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
        "/nonexistent/path/that/does/not/exist".to_string(),
        "testimage:v1".to_string(),
        vec![],
        false,
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("context_path invalid")),
        "expected canonicalize error, got {resp:?}"
    );
}

// handle_run --platform Tests
// ---------------------------------------------------------------------------

/// When `handle_run` receives an invalid platform string, it should return an
/// error rather than silently ignoring it.
#[tokio::test]
async fn test_handle_run_invalid_platform_returns_error() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(4);
    handler::handle_run(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        None,
        vec![],
        false,
        vec![],
        None,
        Some("not/a/valid/platform/triple".to_string()),
        state,
        deps,
        tx,
    )
    .await;

    let resp = rx.recv().await.expect("handler sent no response");
    assert!(
        matches!(resp, DaemonResponse::Error { ref message } if message.contains("platform")),
        "expected platform error, got {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// Platform-aware pull tests
// ---------------------------------------------------------------------------

/// Valid platform override is accepted by resolve_platform_registry and
/// handle_pull proceeds without panicking. The pull itself may fail (no
/// real registry), but the error must NOT be "invalid platform".
#[tokio::test]
async fn test_handle_pull_with_valid_platform_override() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        Some("linux/arm64".to_string()),
        state,
        deps,
    )
    .await;

    // The platform string is valid, so any error must come from the pull
    // itself (network/mock), never from platform parsing.
    match &response {
        DaemonResponse::Error { message } => {
            assert!(
                !message.contains("invalid platform"),
                "valid platform 'linux/arm64' should not produce an invalid-platform error, got: {message}"
            );
        }
        DaemonResponse::Success { .. } => { /* acceptable if mock satisfies */ }
        other => panic!("unexpected response variant: {other:?}"),
    }
}

/// Empty and malformed platform strings must produce an "invalid platform"
/// error response.
#[tokio::test]
async fn test_handle_pull_with_invalid_platform_returns_error() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    for bad_platform in ["", "invalid"] {
        let response = handler::handle_pull(
            "alpine".to_string(),
            Some("latest".to_string()),
            Some(bad_platform.to_string()),
            Arc::clone(&state),
            Arc::clone(&deps),
        )
        .await;

        match &response {
            DaemonResponse::Error { message } => {
                assert!(
                    message.contains("invalid platform"),
                    "platform {bad_platform:?} should trigger 'invalid platform' error, got: {message}"
                );
            }
            other => panic!("expected Error for platform {bad_platform:?}, got {other:?}"),
        }
    }
}

/// When platform is None, handle_pull uses the default registry router
/// and succeeds with the mock registry (regression guard).
#[tokio::test]
async fn test_handle_pull_platform_none_uses_default_router() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let mock_registry = Arc::new(MockRegistry::new());
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images2"))
            .expect("create image store"),
    );
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
        None,
        state,
        deps,
    )
    .await;

    match &response {
        DaemonResponse::Success { message } => {
            assert!(
                message.contains("pulled"),
                "expected 'pulled' in success message, got: {message}"
            );
        }
        other => panic!("expected Success for platform=None pull, got {other:?}"),
    }

    assert_eq!(
        mock_registry.pull_count(),
        1,
        "default router mock should have been called exactly once"
    );
}

// ---------------------------------------------------------------------------
// handle_update Tests
// ---------------------------------------------------------------------------

/// `handle_update` with an explicit image list sends one `UpdateProgress` per
/// image and ends with a terminal `Success` response.
#[tokio::test]
async fn test_handle_update_explicit_images_sends_progress_then_success() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_registry = Arc::new(MockRegistry::new());
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images2"))
            .expect("unwrap in test"),
    );
    let deps = build_deps_with_registry(
        Arc::new(HostnameRegistryRouter::new(
            Arc::clone(&mock_registry) as DynImageRegistry,
            [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
        )),
        Arc::clone(&image_store),
        &temp_dir,
    );
    let state = create_test_state_with_dir(&temp_dir);

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

    // Expect one UpdateProgress for alpine:latest
    let first = rx.recv().await.expect("handler sent no response");
    match &first {
        DaemonResponse::UpdateProgress { image, status } => {
            assert_eq!(image, "alpine:latest");
            assert_eq!(status, "updated");
        }
        other => panic!("expected UpdateProgress, got {other:?}"),
    }

    // Expect terminal Success
    let second = rx.recv().await.expect("handler sent no terminal response");
    match &second {
        DaemonResponse::Success { message } => {
            assert!(
                message.contains("1/1"),
                "expected '1/1' in success message, got: {message}"
            );
        }
        other => panic!("expected Success, got {other:?}"),
    }

    // Pull should have been called once
    assert_eq!(mock_registry.pull_count(), 1);
}

/// `handle_update` with `all = true` and an empty image store sends a terminal
/// `Success` with "0/0" (no images to refresh).
#[tokio::test]
async fn test_handle_update_all_empty_store_sends_zero_progress() {
    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_registry = Arc::new(MockRegistry::new());
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images2"))
            .expect("unwrap in test"),
    );
    let deps = build_deps_with_registry(
        Arc::new(HostnameRegistryRouter::new(
            Arc::clone(&mock_registry) as DynImageRegistry,
            [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
        )),
        Arc::clone(&image_store),
        &temp_dir,
    );
    let state = create_test_state_with_dir(&temp_dir);

    // With an empty image store, all=true means 0 images → terminal Success "0/0"
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
    handler::handle_update(vec![], true, false, false, state, deps, tx).await;

    let response = rx.recv().await.expect("handler sent no response");
    match response {
        DaemonResponse::Success { message } => {
            assert!(
                message.contains("0/0"),
                "expected '0/0' in success message, got: {message}"
            );
        }
        other => panic!("expected Success, got {other:?}"),
    }
}

/// `handle_update` with `containers = true` collects images from container records.
#[tokio::test]
async fn test_handle_update_containers_collects_source_image_refs() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_registry = Arc::new(MockRegistry::new());
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images2"))
            .expect("unwrap in test"),
    );
    let deps = build_deps_with_registry(
        Arc::new(HostnameRegistryRouter::new(
            Arc::clone(&mock_registry) as DynImageRegistry,
            [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
        )),
        Arc::clone(&image_store),
        &temp_dir,
    );
    let state = create_test_state_with_dir(&temp_dir);

    // Add a container with a source_image_ref
    let record = ContainerRecord {
        info: ContainerInfo {
            id: "test-cid-update".to_string(),
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
        source_image_ref: Some("alpine:latest".to_string()),
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
    handler::handle_update(vec![], false, true, false, Arc::clone(&state), deps, tx).await;

    // Should get one UpdateProgress for alpine:latest
    let first = rx.recv().await.expect("handler sent no response");
    match &first {
        DaemonResponse::UpdateProgress { image, status } => {
            assert_eq!(image, "alpine:latest");
            assert_eq!(status, "updated");
        }
        other => panic!("expected UpdateProgress, got {other:?}"),
    }

    let second = rx.recv().await.expect("no terminal response");
    assert!(
        matches!(second, DaemonResponse::Success { .. }),
        "expected Success, got {second:?}"
    );
}

/// `handle_update` with `restart = true` stops Running containers whose source
/// image was updated and reports the stopped count in the success message.
#[cfg(unix)]
#[tokio::test]
async fn test_handle_update_restart_stops_running_containers() {
    use minibox::daemon::state::ContainerRecord;
    use minibox_core::protocol::ContainerInfo;

    let temp_dir = TempDir::new().expect("unwrap in test");
    let mock_registry = Arc::new(MockRegistry::new());
    let image_store = Arc::new(
        minibox_core::image::ImageStore::new(temp_dir.path().join("images2"))
            .expect("unwrap in test"),
    );
    let deps = build_deps_with_registry(
        Arc::new(HostnameRegistryRouter::new(
            Arc::clone(&mock_registry) as DynImageRegistry,
            [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
        )),
        Arc::clone(&image_store),
        &temp_dir,
    );
    let state = create_test_state_with_dir(&temp_dir);

    // Add a Running container with source_image_ref = "alpine:latest" and a
    // fake pid.  stop_inner will attempt to signal it; the PID won't exist on
    // the test host so the signal will fail with ESRCH, but stop_inner still
    // transitions the state to Stopped after the timeout.  To keep the test
    // fast we use a non-running PID that is guaranteed to fail immediately.
    let record = ContainerRecord {
        info: ContainerInfo {
            id: "restart-test-cid".to_string(),
            name: None,
            image: "alpine:latest".to_string(),
            command: "/bin/sh".to_string(),
            state: "Running".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: Some(99999),
        },
        pid: Some(99999),
        rootfs_path: std::path::PathBuf::from("/tmp/fake-restart"),
        cgroup_path: std::path::PathBuf::from("/tmp/fake-restart"),
        post_exit_hooks: vec![],
        rootfs_metadata: None,
        source_image_ref: Some("alpine:latest".to_string()),
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
        vec!["alpine:latest".to_string()],
        false,
        false,
        true, // restart = true
        Arc::clone(&state),
        deps,
        tx,
    )
    .await;

    // Drain all responses to get the terminal Success.
    let success_msg = loop {
        match rx.recv().await.expect("channel closed unexpectedly") {
            DaemonResponse::Success { message } => break message,
            DaemonResponse::UpdateProgress { .. } => continue,
            other => panic!("unexpected response: {other:?}"),
        }
    };

    // The success message must mention "stopped" because the running container
    // should have been stopped. Without creation_params it cannot be restarted,
    // so restarted count should be 0.
    assert!(
        success_msg.contains("stopped"),
        "expected 'stopped' in success message when restart=true, got: {success_msg}"
    );
    assert!(
        success_msg.contains("restarted 0"),
        "expected 'restarted 0' (no creation_params), got: {success_msg}"
    );
}

// ---------------------------------------------------------------------------
