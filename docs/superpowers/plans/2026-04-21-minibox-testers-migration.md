---
status: done
completed: "2026-04-21"
branch: main
---

# minibox-testers: Conformance Infrastructure Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create the `minibox-testers` crate and migrate all test infrastructure (mocks, fixtures,
conformance types, report emitter, daemon helpers) out of `minibox-core` and `daemonbox` into it,
with the existing conformance suite as the acceptance gate.

**Architecture:** `minibox-testers` sits above `minibox-core` in the dependency graph and is a
`[dev-dependency]` only — never compiled into production binaries. `minibox-core` retains all
domain traits; `minibox-testers` owns all test doubles and fixtures. The `test-utils` feature in
`minibox-core` becomes a thin re-export shim so existing call sites compile unchanged during the
migration.

**Tech Stack:** Rust 2024 edition, `cargo nextest`, `cargo check --workspace`, `cargo xtask
test-conformance`, `cargo clippy --workspace`.

---

## Causal Chain

```
T1: Scaffold minibox-testers crate (workspace wiring)
  └─► T2: Migrate report types (ConformanceOutcome, ConformanceRow, ConformanceMatrixResult,
          write_conformance_reports)
        └─► T3: Migrate fixtures (MinimalStoredImageFixture, WritableUpperDirFixture,
                BuildContextFixture, LocalPushTargetFixture, TempContainerFixture,
                MockAdapterBuilder)
              └─► T4: Migrate mocks (MockRegistry, MockFilesystem, MockLimiter, MockRuntime,
                      MockNetwork, MockContainerCommitter, MockImageBuilder, MockImagePusher)
                    └─► T5: Migrate BackendDescriptor + conformance_helpers
                          └─► T6: Wire minibox-core test-utils re-export shim
                                └─► T7: Update all call sites (daemonbox + minibox tests)
                                      └─► T8: Acceptance gate + commit
```

---

## File Map

| Action | Path |
|--------|------|
| Create | `crates/minibox-testers/Cargo.toml` |
| Create | `crates/minibox-testers/src/lib.rs` |
| Create | `crates/minibox-testers/src/report.rs` |
| Create | `crates/minibox-testers/src/fixtures/mod.rs` |
| Create | `crates/minibox-testers/src/fixtures/image.rs` |
| Create | `crates/minibox-testers/src/fixtures/upper_dir.rs` |
| Create | `crates/minibox-testers/src/fixtures/build_context.rs` |
| Create | `crates/minibox-testers/src/fixtures/push_target.rs` |
| Create | `crates/minibox-testers/src/fixtures/container.rs` |
| Create | `crates/minibox-testers/src/mocks/mod.rs` |
| Create | `crates/minibox-testers/src/mocks/registry.rs` |
| Create | `crates/minibox-testers/src/mocks/filesystem.rs` |
| Create | `crates/minibox-testers/src/mocks/limiter.rs` |
| Create | `crates/minibox-testers/src/mocks/runtime.rs` |
| Create | `crates/minibox-testers/src/mocks/network.rs` |
| Create | `crates/minibox-testers/src/mocks/commit.rs` |
| Create | `crates/minibox-testers/src/mocks/build.rs` |
| Create | `crates/minibox-testers/src/mocks/push.rs` |
| Create | `crates/minibox-testers/src/backend/mod.rs` |
| Create | `crates/minibox-testers/src/backend/descriptor.rs` |
| Create | `crates/minibox-testers/src/helpers/mod.rs` |
| Create | `crates/minibox-testers/src/helpers/daemon.rs` |
| Create | `crates/minibox-testers/src/helpers/gc.rs` |
| Modify | `Cargo.toml` (workspace members) |
| Modify | `crates/minibox-core/Cargo.toml` (add minibox-testers dev-dep re-export) |
| Modify | `crates/minibox-core/src/adapters/conformance.rs` (thin re-export shim) |
| Modify | `crates/minibox-core/src/adapters/mocks.rs` (thin re-export shim) |
| Modify | `crates/minibox-core/src/adapters/test_fixtures.rs` (thin re-export shim) |
| Modify | `crates/minibox/Cargo.toml` (add minibox-testers dev-dep) |
| Modify | `crates/daemonbox/Cargo.toml` (add minibox-testers dev-dep) |
| Modify | `crates/minibox/tests/conformance_commit.rs` (update imports) |
| Modify | `crates/minibox/tests/conformance_build.rs` (update imports) |
| Modify | `crates/minibox/tests/conformance_push.rs` (update imports) |
| Modify | `crates/minibox/tests/conformance_report.rs` (update imports) |
| Modify | `crates/daemonbox/tests/conformance_tests.rs` (update imports) |
| Modify | `crates/daemonbox/tests/conformance_helpers.rs` (update imports) |
| Modify | `crates/daemonbox/tests/handler_tests.rs` (update imports) |

---

## Task 1: Scaffold `minibox-testers` crate

