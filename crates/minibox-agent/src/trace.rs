//! File-based adapter for the [`TraceStore`] port.
//!
//! Stores each pipeline trace as `{id}.json` under a configurable directory
//! (default `~/.mbx/traces/`). Rotation removes files older than
//! `MINIBOX_TRACE_RETENTION_DAYS` (default 7) or when total size exceeds
//! `MINIBOX_TRACE_MAX_MB` (default 500).

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::Utc;
use minibox_core::trace::{TraceFilter, TraceStore, TraceSummary};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// On-disk envelope
// ---------------------------------------------------------------------------

/// JSON envelope written to `{dir}/{id}.json`.
#[derive(Debug, Serialize, Deserialize)]
struct TraceEnvelope {
    id: String,
    pipeline: String,
    /// ISO 8601 UTC timestamp.
    timestamp: String,
    exit_code: i32,
    trace: Value,
}

// ---------------------------------------------------------------------------
// FileTraceStore
// ---------------------------------------------------------------------------

/// File-based [`TraceStore`] adapter.
///
/// Each trace is persisted as an individual JSON file named `{id}.json` in
/// the configured directory. Rotation is performed before every `store` call.
pub struct FileTraceStore {
    dir: PathBuf,
    retention_days: i64,
    max_bytes: u64,
}

impl FileTraceStore {
    /// Create a store backed by `dir`. The directory is created if absent.
    ///
    /// `retention_days` and `max_bytes` control rotation policy.
    pub fn new(dir: impl Into<PathBuf>, retention_days: i64, max_bytes: u64) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)
            .with_context(|| format!("create trace directory: {}", dir.display()))?;
        Ok(Self {
            dir,
            retention_days,
            max_bytes,
        })
    }

    /// Build from environment variables, falling back to defaults.
    ///
    /// - Directory: `MINIBOX_TRACE_DIR` or `~/.mbx/traces/`
    /// - Retention: `MINIBOX_TRACE_RETENTION_DAYS` (default 7)
    /// - Max size: `MINIBOX_TRACE_MAX_MB` (default 500)
    pub fn from_env() -> Result<Self> {
        let dir = std::env::var("MINIBOX_TRACE_DIR")
            .ok()
            .map(PathBuf::from)
            .or_else(default_trace_dir)
            .context("cannot determine trace directory: set MINIBOX_TRACE_DIR or HOME")?;

        let retention_days = std::env::var("MINIBOX_TRACE_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(7);

        let max_mb = std::env::var("MINIBOX_TRACE_MAX_MB")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(500);

        Self::new(dir, retention_days, max_mb * 1024 * 1024)
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    fn envelope_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Remove files that exceed the retention policy.
    ///
    /// Two passes:
    /// 1. Delete any file older than `retention_days`.
    /// 2. If total size still exceeds `max_bytes`, delete oldest-first until
    ///    under the limit.
    fn rotate(&self) -> Result<()> {
        let cutoff = Utc::now() - chrono::Duration::days(self.retention_days);

        let mut entries = collect_entries(&self.dir)?;

        // Pass 1: age-based deletion.
        entries.retain(|e| {
            if let Ok(meta) = fs::metadata(&e.path) {
                if let Ok(modified) = meta.modified() {
                    let modified: chrono::DateTime<Utc> = modified.into();
                    if modified < cutoff {
                        let _ = fs::remove_file(&e.path);
                        return false;
                    }
                }
            }
            true
        });

        // Pass 2: size-based deletion (oldest first).
        let total: u64 = entries.iter().map(|e| e.size).sum();
        if total > self.max_bytes {
            // Sort oldest-first (ascending mtime).
            entries.sort_by_key(|e| e.modified);
            let mut running = total;
            for entry in entries {
                if running <= self.max_bytes {
                    break;
                }
                if fs::remove_file(&entry.path).is_ok() {
                    running = running.saturating_sub(entry.size);
                }
            }
        }

        Ok(())
    }
}

impl TraceStore for FileTraceStore {
    fn store(&self, id: &str, pipeline: &str, trace: &Value, exit_code: i32) -> Result<()> {
        self.rotate()
            .context("trace rotation before store")?;

        let envelope = TraceEnvelope {
            id: id.to_owned(),
            pipeline: pipeline.to_owned(),
            timestamp: Utc::now().to_rfc3339(),
            exit_code,
            trace: trace.clone(),
        };

        let path = self.envelope_path(id);
        let json =
            serde_json::to_string_pretty(&envelope).context("serialize trace envelope")?;
        fs::write(&path, json)
            .with_context(|| format!("write trace file: {}", path.display()))?;

        Ok(())
    }

    fn list(&self, filter: &TraceFilter) -> Result<Vec<TraceSummary>> {
        let since: Option<chrono::DateTime<Utc>> = filter
            .since
            .as_deref()
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .with_context(|| format!("parse 'since' timestamp: {s}"))
                    .map(|dt| dt.with_timezone(&Utc))
            })
            .transpose()?;

        let mut summaries: Vec<(chrono::DateTime<Utc>, TraceSummary)> = read_dir_json(&self.dir)?
            .filter_map(|path| {
                let text = fs::read_to_string(&path).ok()?;
                let env: TraceEnvelope = serde_json::from_str(&text).ok()?;

                // Pipeline filter.
                if let Some(ref p) = filter.pipeline {
                    if &env.pipeline != p {
                        return None;
                    }
                }

                let ts = chrono::DateTime::parse_from_rfc3339(&env.timestamp)
                    .ok()?
                    .with_timezone(&Utc);

                // Since filter.
                if let Some(cutoff) = since {
                    if ts < cutoff {
                        return None;
                    }
                }

                let step_count = env
                    .trace
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);

                Some((
                    ts,
                    TraceSummary {
                        id: env.id,
                        pipeline: env.pipeline,
                        timestamp: env.timestamp,
                        exit_code: env.exit_code,
                        step_count,
                    },
                ))
            })
            .collect();

        // Newest-first.
        summaries.sort_by(|a, b| b.0.cmp(&a.0));

        let mut result: Vec<TraceSummary> = summaries.into_iter().map(|(_, s)| s).collect();

        if let Some(limit) = filter.limit {
            result.truncate(limit);
        }

        Ok(result)
    }

    fn load(&self, id: &str) -> Result<Option<Value>> {
        let path = self.envelope_path(id);
        if !path.exists() {
            return Ok(None);
        }
        let text =
            fs::read_to_string(&path).with_context(|| format!("read trace: {}", path.display()))?;
        let env: TraceEnvelope =
            serde_json::from_str(&text).with_context(|| format!("parse trace: {}", path.display()))?;
        Ok(Some(env.trace))
    }
}

