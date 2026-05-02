//! Trace storage port for pipeline execution traces.
//!
//! The [`TraceStore`] trait defines a hexagonal port for persisting and
//! querying pipeline execution traces. [`FileTraceStore`] is the default
//! adapter, writing one JSON file per trace under a configurable base directory.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

// ---------------------------------------------------------------------------
// FileTraceStore — default adapter
// ---------------------------------------------------------------------------

/// On-disk format written by [`FileTraceStore`].
#[derive(Debug, Serialize, Deserialize)]
struct TraceRecord {
    id: String,
    pipeline: String,
    timestamp: String,
    exit_code: i32,
    trace: serde_json::Value,
}

/// Default [`TraceStore`] adapter: writes one JSON file per trace.
///
/// Files are named `<id>.json` and stored flat under `base_dir`.
/// [`list`] scans the directory and deserialises the metadata header;
/// [`load`] reads the full record.
pub struct FileTraceStore {
    base_dir: PathBuf,
}

impl FileTraceStore {
    /// Create a new store backed by `base_dir`, creating it if necessary.
    pub fn new(base_dir: impl AsRef<Path>) -> Result<Self> {
        let base_dir = base_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&base_dir)
            .with_context(|| format!("create trace store dir {}", base_dir.display()))?;
        Ok(Self { base_dir })
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.base_dir.join(format!("{id}.json"))
    }
}

impl TraceStore for FileTraceStore {
    fn store(
        &self,
        id: &str,
        pipeline: &str,
        trace: &serde_json::Value,
        exit_code: i32,
    ) -> Result<()> {
        let timestamp = {
            use std::time::{SystemTime, UNIX_EPOCH};
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            // RFC 3339-ish without chrono dependency: seconds-precision UTC.
            format!("{secs}")
        };

        let record = TraceRecord {
            id: id.to_string(),
            pipeline: pipeline.to_string(),
            timestamp,
            exit_code,
            trace: trace.clone(),
        };

        let json = serde_json::to_string_pretty(&record)
            .with_context(|| format!("serialise trace {id}"))?;

        let path = self.path_for(id);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)
            .with_context(|| format!("write trace tmp {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("rename trace {} → {}", tmp.display(), path.display()))?;
        Ok(())
    }

    fn list(&self, filter: &TraceFilter) -> Result<Vec<TraceSummary>> {
        let mut summaries = Vec::new();

        let entries = match std::fs::read_dir(&self.base_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("read trace dir {}", self.base_dir.display()));
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = match std::fs::read_to_string(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let record: TraceRecord = match serde_json::from_str(&data) {
                Ok(r) => r,
                Err(_) => continue,
            };

            // Apply filter criteria.
            if let Some(ref since) = filter.since {
                if record.timestamp < *since {
                    continue;
                }
            }
            if let Some(ref pipeline) = filter.pipeline {
                if record.pipeline != *pipeline {
                    continue;
                }
            }

            let step_count = record
                .trace
                .get("steps")
                .and_then(|s| s.as_array())
                .map(|a| a.len())
                .unwrap_or(0);

            summaries.push(TraceSummary {
                id: record.id,
                pipeline: record.pipeline,
                timestamp: record.timestamp,
                exit_code: record.exit_code,
                step_count,
            });
        }

        // Newest-first by timestamp string (seconds-since-epoch sorts lexically).
        summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        if let Some(limit) = filter.limit {
            summaries.truncate(limit);
        }

        Ok(summaries)
    }

    fn load(&self, id: &str) -> Result<Option<serde_json::Value>> {
        let path = self.path_for(id);
        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("read trace file {}", path.display())),
        };
        let record: TraceRecord = serde_json::from_str(&data)
            .with_context(|| format!("parse trace file {}", path.display()))?;
        Ok(Some(record.trace))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_trace() -> serde_json::Value {
        serde_json::json!({"steps": [{"name": "run", "exit_code": 0}]})
    }

    #[test]
    fn file_trace_store_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = FileTraceStore::new(tmp.path()).unwrap();

        store
            .store("trace-1", "my-pipeline.cruxx", &sample_trace(), 0)
            .unwrap();

        let loaded = store.load("trace-1").unwrap().expect("should be present");
        assert_eq!(loaded["steps"][0]["name"], "run");
    }

    #[test]
    fn file_trace_store_load_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let store = FileTraceStore::new(tmp.path()).unwrap();
        assert!(store.load("nonexistent").unwrap().is_none());
    }

    #[test]
    fn file_trace_store_list_empty() {
        let tmp = TempDir::new().unwrap();
        let store = FileTraceStore::new(tmp.path()).unwrap();
        let results = store.list(&TraceFilter::default()).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn file_trace_store_list_with_filter() {
        let tmp = TempDir::new().unwrap();
        let store = FileTraceStore::new(tmp.path()).unwrap();

        store
            .store("id-a", "pipeline-a.cruxx", &sample_trace(), 0)
            .unwrap();
        store
            .store(
                "id-b",
                "pipeline-b.cruxx",
                &serde_json::json!({"steps":[]}),
                1,
            )
            .unwrap();

        let filter = TraceFilter {
            pipeline: Some("pipeline-a.cruxx".to_string()),
            ..Default::default()
        };
        let results = store.list(&filter).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "id-a");
        assert_eq!(results[0].step_count, 1);
    }

    #[test]
    fn file_trace_store_list_limit() {
        let tmp = TempDir::new().unwrap();
        let store = FileTraceStore::new(tmp.path()).unwrap();

        store.store("id-1", "p.cruxx", &sample_trace(), 0).unwrap();
        store.store("id-2", "p.cruxx", &sample_trace(), 0).unwrap();
        store.store("id-3", "p.cruxx", &sample_trace(), 0).unwrap();

        let filter = TraceFilter {
            limit: Some(2),
            ..Default::default()
        };
        let results = store.list(&filter).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn noop_trace_store_is_harmless() {
        let store = NoopTraceStore;
        store.store("x", "p", &serde_json::json!({}), 0).unwrap();
        assert!(store.list(&TraceFilter::default()).unwrap().is_empty());
        assert!(store.load("x").unwrap().is_none());
    }
}

/// No-op [`TraceStore`] for use in tests that don't need trace persistence.
pub struct NoopTraceStore;

impl TraceStore for NoopTraceStore {
    fn store(
        &self,
        _id: &str,
        _pipeline: &str,
        _trace: &serde_json::Value,
        _exit_code: i32,
    ) -> Result<()> {
        Ok(())
    }
    fn list(&self, _filter: &TraceFilter) -> Result<Vec<TraceSummary>> {
        Ok(vec![])
    }
    fn load(&self, _id: &str) -> Result<Option<serde_json::Value>> {
        Ok(None)
    }
}
