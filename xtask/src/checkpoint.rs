//! Gate checkpoint system — skip gates that already passed for the current code state.
//!
//! ## Architecture
//!
//! **Domain types:** `GateId`, `CheckpointRecord`, `CheckpointResult`
//! **Ports (traits):** `TreeHasher`, `CheckpointStore`
//! **Adapters:** `GitTreeProbe`, `FileCheckpointStore`
//!
//! A checkpoint is valid when the stored tree hash matches the current workspace
//! state (tracked content + no dirty files). Platform and rustc version are also
//! compared so cross-platform or toolchain-upgrade runs aren't skipped.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Known quality gates that can be checkpointed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GateId {
    Fmt,
    Clippy,
    Lint,
    Verify,
    PreCommit,
    Prepush,
    TestUnit,
    TestConformance,
    TestProperty,
    TestQuickcheck,
    TestTurmoil,
    TestShuttle,
    TestE2e,
    TestIntegration,
    TestSystemSuite,
    TestSandbox,
    BorrowFixtures,
    DocsLint,
}

impl fmt::Display for GateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{self:?}"));
        f.write_str(&s)
    }
}

/// Stored proof that a gate passed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub gate: GateId,
    pub tree_hash: String,
    pub platform: String,
    pub rustc_version: String,
    pub timestamp: DateTime<Utc>,
}

/// Result of checking whether a checkpoint is still valid.
#[derive(Debug)]
pub enum CheckpointResult {
    /// Checkpoint exists and matches current state — gate can be skipped.
    Valid(CheckpointRecord),
    /// No valid checkpoint — gate must run.
    Stale(String),
}

// ---------------------------------------------------------------------------
// Ports (traits)
// ---------------------------------------------------------------------------

/// Probe the current workspace content identity.
pub trait TreeHasher {
    /// Content hash of tracked files (e.g. `git rev-parse HEAD^{tree}`).
    fn tree_hash(&self) -> Result<String>;

    /// True if the working tree has uncommitted changes to tracked files.
    fn is_dirty(&self) -> Result<bool>;
}

/// Persistent storage for checkpoint records.
pub trait CheckpointStore {
    fn load(&self, gate: GateId) -> Result<Option<CheckpointRecord>>;
    fn save(&self, record: &CheckpointRecord) -> Result<()>;
    fn clear(&self, gate: GateId) -> Result<()>;
    fn clear_all(&self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Adapter: GitTreeProbe
// ---------------------------------------------------------------------------

/// Uses git to determine workspace content identity.
pub struct GitTreeProbe;

impl TreeHasher for GitTreeProbe {
    fn tree_hash(&self) -> Result<String> {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD^{tree}"])
            .output()
            .context("failed to run git rev-parse")?;
        if !output.status.success() {
            anyhow::bail!(
                "git rev-parse HEAD^{{tree}} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn is_dirty(&self) -> Result<bool> {
        let output = std::process::Command::new("git")
            .args(["diff", "--stat"])
            .output()
            .context("failed to run git diff")?;
        let tracked_dirty = !output.stdout.is_empty();

        // Also check staged-but-uncommitted changes.
        let staged = std::process::Command::new("git")
            .args(["diff", "--cached", "--stat"])
            .output()
            .context("failed to run git diff --cached")?;
        let staged_dirty = !staged.stdout.is_empty();

        Ok(tracked_dirty || staged_dirty)
    }
}

// ---------------------------------------------------------------------------
// Adapter: FileCheckpointStore
// ---------------------------------------------------------------------------

/// Stores checkpoint records as JSON files in a directory.
pub struct FileCheckpointStore {
    dir: PathBuf,
}

impl FileCheckpointStore {
    #[must_use]
    pub const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Default location: `<workspace_root>/.minibox/checkpoints/`
    #[must_use]
    pub fn default_for_workspace(root: &Path) -> Self {
        Self::new(root.join(".minibox").join("checkpoints"))
    }

    fn path_for(&self, gate: GateId) -> PathBuf {
        self.dir.join(format!("{gate}.json"))
    }
}

impl CheckpointStore for FileCheckpointStore {
    fn load(&self, gate: GateId) -> Result<Option<CheckpointRecord>> {
        let path = self.path_for(gate);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("reading checkpoint {}", path.display()))?;
        let record: CheckpointRecord = serde_json::from_str(&content)
            .with_context(|| format!("parsing checkpoint {}", path.display()))?;
        Ok(Some(record))
    }

    fn save(&self, record: &CheckpointRecord) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating checkpoint dir {}", self.dir.display()))?;
        let path = self.path_for(record.gate);
        let json = serde_json::to_string_pretty(record).context("serializing checkpoint")?;
        std::fs::write(&path, json)
            .with_context(|| format!("writing checkpoint {}", path.display()))?;
        Ok(())
    }

    fn clear(&self, gate: GateId) -> Result<()> {
        let path = self.path_for(gate);
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("removing checkpoint {}", path.display()))?;
        }
        Ok(())
    }

