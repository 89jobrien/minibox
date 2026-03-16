//! Unit tests for daemon handlers using mock adapters.
//!
//! These tests demonstrate the testability benefits of hexagonal architecture.
//! All tests run without real infrastructure (no Docker Hub, cgroups, or Linux).

use minibox_lib::adapters::mocks::{MockFilesystem, MockLimiter, MockRegistry, MockRuntime};
use minibox_lib::protocol::DaemonResponse;
use miniboxd::handler::{self, HandlerDependencies};
use miniboxd::state::DaemonState;
use std::sync::Arc;

/// Helper to create test dependencies with mocks.
fn create_test_deps() -> Arc<HandlerDependencies> {
    Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
    })
}

/// Helper to create daemon state with a test image store.
fn create_test_state() -> Arc<DaemonState> {
    let temp_dir = tempfile::tempdir().unwrap();
    let image_store = minibox_lib::image::ImageStore::new(temp_dir.path()).unwrap();
    Arc::new(DaemonState::new(image_store))
}

// ---------------------------------------------------------------------------
// handle_pull Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_pull_success() {
    let deps = create_test_deps();
    let state = create_test_state();

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
        _ => panic!("expected Success response, got {:?}", response),
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
    let deps = create_test_deps();
    let state = create_test_state();

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
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new().with_pull_failure()),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
    });
    let state = create_test_state();

    let response = handler::handle_pull("alpine".to_string(), None, state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(message.contains("mock pull failure"));
        }
        _ => panic!("expected Error response, got {:?}", response),
    }
}

// ---------------------------------------------------------------------------
// handle_run Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_run_with_cached_image() {
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest")),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
    });
    let state = create_test_state();

    let response = handler::handle_run(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/sh".to_string()],
        None,
        None,
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
        _ => panic!("expected ContainerCreated response, got {:?}", response),
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
    let deps = create_test_deps(); // Image not cached
    let state = create_test_state();

    let response = handler::handle_run(
        "alpine".to_string(),
        None, // defaults to "latest"
        vec!["/bin/echo".to_string(), "hello".to_string()],
        Some(512 * 1024 * 1024), // 512MB memory limit
        Some(500),               // CPU weight
        state,
        deps.clone(),
    )
    .await;

    match response {
        DaemonResponse::ContainerCreated { .. } => {
            // Success - image was pulled
        }
        _ => panic!("expected ContainerCreated, got {:?}", response),
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
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest")),
        filesystem: Arc::new(MockFilesystem::new().with_setup_failure()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new()),
    });
    let state = create_test_state();

    let response = handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(message.contains("filesystem setup failure"));
        }
        _ => panic!("expected Error response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_handle_run_resource_limiter_failure() {
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest")),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new().with_create_failure()),
        runtime: Arc::new(MockRuntime::new()),
    });
    let state = create_test_state();

    let response = handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(message.contains("resource limiter create failure"));
        }
        _ => panic!("expected Error response, got {:?}", response),
    }
}

#[tokio::test]
async fn test_handle_run_runtime_spawn_failure() {
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(MockRegistry::new().with_cached_image("library/alpine", "latest")),
        filesystem: Arc::new(MockFilesystem::new()),
        resource_limiter: Arc::new(MockLimiter::new()),
        runtime: Arc::new(MockRuntime::new().with_spawn_failure()),
    });
    let state = create_test_state();

    let response = handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
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
        _ => panic!("expected ContainerCreated, got {:?}", response),
    }
}

// ---------------------------------------------------------------------------
// handle_remove Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_handle_remove_success() {
    let deps = create_test_deps();
    let state = create_test_state();

    // First create a container
    let create_response = handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
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
    let remove_response = handler::handle_remove(container_id.clone(), state.clone(), deps.clone()).await;

    match remove_response {
        DaemonResponse::Success { message } => {
            assert!(message.contains("removed"));
            assert!(message.contains(&container_id));
        }
        _ => panic!("expected Success, got {:?}", remove_response),
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
    let deps = create_test_deps();
    let state = create_test_state();

    let response = handler::handle_remove("nonexistent123".to_string(), state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(message.contains("not found"));
        }
        _ => panic!("expected Error, got {:?}", response),
    }
}

#[tokio::test]
async fn test_handle_remove_running_container() {
    let deps = create_test_deps();
    let state = create_test_state();

    // Create a container
    let create_response = handler::handle_run(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        state.clone(),
        deps.clone(),
    )
    .await;

    let container_id = match create_response {
        DaemonResponse::ContainerCreated { id } => id,
        _ => panic!("failed to create container"),
    };

    // Wait for it to spawn and be marked Running
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Try to remove while still running
    let response = handler::handle_remove(container_id, state, deps).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(message.contains("still running"));
        }
        _ => panic!("expected Error, got {:?}", response),
    }
}

// ---------------------------------------------------------------------------
// Integration-style test: Full workflow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_full_container_lifecycle() {
    let deps = create_test_deps();
    let state = create_test_state();

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
    let run_response = handler::handle_run(
        "nginx".to_string(),
        Some("alpine".to_string()),
        vec!["/bin/sh".to_string(), "-c".to_string(), "echo test".to_string()],
        Some(256 * 1024 * 1024), // 256MB
        Some(750),               // CPU weight
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
