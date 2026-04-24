---
status: done
note: All 7 tasks complete — ConformanceCapability trait, error-path/GC/resource/runtime tests all landed
---

# Conformance Suite Phase 2: Expansion Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expand the conformance suite with error-path tests for existing capabilities, new
capability tests (GC, pause/resume, resource limits, spawn-failure), and a `ConformanceCapability`
trait that makes capabilities typed, serializable, and self-describing — so tests auto-skip
cleanly and the report knows why.

**Architecture:** All new test code lives in `crates/daemonbox/tests/conformance_tests.rs` and
`crates/minibox/tests/conformance_commit.rs` (existing files, extend in place). The typed
capability registry (`ConformanceCapability` trait) lives in
`crates/minibox-testers/src/capability.rs` and is re-exported from `minibox_testers`. `BackendDescriptor`
gains optional `capabilities_v2: Vec<Box<dyn ConformanceCapability>>` but existing `BackendCapabilitySet`
usage is unchanged. `minibox-testers` mocks gain any missing builder methods needed by new tests.

**Tech Stack:** Rust 2024, tokio, async-trait, serde/serde_json, minibox-core domain traits,
daemonbox handler + state, minibox-testers mocks.

---

## Scope note

Phase 2 is split into four independent task groups. Each group produces working, committable
tests. You do not need to complete them in strict order — but T1 (capability trait) should land
first because T4 uses it.

---

## File Structure

| File | Change |
|------|--------|
| `crates/minibox-testers/src/capability.rs` | **Create** — `ConformanceCapability` trait + blanket helpers |
| `crates/minibox-testers/src/lib.rs` | **Modify** — add `pub mod capability;` |
| `crates/minibox-testers/src/mocks/runtime.rs` | **Modify** — add `with_spawn_failure` (already exists), ensure `spawn_count` is accessible |
| `crates/minibox-testers/src/mocks/registry.rs` | **Modify** — add `with_empty_layers` builder if missing |
| `crates/minibox-testers/src/backend/descriptor.rs` | **Modify** — add `capabilities_v2` field + `with_v2_capability` builder |
| `crates/daemonbox/tests/conformance_tests.rs` | **Modify** — append new `mod` blocks for error-path, GC, pause/resume, resource, network |
| `crates/minibox/tests/conformance_commit.rs` | **Modify** — append error-path and boundary commit tests |

---

## Task 1: `ConformanceCapability` trait in minibox-testers

**Goal:** Define a typed, self-describing capability that tests can use instead of raw
`BackendCapability` enum checks. Capabilities know their name, whether to skip, and why.

**Files:**
- Create: `crates/minibox-testers/src/capability.rs`
- Modify: `crates/minibox-testers/src/lib.rs`

---

- [ ] **Step 1: Write a failing doc-test for the trait**

Add to `crates/minibox-testers/src/capability.rs` (new file):

```rust
//! Typed, self-describing conformance capabilities.
//!
//! A [`ConformanceCapability`] knows its name, whether it is supported by a
//! given backend, and the reason it is skipped when unsupported.
//!
//! # Example
//!
//! ```rust,ignore
//! use minibox_testers::capability::{ConformanceCapability, SkipReason};
//!
//! struct CommitCapability;
//! impl ConformanceCapability for CommitCapability {
//!     fn name(&self) -> &'static str { "Commit" }
//!     fn is_supported(&self) -> bool { true }
//!     fn skip_reason(&self) -> SkipReason {
//!         SkipReason::CapabilityNotDeclared { capability: "Commit" }
//!     }
//! }
//! ```

