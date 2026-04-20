//! Trace storage port for pipeline execution traces.
//!
//! The [`TraceStore`] trait defines a hexagonal port for persisting and
//! querying pipeline execution traces. Adapters (e.g., `FileTraceStore`)
//! implement this in downstream crates.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Unique identifier for a stored trace.
pub type TraceId = String;

/// Summary of a stored trace for listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSummary {
    /// Trace identifier.
    pub id: TraceId,
    /// Pipeline name or path.
    pub pipeline: String,
    /// ISO 8601 timestamp of when the trace was stored.
    pub timestamp: String,
    /// Exit code of the pipeline process.
    pub exit_code: i32,
    /// Number of steps in the trace.
    pub step_count: usize,
}

/// Filter criteria for listing traces.
#[derive(Debug, Clone, Default)]
pub struct TraceFilter {
    /// Only return traces newer than this ISO 8601 timestamp.
    pub since: Option<String>,
    /// Only return traces for this pipeline name/path.
    pub pipeline: Option<String>,
    /// Maximum number of results to return.
    pub limit: Option<usize>,
}

/// Port for persisting and querying pipeline execution traces.
///
/// Implementations must be `Send + Sync` for use in the async daemon.
pub trait TraceStore: Send + Sync {
    /// Persist a trace. The `id` is used as the storage key.
    fn store(
        &self,
        id: &str,
        pipeline: &str,
        trace: &serde_json::Value,
        exit_code: i32,
    ) -> Result<()>;

    /// List traces matching the given filter, ordered newest-first.
    fn list(&self, filter: &TraceFilter) -> Result<Vec<TraceSummary>>;

    /// Load a trace by ID. Returns `None` if not found.
    fn load(&self, id: &str) -> Result<Option<serde_json::Value>>;
}
