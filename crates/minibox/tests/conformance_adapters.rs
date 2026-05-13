//! Conformance tests for adapter trait implementations.
//!
//! This file tests adapter contracts that are not covered by existing conformance
//! suites. It focuses on:
//! - [`NoopNetwork`](minibox::adapters::NoopNetwork) adapter (from network/none.rs)
//! - [`NoopLimiter`](minibox::adapters::NoopLimiter) adapter (from gke.rs)
//! - [`AsAny`](minibox_core::domain::AsAny) downcasting for trait objects
//!
//! All tests use mocks or no-op adapters — no kernel/cgroup interaction or
//! network calls. Each test creates a fresh adapter to avoid shared state.

use minibox::adapters::{NoopLimiter, NoopNetwork};
use minibox::testing::mocks::{MockRegistry, MockRuntime};
use minibox_core::domain::{
    ContainerRuntime, ImageRegistry, NetworkConfig, NetworkProvider, ResourceLimiter,
};
use std::any::Any;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// NoopNetwork conformance tests
// ---------------------------------------------------------------------------

/// `setup` returns an empty string — no network namespace is created.
#[tokio::test]
async fn conformance_noop_network_setup_returns_empty_string() {
    let net = NoopNetwork::new();
    let config = NetworkConfig::default();
    let result = net.setup("container-conformance-1", &config).await;

    assert!(result.is_ok(), "setup must succeed on NoopNetwork");
    assert_eq!(
        result.expect("unwrap in test"),
        "",
        "setup must return empty string (no netns created)"
    );
}

/// `attach` returns Ok(()) — container remains in isolated namespace, no changes made.
#[tokio::test]
async fn conformance_noop_network_attach_succeeds() {
    let net = NoopNetwork::new();
    let result = net.attach("container-conformance-2", 99999).await;

    assert!(result.is_ok(), "attach must succeed on NoopNetwork");
}

/// `cleanup` returns Ok(()) — no-op since nothing was created.
#[tokio::test]
async fn conformance_noop_network_cleanup_succeeds() {
    let net = NoopNetwork::new();
    let result = net.cleanup("container-conformance-3").await;

    assert!(result.is_ok(), "cleanup must succeed on NoopNetwork");
}

/// `stats` returns default (all-zero) NetworkStats.
#[tokio::test]
async fn conformance_noop_network_stats_returns_zero() {
    let net = NoopNetwork::new();
    let result = net.stats("container-conformance-4").await;

    assert!(result.is_ok(), "stats must succeed on NoopNetwork");

    let stats = result.expect("unwrap in test");
    assert_eq!(stats.rx_bytes, 0, "rx_bytes must be 0");
    assert_eq!(stats.rx_packets, 0, "rx_packets must be 0");
    assert_eq!(stats.rx_errors, 0, "rx_errors must be 0");
    assert_eq!(stats.rx_dropped, 0, "rx_dropped must be 0");
    assert_eq!(stats.tx_bytes, 0, "tx_bytes must be 0");
    assert_eq!(stats.tx_packets, 0, "tx_packets must be 0");
    assert_eq!(stats.tx_errors, 0, "tx_errors must be 0");
    assert_eq!(stats.tx_dropped, 0, "tx_dropped must be 0");
}

// ---------------------------------------------------------------------------
// NoopNetwork AsAny downcasting
// ---------------------------------------------------------------------------

/// `NoopNetwork` can be cast to `Arc<dyn NetworkProvider>` and downcast back.
#[tokio::test]
async fn conformance_noop_network_as_any_downcast() {
    let concrete = Arc::new(NoopNetwork::new());

    // Cast to trait object
    let trait_obj: Arc<dyn NetworkProvider> = concrete.clone() as Arc<dyn NetworkProvider>;

    // Downcast back to concrete type
    let any_ref: &dyn Any = trait_obj.as_any();
    let downcast = any_ref.downcast_ref::<NoopNetwork>();

    assert!(
        downcast.is_some(),
        "downcast_ref must succeed for NoopNetwork"
    );
    assert!(
        downcast
            .expect("unwrap in test")
            .setup("test-1", &NetworkConfig::default())
            .await
            .is_ok(),
        "downcast result must be usable"
    );
}

// ---------------------------------------------------------------------------
// NoopLimiter conformance tests
// ---------------------------------------------------------------------------

/// `NoopLimiter::create` returns Ok with a sentinel cgroup path containing the container_id.
#[test]
fn conformance_noop_limiter_create_returns_path() {
    use minibox_core::domain::ResourceConfig;

    let limiter = NoopLimiter::new();
    let config = ResourceConfig {
        memory_limit_bytes: Some(256 * 1024 * 1024),
        cpu_weight: Some(512),
        pids_max: None,
        io_max_bytes_per_sec: None,
    };

    let result = limiter.create("noop-container-1", &config);
    assert!(result.is_ok(), "create must succeed on NoopLimiter");

    let path = result.expect("unwrap in test");
    assert!(
        path.contains("noop-container-1"),
        "returned path must reference the container_id, got: {path}"
    );
}

