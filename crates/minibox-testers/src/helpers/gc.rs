//! No-op image garbage collector for tests.

use minibox_core::image::gc::{ImageGarbageCollector, PruneReport};
use std::sync::{Arc, Mutex};

pub struct NoopImageGc {
    call_count: Arc<Mutex<usize>>,
}

impl Default for NoopImageGc {
    fn default() -> Self {
        Self::new()
    }
}

impl NoopImageGc {
    pub fn new() -> Self {
        Self {
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    pub fn prune_call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl ImageGarbageCollector for NoopImageGc {
    async fn prune(&self, dry_run: bool, _in_use: &[String]) -> anyhow::Result<PruneReport> {
        *self.call_count.lock().unwrap() += 1;
        Ok(PruneReport {
            removed: vec![],
            freed_bytes: 0,
            dry_run,
        })
    }
}
