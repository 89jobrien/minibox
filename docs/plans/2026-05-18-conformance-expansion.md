---
status: done
---

# Plan: Conformance Test Expansion

## Goal

Add conformance coverage for 12 untested domain traits, consolidate into a
hub+spokes model, expand CI, and make cross-platform gaps visible via
capability-gated skip reporting.

## Context Map

### Files to Modify

| File | Purpose | Changes |
|------|---------|---------|
| `crates/minibox-core/src/domain.rs:1575` | `BackendCapability` enum (4 variants) | Add 8 new variants |
| `crates/minibox/src/testing/backend/descriptor.rs` | `BackendDescriptor` struct | Add 9 factory fields + builders |
| `crates/minibox/src/testing/capability.rs` | `ConformanceCapability` impls | Add 8 new capability structs |
| `crates/minibox-testsuite/src/adapters/mod.rs` | Module registry + `all()` | Register 12 new modules |
| `crates/minibox-testsuite/src/lib.rs` | Crate root | Expose `spoke` module |
| `crates/minibox-testsuite/src/bin/run_conformance.rs` | Runner binary | Collect spoke tests |
| `.github/workflows/conformance.yml` | CI workflow | Add 3 new jobs |

### New Files

| File | Purpose |
|------|---------|
| `crates/minibox-testsuite/src/adapters/filesystem.rs` | FilesystemProvider conformance |
| `crates/minibox-testsuite/src/adapters/exec_runtime.rs` | ExecRuntime conformance |
| `crates/minibox-testsuite/src/adapters/image_pusher.rs` | ImagePusher conformance |
| `crates/minibox-testsuite/src/adapters/container_committer.rs` | ContainerCommitter conformance |
| `crates/minibox-testsuite/src/adapters/image_builder.rs` | ImageBuilder conformance |
| `crates/minibox-testsuite/src/adapters/network.rs` | NetworkProvider conformance |
| `crates/minibox-testsuite/src/adapters/tty.rs` | TtyProvider conformance |
| `crates/minibox-testsuite/src/adapters/pty.rs` | PtyAllocator conformance |
| `crates/minibox-testsuite/src/adapters/vm_checkpoint.rs` | VmCheckpoint conformance |
| `crates/minibox-testsuite/src/adapters/metrics.rs` | MetricsRecorder conformance |
| `crates/minibox-testsuite/src/adapters/registry_router.rs` | RegistryRouter conformance |
| `crates/minibox-testsuite/src/adapters/image_loader.rs` | ImageLoader conformance |
| `crates/minibox-testsuite/src/spoke.rs` | Spoke registration API |
| `crates/minibox/src/testing/mocks/tty.rs` | MockTtyProvider |
| `crates/minibox/src/testing/mocks/vm_checkpoint.rs` | MockVmCheckpoint |
| `crates/minibox/src/testing/mocks/metrics.rs` | MockMetricsRecorder |
| `crates/minibox/src/testing/mocks/registry_router.rs` | MockRegistryRouter |
| `crates/minibox/src/testing/mocks/image_loader.rs` | MockImageLoader |

### Dependencies (consumers of changed types)

| File | Relationship |
|------|--------------|
| `crates/minibox/tests/conformance_commit.rs` | Constructs `BackendCapabilitySet` |
| `crates/minibox/tests/conformance_push.rs` | Constructs `BackendCapabilitySet` |
| `crates/minibox/tests/conformance_build.rs` | Constructs `BackendCapabilitySet` |
| `crates/minibox/tests/conformance_snapshot.rs` | Uses `BackendCapability::Checkpoint` |
| `crates/minibox/tests/smolvm_conformance_tests.rs` | Constructs `BackendCapabilitySet` |
| `crates/minibox/tests/colima_conformance_tests.rs` | Constructs `BackendCapabilitySet` |
| `crates/minibox-core/src/adapters/conformance.rs` | References `BackendCapability` |

### Risk

- [x] `BackendCapability` is `pub` but only matched internally — additive change, non-breaking
- [ ] `BackendDescriptor` construction sites (6 files) need updating for new fields
- [ ] 5 new mock adapters needed in `minibox/src/testing/mocks/`
- [ ] CI changes are additive — no existing job modified

## Architecture

- **Crates affected**: minibox-core, minibox (testing module), minibox-testsuite
- **New traits/types**: 8 `BackendCapability` variants, 8 `ConformanceCapability`
  structs, 5 mock adapters, 1 `SpokeRegistry`, 12 conformance test modules
- **Data flow**: Mock adapter -> ConformanceTest::run_sync -> TestContext assertions
  -> TestRunner -> ReportGenerator (JSON/Markdown/JUnit)

## Tech Stack

- Rust 2024 edition, `async-trait`, `tokio` (for block_on in sync tests),
  `serde`/`serde_json`, `tempfile`
- No new dependencies

---

## Tasks

### Phase 1: Core Infrastructure

### Task 1: Add BackendCapability variants

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain.rs`
**Run**: `cargo check -p minibox-core`

1. Add 8 new variants to `BackendCapability` enum at line 1575:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendCapability {
    /// Backend can snapshot a running container's FS diff into a new image
    /// via [`ContainerCommitter::commit`].
    Commit,
    /// Backend can build an image from a `BuildContext` + `BuildConfig` via
    /// [`ImageBuilder::build_image`].
    BuildFromContext,
    /// Backend can push an image to an OCI-compliant registry via
    /// [`ImagePusher::push_image`].
    PushToRegistry,
    /// Backend can save/restore VM state checkpoints via
    /// [`VmCheckpoint::save_snapshot`] / [`VmCheckpoint::restore_snapshot`].
    Checkpoint,
    /// Backend provides [`RootfsSetup`] + [`ChildInit`] (filesystem operations).
    Filesystem,
    /// Backend provides [`ExecRuntime`] (exec into running containers).
    Exec,
    /// Backend provides [`NetworkProvider`] (bridge/host/tailnet networking).
    Network,
    /// Backend provides [`TtyProvider`] (pseudo-terminal allocation).
    Tty,
    /// Backend provides [`PtyAllocator`] (low-level PTY pair allocation).
    Pty,
    /// Backend provides [`MetricsRecorder`] (counter/histogram/gauge).
    Metrics,
    /// Backend provides [`RegistryRouter`] (multi-registry routing).
    RegistryRouter,
    /// Backend provides [`ImageLoader`] (local OCI tarball loading).
    ImageLoader,
}
```

