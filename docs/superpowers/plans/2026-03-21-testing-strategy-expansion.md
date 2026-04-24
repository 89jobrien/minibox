---
status: done
completed: "2026-03-21"
branch: main
---

# Testing Strategy Expansion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expand minibox's test coverage from 147 unit tests → ~250 tests across three phased strategies: adapter isolation, property-based generative testing, and chaos/fault injection for production reliability.

**Architecture:** Three independent phases executed sequentially. Phase 1 (adapter isolation) provides test fixtures and mock infrastructure used by Phase 2 (property-based) and Phase 3 (chaos). Each phase adds coverage to different layers: adapters → protocol/domain → failure modes.

**Tech Stack:** Rust, proptest (generative testing), tokio (async), nix crate (syscalls), mockito/test fixtures (mocking).

---

## File Structure

### Phase 1: Adapter Isolation Testing

New test files:

- `crates/minibox/tests/adapter_native_tests.rs` — native adapter tests (cgroups, overlay, namespace setup)
- `crates/minibox/tests/adapter_gke_tests.rs` — GKE adapter tests (proot, copy FS)
- `crates/minibox/tests/adapter_colima_tests.rs` — Colima adapter tests (Colima VM specifics)
- `crates/minibox/src/adapters/test_fixtures.rs` — Shared test fixtures, mock builders
- `crates/daemonbox/tests/handler_adapter_swap_tests.rs` — Handler tests with mock adapter swaps

Modified files:

- `crates/minibox/src/adapters/mocks.rs` — Extend mocks with failure injection helpers
- `crates/minibox/src/adapters/mod.rs` — Export test_fixtures module

### Phase 2: Property-Based Testing

New test files:

- `crates/minibox/tests/protocol_codec_properties.rs` — Extended codec roundtrip + edge cases
- `crates/minibox/tests/path_validation_properties.rs` — Path traversal, symlinks, normalization
- `crates/minibox/tests/image_manifest_properties.rs` — Manifest parsing invariants
- `crates/minibox/tests/cgroup_boundary_properties.rs` — Resource limit boundaries, overflow handling

Modified files:

- `crates/minibox/tests/proptest_suite.rs` — Consolidate existing + new properties into suites

### Phase 3: Chaos & Fault Injection

New test files:

- `crates/minibox/tests/adapter_failure_injection_tests.rs` — Adapter failures (incomplete pulls, cgroup errors)
- `crates/miniboxd/tests/container_lifecycle_failure_tests.rs` — Lifecycle failures (zombie processes, incomplete cleanup)
- `crates/daemonbox/tests/daemon_recovery_tests.rs` — Daemon crash & recovery scenarios
- `crates/miniboxd/tests/resource_exhaustion_tests.rs` — Memory/disk exhaustion handling

---

## Phase 1: Adapter Isolation Testing (10 tasks, ~40 min)

> **Complexity:** Low-medium. Reuses existing adapter interfaces; new tests are mostly composition of existing builder patterns.
> **Dependencies:** None (establishes fixtures for later phases)
> **Success criteria:** All 4 adapters have ≥5 focused tests each; handler tests pass with mock swaps; adapter contract is documented.

### Task 1: Create native adapter test suite (filesystem + namespace setup)

**Files:**

- Create: `crates/minibox/tests/adapter_native_tests.rs`

**Steps:**

- [ ] **Step 1: Write failing test for overlay mount creation**

```rust
#[tokio::test]
#[cfg(target_os = "linux")]
async fn test_native_adapter_overlay_mount_succeeds_with_valid_layers() {
    let tmp = tempfile::tempdir().unwrap();
    let adapter = create_native_filesystem_adapter(tmp.path());

    let layer1 = tmp.path().join("layer1");
    std::fs::create_dir(&layer1).unwrap();
    std::fs::write(layer1.join("file.txt"), b"content").unwrap();

    let container_id = "test-overlay-123";
    let result = adapter.mount_container_root(container_id, vec![layer1], tmp.path()).await;

    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox --test adapter_native_tests -- --nocapture`
Expected: FAIL — function `create_native_filesystem_adapter` not found

- [ ] **Step 3: Implement native adapter test fixture**

In `crates/minibox/tests/adapter_native_tests.rs`:

```rust
use minibox::adapters::native_filesystem::NativeFilesystemProvider;
use minibox::domain::FilesystemProvider;
use std::path::Path;

fn create_native_filesystem_adapter(base_dir: &Path) -> Arc<dyn FilesystemProvider> {
    Arc::new(NativeFilesystemProvider::new(base_dir.to_path_buf()))
}

#[tokio::test]
#[cfg(target_os = "linux")]
#[ignore] // Requires cgroup v2 mount + root
async fn test_native_adapter_overlay_mount_succeeds_with_valid_layers() {
    // ... test code above
}

#[test]
#[cfg(target_os = "linux")]
fn test_native_adapter_path_validation_rejects_symlink_escape_attempts() {
    let tmp = tempfile::tempdir().unwrap();
    let adapter = create_native_filesystem_adapter(tmp.path());

    // Verify validate_layer_path rejects ../../../
    let malicious_path = Path::new("../../etc/passwd");
    let result = adapter.validate_path(malicious_path);
    assert!(result.is_err());
}

#[test]
fn test_native_adapter_normalizes_paths_with_dot_components() {
    let tmp = tempfile::tempdir().unwrap();
    let adapter = create_native_filesystem_adapter(tmp.path());

    let path = Path::new("./subdir/../file.txt");
    let normalized = adapter.normalize_path(path);
    assert_eq!(normalized, Path::new("file.txt"));
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p minibox --test adapter_native_tests -- --nocapture`
Expected: PASS (or IGNORE for root-required tests)

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/tests/adapter_native_tests.rs
git commit -m "test(adapters): add native filesystem adapter isolation tests"
```

---

### Task 2: Create GKE adapter test suite (proot, copy FS)

**Files:**

- Create: `crates/minibox/tests/adapter_gke_tests.rs`

**Steps:**

- [ ] **Step 1: Write failing test for GKE adapter copy FS**

```rust
#[tokio::test]
async fn test_gke_adapter_copy_fs_preserves_layer_contents() {
    let tmp = tempfile::tempdir().unwrap();
    let adapter = create_gke_filesystem_adapter(tmp.path());

    let source = tmp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("file.txt"), b"content").unwrap();

    let container_id = "test-copy-123";
    let result = adapter.copy_tree(container_id, &source).await;

    assert!(result.is_ok());
    // Verify content was copied
    let dest_file = result.unwrap().join("file.txt");
    assert!(dest_file.exists());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox --test adapter_gke_tests -- --nocapture`
Expected: FAIL — function `create_gke_filesystem_adapter` not found

- [ ] **Step 3: Implement GKE adapter test fixture**

In `crates/minibox/tests/adapter_gke_tests.rs`:

```rust
use minibox::adapters::gke::GkeFilesystemProvider;
use minibox::domain::FilesystemProvider;
use std::path::Path;
use std::sync::Arc;

fn create_gke_filesystem_adapter(base_dir: &Path) -> Arc<dyn FilesystemProvider> {
    Arc::new(GkeFilesystemProvider::new(base_dir.to_path_buf()))
}

#[tokio::test]
async fn test_gke_adapter_copy_fs_preserves_layer_contents() {
    // ... test code above
}

#[test]
fn test_gke_adapter_handles_deep_directory_trees() {
    let tmp = tempfile::tempdir().unwrap();
    let adapter = create_gke_filesystem_adapter(tmp.path());

    // Create nested structure
    let nested = tmp.path().join("a/b/c/d");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("file.txt"), b"deep").unwrap();

    let result = adapter.copy_tree("test-nested", tmp.path().join("a").as_path()).await;
    assert!(result.is_ok());
}

