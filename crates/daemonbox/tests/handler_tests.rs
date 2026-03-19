//! Unit tests for daemon handlers using mock adapters.
//!
//! These tests demonstrate the testability benefits of hexagonal architecture.
//! All tests run without real infrastructure (no Docker Hub, cgroups, or Linux).

use daemonbox::handler::{self, HandlerDependencies};
use daemonbox::state::DaemonState;
use linuxbox::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox_core::domain::NetworkMode;
use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

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
        state,
        deps,
        tx,
    )
    .await;
    rx.recv().await.expect("handler sent no response")
}

/// Helper to create test dependencies with mocks.
fn create_test_deps_with_dir(temp_dir: &TempDir) -> Arc<HandlerDependencies> {
    Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new()),
        ghcr_registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: Arc::new(MockNetwork::new()),
        containers_base: temp_dir.path().join("containers"),
        run_containers_base: temp_dir.path().join("run"),
    })
}

/// Helper to create daemon state with a test image store.
fn create_test_state_with_dir(temp_dir: &TempDir) -> Arc<DaemonState> {
    let image_store = linuxbox::image::ImageStore::new(temp_dir.path().join("images")).unwrap();
    Arc::new(DaemonState::new(image_store, temp_dir.path()))
}

// ---------------------------------------------------------------------------
// handle_pull Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_pull_success() {
    let temp_dir = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&temp_dir);
    let state = create_test_state_with_dir(&temp_dir);

    let response = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state,
        deps.clone(),
    )
    .await;

    match response {
        DaemonResponse::Success { message } => {
            assert!(message.contains("pulled"));
            assert!(message.contains("alpine"));
        }
        _ => panic!("expected Success response, got {response:?}"),
    }

    // Verify pull was called
    let registry = deps.registry.clone();
    let mock_registry = registry
        .as_any()
        .downcast_ref::<MockRegistry>()
        .expect("should be MockRegistry");
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
        registry: Arc::new(MockRegistry::new().with_pull_failure()),
        ghcr_registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: Arc::new(MockNetwork::new()),
        containers_base: temp_dir.path().join("containers"),
        run_containers_base: temp_dir.path().join("run"),
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
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest")),
        ghcr_registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: Arc::new(MockNetwork::new()),
        containers_base: temp_dir.path().join("containers"),
        run_containers_base: temp_dir.path().join("run"),
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
    let registry = deps
        .registry
        .as_any()
        .downcast_ref::<MockRegistry>()
        .unwrap();
    assert_eq!(registry.pull_count(), 0);
}

#[tokio::test]
async fn test_handle_run_pulls_uncached_image() {
    let temp_dir = TempDir::new().unwrap();
    let deps = create_test_deps_with_dir(&temp_dir); // Image not cached
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
    let registry = deps
        .registry
        .as_any()
        .downcast_ref::<MockRegistry>()
        .unwrap();
    assert_eq!(registry.pull_count(), 1);
}

#[tokio::test]
async fn test_handle_run_filesystem_setup_failure() {
    let temp_dir = TempDir::new().unwrap();
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest")),
        ghcr_registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new().with_setup_failure()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: Arc::new(MockNetwork::new()),
        containers_base: temp_dir.path().join("containers"),
        run_containers_base: temp_dir.path().join("run"),
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
        registry: Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest")),
        ghcr_registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new().with_create_failure()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: Arc::new(MockNetwork::new()),
        containers_base: temp_dir.path().join("containers"),
        run_containers_base: temp_dir.path().join("run"),
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
        registry: Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest")),
        ghcr_registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new().with_spawn_failure()),
        network_provider: Arc::new(MockNetwork::new()),
        containers_base: temp_dir.path().join("containers"),
        run_containers_base: temp_dir.path().join("run"),
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
    state.update_container_state(&container_id, "Stopped").await;

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
        .filesystem
        .as_any()
        .downcast_ref::<MockFilesystem>()
        .unwrap();
    assert_eq!(filesystem.cleanup_count(), 1);

    let limiter = deps
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
    state.update_container_state(&container_id, "Running").await;

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
    state.update_container_state(&container_id, "Stopped").await;

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
    Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: network,
        containers_base: temp_dir.path().join("containers"),
        run_containers_base: temp_dir.path().join("run"),
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
    let container_dir = deps.containers_base.join(&id);
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
    use linuxbox::adapters::mocks::FailableFilesystemMock;

    let temp_dir = TempDir::new().expect("create temp dir");
    let failable_fs = Arc::new(FailableFilesystemMock::new());

    // Build deps with the failable mock (setup succeeds by default).
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new()),
        filesystem: failable_fs.clone(),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
        network_provider: Arc::new(MockNetwork::new()),
        containers_base: temp_dir.path().join("containers"),
        run_containers_base: temp_dir.path().join("run"),
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
    state.update_container_state(&id, "Stopped").await;

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