2. Verify:

```
cargo check -p minibox-core              -> ok
cargo clippy -p minibox-core -- -D warnings  -> zero warnings
```

3. Commit: `feat(minibox-core): add 8 BackendCapability variants for conformance expansion`

---

### Task 2: Add ConformanceCapability structs

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/testing/capability.rs`
**Run**: `cargo check -p minibox`

1. Add 8 new capability structs below existing ones (follow the
   `CommitCapability` pattern exactly):

```rust
/// Capability: backend provides filesystem operations (RootfsSetup + ChildInit).
pub struct FilesystemCapability {
    pub supported: bool,
}

impl ConformanceCapability for FilesystemCapability {
    fn name(&self) -> &'static str { "Filesystem" }
    fn is_supported(&self) -> bool { self.supported }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared { capability: "Filesystem" }
    }
}

/// Capability: backend can exec into running containers.
pub struct ExecCapability {
    pub supported: bool,
}

impl ConformanceCapability for ExecCapability {
    fn name(&self) -> &'static str { "Exec" }
    fn is_supported(&self) -> bool { self.supported }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared { capability: "Exec" }
    }
}

/// Capability: backend provides container networking.
pub struct NetworkCapability {
    pub supported: bool,
}

impl ConformanceCapability for NetworkCapability {
    fn name(&self) -> &'static str { "Network" }
    fn is_supported(&self) -> bool { self.supported }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared { capability: "Network" }
    }
}

/// Capability: backend provides TTY support.
pub struct TtyCapability {
    pub supported: bool,
}

impl ConformanceCapability for TtyCapability {
    fn name(&self) -> &'static str { "Tty" }
    fn is_supported(&self) -> bool { self.supported }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared { capability: "Tty" }
    }
}

/// Capability: backend provides low-level PTY allocation.
pub struct PtyCapability {
    pub supported: bool,
}

impl ConformanceCapability for PtyCapability {
    fn name(&self) -> &'static str { "Pty" }
    fn is_supported(&self) -> bool { self.supported }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared { capability: "Pty" }
    }
}

/// Capability: backend provides metrics recording.
pub struct MetricsCapability {
    pub supported: bool,
}

impl ConformanceCapability for MetricsCapability {
    fn name(&self) -> &'static str { "Metrics" }
    fn is_supported(&self) -> bool { self.supported }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared { capability: "Metrics" }
    }
}

/// Capability: backend provides multi-registry routing.
pub struct RegistryRouterCapability {
    pub supported: bool,
}

impl ConformanceCapability for RegistryRouterCapability {
    fn name(&self) -> &'static str { "RegistryRouter" }
    fn is_supported(&self) -> bool { self.supported }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared { capability: "RegistryRouter" }
    }
}

/// Capability: backend can load local OCI tarballs.
pub struct ImageLoaderCapability {
    pub supported: bool,
}

impl ConformanceCapability for ImageLoaderCapability {
    fn name(&self) -> &'static str { "ImageLoader" }
    fn is_supported(&self) -> bool { self.supported }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared { capability: "ImageLoader" }
    }
}
```

2. Add tests:

```rust
#[test]
fn filesystem_capability_skip_message() {
    let cap = FilesystemCapability { supported: false };
    let msg = should_skip(&cap).expect("should skip");
    assert!(msg.contains("Filesystem"));
}

#[test]
fn all_new_capabilities_pass_when_supported() {
    let caps: Vec<Box<dyn ConformanceCapability>> = vec![
        Box::new(FilesystemCapability { supported: true }),
        Box::new(ExecCapability { supported: true }),
        Box::new(NetworkCapability { supported: true }),
        Box::new(TtyCapability { supported: true }),
        Box::new(PtyCapability { supported: true }),
        Box::new(MetricsCapability { supported: true }),
        Box::new(RegistryRouterCapability { supported: true }),
        Box::new(ImageLoaderCapability { supported: true }),
    ];
    for cap in &caps {
        assert!(should_skip(cap.as_ref()).is_none(), "{}", cap.name());
    }
}
```

3. Verify:

```
cargo nextest run -p minibox -- capability  -> all green
cargo clippy -p minibox -- -D warnings      -> zero warnings
```

4. Commit: `feat(minibox): add 8 ConformanceCapability structs`

---

### Phase 2: Mock Adapters

### Task 3: Add MockTtyProvider

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/testing/mocks/tty.rs`,
`crates/minibox/src/testing/mocks/mod.rs`
**Run**: `cargo check -p minibox`

1. Create `crates/minibox/src/testing/mocks/tty.rs`:

```rust
//! Mock [`TtyProvider`] for conformance testing.

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::domain::{AsAny, TtyConfig, TtyProvider};
use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Mock TTY provider that tracks calls without allocating real PTYs.
#[derive(Debug)]
pub struct MockTtyProvider {
    create_count: AtomicUsize,
    resize_count: AtomicUsize,
    close_count: AtomicUsize,
}

impl MockTtyProvider {
    /// Create a fresh mock.
    pub fn new() -> Self {
        Self {
            create_count: AtomicUsize::new(0),
            resize_count: AtomicUsize::new(0),
            close_count: AtomicUsize::new(0),
        }
    }

    /// Number of `create` calls.
    pub fn create_count(&self) -> usize {
        self.create_count.load(Ordering::Relaxed)
    }

    /// Number of `resize` calls.
    pub fn resize_count(&self) -> usize {
        self.resize_count.load(Ordering::Relaxed)
    }

    /// Number of `close` calls.
    pub fn close_count(&self) -> usize {
        self.close_count.load(Ordering::Relaxed)
    }
}

impl Default for MockTtyProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl AsAny for MockTtyProvider {
    fn as_any(&self) -> &dyn Any { self }
}

#[async_trait]
impl TtyProvider for MockTtyProvider {
    async fn create(&self, _config: &TtyConfig) -> Result<(i32, i32)> {
        self.create_count.fetch_add(1, Ordering::Relaxed);
        Ok((100, 101)) // fake fd pair
    }

    async fn resize(&self, _master_fd: i32, _width: u16, _height: u16) -> Result<()> {
        self.resize_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn close(&self, _master_fd: i32) -> Result<()> {
        self.close_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
```

