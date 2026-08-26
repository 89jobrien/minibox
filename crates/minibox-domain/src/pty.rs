//! Pseudo-terminal allocation ports and shared PTY value types.

use std::sync::Arc;

// ---------------------------------------------------------------------------
// PTY Allocator Port (#83)
// ---------------------------------------------------------------------------

/// Configuration for allocating a pseudo-terminal (PTY) for interactive containers.
///
/// Passed to [`PtyAllocator::allocate`] to request a PTY pair with the given
/// terminal dimensions. The caller is responsible for closing the returned file
/// descriptors when no longer needed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PtyConfig {
    /// Whether PTY allocation is requested.
    pub enabled: bool,
    /// Terminal width in columns.
    pub cols: u16,
    /// Terminal height in rows.
    pub rows: u16,
}

/// An allocated PTY pair — a master and a slave file descriptor.
///
/// The master fd is used by the host to read/write the terminal stream.
/// The slave fd is handed to the container process as its controlling terminal.
///
/// # Ownership
///
/// The caller that calls [`PtyAllocator::allocate`] owns both fds and is
/// responsible for closing them. Do NOT call `close()` on them from outside
/// unless you also own the handle.
#[derive(Debug)]
pub struct PtyHandle {
    /// File descriptor for the master side of the PTY.
    pub master_fd: i32,
    /// File descriptor for the slave side of the PTY.
    pub slave_fd: i32,
}

/// Port for allocating a PTY pair.
///
/// Implementations live in the adapter layer. The domain layer never calls
/// `posix_openpt` directly — all OS-level PTY operations go through this trait.
pub trait PtyAllocator: Send + Sync {
    /// Allocate a PTY pair with the terminal dimensions specified in `config`.
    ///
    /// Returns [`PtyHandle`] on success, or `Err` when PTY allocation is not
    /// supported (e.g., [`NullPtyAllocator`]) or when the OS call fails.
    ///
    /// # Errors
    ///
    /// Returns an error if PTY allocation is unsupported or the OS call fails.
    fn allocate(&self, config: &PtyConfig) -> anyhow::Result<PtyHandle>;
}

/// Type alias for a shared, dynamic [`PtyAllocator`] implementation.
pub type DynPtyAllocator = Arc<dyn PtyAllocator>;

/// A no-op [`PtyAllocator`] that always returns `Err`.
///
/// Used as the default adapter when PTY support is not available (e.g., on
/// macOS or in test environments that do not exercise the PTY path).
pub struct NullPtyAllocator;

impl PtyAllocator for NullPtyAllocator {
    fn allocate(&self, _config: &PtyConfig) -> anyhow::Result<PtyHandle> {
        anyhow::bail!("pty: PTY allocation is not supported in this environment")
    }
}

/// A test double [`PtyAllocator`] that returns a pre-configured [`PtyHandle`].
///
/// Enabled only when the `test-utils` feature is active so production binaries
/// do not pull in test scaffolding.
#[cfg(any(test, feature = "test-utils"))]
pub struct MockPtyAllocator {
    master_fd: i32,
    slave_fd: i32,
}

#[cfg(any(test, feature = "test-utils"))]
impl MockPtyAllocator {
    /// Create a `MockPtyAllocator` that returns `master_fd` and `slave_fd`.
    #[must_use]
    pub const fn new(master_fd: i32, slave_fd: i32) -> Self {
        Self {
            master_fd,
            slave_fd,
        }
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl PtyAllocator for MockPtyAllocator {
    fn allocate(&self, _config: &PtyConfig) -> anyhow::Result<PtyHandle> {
        Ok(PtyHandle {
            master_fd: self.master_fd,
            slave_fd: self.slave_fd,
        })
    }
}