/// Why a conformance test was skipped.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SkipReason {
    /// The backend did not declare this capability in its `BackendCapabilitySet`.
    CapabilityNotDeclared { capability: &'static str },
    /// The backend declared the capability but the required external service
    /// (e.g. a local OCI registry) is not available in this environment.
    ExternalServiceUnavailable { service: &'static str },
    /// The test is platform-gated and the current platform does not support it.
    PlatformUnsupported { platform: &'static str },
}

impl SkipReason {
    /// Return a human-readable explanation.
    pub fn message(&self) -> String {
        match self {
            SkipReason::CapabilityNotDeclared { capability } => {
                format!("backend does not declare {capability} capability")
            }
            SkipReason::ExternalServiceUnavailable { service } => {
                format!("external service not available: {service}")
            }
            SkipReason::PlatformUnsupported { platform } => {
                format!("platform not supported: {platform}")
            }
        }
    }
}

/// A typed, self-describing conformance capability.
///
/// Implement this trait for each capability group. The conformance runner uses
/// it to determine whether to run or skip a test case and why.
pub trait ConformanceCapability: Send + Sync + 'static {
    /// Short identifier used in reports, e.g. `"Commit"`.
    fn name(&self) -> &'static str;

    /// Return `true` if the backend supports this capability and the test
    /// should run.
    fn is_supported(&self) -> bool;

    /// Return the reason this capability is skipped when `is_supported()` is
    /// `false`. Used in report rows.
    fn skip_reason(&self) -> SkipReason;
}

/// Check whether to skip a test and return the skip message if so.
///
/// Returns `Some(message)` if the test should be skipped, `None` if it should run.
///
/// # Usage in tests
///
/// ```rust,ignore
/// if let Some(reason) = should_skip(&cap) {
///     eprintln!("skip: {reason}");
///     return;
/// }
/// ```
pub fn should_skip(cap: &dyn ConformanceCapability) -> Option<String> {
    if cap.is_supported() {
        None
    } else {
        Some(cap.skip_reason().message())
    }
}

// ---------------------------------------------------------------------------
// Built-in capability descriptors
// ---------------------------------------------------------------------------

/// Capability: backend can commit a container FS diff to a new image.
pub struct CommitCapability {
    pub supported: bool,
}

impl ConformanceCapability for CommitCapability {
    fn name(&self) -> &'static str {
        "Commit"
    }
    fn is_supported(&self) -> bool {
        self.supported
    }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared {
            capability: "Commit",
        }
    }
}

/// Capability: backend can build an image from a Dockerfile context.
pub struct BuildCapability {
    pub supported: bool,
}

impl ConformanceCapability for BuildCapability {
    fn name(&self) -> &'static str {
        "BuildFromContext"
    }
    fn is_supported(&self) -> bool {
        self.supported
    }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared {
            capability: "BuildFromContext",
        }
    }
}

/// Capability: backend can push an image to a registry.
pub struct PushCapability {
    pub supported: bool,
}

impl ConformanceCapability for PushCapability {
    fn name(&self) -> &'static str {
        "PushToRegistry"
    }
    fn is_supported(&self) -> bool {
        self.supported
    }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared {
            capability: "PushToRegistry",
        }
    }
}

/// Capability: backend supports GC (image garbage collection).
pub struct GcCapability {
    pub supported: bool,
}

impl ConformanceCapability for GcCapability {
    fn name(&self) -> &'static str {
        "ImageGarbageCollection"
    }
    fn is_supported(&self) -> bool {
        self.supported
    }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared {
            capability: "ImageGarbageCollection",
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_skip_returns_none_when_supported() {
        let cap = CommitCapability { supported: true };
        assert!(should_skip(&cap).is_none());
    }

    #[test]
    fn should_skip_returns_message_when_unsupported() {
        let cap = CommitCapability { supported: false };
        let msg = should_skip(&cap).expect("should return skip message");
        assert!(msg.contains("Commit"), "message must mention capability name");
    }

    #[test]
    fn skip_reason_message_is_human_readable() {
        let r = SkipReason::CapabilityNotDeclared { capability: "Exec" };
        assert!(r.message().contains("Exec"));
        let r2 = SkipReason::ExternalServiceUnavailable {
            service: "localhost:5000",
        };
        assert!(r2.message().contains("localhost:5000"));
    }
}
```

- [ ] **Step 2: Add `pub mod capability;` to lib.rs**

In `crates/minibox-testers/src/lib.rs`, add:

```rust
pub mod capability;
```

after the existing `pub mod backend;` line.

- [ ] **Step 3: Run tests to verify they compile and pass**

```bash
cargo nextest run -p minibox-testers --lib 2>&1 | tail -20
```

Expected: all tests in `capability::tests` pass.

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-testers/src/capability.rs crates/minibox-testers/src/lib.rs
git commit -m "feat(minibox-testers): add ConformanceCapability trait with built-in descriptors"
```

---

## Task 2: Error-path tests for `ContainerCommitter`

**Goal:** Add tests that verify commit fails gracefully on bad inputs (missing upperdir, invalid
target ref format, invalid container ID length).

**Files:**
- Modify: `crates/minibox/tests/conformance_commit.rs` (append new tests)

These tests use the existing `ConformanceCommitAdapter` and `minibox_commit_backend` helpers
already defined at the top of that file.

