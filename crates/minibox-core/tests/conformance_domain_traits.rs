//! Conformance tests for domain trait contracts.
//!
//! These tests verify that:
//! - AsAny trait enables downcasting for all mock adapters
//! - Mock adapters implement their traits correctly
//! - Domain types behave as expected
//! - Builder patterns work as documented
//!
//! Tests use the mock adapters available via the `test-utils` feature.

use minibox_core::adapters::mocks::{
    MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};
use minibox_core::domain::{
    AsAny, ChildInit, ContainerHooks, ContainerRuntime, ImageMetadata, ImageRegistry, LayerInfo,
    NetworkProvider, ResourceConfig, ResourceLimiter, RootfsSetup, RuntimeCapabilities,
};
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test: AsAny Downcasting Works for All Mocks
// ---------------------------------------------------------------------------

#[test]
fn conformance_as_any_downcast_mock_registry() {
    let registry = Arc::new(MockRegistry::new());
    let as_any = registry.as_any();

    let downcast = as_any.downcast_ref::<MockRegistry>();
    assert!(
        downcast.is_some(),
        "MockRegistry should downcast to concrete type"
    );
}

#[test]
fn conformance_as_any_downcast_mock_filesystem() {
    let filesystem = Arc::new(MockFilesystem::new());
    let as_any = filesystem.as_any();

    let downcast = as_any.downcast_ref::<MockFilesystem>();
    assert!(
        downcast.is_some(),
        "MockFilesystem should downcast to concrete type"
    );
}

#[test]
fn conformance_as_any_downcast_mock_limiter() {
    let limiter = Arc::new(MockLimiter::new());
    let as_any = limiter.as_any();

    let downcast = as_any.downcast_ref::<MockLimiter>();
    assert!(
        downcast.is_some(),
        "MockLimiter should downcast to concrete type"
    );
}

#[test]
fn conformance_as_any_downcast_mock_runtime() {
    let runtime = Arc::new(MockRuntime::new());
    let as_any = runtime.as_any();

    let downcast = as_any.downcast_ref::<MockRuntime>();
    assert!(
        downcast.is_some(),
        "MockRuntime should downcast to concrete type"
    );
}

#[test]
fn conformance_as_any_downcast_mock_network() {
    let network = Arc::new(MockNetwork::new());
    let as_any = network.as_any();

    let downcast = as_any.downcast_ref::<MockNetwork>();
    assert!(
        downcast.is_some(),
        "MockNetwork should downcast to concrete type"
    );
}

// ---------------------------------------------------------------------------
// Test: AsAny Downcasting to Wrong Type Returns None
// ---------------------------------------------------------------------------

#[test]
fn conformance_as_any_downcast_wrong_type_registry_to_runtime() {
    let registry = Arc::new(MockRegistry::new());
    let as_any = registry.as_any();

    let downcast = as_any.downcast_ref::<MockRuntime>();
    assert!(
        downcast.is_none(),
        "Downcasting MockRegistry to MockRuntime should return None"
    );
}

#[test]
fn conformance_as_any_downcast_wrong_type_filesystem_to_limiter() {
    let filesystem = Arc::new(MockFilesystem::new());
    let as_any = filesystem.as_any();

    let downcast = as_any.downcast_ref::<MockLimiter>();
    assert!(
        downcast.is_none(),
        "Downcasting MockFilesystem to MockLimiter should return None"
    );
}

#[test]
fn conformance_as_any_downcast_wrong_type_network_to_registry() {
    let network = Arc::new(MockNetwork::new());
    let as_any = network.as_any();

    let downcast = as_any.downcast_ref::<MockRegistry>();
    assert!(
        downcast.is_none(),
        "Downcasting MockNetwork to MockRegistry should return None"
    );
}

// ---------------------------------------------------------------------------
// Test: MockRegistry Pull Success Returns ImageMetadata with Expected Fields
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_mock_registry_pull_success_populates_metadata() {
    let registry = MockRegistry::new();
    let image_ref = minibox_core::image::reference::ImageRef {
        registry: "docker.io".to_string(),
        namespace: "library".to_string(),
        name: "alpine".to_string(),
        tag: "latest".to_string(),
    };

    let metadata = registry
        .pull_image(&image_ref)
        .await
        .expect("pull should succeed");

    assert_eq!(metadata.name, "library/alpine");
    assert_eq!(metadata.tag, "latest");
    assert_eq!(metadata.layers.len(), 2, "mock returns 2 layers");

    let first_layer = &metadata.layers[0];
    assert_eq!(first_layer.digest, "sha256:mock-layer-1");
    assert_eq!(first_layer.size, 1024);

    let second_layer = &metadata.layers[1];
    assert_eq!(second_layer.digest, "sha256:mock-layer-2");
    assert_eq!(second_layer.size, 2048);
}

