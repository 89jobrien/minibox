//! Conformance tests for the miniboxd composition-root wiring.
//!
//! Verifies:
//! - The `miniboxd` lib re-exports handler / state / server modules.
//! - `HandlerDependencies` can be assembled from mock adapters (no panic).
//! - `ContainerPolicy` default values match documented contract.
//! - `DaemonState` list/get lifecycle on empty state.
//! - `handle_list` always returns `ContainerList`.
//! - `handle_pull` with a failing registry returns `Error`.
//! - `handle_stop` / `handle_remove` on unknown ID return `Error`.
//!
//! No daemon process, no root, no network.

use daemonbox::handler::ContainerPolicy;
use minibox_core::protocol::DaemonResponse;
use minibox_testers::helpers::{make_mock_deps, make_mock_state};
use minibox_testers::mocks::MockRegistry;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Re-export contract: miniboxd lib must expose handler / state / server
//
// NOTE: These are compile-time guards. If a type is removed, this file will
// fail to compile, which is the intended signal. They do not exercise runtime
// behaviour and should not be counted toward runtime coverage metrics.
// ---------------------------------------------------------------------------

#[test]
fn miniboxd_lib_exposes_handler_module() {
    let _ = std::any::type_name::<miniboxd::handler::HandlerDependencies>();
}

#[test]
fn miniboxd_lib_exposes_state_module() {
    let _ = std::any::type_name::<miniboxd::state::DaemonState>();
}

#[test]
fn miniboxd_lib_exposes_server_module() {
    // Verify the module is accessible (type-check only — no public struct guaranteed).
    let _ = std::any::type_name::<miniboxd::server::PeerCreds>();
}

// ---------------------------------------------------------------------------
// HandlerDependencies construction
// ---------------------------------------------------------------------------

#[test]
fn handler_dependencies_can_be_constructed_with_mocks() {
    let tmp = TempDir::new().unwrap();
    let _deps = make_mock_deps(&tmp);
}

#[test]
fn handler_dependencies_policy_fields_accessible() {
    let tmp = TempDir::new().unwrap();
    let deps = make_mock_deps(&tmp);
    // make_mock_deps sets allow_privileged: true, allow_bind_mounts: true
    assert!(deps.policy.allow_privileged);
    assert!(deps.policy.allow_bind_mounts);
}

// ---------------------------------------------------------------------------
// ContainerPolicy default values
// ---------------------------------------------------------------------------

#[test]
fn container_policy_default_denies_privileged() {
    assert!(!ContainerPolicy::default().allow_privileged);
}

#[test]
fn container_policy_default_denies_bind_mounts() {
    assert!(!ContainerPolicy::default().allow_bind_mounts);
}

// ---------------------------------------------------------------------------
// DaemonState — empty-state contracts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn daemon_state_list_empty_on_creation() {
    let tmp = TempDir::new().unwrap();
    let state = make_mock_state(tmp.path());
    assert!(state.list_containers().await.is_empty());
}

#[tokio::test]
async fn daemon_state_get_missing_returns_none() {
    let tmp = TempDir::new().unwrap();
    let state = make_mock_state(tmp.path());
    assert!(state.get_container("no-such-id").await.is_none());
}

// ---------------------------------------------------------------------------
// handle_list — always ContainerList
// ---------------------------------------------------------------------------

#[tokio::test]
async fn handle_list_empty_state_returns_container_list() {
    let tmp = TempDir::new().unwrap();
    let state = make_mock_state(tmp.path());
    let response = miniboxd::handler::handle_list(state).await;
    assert!(
        matches!(response, DaemonResponse::ContainerList { ref containers } if containers.is_empty()),
        "expected empty ContainerList, got {response:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_pull — failure returns Error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn handle_pull_failure_returns_error_response() {
    let tmp = TempDir::new().unwrap();
    let state = make_mock_state(tmp.path());
    let deps = minibox_testers::helpers::make_mock_deps_with_registry(
        MockRegistry::new().with_pull_failure(),
        &tmp,
    );

    let response = miniboxd::handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state,
        deps,
    )
    .await;

    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "pull failure must return Error, got {response:?}"
    );
}

// ---------------------------------------------------------------------------
// handle_stop / handle_remove — unknown ID → Error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn handle_stop_nonexistent_returns_error() {
    let tmp = TempDir::new().unwrap();
    let state = make_mock_state(tmp.path());
    let deps = make_mock_deps(&tmp);
    let response =
        miniboxd::handler::handle_stop("no-such-container".to_string(), state, deps).await;
    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "stop of nonexistent container must return Error, got {response:?}"
    );
}

#[tokio::test]
async fn handle_remove_nonexistent_returns_error() {
    let tmp = TempDir::new().unwrap();
    let state = make_mock_state(tmp.path());
    let deps = make_mock_deps(&tmp);
    let response =
        miniboxd::handler::handle_remove("no-such-container".to_string(), state, deps).await;
    assert!(
        matches!(response, DaemonResponse::Error { .. }),
        "remove of nonexistent container must return Error, got {response:?}"
    );
}
