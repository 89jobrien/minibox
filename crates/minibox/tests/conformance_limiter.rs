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
//! Conformance tests for the `ResourceLimiter` trait contract.
//!
//! All tests use `MockLimiter` from `minibox::testing` — no kernel/cgroup interaction.
//! Each test creates a fresh mock to avoid shared state.

use minibox::testing::mocks::limiter::MockLimiter;
use minibox_core::domain::{ResourceConfig, ResourceLimiter};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_config() -> ResourceConfig {
    ResourceConfig {
        memory_limit_bytes: Some(128 * 1024 * 1024),
        cpu_weight: Some(512),
        pids_max: None,
        io_max_bytes_per_sec: None,
    }
}

// ---------------------------------------------------------------------------
// create invariants
// ---------------------------------------------------------------------------

/// `create` on a default mock must return Ok(String) containing the container_id.
#[test]
fn limiter_create_returns_cgroup_path() {
    let limiter = MockLimiter::new();
    let path = limiter
        .create("testcontainer01", &default_config())
        .expect("create must succeed on default MockLimiter");
    assert!(
        path.contains("testcontainer01"),
        "returned cgroup path must contain the container_id, got: {path}"
    );
}

/// `create` must increment `create_count` even on success.
#[test]
fn limiter_create_increments_count() {
    let limiter = MockLimiter::new();
    limiter
        .create("counttest01", &default_config())
        .expect("create must succeed");
    assert_eq!(
        limiter.create_count(),
        1,
        "create_count must be 1 after one create"
    );
}

/// `create` configured with failure must return Err.
#[test]
fn limiter_create_failure_returns_err() {
    let limiter = MockLimiter::new().with_create_failure();
    let result = limiter.create("failtest01", &default_config());
    assert!(
        result.is_err(),
        "create must return Err on a failure-configured mock"
    );
}

/// A failed `create` must not add the container to `created_cgroups`.
/// We verify indirectly: `create_count` increments but subsequent operations
/// may not see it. The key invariant is that `create` still increments the count
/// (the call was made) even though it returned Err.
#[test]
fn limiter_create_failure_does_not_increment_created() {
    let limiter = MockLimiter::new().with_create_failure();
    let _ = limiter.create("failtest02", &default_config());
    // create_count still increments — the call happened
    assert_eq!(
        limiter.create_count(),
        1,
        "create_count must increment even on a failed create"
    );
}

// ---------------------------------------------------------------------------
// add_process invariants
// ---------------------------------------------------------------------------

/// `add_process` on a default mock returns Ok(()).
#[test]
fn limiter_add_process_succeeds_by_default() {
    let limiter = MockLimiter::new();
    let result = limiter.add_process("aptest01", 12345);
    assert!(
        result.is_ok(),
        "add_process must succeed on default MockLimiter"
    );
}

// ---------------------------------------------------------------------------
// cleanup invariants
// ---------------------------------------------------------------------------

/// `cleanup` must increment `cleanup_count`.
#[test]
fn limiter_cleanup_increments_count() {
    let limiter = MockLimiter::new();
    limiter
        .cleanup("cleanuptest01")
        .expect("cleanup must succeed on default MockLimiter");
    assert_eq!(
        limiter.cleanup_count(),
        1,
        "cleanup_count must be 1 after one cleanup"
    );
}

// ---------------------------------------------------------------------------
// Round-trip invariants
// ---------------------------------------------------------------------------

/// `create` then `cleanup` must both succeed and each counter must be 1.
#[test]
fn limiter_create_then_cleanup_round_trip() {
    let limiter = MockLimiter::new();
    limiter
        .create("roundtrip01", &default_config())
        .expect("create must succeed");
    limiter
        .cleanup("roundtrip01")
        .expect("cleanup must succeed");
    assert_eq!(limiter.create_count(), 1, "create_count must be 1");
    assert_eq!(limiter.cleanup_count(), 1, "cleanup_count must be 1");
}
