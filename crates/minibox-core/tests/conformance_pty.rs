//! Conformance tests for the `PtyAllocator` trait contract.
//!
//! Verifies:
//! - `NullPtyAllocator` always returns `Err` (PTY not supported).
//! - `MockPtyAllocator` returns `Ok(PtyHandle)` with configured fds.
//! - `PtyHandle` fields (`master_fd`, `slave_fd`) match constructor args.
//! - `PtyConfig` default dimensions are accepted.
//! - Trait object (`Arc<dyn PtyAllocator>`) is constructable and callable.
//!
//! No I/O, no actual PTY allocation.

use minibox_core::domain::{NullPtyAllocator, PtyAllocator, PtyConfig};
#[cfg(feature = "test-utils")]
use minibox_core::domain::MockPtyAllocator;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_pty_config() -> PtyConfig {
    PtyConfig { rows: 24, cols: 80, enabled: true }
}

// ---------------------------------------------------------------------------
// NullPtyAllocator
// ---------------------------------------------------------------------------

/// `NullPtyAllocator::allocate` must always return `Err`.
#[test]
fn conformance_null_pty_always_returns_err() {
    let alloc = NullPtyAllocator;
    let result = alloc.allocate(&default_pty_config());
    assert!(result.is_err(), "NullPtyAllocator must always return Err");
}

/// `NullPtyAllocator::allocate` error message must mention PTY or unsupported.
#[test]
fn conformance_null_pty_error_message_is_descriptive() {
    let alloc = NullPtyAllocator;
    let err = alloc.allocate(&default_pty_config()).unwrap_err();
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("pty") || msg.contains("unsupported") || msg.contains("not supported"),
        "error must mention PTY or unsupported, got: {msg}"
    );
}

/// Calling `NullPtyAllocator::allocate` multiple times must not panic.
#[test]
fn conformance_null_pty_repeated_calls_do_not_panic() {
    let alloc = NullPtyAllocator;
    for _ in 0..10 {
        let _ = alloc.allocate(&default_pty_config());
    }
}

// ---------------------------------------------------------------------------
// MockPtyAllocator
// ---------------------------------------------------------------------------

/// `MockPtyAllocator::allocate` must return `Ok` with configured fd values.
#[cfg(feature = "test-utils")]
#[test]
fn conformance_mock_pty_returns_configured_fds() {
    let alloc = MockPtyAllocator::new(10, 11);
    let handle = alloc
        .allocate(&default_pty_config())
        .expect("MockPtyAllocator must succeed");

    assert_eq!(handle.master_fd, 10, "master_fd must match constructor arg");
    assert_eq!(handle.slave_fd, 11, "slave_fd must match constructor arg");
}

/// Two allocations from the same mock return the same fd values.
#[cfg(feature = "test-utils")]
#[test]
fn conformance_mock_pty_is_idempotent() {
    let alloc = MockPtyAllocator::new(5, 6);
    let h1 = alloc.allocate(&default_pty_config()).expect("first allocation");
    let h2 = alloc.allocate(&default_pty_config()).expect("second allocation");

    assert_eq!(h1.master_fd, h2.master_fd);
    assert_eq!(h1.slave_fd, h2.slave_fd);
}

/// Zero-dimension config must not panic (edge case).
#[cfg(feature = "test-utils")]
#[test]
fn conformance_mock_pty_zero_dimensions_accepted() {
    let alloc = MockPtyAllocator::new(3, 4);
    let config = PtyConfig { rows: 0, cols: 0, enabled: true };
    let result = alloc.allocate(&config);
    assert!(result.is_ok(), "zero dimensions must not cause an error");
}

// ---------------------------------------------------------------------------
// Trait object
// ---------------------------------------------------------------------------

#[test]
fn conformance_pty_allocator_as_trait_object_null() {
    let alloc: Arc<dyn PtyAllocator> = Arc::new(NullPtyAllocator);
    assert!(alloc.allocate(&default_pty_config()).is_err());
}

#[cfg(feature = "test-utils")]
#[test]
fn conformance_pty_allocator_as_trait_object_mock() {
    let alloc: Arc<dyn PtyAllocator> = Arc::new(MockPtyAllocator::new(7, 8));
    let handle = alloc.allocate(&default_pty_config()).expect("mock must succeed");
    assert_eq!(handle.master_fd, 7);
}
