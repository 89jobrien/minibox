//! Conformance tests for the SmolVM adapter suite against the BackendDescriptor
//! framework.
//!
//! These tests validate that the SmolVM adapters (SmolVmRegistry, SmolVmRuntime,
//! SmolVmFilesystem, SmolVmLimiter) correctly declare and implement capabilities
//! through the domain trait interface.
//!
//! SmolVM boots lightweight Linux VMs in <1s via Apple Virtualization.framework.
//! These tests use injected executors and do NOT require a running smolvm
//! instance.

use mbx::adapters::SmolVmRegistry;
use mbx::domain::{ContainerRuntime, ImageRegistry};
use minibox_core::adapters::conformance::BackendDescriptor;
use minibox_core::domain::BackendCapability;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helper: smolvm backend descriptor
// ---------------------------------------------------------------------------

/// Build a SmolVM backend descriptor for testing.
fn smolvm_backend_descriptor() -> BackendDescriptor {
    BackendDescriptor::new("smolvm")
}

// ---------------------------------------------------------------------------
// Conformance tests
// ---------------------------------------------------------------------------

/// SmolVM backend declares no commit/build/push capabilities. Image pulling
/// is handled via the ImageRegistry trait, not BackendCapability flags.
#[test]
fn smolvm_backend_declares_expected_capabilities() {
    let backend = smolvm_backend_descriptor();

    assert_eq!(backend.name, "smolvm", "backend name must be 'smolvm'");
    assert!(
        !backend.capabilities.supports(BackendCapability::Commit),
        "SmolVM does not support Commit (no overlay upperdir exposed)"
    );
    assert!(
        !backend
            .capabilities
            .supports(BackendCapability::BuildFromContext),
        "SmolVM does not support BuildFromContext"
    );
    assert!(
        !backend
            .capabilities
            .supports(BackendCapability::PushToRegistry),
        "SmolVM does not support PushToRegistry"
    );
}

/// SmolVmRegistry.has_image returns true when the injected executor succeeds
/// (simulating `docker images` finding the image in the VM).
#[tokio::test]
async fn smolvm_registry_has_image_delegates_to_docker() {
    let registry = SmolVmRegistry::new().with_executor(Arc::new(|_args: &[&str]| {
        Ok("sha256:abc123def456\n".to_string())
    }));

    assert!(
        registry.has_image("alpine", "latest").await,
        "has_image must return true when executor returns non-empty output"
    );
}

/// SmolVmRegistry.pull_image propagates executor errors.
#[tokio::test]
async fn smolvm_registry_pull_failure_propagates() {
    let registry = SmolVmRegistry::new().with_executor(Arc::new(|args: &[&str]| {
        if args.contains(&"pull") {
            Err(anyhow::anyhow!("connection refused"))
        } else {
            Ok(String::new())
        }
    }));

    let result = registry
        .pull_image(&mbx::image::reference::ImageRef::parse("alpine:3.18").expect("parse"))
        .await;

    assert!(
        result.is_err(),
        "pull_image must return Err when executor fails"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("connection"),
        "error message should indicate the underlying cause, got: {err_msg}"
    );
}

/// SmolVM runtime reports cgroups v2, overlay FS, and network isolation as
/// supported (provided by the Linux kernel inside the VM).
#[test]
fn smolvm_runtime_capabilities() {
    use mbx::adapters::SmolVmRuntime;

    let runtime = SmolVmRuntime::new();
    let caps = runtime.capabilities();

    assert!(
        caps.supports_cgroups_v2,
        "SmolVM reports cgroups v2 support (inside VM)"
    );
    assert!(
        caps.supports_overlay_fs,
        "SmolVM reports overlay FS support (inside VM)"
    );
    assert!(
        caps.supports_network_isolation,
        "SmolVM reports network isolation (VM provides virtualised networking)"
    );
    assert!(
        !caps.supports_user_namespaces,
        "SmolVM does not claim user namespace support"
    );
}

/// Verify the descriptor name is exactly "smolvm" so the conformance report
/// can group results by backend name.
#[test]
fn smolvm_backend_descriptor_name_is_smolvm() {
    let backend = smolvm_backend_descriptor();
    assert_eq!(backend.name, "smolvm", "backend.name must equal 'smolvm'");
}

/// SmolVmFilesystem.setup_rootfs returns a no-op layout (delegation to VM).
#[test]
fn smolvm_filesystem_setup_rootfs_is_noop() {
    use mbx::adapters::SmolVmFilesystem;
    use minibox_core::domain::RootfsSetup;
    use std::path::PathBuf;

    let fs = SmolVmFilesystem::new();
    let dir = PathBuf::from("/tmp/smolvm-test-container");
    let layout = fs
        .setup_rootfs(&[], &dir)
        .expect("setup_rootfs should succeed as no-op");
    assert_eq!(
        layout.merged_dir, dir,
        "merged_dir should equal container_dir (placeholder)"
    );
    assert!(
        layout.rootfs_metadata.is_none(),
        "rootfs_metadata should be None for no-op adapter"
    );
}

/// SmolVmLimiter.create returns the container ID (delegation to VM).
#[test]
fn smolvm_limiter_create_returns_id() {
    use mbx::adapters::SmolVmLimiter;
    use mbx::domain::ResourceLimiter;
    use minibox_core::domain::ResourceConfig;

    let limiter = SmolVmLimiter::new();
    let id = limiter
        .create("smolvm-test-001", &ResourceConfig::default())
        .expect("create should succeed as no-op");
    assert_eq!(id, "smolvm-test-001");
}

/// SmolVmRegistry.has_image strips "library/" prefix for official images.
#[tokio::test]
async fn smolvm_registry_strips_library_prefix() {
    let registry = SmolVmRegistry::new().with_executor(Arc::new(|args: &[&str]| {
        // Verify the filter arg does NOT contain "library/"
        for arg in args {
            if arg.starts_with("reference=") {
                assert!(
                    !arg.contains("library/"),
                    "library/ prefix should be stripped, got: {arg}"
                );
            }
        }
        Ok("sha256:abc\n".to_string())
    }));

    assert!(registry.has_image("library/alpine", "latest").await);
}
