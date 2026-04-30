---
status: archived
completed: "2026-04-20"
branch: main
note: "Protocol wiring landed (RunPipeline, slashcrux fields); minibox-agent and minibox-crux-plugin crates never created"
---

# Crux-Minibox Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire crux pipeline execution into minibox so that `.cruxx`
pipelines run inside minibox containers with structured tracing, daemon
integration, and a plugin binary exposing container ops to crux.

**Architecture:** Hexagonal — `TraceStore` trait (port) in
`minibox-core`, `FileTraceStore` adapter in `minibox-agent`.
`PipelineRunner` in `minibox-agent` orchestrates execution.
`minibox-crux-plugin` is a standalone binary crate implementing
`cruxx-plugin` JSON-RPC. `handle_pipeline` in `daemonbox` ties it
together. Protocol additions follow existing `serde(tag = "type")` +
`serde(default)` conventions.

**Tech Stack:** Rust 2024, serde, tokio, cruxx-types, cruxx-script,
cruxx-plugin, minibox-client, minibox-secrets

**Spec:** `docs/superpowers/specs/2026-04-20-crux-maestro-integration-design.md`

**Sentinel review findings (must-address):**

1. TraceStore trait must live in `minibox-core`, not `minibox-agent`
   (avoids inverted dependency daemonbox -> minibox-agent)
2. `PipelineComplete` uses `serde_json::Value` for the trace field,
   keeping `minibox-core` crux-agnostic — consumers deserialize into
   `Crux<T>` themselves
3. `handle_pipeline` needs a daemon-side recursion/depth gate
4. Multi-container fan-out needs group-based cleanup on parent death
5. `minibox-crux-plugin` is its own crate (not a `[[bin]]` in
   minibox-agent)

---

## Prerequisite: Phase 0 (crux workspace — out of scope)

These must ship before Phase 2 begins:

- `cruxx-types` crate extracted and published (or git-pinned)
- `cruxx-script` pipeline loading API stable
- `cruxx-plugin` JSON-RPC protocol crate stable

Track as a blocker. If unpublished, use `git` dependency with pinned rev
in minibox's `Cargo.toml`.

---

## Phase 1: Protocol + TraceStore

### Task 1.1: Add `RunPipeline` to `DaemonRequest`

**Files:**

- Modify: `crates/minibox-core/src/protocol.rs`
- Modify: `crates/minibox-core/tests/` (snapshot tests)

- [ ] **Step 1: Add RunPipeline variant**

In `crates/minibox-core/src/protocol.rs`, add after the last
`DaemonRequest` variant (before the closing `}`):

```rust
    /// Run a crux pipeline inside a container.
    ///
    /// Higher-level than `Run` — bundles image pull + container create +
    /// pipeline execution + trace collection.
    RunPipeline {
        /// Path to the `.cruxx` pipeline file (host-side).
        pipeline_path: String,
        /// Optional JSON input to the pipeline.
        #[serde(default)]
        input: Option<serde_json::Value>,
        /// Container image to use. Defaults to `cruxx-runtime:latest`.
        #[serde(default)]
        image: Option<String>,
        /// Token/step/time budget for the pipeline execution.
        #[serde(default)]
        budget: Option<serde_json::Value>,
        /// Additional environment variables as (KEY, VALUE) pairs.
        #[serde(default)]
        env: Vec<(String, String)>,
        /// Maximum container nesting depth (daemon-enforced).
        /// Defaults to 3. Requests exceeding this are rejected.
        #[serde(default = "default_max_depth")]
        max_depth: u32,
    },
```

Add the default function above the enum:

```rust
fn default_max_depth() -> u32 {
    3
}
```

- [ ] **Step 2: Add `serde_json` dep to minibox-core if not present**

Check `crates/minibox-core/Cargo.toml` — `serde_json` is likely already
a dependency. If not, add:

```toml
serde_json = "1"
```

- [ ] **Step 3: Run `cargo check -p minibox-core`**

Expected: compiles. Fix any issues.

- [ ] **Step 4: Add snapshot test**

In the existing snapshot test file for protocol
(`crates/minibox-core/tests/` or inline `#[cfg(test)]` in protocol.rs),
add:

```rust
#[test]
fn run_pipeline_request_snapshot() {
    let req = DaemonRequest::RunPipeline {
        pipeline_path: "/workspace/.cruxx/pipelines/work.cruxx".into(),
        input: Some(serde_json::json!({"prompt": "hello"})),
        image: None,
        budget: None,
        env: vec![("CRUX_LOG".into(), "debug".into())],
        max_depth: 3,
    };
    insta::assert_json_snapshot!(req);
}

#[test]
fn run_pipeline_request_minimal_snapshot() {
    let req = DaemonRequest::RunPipeline {
        pipeline_path: "work.cruxx".into(),
        input: None,
        image: None,
        budget: None,
        env: vec![],
        max_depth: 3,
    };
    insta::assert_json_snapshot!(req);
}
```

- [ ] **Step 5: Run tests, accept snapshots**

```bash
cargo test -p minibox-core -- run_pipeline
cargo insta accept
```

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-core/
git commit -m "feat(protocol): add RunPipeline request variant"
```

---

### Task 1.2: Add `PipelineComplete` to `DaemonResponse`

**Files:**

- Modify: `crates/minibox-core/src/protocol.rs`
- Modify: `crates/daemonbox/src/server.rs` (is_terminal_response)

- [ ] **Step 1: Add PipelineComplete variant**

In `DaemonResponse`, add:

```rust
    /// Pipeline execution completed.
    ///
    /// Terminal response for `RunPipeline` requests. The `trace` field
    /// contains the full `Crux<T>` trace serialized as JSON — consumers
    /// deserialize into their concrete trace type.
    PipelineComplete {
        /// Serialized execution trace (crux-agnostic JSON).
        trace: serde_json::Value,
        /// Container ID that ran the pipeline.
        container_id: String,
        /// Exit code of the `crux run` process.
        exit_code: i32,
    },
