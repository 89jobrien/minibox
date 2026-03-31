//! Integration tests for miniboxd using real infrastructure.
//!
//! These tests require Linux with root privileges and verify the complete
//! stack against real Docker Hub, overlay filesystem, cgroups, and namespaces.
//!
//! **Requirements:**
//! - Linux kernel 5.0+ with cgroups v2
//! - Root privileges (sudo)
//! - Network access to Docker Hub
//! - overlay filesystem support
//!
//! **Running:**
//! ```bash
//! # On Linux as root
//! sudo -E cargo test -p miniboxd --test integration_tests -- --test-threads=1
//! ```
//!
//! **Note:** These tests modify system state (mounts, cgroups) and should run
//! sequentially (--test-threads=1) to avoid conflicts.

#![cfg(target_os = "linux")]

use mbx::adapters::{
    CgroupV2Limiter, DockerHubRegistry, LinuxNamespaceRuntime, NoopNetwork, OverlayFilesystem,
};
use mbx::image::ImageStore;
use mbx::protocol::DaemonResponse;
use miniboxd::handler::{self, HandlerDependencies};
use miniboxd::state::DaemonState;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

/// Helper to create real infrastructure dependencies with temporary storage.
fn create_real_deps() -> (Arc<HandlerDependencies>, Arc<DaemonState>, TempDir) {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let images_dir = temp_dir.path().join("images");
    let image_store = ImageStore::new(&images_dir).expect("failed to create image store");
    let state = Arc::new(DaemonState::new(image_store, temp_dir.path()));

    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(
            DockerHubRegistry::new(Arc::clone(&state.image_store))
                .expect("failed to create registry"),
        ),
        filesystem: Arc::new(OverlayFilesystem::new_with_base(images_dir)),
        resource_limiter: Arc::new(CgroupV2Limiter::new()),
        runtime: Arc::new(LinuxNamespaceRuntime::new()),
        network_provider: Arc::new(NoopNetwork::new()),
        containers_base: temp_dir.path().join("containers"),
        run_containers_base: temp_dir.path().join("run"),
        metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
    });

    (deps, state, temp_dir)
}

/// Check if running as root (required for integration tests).
fn require_root() {
    if unsafe { libc::geteuid() } != 0 {
        panic!("Integration tests require root privileges. Run with: sudo -E cargo test");
    }
}

/// Check if cgroups v2 is available.
fn require_cgroups_v2() {
    let cgroup_mount =
        std::fs::read_to_string("/proc/mounts").expect("failed to read /proc/mounts");

    if !cgroup_mount.contains("cgroup2") {
        panic!("cgroups v2 not available. Ensure /sys/fs/cgroup is mounted as cgroup2");
    }
}

// ---------------------------------------------------------------------------
// Image Pull Integration Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // Requires root and network
async fn test_pull_real_image_from_dockerhub() {
    require_root();

    let (deps, state, _temp) = create_real_deps();

    // Pull a tiny real image from Docker Hub
    let response = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state.clone(),
        deps,
    )
    .await;

    match response {
        DaemonResponse::Success { message } => {
            assert!(message.contains("pulled"));
            assert!(message.contains("alpine"));
        }
        _ => panic!("expected Success, got {:?}", response),
    }

    // Verify image is cached
    assert!(state.image_store.has_image("library/alpine", "latest"));

    // Verify layers were extracted
    let layers = state
        .image_store
        .get_image_layers("library/alpine", "latest")
        .expect("failed to get layers");
    assert!(!layers.is_empty(), "image should have layers");
}

#[tokio::test]
#[ignore] // Requires root and network
async fn test_pull_nonexistent_image() {
    require_root();

    let (deps, state, _temp) = create_real_deps();

    let response = handler::handle_pull(
        "nonexistent-image-that-does-not-exist-12345".to_string(),
        Some("latest".to_string()),
        state,
        deps,
    )
    .await;

    match response {
        DaemonResponse::Error { message } => {
            // Should fail to pull
            assert!(message.contains("error") || message.contains("not found"));
        }
        _ => panic!("expected Error for nonexistent image, got {:?}", response),
    }
}