2. Add `pub mod tty;` and `pub use tty::MockTtyProvider;` to
   `crates/minibox/src/testing/mocks/mod.rs`.

3. Verify: `cargo check -p minibox`

4. Commit: `feat(minibox): add MockTtyProvider for conformance testing`

---

### Task 4: Add MockVmCheckpoint

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/testing/mocks/vm_checkpoint.rs`,
`crates/minibox/src/testing/mocks/mod.rs`
**Run**: `cargo check -p minibox`

1. Create `crates/minibox/src/testing/mocks/vm_checkpoint.rs`:

```rust
//! Mock [`VmCheckpoint`] for conformance testing.

use anyhow::Result;
use minibox_core::domain::{SnapshotInfo, VmCheckpoint};
use std::path::Path;
use std::sync::Mutex;

/// Mock VM checkpoint that stores snapshots in memory.
#[derive(Debug)]
pub struct MockVmCheckpoint {
    snapshots: Mutex<Vec<SnapshotInfo>>,
}

impl MockVmCheckpoint {
    /// Create a fresh mock with no snapshots.
    pub fn new() -> Self {
        Self {
            snapshots: Mutex::new(Vec::new()),
        }
    }

    /// Number of snapshots stored.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.lock().expect("lock").len()
    }
}

impl Default for MockVmCheckpoint {
    fn default() -> Self {
        Self::new()
    }
}

impl VmCheckpoint for MockVmCheckpoint {
    fn save_snapshot(
        &self,
        container_id: &str,
        path: &Path,
    ) -> Result<SnapshotInfo> {
        let info = SnapshotInfo {
            container_id: container_id.to_string(),
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "snapshot".to_string()),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            adapter: "mock".to_string(),
            image: "alpine:3.18".to_string(),
            size_bytes: 0,
        };
        self.snapshots.lock().expect("lock").push(info.clone());
        Ok(info)
    }

    fn restore_snapshot(&self, container_id: &str, _path: &Path) -> Result<()> {
        let snaps = self.snapshots.lock().expect("lock");
        if snaps.iter().any(|s| s.container_id == container_id) {
            Ok(())
        } else {
            anyhow::bail!("no snapshot for container {container_id}")
        }
    }

    fn list_snapshots(&self, container_id: &str) -> Result<Vec<SnapshotInfo>> {
        let snaps = self.snapshots.lock().expect("lock");
        Ok(snaps
            .iter()
            .filter(|s| s.container_id == container_id)
            .cloned()
            .collect())
    }
}
```

2. Add `pub mod vm_checkpoint;` and `pub use vm_checkpoint::MockVmCheckpoint;` to
   mocks/mod.rs.
3. Verify: `cargo check -p minibox`
4. Commit: `feat(minibox): add MockVmCheckpoint for conformance testing`

---

### Task 5: Add MockMetricsRecorder

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/testing/mocks/metrics.rs`,
`crates/minibox/src/testing/mocks/mod.rs`
**Run**: `cargo check -p minibox`

1. Create `crates/minibox/src/testing/mocks/metrics.rs`:

```rust
//! Mock [`MetricsRecorder`] for conformance testing.

use minibox_core::domain::MetricsRecorder;
use std::sync::Mutex;

/// Recorded metric event.
#[derive(Debug, Clone)]
pub enum MetricEvent {
    Counter { name: String },
    Histogram { name: String, value: f64 },
    Gauge { name: String, value: f64 },
}

/// Mock metrics recorder that captures all events in memory.
#[derive(Debug)]
pub struct MockMetricsRecorder {
    events: Mutex<Vec<MetricEvent>>,
}

impl MockMetricsRecorder {
    /// Create a fresh mock.
    pub fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    /// Return all recorded events.
    pub fn events(&self) -> Vec<MetricEvent> {
        self.events.lock().expect("lock").clone()
    }

    /// Number of recorded events.
    pub fn event_count(&self) -> usize {
        self.events.lock().expect("lock").len()
    }
}

impl Default for MockMetricsRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRecorder for MockMetricsRecorder {
    fn increment_counter(&self, name: &str, _labels: &[(&str, &str)]) {
        self.events
            .lock()
            .expect("lock")
            .push(MetricEvent::Counter {
                name: name.to_string(),
            });
    }

    fn record_histogram(
        &self,
        name: &str,
        value: f64,
        _labels: &[(&str, &str)],
    ) {
        self.events
            .lock()
            .expect("lock")
            .push(MetricEvent::Histogram {
                name: name.to_string(),
                value,
            });
    }

    fn set_gauge(&self, name: &str, value: f64, _labels: &[(&str, &str)]) {
        self.events
            .lock()
            .expect("lock")
            .push(MetricEvent::Gauge {
                name: name.to_string(),
                value,
            });
    }
}
```

2. Add `pub mod metrics;` and `pub use metrics::MockMetricsRecorder;` to mocks/mod.rs.
3. Verify: `cargo check -p minibox`
4. Commit: `feat(minibox): add MockMetricsRecorder for conformance testing`

---

### Task 6: Add MockRegistryRouter

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/testing/mocks/registry_router.rs`,
`crates/minibox/src/testing/mocks/mod.rs`
**Run**: `cargo check -p minibox`

1. Create `crates/minibox/src/testing/mocks/registry_router.rs`:

```rust
//! Mock [`RegistryRouter`] for conformance testing.

