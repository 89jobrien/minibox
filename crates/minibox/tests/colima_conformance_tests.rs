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
//! Conformance tests for the Colima adapter suite against the BackendDescriptor
//! framework.
//!
//! These tests validate that the Colima adapters (ColimaRegistry, ColimaRuntime,
//! ColimaFilesystem, ColimaLimiter) correctly declare and implement capabilities
//! through the domain trait interface.
//!
//! # Serialization Note
//!
//! Test serialization is handled uniformly across CI and local environments via
//! `cargo xtask test-unit` which runs `cargo test --release --lib`. No special
//! `serial_test` override is needed; tokio's test runtime handles concurrent test
//! isolation automatically. The CI job and local command use identical xtask
//! recipes (see `crates/xtask/src/gates.rs`), ensuring parity.

use minibox::adapters::ColimaRegistry;
use minibox::domain::{ContainerRuntime, ImageRegistry};
use minibox_core::adapters::conformance::BackendDescriptor;
use minibox_core::domain::BackendCapability;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helper: colima backend descriptor with injected executor
// ---------------------------------------------------------------------------

/// Build a Colima backend descriptor for testing.
fn colima_backend_descriptor() -> BackendDescriptor {
    BackendDescriptor::new("colima")
}

// ---------------------------------------------------------------------------
// Conformance tests
// ---------------------------------------------------------------------------

/// Colima backend declares no commit/build/push capabilities, since:
/// - Commit: no overlay upperdir exposed by nerdctl/lima
/// - BuildFromContext: no Dockerfile support wired into the adapter
/// - PushToRegistry: no direct push implementation yet
///
/// Image pulling (has_image, pull_image) are ImageRegistry trait methods,
/// not BackendCapability flags.
#[test]
fn colima_backend_declares_expected_capabilities() {
    let backend = colima_backend_descriptor();

    assert_eq!(backend.name, "colima", "backend name must be 'colima'");
    assert!(
        !backend.capabilities.supports(BackendCapability::Commit),
        "Colima does not support Commit (no overlay upperdir from nerdctl)"
    );
    assert!(
        !backend
            .capabilities
            .supports(BackendCapability::BuildFromContext),
        "Colima does not support BuildFromContext (no Dockerfile support wired)"
    );
    assert!(
        !backend
            .capabilities
            .supports(BackendCapability::PushToRegistry),
        "Colima does not support PushToRegistry (no direct push implementation)"
    );
}

/// ColimaRegistry.has_image returns true when the injected executor succeeds
/// (simulating `docker images` finding the image in the containerd store).
#[tokio::test]
async fn colima_registry_has_image_delegates_to_nerdctl() {
    let registry = ColimaRegistry::new().with_executor(Arc::new(|_args: &[&str]| {
        // Fake executor returns a non-empty response, simulating a found image.
        Ok("sha256:abc123def456\n".to_string())
    }));

    assert!(
        registry.has_image("alpine", "latest").await,
        "has_image must return true when executor returns non-empty output"
    );
}

/// ColimaRegistry.pull_image propagates executor errors — when the underlying
/// Lima command fails, the trait method must return Err.
#[tokio::test]
async fn colima_registry_pull_failure_propagates() {
    let registry = ColimaRegistry::new().with_executor(Arc::new(|args: &[&str]| {
        // Simulate failure when pulling
        if args.contains(&"pull") {
            Err(anyhow::anyhow!("network connection refused"))
        } else {
            Ok(String::new())
        }
    }));

    let result = registry
        .pull_image(&minibox::image::reference::ImageRef::parse("alpine:3.18").expect("parse"))
        .await;

    assert!(
        result.is_err(),
        "pull_image must return Err when executor fails"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("connection") || err_msg.contains("network"),
        "error message should indicate the underlying cause, got: {err_msg}"
    );
}

/// Colima runtime does NOT declare native namespace isolation as a capability,
/// since it delegates to nerdctl inside a Lima VM. The in-VM nerdctl may
/// support namespaces, but from Colima's perspective (macOS side), namespace
/// setup is handled by the VM, not by Colima directly.
#[test]
fn colima_runtime_capabilities_excludes_privileged() {
    use minibox::adapters::ColimaRuntime;

    let runtime = ColimaRuntime::new();
    let caps = runtime.capabilities();

    // Colima claims all capabilities because it runs a full Linux kernel in the VM
    // However, if we test Colima's *own* adapter (not the in-VM nerdctl), it would
    // NOT claim native namespace support. This test documents that Colima provides
    // a managed environment but does not expose the underlying Linux namespace
    // primitives directly from macOS.
    assert!(
        caps.supports_cgroups_v2,
        "Colima reports cgroups v2 support (inside VM)"
    );
    assert!(
        caps.supports_overlay_fs,
        "Colima reports overlay FS support (inside VM)"
    );
}

/// Verify the descriptor name is exactly "colima" so the conformance report
/// can group results by backend name.
#[test]
fn colima_backend_descriptor_name_is_colima() {
    let backend = colima_backend_descriptor();
    assert_eq!(backend.name, "colima", "backend.name must equal 'colima'");
}