// ---------------------------------------------------------------------------
// Private utilities
// ---------------------------------------------------------------------------

fn default_trace_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".mbx").join("traces"))
}

struct DirEntry {
    path: PathBuf,
    size: u64,
    modified: std::time::SystemTime,
}

fn collect_entries(dir: &Path) -> Result<Vec<DirEntry>> {
    let mut out = Vec::new();
    let read = fs::read_dir(dir).with_context(|| format!("read dir: {}", dir.display()))?;
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                out.push(DirEntry {
                    path,
                    size: meta.len(),
                    modified,
                });
            }
        }
    }
    Ok(out)
}

fn read_dir_json(dir: &Path) -> Result<impl Iterator<Item = PathBuf>> {
    let entries = collect_entries(dir)?;
    Ok(entries.into_iter().map(|e| e.path))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_store(dir: &TempDir) -> FileTraceStore {
        FileTraceStore::new(dir.path(), 7, 500 * 1024 * 1024)
            .expect("create FileTraceStore")
    }

    fn sample_trace() -> Value {
        json!([{"step": "run", "output": "ok"}])
    }

    #[test]
    fn store_and_load_roundtrip() {
        let tmp = TempDir::new().expect("tempdir");
        let store = make_store(&tmp);

        store
            .store("abc123", "my-pipeline", &sample_trace(), 0)
            .expect("store");

        let loaded = store.load("abc123").expect("load").expect("should be Some");
        assert_eq!(loaded, sample_trace());
    }

    #[test]
    fn list_filters_by_pipeline() {
        let tmp = TempDir::new().expect("tempdir");
        let store = make_store(&tmp);

        store
            .store("id1", "pipeline-a", &sample_trace(), 0)
            .expect("store 1");
        store
            .store("id2", "pipeline-a", &sample_trace(), 0)
            .expect("store 2");
        store
            .store("id3", "pipeline-b", &sample_trace(), 1)
            .expect("store 3");

        let filter = TraceFilter {
            pipeline: Some("pipeline-a".to_owned()),
            ..Default::default()
        };
        let results = store.list(&filter).expect("list");
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|s| s.pipeline == "pipeline-a"));
    }

    #[test]
    fn list_respects_limit() {
        let tmp = TempDir::new().expect("tempdir");
        let store = make_store(&tmp);

        for i in 0..10 {
            store
                .store(&format!("id{i}"), "pipe", &sample_trace(), 0)
                .expect("store");
        }

        let filter = TraceFilter {
            limit: Some(3),
            ..Default::default()
        };
        let results = store.list(&filter).expect("list");
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let tmp = TempDir::new().expect("tempdir");
        let store = make_store(&tmp);

        let result = store.load("no-such-id").expect("load");
        assert!(result.is_none());
    }
}