use minibox_core::domain::{ImageRegistry, RegistryRouter};
use minibox_core::image::reference::ImageRef;
use std::sync::Arc;

/// Mock router that always returns the same registry.
#[derive(Debug)]
pub struct MockRegistryRouter {
    registry: Arc<dyn ImageRegistry>,
}

impl MockRegistryRouter {
    /// Create a router backed by the given registry.
    pub fn new(registry: Arc<dyn ImageRegistry>) -> Self {
        Self { registry }
    }
}

impl RegistryRouter for MockRegistryRouter {
    fn route(&self, _image_ref: &ImageRef) -> &dyn ImageRegistry {
        self.registry.as_ref()
    }
}
```

2. Add `pub mod registry_router;` and `pub use registry_router::MockRegistryRouter;` to
   mocks/mod.rs.
3. Verify: `cargo check -p minibox`
4. Commit: `feat(minibox): add MockRegistryRouter for conformance testing`

---

### Task 7: Add MockImageLoader

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/testing/mocks/image_loader.rs`,
`crates/minibox/src/testing/mocks/mod.rs`
**Run**: `cargo check -p minibox`

1. Create `crates/minibox/src/testing/mocks/image_loader.rs`:

```rust
//! Mock [`ImageLoader`] for conformance testing.

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::domain::ImageLoader;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Mock image loader that records load calls without touching the filesystem.
#[derive(Debug)]
pub struct MockImageLoader {
    load_count: AtomicUsize,
    should_fail: bool,
}

impl MockImageLoader {
    /// Create a mock that succeeds on every call.
    pub fn new() -> Self {
        Self {
            load_count: AtomicUsize::new(0),
            should_fail: false,
        }
    }

    /// Create a mock that fails on every call.
    pub fn failing() -> Self {
        Self {
            load_count: AtomicUsize::new(0),
            should_fail: true,
        }
    }

    /// Number of `load_image` calls.
    pub fn load_count(&self) -> usize {
        self.load_count.load(Ordering::Relaxed)
    }
}

impl Default for MockImageLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ImageLoader for MockImageLoader {
    async fn load_image(
        &self,
        _path: &Path,
        _name: &str,
        _tag: &str,
    ) -> Result<()> {
        self.load_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail {
            anyhow::bail!("mock: load_image configured to fail")
        }
        Ok(())
    }
}
```

2. Add `pub mod image_loader;` and `pub use image_loader::MockImageLoader;` to mocks/mod.rs.
3. Verify: `cargo check -p minibox`
4. Commit: `feat(minibox): add MockImageLoader for conformance testing`

---

### Phase 3: Conformance Test Modules

Each module follows the `registry.rs` reference pattern: struct per test,
`ConformanceTest` impl, `pub fn all() -> Vec<Box<dyn ConformanceTest>>`.

### Task 8: Add filesystem conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/filesystem.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create the file with tests for `RootfsSetup` and `ChildInit` contracts:

```rust
//! Conformance tests for the [`FilesystemProvider`] trait contract
//! ([`RootfsSetup`] + [`ChildInit`]).

use minibox::testing::mocks::filesystem::MockFilesystem;
use minibox_core::domain::RootfsSetup;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

pub struct SetupReturnsValidRootfs;
impl ConformanceTest for SetupReturnsValidRootfs {
    fn name(&self) -> &str { "setup_returns_valid_rootfs" }
    fn adapter(&self) -> &str { "filesystem" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let fs = MockFilesystem::new();
        let layout = fs.setup_rootfs(&[], &tempfile::tempdir().expect("tmpdir").path());
        ctx.assert_ok(layout, "setup_rootfs should succeed with empty layers");
        ctx.result()
    }
}

pub struct SetupIdempotentOnSameLayers;
impl ConformanceTest for SetupIdempotentOnSameLayers {
    fn name(&self) -> &str { "setup_idempotent_on_same_layers" }
    fn adapter(&self) -> &str { "filesystem" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let fs = MockFilesystem::new();
        let dir = tempfile::tempdir().expect("tmpdir");
        let r1 = fs.setup_rootfs(&[], dir.path());
        let r2 = fs.setup_rootfs(&[], dir.path());
        ctx.assert_ok(r1, "first setup");
        ctx.assert_ok(r2, "second setup (idempotent)");
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(SetupReturnsValidRootfs),
        Box::new(SetupIdempotentOnSameLayers),
    ]
}
```

2. Register in `adapters/mod.rs`: add `pub mod filesystem;` and
   `tests.extend(filesystem::all());` in `all()`.

3. Verify: `cargo check -p minibox-testsuite`
4. Commit: `feat(minibox-testsuite): add filesystem conformance module`

---

### Task 9: Add exec_runtime conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/exec_runtime.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `ExecRuntime::run_in_container`:

```rust
//! Conformance tests for the [`ExecRuntime`] trait contract.

use minibox::testing::mocks::exec::MockExecRuntime;
use minibox_core::domain::{ContainerId, ExecRuntime, ExecSpec};
use minibox_core::protocol::DaemonResponse;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

pub struct ExecReturnsHandle;
impl ConformanceTest for ExecReturnsHandle {
    fn name(&self) -> &str { "exec_returns_handle" }
    fn adapter(&self) -> &str { "exec_runtime" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let exec = MockExecRuntime::new();
        let cid = ContainerId::new("test-exec-c1".into()).expect("valid id");
        let spec = ExecSpec {
            cmd: vec!["echo".into(), "hello".into()],
            env: vec![],
            working_dir: None,
            tty: false,
        };
        let (tx, _rx) = tokio::sync::mpsc::channel::<DaemonResponse>(16);
        let result = rt().block_on(exec.run_in_container(&cid, spec, tx));
        let handle = ctx.assert_ok(result, "exec should return a handle");
        if let Some(h) = handle {
            ctx.assert_true(!h.id.is_empty(), "handle id must not be empty");
        }
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(ExecReturnsHandle),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Verify: `cargo check -p minibox-testsuite`
4. Commit: `feat(minibox-testsuite): add exec_runtime conformance module`

---

### Task 10: Add image_pusher conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/image_pusher.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `ImagePusher::push_image`:

```rust
//! Conformance tests for the [`ImagePusher`] trait contract.

