//! Integration tests for the GKE adapter suite (minibox-31).
//!
//! These tests exercise the unprivileged GKE adapters (`NoopLimiter`,
//! `CopyFilesystem`, `ProotRuntime`) through their domain trait interfaces.
//! Tests do NOT require root or cgroups — all tests run on any platform
//! including macOS.

use minibox::adapters::{CopyFilesystem, NoopLimiter, ProotRuntime};
use minibox::domain::{ResourceConfig, ResourceLimiter, RootfsSetup};
use std::path::PathBuf;
use std::sync::Mutex;
use tempfile::TempDir;

// ============================================================================
// NoopLimiter Tests
// ============================================================================

#[test]
fn noop_limiter_create_returns_sentinel_path() {
    let limiter = NoopLimiter::new();
    let config = ResourceConfig::default();
    let path = limiter
        .create("test-container", &config)
        .expect("unwrap in test");
    assert_eq!(
        path, "noop:test-container",
        "create should return sentinel path format"
    );
}

#[test]
fn noop_limiter_create_with_various_ids() {
    let limiter = NoopLimiter::new();
    let config = ResourceConfig::default();

    for id in &["a", "container-123", "my-app-prod", "abc123def456"] {
        let path = limiter.create(id, &config).expect("unwrap in test");
        assert_eq!(
            path,
            format!("noop:{}", id),
            "sentinel path should match container ID"
        );
    }
}

#[test]
fn noop_limiter_add_process_succeeds() {
    let limiter = NoopLimiter::new();
    assert!(
        limiter.add_process("test-container", 1234).is_ok(),
        "add_process should succeed for any container and PID"
    );
}

#[test]
fn noop_limiter_add_process_various_pids() {
    let limiter = NoopLimiter::new();

    for pid in &[1u32, 100, 1234, u32::MAX - 1] {
        assert!(
            limiter.add_process("container", *pid).is_ok(),
            "add_process should accept any valid PID"
        );
    }
}

#[test]
fn noop_limiter_cleanup_succeeds() {
    let limiter = NoopLimiter::new();
    assert!(
        limiter.cleanup("test-container").is_ok(),
        "cleanup should succeed for any container"
    );
}

#[test]
fn noop_limiter_is_copy() {
    let limiter1 = NoopLimiter::new();
    let limiter2 = limiter1;
    let limiter3 = limiter1;

    let config = ResourceConfig::default();
    assert!(limiter1.create("c1", &config).is_ok());
    assert!(limiter2.create("c2", &config).is_ok());
    assert!(limiter3.create("c3", &config).is_ok());
}

// ============================================================================
// CopyFilesystem Tests
// ============================================================================

#[test]
fn copy_filesystem_merges_single_layer() {
    let dir = TempDir::new().expect("unwrap in test");

    // Create a single layer with bin/sh
    let layer = dir.path().join("layer");
    std::fs::create_dir_all(layer.join("bin")).expect("unwrap in test");
    std::fs::write(layer.join("bin/sh"), "#!/bin/sh\n").expect("unwrap in test");

    let container_dir = dir.path().join("container");
    let fs = CopyFilesystem::new();
    let merged = fs
        .setup_rootfs(std::slice::from_ref(&layer), &container_dir)
        .expect("unwrap in test");

    assert!(
        merged.merged_dir.join("bin/sh").exists(),
        "merged should contain bin/sh from layer"
    );
    let content =
        std::fs::read_to_string(merged.merged_dir.join("bin/sh")).expect("unwrap in test");
    assert_eq!(content, "#!/bin/sh\n");
}

#[test]
fn copy_filesystem_later_layer_overwrites_earlier() {
    let dir = TempDir::new().expect("unwrap in test");

    // Create layer0 with etc/os-release
    let layer0 = dir.path().join("layer0");
    std::fs::create_dir_all(layer0.join("etc")).expect("unwrap in test");
    std::fs::write(layer0.join("etc/os-release"), "layer0-content").expect("unwrap in test");

    // Create layer1 with the same file, different content
    let layer1 = dir.path().join("layer1");
    std::fs::create_dir_all(layer1.join("etc")).expect("unwrap in test");
    std::fs::write(layer1.join("etc/os-release"), "layer1-content").expect("unwrap in test");

    let container_dir = dir.path().join("container");
    let fs = CopyFilesystem::new();
    let merged = fs
        .setup_rootfs(&[layer0, layer1], &container_dir)
        .expect("unwrap in test");

    // layer1 (later) should overwrite layer0 (earlier)
    let content =
        std::fs::read_to_string(merged.merged_dir.join("etc/os-release")).expect("unwrap in test");
    assert_eq!(
        content, "layer1-content",
        "later layer should overwrite earlier layer"
    );
}