// ---------------------------------------------------------------------------
// Container Lifecycle Integration Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // Requires root
async fn test_run_simple_container() {
    require_root();
    require_cgroups_v2();

    let (deps, state, _temp) = create_real_deps();

    // Pre-pull image to speed up test
    let _ = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state.clone(),
        deps.clone(),
    )
    .await;

    // Run a simple echo command
    let response = handler::handle_run(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/echo".to_string(), "hello from container".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps,
    )
    .await;

    let container_id = match response {
        DaemonResponse::ContainerCreated { id } => id,
        _ => panic!("expected ContainerCreated, got {:?}", response),
    };

    // Wait for container to spawn
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Container should be in state
    let container = state.get_container(&container_id).await;
    assert!(container.is_some(), "container should exist in state");
}

#[tokio::test]
#[ignore] // Requires root
async fn test_run_container_with_resource_limits() {
    require_root();
    require_cgroups_v2();

    let (deps, state, _temp) = create_real_deps();

    // Pre-pull image
    let _ = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state.clone(),
        deps.clone(),
    )
    .await;

    // Run with strict resource limits
    let response = handler::handle_run(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "sleep 1".to_string(),
        ],
        Some(128 * 1024 * 1024), // 128MB memory limit
        Some(250),               // CPU weight 250 (quarter of default)
        false,
        state.clone(),
        deps,
    )
    .await;

    let container_id = match response {
        DaemonResponse::ContainerCreated { id } => id,
        _ => panic!("expected ContainerCreated, got {:?}", response),
    };

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify cgroup was created with limits
    let cgroup_path = format!("/sys/fs/cgroup/minibox/{}", container_id);

    // Check memory limit
    let memory_max = std::fs::read_to_string(format!("{}/memory.max", cgroup_path))
        .expect("failed to read memory.max");
    assert_eq!(memory_max.trim(), "134217728"); // 128MB in bytes

    // Check CPU weight
    let cpu_weight = std::fs::read_to_string(format!("{}/cpu.weight", cgroup_path))
        .expect("failed to read cpu.weight");
    assert_eq!(cpu_weight.trim(), "250");
}

#[tokio::test]
#[ignore] // Requires root
async fn test_container_removal_cleanup() {
    require_root();
    require_cgroups_v2();

    let (deps, state, _temp) = create_real_deps();

    // Pre-pull image
    let _ = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state.clone(),
        deps.clone(),
    )
    .await;

    // Create and run container
    let response = handler::handle_run(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec!["/bin/true".to_string()],
        None,
        None,
        false,
        state.clone(),
        deps.clone(),
    )
    .await;

    let container_id = match response {
        DaemonResponse::ContainerCreated { id } => id,
        _ => panic!("expected ContainerCreated, got {:?}", response),
    };

    // Wait for container to exit
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Mark as stopped (normally done by reaper)
    state.update_container_state(&container_id, "Stopped").await;

    // Remove container
    let remove_response = handler::handle_remove(container_id.clone(), state.clone(), deps).await;

    assert!(matches!(remove_response, DaemonResponse::Success { .. }));

    // Verify cleanup
    let container = state.get_container(&container_id).await;
    assert!(
        container.is_none(),
        "container should be removed from state"
    );

    // Verify cgroup was cleaned up
    let cgroup_path = format!("/sys/fs/cgroup/minibox/{}", container_id);
    assert!(
        !std::path::Path::new(&cgroup_path).exists(),
        "cgroup should be removed"
    );
}

// ---------------------------------------------------------------------------
// Filesystem Integration Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // Requires root
async fn test_overlay_filesystem_setup() {
    require_root();

    let (deps, state, _temp) = create_real_deps();

    // Pull alpine to get real layers
    let _ = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state.clone(),
        deps.clone(),
    )
    .await;

    // Get layer paths
    let layers = state
        .image_store
        .get_image_layers("library/alpine", "latest")
        .expect("failed to get layers");

    // Setup overlay filesystem
    let container_dir = _temp.path().join("test-container");
    std::fs::create_dir(&container_dir).expect("failed to create container dir");

    let rootfs = deps
        .filesystem
        .setup_rootfs(&layers, &container_dir)
        .expect("failed to setup rootfs");

    // Verify merged directory exists and contains expected files
    assert!(rootfs.exists(), "rootfs should exist");
    assert!(
        rootfs.join("bin").exists(),
        "bin directory should exist in rootfs"
    );
    assert!(
        rootfs.join("etc").exists(),
        "etc directory should exist in rootfs"
    );

    // Cleanup
    deps.filesystem
        .cleanup(&container_dir)
        .expect("failed to cleanup filesystem");
}

