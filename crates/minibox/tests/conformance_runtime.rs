//! Conformance tests for the `ContainerRuntime` trait contract.
//!
//! All tests use `MockRuntime` from `minibox-testers` — no syscalls are made.
//! Each test creates a fresh mock to avoid shared state.

use minibox_core::domain::{ContainerRuntime, ContainerSpawnConfig};
use minibox_testers::mocks::runtime::MockRuntime;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_spawn_config() -> ContainerSpawnConfig {
    ContainerSpawnConfig {
        rootfs: std::path::PathBuf::from("/mock/rootfs"),
        command: "/bin/sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "conformance-test".to_string(),
        cgroup_path: std::path::PathBuf::from("/mock/cgroup/conformanceruntime01"),
        capture_output: false,
        hooks: minibox_core::domain::ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![],
        privileged: false,
    }
}

// ---------------------------------------------------------------------------
// Spawn success invariants
// ---------------------------------------------------------------------------

/// A successful spawn must return a PID greater than zero.
#[tokio::test]
async fn runtime_spawn_returns_nonzero_pid() {
    let runtime = MockRuntime::new();
    let result = runtime
        .spawn_process(&default_spawn_config())
        .await
        .expect("spawn_process must succeed on default MockRuntime");
    assert!(
        result.pid > 0,
        "spawned PID must be > 0, got: {}",
        result.pid
    );
}

/// `spawn_process` must increment `spawn_count`.
#[tokio::test]
async fn runtime_spawn_increments_count() {
    let runtime = MockRuntime::new();
    runtime
        .spawn_process(&default_spawn_config())
        .await
        .expect("spawn_process must succeed");
    assert_eq!(
        runtime.spawn_count(),
        1,
        "spawn_count must be 1 after one spawn"
    );
}

/// Two successive spawns must return different PIDs (monotonically increasing).
#[tokio::test]
async fn runtime_successive_spawns_get_different_pids() {
    let runtime = MockRuntime::new();
    let cfg = default_spawn_config();
    let first = runtime
        .spawn_process(&cfg)
        .await
        .expect("first spawn must succeed");
    let second = runtime
        .spawn_process(&cfg)
        .await
        .expect("second spawn must succeed");
    assert_ne!(
        first.pid, second.pid,
        "successive spawns must return different PIDs"
    );
}

// ---------------------------------------------------------------------------
// Spawn failure invariants
// ---------------------------------------------------------------------------

/// `spawn_process` on a failure-configured mock must return Err.
#[tokio::test]
async fn runtime_spawn_failure_returns_err() {
    let runtime = MockRuntime::new().with_spawn_failure();
    let result = runtime.spawn_process(&default_spawn_config()).await;
    assert!(
        result.is_err(),
        "spawn_process must return Err on a failure-configured mock"
    );
}

/// A failed `spawn_process` still increments `spawn_count` — the attempt was made.
#[tokio::test]
async fn runtime_spawn_failure_still_increments_count() {
    let runtime = MockRuntime::new().with_spawn_failure();
    let _ = runtime.spawn_process(&default_spawn_config()).await;
    assert_eq!(
        runtime.spawn_count(),
        1,
        "spawn_count must increment even on a failed spawn"
    );
}

// ---------------------------------------------------------------------------
// Capabilities invariant
// ---------------------------------------------------------------------------

/// `capabilities()` must return a `RuntimeCapabilities` struct without panicking.
#[test]
fn runtime_capabilities_returns_struct() {
    let runtime = MockRuntime::new();
    // This must not panic — the mock always returns a zeroed capabilities struct.
    let caps = runtime.capabilities();
    // MockRuntime declares no special capabilities — verify the contract, not values.
    let _ = caps.supports_user_namespaces;
    let _ = caps.supports_cgroups_v2;
    let _ = caps.supports_overlay_fs;
    let _ = caps.supports_network_isolation;
}

// ---------------------------------------------------------------------------
// Sync/async consistency invariant
// ---------------------------------------------------------------------------

/// `spawn_process_sync` and async `spawn_process` share state: both succeed when
/// the mock is in the default (success) configuration.
#[tokio::test]
async fn runtime_spawn_sync_consistent_with_async() {
    let runtime = MockRuntime::new();
    let cfg = default_spawn_config();

    let async_result = runtime
        .spawn_process(&cfg)
        .await
        .expect("async spawn must succeed on default MockRuntime");

    let sync_result = runtime
        .spawn_process_sync(&cfg)
        .expect("sync spawn must succeed on default MockRuntime");

    assert!(async_result.pid > 0, "async PID must be > 0");
    assert!(sync_result.pid > 0, "sync PID must be > 0");
    assert_ne!(
        async_result.pid, sync_result.pid,
        "async and sync spawns must return different (sequential) PIDs"
    );
    assert_eq!(
        runtime.spawn_count(),
        2,
        "both spawns must be counted — spawn_count must be 2"
    );
}

/// When configured for failure, both sync and async variants return Err.
#[tokio::test]
async fn runtime_spawn_failure_consistent_sync_and_async() {
    let runtime = MockRuntime::new().with_spawn_failure();
    let cfg = default_spawn_config();

    let async_result = runtime.spawn_process(&cfg).await;
    let sync_result = runtime.spawn_process_sync(&cfg);

    assert!(
        async_result.is_err(),
        "async spawn must return Err on failure-configured mock"
    );
    assert!(
        sync_result.is_err(),
        "sync spawn must return Err on failure-configured mock"
    );
    assert_eq!(
        runtime.spawn_count(),
        2,
        "both failed spawns must be counted"
    );
}