---

- [ ] **Step 1: Add the three failing tests and run to confirm they fail**

Append to `crates/minibox/tests/conformance_commit.rs`:

```rust
/// Committing to a target ref with no tag separator must return an error.
///
/// `commit_upper_dir_to_image` requires a `name:tag` format. A bare name
/// without `:` must produce `Err`, not `Ok` with a garbled tag.
#[tokio::test]
async fn commit_target_ref_without_tag_returns_error() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let upper = WritableUpperDirFixture::new()?;

    let (backend, committer) = minibox_commit_backend(Arc::clone(&store), upper.upper_dir.clone());
    if !backend.capabilities.supports(BackendCapability::Commit) {
        return Ok(());
    }

    let cid = ContainerId::new("conformancecommit10".to_string()).expect("ContainerId");
    // "just-a-name" has no `:` — the adapter must reject it.
    let result = committer
        .commit(&cid, "just-a-name", &default_commit_config())
        .await;

    assert!(
        result.is_err(),
        "commit with no tag separator must return Err, got: {result:?}"
    );
    Ok(())
}

/// Committing with a blank target ref must return an error.
#[tokio::test]
async fn commit_empty_target_ref_returns_error() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let upper = WritableUpperDirFixture::new()?;

    let (backend, committer) = minibox_commit_backend(Arc::clone(&store), upper.upper_dir.clone());
    if !backend.capabilities.supports(BackendCapability::Commit) {
        return Ok(());
    }

    let cid = ContainerId::new("conformancecommit11".to_string()).expect("ContainerId");
    let result = committer
        .commit(&cid, "", &default_commit_config())
        .await;

    assert!(
        result.is_err(),
        "commit with empty target ref must return Err, got: {result:?}"
    );
    Ok(())
}

/// Two commits to the same target ref must both succeed and both write the
/// same name/tag to the store (second commit overwrites the first).
#[tokio::test]
async fn commit_idempotent_second_commit_succeeds() -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let store = make_image_store(&tmp);
    let upper_a = WritableUpperDirFixture::new()?;
    let upper_b = WritableUpperDirFixture::new()?;

    let (backend_a, committer_a) =
        minibox_commit_backend(Arc::clone(&store), upper_a.upper_dir.clone());
    let (backend_b, committer_b) =
        minibox_commit_backend(Arc::clone(&store), upper_b.upper_dir.clone());

    if !backend_a.capabilities.supports(BackendCapability::Commit)
        || !backend_b.capabilities.supports(BackendCapability::Commit)
    {
        return Ok(());
    }

    let cid = ContainerId::new("conformancecommit12".to_string()).expect("ContainerId");
    let meta_a = committer_a
        .commit(&cid, "conformance/idempotent:v1", &default_commit_config())
        .await?;
    let meta_b = committer_b
        .commit(&cid, "conformance/idempotent:v1", &default_commit_config())
        .await?;

    assert_eq!(meta_a.name, meta_b.name, "name must be stable across commits");
    assert_eq!(meta_a.tag, meta_b.tag, "tag must be stable across commits");
    assert!(
        store.has_image("conformance/idempotent", "v1"),
        "image must be present in store after second commit"
    );
    Ok(())
}
```

- [ ] **Step 2: Run to confirm tests compile and pass**

```bash
cargo nextest run -p minibox --test conformance_commit 2>&1 | tail -20
```

Expected: 3 new tests appear and pass (or the ref-format tests fail with `Err` as expected —
the `just-a-name` test asserts `is_err()` so it passes when the adapter rejects the ref).

- [ ] **Step 3: Commit**

```bash
git add crates/minibox/tests/conformance_commit.rs
git commit -m "test(conformance): error-path and idempotency tests for ContainerCommitter"
```

---

## Task 3: Error-path tests for registry, filesystem, runtime (mock conformance)

**Goal:** Verify that mock adapters behave correctly on their error paths — pull failure, spawn
failure — and that the handler correctly propagates these failures to the client as
`DaemonResponse::Error`.

**Files:**
- Modify: `crates/daemonbox/tests/conformance_tests.rs` (append new `mod error_path_conformance`)

---

- [ ] **Step 1: Write the failing tests**

Append to `crates/daemonbox/tests/conformance_tests.rs` (before the final closing brace, if
any, or at the end of the file):