#[tokio::test]
async fn conformance_mock_registry_pull_failure_returns_error() {
    let registry = MockRegistry::new().with_pull_failure();
    let image_ref = minibox_core::image::reference::ImageRef {
        registry: "docker.io".to_string(),
        namespace: "library".to_string(),
        name: "alpine".to_string(),
        tag: "latest".to_string(),
    };

    let result = registry.pull_image(&image_ref).await;
    assert!(result.is_err(), "pull should fail when configured");
}

// ---------------------------------------------------------------------------
// Test: MockRegistry with_cached_image then has_image Returns True
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_mock_registry_cached_image_lookup() {
    let registry = MockRegistry::new().with_cached_image("alpine", "latest");

    let has_image = registry.has_image("alpine", "latest").await;
    assert!(has_image, "has_image should return true for cached image");

    let not_present = registry.has_image("ubuntu", "22.04").await;
    assert!(
        !not_present,
        "has_image should return false for non-cached image"
    );
}

#[tokio::test]
async fn conformance_mock_registry_multiple_cached_images() {
    let registry = MockRegistry::new()
        .with_cached_image("alpine", "latest")
        .with_cached_image("ubuntu", "22.04")
        .with_cached_image("debian", "bookworm");

    assert!(registry.has_image("alpine", "latest").await);
    assert!(registry.has_image("ubuntu", "22.04").await);
    assert!(registry.has_image("debian", "bookworm").await);
    assert!(!registry.has_image("centos", "7").await);
}

// ---------------------------------------------------------------------------
// Test: MockRegistry Pull Count Tracking
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_mock_registry_pull_count_incremented() {
    let registry = MockRegistry::new();
    let image_ref = minibox_core::image::reference::ImageRef {
        registry: "docker.io".to_string(),
        namespace: "library".to_string(),
        name: "alpine".to_string(),
        tag: "latest".to_string(),
    };

    assert_eq!(registry.pull_count(), 0);

    let _ = registry.pull_image(&image_ref).await;
    assert_eq!(registry.pull_count(), 1);

    let _ = registry.pull_image(&image_ref).await;
    assert_eq!(registry.pull_count(), 2);
}

// ---------------------------------------------------------------------------
// Test: MockLimiter Create/Add_Process/Cleanup Lifecycle
// ---------------------------------------------------------------------------

#[test]
fn conformance_mock_limiter_full_lifecycle() {
    let limiter = MockLimiter::new();
    let container_id = "test-container-123";
    let config = ResourceConfig {
        memory_limit_bytes: Some(1024 * 1024 * 512), // 512 MB
        cpu_weight: Some(256),
        pids_max: None,
        io_max_bytes_per_sec: None,
    };

    assert_eq!(limiter.create_count(), 0);
    assert_eq!(limiter.cleanup_count(), 0);

    let result = limiter.create(container_id, &config);
    assert!(result.is_ok(), "create should succeed");
    assert_eq!(limiter.create_count(), 1);

    let cgroup_path = result.expect("create returned Ok");
    assert!(
        cgroup_path.contains(container_id),
        "cgroup path should contain container ID"
    );

    let add_result = limiter.add_process(container_id, 12345);
    assert!(add_result.is_ok(), "add_process should succeed");

    let cleanup_result = limiter.cleanup(container_id);
    assert!(cleanup_result.is_ok(), "cleanup should succeed");
    assert_eq!(limiter.cleanup_count(), 1);
}

#[test]
fn conformance_mock_limiter_create_failure() {
    let limiter = MockLimiter::new().with_create_failure();
    let config = ResourceConfig::default();

    let result = limiter.create("test-container", &config);
    assert!(result.is_err(), "create should fail when configured");
    assert_eq!(
        limiter.create_count(),
        1,
        "attempt should be counted even on failure"
    );
}

// ---------------------------------------------------------------------------
// Test: MockFilesystem RootfsSetup/ChildInit Lifecycle
// ---------------------------------------------------------------------------

#[test]
fn conformance_mock_filesystem_rootfs_setup_success() {
    let filesystem = MockFilesystem::new();
    let layers = vec![PathBuf::from("/layer1"), PathBuf::from("/layer2")];
    let container_dir = PathBuf::from("/containers/test-123");

    let result = filesystem.setup_rootfs(&layers, &container_dir);
    assert!(result.is_ok(), "setup_rootfs should succeed");
    assert_eq!(filesystem.setup_count(), 1);

    let layout = result.expect("setup_rootfs returned Ok");
    assert_eq!(layout.merged_dir, container_dir.join("merged"));
}