**Files:**
- Create: `crates/minibox-testers/Cargo.toml`
- Create: `crates/minibox-testers/src/lib.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Create the crate directory and Cargo.toml**

    ```bash
    mkdir -p crates/minibox-testers/src
    ```

    Write `crates/minibox-testers/Cargo.toml`:

    ```toml
    [package]
    name = "minibox-testers"
    version.workspace = true
    edition.workspace = true
    license.workspace = true
    publish = false

    [dependencies]
    minibox-core = { workspace = true }
    minibox = { workspace = true }
    daemonbox = { workspace = true }
    anyhow = { workspace = true }
    async-trait = { workspace = true }
    serde = { workspace = true }
    serde_json = { workspace = true }
    tokio = { workspace = true }
    tempfile = { workspace = true }
    ```

- [ ] **Step 2: Create the stub `lib.rs`**

    Write `crates/minibox-testers/src/lib.rs`:

    ```rust
    //! Test infrastructure for minibox — mocks, fixtures, conformance types.
    //!
    //! This crate is a `[dev-dependency]` only. It must never be compiled into
    //! production binaries. All modules are public so downstream test files can
    //! import them directly.

    pub mod backend;
    pub mod fixtures;
    pub mod helpers;
    pub mod mocks;
    pub mod report;
    ```

- [ ] **Step 3: Add `minibox-testers` to the workspace**

    In the root `Cargo.toml`, add `"crates/minibox-testers"` to the `members` array:

    ```toml
    members = [
        "crates/minibox-oci",
        "crates/minibox-core",
        "crates/minibox",
        "crates/miniboxd",
        "crates/minibox-cli",
        "crates/minibox-bench",
        "crates/minibox-macros",
        "crates/daemonbox",
        "crates/macbox",
        "crates/tailbox",
        "crates/winbox",
        "crates/minibox-llm",
        "crates/minibox-secrets",
        "crates/minibox-client",
        "crates/xtask",
        "crates/miniboxctl",
        "crates/dashbox",
        "crates/dockerbox",
        "crates/minibox-agent",
        "crates/minibox-testers",   # ← add this line
    ]
    ```

    Also add to `[workspace.dependencies]`:

    ```toml
    minibox-testers = { path = "crates/minibox-testers" }
    ```

- [ ] **Step 4: Verify the crate compiles empty**

    ```bash
    cargo check -p minibox-testers
    ```

    Expected: compiles with no errors (stub lib.rs with empty submodule stubs is fine after
    next step).

    If cargo complains about missing module files, create stubs:

    ```bash
    mkdir -p crates/minibox-testers/src/{backend,fixtures,helpers,mocks}
    touch crates/minibox-testers/src/backend/mod.rs
    touch crates/minibox-testers/src/fixtures/mod.rs
    touch crates/minibox-testers/src/helpers/mod.rs
    touch crates/minibox-testers/src/mocks/mod.rs
    touch crates/minibox-testers/src/report.rs
    ```

---

## Task 2: Migrate report types

Move `ConformanceOutcome`, `ConformanceRow`, `ConformanceMatrixResult`, and
`write_conformance_reports` from `minibox-core/src/adapters/conformance.rs` into
`minibox-testers/src/report.rs`. These types have no domain trait dependencies — they only use
`serde` and `std`.

**Files:**
- Create: `crates/minibox-testers/src/report.rs`

- [ ] **Step 1: Write `report.rs` with the migrated types**

    Write `crates/minibox-testers/src/report.rs` with the full content copied verbatim from the
    report-types section of `minibox-core/src/adapters/conformance.rs` (lines 351–466), adjusted
    to remove the `crate::` prefixes:

    ```rust
    //! Conformance report types and emitter.
    //!
    //! `ConformanceMatrixResult` is the top-level serializable report. After all
    //! conformance tests pass, `write_conformance_reports` emits `report.md` and
    //! `report.json` to the artifact directory.

    use std::path::{Path, PathBuf};

    /// Outcome of a single conformance test case.
    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ConformanceOutcome {
        /// Test ran and passed.
        Pass,
        /// Test was skipped (capability not declared by backend).
        Skip,
        /// Test ran and failed.
        Fail,
    }

    impl ConformanceOutcome {
        /// Display string used in Markdown tables.
        pub fn as_str(&self) -> &'static str {
            match self {
                ConformanceOutcome::Pass => "pass",
                ConformanceOutcome::Skip => "skip",
                ConformanceOutcome::Fail => "fail",
            }
        }
    }

    /// One row in the conformance matrix.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct ConformanceRow {
        pub backend: String,
        pub capability: String,
        pub test_name: String,
        pub outcome: ConformanceOutcome,
        pub message: Option<String>,
    }

    /// Aggregated result of the full conformance matrix run.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct ConformanceMatrixResult {
        pub timestamp: String,
        pub rows: Vec<ConformanceRow>,
    }

    impl ConformanceMatrixResult {
        /// Create a result with the current UTC timestamp (seconds since epoch).
        pub fn new(rows: Vec<ConformanceRow>) -> Self {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Self {
                timestamp: format!("{ts}"),
                rows,
            }
        }

        /// Count rows with a given outcome.
        pub fn count(&self, outcome: &ConformanceOutcome) -> usize {
            self.rows.iter().filter(|r| &r.outcome == outcome).count()
        }
    }

    /// Write `report.md` and `report.json` under `artifact_dir`.
    ///
    /// `artifact_dir` is created if it does not exist.
    pub fn write_conformance_reports(
        result: &ConformanceMatrixResult,
        artifact_dir: &Path,
    ) -> std::io::Result<(PathBuf, PathBuf)> {
        std::fs::create_dir_all(artifact_dir)?;

        let json_path = artifact_dir.join("report.json");
        let json = serde_json::to_string_pretty(result)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&json_path, json.as_bytes())?;

        let md_path = artifact_dir.join("report.md");
        let mut md = String::new();
        md.push_str("# Conformance Suite Report\n\n");
        md.push_str(&format!("**Timestamp:** {}\n\n", result.timestamp));
        md.push_str(&format!(
            "**Pass:** {}  **Skip:** {}  **Fail:** {}\n\n",
            result.count(&ConformanceOutcome::Pass),
            result.count(&ConformanceOutcome::Skip),
            result.count(&ConformanceOutcome::Fail),
        ));
        md.push_str("| Backend | Capability | Test | Outcome | Message |\n");
        md.push_str("|---------|------------|------|---------|--------|\n");
        for row in &result.rows {
            let msg = row.message.as_deref().unwrap_or("");
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                row.backend, row.capability, row.test_name, row.outcome.as_str(), msg
            ));
        }
        std::fs::write(&md_path, md.as_bytes())?;

        Ok((md_path, json_path))
    }
    ```

- [ ] **Step 2: Verify it compiles**

    ```bash
    cargo check -p minibox-testers
    ```

    Expected: no errors.

---

## Task 3: Migrate fixtures

Move all fixture types from `minibox-core/src/adapters/conformance.rs` and
`minibox-core/src/adapters/test_fixtures.rs` into `minibox-testers/src/fixtures/`.

**Files:**
- Create: `crates/minibox-testers/src/fixtures/mod.rs`
- Create: `crates/minibox-testers/src/fixtures/image.rs`
- Create: `crates/minibox-testers/src/fixtures/upper_dir.rs`
- Create: `crates/minibox-testers/src/fixtures/build_context.rs`
- Create: `crates/minibox-testers/src/fixtures/push_target.rs`
- Create: `crates/minibox-testers/src/fixtures/container.rs`

- [ ] **Step 1: Write `fixtures/mod.rs`**

    ```rust
    //! On-disk fixtures for conformance and integration tests.

    pub mod build_context;
    pub mod container;
    pub mod image;
    pub mod push_target;
    pub mod upper_dir;

    // Re-export all fixture types at the module level for convenience.
    pub use build_context::BuildContextFixture;
    pub use container::{MockAdapterBuilder, MockAdapterSet, TempContainerFixture};
    pub use image::MinimalStoredImageFixture;
    pub use push_target::LocalPushTargetFixture;
    pub use upper_dir::WritableUpperDirFixture;
    ```

- [ ] **Step 2: Write `fixtures/image.rs`** (copy `MinimalStoredImageFixture` verbatim from
    `minibox-core/src/adapters/conformance.rs` lines 142–206, removing the `crate::` prefix):

    ```rust
    //! Minimal stored OCI image fixture for conformance tests.

    use std::path::PathBuf;
    use tempfile::TempDir;

    /// A temporary directory tree that mimics the on-disk layout of a stored OCI
    /// image with one empty layer.
    pub struct MinimalStoredImageFixture {
        pub dir: TempDir,
        pub images_dir: PathBuf,
        pub layer_dir: PathBuf,
        pub layer_digest: String,
        pub image_name: String,
    }

    impl MinimalStoredImageFixture {
        pub fn new(image_name: Option<&str>) -> std::io::Result<Self> {
            let name = image_name.unwrap_or("conformance-base").to_string();
            let digest =
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string();
            let stripped = digest.strip_prefix("sha256:").unwrap_or(&digest);

            let dir = TempDir::new()?;
            let images_dir = dir.path().join("images");
            let layer_dir = images_dir.join(&name).join(stripped);
            std::fs::create_dir_all(&layer_dir)?;

            let manifests_dir = dir.path().join("manifests");
            std::fs::create_dir_all(&manifests_dir)?;
            std::fs::write(
                manifests_dir.join(format!("{name}.json")),
                b"{\"placeholder\":true}\n",
            )?;

            Ok(Self {
                dir,
                images_dir,
                layer_dir,
                layer_digest: digest,
                image_name: name,
            })
        }
    }
    ```

- [ ] **Step 3: Write `fixtures/upper_dir.rs`** (copy `WritableUpperDirFixture` from
    `minibox-core/src/adapters/conformance.rs` lines 208–257):

    ```rust
    //! Writable overlay upperdir fixture for commit conformance tests.

    use std::path::PathBuf;
    use tempfile::TempDir;

    /// A temporary directory pair simulating an overlay FS upper + work dir,
    /// seeded with a single sentinel file.
    pub struct WritableUpperDirFixture {
        pub dir: TempDir,
        pub upper_dir: PathBuf,
        pub work_dir: PathBuf,
        pub sentinel_filename: &'static str,
    }

    impl WritableUpperDirFixture {
        pub fn new() -> std::io::Result<Self> {
            let dir = TempDir::new()?;
            let upper_dir = dir.path().join("upper");
            let work_dir = dir.path().join("work");
            std::fs::create_dir_all(&upper_dir)?;
            std::fs::create_dir_all(&work_dir)?;

            let sentinel_filename = "conformance-sentinel";
            std::fs::write(upper_dir.join(sentinel_filename), b"1")?;

            Ok(Self {
                dir,
                upper_dir,
                work_dir,
                sentinel_filename,
            })
        }
    }
    ```

- [ ] **Step 4: Write `fixtures/build_context.rs`** (copy `BuildContextFixture` from
    `minibox-core/src/adapters/conformance.rs` lines 259–302):

    ```rust
    //! Minimal build context fixture for ImageBuilder conformance tests.

    use std::path::PathBuf;
    use tempfile::TempDir;

    /// A minimal build context directory with a one-instruction Dockerfile.
    pub struct BuildContextFixture {
        pub dir: TempDir,
        pub context_dir: PathBuf,
        pub dockerfile: PathBuf,
    }

    impl BuildContextFixture {
        pub fn new() -> std::io::Result<Self> {
            let dir = TempDir::new()?;
            let context_dir = dir.path().to_path_buf();
            let dockerfile = context_dir.join("Dockerfile");

            std::fs::write(&dockerfile, b"FROM scratch\nCOPY hello.txt /hello.txt\n")?;
            std::fs::write(context_dir.join("hello.txt"), b"conformance\n")?;

            Ok(Self {
                dir,
                context_dir,
                dockerfile,
            })
        }
    }
    ```

- [ ] **Step 5: Write `fixtures/push_target.rs`** (copy `LocalPushTargetFixture` from
    `minibox-core/src/adapters/conformance.rs` lines 304–346):

    ```rust
    //! Local OCI registry push target fixture.

    /// A locally-resolvable push target reference for push conformance tests.
    pub struct LocalPushTargetFixture {
        pub image_ref: String,
        pub registry_host: String,
        pub repository: String,
        pub tag: String,
    }

    impl LocalPushTargetFixture {
        pub fn new(repository: &str) -> Self {
            let registry_host = "localhost:5000".to_string();
            let tag = "latest".to_string();
            let image_ref = format!("{registry_host}/{repository}:{tag}");
            Self {
                image_ref,
                registry_host,
                repository: repository.to_string(),
                tag,
            }
        }
    }
    ```

- [ ] **Step 6: Write `fixtures/container.rs`** (copy `MockAdapterBuilder`, `MockAdapterSet`,
    and `TempContainerFixture` from `minibox-core/src/adapters/test_fixtures.rs`):

    ```rust
    //! Container-level fixtures: MockAdapterBuilder, MockAdapterSet, TempContainerFixture.

    use minibox_core::domain::{
        DynContainerRuntime, DynFilesystemProvider, DynImageRegistry, DynResourceLimiter,
    };
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    use crate::mocks::{MockFilesystem, MockLimiter, MockRegistry, MockRuntime};

    /// A complete set of mock domain adapters, ready for injection into tests.
    pub struct MockAdapterSet {
        pub filesystem: DynFilesystemProvider,
        pub limiter: DynResourceLimiter,
        pub registry: DynImageRegistry,
        pub runtime: DynContainerRuntime,
    }

    /// Builder for constructing [`MockAdapterSet`] with configurable failure modes.
    pub struct MockAdapterBuilder {
        fail_setup: bool,
        fail_create: bool,
        fail_pull: bool,
        fail_spawn: bool,
        cached_images: Vec<(String, String)>,
    }

    impl MockAdapterBuilder {
        pub fn new() -> Self {
            Self {
                fail_setup: false,
                fail_create: false,
                fail_pull: false,
                fail_spawn: false,
                cached_images: Vec::new(),
            }
        }

        pub fn with_setup_failure(mut self) -> Self {
            self.fail_setup = true;
            self
        }

        pub fn with_create_failure(mut self) -> Self {
            self.fail_create = true;
            self
        }

        pub fn with_pull_failure(mut self) -> Self {
            self.fail_pull = true;
            self
        }

        pub fn with_spawn_failure(mut self) -> Self {
            self.fail_spawn = true;
            self
        }

        pub fn with_cached_image(mut self, name: &str, tag: &str) -> Self {
            self.cached_images.push((name.to_string(), tag.to_string()));
            self
        }

        pub fn build(self) -> MockAdapterSet {
            let mut registry = MockRegistry::new();
            for (name, tag) in &self.cached_images {
                registry = registry.with_cached_image(name, tag);
            }
            if self.fail_pull {
                registry = registry.with_pull_failure();
            }

            let filesystem = if self.fail_setup {
                MockFilesystem::new().with_setup_failure()
            } else {
                MockFilesystem::new()
            };

            let limiter = if self.fail_create {
                MockLimiter::new().with_create_failure()
            } else {
                MockLimiter::new()
            };

            let runtime = if self.fail_spawn {
                MockRuntime::new().with_spawn_failure()
            } else {
                MockRuntime::new()
            };

            MockAdapterSet {
                filesystem: Arc::new(filesystem),
                limiter: Arc::new(limiter),
                registry: Arc::new(registry),
                runtime: Arc::new(runtime),
            }
        }
    }

    impl Default for MockAdapterBuilder {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Temporary directory fixture providing `images/` and `containers/` subdirs.
    pub struct TempContainerFixture {
        pub dir: TempDir,
        pub images_dir: PathBuf,
        pub containers_dir: PathBuf,
    }

    impl TempContainerFixture {
        pub fn new() -> std::io::Result<Self> {
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
    ```

- [ ] **Step 7: Verify fixtures compile**

    ```bash
    cargo check -p minibox-testers
    ```

    Expected: no errors (mocks not yet defined — the `use crate::mocks::*` imports in
    `container.rs` will cause errors until Task 4 is complete; if so, temporarily comment out
    `container.rs` content and re-enable after Task 4).

---

## Task 4: Migrate mocks

Move all mock types from `minibox-core/src/adapters/mocks.rs` into
`minibox-testers/src/mocks/`. The source file is large (~600 lines); split by type.

**Files:**
- Create: `crates/minibox-testers/src/mocks/mod.rs`
- Create: `crates/minibox-testers/src/mocks/registry.rs`
- Create: `crates/minibox-testers/src/mocks/filesystem.rs`
- Create: `crates/minibox-testers/src/mocks/limiter.rs`
- Create: `crates/minibox-testers/src/mocks/runtime.rs`
- Create: `crates/minibox-testers/src/mocks/network.rs`
- Create: `crates/minibox-testers/src/mocks/commit.rs`
- Create: `crates/minibox-testers/src/mocks/build.rs`
- Create: `crates/minibox-testers/src/mocks/push.rs`

- [ ] **Step 1: Read the full mocks source**

    Read `crates/minibox-core/src/adapters/mocks.rs` in full to understand all types before
    splitting. Key types to migrate (in order of declaration):

    - `MockRegistry` + `MockRegistryState`
    - `MockFilesystem` + `MockFilesystemState`
    - `MockLimiter` + `MockLimiterState`
    - `MockRuntime` + `MockRuntimeState`
    - `MockNetwork` + `MockNetworkState`
    - `MockContainerCommitter`
    - `MockImageBuilder`
    - `MockImagePusher`

- [ ] **Step 2: Write `mocks/mod.rs`**

    ```rust
    //! Mock adapters for minibox domain traits.
    //!
    //! All mocks track call counts and can be configured to fail on demand via
    //! builder methods. State is shared behind `Arc<Mutex<…>>` so mocks can be
    //! cloned and observed from the test after injection.

    pub mod build;
    pub mod commit;
    pub mod filesystem;
    pub mod limiter;
    pub mod network;
    pub mod push;
    pub mod registry;
    pub mod runtime;

    pub use build::MockImageBuilder;
    pub use commit::MockContainerCommitter;
    pub use filesystem::MockFilesystem;
    pub use limiter::MockLimiter;
    pub use network::MockNetwork;
    pub use push::MockImagePusher;
    pub use registry::MockRegistry;
    pub use runtime::MockRuntime;
    ```

- [ ] **Step 3: Write each mock file**

    For each mock type, create the corresponding file in `crates/minibox-testers/src/mocks/`
    by copying its implementation from `crates/minibox-core/src/adapters/mocks.rs`. Replace
    `use crate::` imports with `use minibox_core::` and `use minibox::` as appropriate.

    The import header for most mock files will be:

    ```rust
    use minibox_core::domain::{/* trait being implemented */};
    use anyhow::Result;
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    ```

    `MockRegistry` additionally needs:
    ```rust
    use minibox_core::domain::{ImageRegistry, ImageMetadata, LayerInfo};
    use minibox_core::image::reference::ImageRef;
    use std::path::PathBuf;
    ```

    `MockFilesystem` additionally needs:
    ```rust
    use minibox_core::domain::{FilesystemProvider, RootfsLayout, RootfsSetup, ChildInit};
    use std::path::{Path, PathBuf};
    ```

    `MockRuntime` additionally needs:
    ```rust
    use minibox_core::domain::{ContainerRuntime, ContainerSpawnConfig, SpawnResult,
        RuntimeCapabilities, ContainerHooks};
    use std::path::PathBuf;
    ```

    Copy the struct, state struct, `impl MockX { ... }`, and all trait impls verbatim —
    only change import paths.

- [ ] **Step 4: Verify mocks compile**

    ```bash
    cargo check -p minibox-testers
    ```

    Expected: no errors. Fix any import path issues — the pattern is always `minibox_core::domain::Trait` instead of `crate::domain::Trait`.

---

## Task 5: Migrate `BackendDescriptor` and daemon helpers

**Files:**
- Create: `crates/minibox-testers/src/backend/mod.rs`
- Create: `crates/minibox-testers/src/backend/descriptor.rs`
- Create: `crates/minibox-testers/src/helpers/mod.rs`
- Create: `crates/minibox-testers/src/helpers/daemon.rs`
- Create: `crates/minibox-testers/src/helpers/gc.rs`

- [ ] **Step 1: Write `backend/descriptor.rs`**

    Copy `BackendDescriptor` from `minibox-core/src/adapters/conformance.rs` (lines 54–137).
    Replace `use crate::domain::` with `use minibox_core::domain::`:

    ```rust
    //! BackendDescriptor — describes a backend under conformance test.

    use minibox_core::domain::{
        BackendCapability, BackendCapabilitySet, DynContainerCommitter, DynImageBuilder,
        DynImagePusher,
    };

    /// Describes a concrete backend under conformance test.
    pub struct BackendDescriptor {
        pub name: &'static str,
        pub capabilities: BackendCapabilitySet,
        pub make_committer: Option<Box<dyn Fn() -> DynContainerCommitter + Send + Sync>>,
        pub make_builder: Option<Box<dyn Fn() -> DynImageBuilder + Send + Sync>>,
        pub make_pusher: Option<Box<dyn Fn() -> DynImagePusher + Send + Sync>>,
    }

    impl BackendDescriptor {
        pub fn new(name: &'static str) -> Self {
            Self {
                name,
                capabilities: BackendCapabilitySet::new(),
                make_committer: None,
                make_builder: None,
                make_pusher: None,
            }
        }

        pub fn with_capability(mut self, cap: BackendCapability) -> Self {
            self.capabilities = self.capabilities.with(cap);
            self
        }

        pub fn with_committer<F>(mut self, f: F) -> Self
        where
            F: Fn() -> DynContainerCommitter + Send + Sync + 'static,
        {
            self.capabilities = self.capabilities.with(BackendCapability::Commit);
            self.make_committer = Some(Box::new(f));
            self
        }

        pub fn with_builder<F>(mut self, f: F) -> Self
        where
            F: Fn() -> DynImageBuilder + Send + Sync + 'static,
        {
            self.capabilities = self.capabilities.with(BackendCapability::BuildFromContext);
            self.make_builder = Some(Box::new(f));
            self
        }

        pub fn with_pusher<F>(mut self, f: F) -> Self
        where
            F: Fn() -> DynImagePusher + Send + Sync + 'static,
        {
            self.capabilities = self.capabilities.with(BackendCapability::PushToRegistry);
            self.make_pusher = Some(Box::new(f));
            self
        }
    }
    ```

- [ ] **Step 2: Write `backend/mod.rs`**

    ```rust
    pub mod descriptor;
    pub use descriptor::BackendDescriptor;
    ```

- [ ] **Step 3: Write `helpers/gc.rs`** (the `NoopImageGc` that appears duplicated in both
    `conformance_tests.rs` and `conformance_helpers.rs`):

    ```rust
    //! No-op image garbage collector for tests.

    use minibox_core::image::gc::{ImageGarbageCollector, PruneReport};

    pub struct NoopImageGc;

    #[async_trait::async_trait]
    impl ImageGarbageCollector for NoopImageGc {
        async fn prune(
            &self,
            dry_run: bool,
            _in_use: &[String],
        ) -> anyhow::Result<PruneReport> {
            Ok(PruneReport {
                removed: vec![],
                freed_bytes: 0,
                dry_run,
            })
        }
    }
    ```

- [ ] **Step 4: Write `helpers/daemon.rs`**

    Copy `make_mock_deps`, `make_mock_deps_with_registry`, and `make_mock_state` from
    `crates/daemonbox/tests/conformance_helpers.rs`, updating imports:

    ```rust
    //! Daemon-level test helpers: HandlerDependencies + DaemonState factories.

    use daemonbox::handler::{
        BuildDeps, EventDeps, ExecDeps, HandlerDependencies, ImageDeps, LifecycleDeps,
        NoopImageLoader, PtySessionRegistry,
    };
    use daemonbox::state::DaemonState;
    use minibox::adapters::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRuntime};
    use minibox_core::adapters::HostnameRegistryRouter;
    use minibox_core::domain::DynImageRegistry;
    use minibox_core::events::{BroadcastEventBroker, NoopEventSink};
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::TempDir;

    use crate::helpers::gc::NoopImageGc;
    use crate::mocks::{MockNetwork as TestersMockNetwork, MockRegistry};

    /// Build a [`HandlerDependencies`] wired with mock adapters, rooted under `temp_dir`.
    pub fn make_mock_deps(temp_dir: &TempDir) -> Arc<HandlerDependencies> {
        make_mock_deps_with_registry(MockRegistry::new(), temp_dir)
    }

    /// Build mock deps with a specific `registry`.
    pub fn make_mock_deps_with_registry(
        registry: MockRegistry,
        temp_dir: &TempDir,
    ) -> Arc<HandlerDependencies> {
        let image_store = Arc::new(
            minibox_core::image::ImageStore::new(temp_dir.path().join("img")).unwrap(),
        );
        Arc::new(HandlerDependencies {
            image: ImageDeps {
                registry_router: Arc::new(HostnameRegistryRouter::new(
                    Arc::new(registry) as DynImageRegistry,
                    [("ghcr.io", Arc::new(MockRegistry::new()) as DynImageRegistry)],
                )),
                image_loader: Arc::new(NoopImageLoader),
                image_gc: Arc::new(NoopImageGc),
                image_store,
            },
            lifecycle: LifecycleDeps {
                filesystem: Arc::new(MockFilesystem::new()),
                resource_limiter: Arc::new(MockLimiter::new()),
                runtime: Arc::new(MockRuntime::new()),
                network_provider: Arc::new(MockNetwork::new()),
                containers_base: temp_dir.path().join("containers"),
                run_containers_base: temp_dir.path().join("run"),
            },
            exec: ExecDeps {
                exec_runtime: None,
                pty_sessions: Arc::new(tokio::sync::Mutex::new(
                    PtySessionRegistry::default(),
                )),
            },
            build: BuildDeps {
                image_pusher: None,
                commit_adapter: None,
                image_builder: None,
            },
            events: EventDeps {
                event_sink: Arc::new(NoopEventSink),
                event_source: Arc::new(BroadcastEventBroker::new()),
                metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
            },
            policy: daemonbox::handler::ContainerPolicy {
                allow_bind_mounts: true,
                allow_privileged: true,
            },
        })
    }

    /// Build a mock [`DaemonState`] rooted under `base`.
    pub fn make_mock_state(base: &Path) -> Arc<DaemonState> {
        let image_store =
            minibox::image::ImageStore::new(base.join("images")).unwrap();
        Arc::new(DaemonState::new(image_store, base))
    }
    ```

    **Note on MockNetwork:** `daemonbox` tests import `MockNetwork` from `minibox::adapters::mocks`.
    After migration, import it from `minibox_testers::mocks::MockNetwork` in test files. The
    daemon helper above bridges both until call sites are updated in Task 7.

- [ ] **Step 5: Write `helpers/mod.rs`**

    ```rust
    pub mod daemon;
    pub mod gc;

    pub use daemon::{make_mock_deps, make_mock_deps_with_registry, make_mock_state};
    pub use gc::NoopImageGc;
    ```

- [ ] **Step 6: Verify the full crate compiles**

    ```bash
    cargo check -p minibox-testers
    ```

    Expected: no errors.

---

## Task 6: Wire `minibox-core` re-export shim

Replace the implementation in `minibox-core/src/adapters/conformance.rs` with a thin re-export
so existing `use minibox_core::adapters::conformance::*` call sites continue to compile.

**Files:**
- Modify: `crates/minibox-core/Cargo.toml`
- Modify: `crates/minibox-core/src/adapters/conformance.rs`
- Modify: `crates/minibox-core/src/adapters/mocks.rs`
- Modify: `crates/minibox-core/src/adapters/test_fixtures.rs`

- [ ] **Step 1: Add `minibox-testers` as a dev-dependency of `minibox-core`**

    In `crates/minibox-core/Cargo.toml`, add:

    ```toml
    [dev-dependencies]
    minibox-testers = { workspace = true }
    # ... existing dev-deps unchanged
    ```

    And add `minibox-testers` to the `test-utils` feature so it is available when the feature
    is enabled by downstream crates:

    ```toml
    [features]
    test-utils = ["dep:minibox-testers"]

    [dependencies]
    # ... existing deps unchanged ...
    minibox-testers = { workspace = true, optional = true }
    ```

- [ ] **Step 2: Replace `conformance.rs` with a re-export shim**

    Overwrite `crates/minibox-core/src/adapters/conformance.rs`:

    ```rust
    //! Conformance test infrastructure — re-exported from `minibox-testers`.
    //!
    //! This module is a thin shim. All types now live in the `minibox-testers` crate.
    //! Existing `use minibox_core::adapters::conformance::*` imports continue to work.

    #[cfg(feature = "test-utils")]
    pub use minibox_testers::backend::BackendDescriptor;
    #[cfg(feature = "test-utils")]
    pub use minibox_testers::fixtures::{
        BuildContextFixture, LocalPushTargetFixture, MinimalStoredImageFixture,
        WritableUpperDirFixture,
    };
    #[cfg(feature = "test-utils")]
    pub use minibox_testers::report::{
        ConformanceMatrixResult, ConformanceOutcome, ConformanceRow, write_conformance_reports,
    };
    ```

- [ ] **Step 3: Replace `mocks.rs` with a re-export shim**

    Overwrite `crates/minibox-core/src/adapters/mocks.rs`:

    ```rust
    //! Mock adapters — re-exported from `minibox-testers`.

    #[cfg(feature = "test-utils")]
    pub use minibox_testers::mocks::{
        MockContainerCommitter, MockFilesystem, MockImageBuilder, MockImagePusher,
        MockLimiter, MockNetwork, MockRegistry, MockRuntime,
    };
    ```

- [ ] **Step 4: Replace `test_fixtures.rs` with a re-export shim**

    Overwrite `crates/minibox-core/src/adapters/test_fixtures.rs`:

    ```rust
    //! Test fixtures — re-exported from `minibox-testers`.

    #[cfg(feature = "test-utils")]
    pub use minibox_testers::fixtures::{
        MockAdapterBuilder, MockAdapterSet, TempContainerFixture,
    };
    ```

- [ ] **Step 5: Verify `minibox-core` still compiles**

    ```bash
    cargo check -p minibox-core
    cargo check -p minibox-core --features test-utils
    ```

    Expected: both pass with no errors.

---

## Task 7: Update all call sites

Add `minibox-testers` as a dev-dependency to `minibox` and `daemonbox`, then update all test
files to import from `minibox_testers` directly (rather than through the `minibox-core` shim).

**Files:**
- Modify: `crates/minibox/Cargo.toml`
- Modify: `crates/daemonbox/Cargo.toml`
- Modify: `crates/minibox/tests/conformance_commit.rs`
- Modify: `crates/minibox/tests/conformance_build.rs`
- Modify: `crates/minibox/tests/conformance_push.rs`
- Modify: `crates/minibox/tests/conformance_report.rs`
- Modify: `crates/daemonbox/tests/conformance_tests.rs`
- Modify: `crates/daemonbox/tests/conformance_helpers.rs`

- [ ] **Step 1: Add `minibox-testers` dev-dep to `minibox` and `daemonbox`**

    In `crates/minibox/Cargo.toml` `[dev-dependencies]`:
    ```toml
    minibox-testers = { workspace = true }
    ```

    In `crates/daemonbox/Cargo.toml` `[dev-dependencies]`:
    ```toml
    minibox-testers = { workspace = true }
    ```

- [ ] **Step 2: Update `conformance_commit.rs`**

    Replace:
    ```rust
    use minibox_core::adapters::conformance::{BackendDescriptor, WritableUpperDirFixture};
    ```
    With:
    ```rust
    use minibox_testers::backend::BackendDescriptor;
    use minibox_testers::fixtures::WritableUpperDirFixture;
    ```

- [ ] **Step 3: Update `conformance_build.rs`**

    Replace:
    ```rust
    use minibox_core::adapters::conformance::{BackendDescriptor, BuildContextFixture};
    ```
    With:
    ```rust
    use minibox_testers::backend::BackendDescriptor;
    use minibox_testers::fixtures::BuildContextFixture;
    ```

- [ ] **Step 4: Update `conformance_push.rs`**

    Replace:
    ```rust
    use minibox_core::adapters::conformance::{BackendDescriptor, LocalPushTargetFixture};
    use minibox_core::adapters::mocks::MockImagePusher;
    ```
    With:
    ```rust
    use minibox_testers::backend::BackendDescriptor;
    use minibox_testers::fixtures::LocalPushTargetFixture;
    use minibox_testers::mocks::MockImagePusher;
    ```

- [ ] **Step 5: Update `conformance_report.rs`**

    Replace:
    ```rust
    use minibox_core::adapters::conformance::{
        ConformanceMatrixResult, ConformanceOutcome, ConformanceRow, write_conformance_reports,
    };
    ```
    With:
    ```rust
    use minibox_testers::report::{
        ConformanceMatrixResult, ConformanceOutcome, ConformanceRow, write_conformance_reports,
    };
    ```

- [ ] **Step 6: Update `daemonbox/tests/conformance_tests.rs`**

    Replace:
    ```rust
    use minibox::adapters::mocks::{
        MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
    };
    use minibox_core::adapters::conformance::BackendDescriptor;
    use minibox_core::adapters::mocks::{MockContainerCommitter, MockImageBuilder, MockImagePusher};
    ```
    With:
    ```rust
    use minibox_testers::backend::BackendDescriptor;
    use minibox_testers::mocks::{
        MockContainerCommitter, MockFilesystem, MockImageBuilder, MockImagePusher,
        MockLimiter, MockNetwork, MockRegistry, MockRuntime,
    };
    ```

    Also replace the inline `NoopImageGc` struct and its impl with:
    ```rust
    use minibox_testers::helpers::NoopImageGc;
    ```

- [ ] **Step 7: Update `daemonbox/tests/conformance_helpers.rs`**

    Replace the inline `NoopImageGc` with:
    ```rust
    use minibox_testers::helpers::NoopImageGc;
    ```

    Replace mock imports:
    ```rust
    use minibox::adapters::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime};
    ```
    With:
    ```rust
    use minibox_testers::mocks::{MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime};
    ```

- [ ] **Step 8: Verify workspace compiles**

    ```bash
    cargo check --workspace
    ```

    Expected: no errors. Fix any remaining import path mismatches.

---

## Task 8: Acceptance gate + commit

Run the full acceptance criteria from the spec, then commit.

- [ ] **Step 1: Run the conformance suite**

    ```bash
    cargo xtask test-conformance
    ```

    Expected: exits 0. Check the emitted `artifacts/conformance/report.md` — should show
    `Pass: 11  Skip: 3  Fail: 0` (matching pre-migration baseline).

- [ ] **Step 2: Run the full workspace test suite**

    ```bash
    cargo nextest run --workspace
    ```

    Expected: all tests pass.

- [ ] **Step 3: Run clippy**

    ```bash
    cargo clippy --workspace -- -D warnings
    ```

    Expected: zero warnings. Fix any that appear (typically unused import or dead_code from
    the shim re-exports — suppress with `#[allow(unused_imports)]` on the shim `use` lines
    if needed).