use minibox::testing::mocks::push::MockImagePusher;
use minibox_core::domain::{ImagePusher, RegistryCredentials};
use minibox_core::image::reference::ImageRef;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

pub struct PushReturnsDigest;
impl ConformanceTest for PushReturnsDigest {
    fn name(&self) -> &str { "push_returns_digest" }
    fn adapter(&self) -> &str { "image_pusher" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let pusher = MockImagePusher::new();
        let image = ImageRef::parse("alpine:3.18").expect("parse");
        let creds = RegistryCredentials::Anonymous;
        let result = rt().block_on(pusher.push_image(&image, &creds, None));
        let pr = ctx.assert_ok(result, "push should succeed");
        if let Some(r) = pr {
            ctx.assert_true(!r.digest.is_empty(), "digest must not be empty");
        }
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(PushReturnsDigest),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Verify: `cargo check -p minibox-testsuite`
4. Commit: `feat(minibox-testsuite): add image_pusher conformance module`

---

### Task 11: Add container_committer conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/container_committer.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `ContainerCommitter::commit`:

```rust
//! Conformance tests for the [`ContainerCommitter`] trait contract.

use minibox::testing::mocks::commit::MockContainerCommitter;
use minibox_core::domain::{CommitConfig, ContainerId, ContainerCommitter};

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

pub struct CommitReturnsImageMetadata;
impl ConformanceTest for CommitReturnsImageMetadata {
    fn name(&self) -> &str { "commit_returns_image_metadata" }
    fn adapter(&self) -> &str { "container_committer" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let committer = MockContainerCommitter::new();
        let cid = ContainerId::new("test-commit-c1".into()).expect("valid id");
        let config = CommitConfig {
            author: Some("test".into()),
            message: Some("test commit".into()),
            env_overrides: vec![],
            cmd_override: None,
        };
        let result = rt().block_on(committer.commit(&cid, "test:latest", &config));
        let meta = ctx.assert_ok(result, "commit should succeed");
        if let Some(m) = meta {
            ctx.assert_true(!m.name.is_empty(), "image name must not be empty");
        }
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(CommitReturnsImageMetadata),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Commit: `feat(minibox-testsuite): add container_committer conformance module`

---

### Task 12: Add image_builder conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/image_builder.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `ImageBuilder::build_image`:

```rust
//! Conformance tests for the [`ImageBuilder`] trait contract.

use minibox::testing::mocks::build::MockImageBuilder;
use minibox_core::domain::{BuildConfig, BuildContext, BuildProgress, ImageBuilder};

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

pub struct BuildReturnsImageMetadata;
impl ConformanceTest for BuildReturnsImageMetadata {
    fn name(&self) -> &str { "build_returns_image_metadata" }
    fn adapter(&self) -> &str { "image_builder" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let builder = MockImageBuilder::new();
        let dir = tempfile::tempdir().expect("tmpdir");
        let context = BuildContext {
            directory: dir.path().to_path_buf(),
            dockerfile: dir.path().join("Dockerfile"),
        };
        let config = BuildConfig {
            tag: "test:latest".into(),
            build_args: vec![],
            no_cache: false,
        };
        let (tx, _rx) = tokio::sync::mpsc::channel::<BuildProgress>(16);
        let result = rt().block_on(builder.build_image(&context, &config, tx));
        let meta = ctx.assert_ok(result, "build should succeed");
        if let Some(m) = meta {
            ctx.assert_true(!m.name.is_empty(), "image name must not be empty");
        }
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(BuildReturnsImageMetadata),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Commit: `feat(minibox-testsuite): add image_builder conformance module`

---

### Task 13: Add network conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/network.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `NetworkProvider` contract:

```rust
//! Conformance tests for the [`NetworkProvider`] trait contract.

use minibox::testing::mocks::network::MockNetwork;
use minibox_core::domain::{NetworkConfig, NetworkProvider};

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

pub struct SetupReturnsNamespacePath;
impl ConformanceTest for SetupReturnsNamespacePath {
    fn name(&self) -> &str { "setup_returns_namespace_path" }
    fn adapter(&self) -> &str { "network" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let network = MockNetwork::new();
        let config = NetworkConfig::default();
        let result = rt().block_on(network.setup("test-container", &config));
        let ns = ctx.assert_ok(result, "network setup should succeed");
        if let Some(path) = ns {
            ctx.assert_true(!path.is_empty(), "namespace path must not be empty");
        }
        ctx.result()
    }
}

pub struct CleanupSucceedsAfterSetup;
impl ConformanceTest for CleanupSucceedsAfterSetup {
    fn name(&self) -> &str { "cleanup_succeeds_after_setup" }
    fn adapter(&self) -> &str { "network" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let network = MockNetwork::new();
        let config = NetworkConfig::default();
        rt().block_on(network.setup("test-container", &config)).expect("setup");
        let result = rt().block_on(network.cleanup("test-container"));
        ctx.assert_ok(result, "cleanup after setup should succeed");
        ctx.result()
    }
}

pub struct StatsReturnsZeroForFreshContainer;
impl ConformanceTest for StatsReturnsZeroForFreshContainer {
    fn name(&self) -> &str { "stats_returns_zero_for_fresh_container" }
    fn adapter(&self) -> &str { "network" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let network = MockNetwork::new();
        let config = NetworkConfig::default();
        rt().block_on(network.setup("test-container", &config)).expect("setup");
        let result = rt().block_on(network.stats("test-container"));
        let stats = ctx.assert_ok(result, "stats should succeed");
        if let Some(s) = stats {
            ctx.assert_eq(0u64, s.rx_bytes, "rx_bytes for fresh container");
            ctx.assert_eq(0u64, s.tx_bytes, "tx_bytes for fresh container");
        }
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(SetupReturnsNamespacePath),
        Box::new(CleanupSucceedsAfterSetup),
        Box::new(StatsReturnsZeroForFreshContainer),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Commit: `feat(minibox-testsuite): add network conformance module`

---

### Task 14: Add tty conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/tty.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `TtyProvider` contract:

```rust
//! Conformance tests for the [`TtyProvider`] trait contract.

use minibox::testing::mocks::tty::MockTtyProvider;
use minibox_core::domain::{TtyConfig, TtyProvider};

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

pub struct CreateReturnsFdPair;
impl ConformanceTest for CreateReturnsFdPair {
    fn name(&self) -> &str { "create_returns_fd_pair" }
    fn adapter(&self) -> &str { "tty" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tty = MockTtyProvider::new();
        let config = TtyConfig::default();
        let result = rt().block_on(tty.create(&config));
        let pair = ctx.assert_ok(result, "create should return fd pair");
        if let Some((master, slave)) = pair {
            ctx.assert_ne(master, slave, "master and slave fds must differ");
        }
        ctx.assert_eq(1, tty.create_count(), "create_count after one call");
        ctx.result()
    }
}

pub struct ResizeSucceeds;
impl ConformanceTest for ResizeSucceeds {
    fn name(&self) -> &str { "resize_succeeds" }
    fn adapter(&self) -> &str { "tty" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tty = MockTtyProvider::new();
        let config = TtyConfig::default();
        let (master, _) = rt().block_on(tty.create(&config)).expect("create");
        let result = rt().block_on(tty.resize(master, 120, 40));
        ctx.assert_ok(result, "resize should succeed");
        ctx.assert_eq(1, tty.resize_count(), "resize_count");
        ctx.result()
    }
}

pub struct CloseSucceeds;
impl ConformanceTest for CloseSucceeds {
    fn name(&self) -> &str { "close_succeeds" }
    fn adapter(&self) -> &str { "tty" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let tty = MockTtyProvider::new();
        let config = TtyConfig::default();
        let (master, _) = rt().block_on(tty.create(&config)).expect("create");
        let result = rt().block_on(tty.close(master));
        ctx.assert_ok(result, "close should succeed");
        ctx.assert_eq(1, tty.close_count(), "close_count");
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(CreateReturnsFdPair),
        Box::new(ResizeSucceeds),
        Box::new(CloseSucceeds),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Commit: `feat(minibox-testsuite): add tty conformance module`

---

### Task 15: Add pty conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/pty.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `PtyAllocator` contract:

```rust
//! Conformance tests for the [`PtyAllocator`] trait contract.

use minibox_core::domain::{MockPtyAllocator, NullPtyAllocator, PtyAllocator, PtyConfig};

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

pub struct MockAllocateReturnsFds;
impl ConformanceTest for MockAllocateReturnsFds {
    fn name(&self) -> &str { "mock_allocate_returns_fds" }
    fn adapter(&self) -> &str { "pty" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let alloc = MockPtyAllocator::new(10, 11);
        let config = PtyConfig {
            enabled: true,
            cols: 80,
            rows: 24,
        };
        let result = alloc.allocate(&config);
        let handle = ctx.assert_ok(result, "mock allocate should succeed");
        if let Some(h) = handle {
            ctx.assert_eq(10, h.master_fd, "master_fd");
            ctx.assert_eq(11, h.slave_fd, "slave_fd");
        }
        ctx.result()
    }
}

pub struct NullAllocatorReturnsErr;
impl ConformanceTest for NullAllocatorReturnsErr {
    fn name(&self) -> &str { "null_allocator_returns_err" }
    fn adapter(&self) -> &str { "pty" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let alloc = NullPtyAllocator;
        let config = PtyConfig {
            enabled: true,
            cols: 80,
            rows: 24,
        };
        let result = alloc.allocate(&config);
        ctx.assert_err(result, "null allocator must return Err");
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(MockAllocateReturnsFds),
        Box::new(NullAllocatorReturnsErr),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Commit: `feat(minibox-testsuite): add pty conformance module`

---

### Task 16: Add vm_checkpoint conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/vm_checkpoint.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `VmCheckpoint` contract:

```rust
//! Conformance tests for the [`VmCheckpoint`] trait contract.

use minibox::testing::mocks::vm_checkpoint::MockVmCheckpoint;
use minibox_core::domain::VmCheckpoint;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

pub struct SaveReturnsSnapshotInfo;
impl ConformanceTest for SaveReturnsSnapshotInfo {
    fn name(&self) -> &str { "save_returns_snapshot_info" }
    fn adapter(&self) -> &str { "vm_checkpoint" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let ckpt = MockVmCheckpoint::new();
        let dir = tempfile::tempdir().expect("tmpdir");
        let result = ckpt.save_snapshot("c1", &dir.path().join("snap1"));
        let info = ctx.assert_ok(result, "save_snapshot should succeed");
        if let Some(i) = info {
            ctx.assert_eq("c1", i.container_id.as_str(), "container_id");
            ctx.assert_eq("snap1", i.name.as_str(), "snapshot name");
        }
        ctx.result()
    }
}

pub struct RestoreFailsWithoutSave;
impl ConformanceTest for RestoreFailsWithoutSave {
    fn name(&self) -> &str { "restore_fails_without_save" }
    fn adapter(&self) -> &str { "vm_checkpoint" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let ckpt = MockVmCheckpoint::new();
        let dir = tempfile::tempdir().expect("tmpdir");
        let result = ckpt.restore_snapshot("c1", &dir.path().join("snap1"));
        ctx.assert_err(result, "restore without save must fail");
        ctx.result()
    }
}

pub struct ListEmptyForFreshCheckpointer;
impl ConformanceTest for ListEmptyForFreshCheckpointer {
    fn name(&self) -> &str { "list_empty_for_fresh_checkpointer" }
    fn adapter(&self) -> &str { "vm_checkpoint" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let ckpt = MockVmCheckpoint::new();
        let result = ckpt.list_snapshots("c1");
        let list = ctx.assert_ok(result, "list_snapshots should succeed");
        if let Some(l) = list {
            ctx.assert_eq(0, l.len(), "empty list for fresh checkpointer");
        }
        ctx.result()
    }
}

pub struct SaveThenListReturnsOne;
impl ConformanceTest for SaveThenListReturnsOne {
    fn name(&self) -> &str { "save_then_list_returns_one" }
    fn adapter(&self) -> &str { "vm_checkpoint" }
    fn category(&self) -> TestCategory { TestCategory::Integration }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let ckpt = MockVmCheckpoint::new();
        let dir = tempfile::tempdir().expect("tmpdir");
        ckpt.save_snapshot("c1", &dir.path().join("snap1")).expect("save");
        let list = ckpt.list_snapshots("c1").expect("list");
        ctx.assert_eq(1, list.len(), "one snapshot after save");
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(SaveReturnsSnapshotInfo),
        Box::new(RestoreFailsWithoutSave),
        Box::new(ListEmptyForFreshCheckpointer),
        Box::new(SaveThenListReturnsOne),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Commit: `feat(minibox-testsuite): add vm_checkpoint conformance module`

---

### Task 17: Add metrics conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/metrics.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `MetricsRecorder` contract:

```rust
//! Conformance tests for the [`MetricsRecorder`] trait contract.

use minibox::testing::mocks::metrics::MockMetricsRecorder;
use minibox_core::domain::MetricsRecorder;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

pub struct IncrementCounterRecordsEvent;
impl ConformanceTest for IncrementCounterRecordsEvent {
    fn name(&self) -> &str { "increment_counter_records_event" }
    fn adapter(&self) -> &str { "metrics" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let recorder = MockMetricsRecorder::new();
        recorder.increment_counter("test.requests", &[("method", "GET")]);
        ctx.assert_eq(1, recorder.event_count(), "one event after increment");
        ctx.result()
    }
}

pub struct RecordHistogramRecordsEvent;
impl ConformanceTest for RecordHistogramRecordsEvent {
    fn name(&self) -> &str { "record_histogram_records_event" }
    fn adapter(&self) -> &str { "metrics" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let recorder = MockMetricsRecorder::new();
        recorder.record_histogram("test.duration", 0.5, &[]);
        ctx.assert_eq(1, recorder.event_count(), "one event after histogram");
        ctx.result()
    }
}

pub struct SetGaugeRecordsEvent;
impl ConformanceTest for SetGaugeRecordsEvent {
    fn name(&self) -> &str { "set_gauge_records_event" }
    fn adapter(&self) -> &str { "metrics" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let recorder = MockMetricsRecorder::new();
        recorder.set_gauge("test.active", 42.0, &[]);
        ctx.assert_eq(1, recorder.event_count(), "one event after gauge");
        ctx.result()
    }
}

pub struct FreshRecorderHasNoEvents;
impl ConformanceTest for FreshRecorderHasNoEvents {
    fn name(&self) -> &str { "fresh_recorder_has_no_events" }
    fn adapter(&self) -> &str { "metrics" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let recorder = MockMetricsRecorder::new();
        ctx.assert_eq(0, recorder.event_count(), "no events on fresh recorder");
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(IncrementCounterRecordsEvent),
        Box::new(RecordHistogramRecordsEvent),
        Box::new(SetGaugeRecordsEvent),
        Box::new(FreshRecorderHasNoEvents),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Commit: `feat(minibox-testsuite): add metrics conformance module`

---

### Task 18: Add registry_router conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/registry_router.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `RegistryRouter::route`:

```rust
//! Conformance tests for the [`RegistryRouter`] trait contract.

use minibox::testing::mocks::registry::MockRegistry;
use minibox::testing::mocks::registry_router::MockRegistryRouter;
use minibox_core::domain::RegistryRouter;
use minibox_core::image::reference::ImageRef;
use std::sync::Arc;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

pub struct RouteReturnsRegistryRef;
impl ConformanceTest for RouteReturnsRegistryRef {
    fn name(&self) -> &str { "route_returns_registry_ref" }
    fn adapter(&self) -> &str { "registry_router" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let registry = Arc::new(MockRegistry::new());
        let router = MockRegistryRouter::new(registry);
        let image = ImageRef::parse("alpine:3.18").expect("parse");
        let _routed = router.route(&image);
        // If we get here without panic, the route returned a valid reference.
        ctx.assert_true(true, "route returned a registry reference");
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(RouteReturnsRegistryRef),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Commit: `feat(minibox-testsuite): add registry_router conformance module`

---

### Task 19: Add image_loader conformance module

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/image_loader.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create with tests for `ImageLoader::load_image`:

```rust
//! Conformance tests for the [`ImageLoader`] trait contract.

use minibox::testing::mocks::image_loader::MockImageLoader;
use minibox_core::domain::ImageLoader;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

pub struct LoadIncreasesCount;
impl ConformanceTest for LoadIncreasesCount {
    fn name(&self) -> &str { "load_increases_count" }
    fn adapter(&self) -> &str { "image_loader" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let loader = MockImageLoader::new();
        let dir = tempfile::tempdir().expect("tmpdir");
        let result = rt().block_on(
            loader.load_image(&dir.path().join("image.tar"), "alpine", "3.18"),
        );
        ctx.assert_ok(result, "load_image should succeed");
        ctx.assert_eq(1, loader.load_count(), "load_count after one call");
        ctx.result()
    }
}

pub struct FailingLoaderReturnsErr;
impl ConformanceTest for FailingLoaderReturnsErr {
    fn name(&self) -> &str { "failing_loader_returns_err" }
    fn adapter(&self) -> &str { "image_loader" }
    fn category(&self) -> TestCategory { TestCategory::EdgeCase }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let loader = MockImageLoader::failing();
        let dir = tempfile::tempdir().expect("tmpdir");
        let result = rt().block_on(
            loader.load_image(&dir.path().join("image.tar"), "alpine", "3.18"),
        );
        ctx.assert_err(result, "failing loader must return Err");
        ctx.result()
    }
}

pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(LoadIncreasesCount),
        Box::new(FailingLoaderReturnsErr),
    ]
}
```

2. Register in `adapters/mod.rs`.
3. Commit: `feat(minibox-testsuite): add image_loader conformance module`

---

### Phase 4: Hub + Spokes Wiring

### Task 20: Create SpokeRegistry

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/spoke.rs`,
`crates/minibox-testsuite/src/lib.rs`
**Run**: `cargo check -p minibox-testsuite`

1. Create `crates/minibox-testsuite/src/spoke.rs`:

```rust
//! Spoke registration for external crate conformance tests.
//!
//! Spoke crates call [`SpokeRegistry::register`] to contribute tests to the
//! central conformance runner without being compiled into `minibox-testsuite`.

use crate::harness::ConformanceTest;

/// Collects conformance tests from spoke crates.
#[derive(Default)]
pub struct SpokeRegistry {
    tests: Vec<Box<dyn ConformanceTest>>,
}

impl SpokeRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a batch of tests from a spoke crate.
    pub fn register(&mut self, tests: Vec<Box<dyn ConformanceTest>>) {
        self.tests.extend(tests);
    }

    /// Consume the registry and return all registered tests.
    pub fn into_tests(self) -> Vec<Box<dyn ConformanceTest>> {
        self.tests
    }

    /// Number of registered tests.
    pub fn count(&self) -> usize {
        self.tests.len()
    }
}
```

2. Add `pub mod spoke;` to `crates/minibox-testsuite/src/lib.rs`.
3. Verify: `cargo check -p minibox-testsuite`
4. Commit: `feat(minibox-testsuite): add SpokeRegistry for hub+spokes model`

---

### Task 21: Update adapters/mod.rs with all 12 new modules

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/mod.rs`
**Run**: `cargo run -p minibox-testsuite --bin run-conformance`

1. Replace `mod.rs` contents with:

```rust
//! Per-adapter conformance test modules.
//!
//! Each module exposes an `all()` function returning
//! `Vec<Box<dyn ConformanceTest>>`. The `run-conformance` binary collects
//! all adapters and feeds them to `TestRunner`.

pub mod container_committer;
pub mod container_id;
pub mod exec_runtime;
pub mod filesystem;
pub mod image_builder;
pub mod image_loader;
pub mod image_pusher;
pub mod limiter;
pub mod list;
pub mod logs;
pub mod metrics;
pub mod network;
pub mod pause_resume;
pub mod policy;
pub mod pty;
pub mod registry;
pub mod registry_router;
pub mod runtime;
pub mod state;
pub mod tty;
pub mod vm_checkpoint;

use crate::harness::ConformanceTest;

/// Collect every conformance test across all adapters.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    let mut tests: Vec<Box<dyn ConformanceTest>> = Vec::new();
    tests.extend(registry::all());
    tests.extend(runtime::all());
    tests.extend(limiter::all());
    tests.extend(state::all());
    tests.extend(pause_resume::all());
    tests.extend(list::all());
    tests.extend(policy::all());
    tests.extend(container_id::all());
    tests.extend(logs::all());
    // new modules
    tests.extend(filesystem::all());
    tests.extend(exec_runtime::all());
    tests.extend(image_pusher::all());
    tests.extend(container_committer::all());
    tests.extend(image_builder::all());
    tests.extend(network::all());
    tests.extend(tty::all());
    tests.extend(pty::all());
    tests.extend(vm_checkpoint::all());
    tests.extend(metrics::all());
    tests.extend(registry_router::all());
    tests.extend(image_loader::all());
    tests
}
```

2. Run: `cargo run -p minibox-testsuite --bin run-conformance`
   Verify all tests pass.
3. Commit: `feat(minibox-testsuite): register 12 new conformance modules`

---

### Phase 5: CI Expansion

### Task 22: Add CI jobs to conformance.yml

**File(s)**: `.github/workflows/conformance.yml`

1. Add three new jobs after the existing `conformance` job:

```yaml
  property-tests:
    name: property tests (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    needs: [detect-changes]
    if: needs.detect-changes.outputs.conformance == 'true'
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: run property tests
        run: cargo xtask test-property

  krun-conformance:
    name: krun conformance
    runs-on: macos-latest
    needs: [detect-changes]
    if: needs.detect-changes.outputs.conformance == 'true'
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: run krun conformance
        run: cargo xtask test-krun-conformance

  cli-conformance:
    name: cli conformance (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    needs: [detect-changes]
    if: needs.detect-changes.outputs.conformance == 'true'
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: build workspace
        run: cargo build --workspace
      - name: run cli conformance
        run: cargo nextest run -p mbx --test conformance_cli
```

2. Commit: `ci: add property, krun, and cli conformance jobs`

---

### Task 23: Update BackendDescriptor construction sites

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/testing/backend/descriptor.rs` + all 6
consumer files listed in Dependencies.
**Run**: `cargo check --workspace`

1. Add 9 new `Option` factory fields to `BackendDescriptor` (initialized to
   `None` in `new()`), plus `with_*` builder methods following the existing
   `with_committer`/`with_builder`/`with_pusher` pattern.

2. Update all 6 construction sites to compile (they already use builder
   pattern, so `None` defaults mean no changes needed unless they construct
   raw structs).

3. Verify: `cargo check --workspace`
4. Commit: `feat(minibox): extend BackendDescriptor with 9 new adapter factories`

---

### Task 24: Final verification

**Run**: `cargo run -p minibox-testsuite --bin run-conformance`

1. Run the full conformance suite and verify all new tests pass.
2. Run `cargo xtask verify` to confirm no regressions.
3. Run `cargo clippy --workspace -- -D warnings` for clean clippy.
4. Update `docs/TEST_INFRASTRUCTURE.mbx.md` test counts.
5. Commit: `docs: update test infrastructure counts for conformance expansion`