#[test]
fn conformance_mock_filesystem_setup_failure() {
    let filesystem = MockFilesystem::new().with_setup_failure();
    let layers = vec![];
    let container_dir = PathBuf::from("/containers/test-123");

    let result = filesystem.setup_rootfs(&layers, &container_dir);
    assert!(result.is_err(), "setup_rootfs should fail when configured");
    assert_eq!(filesystem.setup_count(), 1);
}

#[test]
fn conformance_mock_filesystem_cleanup_success() {
    let filesystem = MockFilesystem::new();
    let container_dir = PathBuf::from("/containers/test-123");

    let result = filesystem.cleanup(&container_dir);
    assert!(result.is_ok(), "cleanup should succeed");
    assert_eq!(filesystem.cleanup_count(), 1);
}

#[test]
fn conformance_mock_filesystem_pivot_root_success() {
    let filesystem = MockFilesystem::new();
    let new_root = PathBuf::from("/containers/test-123/rootfs");

    let result = filesystem.pivot_root(&new_root);
    assert!(result.is_ok(), "pivot_root should succeed");
}

// ---------------------------------------------------------------------------
// Test: MockRuntime Capabilities
// ---------------------------------------------------------------------------

#[test]
fn conformance_mock_runtime_capabilities_returns_struct() {
    let runtime = MockRuntime::new();
    let capabilities = runtime.capabilities();

    assert!(!capabilities.supports_user_namespaces);
    assert!(!capabilities.supports_cgroups_v2);
    assert!(!capabilities.supports_overlay_fs);
    assert!(!capabilities.supports_network_isolation);
    assert_eq!(capabilities.max_containers, None);
}

#[test]
fn conformance_mock_runtime_capabilities_type() {
    let runtime = MockRuntime::new();
    let caps: RuntimeCapabilities = runtime.capabilities();

    assert!(
        !caps.supports_user_namespaces,
        "mock runtime does not support user namespaces"
    );
}

// ---------------------------------------------------------------------------
// Test: MockRuntime Spawn Process
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_mock_runtime_spawn_increments_pid() {
    use minibox_core::domain::ContainerSpawnConfig;

    let runtime = MockRuntime::new();
    let config = ContainerSpawnConfig {
        rootfs: PathBuf::from("/rootfs"),
        command: "sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "container".to_string(),
        cgroup_path: PathBuf::from("/sys/fs/cgroup/test"),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![],
        privileged: false,
        image_ref: None,
    };

    let result1 = runtime.spawn_process(&config).await;
    assert!(result1.is_ok());
    let pid1 = result1.expect("spawn_process returned Ok").pid;
    assert_eq!(pid1, 10000);

    let result2 = runtime.spawn_process(&config).await;
    assert!(result2.is_ok());
    let pid2 = result2.expect("spawn_process returned Ok").pid;
    assert_eq!(pid2, 10001);
}

#[tokio::test]
async fn conformance_mock_runtime_spawn_failure() {
    use minibox_core::domain::ContainerSpawnConfig;

    let runtime = MockRuntime::new().with_spawn_failure();
    let config = ContainerSpawnConfig {
        rootfs: PathBuf::from("/rootfs"),
        command: "sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "container".to_string(),
        cgroup_path: PathBuf::from("/sys/fs/cgroup/test"),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![],
        privileged: false,
        image_ref: None,
    };

    let result = runtime.spawn_process(&config).await;
    assert!(result.is_err(), "spawn_process should fail when configured");
    assert_eq!(
        runtime.spawn_count(),
        1,
        "attempt should be counted even on failure"
    );
}

// ---------------------------------------------------------------------------
// Test: MockNetwork Setup/Attach/Cleanup Lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn conformance_mock_network_full_lifecycle() {
    use minibox_core::domain::NetworkConfig;

    let network = MockNetwork::new();
    let container_id = "test-container-456";
    let config = NetworkConfig::default();

    assert_eq!(network.setup_count(), 0);
    assert_eq!(network.cleanup_count(), 0);

    let setup_result = network.setup(container_id, &config).await;
    assert!(setup_result.is_ok(), "setup should succeed");
    assert_eq!(network.setup_count(), 1);

    let netns = setup_result.expect("setup returned Ok");
    assert_eq!(netns, "/mock/netns");

    let attach_result = network.attach(container_id, 54321).await;
    assert!(attach_result.is_ok(), "attach should succeed");

    let cleanup_result = network.cleanup(container_id).await;
    assert!(cleanup_result.is_ok(), "cleanup should succeed");
    assert_eq!(network.cleanup_count(), 1);
}

