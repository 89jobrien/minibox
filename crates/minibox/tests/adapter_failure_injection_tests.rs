//! Adapter failure injection tests.
//!
//! Verifies error propagation and cleanup behavior when adapters
//! fail at different points in the container lifecycle.

use minibox::adapters::mocks::{
    FailableFilesystemMock, MockFilesystem, MockLimiter, MockRegistry, MockRuntime,
};
use minibox::domain::{
    ContainerHooks, ContainerRuntime, ContainerSpawnConfig, ImageRegistry, ResourceConfig,
    ResourceLimiter, RootfsSetup,
};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// 1. Registry failure propagation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_registry_pull_failure_returns_descriptive_error() {
    let registry = MockRegistry::new().with_pull_failure();
    let result = registry
        .pull_image(&minibox::image::reference::ImageRef::parse("alpine").unwrap())
        .await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("mock pull failure"),
        "error message must mention 'mock pull failure'"
    );
    assert_eq!(registry.pull_count(), 1, "pull was attempted once");
}

// ---------------------------------------------------------------------------
// 2. Filesystem setup failure + cleanup verification
// ---------------------------------------------------------------------------

#[test]
fn test_filesystem_setup_failure_doesnt_leave_partial_state() {
    let fs = MockFilesystem::new().with_setup_failure();
    let result = fs.setup_rootfs(&[], Path::new("/container"));
    assert!(result.is_err());
    assert_eq!(fs.setup_count(), 1);
    // Cleanup must succeed even after a failed setup
    assert!(
        fs.cleanup(Path::new("/container")).is_ok(),
        "cleanup must not panic or fail after a setup failure"
    );
    assert_eq!(fs.cleanup_count(), 1);
}

// ---------------------------------------------------------------------------
// 3. Failable mock toggle mid-lifecycle
// ---------------------------------------------------------------------------

#[test]
fn test_failable_filesystem_recovery_after_transient_failure() {
    let fs = FailableFilesystemMock::new();

    // First call succeeds
    assert!(
        fs.setup_rootfs(&[], Path::new("/c1")).is_ok(),
        "initial setup must succeed"
    );

    // Inject failure
    fs.set_fail_setup(true);
    assert!(
        fs.setup_rootfs(&[], Path::new("/c2")).is_err(),
        "setup must fail when injected"
    );

    // Recover
    fs.set_fail_setup(false);
    assert!(
        fs.setup_rootfs(&[], Path::new("/c3")).is_ok(),
        "setup must succeed after recovery"
    );

    assert_eq!(fs.setup_count(), 3, "all three attempts must be counted");
}

// ---------------------------------------------------------------------------
// 4. Limiter failure after filesystem setup (partial lifecycle failure)
// ---------------------------------------------------------------------------

#[test]
fn test_limiter_failure_after_successful_setup_requires_cleanup() {
    let fs = MockFilesystem::new();
    let limiter = MockLimiter::new().with_create_failure();

    // Filesystem setup succeeds
    let rootfs = fs.setup_rootfs(&[], Path::new("/container")).unwrap();
    assert!(
        rootfs.merged_dir.ends_with("merged"),
        "rootfs path must end with 'merged'"
    );

    // Limiter fails
    let result = limiter.create("test-container", &ResourceConfig::default());
    assert!(result.is_err(), "limiter create must fail");

    // Cleanup must still work on the filesystem despite the limiter failure
    assert!(
        fs.cleanup(Path::new("/container")).is_ok(),
        "filesystem cleanup must succeed after a limiter failure"
    );
    assert_eq!(fs.cleanup_count(), 1);
}

// ---------------------------------------------------------------------------
// 5. Runtime spawn failure after both setup and limiter succeed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_failure_after_successful_setup_and_limits() {
    let fs = MockFilesystem::new();
    let limiter = MockLimiter::new();
    let runtime = MockRuntime::new().with_spawn_failure();

    let rootfs = fs.setup_rootfs(&[], Path::new("/container")).unwrap();
    let cgroup_path = limiter
        .create("test-container", &ResourceConfig::default())
        .unwrap();

    let config = ContainerSpawnConfig {
        rootfs: rootfs.merged_dir,
        command: "/bin/sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "test".to_string(),
        cgroup_path: PathBuf::from(&cgroup_path),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
        mounts: vec![],    // placeholder — Task 6 replaces this
        privileged: false, // placeholder — Task 6 replaces this
        image_ref: None,
    };

    let result = runtime.spawn_process(&config).await;
    assert!(result.is_err(), "spawn must fail");
    assert_eq!(runtime.spawn_count(), 1, "spawn was attempted once");

    // Both adapter cleanups must succeed after the runtime failure
    assert!(
        limiter.cleanup("test-container").is_ok(),
        "limiter cleanup must succeed"
    );
    assert!(
        fs.cleanup(Path::new("/container")).is_ok(),
        "filesystem cleanup must succeed"
    );
    assert_eq!(limiter.cleanup_count(), 1);
    assert_eq!(fs.cleanup_count(), 1);
}

// ---------------------------------------------------------------------------
// 6. Cleanup failure is non-fatal (warn, don't crash)
// ---------------------------------------------------------------------------

#[test]
fn test_cleanup_failure_does_not_panic() {
    let fs = FailableFilesystemMock::new();
    fs.set_fail_cleanup(true);

    // Cleanup must return an error but must not panic
    let result = fs.cleanup(Path::new("/container"));
    assert!(result.is_err(), "cleanup must return error when injected");
    assert_eq!(fs.cleanup_count(), 1, "cleanup attempt must be counted");
}

// ---------------------------------------------------------------------------
// 7. Sequential containers: failure on one doesn't affect another
// ---------------------------------------------------------------------------

#[test]
fn test_independent_container_failures() {
    let fs = FailableFilesystemMock::new();

    // Container 1: success
    assert!(fs.setup_rootfs(&[], Path::new("/c1")).is_ok());
    assert!(fs.cleanup(Path::new("/c1")).is_ok());

    // Container 2: setup fails
    fs.set_fail_setup(true);
    assert!(
        fs.setup_rootfs(&[], Path::new("/c2")).is_err(),
        "c2 setup must fail"
    );

    // Container 3: success again after clearing failure
    fs.set_fail_setup(false);
    assert!(
        fs.setup_rootfs(&[], Path::new("/c3")).is_ok(),
        "c3 setup must succeed"
    );
    assert!(fs.cleanup(Path::new("/c3")).is_ok());

    assert_eq!(fs.setup_count(), 3, "three setup calls total");
    assert_eq!(fs.cleanup_count(), 2, "two successful cleanups");
}

// ---------------------------------------------------------------------------
// 8. Registry pull count tracks all attempts (success and failure)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_registry_tracks_all_pull_attempts() {
    let registry = MockRegistry::new();

    // First pull succeeds
    assert!(
        registry
            .pull_image(&minibox::image::reference::ImageRef::parse("alpine").unwrap())
            .await
            .is_ok()
    );
    assert_eq!(registry.pull_count(), 1);

    // Image is now cached
    assert!(
        registry.has_image("library/alpine", "latest").await,
        "image must be cached after pull"
    );

    // Second pull also succeeds (re-pull)
    assert!(
        registry
            .pull_image(&minibox::image::reference::ImageRef::parse("alpine").unwrap())
            .await
            .is_ok()
    );
    assert_eq!(
        registry.pull_count(),
        2,
        "both pull attempts must be counted"
    );
}
