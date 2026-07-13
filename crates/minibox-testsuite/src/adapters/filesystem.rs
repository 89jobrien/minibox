//! Conformance tests for the [`RootfsSetup`] + [`ChildInit`] trait contract.
//!
//! All tests use `MockFilesystem` — no real mounts or syscalls are made.

use minibox::testing::mocks::filesystem::MockFilesystem;
use minibox_core::domain::RootfsSetup;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("create tempdir")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "setup_rootfs_succeeds_with_no_layers",
    adapter: "filesystem",
    capability: Filesystem,
    category: Unit,
    |ctx| {
        let fs = MockFilesystem::new();
        let dir = tmp();
        let result = fs.setup_rootfs(&[], dir.path());
        ctx.assert_ok(result, "setup_rootfs with empty layers should succeed");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "setup_rootfs_increments_count",
    adapter: "filesystem",
    capability: Filesystem,
    category: Unit,
    |ctx| {
        let fs = MockFilesystem::new();
        let dir = tmp();
        let _ = fs.setup_rootfs(&[], dir.path());
        ctx.assert_eq(1, fs.setup_count(), "setup_count after one call");
        let _ = fs.setup_rootfs(&[], dir.path());
        ctx.assert_eq(2, fs.setup_count(), "setup_count after two calls");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "cleanup_increments_count",
    adapter: "filesystem",
    capability: Filesystem,
    category: Unit,
    |ctx| {
        let fs = MockFilesystem::new();
        let dir = tmp();
        let _ = fs.cleanup(dir.path());
        ctx.assert_eq(1, fs.cleanup_count(), "cleanup_count after one call");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "setup_rootfs_failure_returns_err",
    adapter: "filesystem",
    capability: Filesystem,
    category: EdgeCase,
    |ctx| {
        let fs = MockFilesystem::new().with_setup_failure();
        let dir = tmp();
        let result = fs.setup_rootfs(&[], dir.path());
        ctx.assert_err(
            result,
            "setup_rootfs with failure configured must return Err",
        );
        ctx.result()
    }
}
