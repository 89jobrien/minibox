// dashbox/src/data/mod.rs
pub mod agents;
pub mod bench;
pub mod ci;
pub mod git;
pub mod metrics;
pub mod todos;

use anyhow::Result;
use std::time::Instant;

pub trait DataSource {
    type Data;
    fn load(&self) -> Result<Self::Data>;
}

/// Wrapper that caches loaded data and tracks staleness.
pub struct CachedSource<S: DataSource> {
    source: S,
    cached: Option<Result<S::Data, String>>,
    last_load: Option<Instant>,
    stale_secs: u64,
}

impl<S: DataSource> CachedSource<S> {
    pub fn new(source: S, stale_secs: u64) -> Self {
        Self {
            source,
            cached: None,
            last_load: None,
            stale_secs,
        }
    }

    pub fn is_stale(&self) -> bool {
        self.last_load
            .map(|t| t.elapsed().as_secs() >= self.stale_secs)
            .unwrap_or(true)
    }

    pub fn refresh(&mut self) {
        self.cached = Some(self.source.load().map_err(|e| e.to_string()));
        self.last_load = Some(Instant::now());
    }

    pub fn get(&self) -> Option<&Result<S::Data, String>> {
        self.cached.as_ref()
    }

    pub fn ensure_fresh(&mut self) {
        if self.is_stale() {
            self.refresh();
        }
    }
}