#[test]
fn copy_filesystem_empty_layers_creates_empty_merged() {
    let dir = TempDir::new().expect("unwrap in test");
    let container_dir = dir.path().join("container");

    let fs = CopyFilesystem::new();
    let merged = fs
        .setup_rootfs(&[], &container_dir)
        .expect("unwrap in test");

    assert!(
        merged.merged_dir.exists(),
        "empty layers should create merged directory"
    );
    assert!(
        merged.merged_dir.ends_with("merged"),
        "merged dir should have correct name"
    );
    // Merged dir should exist but be empty (no files to copy)
    assert!(
        std::fs::read_dir(&merged.merged_dir)
            .expect("unwrap in test")
            .next()
            .is_none(),
        "empty layers should result in empty merged directory"
    );
}

#[test]
fn copy_filesystem_cleanup_removes_container_dir() {
    let dir = TempDir::new().expect("unwrap in test");
    let container_dir = dir.path().join("container");

    // Create some structure first
    std::fs::create_dir_all(container_dir.join("merged/etc")).expect("unwrap in test");
    std::fs::write(container_dir.join("merged/etc/test"), "data").expect("unwrap in test");

    assert!(
        container_dir.exists(),
        "container dir should exist before cleanup"
    );

    let fs = CopyFilesystem::new();
    fs.cleanup(&container_dir).expect("unwrap in test");

    assert!(
        !container_dir.exists(),
        "cleanup should remove container directory"
    );
}

#[cfg(unix)]
#[test]
fn copy_filesystem_preserves_symlinks() {
    let dir = TempDir::new().expect("unwrap in test");

    // Create a layer with a symlink
    let layer = dir.path().join("layer");
    std::fs::create_dir_all(layer.join("bin")).expect("unwrap in test");
    std::fs::write(layer.join("bin/busybox"), "fake-busybox-binary").expect("unwrap in test");

    // Create a relative symlink sh -> busybox
    std::os::unix::fs::symlink("busybox", layer.join("bin/sh")).expect("unwrap in test");

    let container_dir = dir.path().join("container");
    let fs = CopyFilesystem::new();
    let merged = fs
        .setup_rootfs(&[layer], &container_dir)
        .expect("unwrap in test");

    // Verify symlink was copied and preserved
    let link_target = std::fs::read_link(merged.merged_dir.join("bin/sh")).expect("unwrap in test");
    assert_eq!(
        link_target,
        PathBuf::from("busybox"),
        "relative symlink should be preserved"
    );
}

#[test]
fn copy_filesystem_handles_directory_permissions() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().expect("unwrap in test");

        // Create a layer with a directory with specific permissions
        let layer = dir.path().join("layer");
        let app_dir = layer.join("app");
        std::fs::create_dir_all(&app_dir).expect("unwrap in test");
        std::fs::set_permissions(&app_dir, std::fs::Permissions::from_mode(0o755))
            .expect("unwrap in test");

        let container_dir = dir.path().join("container");
        let fs = CopyFilesystem::new();
        let merged = fs
            .setup_rootfs(&[layer], &container_dir)
            .expect("unwrap in test");

        let mode = std::fs::metadata(merged.merged_dir.join("app"))
            .expect("unwrap in test")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o755, "directory permissions should be preserved");
    }

    #[cfg(not(unix))]
    {
        // On non-Unix platforms, just verify the directory exists
        let dir = TempDir::new().expect("unwrap in test");
        let layer = dir.path().join("layer");
        std::fs::create_dir_all(&layer.join("app")).expect("unwrap in test");

        let container_dir = dir.path().join("container");
        let fs = CopyFilesystem::new();
        let merged = fs
            .setup_rootfs(&[layer], &container_dir)
            .expect("unwrap in test");

        assert!(merged.merged_dir.join("app").is_dir());
    }
}

// ============================================================================
// ProotRuntime Tests
// ============================================================================

#[test]
fn proot_runtime_new_rejects_nonexistent_binary() {
    let result = ProotRuntime::new("/nonexistent/proot/binary");
    assert!(
        result.is_err(),
        "new() should reject nonexistent proot binary"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("proot binary not found") || err_msg.contains("not found"),
        "error message should indicate proot not found, got: {err_msg}"
    );
}