    fn clear_all(&self) -> Result<()> {
        if self.dir.exists() {
            std::fs::remove_dir_all(&self.dir)
                .with_context(|| format!("removing checkpoint dir {}", self.dir.display()))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Domain logic
// ---------------------------------------------------------------------------

fn current_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn current_rustc_version() -> String {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .map_or_else(
            || "unknown".to_string(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
}

/// Check whether a gate's checkpoint is still valid.
pub fn check(
    gate: GateId,
    hasher: &dyn TreeHasher,
    store: &dyn CheckpointStore,
) -> Result<CheckpointResult> {
    let record = match store.load(gate)? {
        Some(r) => r,
        None => return Ok(CheckpointResult::Stale("no checkpoint found".into())),
    };

    if hasher.is_dirty()? {
        return Ok(CheckpointResult::Stale("working tree is dirty".into()));
    }

    let current_hash = hasher.tree_hash()?;
    if record.tree_hash != current_hash {
        return Ok(CheckpointResult::Stale(format!(
            "tree hash changed: {} -> {}",
            &record.tree_hash[..8.min(record.tree_hash.len())],
            &current_hash[..8.min(current_hash.len())],
        )));
    }

    let platform = current_platform();
    if record.platform != platform {
        return Ok(CheckpointResult::Stale(format!(
            "platform changed: {} -> {platform}",
            record.platform
        )));
    }

    let rustc = current_rustc_version();
    if record.rustc_version != rustc {
        return Ok(CheckpointResult::Stale(format!(
            "rustc changed: {} -> {rustc}",
            record.rustc_version
        )));
    }

    Ok(CheckpointResult::Valid(record))
}

/// Record that a gate passed for the current state.
pub fn record(gate: GateId, hasher: &dyn TreeHasher, store: &dyn CheckpointStore) -> Result<()> {
    let tree_hash = hasher.tree_hash()?;
    let record = CheckpointRecord {
        gate,
        tree_hash,
        platform: current_platform(),
        rustc_version: current_rustc_version(),
        timestamp: Utc::now(),
    };
    store.save(&record)
}

/// Returns true if `MINIBOX_FORCE_GATES` is set or `--force` is in argv.
#[must_use]
pub fn force_requested() -> bool {
    if std::env::var("MINIBOX_FORCE_GATES").is_ok() {
        return true;
    }
    std::env::args().any(|a| a == "--force")
}

/// Run a gate with checkpoint logic. Skips if a valid checkpoint exists
/// (unless force is requested). Records a checkpoint on success.
pub fn run_gated<F>(
    gate: GateId,
    hasher: &dyn TreeHasher,
    store: &dyn CheckpointStore,
    f: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    if !force_requested() {
        match check(gate, hasher, store)? {
            CheckpointResult::Valid(r) => {
                eprintln!(
                    "checkpoint: {gate} skipped (passed at {} for {})",
                    r.timestamp.format("%H:%M:%S"),
                    &r.tree_hash[..8.min(r.tree_hash.len())],
                );
                return Ok(());
            }
            CheckpointResult::Stale(reason) => {
                eprintln!("checkpoint: {gate} must run ({reason})");
            }
        }
    }

    f()?;

    // Only record if the tree is clean — dirty checkpoints are meaningless.
    if hasher.is_dirty().unwrap_or(true) {
        eprintln!("checkpoint: {gate} passed but tree is dirty, not recording");
    } else {
        record(gate, hasher, store)?;
        eprintln!("checkpoint: {gate} recorded");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeHasher {
        hash: String,
        dirty: bool,
    }

    impl TreeHasher for FakeHasher {
        fn tree_hash(&self) -> Result<String> {
            Ok(self.hash.clone())
        }
        fn is_dirty(&self) -> Result<bool> {
            Ok(self.dirty)
        }
    }

    struct MemoryStore {
        records: RefCell<std::collections::HashMap<GateId, CheckpointRecord>>,
    }

    impl MemoryStore {
        fn new() -> Self {
            Self {
                records: RefCell::new(std::collections::HashMap::new()),
            }
        }
    }

    impl CheckpointStore for MemoryStore {
        fn load(&self, gate: GateId) -> Result<Option<CheckpointRecord>> {
            Ok(self.records.borrow().get(&gate).cloned())
        }
        fn save(&self, record: &CheckpointRecord) -> Result<()> {
            self.records
                .borrow_mut()
                .insert(record.gate, record.clone());
            Ok(())
        }
        fn clear(&self, gate: GateId) -> Result<()> {
            self.records.borrow_mut().remove(&gate);
            Ok(())
        }
        fn clear_all(&self) -> Result<()> {
            self.records.borrow_mut().clear();
            Ok(())
        }
    }

    #[test]
    fn stale_when_no_checkpoint() {
        let hasher = FakeHasher {
            hash: "abc123".into(),
            dirty: false,
        };
        let store = MemoryStore::new();
        let result = check(GateId::Fmt, &hasher, &store).unwrap();
        assert!(matches!(result, CheckpointResult::Stale(_)));
    }

    #[test]
    fn valid_when_hash_matches() {
        let hasher = FakeHasher {
            hash: "abc123".into(),
            dirty: false,
        };
        let store = MemoryStore::new();
        store
            .save(&CheckpointRecord {
                gate: GateId::Fmt,
                tree_hash: "abc123".into(),
                platform: current_platform(),
                rustc_version: current_rustc_version(),
                timestamp: Utc::now(),
            })
            .unwrap();
        let result = check(GateId::Fmt, &hasher, &store).unwrap();
        assert!(matches!(result, CheckpointResult::Valid(_)));
    }

    #[test]
    fn stale_when_dirty() {
        let hasher = FakeHasher {
            hash: "abc123".into(),
            dirty: true,
        };
        let store = MemoryStore::new();
        store
            .save(&CheckpointRecord {
                gate: GateId::Fmt,
                tree_hash: "abc123".into(),
                platform: current_platform(),
                rustc_version: current_rustc_version(),
                timestamp: Utc::now(),
            })
            .unwrap();
        let result = check(GateId::Fmt, &hasher, &store).unwrap();
        assert!(matches!(result, CheckpointResult::Stale(_)));
    }

    #[test]
    fn stale_when_hash_differs() {
        let hasher = FakeHasher {
            hash: "def456".into(),
            dirty: false,
        };
        let store = MemoryStore::new();
        store
            .save(&CheckpointRecord {
                gate: GateId::Fmt,
                tree_hash: "abc123".into(),
                platform: current_platform(),
                rustc_version: current_rustc_version(),
                timestamp: Utc::now(),
            })
            .unwrap();
        let result = check(GateId::Fmt, &hasher, &store).unwrap();
        assert!(matches!(result, CheckpointResult::Stale(_)));
    }

    #[test]
    fn stale_when_platform_differs() {
        let hasher = FakeHasher {
            hash: "abc123".into(),
            dirty: false,
        };
        let store = MemoryStore::new();
        store
            .save(&CheckpointRecord {
                gate: GateId::Fmt,
                tree_hash: "abc123".into(),
                platform: "other-platform".into(),
                rustc_version: current_rustc_version(),
                timestamp: Utc::now(),
            })
            .unwrap();
        let result = check(GateId::Fmt, &hasher, &store).unwrap();
        assert!(matches!(result, CheckpointResult::Stale(_)));
    }

    #[test]
    fn run_gated_skips_valid() {
        let hasher = FakeHasher {
            hash: "abc123".into(),
            dirty: false,
        };
        let store = MemoryStore::new();
        store
            .save(&CheckpointRecord {
                gate: GateId::Lint,
                tree_hash: "abc123".into(),
                platform: current_platform(),
                rustc_version: current_rustc_version(),
                timestamp: Utc::now(),
            })
            .unwrap();

        let ran = RefCell::new(false);
        run_gated(GateId::Lint, &hasher, &store, || {
            *ran.borrow_mut() = true;
            Ok(())
        })
        .unwrap();
        assert!(!*ran.borrow(), "gate should have been skipped");
    }

    #[test]
    fn run_gated_records_on_success() {
        let hasher = FakeHasher {
            hash: "abc123".into(),
            dirty: false,
        };
        let store = MemoryStore::new();

        run_gated(GateId::Lint, &hasher, &store, || Ok(())).unwrap();
        assert!(store.load(GateId::Lint).unwrap().is_some());
    }

    #[test]
    fn run_gated_no_record_when_dirty() {
        let hasher = FakeHasher {
            hash: "abc123".into(),
            dirty: true,
        };
        let store = MemoryStore::new();

        run_gated(GateId::Lint, &hasher, &store, || Ok(())).unwrap();
        assert!(store.load(GateId::Lint).unwrap().is_none());
    }
}
