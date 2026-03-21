//! Tests that exercise handler behaviour when adapters are swapped between
//! success and failure configurations.
//!
//! The goal is to verify that each failure point in the handler pipeline
//! (registry pull, filesystem setup, resource limiter create, runtime spawn)
//! produces the expected `DaemonResponse` variant, and that success paths
//! work end-to-end regardless of which concrete mock is wired in.

use daemonbox::handler::{self, HandlerDependencies};
use daemonbox::state::DaemonState;
use minibox_lib::adapters::mocks::{MockFilesystem, MockLimiter, MockRegistry, MockRuntime};
use minibox_lib::protocol::DaemonResponse;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared helpers (mirror handler_tests.rs patterns exactly)
// ---------------------------------------------------------------------------

/// Drive `handle_run` through a one-shot channel and return the first message.
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
        state,
        deps,
        tx,
    )
    .await;
    rx.recv().await.expect("handler sent no response")
}

fn make_deps(
    registry: MockRegistry,
    filesystem: MockFilesystem,
    resource_limiter: MockLimiter,
    runtime: MockRuntime,
    tmp: &TempDir,
) -> Arc<HandlerDependencies> {
    Arc::new(HandlerDependencies {
        registry: Arc::new(registry),
        filesystem: Arc::new(filesystem),
        resource_limiter: Arc::new(resource_limiter),
        runtime: Arc::new(runtime),
        containers_base: tmp.path().join("containers"),
        run_containers_base: tmp.path().join("run"),
    })
}

fn make_state(tmp: &TempDir) -> Arc<DaemonState> {
    let image_store = minibox_lib::image::ImageStore::new(tmp.path().join("images")).unwrap();
    Arc::new(DaemonState::new(image_store, tmp.path()))
}

// ---------------------------------------------------------------------------
// Test 1 — all-success mocks: Run request returns ContainerCreated
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_run_with_all_success_adapters() {
    let tmp = TempDir::new().unwrap();
    let deps = make_deps(
        MockRegistry::new().with_cached_image("library/alpine", "latest"),
        MockFilesystem::new(),
        MockLimiter::new(),
        MockRuntime::new(),
        &tmp,
    );
    let state = make_state(&tmp);

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
            assert_eq!(id.len(), 16);

            // Verify container landed in state
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            assert!(state.get_container(&id).await.is_some());
        }
        other => panic!("expected ContainerCreated, got {other:?}"),
    }

    // Image was pre-cached, so no pull should have been issued
    let registry = deps
        .registry
        .as_any()
        .downcast_ref::<MockRegistry>()
        .unwrap();
    assert_eq!(registry.pull_count(), 0);
}

// ---------------------------------------------------------------------------
// Test 2 — registry pull failure returns Error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_run_with_registry_pull_failure() {
    let tmp = TempDir::new().unwrap();
    // Registry has no cached image AND will fail on pull — handler must propagate the error.
    let deps = make_deps(
        MockRegistry::new().with_pull_failure(),
        MockFilesystem::new(),
        MockLimiter::new(),
        MockRuntime::new(),
        &tmp,
    );
    let state = make_state(&tmp);

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
            assert!(
                message.contains("mock pull failure"),
                "expected pull error message, got: {message}"
            );
        }
        other => panic!("expected Error response, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 3 — filesystem setup failure returns Error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_run_with_filesystem_setup_failure() {
    let tmp = TempDir::new().unwrap();
    let deps = make_deps(
        MockRegistry::new().with_cached_image("library/alpine", "latest"),
        MockFilesystem::new().with_setup_failure(),
        MockLimiter::new(),
        MockRuntime::new(),
        &tmp,
    );
    let state = make_state(&tmp);

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
                message.contains("filesystem setup failure"),
                "expected filesystem error message, got: {message}"
            );
        }
        other => panic!("expected Error response, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 4 — resource limiter create failure returns Error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_run_with_limiter_create_failure() {
    let tmp = TempDir::new().unwrap();
    let deps = make_deps(
        MockRegistry::new().with_cached_image("library/alpine", "latest"),
        MockFilesystem::new(),
        MockLimiter::new().with_create_failure(),
        MockRuntime::new(),
        &tmp,
    );
    let state = make_state(&tmp);

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
            assert!(
                message.contains("resource limiter create failure"),
                "expected limiter error message, got: {message}"
            );
        }
        other => panic!("expected Error response, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 5 — handle_list works regardless of adapter state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_list_works_with_failing_adapters() {
    let tmp = TempDir::new().unwrap();
    // All adapters configured to fail — list should still return an empty list
    // because it only touches DaemonState, not any infrastructure adapter.
    let _deps = make_deps(
        MockRegistry::new().with_pull_failure(),
        MockFilesystem::new().with_setup_failure(),
        MockLimiter::new().with_create_failure(),
        MockRuntime::new().with_spawn_failure(),
        &tmp,
    );
    let state = make_state(&tmp);

    let response = handler::handle_list(state).await;

    match response {
        DaemonResponse::ContainerList { containers } => {
            assert!(containers.is_empty());
        }
        other => panic!("expected ContainerList, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 6 — handle_stop for unknown container returns Error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_stop_unknown_container_returns_error() {
    let tmp = TempDir::new().unwrap();
    let state = make_state(&tmp);

    let response = handler::handle_stop("does_not_exist_abc123".to_string(), state).await;

    match response {
        DaemonResponse::Error { message } => {
            assert!(
                message.contains("not found"),
                "expected 'not found' in message, got: {message}"
            );
        }
        other => panic!("expected Error response, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 7 — swapping from success registry to pull-failure adapter mid-suite
// ---------------------------------------------------------------------------
//
// Demonstrates that the same handler wiring produces different outcomes when
// only the registry is swapped — the filesystem/limiter/runtime remain identical.

#[tokio::test]
async fn test_pull_success_then_pull_failure_different_deps() {
    let tmp = TempDir::new().unwrap();

    // First request: registry succeeds (image not cached, pulled on demand).
    let deps_ok = make_deps(
        MockRegistry::new(), // no cached image, but pull succeeds
        MockFilesystem::new(),
        MockLimiter::new(),
        MockRuntime::new(),
        &tmp,
    );
    let state = make_state(&tmp);

    let ok_response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps_ok.clone(),
    )
    .await;

    assert!(
        matches!(ok_response, DaemonResponse::ContainerCreated { .. }),
        "expected ContainerCreated for success deps, got {ok_response:?}"
    );

    let registry_ok = deps_ok
        .registry
        .as_any()
        .downcast_ref::<MockRegistry>()
        .unwrap();
    assert_eq!(
        registry_ok.pull_count(),
        1,
        "pull should have been called once"
    );

    // Second request: registry fails on pull — completely separate deps.
    let tmp2 = TempDir::new().unwrap();
    let deps_fail = make_deps(
        MockRegistry::new().with_pull_failure(),
        MockFilesystem::new(),
        MockLimiter::new(),
        MockRuntime::new(),
        &tmp2,
    );
    let state2 = make_state(&tmp2);

    let fail_response = handle_run_once(
        "alpine".to_string(),
        None,
        vec!["/bin/sh".to_string()],
        None,
        None,
        false,
        state2,
        deps_fail,
    )
    .await;

    assert!(
        matches!(fail_response, DaemonResponse::Error { .. }),
        "expected Error for failing registry deps, got {fail_response:?}"
    );
}