```

- [ ] **Step 2: Update `is_terminal_response` in server.rs**

In `crates/daemonbox/src/server.rs`, find the `is_terminal_response`
function (or equivalent match). Add `DaemonResponse::PipelineComplete
{ .. }` as a terminal variant (returns `true`).

- [ ] **Step 3: Add snapshot test**

```rust
#[test]
fn pipeline_complete_response_snapshot() {
    let resp = DaemonResponse::PipelineComplete {
        trace: serde_json::json!({
            "id": "01HYX...",
            "steps": [],
            "result": "ok"
        }),
        container_id: "abc123def456".into(),
        exit_code: 0,
    };
    insta::assert_json_snapshot!(resp);
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p minibox-core -- pipeline_complete
cargo insta accept
cargo check -p daemonbox
```

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-core/ crates/daemonbox/src/server.rs
git commit -m "feat(protocol): add PipelineComplete response variant"
```

---

### Task 1.3: TraceStore trait in minibox-core

**Files:**

- Create: `crates/minibox-core/src/trace.rs`
- Modify: `crates/minibox-core/src/lib.rs`

- [ ] **Step 1: Write the trait definition**

Create `crates/minibox-core/src/trace.rs`:

```rust
//! Trace storage port for pipeline execution traces.
//!
//! The `TraceStore` trait defines a hexagonal port for persisting and
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
    /// Persist a trace. The `id` field in the trace JSON is used as the
    /// storage key.
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
```

- [ ] **Step 2: Register module in lib.rs**

Add to `crates/minibox-core/src/lib.rs`:

```rust
pub mod trace;
```

- [ ] **Step 3: Run check**

```bash
cargo check -p minibox-core
```

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-core/src/trace.rs crates/minibox-core/src/lib.rs
git commit -m "feat(core): add TraceStore trait (hexagonal port)"
```

---

### Task 1.4: FileTraceStore adapter in minibox-agent

**Files:**

- Create: `crates/minibox-agent/src/trace.rs`
- Modify: `crates/minibox-agent/src/lib.rs`
- Modify: `crates/minibox-agent/Cargo.toml`

- [ ] **Step 1: Add dependencies**

In `crates/minibox-agent/Cargo.toml`, add:

```toml
[dependencies]
minibox-core = { path = "../minibox-core" }
chrono = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

(Some of these may already be present — check first.)

- [ ] **Step 2: Write FileTraceStore**

Create `crates/minibox-agent/src/trace.rs`:

```rust
//! File-based trace storage adapter.
//!
//! Writes traces to `~/.minibox/traces/<id>.json`. Enforces 7-day retention
//! and 500MB cap, rotating oldest traces when either limit is hit.
//! Override with `MINIBOX_TRACE_RETENTION_DAYS` and `MINIBOX_TRACE_MAX_MB`.

use anyhow::{Context, Result};
use minibox_core::trace::{TraceFilter, TraceId, TraceStore, TraceSummary};
use std::fs;
use std::path::{Path, PathBuf};

/// File-based [`TraceStore`] adapter.
///
/// Stores one JSON file per trace in the configured directory.
pub struct FileTraceStore {
    dir: PathBuf,
    retention_days: u64,
    max_bytes: u64,
}

impl FileTraceStore {
    /// Create a new `FileTraceStore` writing to `dir`.
    ///
    /// Creates the directory if it does not exist.
    pub fn new(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir)
            .with_context(|| format!("create trace dir: {}", dir.display()))?;

        let retention_days = std::env::var("MINIBOX_TRACE_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);

        let max_bytes = std::env::var("MINIBOX_TRACE_MAX_MB")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(|mb| mb * 1024 * 1024)
            .unwrap_or(500 * 1024 * 1024);

        Ok(Self {
            dir,
            retention_days,
            max_bytes,
        })
    }

    /// Default trace directory: `~/.minibox/traces/`.
    pub fn default_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".minibox")
            .join("traces")
    }

    /// Remove traces older than retention period or when total size
    /// exceeds cap.
    fn rotate(&self) -> Result<()> {
        let cutoff = chrono::Utc::now()
            - chrono::Duration::days(self.retention_days as i64);

        let mut entries: Vec<(PathBuf, std::fs::Metadata)> = fs::read_dir(&self.dir)
            .with_context(|| format!("read trace dir: {}", self.dir.display()))?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension().and_then(|s| s.to_str()) == Some("json")
            })
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                Some((e.path(), meta))
            })
            .collect();

        // Sort oldest-first by modified time.
        entries.sort_by_key(|(_, m)| {
            m.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });

        // Remove expired traces.
        for (path, meta) in &entries {
            let modified = meta
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let modified_dt: chrono::DateTime<chrono::Utc> = modified.into();
            if modified_dt < cutoff {
                let _ = fs::remove_file(path);
            }
        }

        // Check total size and remove oldest until under cap.
        let mut total: u64 = entries.iter().map(|(_, m)| m.len()).sum();
        for (path, meta) in &entries {
            if total <= self.max_bytes {
                break;
            }
            if path.exists() {
                let _ = fs::remove_file(path);
                total = total.saturating_sub(meta.len());
            }
        }

        Ok(())
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
        // Rotate before writing.
        let _ = self.rotate();

        let envelope = serde_json::json!({
            "id": id,
            "pipeline": pipeline,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "exit_code": exit_code,
            "trace": trace,
        });

        let path = self.dir.join(format!("{id}.json"));
        let content = serde_json::to_string_pretty(&envelope)
            .context("serialize trace")?;
        fs::write(&path, content)
            .with_context(|| format!("write trace: {}", path.display()))?;
        Ok(())
    }

    fn list(&self, filter: &TraceFilter) -> Result<Vec<TraceSummary>> {
        let mut results = Vec::new();

        for entry in fs::read_dir(&self.dir)
            .with_context(|| format!("read trace dir: {}", self.dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }

            let content = fs::read_to_string(&path)
                .with_context(|| format!("read trace: {}", path.display()))?;
            let envelope: serde_json::Value = serde_json::from_str(&content)
                .with_context(|| format!("parse trace: {}", path.display()))?;

            let id = envelope["id"].as_str().unwrap_or("").to_string();
            let pipeline = envelope["pipeline"].as_str().unwrap_or("").to_string();
            let timestamp = envelope["timestamp"].as_str().unwrap_or("").to_string();
            let exit_code = envelope["exit_code"].as_i64().unwrap_or(-1) as i32;
            let step_count = envelope["trace"]["steps"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);

            // Apply filters.
            if let Some(ref since) = filter.since {
                if timestamp < *since {
                    continue;
                }
            }
            if let Some(ref p) = filter.pipeline {
                if !pipeline.contains(p.as_str()) {
                    continue;
                }
            }

            results.push(TraceSummary {
                id,
                pipeline,
                timestamp,
                exit_code,
                step_count,
            });
        }

        // Sort newest-first.
        results.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        if let Some(limit) = filter.limit {
            results.truncate(limit);
        }

        Ok(results)
    }

    fn load(&self, id: &str) -> Result<Option<serde_json::Value>> {
        let path = self.dir.join(format!("{id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("read trace: {}", path.display()))?;
        let envelope: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("parse trace: {}", path.display()))?;
        Ok(Some(envelope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (FileTraceStore, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = FileTraceStore::new(tmp.path().to_path_buf())
            .expect("create store");
        (store, tmp)
    }

    #[test]
    fn store_and_load_roundtrip() {
        let (store, _tmp) = test_store();
        let trace = serde_json::json!({"steps": [{"name": "step1"}]});
        store
            .store("trace-001", "work.cruxx", &trace, 0)
            .expect("store");

        let loaded = store.load("trace-001").expect("load").expect("found");
        assert_eq!(loaded["pipeline"], "work.cruxx");
        assert_eq!(loaded["exit_code"], 0);
        assert_eq!(loaded["trace"]["steps"][0]["name"], "step1");
    }

    #[test]
    fn list_filters_by_pipeline() {
        let (store, _tmp) = test_store();
        let trace = serde_json::json!({"steps": []});
        store.store("t1", "build.cruxx", &trace, 0).unwrap();
        store.store("t2", "work.cruxx", &trace, 0).unwrap();
        store.store("t3", "work.cruxx", &trace, 1).unwrap();

        let filter = TraceFilter {
            pipeline: Some("work".into()),
            ..Default::default()
        };
        let results = store.list(&filter).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.pipeline.contains("work")));
    }

    #[test]
    fn list_respects_limit() {
        let (store, _tmp) = test_store();
        let trace = serde_json::json!({"steps": []});
        for i in 0..10 {
            store
                .store(&format!("t{i}"), "work.cruxx", &trace, 0)
                .unwrap();
        }

        let filter = TraceFilter {
            limit: Some(3),
            ..Default::default()
        };
        let results = store.list(&filter).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let (store, _tmp) = test_store();
        assert!(store.load("nonexistent").unwrap().is_none());
    }
}
```

- [ ] **Step 3: Register module in lib.rs**

Add to `crates/minibox-agent/src/lib.rs`:

```rust
pub mod trace;
pub use trace::FileTraceStore;
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p minibox-agent -- trace
```

Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-agent/src/trace.rs crates/minibox-agent/src/lib.rs crates/minibox-agent/Cargo.toml
git commit -m "feat(agent): add FileTraceStore adapter for pipeline traces"
```

---

## Phase 2: PipelineRunner + Daemon Handler

### Task 2.1: Minibox handler set (crux handler implementations)

**Files:**

- Create: `crates/minibox-agent/src/handlers.rs`
- Modify: `crates/minibox-agent/src/lib.rs`
- Modify: `crates/minibox-agent/Cargo.toml`

**Blocked by:** Phase 0 (`cruxx-script` and `cruxx-plugin` published)

- [ ] **Step 1: Add cruxx + minibox-client deps**

In `crates/minibox-agent/Cargo.toml`:

```toml
[dependencies]
minibox-client = { path = "../minibox-client" }
minibox-secrets = { path = "../minibox-secrets" }
cruxx-types = "0.1"      # or git dep
cruxx-script = "0.1"     # or git dep
```

- [ ] **Step 2: Write handler module skeleton**

Create `crates/minibox-agent/src/handlers.rs`:

```rust
//! Minibox handler set for crux pipeline `HandlerRegistry`.
//!
//! Each handler wraps a `DaemonClient` call and maps the result to a
//! crux step outcome. Handlers are registered under the `minibox::`
//! namespace.

use anyhow::{Context, Result};
use minibox_client::DaemonClient;
use minibox_core::protocol::DaemonRequest;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// Collection of minibox handlers for registration with a crux
/// `HandlerRegistry`.
pub struct MiniboxHandlers {
    client: Arc<DaemonClient>,
}

impl MiniboxHandlers {
    /// Create handlers connected to the daemon at the given socket.
    pub fn new(socket_path: PathBuf) -> Result<Self> {
        let client = Arc::new(
            DaemonClient::connect(&socket_path)
                .with_context(|| {
                    format!(
                        "connect to miniboxd: {}",
                        socket_path.display()
                    )
                })?,
        );
        Ok(Self { client })
    }

    /// Execute `minibox::container::run` — create and start a container.
    pub async fn container_run(&self, params: Value) -> Result<Value> {
        let image = params["image"]
            .as_str()
            .context("minibox::container::run requires 'image' field")?
            .to_string();
        let tag = params["tag"].as_str().map(String::from);
        let command: Vec<String> = params["command"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let req = DaemonRequest::Run {
            image,
            tag,
            command,
            memory_limit_bytes: params["memory_limit_bytes"].as_u64(),
            cpu_weight: params["cpu_weight"].as_u64(),
            ephemeral: true,
            network: None,
            env: vec![],
            mounts: vec![],
            privileged: false,
            name: None,
            tty: false,
        };

        let response = self.client.send_request(&req).await
            .context("minibox::container::run request failed")?;
        Ok(serde_json::to_value(&response)
            .context("serialize run response")?)
    }

    /// Execute `minibox::container::stop`.
    pub async fn container_stop(&self, params: Value) -> Result<Value> {
        let id = params["id"]
            .as_str()
            .context("minibox::container::stop requires 'id' field")?
            .to_string();

        let req = DaemonRequest::Stop { id };
        let response = self.client.send_request(&req).await
            .context("minibox::container::stop request failed")?;
        Ok(serde_json::to_value(&response)
            .context("serialize stop response")?)
    }

    /// Execute `minibox::container::rm`.
    pub async fn container_rm(&self, params: Value) -> Result<Value> {
        let id = params["id"]
            .as_str()
            .context("minibox::container::rm requires 'id' field")?
            .to_string();

        let req = DaemonRequest::Remove { id };
        let response = self.client.send_request(&req).await
            .context("minibox::container::rm request failed")?;
        Ok(serde_json::to_value(&response)
            .context("serialize rm response")?)
    }

    /// Execute `minibox::container::ps`.
    pub async fn container_ps(&self, _params: Value) -> Result<Value> {
        let req = DaemonRequest::List;
        let response = self.client.send_request(&req).await
            .context("minibox::container::ps request failed")?;
        Ok(serde_json::to_value(&response)
            .context("serialize ps response")?)
    }

    /// Execute `minibox::image::pull`.
    pub async fn image_pull(&self, params: Value) -> Result<Value> {
        let image = params["image"]
            .as_str()
            .context("minibox::image::pull requires 'image' field")?
            .to_string();
        let tag = params["tag"].as_str().map(String::from);

        let req = DaemonRequest::Pull { image, tag };
        let response = self.client.send_request(&req).await
            .context("minibox::image::pull request failed")?;
        Ok(serde_json::to_value(&response)
            .context("serialize pull response")?)
    }

    /// Execute `minibox::env::inject` — resolve secrets via minibox-secrets.
    pub async fn env_inject(&self, params: Value) -> Result<Value> {
        let keys: Vec<String> = params["keys"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Resolve via minibox-secrets CredentialProvider chain.
        let mut resolved = serde_json::Map::new();
        for key in &keys {
            // Delegate to minibox_secrets::resolve() when wired.
            // For now, read from environment as fallback.
            let value = std::env::var(key).unwrap_or_default();
            resolved.insert(key.clone(), Value::String(value));
        }
        Ok(Value::Object(resolved))
    }
}
```

- [ ] **Step 3: Register module**

Add to `crates/minibox-agent/src/lib.rs`:

```rust
pub mod handlers;
pub use handlers::MiniboxHandlers;
```

- [ ] **Step 4: Check compilation**

```bash
cargo check -p minibox-agent
```

This will fail if `cruxx-types`/`cruxx-script` are not yet available.
Gate with a `cfg` feature if needed for incremental development:

```toml
[features]
pipeline = ["cruxx-types", "cruxx-script"]
```

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-agent/
git commit -m "feat(agent): add MiniboxHandlers for crux pipeline registry"
```

---

### Task 2.2: PipelineRunner

**Files:**

- Create: `crates/minibox-agent/src/pipeline.rs`
- Modify: `crates/minibox-agent/src/lib.rs`

**Blocked by:** Phase 0 (`cruxx-script` API)

- [ ] **Step 1: Write PipelineRunner**

Create `crates/minibox-agent/src/pipeline.rs`:

```rust
//! Pipeline runner — loads `.cruxx` files and executes them with
//! minibox-specific handlers registered.

use anyhow::{Context, Result, bail};
use minibox_core::trace::TraceStore;
use std::path::{Path, PathBuf};

use crate::handlers::MiniboxHandlers;

/// Discovery order for `.cruxx` pipeline files.
const DISCOVERY_DIRS: &[&str] = &[
    ".cruxx/pipelines",   // project-specific
];

/// Loads and executes `.cruxx` pipelines with minibox handlers.
pub struct PipelineRunner {
    handlers: MiniboxHandlers,
    trace_store: Box<dyn TraceStore>,
    user_pipelines_dir: PathBuf,
}

impl PipelineRunner {
    /// Create a new runner with the given handlers and trace store.
    pub fn new(
        handlers: MiniboxHandlers,
        trace_store: Box<dyn TraceStore>,
    ) -> Self {
        let user_pipelines_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".cruxx/pipelines");

        Self {
            handlers,
            trace_store,
            user_pipelines_dir,
        }
    }

    /// Resolve a pipeline path using discovery order:
    /// 1. Explicit path (if absolute or exists relative to cwd)
    /// 2. `<workspace>/.cruxx/pipelines/<name>`
    /// 3. `~/.cruxx/pipelines/<name>`
    pub fn resolve_pipeline(&self, path: &str, workspace: Option<&Path>) -> Result<PathBuf> {
        let p = Path::new(path);

        // Explicit absolute path.
        if p.is_absolute() && p.exists() {
            return Ok(p.to_path_buf());
        }

        // Relative to cwd.
        if p.exists() {
            return Ok(p.canonicalize().context("canonicalize pipeline path")?);
        }

        // Project-specific.
        if let Some(ws) = workspace {
            for dir in DISCOVERY_DIRS {
                let candidate = ws.join(dir).join(path);
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }

        // User-global.
        let user_candidate = self.user_pipelines_dir.join(path);
        if user_candidate.exists() {
            return Ok(user_candidate);
        }

        bail!(
            "pipeline not found: '{}' (searched cwd, project .cruxx/, ~/.cruxx/)",
            path
        );
    }

    /// Execute a pipeline and store the resulting trace.
    ///
    /// Returns the serialized trace as JSON.
    ///
    /// NOTE: Full implementation requires `cruxx-script` crate. This is
    /// a structural placeholder that will be completed when Phase 0
    /// delivers the crux workspace prerequisites.
    pub async fn run(
        &self,
        pipeline_path: &Path,
        input: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        // TODO(phase-0): Replace with cruxx_script::load + execute.
        // For now, validate the path exists and return a stub trace.
        if !pipeline_path.exists() {
            bail!(
                "pipeline file not found: {}",
                pipeline_path.display()
            );
        }

        let trace_id = uuid::Uuid::new_v4().to_string();
        let pipeline_name = pipeline_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Stub trace structure matching Crux<T> serialization.
        let trace = serde_json::json!({
            "id": trace_id,
            "pipeline": pipeline_name,
            "input": input,
            "steps": [],
            "result": null,
            "budget_consumed": {"tokens": 0, "steps": 0},
        });

        // Store the trace.
        self.trace_store
            .store(&trace_id, &pipeline_name, &trace, 0)
            .with_context(|| {
                format!("store trace for pipeline: {}", pipeline_name)
            })?;

        Ok(trace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::FileTraceStore;
    use tempfile::TempDir;

    fn test_runner(tmp: &TempDir) -> PipelineRunner {
        let socket = PathBuf::from("/tmp/test-miniboxd.sock");
        // Handlers won't connect in tests — only testing discovery.
        // Use a struct that doesn't require live socket for unit tests.
        let handlers = MiniboxHandlers {
            client: std::sync::Arc::new(unsafe {
                // SAFETY: We never call client methods in discovery tests.
                std::mem::zeroed()
            }),
        };
        let store = Box::new(
            FileTraceStore::new(tmp.path().join("traces")).unwrap(),
        );
        PipelineRunner::new(handlers, store)
    }

    #[test]
    fn resolve_explicit_absolute_path() {
        let tmp = TempDir::new().unwrap();
        let runner = test_runner(&tmp);

        let pipeline = tmp.path().join("test.cruxx");
        std::fs::write(&pipeline, "version: 1").unwrap();

        let resolved = runner
            .resolve_pipeline(pipeline.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(resolved, pipeline);
    }

    #[test]
    fn resolve_project_discovery() {
        let tmp = TempDir::new().unwrap();
        let runner = test_runner(&tmp);

        let project_dir = tmp.path().join("project");
        let pipeline_dir = project_dir.join(".cruxx/pipelines");
        std::fs::create_dir_all(&pipeline_dir).unwrap();
        std::fs::write(pipeline_dir.join("work.cruxx"), "version: 1")
            .unwrap();

        let resolved = runner
            .resolve_pipeline("work.cruxx", Some(&project_dir))
            .unwrap();
        assert_eq!(resolved, pipeline_dir.join("work.cruxx"));
    }

    #[test]
    fn resolve_not_found_errors() {
        let tmp = TempDir::new().unwrap();
        let runner = test_runner(&tmp);

        let result = runner.resolve_pipeline("nonexistent.cruxx", None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("pipeline not found")
        );
    }
}
```

- [ ] **Step 2: Register module**

Add to `crates/minibox-agent/src/lib.rs`:

```rust
pub mod pipeline;
pub use pipeline::PipelineRunner;
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p minibox-agent -- pipeline
```

Note: The `test_runner` helper uses `mem::zeroed()` which is unsound —
replace with a proper mock client trait before merging. This is
acceptable for initial scaffolding.

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-agent/
git commit -m "feat(agent): add PipelineRunner with discovery logic"
```

---

### Task 2.3: handle_pipeline in daemonbox

**Files:**

- Modify: `crates/daemonbox/src/handler.rs`
- Modify: `crates/daemonbox/src/server.rs`
- Modify: `crates/daemonbox/Cargo.toml`

- [ ] **Step 1: Add minibox-core trace dep to daemonbox**

In `crates/daemonbox/Cargo.toml`, ensure `minibox-core` is a dependency
(it likely already is). No new crate deps needed — `handle_pipeline`
delegates to existing handler infrastructure.

- [ ] **Step 2: Add handler function skeleton**

In `crates/daemonbox/src/handler.rs`, add:

```rust
/// Handle a `RunPipeline` request.
///
/// Orchestrates: validate path → pull image → create container →
/// mount pipeline → execute `crux run` → stream output → store trace.
///
/// Enforces `max_depth` to prevent recursive container bombs.
#[instrument(skip(deps, state, tx))]
pub async fn handle_pipeline(
    pipeline_path: String,
    input: Option<serde_json::Value>,
    image: Option<String>,
    budget: Option<serde_json::Value>,
    env: Vec<(String, String)>,
    max_depth: u32,
    deps: &HandlerDependencies,
    state: Arc<TokioMutex<DaemonState>>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    // Depth gate: reject if max_depth == 0 (recursion exhausted).
    if max_depth == 0 {
        send_error(
            &tx,
            "handle_pipeline",
            "pipeline recursion depth exceeded (max_depth = 0)".into(),
        )
        .await;
        return;
    }

    // Validate pipeline_path — reject traversal attempts.
    let pipeline = std::path::Path::new(&pipeline_path);
    if pipeline_path.contains("..") {
        send_error(
            &tx,
            "handle_pipeline",
            format!(
                "pipeline path rejected: contains '..': {}",
                pipeline_path
            ),
        )
        .await;
        return;
    }

    if !pipeline.exists() {
        send_error(
            &tx,
            "handle_pipeline",
            format!("pipeline not found: {}", pipeline_path),
        )
        .await;
        return;
    }

    // TODO: Full implementation in Phase 2 completion:
    // 1. Pull image (default: cruxx-runtime:latest)
    // 2. Create container with overlay + pipeline mounted
    // 3. Build crux run command with --budget and minibox-crux-plugin
    // 4. Execute via handle_run_streaming infrastructure
    // 5. Collect trace from container output
    // 6. Send PipelineComplete response

    let image_name = image.unwrap_or_else(|| "cruxx-runtime".into());
    info!(
        pipeline = %pipeline_path,
        image = %image_name,
        max_depth,
        "pipeline: execution requested (stub)"
    );

    // Stub: return a PipelineComplete with empty trace.
    let trace = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "steps": [],
        "result": null,
    });

    if tx
        .send(DaemonResponse::PipelineComplete {
            trace,
            container_id: "stub-not-yet-implemented".into(),
            exit_code: 0,
        })
        .await
        .is_err()
    {
        warn!("handle_pipeline: client disconnected before completion");
    }
}
```

- [ ] **Step 3: Wire dispatch in server.rs**

In `crates/daemonbox/src/server.rs`, in the request dispatch match, add:

```rust
DaemonRequest::RunPipeline {
    pipeline_path,
    input,
    image,
    budget,
    env,
    max_depth,
} => {
    handle_pipeline(
        pipeline_path,
        input,
        image,
        budget,
        env,
        max_depth,
        &deps,
        Arc::clone(&state),
        tx,
    )
    .await;
}
```

- [ ] **Step 4: Check compilation**

```bash
cargo check -p daemonbox
```

- [ ] **Step 5: Commit**

```bash
git add crates/daemonbox/
git commit -m "feat(daemon): add handle_pipeline stub with depth gate"
```

---

## Phase 3: Plugin Binary

### Task 3.1: Create minibox-crux-plugin crate

**Files:**

- Create: `crates/minibox-crux-plugin/Cargo.toml`
- Create: `crates/minibox-crux-plugin/src/main.rs`
- Modify: `Cargo.toml` (workspace members)

**Blocked by:** Phase 0 (`cruxx-plugin` crate)

- [ ] **Step 1: Create crate directory**

```bash
mkdir -p crates/minibox-crux-plugin/src
```

- [ ] **Step 2: Write Cargo.toml**

Create `crates/minibox-crux-plugin/Cargo.toml`:

```toml
[package]
name = "minibox-crux-plugin"
version = "0.1.0"
edition = "2024"
license.workspace = true
description = "Crux plugin binary exposing minibox container ops via JSON-RPC"

[[bin]]
name = "minibox-crux-plugin"
path = "src/main.rs"

[dependencies]
minibox-client = { path = "../minibox-client" }
minibox-core = { path = "../minibox-core" }
minibox-secrets = { path = "../minibox-secrets" }
anyhow = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true, features = ["full"] }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

