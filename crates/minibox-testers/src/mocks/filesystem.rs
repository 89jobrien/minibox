//! Mock implementations of [`FilesystemProvider`].

use anyhow::Result;
use minibox_core::domain::{AsAny, ChildInit, RootfsLayout, RootfsSetup};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// MockFilesystem
// ---------------------------------------------------------------------------

/// Mock implementation of [`FilesystemProvider`] for testing.
///
/// Simulates filesystem operations without any actual mounts or syscalls.
/// Tracks `setup_rootfs` and `cleanup` call counts and can be configured to
/// fail on demand.
#[derive(Debug, Clone)]
pub struct MockFilesystem {
    state: Arc<Mutex<MockFilesystemState>>,
}

#[derive(Debug)]
pub struct MockFilesystemState {
    /// Whether `setup_rootfs` should succeed.
    setup_should_succeed: bool,
    /// Whether `pivot_root` should succeed.
    pivot_should_succeed: bool,
    /// Whether `cleanup` should succeed.
    cleanup_should_succeed: bool,
    /// Running count of `setup_rootfs` invocations.
    setup_count: usize,
    /// Running count of `cleanup` invocations.
    cleanup_count: usize,
}

impl MockFilesystem {
    /// Create a new mock filesystem with all operations succeeding by default.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockFilesystemState {
                setup_should_succeed: true,
                pivot_should_succeed: true,
                cleanup_should_succeed: true,
                setup_count: 0,
                cleanup_count: 0,
            })),
        }
    }

    /// Configure `setup_rootfs` to return an error on the next call.
    pub fn with_setup_failure(self) -> Self {
        self.state.lock().unwrap().setup_should_succeed = false;
        self
    }

    /// Return the number of times `setup_rootfs` has been called.
    pub fn setup_count(&self) -> usize {
        self.state.lock().unwrap().setup_count
    }

    /// Return the number of times `cleanup` has been called.
    pub fn cleanup_count(&self) -> usize {
        self.state.lock().unwrap().cleanup_count
    }
}

impl RootfsSetup for MockFilesystem {
    /// Simulate rootfs setup by returning `container_dir/merged`.
    ///
    /// Increments the setup counter. Returns an error if configured via
    /// [`with_setup_failure`].
    fn setup_rootfs(&self, _layers: &[PathBuf], container_dir: &Path) -> Result<RootfsLayout> {
        let mut state = self.state.lock().unwrap();
        state.setup_count += 1;

        if !state.setup_should_succeed {
            anyhow::bail!("mock filesystem setup failure");
        }

        Ok(RootfsLayout {
            merged_dir: container_dir.join("merged"),
            rootfs_metadata: None,
            source_image_ref: None,
        })
    }

    /// Simulate filesystem cleanup.
    ///
    /// Increments the cleanup counter. Returns an error if the mock is
    /// configured to fail cleanup.
    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.cleanup_count += 1;

        if !state.cleanup_should_succeed {
            anyhow::bail!("mock cleanup failure");
        }
        Ok(())
    }
}

impl ChildInit for MockFilesystem {
    /// Simulate `pivot_root` — succeeds unless configured to fail.
    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        let state = self.state.lock().unwrap();
        if !state.pivot_should_succeed {
            anyhow::bail!("mock pivot_root failure");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FailableFilesystemMock
// ---------------------------------------------------------------------------

/// Filesystem mock with runtime-toggleable failure injection via atomics.
///
/// Unlike [`MockFilesystem`] whose failure modes are fixed at construction
/// time via builder methods, this mock lets tests flip failures on and off
/// between individual calls using atomic stores. This is useful for testing
/// error-recovery paths such as cleanup-after-setup-failure.
///
/// Uses `SeqCst` ordering throughout to avoid races in parallel test scenarios.
pub struct FailableFilesystemMock {
    /// Whether the next `setup_rootfs` call should return an error.
    should_fail_setup: AtomicBool,
    /// Whether the next `cleanup` call should return an error.
    should_fail_cleanup: AtomicBool,
    /// Running count of `setup_rootfs` invocations.
    setup_count: AtomicUsize,
    /// Running count of `cleanup` invocations.
    cleanup_count: AtomicUsize,
}

impl FailableFilesystemMock {
    /// Create a new mock with both operations succeeding by default.
    pub fn new() -> Self {
        Self {
            should_fail_setup: AtomicBool::new(false),
            should_fail_cleanup: AtomicBool::new(false),
            setup_count: AtomicUsize::new(0),
            cleanup_count: AtomicUsize::new(0),
        }
    }

    /// Toggle whether `setup_rootfs` returns an error on the next call.
    ///
    /// Pass `true` to inject a failure; `false` to restore success.
    pub fn set_fail_setup(&self, fail: bool) {
        self.should_fail_setup.store(fail, Ordering::SeqCst);
    }

    /// Toggle whether `cleanup` returns an error on the next call.
    ///
    /// Pass `true` to inject a failure; `false` to restore success.
    pub fn set_fail_cleanup(&self, fail: bool) {
        self.should_fail_cleanup.store(fail, Ordering::SeqCst);
    }

    /// Return the number of times `setup_rootfs` has been called.
    pub fn setup_count(&self) -> usize {
        self.setup_count.load(Ordering::SeqCst)
    }

    /// Return the number of times `cleanup` has been called.
    pub fn cleanup_count(&self) -> usize {
        self.cleanup_count.load(Ordering::SeqCst)
    }
}

impl RootfsSetup for FailableFilesystemMock {
    /// Simulate rootfs setup, honouring the current failure toggle.
    fn setup_rootfs(&self, _layers: &[PathBuf], container_dir: &Path) -> Result<RootfsLayout> {
        self.setup_count.fetch_add(1, Ordering::SeqCst);
        if self.should_fail_setup.load(Ordering::SeqCst) {
            anyhow::bail!("injected setup failure");
        }
        Ok(RootfsLayout {
            merged_dir: container_dir.join("merged"),
            rootfs_metadata: None,
            source_image_ref: None,
        })
    }

    /// Simulate filesystem cleanup, honouring the current failure toggle.
    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        self.cleanup_count.fetch_add(1, Ordering::SeqCst);
        if self.should_fail_cleanup.load(Ordering::SeqCst) {
            anyhow::bail!("injected cleanup failure");
        }
        Ok(())
    }
}

impl ChildInit for FailableFilesystemMock {
    /// Always succeeds — `pivot_root` failure injection is not supported by this mock.
    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        Ok(())
    }
}

impl AsAny for MockFilesystem {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

impl Default for MockFilesystem {
    fn default() -> Self {
        Self::new()
    }
}

impl AsAny for FailableFilesystemMock {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

impl Default for FailableFilesystemMock {
    fn default() -> Self {
        Self::new()
    }
}