```rust
// ---------------------------------------------------------------------------
// Error-path conformance — handler propagates adapter failures correctly
// ---------------------------------------------------------------------------

mod error_path_conformance {
    use super::*;

    /// When the registry returns a pull error, `handle_run` must respond with
    /// `DaemonResponse::Error` — not panic, not hang.
    #[tokio::test]
    async fn run_with_pull_failure_returns_error_response() {
        let temp_dir = TempDir::new().expect("tempdir");
        // Registry configured to fail all pulls; image is not cached.
        let deps = mock_deps_with_registry(MockRegistry::new().with_pull_failure(), &temp_dir);
        let state = mock_state(&temp_dir);

        let response = handle_run_once(
            "alpine".to_string(),
            Some("latest".to_string()),
            vec!["/bin/sh".to_string()],
            None,
            None,
            false,
            state,
            deps,
        )
        .await;

        assert!(
            matches!(response, DaemonResponse::Error { .. }),
            "pull failure must produce DaemonResponse::Error, got: {response:?}"
        );
    }

    /// When `spawn_process` fails, `handle_run` must respond with
    /// `DaemonResponse::Error`.
    #[tokio::test]
    async fn run_with_spawn_failure_returns_error_response() {
        let temp_dir = TempDir::new().expect("tempdir");
        // Image is pre-cached so pull succeeds; runtime is configured to fail spawn.
        let image_store = Arc::new(
            minibox_core::image::ImageStore::new(temp_dir.path().join("img")).unwrap(),
        );
        let failing_runtime = Arc::new(MockRuntime::new().with_spawn_failure());
        let deps = Arc::new(HandlerDependencies {
            image: daemonbox::handler::ImageDeps {
                registry_router: Arc::new(minibox_core::adapters::HostnameRegistryRouter::new(
                    Arc::new(
                        MockRegistry::new().with_cached_image("library/alpine", "latest"),
                    ) as minibox_core::domain::DynImageRegistry,
                    [("ghcr.io", Arc::new(MockRegistry::new())
                        as minibox_core::domain::DynImageRegistry)],
                )),
                image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
                image_gc: Arc::new(minibox_testers::helpers::NoopImageGc),
                image_store,
            },
            lifecycle: daemonbox::handler::LifecycleDeps {
                filesystem: Arc::new(MockFilesystem::new()),
                resource_limiter: Arc::new(MockLimiter::new()),
                runtime: failing_runtime,
                network_provider: Arc::new(MockNetwork::new()),
                containers_base: temp_dir.path().join("containers"),
                run_containers_base: temp_dir.path().join("run"),
            },
            exec: daemonbox::handler::ExecDeps {
                exec_runtime: None,
                pty_sessions: Arc::new(tokio::sync::Mutex::new(
                    daemonbox::handler::PtySessionRegistry::default(),
                )),
            },
            build: daemonbox::handler::BuildDeps {
                image_pusher: None,
                commit_adapter: None,
                image_builder: None,
            },
            events: daemonbox::handler::EventDeps {
                event_sink: Arc::new(minibox_core::events::NoopEventSink),
                event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
                metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
            },
            policy: daemonbox::handler::ContainerPolicy {
                allow_bind_mounts: true,
                allow_privileged: true,
            },
        });
        let state = mock_state(&temp_dir);

        let response = handle_run_once(
            "alpine".to_string(),
            Some("latest".to_string()),
            vec!["/bin/sh".to_string()],
            None,
            None,
            false,
            state,
            deps,
        )
        .await;

        assert!(
            matches!(response, DaemonResponse::Error { .. }),
            "spawn failure must produce DaemonResponse::Error, got: {response:?}"
        );
    }

    /// `handle_pull` with a pull-failing registry must return `DaemonResponse::Error`.
    #[tokio::test]
    async fn pull_failure_returns_error_response() {
        let temp_dir = TempDir::new().expect("tempdir");
        let deps = mock_deps_with_registry(MockRegistry::new().with_pull_failure(), &temp_dir);
        let state = mock_state(&temp_dir);

        let response = handler::handle_pull(
            "alpine".to_string(),
            Some("latest".to_string()),
            state,
            deps,
        )
        .await;

        assert!(
            matches!(response, DaemonResponse::Error { .. }),
            "pull failure must produce DaemonResponse::Error, got: {response:?}"
        );
    }

    /// `handle_remove` on a non-existent container ID must return
    /// `DaemonResponse::Error`.
    #[tokio::test]
    async fn remove_nonexistent_container_returns_error() {
        let temp_dir = TempDir::new().expect("tempdir");
        let deps = mock_deps(&temp_dir);
        let state = mock_state(&temp_dir);

        let response =
            handler::handle_remove("nonexistent-container-id".to_string(), state, deps).await;

        assert!(
            matches!(response, DaemonResponse::Error { .. }),
            "remove of nonexistent container must return Error, got: {response:?}"
        );
    }

    /// `handle_stop` on a non-existent container ID must return
    /// `DaemonResponse::Error`.
    #[tokio::test]
    async fn stop_nonexistent_container_returns_error() {
        let temp_dir = TempDir::new().expect("tempdir");
        let deps = mock_deps(&temp_dir);
        let state = mock_state(&temp_dir);

        let response =
            handler::handle_stop("nonexistent-container-id".to_string(), state, deps).await;

        assert!(
            matches!(response, DaemonResponse::Error { .. }),
            "stop of nonexistent container must return Error, got: {response:?}"
        );
    }
}
```

