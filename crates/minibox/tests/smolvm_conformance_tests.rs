#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::unwrap_in_result,
    clippy::collapsible_if,
    clippy::match_same_arms,
    clippy::only_used_in_recursion,
    clippy::used_underscore_binding,
    clippy::map_unwrap_or,
    clippy::manual_assert,
    clippy::as_ptr_cast_mut,
    clippy::ptr_as_ptr,
    clippy::must_use_candidate,
    clippy::used_underscore_items,
    clippy::missing_const_for_fn,
    clippy::manual_string_new,
    clippy::semicolon_if_nothing_returned,
    clippy::unreadable_literal,
    clippy::default_constructed_unit_structs,
    clippy::ref_as_ptr,
    clippy::allow_attributes_without_reason,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_raw_string_hashes,
    clippy::manual_is_variant_and,
    clippy::ignore_without_reason,
    clippy::default_trait_access,
    clippy::cast_lossless,
    clippy::match_wild_err_arm,
    clippy::format_push_string,
    clippy::bool_assert_comparison,
    clippy::struct_excessive_bools
)]
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

use minibox::adapters::SmolVmRegistry;
use minibox::domain::{ContainerRuntime, ImageLoader, ImageRegistry};
use minibox_core::adapters::conformance::BackendDescriptor;
use minibox_core::domain::BackendCapability;
use minibox_core::image::ImageStore;
use std::sync::Arc;

/// Build a registry backed by a throwaway temp-dir image store, for tests
/// that don't care about the specific store location.
fn test_registry() -> SmolVmRegistry {
    let tmp = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(ImageStore::new(tmp.path().join("images")).expect("ImageStore::new"));
    std::mem::forget(tmp); // keep tempdir alive for the registry's lifetime
    SmolVmRegistry::new(store).expect("SmolVmRegistry::new")
}

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

/// SmolVmRegistry.has_image now checks the host-side image store (which
/// persists across ephemeral VM instances) rather than asking the guest via
/// an injected executor — see the doc comment on `SmolVmRegistry` for why
/// the pull path moved off the VM.
#[tokio::test]
async fn smolvm_registry_has_image_checks_host_store() {
    let registry = test_registry();

    assert!(
        !registry.has_image("alpine", "latest").await,
        "has_image must return false for an image never pulled into the host store"
    );
}

// Note: pull_image's happy/error paths now depend on the real host-side
// DockerHubRegistry (network I/O), which has no executor injection point —
// unlike the old VM-delegated implementation, its failure modes aren't
// unit-testable here. Coverage for the host pull itself lives with
// DockerHubRegistry's own tests; this suite still covers load_image below,
// which is what remains VM-executor-driven.

/// SmolVmRegistry.load_image imports the host tarball into the VM-local Docker
/// image cache and tags it with the requested `name:tag`.
#[tokio::test]
async fn smolvm_registry_load_image_imports_tarball_into_vm_cache() {
    let tmp = tempfile::TempDir::new().expect("create temp dir");
    let tarball = tmp.path().join("image.tar");
    std::fs::write(&tarball, b"fake tarball bytes").expect("write tarball");

    let calls = Arc::new(std::sync::Mutex::new(Vec::<Vec<String>>::new()));
    let captured = Arc::clone(&calls);
    let registry = test_registry().with_executor(Arc::new(move |args: &[&str]| {
        captured
            .lock()
            .expect("calls lock")
            .push(args.iter().map(|arg| (*arg).to_string()).collect());
        Ok("Loaded image: source/name:oldtag\n".to_string())
    }));

    registry
        .load_image(&tarball, "library/foo", "latest")
        .await
        .expect("load image into smolvm");

    let calls = calls.lock().expect("calls lock");
    let flattened = calls
        .iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        flattened.contains("docker load"),
        "smolvm load must run docker load inside the VM, got: {flattened}"
    );
    assert!(
        flattened.contains("docker tag"),
        "smolvm load must tag the loaded image for mbx run, got: {flattened}"
    );
    // The `library/` namespace prefix is stripped so the tag matches what
    // `has_image`/`pull_image` look for later (issue #457 regression coverage).
    assert!(
        flattened.contains("foo:latest") && !flattened.contains("library/foo:latest"),
        "smolvm load must tag with the library/ prefix stripped, got: {flattened}"
    );
}

/// Missing local tarballs must fail before invoking smolvm.
#[tokio::test]
async fn smolvm_registry_load_image_rejects_missing_tarball() {
    let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let called_by_exec = Arc::clone(&called);
    let registry = test_registry().with_executor(Arc::new(move |_args: &[&str]| {
        called_by_exec.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(String::new())
    }));

    let result = registry
        .load_image(
            std::path::Path::new("/definitely/missing/minibox-image.tar"),
            "library/foo",
            "latest",
        )
        .await;

    assert!(result.is_err(), "missing tarball must return an error");
    assert!(
        !called.load(std::sync::atomic::Ordering::Relaxed),
        "missing tarball must not invoke smolvm"
    );
}

/// SmolVM runtime reports cgroups v2, overlay FS, and network isolation as
/// supported (provided by the Linux kernel inside the VM).
#[test]
fn smolvm_runtime_capabilities() {
    use minibox::adapters::SmolVmRuntime;

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
    use minibox::adapters::SmolVmFilesystem;
    use minibox_core::domain::RootfsSetup;
    use std::path::PathBuf;

    let fs = SmolVmFilesystem::new();
    let dir = PathBuf::from("/tmp/smolvm-test-container");
    let layout = fs
        .setup_rootfs(&[], &dir)
        .expect("setup_rootfs should succeed as no-op");
    assert_eq!(
        &*layout.merged_dir,
        dir.as_path(),
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
    use minibox::adapters::SmolVmLimiter;
    use minibox::domain::ResourceLimiter;
    use minibox_core::domain::ResourceConfig;

    let limiter = SmolVmLimiter::new();
    let id = limiter
        .create("smolvm-test-001", &ResourceConfig::default())
        .expect("create should succeed as no-op");
    assert_eq!(id, "smolvm-test-001");
}

// Note: the "library/" prefix stripping guarantee for locally-loaded images
// is covered by `target_ref_strips_library_prefix` (unit test in smolvm.rs)
// and by `smolvm_registry_load_image_imports_tarball_into_vm_cache` above —
// has_image itself no longer talks to the VM, so there's nothing
// docker-argument-shaped left to assert on here.