#[test]
fn proot_runtime_new_accepts_existing_binary() {
    // Use /bin/sh as a stand-in for proot (it exists on Unix)
    #[cfg(unix)]
    {
        let runtime = ProotRuntime::new("/bin/sh");
        assert!(runtime.is_ok(), "new() should accept existing binary");
    }
}

#[test]
fn proot_runtime_from_env_uses_env_var() {
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    let _guard = ENV_MUTEX.lock().expect("unwrap in test");

    // SAFETY: serialized by ENV_MUTEX; no other thread reads MINIBOX_PROOT_PATH concurrently.
    unsafe { std::env::set_var("MINIBOX_PROOT_PATH", "/bin/sh") };

    let result = ProotRuntime::from_env();

    // SAFETY: same mutex guard; restoring env before any other test runs.
    unsafe { std::env::remove_var("MINIBOX_PROOT_PATH") };

    assert!(
        result.is_ok(),
        "from_env() should use MINIBOX_PROOT_PATH when set"
    );
}

#[test]
fn proot_runtime_from_env_rejects_nonexistent_path_in_env() {
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    let _guard = ENV_MUTEX.lock().expect("unwrap in test");

    // SAFETY: serialized by ENV_MUTEX; no other thread reads MINIBOX_PROOT_PATH concurrently.
    unsafe { std::env::set_var("MINIBOX_PROOT_PATH", "/nonexistent/proot") };

    let result = ProotRuntime::from_env();

    // SAFETY: same mutex guard; restoring env before any other test runs.
    unsafe { std::env::remove_var("MINIBOX_PROOT_PATH") };

    assert!(
        result.is_err(),
        "from_env() should fail when MINIBOX_PROOT_PATH points to nonexistent binary"
    );
}

// ============================================================================
// Adapter Isolation Tests
// ============================================================================

/// Verify that adapters can be used simultaneously without interfering
/// with each other or with system state.
#[test]
fn adapters_can_be_used_simultaneously() {
    let dir = TempDir::new().expect("unwrap in test");

    let limiter1 = NoopLimiter::new();
    let limiter2 = NoopLimiter::new();

    let layer1 = dir.path().join("layer1");
    std::fs::create_dir_all(layer1.join("bin")).expect("unwrap in test");
    std::fs::write(layer1.join("bin/sh"), "fake").expect("unwrap in test");

    let container_dir1 = dir.path().join("container1");
    let container_dir2 = dir.path().join("container2");

    let fs1 = CopyFilesystem::new();
    let fs2 = CopyFilesystem::new();

    let config = ResourceConfig::default();

    // All adapters should work in parallel
    let c1 = limiter1.create("c1", &config).expect("unwrap in test");
    let c2 = limiter2.create("c2", &config).expect("unwrap in test");
    let merged1 = fs1
        .setup_rootfs(std::slice::from_ref(&layer1), &container_dir1)
        .expect("unwrap in test");
    let merged2 = fs2
        .setup_rootfs(std::slice::from_ref(&layer1), &container_dir2)
        .expect("unwrap in test");

    assert_eq!(c1, "noop:c1");
    assert_eq!(c2, "noop:c2");
    assert!(merged1.merged_dir.exists());
    assert!(merged2.merged_dir.exists());

    // Both should be independent
    assert_ne!(
        merged1.merged_dir, merged2.merged_dir,
        "different container dirs should have different merged paths"
    );
}

#[test]
fn copy_filesystem_multiple_instances_independent() {
    let dir = TempDir::new().expect("unwrap in test");

    let fs1 = CopyFilesystem::new();
    let fs2 = CopyFilesystem::new();

    let layer = dir.path().join("layer");
    std::fs::create_dir_all(layer.join("etc")).expect("unwrap in test");
    std::fs::write(layer.join("etc/test"), "data").expect("unwrap in test");

    let c1 = dir.path().join("container1");
    let c2 = dir.path().join("container2");

    let m1 = fs1
        .setup_rootfs(std::slice::from_ref(&layer), &c1)
        .expect("unwrap in test");
    let m2 = fs2
        .setup_rootfs(std::slice::from_ref(&layer), &c2)
        .expect("unwrap in test");

    assert!(m1.merged_dir.exists());
    assert!(m2.merged_dir.exists());
    assert_ne!(m1.merged_dir, m2.merged_dir);

    // Clean up independently
    fs1.cleanup(&c1).expect("unwrap in test");
    assert!(!c1.exists());
    assert!(c2.exists());

    fs2.cleanup(&c2).expect("unwrap in test");
    assert!(!c2.exists());
}
