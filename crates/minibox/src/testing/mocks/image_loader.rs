//! Mock [`ImageLoader`] for conformance testing.

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::domain::ImageLoader;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Mock image loader that records load calls without touching the filesystem.
///
/// Use [`MockImageLoader::failing`] to exercise error-handling paths.
#[derive(Debug)]
pub struct MockImageLoader {
    load_count: AtomicUsize,
    should_fail: bool,
}

impl MockImageLoader {
    /// Create a mock that succeeds on every call.
    pub fn new() -> Self {
        Self {
            load_count: AtomicUsize::new(0),
            should_fail: false,
        }
    }

    /// Create a mock that returns an error on every call.
    pub fn failing() -> Self {
        Self {
            load_count: AtomicUsize::new(0),
            should_fail: true,
        }
    }

    /// Number of `load_image` calls made so far.
    pub fn load_count(&self) -> usize {
        self.load_count.load(Ordering::Relaxed)
    }
}

impl Default for MockImageLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ImageLoader for MockImageLoader {
    async fn load_image(&self, _path: &Path, _name: &str, _tag: &str) -> Result<()> {
        self.load_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail {
            anyhow::bail!("mock: load_image configured to fail");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn load_image_increments_count() {
        let mock = MockImageLoader::new();
        mock.load_image(&PathBuf::from("/tmp/image.tar"), "alpine", "3.18")
            .await
            .expect("should succeed");
        assert_eq!(mock.load_count(), 1);
    }

    #[tokio::test]
    async fn failing_mock_returns_error() {
        let mock = MockImageLoader::failing();
        let result = mock
            .load_image(&PathBuf::from("/tmp/image.tar"), "alpine", "3.18")
            .await;
        assert!(result.is_err());
        assert_eq!(mock.load_count(), 1);
    }
}