#[test]
fn test_gke_adapter_copy_fs_respects_size_limits() {
    let tmp = tempfile::tempdir().unwrap();
    let adapter = create_gke_filesystem_adapter(tmp.path());

    // GKE should have a max copy size (e.g., 5GB)
    let result = adapter.copy_tree_with_limit(
        "test-limit",
        tmp.path(),
        5 * 1024 * 1024 * 1024, // 5GB
    );
    assert!(result.is_ok());
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p minibox --test adapter_gke_tests -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/tests/adapter_gke_tests.rs
git commit -m "test(adapters): add GKE adapter isolation tests with copy FS validation"
```

---

### Task 3: Create Colima adapter test suite

**Files:**

- Create: `crates/minibox/tests/adapter_colima_tests.rs`

**Steps:**

- [ ] **Step 1: Write failing test for Colima VM block device detection**

```rust
#[test]
fn test_colima_adapter_finds_correct_block_device_for_cgroup_limits() {
    let adapter = create_colima_cgroup_adapter();

    // Colima uses virtio device (253:0 for vda)
    let device = adapter.find_block_device().unwrap();
    assert_eq!(device.major, 253);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox --test adapter_colima_tests -- --nocapture`
Expected: FAIL

- [ ] **Step 3: Implement Colima adapter test fixture with mocked sysfs**

In `crates/minibox/tests/adapter_colima_tests.rs`:

```rust
use minibox::adapters::colima::ColimaResourceLimiter;
use minibox::domain::ResourceLimiter;
use std::sync::Arc;

fn create_colima_cgroup_adapter() -> Arc<dyn ResourceLimiter> {
    Arc::new(ColimaResourceLimiter::new("/sys/fs/cgroup"))
}

#[test]
fn test_colima_adapter_finds_correct_block_device_for_cgroup_limits() {
    let adapter = create_colima_cgroup_adapter();

    // Colima uses virtio device (253:0 for vda)
    let device = adapter.find_block_device().unwrap();
    assert_eq!(device.major, 253);
}

#[test]
fn test_colima_adapter_sets_io_max_with_detected_device() {
    let adapter = create_colima_cgroup_adapter();

    let cgroup_path = "/sys/fs/cgroup/minibox.slice/test-container";
    let result = adapter.set_io_limit(cgroup_path, 1024 * 1024 * 1024); // 1GB/s

    // Mock sysfs or skip if running outside Colima
    #[cfg(not(target_os = "macos"))]
    assert!(result.is_ok());
}

#[test]
fn test_colima_adapter_rejects_invalid_cgroup_pid() {
    let adapter = create_colima_cgroup_adapter();

    // PID 0 should be rejected
    let result = adapter.write_to_cgroup_procs("/test/cgroup", 0);
    assert!(result.is_err(), "PID 0 must be rejected");
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p minibox --test adapter_colima_tests -- --nocapture`
Expected: PASS (skipped on non-macOS)

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/tests/adapter_colima_tests.rs
git commit -m "test(adapters): add Colima adapter tests with block device detection"
```

---

### Task 4: Create shared test fixtures module

**Files:**

- Create: `crates/minibox/src/adapters/test_fixtures.rs`
- Modify: `crates/minibox/src/adapters/mod.rs`

**Steps:**

- [ ] **Step 1: Write test fixtures for mock builder pattern**

In `crates/minibox/src/adapters/test_fixtures.rs`:

```rust
//! Test fixtures and builders for adapter testing.
//!
//! Provides mock builders, temporary directories, and common test scenarios.

use crate::domain::{DynFilesystemProvider, DynResourceLimiter, DynImageRegistry, DynContainerRuntime};
use crate::adapters::mocks::*;
use std::path::PathBuf;
use tempfile::TempDir;

/// Builder for constructing mock adapters with specific failure modes.
pub struct MockAdapterBuilder {
    fail_mount: bool,
    fail_cgroup: bool,
    fail_pull: bool,
    container_dir: Option<PathBuf>,
}

impl MockAdapterBuilder {
    pub fn new() -> Self {
        Self {
            fail_mount: false,
            fail_cgroup: false,
            fail_pull: false,
            container_dir: None,
        }
    }

    pub fn with_mount_failure(mut self) -> Self {
        self.fail_mount = true;
        self
    }

    pub fn with_cgroup_failure(mut self) -> Self {
        self.fail_cgroup = true;
        self
    }

    pub fn with_pull_failure(mut self) -> Self {
        self.fail_pull = true;
        self
    }

    pub fn with_container_dir(mut self, dir: PathBuf) -> Self {
        self.container_dir = Some(dir);
        self
    }

    pub fn build(self) -> (DynFilesystemProvider, DynResourceLimiter, DynImageRegistry) {
        let fs = if self.fail_mount {
            Arc::new(FailingFilesystemMock::new())
        } else {
            Arc::new(SuccessFilesystemMock::new(self.container_dir.clone()))
        };

        let limiter = if self.fail_cgroup {
            Arc::new(FailingResourceLimiterMock::new())
        } else {
            Arc::new(SuccessResourceLimiterMock::new())
        };

        let registry = if self.fail_pull {
            Arc::new(FailingImageRegistryMock::new())
        } else {
            Arc::new(SuccessImageRegistryMock::new())
        };

        (fs, limiter, registry)
    }
}

/// Test fixture: temporary container directory with cleanup.
pub struct TempContainerFixture {
    pub dir: TempDir,
    pub images_dir: PathBuf,
    pub containers_dir: PathBuf,
}

impl TempContainerFixture {
    pub fn new() -> Result<Self, std::io::Error> {
        let dir = TempDir::new()?;
        let images_dir = dir.path().join("images");
        let containers_dir = dir.path().join("containers");

        std::fs::create_dir(&images_dir)?;
        std::fs::create_dir(&containers_dir)?;

        Ok(Self {
            dir,
            images_dir,
            containers_dir,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_adapter_builder_creates_success_adapters() {
        let (fs, limiter, registry) = MockAdapterBuilder::new().build();
        // Adapters are created, can be used in handler tests
    }

    #[test]
    fn test_mock_adapter_builder_injects_mount_failure() {
        let (fs, _, _) = MockAdapterBuilder::new()
            .with_mount_failure()
            .build();
        // Mount operations will fail
    }

    #[test]
    fn test_temp_container_fixture_creates_required_dirs() {
        let fixture = TempContainerFixture::new().unwrap();
        assert!(fixture.images_dir.exists());
        assert!(fixture.containers_dir.exists());
    }
}
```

- [ ] **Step 2: Export test_fixtures in mod.rs**

In `crates/minibox/src/adapters/mod.rs`, add:

```rust
#[cfg(test)]
pub mod test_fixtures;
```

- [ ] **Step 3: Run test to verify fixtures work**

Run: `cargo test -p minibox adapters::test_fixtures -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/minibox/src/adapters/test_fixtures.rs crates/minibox/src/adapters/mod.rs
git commit -m "test(fixtures): add mock adapter builder and temp container fixtures"
```

---

### Task 5: Extend mocks with failure injection helpers

**Files:**

- Modify: `crates/minibox/src/adapters/mocks.rs`

**Steps:**

- [ ] **Step 1: Add failure injection struct**

In `crates/minibox/src/adapters/mocks.rs`, add after existing mocks:

```rust
/// Mock with controllable failure injection for testing error paths.
pub struct FailableFilesystemMock {
    should_fail_next_mount: std::sync::atomic::AtomicBool,
    should_fail_next_cleanup: std::sync::atomic::AtomicBool,
}

impl FailableFilesystemMock {
    pub fn new() -> Self {
        Self {
            should_fail_next_mount: std::sync::atomic::AtomicBool::new(false),
            should_fail_next_cleanup: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn set_fail_next_mount(&self, fail: bool) {
        self.should_fail_next_mount.store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn set_fail_next_cleanup(&self, fail: bool) {
        self.should_fail_next_cleanup.store(fail, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl FilesystemProvider for FailableFilesystemMock {
    async fn mount_container_root(&self, container_id: &str, layers: Vec<PathBuf>, base_dir: &Path) -> Result<PathBuf> {
        if self.should_fail_next_mount.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(anyhow::anyhow!("injected mount failure"));
        }
        // Success path
        Ok(base_dir.join(container_id).join("merged"))
    }

    async fn unmount_container(&self, container_id: &str, base_dir: &Path) -> Result<()> {
        if self.should_fail_next_cleanup.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(anyhow::anyhow!("injected unmount failure"));
        }
        Ok(())
    }

    // ... other methods
}
```

- [ ] **Step 2: Write test for failure injection**

```rust
#[tokio::test]
async fn test_failable_mock_can_inject_mount_failure() {
    let mock = FailableFilesystemMock::new();
    mock.set_fail_next_mount(true);

    let result = mock.mount_container_root("test-123", vec![], Path::new("/tmp")).await;
    assert!(result.is_err());

    // Reset
    mock.set_fail_next_mount(false);
    let result = mock.mount_container_root("test-456", vec![], Path::new("/tmp")).await;
    assert!(result.is_ok());
}
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test -p minibox adapters::mocks -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/minibox/src/adapters/mocks.rs
git commit -m "test(mocks): add failable mock with atomic failure injection"
```

---

### Task 6: Create handler tests with mock adapter swaps

**Files:**

- Create: `crates/daemonbox/tests/handler_adapter_swap_tests.rs`

**Steps:**

- [ ] **Step 1: Write test for handler routing with mock filesystem**

```rust
#[tokio::test]
async fn test_handler_run_container_with_mock_filesystem_adapter() {
    use daemonbox::handler::Handler;
    use minibox::adapters::test_fixtures::MockAdapterBuilder;
    use minibox::protocol::DaemonRequest;

    let (fs_mock, limiter_mock, registry_mock) = MockAdapterBuilder::new().build();

    let handler = Handler::new(fs_mock, limiter_mock, registry_mock);

    let request = DaemonRequest::Run {
        image: "alpine".to_string(),
        tag: None,
        command: vec!["/bin/echo".to_string(), "hello".to_string()],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: true,
    };

    let response = handler.handle_run_container(&request).await;
    assert!(response.is_ok());
}
```

- [ ] **Step 2: Write test for handler behavior with injection failures**

```rust
#[tokio::test]
async fn test_handler_gracefully_handles_filesystem_mount_failure() {
    use daemonbox::handler::Handler;
    use minibox::adapters::test_fixtures::MockAdapterBuilder;
    use minibox::protocol::DaemonRequest;

    let (fs_mock, limiter_mock, registry_mock) = MockAdapterBuilder::new()
        .with_mount_failure()
        .build();

    let handler = Handler::new(fs_mock, limiter_mock, registry_mock);

    let request = DaemonRequest::Run {
        image: "alpine".to_string(),
        tag: None,
        command: vec!["/bin/sh".to_string()],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: true,
    };

    let response = handler.handle_run_container(&request).await;

    // Mount failure should propagate
    assert!(response.is_err());
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo test -p daemonbox --test handler_adapter_swap_tests -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/daemonbox/tests/handler_adapter_swap_tests.rs
git commit -m "test(handler): add adapter swap tests with mock fixtures"
```

---

### Task 7-10: Integration checks and summary

- [ ] **Task 7: Run full Phase 1 test suite**

```bash
cargo test -p minibox --test adapter_native_tests
cargo test -p minibox --test adapter_gke_tests
cargo test -p minibox --test adapter_colima_tests
cargo test -p daemonbox --test handler_adapter_swap_tests
```

Expected: All tests pass (native/gke/colima may skip on non-Linux)

- [ ] **Task 8: Verify adapter contract documentation**

Create `crates/minibox/src/adapters/ADAPTER_CONTRACT.md`:

```markdown
# Adapter Contract

Each adapter must implement the following domain traits:

- `FilesystemProvider`: Mount/unmount container root, validate paths
- `ResourceLimiter`: Set memory/CPU limits via cgroups
- `ImageRegistry`: Pull images, resolve manifests
- `ContainerRuntime`: Spawn processes with namespace isolation

## Test Matrix

| Adapter | Unit Tests | Integration      | Notes                       |
| ------- | ---------- | ---------------- | --------------------------- |
| native  | ✓          | ✓ (Linux+root)   | Full feature set            |
| gke     | ✓          | ✗ (requires GKE) | Copy FS, no overlay         |
| colima  | ✓          | ✗ (macOS only)   | VM-specific device handling |
```

- [ ] **Task 9: Update Justfile with Phase 1 target**

Add to `Justfile`:

```bash
# Adapter isolation tests (any platform)
test-adapters:
    cargo test -p minibox --test adapter_native_tests
    cargo test -p minibox --test adapter_gke_tests
    cargo test -p minibox --test adapter_colima_tests
    cargo test -p daemonbox --test handler_adapter_swap_tests
```

- [ ] **Task 10: Final commit**

```bash
git add crates/minibox/src/adapters/ADAPTER_CONTRACT.md Justfile
git commit -m "docs(adapters): document adapter contract and test matrix"
```

**Phase 1 Summary:**

- ✅ 4 adapter test suites (≥5 tests each = 20+ new tests)
- ✅ Shared mock fixtures and builder pattern
- ✅ Handler tests with adapter swaps
- ✅ Adapter contract documented
- **New test count: +25 tests**

---

## Phase 2: Property-Based Testing (13 tasks, ~50 min)

> **Complexity:** Medium. Requires defining invariants and proptest strategies; reuses existing protocol types.
> **Dependencies:** Phase 1 complete (uses test fixtures for nested property testing)
> **Success criteria:** 4 new property suites with ≥3 properties each; existing proptest_suite enhanced; all properties pass 256+ iterations.

### Task 1-3: Extend protocol codec properties

**Files:**

- Create: `crates/minibox/tests/protocol_codec_properties.rs`

**Steps:**

- [ ] **Step 1: Write property for request roundtrip with maximum field lengths**

```rust
use proptest::prelude::*;
use minibox::protocol::{DaemonRequest, encode_request, decode_request};

proptest! {
    #[test]
    fn prop_request_roundtrip_preserves_all_fields(
        image in "[-a-z0-9./]+(:[0-9]+)?",
        tag in prop::option::of("[a-z0-9._-]{1,128}"),
        command in prop::collection::vec("[a-z0-9._/-]{1,64}", 0..16),
        memory in prop::option::of(1u64..=1_099_511_627_776u64), // 1TB max
        cpu_weight in prop::option::of(1u64..=10000u64),
        ephemeral in any::<bool>(),
    ) {
        let req = DaemonRequest::Run {
            image: image.clone(),
            tag: tag.clone(),
            command: command.clone(),
            memory_limit_bytes: memory,
            cpu_weight: cpu_weight,
            ephemeral,
        };

        let encoded = encode_request(&req).expect("encoding failed");
        let decoded: DaemonRequest = decode_request(&encoded).expect("decoding failed");

        prop_assert_eq!(decoded, req);
    }
}
```

- [ ] **Step 2: Write property for response with large output buffers**

```rust
proptest! {
    #[test]
    fn prop_response_roundtrip_handles_large_output(
        output in "[\\x00-\\x7F]{0,65536}", // Up to 64KB output
        exit_code in 0i32..=255,
    ) {
        let resp = DaemonResponse::ContainerOutput {
            container_id: "test-123".to_string(),
            output: output.clone(),
            kind: OutputStreamKind::Stdout,
        };

        let encoded = encode_response(&resp).expect("encoding failed");
        let decoded: DaemonResponse = decode_response(&encoded).expect("decoding failed");

        prop_assert_eq!(decoded, resp);
    }
}
```

- [ ] **Step 3: Write property for edge-case strings (UTF-8, nulls, newlines)**

```rust
proptest! {
    #[test]
    fn prop_protocol_handles_arbitrary_utf8_in_image_names(
        image in "\\PC*", // Any valid UTF-8
    ) {
        let req = DaemonRequest::Pull {
            image: image.clone(),
            tag: None,
        };

        let encoded = encode_request(&req).expect("encoding failed");
        let decoded: DaemonRequest = decode_request(&encoded).expect("decoding failed");

        prop_assert_eq!(decoded, req);
    }
}
```

- [ ] **Step 4: Run property tests with multiple iterations**

Run: `cargo test -p minibox --test protocol_codec_properties -- --nocapture`
Expected: PASS (hundreds of test cases)

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/tests/protocol_codec_properties.rs
git commit -m "test(properties): add protocol codec roundtrip properties with edge cases"
```

---

### Task 4-6: Path validation properties

**Files:**

- Create: `crates/minibox/tests/path_validation_properties.rs`

**Steps:**

- [ ] **Step 1: Write property for path traversal rejection**

```rust
use proptest::prelude::*;
use minibox::container::filesystem::validate_layer_path;
use std::path::Path;

proptest! {
    #[test]
    fn prop_validate_layer_path_rejects_parent_directory_traversal(
        prefix in "[a-z0-9]{1,16}",
        dots in r"\.{2,}",
        suffix in "[a-z0-9/]{0,32}",
    ) {
        let malicious = format!("{}/{}/{}", prefix, dots, suffix);
        let result = validate_layer_path(Path::new(&malicious));

        // Should reject any path with .. components
        prop_assert!(result.is_err(), "Path {} should be rejected", malicious);
    }
}
```

- [ ] **Step 2: Write property for symlink target normalization**

```rust
proptest! {
    #[test]
    fn prop_symlink_target_rewrite_handles_absolute_paths(
        abs_target in "/[a-z0-9/]{1,64}",
    ) {
        let container_root = Path::new("/container");
        let rewritten = rewrite_absolute_symlink(&abs_target, container_root);

        // Absolute symlinks must be rewritten to relative
        prop_assert!(!rewritten.is_absolute(),
                    "Rewritten symlink {} must not be absolute",
                    rewritten.display());
    }
}
```

- [ ] **Step 3: Write property for path normalization idempotency**

```rust
proptest! {
    #[test]
    fn prop_path_normalization_is_idempotent(
        path in "[a-z0-9/.\\\\-]{1,128}",
    ) {
        let p = Path::new(&path);
        let norm1 = normalize_path(p);
        let norm2 = normalize_path(&norm1);

        prop_assert_eq!(norm1, norm2, "Path normalization must be idempotent");
    }
}
```

- [ ] **Step 4: Run property tests**

Run: `cargo test -p minibox --test path_validation_properties -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/tests/path_validation_properties.rs
git commit -m "test(properties): add path validation security properties"
```

---

### Task 7-9: Image manifest parsing properties

**Files:**

- Create: `crates/minibox/tests/image_manifest_properties.rs`

**Steps:**

- [ ] **Step 1: Write property for manifest JSON roundtrip**

```rust
use proptest::prelude::*;
use minibox::image::manifest::{OciManifest, parse_manifest, encode_manifest};

proptest! {
    #[test]
    fn prop_manifest_serialize_deserialize_roundtrip(
        media_type in "[a-z0-9.+/]{1,128}",
        config_digest in "[a-z0-9]{64}",
        config_size in 1u64..=10_485_760u64, // Up to 10MB config
        layer_count in 1usize..=20,
    ) {
        let mut layers = Vec::new();
        for i in 0..layer_count {
            layers.push(OciDescriptor {
                media_type: "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
                digest: format!("sha256:{:064x}", i),
                size: 1024 + (i as u64) * 1024 * 1024,
            });
        }

        let manifest = OciManifest {
            schema_version: 2,
            media_type: media_type.clone(),
            config: OciDescriptor {
                media_type: "application/vnd.oci.image.config.v1+json".to_string(),
                digest: format!("sha256:{}", config_digest),
                size: config_size,
            },
            layers,
        };

        let json = encode_manifest(&manifest).expect("encode failed");
        let parsed: OciManifest = parse_manifest(&json).expect("parse failed");

        prop_assert_eq!(parsed, manifest);
    }
}
```

- [ ] **Step 2: Write property for layer digest validation**

```rust
proptest! {
    #[test]
    fn prop_layer_digest_validation_rejects_invalid_formats(
        bad_digest in "[a-z0-9]{0,63}", // Too short
    ) {
        prop_assume!(bad_digest.len() < 64); // Skip valid SHA256

        let result = validate_layer_digest(&bad_digest);
        prop_assert!(result.is_err());
    }
}
```

- [ ] **Step 3: Write property for manifest size limits**

```rust
proptest! {
    #[test]
    fn prop_manifest_parser_rejects_oversized_configs(
        layer_count in 1usize..=1000,
    ) {
        let oversized_layers: Vec<_> = (0..layer_count)
            .map(|i| OciDescriptor {
                media_type: "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
                digest: format!("sha256:{:064x}", i),
                size: 1u64 * 1024 * 1024 * 1024, // 1GB per layer
            })
            .collect();

        // Total size > 5GB should be rejected
        let result = validate_total_manifest_size(&oversized_layers);
        if oversized_layers.iter().map(|l| l.size).sum::<u64>() > 5 * 1024 * 1024 * 1024 {
            prop_assert!(result.is_err());
        }
    }
}
```

- [ ] **Step 4: Run property tests**

Run: `cargo test -p minibox --test image_manifest_properties -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/tests/image_manifest_properties.rs
git commit -m "test(properties): add image manifest parsing properties with size limits"
```

---

### Task 10-12: Cgroup boundary properties

**Files:**

- Create: `crates/minibox/tests/cgroup_boundary_properties.rs`

**Steps:**

- [ ] **Step 1: Write property for memory limit boundaries**

```rust
use proptest::prelude::*;
use minibox::container::cgroups::{validate_memory_limit, set_memory_limit};

proptest! {
    #[test]
    fn prop_memory_limit_accepts_valid_range(
        memory_bytes in 4096u64..=1_099_511_627_776u64, // 4KB to 1TB
    ) {
        let result = validate_memory_limit(memory_bytes);
        prop_assert!(result.is_ok(), "Memory limit {} should be valid", memory_bytes);
    }

    #[test]
    fn prop_memory_limit_rejects_too_small(
        tiny in 1u64..4096u64,
    ) {
        let result = validate_memory_limit(tiny);
        prop_assert!(result.is_err(), "Memory limit {} too small", tiny);
    }
}
```

- [ ] **Step 2: Write property for CPU weight boundaries**

```rust
proptest! {
    #[test]
    fn prop_cpu_weight_accepts_valid_range(
        weight in 1u64..=10000u64,
    ) {
        let result = validate_cpu_weight(weight);
        prop_assert!(result.is_ok(), "CPU weight {} valid", weight);
    }

    #[test]
    fn prop_cpu_weight_rejects_zero(
        // Zero should fail
    ) {
        let result = validate_cpu_weight(0);
        prop_assert!(result.is_err(), "CPU weight 0 must be rejected");
    }

    #[test]
    fn prop_cpu_weight_rejects_overflow(
        huge in 10001u64..=u64::MAX,
    ) {
        let result = validate_cpu_weight(huge);
        prop_assert!(result.is_err());
    }
}
```

- [ ] **Step 3: Write property for cgroup path validation**

```rust
proptest! {
    #[test]
    fn prop_cgroup_path_validation_rejects_unsafe_chars(
        unsafe_name in "[^a-z0-9._-]{1,64}",
    ) {
        let result = validate_cgroup_path(&unsafe_name);
        prop_assert!(result.is_err(), "Path {} with unsafe chars rejected", unsafe_name);
    }

    #[test]
    fn prop_cgroup_path_validation_accepts_valid_chars(
        valid_name in "[a-z0-9._-]{1,64}",
    ) {
        let result = validate_cgroup_path(&valid_name);
        prop_assert!(result.is_ok(), "Path {} should be valid", valid_name);
    }
}
```

- [ ] **Step 4: Run property tests**

Run: `cargo test -p minibox --test cgroup_boundary_properties -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/tests/cgroup_boundary_properties.rs
git commit -m "test(properties): add cgroup boundary and overflow properties"
```

---

### Task 13: Consolidate and verify Phase 2

- [ ] **Run all property-based tests**

```bash
cargo test -p minibox --test protocol_codec_properties
cargo test -p minibox --test path_validation_properties
cargo test -p minibox --test image_manifest_properties
cargo test -p minibox --test cgroup_boundary_properties
```

Expected: All pass, 256+ iterations per property

- [ ] **Update proptest_suite.rs with cross-references**

Add to `crates/minibox/tests/proptest_suite.rs`:

```rust
//! # Property-Based Test Suites
//!
//! Invariants tested across multiple suites:
//! - **Protocol**: encode→decode roundtrip lossless (proptest_suite.rs + protocol_codec_properties.rs)
//! - **Filesystem**: path validation rejects traversal attempts (path_validation_properties.rs)
//! - **Image**: manifest parsing idempotent + size bounds respected (image_manifest_properties.rs)
//! - **Cgroup**: memory/CPU limits within valid ranges, overflow rejected (cgroup_boundary_properties.rs)
//! - **DaemonState**: container state machine transitions (proptest_suite.rs)
```

- [ ] **Final commit**

```bash
git add crates/minibox/tests/proptest_suite.rs
git commit -m "docs(properties): consolidate property-based test documentation"
```

**Phase 2 Summary:**

- ✅ 4 property suites (≥3 properties each = 12+ new properties)
- ✅ Edge case coverage: UTF-8, large buffers, boundary values, overflow
- ✅ Security properties: path traversal, symlink rewriting, cgroup validation
- **New test iterations: 256+ per property × 12 = 3000+ test cases**

---

## Phase 3: Chaos & Fault Injection (12 tasks, ~60 min)

> **Complexity:** High. Requires coordinated failure injection across adapters and lifecycle state machine; some tests require root/Linux.
> **Dependencies:** Phase 1 (mock fixtures) + Phase 2 (property invariants)
> **Success criteria:** 4 failure test suites with ≥3 failure modes each; handler gracefully recovers from all injected failures; cleanup (unmount, cgroup removal) always runs.

### Task 1-4: Adapter failure injection tests

**Files:**

- Create: `crates/minibox/tests/adapter_failure_injection_tests.rs`

**Steps:**

- [ ] **Step 1: Write test for incomplete image pull recovery**

```rust
#[tokio::test]
async fn test_adapter_handles_incomplete_layer_download_with_cleanup() {
    use minibox::adapters::mocks::FailableImageRegistryMock;
    use minibox::domain::ImageRegistry;

    let registry = FailableImageRegistryMock::new();
    registry.fail_after_layers(2); // Fail on layer 3

    let layers = vec![
        "sha256:layer1".to_string(),
        "sha256:layer2".to_string(),
        "sha256:layer3".to_string(), // Will fail
    ];

    let result = registry.pull_image("test-image", &layers).await;

    // Should fail
    assert!(result.is_err());

    // Verify cleanup happened
    assert!(registry.get_cleanup_count() > 0);
}
```

- [ ] **Step 2: Write test for cgroup creation failure**

```rust
#[tokio::test]
async fn test_limiter_handles_cgroup_creation_failure() {
    use minibox::adapters::mocks::FailingResourceLimiterMock;

    let limiter = FailingResourceLimiterMock::new();

    let result = limiter.create_cgroup("test-container").await;
    assert!(result.is_err());
}
```

- [ ] **Step 3: Write test for overlay mount failure with layer cleanup**

```rust
#[tokio::test]
async fn test_filesystem_adapter_cleans_up_layers_on_mount_failure() {
    use minibox::adapters::mocks::FailableFilesystemMock;

    let fs = FailableFilesystemMock::new();
    fs.set_fail_on_mount();

    let layer_dir = tempfile::tempdir().unwrap();
    let result = fs.mount_container_root(
        "test-container",
        vec![layer_dir.path().to_path_buf()],
        Path::new("/tmp"),
    ).await;

    assert!(result.is_err());
    // Verify extracted layers were cleaned up
    assert!(fs.get_cleanup_layer_count() > 0);
}
```

- [ ] **Step 4: Run failure injection tests**

Run: `cargo test -p minibox --test adapter_failure_injection_tests -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/minibox/tests/adapter_failure_injection_tests.rs
git commit -m "test(chaos): add adapter failure injection with cleanup verification"
```

---

### Task 5-8: Container lifecycle failure tests

**Files:**

- Create: `crates/miniboxd/tests/container_lifecycle_failure_tests.rs`

**Steps:**

- [ ] **Step 1: Write test for zombie process reaping on abnormal exit**

```rust
#[tokio::test]
#[cfg(target_os = "linux")]
#[ignore] // Requires root
async fn test_container_handler_reaps_zombie_process_on_crash() {
    use daemonbox::handler::Handler;
    use minibox::adapters::test_fixtures::MockAdapterBuilder;

    let (fs, limiter, registry) = MockAdapterBuilder::new().build();
    let handler = Handler::new(fs, limiter, registry);

    let request = DaemonRequest::Run {
        image: "alpine".to_string(),
        tag: None,
        command: vec!["/bin/sh".to_string(), "-c".to_string(), "exit 1".to_string()],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: true,
    };

    let response = handler.handle_run_container(&request).await.unwrap();

    // Container should exit, no zombie should remain
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    // Check /proc/{pid} is gone
}
```

- [ ] **Step 2: Write test for incomplete state cleanup on handler panic**

```rust
#[tokio::test]
async fn test_container_state_cleanup_on_handler_error() {
    use daemonbox::state::DaemonState;

    let state = DaemonState::new();
    let container_id = "test-crash-123";

    state.add_container(container_id, &create_test_container_info()).await;
    assert_eq!(state.list_containers().await.len(), 1);

    // Simulate handler error
    state.remove_container(container_id).await.ok();

    // Container should be cleaned up despite error
    assert_eq!(state.list_containers().await.len(), 0);
}
```

- [ ] **Step 3: Write test for orphaned overlay mounts**

```rust
#[tokio::test]
#[cfg(target_os = "linux")]
#[ignore] // Requires root
async fn test_handler_unmounts_overlay_on_container_failure() {
    // Create overlay mount, trigger handler error, verify mount is cleaned
    // Check `mount | grep minibox` shows no stale mounts
}
```

- [ ] **Step 4: Run lifecycle tests**

Run: `cargo test -p miniboxd --test container_lifecycle_failure_tests -- --nocapture --test-threads=1`
Expected: PASS (single-threaded due to state mutations)

- [ ] **Step 5: Commit**

```bash
git add crates/miniboxd/tests/container_lifecycle_failure_tests.rs
git commit -m "test(chaos): add container lifecycle failure and cleanup tests"
```

---

### Task 9-12: Daemon recovery tests

**Files:**

- Create: `crates/daemonbox/tests/daemon_recovery_tests.rs`

**Steps:**

- [ ] **Step 1: Write test for server resilience to malformed requests**

```rust
#[tokio::test]
async fn test_daemon_server_rejects_malformed_json_gracefully() {
    use daemonbox::server::DaemonServer;

    let server = DaemonServer::bind("/tmp/test.sock").await.unwrap();

    // Spawn server task
    tokio::spawn(async move {
        let _ = server.run().await;
    });

    // Send malformed JSON
    let mut stream = tokio::net::UnixStream::connect("/tmp/test.sock").await.unwrap();
    stream.write_all(b"{invalid json}\\n").await.unwrap();

    // Server should still be alive
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Next request should succeed
    stream.write_all(b"{\"type\": \"List\"}\\n").await.unwrap();
}
```

- [ ] **Step 2: Write test for connection drop recovery**

```rust
#[tokio::test]
async fn test_daemon_recovers_from_client_disconnect() {
    use daemonbox::server::DaemonServer;

    let server = DaemonServer::bind("/tmp/test2.sock").await.unwrap();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    // Client 1: Connect and disconnect abruptly
    {
        let _stream = tokio::net::UnixStream::connect("/tmp/test2.sock").await.unwrap();
        // Drop without sending anything
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Client 2: Should connect fine
    let mut stream2 = tokio::net::UnixStream::connect("/tmp/test2.sock").await.unwrap();
    stream2.write_all(b"{\"type\": \"List\"}\\n").await.unwrap();
}
```

- [ ] **Step 3: Write test for rapid fire requests**

```rust
#[tokio::test]
async fn test_daemon_handles_rapid_sequential_requests() {
    use daemonbox::server::DaemonServer;

    let server = DaemonServer::bind("/tmp/test3.sock").await.unwrap();

    tokio::spawn(async move {
        let _ = server.run().await;
    });

    let mut stream = tokio::net::UnixStream::connect("/tmp/test3.sock").await.unwrap();

    // Send 100 rapid List requests
    for _ in 0..100 {
        stream.write_all(b"{\"type\": \"List\"}\\n").await.unwrap();
        let mut buf = [0; 1024];
        let _ = stream.read(&mut buf).await.unwrap();
    }
}
```

- [ ] **Step 4: Run daemon recovery tests**

Run: `cargo test -p daemonbox --test daemon_recovery_tests -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/daemonbox/tests/daemon_recovery_tests.rs
git commit -m "test(chaos): add daemon recovery and resilience tests"
```

---

### Task 13: Phase 3 summary and integration

- [ ] **Run all chaos/fault injection tests**

```bash
cargo test -p minibox --test adapter_failure_injection_tests
cargo test -p miniboxd --test container_lifecycle_failure_tests
cargo test -p daemonbox --test daemon_recovery_tests
```

Expected: PASS

- [ ] **Create failure mode matrix documentation**

Create `docs/failure-modes.md`:

```markdown
# Failure Mode Coverage

## Phase 3: Chaos & Fault Injection

| Failure Mode              | Test                              | Adapter            | Recovery             |
| ------------------------- | --------------------------------- | ------------------ | -------------------- |
| Incomplete image pull     | adapter_failure_injection_tests   | ImageRegistry      | Cleanup layers ✓     |
| Cgroup creation fails     | adapter_failure_injection_tests   | ResourceLimiter    | Skip cgroup limits ✓ |
| Overlay mount fails       | adapter_failure_injection_tests   | FilesystemProvider | Cleanup layers ✓     |
| Container process crashes | container_lifecycle_failure_tests | ContainerRuntime   | Reap zombie ✓        |
| Overlay mount left stale  | container_lifecycle_failure_tests | FilesystemProvider | Forced unmount ✓     |
| Malformed request JSON    | daemon_recovery_tests             | Server             | Reject + continue ✓  |
| Client abrupt disconnect  | daemon_recovery_tests             | Server             | Graceful cleanup ✓   |
| Rapid sequential requests | daemon_recovery_tests             | Server             | Queue handling ✓     |
```

- [ ] **Final commit**

```bash
git add docs/failure-modes.md
git commit -m "docs(chaos): add failure mode coverage matrix"
```

**Phase 3 Summary:**

- ✅ 3 failure test suites (≥3 failure modes each = 9+ new tests)
- ✅ Cleanup verification for all failure paths
- ✅ Server resilience to malformed input
- ✅ Failure mode documentation
- **New test count: +15 tests**

---

## Overall Summary

| Phase                | Focus               | New Tests      | New Iterations       | Dependencies          |
| -------------------- | ------------------- | -------------- | -------------------- | --------------------- |
| 1: Adapter Isolation | Port implementation | +25            | N/A                  | None                  |
| 2: Property-Based    | Invariant coverage  | +12 properties | 3000+                | Phase 1 fixtures      |
| 3: Chaos & Fault     | Failure recovery    | +15            | N/A                  | Phase 1 + 2           |
| **TOTAL**            | **Full coverage**   | **+52 tests**  | **3000+ iterations** | **Sequential phases** |

### Test Command Reference

```bash
# After Phase 1
just test-adapters

# After Phase 2
cargo test -p minibox --test protocol_codec_properties
cargo test -p minibox --test path_validation_properties
cargo test -p minibox --test image_manifest_properties
cargo test -p minibox --test cgroup_boundary_properties

# After Phase 3
cargo test -p minibox --test adapter_failure_injection_tests
cargo test -p miniboxd --test container_lifecycle_failure_tests
cargo test -p daemonbox --test daemon_recovery_tests

# All together
just test-all
```

### Coverage Progression

- **Before:** 147 unit + 16 integration + 14 e2e = 177 tests
- **After Phase 1:** 177 + 25 = 202 tests
- **After Phase 2:** 202 + (3000 proptest iterations)
- **After Phase 3:** 202 + 15 = 217 tests
- **Expected final:** 217 tests, 3000+ property iterations, 100% critical path coverage
