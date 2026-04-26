//! Mock implementation of [`ImageBuilder`].

use async_trait::async_trait;
use minibox_core::domain::{
    AsAny, BuildConfig, BuildContext, BuildProgress, ImageBuilder, ImageMetadata, LayerInfo,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// MockImageBuilder
// ---------------------------------------------------------------------------

/// In-memory mock for [`ImageBuilder`].
///
/// Parses `config.tag` as `"name:tag"` and returns synthetic [`ImageMetadata`].
pub struct MockImageBuilder {
    call_count: AtomicUsize,
    should_fail: AtomicBool,
}

impl MockImageBuilder {
    /// Create a new mock builder.
    pub fn new() -> Self {
        Self {
            call_count: AtomicUsize::new(0),
            should_fail: AtomicBool::new(false),
        }
    }

    /// Configure all subsequent `build_image` calls to return an error.
    pub fn with_failure(self) -> Self {
        self.should_fail.store(true, Ordering::SeqCst);
        self
    }

    /// Number of times `build_image` has been called.
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl Default for MockImageBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AsAny for MockImageBuilder {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

#[async_trait]
impl ImageBuilder for MockImageBuilder {
    async fn build_image(
        &self,
        _context: &BuildContext,
        config: &BuildConfig,
        progress_tx: tokio::sync::mpsc::Sender<BuildProgress>,
    ) -> anyhow::Result<ImageMetadata> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        if self.should_fail.load(Ordering::SeqCst) {
            anyhow::bail!("mock build failure");
        }
        let _ = progress_tx
            .send(BuildProgress {
                step: 1,
                total_steps: 1,
                message: "mock build step".to_string(),
            })
            .await;

        let (name, tag) = if let Some((n, t)) = config.tag.rsplit_once(':') {
            (n.to_string(), t.to_string())
        } else {
            (config.tag.clone(), "latest".to_string())
        };
        Ok(ImageMetadata {
            name,
            tag,
            layers: vec![LayerInfo {
                digest: "sha256:build00000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                size: 2048,
            }],
        })
    }
}
