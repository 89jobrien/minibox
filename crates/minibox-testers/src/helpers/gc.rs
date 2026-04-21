//! No-op image garbage collector for tests.

use minibox_core::image::gc::{ImageGarbageCollector, PruneReport};

pub struct NoopImageGc;

#[async_trait::async_trait]
impl ImageGarbageCollector for NoopImageGc {
    async fn prune(
        &self,
        dry_run: bool,
        _in_use: &[String],
    ) -> anyhow::Result<PruneReport> {
        Ok(PruneReport {
            removed: vec![],
            freed_bytes: 0,
            dry_run,
        })
    }
}