- [ ] **Step 3: Write main.rs**

Create `crates/minibox-crux-plugin/src/main.rs`:

```rust
//! minibox-crux-plugin — JSON-RPC plugin binary for crux pipelines.
//!
//! Implements the cruxx-plugin protocol: newline-delimited JSON over
//! stdin/stdout. Each request specifies a handler name and params;
//! this binary routes to `DaemonClient` operations.
//!
//! Protocol:
//! - Input (stdin): `{"id":"...","method":"minibox::container::run","params":{...}}\n`
//! - Output (stdout): `{"id":"...","result":{...}}\n` or `{"id":"...","error":"..."}\n`

use anyhow::{Context, Result};
use minibox_client::DaemonClient;
use minibox_core::protocol::DaemonRequest;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct RpcRequest {
    id: String,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn socket_path() -> PathBuf {
    std::env::var("MINIBOX_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/run/minibox/miniboxd.sock"))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter("minibox_crux_plugin=info")
        .init();

    let socket = socket_path();
    tracing::info!(socket = %socket.display(), "plugin: starting");

    let stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();

    for line in stdin.lines() {
        let line = line.context("read stdin line")?;
        if line.trim().is_empty() {
            continue;
        }

        let req: RpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = RpcResponse {
                    id: "unknown".into(),
                    result: None,
                    error: Some(format!("parse error: {e}")),
                };
                writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                stdout.flush()?;
                continue;
            }
        };

        let resp = handle_request(&socket, &req).await;
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }

    Ok(())
}

async fn handle_request(socket: &PathBuf, req: &RpcRequest) -> RpcResponse {
    match dispatch(socket, &req.method, &req.params).await {
        Ok(result) => RpcResponse {
            id: req.id.clone(),
            result: Some(result),
            error: None,
        },
        Err(e) => RpcResponse {
            id: req.id.clone(),
            result: None,
            error: Some(format!("{e:#}")),
        },
    }
}

async fn dispatch(
    socket: &PathBuf,
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let client = DaemonClient::connect(socket)
        .with_context(|| format!("connect: {}", socket.display()))?;

    let daemon_req = match method {
        "minibox::container::run" => DaemonRequest::Run {
            image: params["image"]
                .as_str()
                .context("missing 'image'")?
                .into(),
            tag: params["tag"].as_str().map(Into::into),
            command: params["command"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(Into::into))
                        .collect()
                })
                .unwrap_or_default(),
            memory_limit_bytes: params["memory_limit_bytes"].as_u64(),
            cpu_weight: params["cpu_weight"].as_u64(),
            ephemeral: params["ephemeral"].as_bool().unwrap_or(true),
            network: None,
            env: vec![],
            mounts: vec![],
            privileged: false,
            name: None,
            tty: false,
        },
        "minibox::container::stop" => DaemonRequest::Stop {
            id: params["id"]
                .as_str()
                .context("missing 'id'")?
                .into(),
        },
        "minibox::container::rm" => DaemonRequest::Remove {
            id: params["id"]
                .as_str()
                .context("missing 'id'")?
                .into(),
        },
        "minibox::container::ps" => DaemonRequest::List,
        "minibox::image::pull" => DaemonRequest::Pull {
            image: params["image"]
                .as_str()
                .context("missing 'image'")?
                .into(),
            tag: params["tag"].as_str().map(Into::into),
        },
        other => {
            anyhow::bail!("unknown method: {other}");
        }
    };

    let response = client
        .send_request(&daemon_req)
        .await
        .context("daemon request failed")?;
    serde_json::to_value(&response).context("serialize response")
}
```

