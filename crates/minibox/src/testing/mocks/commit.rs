//! Mock implementation of [`ContainerCommitter`].

use async_trait::async_trait;
use minibox_core::domain::{
    AsAny, CommitConfig, ContainerCommitter, ContainerId, ImageMetadata, LayerInfo,
};
use std::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// MockContainerCommitter
// ---------------------------------------------------------------------------

/// In-memory mock for [`ContainerCommitter`].
///
/// Parses the `name:tag` target ref and returns synthetic [`ImageMetadata`].
/// Tracks commit call count for assertion.
pub struct MockContainerCommitter {
    call_count: AtomicUsize,
}

impl MockContainerCommitter {
    /// Create a new, unconfigured mock committer.
    pub fn new() -> Self {
        Self {
            call_count: AtomicUsize::new(0),
        }
    }

    /// Number of times `commit` has been called.
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl Default for MockContainerCommitter {
    fn default() -> Self {
        Self::new()
    }
}

impl AsAny for MockContainerCommitter {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

#[async_trait]
impl ContainerCommitter for MockContainerCommitter {
    async fn commit(
        &self,
        _container_id: &ContainerId,
        target_ref: &str,
        _config: &CommitConfig,
    ) -> anyhow::Result<ImageMetadata> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        // Parse "name:tag" — fall back to "name:latest" if no colon.
        let (name, tag) = if let Some((n, t)) = target_ref.rsplit_once(':') {
            (n.to_string(), t.to_string())
        } else {
            (target_ref.to_string(), "latest".to_string())
        };
        Ok(ImageMetadata {
            name,
            tag,
            layers: vec![LayerInfo {
                digest: "sha256:mock000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                size: 1024,
            }],
        })
    }
}