#[tokio::test]
async fn conformance_mock_network_setup_failure() {
    use minibox_core::domain::NetworkConfig;

    let network = MockNetwork::new().with_setup_failure();
    let config = NetworkConfig::default();

    let result = network.setup("test-container", &config).await;
    assert!(result.is_err(), "setup should fail when configured");
    assert_eq!(network.setup_count(), 1);
}

#[tokio::test]
async fn conformance_mock_network_cleanup_failure() {
    use minibox_core::domain::NetworkConfig;

    let network = MockNetwork::new().with_cleanup_failure();
    let config = NetworkConfig::default();

    let _ = network.setup("test-container", &config).await;
    let cleanup_result = network.cleanup("test-container").await;
    assert!(
        cleanup_result.is_err(),
        "cleanup should fail when configured"
    );
    assert_eq!(network.cleanup_count(), 1);
}

#[tokio::test]
async fn conformance_mock_network_stats() {
    let network = MockNetwork::new();

    let stats_result = network.stats("test-container").await;
    assert!(stats_result.is_ok(), "stats should succeed");

    let stats = stats_result.expect("stats returned Ok");
    assert_eq!(stats.tx_bytes, 0, "default stats should have zero values");
    assert_eq!(stats.rx_bytes, 0);
}

// ---------------------------------------------------------------------------
// Test: ResourceConfig Default
// ---------------------------------------------------------------------------

#[test]
fn conformance_resource_config_default_all_none() {
    let config = ResourceConfig::default();

    assert_eq!(config.memory_limit_bytes, None);
    assert_eq!(config.cpu_weight, None);
    assert_eq!(config.pids_max, None);
    assert_eq!(config.io_max_bytes_per_sec, None);
}

#[test]
fn conformance_resource_config_with_values() {
    let config = ResourceConfig {
        memory_limit_bytes: Some(2048),
        cpu_weight: Some(512),
        pids_max: Some(100),
        io_max_bytes_per_sec: Some(1024 * 1024),
    };

    assert_eq!(config.memory_limit_bytes, Some(2048));
    assert_eq!(config.cpu_weight, Some(512));
    assert_eq!(config.pids_max, Some(100));
    assert_eq!(config.io_max_bytes_per_sec, Some(1024 * 1024));
}

// ---------------------------------------------------------------------------
// Test: ImageMetadata Fields
// ---------------------------------------------------------------------------

#[test]
fn conformance_image_metadata_construction_and_fields() {
    let metadata = ImageMetadata {
        name: "library/alpine".to_string(),
        tag: "3.18".to_string(),
        layers: vec![
            LayerInfo {
                digest: "sha256:abc123".to_string(),
                size: 5242880,
            },
            LayerInfo {
                digest: "sha256:def456".to_string(),
                size: 10485760,
            },
        ],
    };

    assert_eq!(metadata.name, "library/alpine");
    assert_eq!(metadata.tag, "3.18");
    assert_eq!(metadata.layers.len(), 2);

    let first = &metadata.layers[0];
    assert_eq!(first.digest, "sha256:abc123");
    assert_eq!(first.size, 5242880);
}

// ---------------------------------------------------------------------------
// Test: Trait Object Trait Bounds
// ---------------------------------------------------------------------------

#[test]
fn conformance_registry_as_trait_object() {
    let registry: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());

    // Verify we can call trait methods through the trait object
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Check that the trait object has the right Send + Sync bounds
        let _: Arc<dyn ImageRegistry + Send + Sync> = registry.clone();
    }));
    assert!(result.is_ok());
}

#[test]
fn conformance_resource_limiter_as_trait_object() {
    let limiter: Arc<dyn ResourceLimiter> = Arc::new(MockLimiter::new());

    // Verify we can call trait methods through the trait object
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Arc<dyn ResourceLimiter + Send + Sync> = limiter.clone();
    }));
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// Test: Integration — Mock Registry + Limiter Workflow
// ---------------------------------------------------------------------------

#[test]
fn conformance_registry_limiter_integration() {
    let registry = Arc::new(MockRegistry::new().with_cached_image("alpine", "latest"));
    let limiter = Arc::new(MockLimiter::new());

    let config = ResourceConfig {
        memory_limit_bytes: Some(512 * 1024 * 1024),
        cpu_weight: None,
        pids_max: None,
        io_max_bytes_per_sec: None,
    };

    let cgroup_result = limiter.create("test-container", &config);
    assert!(cgroup_result.is_ok());

    let has_image = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        registry.has_image_sync("alpine", "latest")
    }));
    assert!(has_image.expect("no panic"), "image should be cached");
}