- [ ] **Step 4: Add to workspace members**

In root `Cargo.toml`, add to `[workspace] members`:

```toml
"crates/minibox-crux-plugin",
```

- [ ] **Step 5: Check compilation**

```bash
cargo check -p minibox-crux-plugin
```

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-crux-plugin/ Cargo.toml
git commit -m "feat: add minibox-crux-plugin binary crate"
```

---

## Phase 4: CLI + TUI

### Task 4.1: CLI subcommands (run-pipeline, traces)

**Files:**

- Create: `crates/minibox-cli/src/commands/pipeline.rs`
- Create: `crates/minibox-cli/src/commands/traces.rs`
- Modify: `crates/minibox-cli/src/commands/mod.rs`
- Modify: `crates/minibox-cli/src/main.rs`

- [ ] **Step 1: Write pipeline command**

Create `crates/minibox-cli/src/commands/pipeline.rs`:

```rust
//! `minibox run-pipeline` subcommand.

use anyhow::{Context, Result};
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

pub async fn run_pipeline(
    client: &DaemonClient,
    pipeline_path: String,
    input_path: Option<String>,
) -> Result<()> {
    let input = if let Some(path) = input_path {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("read input: {path}"))?;
        Some(
            serde_json::from_str(&content)
                .with_context(|| format!("parse input JSON: {path}"))?,
        )
    } else {
        None
    };

    let req = DaemonRequest::RunPipeline {
        pipeline_path,
        input,
        image: None,
        budget: None,
        env: vec![],
        max_depth: 3,
    };

    let mut stream = client
        .send_request_stream(&req)
        .await
        .context("send RunPipeline")?;

    while let Some(response) = stream.next().await {
        match response? {
            DaemonResponse::ContainerOutput { stream: kind, data } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .unwrap_or_default();
                let out = String::from_utf8_lossy(&bytes);
                eprint!("{out}");
            }
            DaemonResponse::PipelineComplete {
                trace,
                container_id,
                exit_code,
            } => {
                eprintln!(
                    "\n--- pipeline complete (container={}, exit={})",
                    container_id, exit_code
                );
                if exit_code != 0 {
                    std::process::exit(exit_code);
                }
                // Print trace summary.
                let steps = trace["steps"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                eprintln!("    trace: {} steps", steps);
                return Ok(());
            }
            DaemonResponse::Error { message } => {
                anyhow::bail!("daemon error: {message}");
            }
            _ => {}
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Write traces command**

Create `crates/minibox-cli/src/commands/traces.rs`:

```rust
//! `minibox traces` subcommands (list, show).

use anyhow::{Context, Result};
use minibox_core::trace::{FileTraceStore, TraceFilter, TraceStore};

pub fn list_traces(since: Option<String>) -> Result<()> {
    let store = minibox_agent::FileTraceStore::new(
        minibox_agent::FileTraceStore::default_dir(),
    )?;

    let filter = TraceFilter {
        since,
        limit: Some(20),
        ..Default::default()
    };

    let traces = store.list(&filter)?;

    if traces.is_empty() {
        println!("No traces found.");
        return Ok(());
    }

    println!(
        "{:<38} {:<20} {:<6} {:<5}",
        "ID", "PIPELINE", "EXIT", "STEPS"
    );
    for t in &traces {
        println!(
            "{:<38} {:<20} {:<6} {:<5}",
            t.id, t.pipeline, t.exit_code, t.step_count
        );
    }

    Ok(())
}

pub fn show_trace(id: String) -> Result<()> {
    let store = minibox_agent::FileTraceStore::new(
        minibox_agent::FileTraceStore::default_dir(),
    )?;

    match store.load(&id)? {
        Some(trace) => {
            println!("{}", serde_json::to_string_pretty(&trace)?);
        }
        None => {
            anyhow::bail!("trace not found: {id}");
        }
    }

    Ok(())
}
```

- [ ] **Step 3: Register modules and wire into clap**

Add to `crates/minibox-cli/src/commands/mod.rs`:

```rust
pub mod pipeline;
pub mod traces;
```

Wire into `main.rs` clap subcommands (follow existing pattern for
`run`, `pull`, etc.). Add:

```rust
Command::RunPipeline { path, input } => {
    commands::pipeline::run_pipeline(&client, path, input).await?;
}
Command::TracesList { since } => {
    commands::traces::list_traces(since)?;
}
Command::TracesShow { id } => {
    commands::traces::show_trace(id)?;
}
```

- [ ] **Step 4: Check compilation**

```bash
cargo check -p minibox-cli
```

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-cli/
git commit -m "feat(cli): add run-pipeline and traces subcommands"
```

---

### Task 4.2: Dashbox Traces tab

**Files:**

- Create: `crates/dashbox/src/tabs/traces.rs` (or equivalent per
  existing tab pattern)
- Modify: `crates/dashbox/src/` (tab registration)

- [ ] **Step 1: Study existing tab pattern**

Read the existing dashbox tab module structure (e.g., `agents.rs` or
`bench.rs`) to understand the rendering pattern.

- [ ] **Step 2: Create traces tab**

Follow the established pattern. The tab should:

- Read `~/.minibox/traces/` via `FileTraceStore::list()`
- Display a table of recent traces (id, pipeline, timestamp, exit,
  steps)
- On selection, show full trace JSON with step drill-down

- [ ] **Step 3: Register tab**

Add to dashbox's tab enum/vector.

- [ ] **Step 4: Run `cargo check -p dashbox`**

- [ ] **Step 5: Commit**

```bash
git add crates/dashbox/
git commit -m "feat(dashbox): add Traces tab for pipeline trace viewing"
```

---

## Phase 5: Multi-Container + VPS Validation

### Task 5.1: Container group lifecycle

**Files:**

- Modify: `crates/daemonbox/src/state.rs`
- Modify: `crates/daemonbox/src/handler.rs`

- [ ] **Step 1: Add group_id field to ContainerRecord**

In `crates/daemonbox/src/state.rs`, add to `ContainerRecord`:

```rust
/// Pipeline group ID. Containers in the same group are reaped together
/// when the parent pipeline exits or times out.
#[serde(default)]
pub group_id: Option<String>,
```

- [ ] **Step 2: Add reap_group method to DaemonState**

```rust
/// Stop and remove all containers belonging to a group.
pub async fn reap_group(&mut self, group_id: &str) -> Vec<String> {
    let ids: Vec<String> = self
        .containers
        .iter()
        .filter(|(_, r)| r.group_id.as_deref() == Some(group_id))
        .map(|(id, _)| id.clone())
        .collect();
    // Caller is responsible for actually stopping processes.
    ids
}
```

- [ ] **Step 3: Wire group cleanup in handle_pipeline error paths**

When `handle_pipeline` fails or the parent exits, call `reap_group`
with the pipeline's group_id to clean up child containers.

- [ ] **Step 4: Tests**

Write a unit test in handler_tests.rs that:

1. Creates a state with 3 containers sharing a group_id
2. Calls `reap_group`
3. Verifies all 3 are returned for cleanup

- [ ] **Step 5: Commit**

```bash
git add crates/daemonbox/
git commit -m "feat(daemon): add container group lifecycle for pipeline cleanup"
```

---

### Task 5.2: VPS e2e validation

**Files:**

- Create: `tests/e2e/pipeline_e2e.rs` (or extend existing e2e suite)

- [ ] **Step 1: Write e2e test**

Test on VPS (`MINIBOX_ADAPTER=native`):

1. Create a trivial `.cruxx` pipeline file
2. Send `RunPipeline` request to daemon
3. Verify `PipelineComplete` response with valid trace
4. Verify trace is stored in `~/.minibox/traces/`

- [ ] **Step 2: Run on VPS**

```bash
ssh minibox 'cd /opt/minibox && cargo xtask test-e2e-suite'
```

- [ ] **Step 3: Commit test**

```bash
git add tests/
git commit -m "test(e2e): add pipeline execution e2e test"
```

---

## Dependency Graph

```
Phase 0 (crux workspace) ─────────────────────────────────────┐
                                                               │
Phase 1.1 (RunPipeline protocol) ──► Phase 1.2 (PipelineComplete) │
Phase 1.3 (TraceStore trait) ──► Phase 1.4 (FileTraceStore)    │
                                                               │
Phase 2.1 (handlers) ◄────────────────────────────────────────┘
Phase 2.2 (PipelineRunner) ◄── Phase 2.1
Phase 2.3 (handle_pipeline) ◄── Phase 2.2 + Phase 1.1 + 1.2
                                   │
Phase 3.1 (plugin binary) ◄───────┘
                                   │
Phase 4.1 (CLI) ◄─── Phase 1.2 + 1.4
Phase 4.2 (dashbox) ◄── Phase 1.4
                                   │
Phase 5.1 (group lifecycle) ◄──────┘
Phase 5.2 (VPS e2e) ◄── all above
```

## Risk Register

| Risk                                            | Impact           | Mitigation                                                   |
| ----------------------------------------------- | ---------------- | ------------------------------------------------------------ |
| Phase 0 blocked (crux crates unpublished)       | Blocks Phase 2+  | Use git deps with pinned rev                                 |
| `DaemonClient` API changes needed for streaming | Delays Phase 3   | Plugin can use blocking client initially                     |
| `cruxx-plugin` protocol changes                 | Rework Phase 3   | Pin to specific version, don't over-abstract                 |
| Budget enforcement is client-side only          | Security gap     | Document as known limitation; daemon-side tokens are Phase 6 |
| Multi-container orphans on OOM/SIGKILL          | Leaked resources | Group reaper + daemon-side timeout watchdog                  |