- [ ] **Step 4: Verify no production crate depends on `minibox-testers`**

    ```bash
    cargo tree -p miniboxd | grep minibox-testers
    cargo tree -p minibox-cli | grep minibox-testers
    ```

    Expected: no output. If `minibox-testers` appears, a `[dependencies]` entry was added
    incorrectly — it must only be in `[dev-dependencies]`.

- [ ] **Step 5: Commit**

    ```bash
    git add \
      crates/minibox-testers/ \
      crates/minibox-core/Cargo.toml \
      crates/minibox-core/src/adapters/conformance.rs \
      crates/minibox-core/src/adapters/mocks.rs \
      crates/minibox-core/src/adapters/test_fixtures.rs \
      crates/minibox/Cargo.toml \
      crates/minibox/tests/conformance_commit.rs \
      crates/minibox/tests/conformance_build.rs \
      crates/minibox/tests/conformance_push.rs \
      crates/minibox/tests/conformance_report.rs \
      crates/daemonbox/Cargo.toml \
      crates/daemonbox/tests/conformance_tests.rs \
      crates/daemonbox/tests/conformance_helpers.rs \
      Cargo.toml
    git commit -m "refactor(testers): extract minibox-testers crate from minibox-core

    Moves all test infrastructure (mocks, fixtures, conformance report types,
    BackendDescriptor, daemon helpers) into a dedicated minibox-testers crate.
    minibox-core test-utils feature becomes a thin re-export shim.
    Conformance suite passes unchanged as the acceptance gate (11 pass, 3 skip)."
    ```

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Task |
|-----------------|------|
| `minibox-testers` crate exists, compiles | T1 |
| All types at new paths in `minibox-testers` | T2–T5 |
| `minibox-core` `test-utils` re-exports compile | T6 |
| `cargo xtask test-conformance` passes with same counts | T8 |
| `cargo nextest run --workspace` passes | T8 |
| `cargo clippy --workspace` zero new warnings | T8 |
| No production crate depends on `minibox-testers` | T8 |

**Placeholder scan:** All steps contain concrete commands, exact file paths, and complete code.
No TBD, TODO, or "similar to" references.

**Type consistency:**
- `MockRegistry`, `MockFilesystem`, `MockLimiter`, `MockRuntime`, `MockNetwork` defined in T4,
  used in T5 (`helpers/daemon.rs`) and T7 (call site updates) — consistent.
- `BackendDescriptor` defined in T5, used in T7 call site updates — consistent.
- `ConformanceOutcome`, `ConformanceRow`, `ConformanceMatrixResult`, `write_conformance_reports`
  defined in T2, re-exported in T6, imported in T7 — consistent.
- `NoopImageGc` defined in T5 (`helpers/gc.rs`), imported in T7 — consistent.