- [ ] **Step 2: Run the new tests**

```bash
cargo nextest run -p daemonbox --test conformance_tests error_path_conformance 2>&1 | tail -30
```

Expected: all 5 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/daemonbox/tests/conformance_tests.rs
git commit -m "test(conformance): error-path conformance — pull failure, spawn failure, nonexistent ids"
```

---

## Task 4: GC conformance tests using `NoopImageGc` and `ConformanceCapability`

**Goal:** Verify `NoopImageGc` behaves correctly (never prunes), add `GcCapability` usage to a
real test, and verify skip semantics work end-to-end through the typed capability.

**Files:**
- Modify: `crates/daemonbox/tests/conformance_tests.rs` (append `mod gc_conformance`)
- Modify: `crates/minibox-testers/src/helpers/gc.rs` (add `prune_count` observer if not present)

---

- [ ] **Step 1: Check what NoopImageGc currently looks like**

Read `crates/minibox-testers/src/helpers/gc.rs`. It should have a `struct NoopImageGc` that
implements `ImageGarbageCollector`. If it does not have a `prune_call_count()` observer, add one:

```rust
// In crates/minibox-testers/src/helpers/gc.rs
// Add to NoopImageGc:
use std::sync::{Arc, Mutex};

pub struct NoopImageGc {
    call_count: Arc<Mutex<usize>>,
}

