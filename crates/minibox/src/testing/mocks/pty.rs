//! Mock [`PtyAllocator`] for conformance testing.
//!
//! The domain already ships a minimal `MockPtyAllocator` behind `test-utils`,
//! but this richer version lives in the mocks layer where it can track call
//! counts without leaking test scaffolding into the core crate.

use minibox_core::domain::{PtyAllocator, PtyConfig, PtyHandle};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Mock PTY allocator that tracks calls and returns synthetic fd pairs.
///
/// Useful for conformance tests that verify the `PtyAllocator` contract
/// without requiring OS-level PTY support.
#[derive(Debug)]
pub struct MockPtyAllocator {
    allocate_count: AtomicUsize,
    should_fail: bool,
}

impl MockPtyAllocator {
    /// Create a mock that succeeds on every call.
    pub fn new() -> Self {
        Self {
            allocate_count: AtomicUsize::new(0),
            should_fail: false,
        }
    }

    /// Create a mock that fails on every call.
    pub fn failing() -> Self {
        Self {
            allocate_count: AtomicUsize::new(0),
            should_fail: true,
        }
    }

    /// Number of `allocate` calls made so far.
    pub fn allocate_count(&self) -> usize {
        self.allocate_count.load(Ordering::Relaxed)
    }
}

impl Default for MockPtyAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyAllocator for MockPtyAllocator {
    fn allocate(&self, _config: &PtyConfig) -> anyhow::Result<PtyHandle> {
        self.allocate_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail {
            anyhow::bail!("mock: PtyAllocator configured to fail");
        }
        // Return synthetic fd numbers. Tests must not close these.
        const MOCK_MASTER_FD: i32 = 100;
        const MOCK_SLAVE_FD: i32 = 101;
        Ok(PtyHandle {
            master_fd: MOCK_MASTER_FD,
            slave_fd: MOCK_SLAVE_FD,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_increments_count() {
        let mock = MockPtyAllocator::new();
        let cfg = PtyConfig {
            enabled: true,
            cols: 80,
            rows: 24,
        };
        mock.allocate(&cfg).expect("allocate should succeed");
        assert_eq!(mock.allocate_count(), 1);
    }

    #[test]
    fn failing_mock_returns_error() {
        let mock = MockPtyAllocator::failing();
        let cfg = PtyConfig {
            enabled: true,
            cols: 80,
            rows: 24,
        };
        assert!(mock.allocate(&cfg).is_err());
    }
}