// ---------------------------------------------------------------------------
// End-to-End Integration Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // Requires root and network
async fn test_complete_container_lifecycle() {
    require_root();
    require_cgroups_v2();

    let (deps, state, _temp) = create_real_deps();

    // 1. Pull image
    let pull_response = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state.clone(),
        deps.clone(),
    )
    .await;
    assert!(matches!(pull_response, DaemonResponse::Success { .. }));

    // 2. Verify image is cached
    assert!(state.image_store.has_image("library/alpine", "latest"));

    // 3. Run container with resource limits
    let run_response = handler::handle_run(
        "alpine".to_string(),
        Some("latest".to_string()),
        vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "echo test && sleep 1".to_string(),
        ],
        Some(256 * 1024 * 1024), // 256MB
        Some(500),               // CPU weight 500
        false,
        state.clone(),
        deps.clone(),
    )
    .await;

    let container_id = match run_response {
        DaemonResponse::ContainerCreated { id } => id,
        _ => panic!("expected ContainerCreated, got {:?}", run_response),
    };

    // 4. List containers
    tokio::time::sleep(Duration::from_millis(200)).await;
    let list_response = handler::handle_list(state.clone()).await;
    match list_response {
        DaemonResponse::ContainerList { containers } => {
            assert_eq!(containers.len(), 1);
            assert_eq!(containers[0].id, container_id);
            assert_eq!(containers[0].image, "alpine:latest");
        }
        _ => panic!("expected ContainerList, got {:?}", list_response),
    }

    // 5. Wait for container to exit
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // 6. Mark as stopped and remove
    state.update_container_state(&container_id, "Stopped").await;
    let remove_response = handler::handle_remove(container_id.clone(), state.clone(), deps).await;
    assert!(matches!(remove_response, DaemonResponse::Success { .. }));

    // 7. Verify complete cleanup
    assert!(state.get_container(&container_id).await.is_none());
}

// ---------------------------------------------------------------------------
// Performance and Stress Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // Requires root, long running
async fn test_multiple_concurrent_containers() {
    require_root();
    require_cgroups_v2();

    let (deps, state, _temp) = create_real_deps();

    // Pre-pull image once
    let _ = handler::handle_pull(
        "alpine".to_string(),
        Some("latest".to_string()),
        state.clone(),
        deps.clone(),
    )
    .await;

    // Spawn 5 containers concurrently
    let mut tasks = vec![];
    for i in 0..5 {
        let state_clone = state.clone();
        let deps_clone = deps.clone();

        let task = tokio::spawn(async move {
            handler::handle_run(
                "alpine".to_string(),
                Some("latest".to_string()),
                vec![
                    "/bin/sh".to_string(),
                    "-c".to_string(),
                    format!("echo container {} && sleep 1", i),
                ],
                Some(64 * 1024 * 1024), // 64MB per container
                None,
                false,
                state_clone,
                deps_clone,
            )
            .await
        });

        tasks.push(task);
    }

    // Wait for all spawns to complete
    let mut container_ids = vec![];
    for task in tasks {
        let response = task.await.expect("task panicked");
        match response {
            DaemonResponse::ContainerCreated { id } => {
                container_ids.push(id);
            }
            _ => panic!("expected ContainerCreated"),
        }
    }

    assert_eq!(container_ids.len(), 5, "should have created 5 containers");

    // Verify all containers exist
    tokio::time::sleep(Duration::from_millis(500)).await;
    let list_response = handler::handle_list(state.clone()).await;
    match list_response {
        DaemonResponse::ContainerList { containers } => {
            assert_eq!(containers.len(), 5, "should list 5 containers");
        }
        _ => panic!("expected ContainerList"),
    }

    // Cleanup all containers
    tokio::time::sleep(Duration::from_millis(1500)).await;
    for id in container_ids {
        state.update_container_state(&id, "Stopped").await;
        let _ = handler::handle_remove(id, state.clone(), deps.clone()).await;
    }
}