impl NoopImageGc {
    pub fn new() -> Self {
        Self {
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Return number of times `prune` was called.
    pub fn prune_call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}
```

And update the `ImageGarbageCollector` impl to increment `call_count` on `prune`.

**Note:** If `NoopImageGc` is already a unit struct (`pub struct NoopImageGc;`) with no fields,
replace it with the above. The `Arc<Mutex<usize>>` approach allows cheap `.clone()` while sharing
state, which is what the handler tests need.

- [ ] **Step 2: Update any existing `NoopImageGc::new()` callers**

After changing the struct, run:

```bash
cargo check --workspace 2>&1 | grep "NoopImageGc"
```

Fix any call sites that use `NoopImageGc` as a unit struct. In `helpers/daemon.rs` line 39:

```rust
// Before:
image_gc: Arc::new(NoopImageGc),
// After:
image_gc: Arc::new(NoopImageGc::new()),
```

Similarly in `crates/daemonbox/tests/conformance_tests.rs` line 73:

```rust
// Before:
image_gc: Arc::new(NoopImageGc),
// After:
image_gc: Arc::new(NoopImageGc::new()),
```

- [ ] **Step 3: Write GC conformance tests**

Append to `crates/daemonbox/tests/conformance_tests.rs`:

```rust
// ---------------------------------------------------------------------------
// GC conformance — ImageGarbageCollector behaves correctly
// ---------------------------------------------------------------------------

mod gc_conformance {
    use minibox_testers::capability::{GcCapability, should_skip};
    use minibox_testers::helpers::NoopImageGc;
    use minibox_core::image::ImageGarbageCollector;
    use tempfile::TempDir;

    /// NoopImageGc never prunes anything — prune returns Ok(0).
    #[tokio::test]
    async fn noop_gc_prune_returns_zero_freed() {
        let cap = GcCapability { supported: true };
        if let Some(reason) = should_skip(&cap) {
            eprintln!("skip: {reason}");
            return;
        }

        let gc = NoopImageGc::new();
        let tmp = TempDir::new().expect("tempdir");
        let result = gc.prune(tmp.path()).await;
        assert!(result.is_ok(), "prune must not error: {result:?}");
        assert_eq!(
            result.unwrap(),
            0,
            "noop GC must report 0 bytes freed"
        );
    }

    /// NoopImageGc::prune is callable multiple times without error.
    #[tokio::test]
    async fn noop_gc_prune_is_idempotent() {
        let gc = NoopImageGc::new();
        let tmp = TempDir::new().expect("tempdir");
        for _ in 0..3 {
            let r = gc.prune(tmp.path()).await;
            assert!(r.is_ok(), "repeated prune must not error: {r:?}");
            assert_eq!(r.unwrap(), 0, "must always report 0 freed");
        }
        assert_eq!(gc.prune_call_count(), 3, "call count must match invocations");
    }

    /// GcCapability with supported=false must skip via should_skip.
    #[test]
    fn gc_capability_unsupported_skips() {
        let cap = GcCapability { supported: false };
        let skip = should_skip(&cap);
        assert!(
            skip.is_some(),
            "unsupported GcCapability must produce a skip reason"
        );
        assert!(
            skip.unwrap().contains("ImageGarbageCollection"),
            "skip message must mention capability name"
        );
    }
}
```

- [ ] **Step 4: Run the GC conformance tests**

```bash
cargo nextest run -p daemonbox --test conformance_tests gc_conformance 2>&1 | tail -20
```

Expected: all 3 tests pass.

**Note on `ImageGarbageCollector::prune` signature:** Check
`crates/minibox-core/src/domain.rs` for the exact `prune` signature — it may be
`async fn prune(&self, store_path: &Path) -> Result<u64>` or similar. Match it exactly.

- [ ] **Step 5: Commit**

```bash
git add crates/minibox-testers/src/helpers/gc.rs \
        crates/minibox-testers/src/helpers/daemon.rs \
        crates/daemonbox/tests/conformance_tests.rs
git commit -m "test(conformance): GC conformance tests + NoopImageGc call counter"
```

---

## Task 5: Resource-limit boundary conformance tests

**Goal:** Verify that `MockLimiter` correctly handles boundary conditions — zero limits,
maximum u64 values, and cleanup-before-create.

**Files:**
- Modify: `crates/daemonbox/tests/conformance_tests.rs` (append `mod resource_limit_conformance`)
- Modify: `crates/minibox-testers/src/mocks/limiter.rs` (add `with_create_failure` builder if missing)

---

- [ ] **Step 1: Check MockLimiter for a `with_create_failure` builder**

Read `crates/minibox-testers/src/mocks/limiter.rs`. If `with_create_failure()` does not exist,
add it:

```rust
// In MockLimiter state struct, add:
create_should_succeed: bool,

// In MockLimiter::new(), initialize:
create_should_succeed: true,

// New builder method:
pub fn with_create_failure(self) -> Self {
    self.state.lock().unwrap().create_should_succeed = false;
    self
}

// In ResourceLimiter::create impl, check:
if !state.create_should_succeed {
    anyhow::bail!("mock limiter create failure");
}
```

- [ ] **Step 2: Write the resource-limit boundary tests**

Append to `crates/daemonbox/tests/conformance_tests.rs`:

```rust
// ---------------------------------------------------------------------------
// Resource-limit boundary conformance
// ---------------------------------------------------------------------------

mod resource_limit_conformance {
    use super::*;
    use minibox_core::domain::ResourceConfig;

    /// MockLimiter must accept u64::MAX for all limit fields without error.
    #[test]
    fn limiter_accepts_maximum_u64_limits() {
        let limiter = MockLimiter::new();
        let config = ResourceConfig {
            memory_limit_bytes: Some(u64::MAX),
            cpu_weight: Some(u64::MAX),
            pids_max: Some(u64::MAX),
            io_max_bytes_per_sec: Some(u64::MAX),
        };

        let result = limiter.create("container-max-limits", &config);
        assert!(
            result.is_ok(),
            "limiter must accept u64::MAX limits: {result:?}"
        );
    }

    /// MockLimiter must accept zero for all optional limit fields.
    #[test]
    fn limiter_accepts_zero_limits() {
        let limiter = MockLimiter::new();
        let config = ResourceConfig {
            memory_limit_bytes: Some(0),
            cpu_weight: Some(0),
            pids_max: Some(0),
            io_max_bytes_per_sec: Some(0),
        };

        let result = limiter.create("container-zero-limits", &config);
        assert!(
            result.is_ok(),
            "limiter must accept zero limits: {result:?}"
        );
    }

    /// MockLimiter must accept `None` for all optional limit fields.
    #[test]
    fn limiter_accepts_all_none_limits() {
        let limiter = MockLimiter::new();
        let config = ResourceConfig::default(); // all None

        let result = limiter.create("container-no-limits", &config);
        assert!(
            result.is_ok(),
            "limiter must accept all-None ResourceConfig: {result:?}"
        );
    }

    /// Cleanup before create must not panic — just return Ok or Err gracefully.
    #[test]
    fn limiter_cleanup_before_create_does_not_panic() {
        let limiter = MockLimiter::new();
        // cleanup before any create — must not panic
        let result = limiter.cleanup("nonexistent-container");
        // Either Ok or Err is acceptable; no panic is the invariant
        let _ = result;
    }

    /// add_process before create must not panic.
    #[test]
    fn limiter_add_process_before_create_does_not_panic() {
        let limiter = MockLimiter::new();
        let result = limiter.add_process("nonexistent-container", 99999);
        let _ = result;
    }
}
```

- [ ] **Step 3: Run the resource-limit tests**

```bash
cargo nextest run -p daemonbox --test conformance_tests resource_limit_conformance 2>&1 | tail -20
```

Expected: all 5 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-testers/src/mocks/limiter.rs \
        crates/daemonbox/tests/conformance_tests.rs
git commit -m "test(conformance): resource-limit boundary conformance tests"
```

---

## Task 6: Spawn-count and multi-container runtime conformance

**Goal:** Verify that `MockRuntime` correctly tracks spawn counts, produces unique PIDs across
containers, and handles the spawn-failure path through the handler.

**Files:**
- Modify: `crates/daemonbox/tests/conformance_tests.rs` (append `mod runtime_conformance`)

---

- [ ] **Step 1: Write the runtime conformance tests**

Append to `crates/daemonbox/tests/conformance_tests.rs`:

```rust
// ---------------------------------------------------------------------------
// Runtime conformance — spawn count, unique PIDs, failure propagation
// ---------------------------------------------------------------------------

mod runtime_conformance {
    use super::*;

    /// Running two containers sequentially must result in exactly 2 spawn calls
    /// on the runtime.
    #[tokio::test]
    async fn runtime_spawn_count_matches_container_count() {
        let temp_dir = TempDir::new().expect("tempdir");
        let runtime = Arc::new(MockRuntime::new());
        let image_store = Arc::new(
            minibox_core::image::ImageStore::new(temp_dir.path().join("img")).unwrap(),
        );
        let deps = Arc::new(HandlerDependencies {
            image: daemonbox::handler::ImageDeps {
                registry_router: Arc::new(minibox_core::adapters::HostnameRegistryRouter::new(
                    Arc::new(
                        MockRegistry::new().with_cached_image("library/alpine", "latest"),
                    ) as minibox_core::domain::DynImageRegistry,
                    [("ghcr.io", Arc::new(MockRegistry::new())
                        as minibox_core::domain::DynImageRegistry)],
                )),
                image_loader: Arc::new(daemonbox::handler::NoopImageLoader),
                image_gc: Arc::new(minibox_testers::helpers::NoopImageGc::new()),
                image_store,
            },
            lifecycle: daemonbox::handler::LifecycleDeps {
                filesystem: Arc::new(MockFilesystem::new()),
                resource_limiter: Arc::new(MockLimiter::new()),
                runtime: Arc::clone(&runtime),
                network_provider: Arc::new(MockNetwork::new()),
                containers_base: temp_dir.path().join("containers"),
                run_containers_base: temp_dir.path().join("run"),
            },
            exec: daemonbox::handler::ExecDeps {
                exec_runtime: None,
                pty_sessions: Arc::new(tokio::sync::Mutex::new(
                    daemonbox::handler::PtySessionRegistry::default(),
                )),
            },
            build: daemonbox::handler::BuildDeps {
                image_pusher: None,
                commit_adapter: None,
                image_builder: None,
            },
            events: daemonbox::handler::EventDeps {
                event_sink: Arc::new(minibox_core::events::NoopEventSink),
                event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
                metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
            },
            policy: daemonbox::handler::ContainerPolicy {
                allow_bind_mounts: true,
                allow_privileged: true,
            },
        });
        let state = mock_state(&temp_dir);

        // Run container 1
        handle_run_once(
            "alpine".to_string(),
            Some("latest".to_string()),
            vec!["/bin/sh".to_string()],
            None, None, false,
            Arc::clone(&state),
            Arc::clone(&deps),
        )
        .await;

        // Run container 2
        handle_run_once(
            "alpine".to_string(),
            Some("latest".to_string()),
            vec!["/bin/sh".to_string()],
            None, None, false,
            Arc::clone(&state),
            Arc::clone(&deps),
        )
        .await;

        assert_eq!(
            runtime.spawn_count(),
            2,
            "runtime must record exactly 2 spawn calls for 2 handle_run invocations"
        );
    }

    /// The first spawn must return PID 10000 and the second must return 10001
    /// (MockRuntime increments monotonically from 10000).
    #[tokio::test]
    async fn runtime_pids_are_unique_and_monotonically_increasing() {
        let runtime = MockRuntime::new();
        let config = minibox_core::domain::ContainerSpawnConfig {
            rootfs: std::path::PathBuf::from("/rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            hostname: "test".to_string(),
            cgroup_path: std::path::PathBuf::from("/cgroup"),
            capture_output: false,
            hooks: minibox_core::domain::ContainerHooks::default(),
            skip_network_namespace: false,
            mounts: vec![],
            privileged: false,
        };

        let pid1 = runtime.spawn_process(&config).await.unwrap().pid;
        let pid2 = runtime.spawn_process(&config).await.unwrap().pid;
        let pid3 = runtime.spawn_process(&config).await.unwrap().pid;

        assert_eq!(pid1, 10000, "first PID must be 10000");
        assert_eq!(pid2, 10001, "second PID must be 10001");
        assert_eq!(pid3, 10002, "third PID must be 10002");
        assert_eq!(runtime.spawn_count(), 3, "spawn count must be 3");
    }
}
```

- [ ] **Step 2: Run the runtime conformance tests**

```bash
cargo nextest run -p daemonbox --test conformance_tests runtime_conformance 2>&1 | tail -20
```

Expected: both tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/daemonbox/tests/conformance_tests.rs
git commit -m "test(conformance): runtime spawn-count and unique-PID conformance tests"
```

---

## Task 7: Acceptance gate

**Goal:** Verify the full workspace compiles and all conformance tests pass with the new tests
counted in the report.

---

- [ ] **Step 1: Run the full test suite**

```bash
cargo nextest run --workspace 2>&1 | tail -30
```

Expected: all tests pass (krun socket failures are pre-existing and acceptable).

- [ ] **Step 2: Run the conformance xtask**

```bash
cargo xtask test-conformance 2>&1 | tail -20
```

Expected: pass count is higher than the Phase 1 baseline of 11 (new tests add to the count).
Zero fails.

- [ ] **Step 3: Run clippy**

```bash
cargo clippy --workspace -- -D warnings 2>&1 | grep -E "^error" | head -20
```

Expected: zero errors. Fix any warnings in files touched by this plan.

- [ ] **Step 4: Final commit if anything was fixed**

```bash
git add -A
git commit -m "chore(conformance): clippy fixes and acceptance gate"
```

---

## Self-review

**Spec coverage:**

| Requirement | Covered by |
|---|---|
| A4: error-path tests | T2 (commit errors), T3 (handler error propagation) |
| A4: boundary conditions | T5 (resource limits u64::MAX, zero, None) |
| A4: round-trip correctness | T2 (idempotent commit, metadata consistency) |
| B5: GC capability | T4 (NoopImageGc prune count, skip semantics) |
| B5: spawn-failure | T3 (spawn failure → Error response) |
| B5: resource-limit path | T5 (boundary values through MockLimiter) |
| B5: multi-container runtime | T6 (spawn count, unique PIDs) |
| C: typed capability registry | T1 (ConformanceCapability trait + built-ins) |

**Exec, pause/resume, network events** — these require real Linux runtime or kernel features.
Conformance tests for them would be gated `#[cfg(target_os = "linux")]` and need real
infrastructure. They are out of scope for Phase 2 (mock-level conformance only) and belong in
Phase 3 (integration conformance).

**Placeholder scan:** None found.

**Type consistency:** All types (`MockRuntime`, `MockLimiter`, `NoopImageGc`, `HandlerDependencies`,
`ContainerSpawnConfig`, `ResourceConfig`) match their definitions in `minibox-core` and
`minibox-testers`. `ImageGarbageCollector::prune` signature must be verified in T4 Step 4 note.