/// `NoopLimiter::add_process` returns Ok(()) — no-op for any container and PID.
#[test]
fn conformance_noop_limiter_add_process_succeeds() {
    let limiter = NoopLimiter::new();
    let result = limiter.add_process("noop-container-2", 54321);

    assert!(result.is_ok(), "add_process must succeed on NoopLimiter");
}

/// `NoopLimiter::cleanup` returns Ok(()) — no-op.
#[test]
fn conformance_noop_limiter_cleanup_succeeds() {
    let limiter = NoopLimiter::new();
    let result = limiter.cleanup("noop-container-3");

    assert!(result.is_ok(), "cleanup must succeed on NoopLimiter");
}

// ---------------------------------------------------------------------------
// NoopLimiter AsAny downcasting
// ---------------------------------------------------------------------------

/// `NoopLimiter` can be cast to `Arc<dyn ResourceLimiter>` and downcast back.
#[test]
fn conformance_noop_limiter_as_any_downcast() {
    let concrete = Arc::new(NoopLimiter::new());

    // Cast to trait object
    let trait_obj: Arc<dyn ResourceLimiter> = concrete as Arc<dyn ResourceLimiter>;

    // Downcast back to concrete type
    let any_ref: &dyn Any = trait_obj.as_any();
    let downcast = any_ref.downcast_ref::<NoopLimiter>();

    assert!(
        downcast.is_some(),
        "downcast_ref must succeed for NoopLimiter"
    );
}

// ---------------------------------------------------------------------------
// MockRegistry AsAny downcasting (from minibox_testers)
// ---------------------------------------------------------------------------

/// `MockRegistry` from minibox::testing can be cast to `Arc<dyn ImageRegistry>`
/// and downcast back to the concrete type.
#[tokio::test]
async fn conformance_mock_registry_as_any_downcast() {
    let concrete = Arc::new(MockRegistry::new().with_cached_image("library/test", "latest"));

    // Cast to trait object
    let trait_obj: Arc<dyn ImageRegistry> = concrete.clone() as Arc<dyn ImageRegistry>;

    // Downcast back to concrete type
    let any_ref: &dyn Any = trait_obj.as_any();
    let downcast = any_ref.downcast_ref::<MockRegistry>();

    assert!(
        downcast.is_some(),
        "downcast_ref must succeed for MockRegistry"
    );

    // Verify the downcast result is usable
    assert!(
        downcast.expect("unwrap in test").has_image("library/test", "latest").await,
        "downcast result must be usable and cached image must return true"
    );
}

// ---------------------------------------------------------------------------
// MockRuntime AsAny downcasting (from minibox_testers)
// ---------------------------------------------------------------------------

/// `MockRuntime` from minibox::testing can be cast to `Arc<dyn ContainerRuntime>`
/// and downcast back to the concrete type.
#[tokio::test]
async fn conformance_mock_runtime_as_any_downcast() {
    let concrete = Arc::new(MockRuntime::new());

    // Cast to trait object
    let trait_obj: Arc<dyn ContainerRuntime> = concrete.clone() as Arc<dyn ContainerRuntime>;

    // Downcast back to concrete type
    let any_ref: &dyn Any = trait_obj.as_any();
    let downcast = any_ref.downcast_ref::<MockRuntime>();

    assert!(
        downcast.is_some(),
        "downcast_ref must succeed for MockRuntime"
    );

    // Verify the downcast result is usable (spawn_count accessor)
    assert_eq!(
        downcast.expect("unwrap in test").spawn_count(),
        0,
        "downcast result must be usable (spawn_count returns a u32)"
    );
}

// ---------------------------------------------------------------------------
// Adapter composition
// ---------------------------------------------------------------------------

/// Multiple adapters can coexist in the same async context without conflict.
#[tokio::test]
async fn conformance_adapters_can_coexist() {
    let network = NoopNetwork::new();
    let limiter = NoopLimiter::new();
    let config = NetworkConfig::default();

    // Both can be used together
    let net_result = network.setup("coexist-1", &config).await;
    let lim_result = limiter.add_process("coexist-1", 12345);

    assert!(net_result.is_ok());
    assert!(lim_result.is_ok());
}

/// Trait objects can be stored in heterogeneous collections by downcasting.
#[test]
fn conformance_adapters_downcast_from_trait_objects() {
    let noop_limiter: Arc<dyn ResourceLimiter> =
        Arc::new(NoopLimiter::new()) as Arc<dyn ResourceLimiter>;

    // Downcast to check type at runtime
    let any_ref: &dyn Any = noop_limiter.as_any();
    let is_noop = any_ref.downcast_ref::<NoopLimiter>().is_some();

    assert!(is_noop, "should be able to identify NoopLimiter at runtime");
}
