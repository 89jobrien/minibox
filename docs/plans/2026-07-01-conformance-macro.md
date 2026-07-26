# Plan: Declarative Conformance Test Macro with Auto-Collection

## Goal

Replace hand-written `ConformanceTest` boilerplate with a
`conformance_test!` macro + `inventory` auto-collection, extend
`BackendDescriptor` to all 12 capabilities, and wire capability-based
auto-skip through `TestContext`.

## Architecture

- **Crates affected**: `minibox-core` (descriptor expansion),
  `minibox-testsuite` (macro, context, runner, adapter modules, spoke
  removal, binaries)
- **New types**: `ConformanceTestEntry` (inventory submission wrapper),
  `CapabilityExtras` (type alias)
- **New macro**: `conformance_test!` in `minibox-testsuite`
- **Data flow**: `conformance_test!` -> `inventory::submit!` ->
  `TestRunner::collect_inventory()` -> `run_sync` with
  `TestContext<'d>` -> `TestSummary` -> `ReportGenerator`

## Tech Stack

- Rust 2024
- New dependency: `inventory` (zero transitive deps)

## Tasks

### Task 1: Add `inventory` to workspace

**Crate**: workspace root
**File(s)**: `Cargo.toml` (workspace)

1. Add `inventory` to `[workspace.dependencies]`:

   ```toml
   inventory = "0.3"
   ```

2. Add `inventory` to `minibox-core/Cargo.toml` under `[dependencies]`
   gated on `test-utils`:

   ```toml
   inventory = { workspace = true, optional = true }
   ```

   Update the `test-utils` feature:

   ```toml
   test-utils = ["dep:tempfile", "dep:inventory"]
   ```

3. Add `inventory` to `minibox-testsuite/Cargo.toml` under
   `[dependencies]`:

   ```toml
   inventory = { workspace = true }
   ```

4. Verify:

   ```
   cargo check -p minibox-core --features test-utils
   cargo check -p minibox-testsuite
   ```

5. Commit: `git commit -m "build: add inventory dep for conformance auto-collection"`

---

### Task 2: Expand `BackendDescriptor` with `extras` map

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/adapters/conformance.rs`
**Run**: `cargo nextest run -p minibox-core --features test-utils`

1. Write failing test at the bottom of the existing `mod tests`:

   ```rust
   #[test]
   fn descriptor_with_extra_stores_and_retrieves() {
       use std::sync::Arc;
       let d = BackendDescriptor::new("test")
           .with_extra(
               BackendCapability::Exec,
               Box::new(|| Arc::new(()) as Arc<dyn std::any::Any + Send + Sync>),
           );
       assert!(d.capabilities.supports(BackendCapability::Exec));
       assert!(d.extras.contains_key(&BackendCapability::Exec));
   }

   #[test]
   fn descriptor_extras_default_empty() {
       let d = BackendDescriptor::new("test");
       assert!(d.extras.is_empty());
   }
   ```

   Run: `cargo nextest run -p minibox-core --features test-utils -- descriptor_with_extra`
   Expected: FAIL (no `extras` field, no `with_extra` method)

2. Implement. Add imports at the top of `conformance.rs`:

   ```rust
   use std::any::Any;
   use std::collections::HashMap;
   ```

   Add type alias before `BackendDescriptor`:

   ```rust
   /// Type-erased capability factory map for capabilities beyond
   /// the 3 named fields.
   pub type CapabilityExtras =
       HashMap<BackendCapability, Box<dyn Any + Send + Sync>>;
   ```

   Add field to `BackendDescriptor`:

   ```rust
   /// Type-erased factories for capabilities not covered by the
   /// named `make_*` fields. See [`with_extra`](Self::with_extra).
   pub extras: CapabilityExtras,
   ```

   Update `BackendDescriptor::new` to initialize `extras`:

   ```rust
   extras: HashMap::new(),
   ```

   Add methods to `impl BackendDescriptor`:

   ```rust
   /// Register a type-erased factory for any capability.
   /// Also adds the capability flag.
   #[must_use]
   pub fn with_extra<T: Send + Sync + 'static>(
       mut self,
       cap: BackendCapability,
       factory: Box<dyn Fn() -> T + Send + Sync>,
   ) -> Self {
       self.capabilities = self.capabilities.with(cap);
       self.extras.insert(cap, Box::new(factory));
       self
   }
   ```

3. Verify:

   ```
   cargo nextest run -p minibox-core --features test-utils  -> all green
   cargo clippy -p minibox-core --features test-utils -- -D warnings  -> zero
   ```

4. Commit: `git commit -m "feat(core): extend BackendDescriptor with extras capability map"`

---

### Task 3: Add lifetime to `TestContext` and descriptor field

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/harness/context.rs`
**Run**: `cargo nextest run -p minibox-testsuite`

1. Write failing test:

   ```rust
   #[test]
   fn context_with_descriptor_supports_capability() {
       use minibox_core::adapters::conformance::BackendDescriptor;
       use minibox_core::domain::BackendCapability;

       let desc = BackendDescriptor::new("test")
           .with_capability(BackendCapability::Commit);
       let ctx = TestContext::with_descriptor(&desc);
       assert!(ctx.supports(BackendCapability::Commit));
       assert!(!ctx.supports(BackendCapability::Exec));
   }

   #[test]
   fn context_without_descriptor_does_not_support() {
       use minibox_core::domain::BackendCapability;

       let ctx = TestContext::new();
       assert!(!ctx.supports(BackendCapability::Commit));
   }
   ```

   Expected: FAIL (no `with_descriptor`, no `supports`)

2. Implement. Update `TestContext` to add a lifetime and descriptor:

   ```rust
   use minibox_core::adapters::conformance::BackendDescriptor;
   use minibox_core::domain::BackendCapability;

   pub struct TestContext<'d> {
       failures: Vec<String>,
       log: Vec<LogEntry>,
       descriptor: Option<&'d BackendDescriptor>,
   }

   impl Default for TestContext<'_> {
       fn default() -> Self {
           Self::new()
       }
   }

   impl<'d> TestContext<'d> {
       #[must_use]
       pub const fn new() -> Self {
           Self {
               failures: Vec::new(),
               log: Vec::new(),
               descriptor: None,
           }
       }

       #[must_use]
       pub const fn with_descriptor(
           descriptor: &'d BackendDescriptor,
       ) -> Self {
           Self {
               failures: Vec::new(),
               log: Vec::new(),
               descriptor: Some(descriptor),
           }
       }

       /// Check if the backend supports a capability.
       /// Returns `false` if no descriptor is set.
       #[must_use]
       pub fn supports(&self, cap: BackendCapability) -> bool {
           self.descriptor
               .map_or(false, |d| d.capabilities.supports(cap))
       }

       /// Access the backend descriptor, if set.
       #[must_use]
       pub fn descriptor(&self) -> Option<&BackendDescriptor> {
           self.descriptor
       }

       // ... all existing assert_* and log_* methods unchanged,
       // but now on impl<'d> TestContext<'d> ...
   }
   ```

   All existing methods move into the `impl<'d>` block unchanged.

3. Fix all call sites that reference `TestContext` without a lifetime:
   - `harness/traits.rs`: `fn run_sync(&self, ctx: &mut TestContext<'_>) -> TestResult;`
   - `harness/runner.rs`: `let mut ctx = TestContext::new();` (unchanged, infers `'_`)
   - `harness/mod.rs` re-export: unchanged (re-exports the type)
   - `spoke.rs` tests: `TestContext::new()` (unchanged)
   - All 22 adapter modules: `fn run_sync(&self, ctx: &mut TestContext<'_>) -> TestResult`

4. Verify:

   ```
   cargo nextest run -p minibox-testsuite  -> all green
   cargo clippy -p minibox-testsuite -- -D warnings  -> zero
   ```

5. Commit: `git commit -m "feat(testsuite): add lifetime and descriptor to TestContext"`

---

### Task 4: Add `required_capability` to `ConformanceTest` trait

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/harness/traits.rs`
**Run**: `cargo nextest run -p minibox-testsuite`

1. Add default method to `ConformanceTest`:

   ```rust
   /// Capability required by this test. If `Some`, the runner
   /// auto-skips when the backend descriptor does not declare it.
   /// Default: `None` (always runs).
   fn required_capability(
       &self,
   ) -> Option<minibox_core::domain::BackendCapability> {
       None
   }
   ```

2. This is additive (default method), so all existing impls compile
   without changes.

3. Verify:

   ```
   cargo nextest run -p minibox-testsuite  -> all green
   cargo clippy -p minibox-testsuite -- -D warnings  -> zero
   ```

4. Commit: `git commit -m "feat(testsuite): add required_capability default to ConformanceTest"`

---

### Task 5: Define `ConformanceTestEntry` and `conformance_test!` macro

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/harness/macros.rs` (new file),
`crates/minibox-testsuite/src/harness/mod.rs`,
`crates/minibox-testsuite/src/lib.rs`
**Run**: `cargo nextest run -p minibox-testsuite`

1. Create `crates/minibox-testsuite/src/harness/macros.rs`:

   ```rust
   //! `ConformanceTestEntry` and the `conformance_test!` macro.

   use super::traits::ConformanceTest;

   /// Wrapper submitted to `inventory` by each `conformance_test!`
   /// invocation.
   pub struct ConformanceTestEntry {
       pub make: fn() -> Box<dyn ConformanceTest>,
   }

   inventory::collect!(ConformanceTestEntry);
   ```

2. Add the macro to `crates/minibox-testsuite/src/lib.rs` (must be at
   crate root for `#[macro_export]`):

   ```rust
   /// Declare a conformance test with inventory auto-registration.
   ///
   /// # With capability (auto-skip when unsupported)
   ///
   /// ```rust,ignore
   /// conformance_test! {
   ///     name: "commit_roundtrip",
   ///     adapter: "container_committer",
   ///     capability: Commit,
   ///     category: Unit,
   ///     |ctx| {
   ///         ctx.result()
   ///     }
   /// }
   /// ```
   ///
   /// # Without capability (always runs)
   ///
   /// ```rust,ignore
   /// conformance_test! {
   ///     name: "pull_increments_count",
   ///     adapter: "registry",
   ///     category: Unit,
   ///     |ctx| {
   ///         ctx.result()
   ///     }
   /// }
   /// ```
   #[macro_export]
   macro_rules! conformance_test {
       // Variant WITH capability
       (
           name: $name:expr,
           adapter: $adapter:expr,
           capability: $cap:ident,
           category: $cat:ident,
           |$ctx:ident| $body:block
       ) => {
           $crate::conformance_test!(@inner
               $name, $adapter,
               Some(minibox_core::domain::BackendCapability::$cap),
               $cat, $ctx, $body
           );
       };
       // Variant WITHOUT capability
       (
           name: $name:expr,
           adapter: $adapter:expr,
           category: $cat:ident,
           |$ctx:ident| $body:block
       ) => {
           $crate::conformance_test!(@inner
               $name, $adapter,
               None,
               $cat, $ctx, $body
           );
       };
       // Internal expansion
       (@inner
           $name:expr, $adapter:expr,
           $cap:expr,
           $cat:ident, $ctx:ident, $body:block
       ) => {
           ::paste::paste! {
               struct [<
                   __conformance_
                   $adapter:snake _
                   $name:snake
               >];

               impl $crate::harness::ConformanceTest
                   for [<
                       __conformance_
                       $adapter:snake _
                       $name:snake
                   >]
               {
                   fn name(&self) -> &str { $name }
                   fn adapter(&self) -> &str { $adapter }
                   fn category(&self)
                       -> $crate::harness::TestCategory
                   {
                       $crate::harness::TestCategory::$cat
                   }
                   fn required_capability(&self)
                       -> Option<minibox_core::domain::BackendCapability>
                   {
                       $cap
                   }
                   fn run_sync(
                       &self,
                       $ctx: &mut $crate::harness::TestContext<'_>,
                   ) -> $crate::harness::TestResult
                   $body
               }

               inventory::submit! {
                   $crate::harness::ConformanceTestEntry {
                       make: || -> Box<dyn $crate::harness::ConformanceTest> {
                           Box::new([<
                               __conformance_
                               $adapter:snake _
                               $name:snake
                           >])
                       },
                   }
               }
           }
       };
   }
   ```

3. Add `paste` to workspace deps in root `Cargo.toml`:

   ```toml
   paste = "1"
   ```

   Add to `minibox-testsuite/Cargo.toml`:

   ```toml
   paste = { workspace = true }
   ```

4. Update `harness/mod.rs` to add the module and re-export:

   ```rust
   pub mod macros;
   // Add to re-exports:
   pub use macros::ConformanceTestEntry;
   ```

5. Write test in `crates/minibox-testsuite/src/harness/macros.rs`:

   ```rust
   #[cfg(test)]
   mod tests {
       // Invoke the macro to verify it compiles and registers.
       crate::conformance_test! {
           name: "macro_smoke_test",
           adapter: "macro_test",
           category: Unit,
           |ctx| {
               ctx.assert_true(true, "macro works");
               ctx.result()
           }
       }

       crate::conformance_test! {
           name: "macro_with_capability",
           adapter: "macro_test",
           capability: Commit,
           category: Unit,
           |ctx| {
               ctx.result()
           }
       }

       #[test]
       fn macro_generated_tests_are_in_inventory() {
           let count = inventory::iter::<super::ConformanceTestEntry>
               .into_iter()
               .filter(|e| (e.make)().adapter() == "macro_test")
               .count();
           // At least the 2 tests declared above.
           assert!(count >= 2, "expected >= 2 macro_test entries, got {count}");
       }
   }
   ```

6. Verify:

   ```
   cargo nextest run -p minibox-testsuite  -> all green
   cargo clippy -p minibox-testsuite -- -D warnings  -> zero
   ```

7. Commit: `git commit -m "feat(testsuite): add conformance_test! macro with inventory registration"`

---

### Task 6: Add `collect_inventory` and `with_descriptor` to `TestRunner`

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/harness/runner.rs`
**Run**: `cargo nextest run -p minibox-testsuite`

1. Write failing test:

   ```rust
   #[test]
   fn collect_inventory_finds_registered_tests() {
       let runner = TestRunner::collect_inventory();
       // Must find at least the macro smoke tests from Task 5.
       assert!(runner.count() > 0);
   }
   ```

   Expected: FAIL (no `collect_inventory` method)

2. Implement. Add to `TestRunner`:

   ```rust
   use minibox_core::adapters::conformance::BackendDescriptor;
   use super::macros::ConformanceTestEntry;

   pub struct TestRunner {
       tests: Vec<Box<dyn ConformanceTest>>,
       filter: RunnerFilter,
       descriptor: Option<BackendDescriptor>,
   }
   ```

   Update `TestRunner::new()` to initialize `descriptor: None`.

   Add methods:

   ```rust
   /// Collect all tests registered via `inventory`.
   #[must_use]
   pub fn collect_inventory() -> Self {
       let tests: Vec<Box<dyn ConformanceTest>> =
           inventory::iter::<ConformanceTestEntry>
               .into_iter()
               .map(|entry| (entry.make)())
               .collect();
       Self {
           tests,
           filter: RunnerFilter::default(),
           descriptor: None,
       }
   }

   /// Set the backend descriptor for capability-based auto-skip.
   #[must_use]
   pub fn with_descriptor(mut self, desc: BackendDescriptor) -> Self {
       self.descriptor = Some(desc);
       self
   }
   ```

   Update `run()` to use the descriptor for auto-skip and pass it to
   `TestContext`:

   ```rust
   pub fn run(&self) -> TestSummary {
       let suite_start = Instant::now();
       let mut results = Vec::new();

       for test in &self.tests {
           if !self.passes_filter(test.as_ref()) {
               continue;
           }

           // Auto-skip if required capability is not supported.
           if let Some(cap) = test.required_capability() {
               if let Some(ref desc) = self.descriptor {
                   if !desc.capabilities.supports(cap) {
                       results.push(TestRunResult {
                           id: test.id(),
                           name: test.name().to_string(),
                           adapter: test.adapter().to_string(),
                           category: test.category(),
                           result: TestResult::Skipped {
                               reason: format!(
                                   "backend does not support {cap:?}"
                               ),
                           },
                           duration_ms: 0,
                           failures: Vec::new(),
                       });
                       continue;
                   }
               }
           }

           let start = Instant::now();
           let mut ctx = match &self.descriptor {
               Some(desc) => TestContext::with_descriptor(desc),
               None => TestContext::new(),
           };
           let result = test.run_sync(&mut ctx);
           let duration_ms = start.elapsed().as_millis() as u64;

           let failures = ctx.failures().to_vec();
           results.push(TestRunResult {
               id: test.id(),
               name: test.name().to_string(),
               adapter: test.adapter().to_string(),
               category: test.category(),
               result,
               duration_ms,
               failures,
           });
       }

       let mut summary = TestSummary {
           duration_ms: suite_start.elapsed().as_millis() as u64,
           ..Default::default()
       };
       for r in &results {
           summary.total += 1;
           match &r.result {
               TestResult::Pass => summary.passed += 1,
               TestResult::Fail { .. } => summary.failed += 1,
               TestResult::Skipped { .. } => summary.skipped += 1,
           }
       }
       summary.results = results;
       summary
   }
   ```

3. Verify:

   ```
   cargo nextest run -p minibox-testsuite  -> all green
   cargo clippy -p minibox-testsuite -- -D warnings  -> zero
   ```

4. Commit: `git commit -m "feat(testsuite): add collect_inventory and with_descriptor to TestRunner"`

---

### Task 7: Convert `registry` adapter module as proof

**Crate**: `minibox-testsuite`
**File(s)**: `crates/minibox-testsuite/src/adapters/registry.rs`
**Run**: `cargo nextest run -p minibox-testsuite`

1. Rewrite `registry.rs` using the macro. Replace all 6 hand-written
   structs + `all()` with:

   ```rust
   //! Conformance tests for the `ImageRegistry` trait contract.

   use minibox::testing::mocks::registry::MockRegistry;
   use minibox_core::domain::ImageRegistry;
   use minibox_core::image::reference::ImageRef;

   fn alpine() -> ImageRef {
       ImageRef::parse("alpine:3.18").expect("parse alpine ref")
   }

   fn rt() -> tokio::runtime::Runtime {
       tokio::runtime::Runtime::new().expect("build Tokio runtime")
   }

   crate::conformance_test! {
       name: "pull_increments_count",
       adapter: "registry",
       category: Unit,
       |ctx| {
           let registry = MockRegistry::new();
           rt().block_on(registry.pull_image(&alpine()))
               .expect("pull");
           ctx.assert_eq(
               1,
               registry.pull_count(),
               "pull_count after one pull",
           );
           ctx.result()
       }
   }

   crate::conformance_test! {
       name: "multiple_pulls_increment_count",
       adapter: "registry",
       category: Unit,
       |ctx| {
           let registry = MockRegistry::new();
           let image = alpine();
           for _ in 0..4 {
               rt().block_on(registry.pull_image(&image))
                   .expect("pull");
           }
           ctx.assert_eq(
               4,
               registry.pull_count(),
               "pull_count after 4 pulls",
           );
           ctx.result()
       }
   }

   crate::conformance_test! {
       name: "has_image_after_pull",
       adapter: "registry",
       category: Unit,
       |ctx| {
           let registry = MockRegistry::new();
           let r = alpine();
           rt().block_on(registry.pull_image(&r)).expect("pull");
           ctx.assert_true(
               rt().block_on(
                   registry.has_image(&r.cache_name(), &r.tag),
               ),
               "has_image after pull",
           );
           ctx.result()
       }
   }

   crate::conformance_test! {
       name: "fresh_registry_has_no_images",
       adapter: "registry",
       category: EdgeCase,
       |ctx| {
           let registry = MockRegistry::new();
           ctx.assert_false(
               rt().block_on(registry.has_image("alpine", "3.18")),
               "no images before pull",
           );
           ctx.assert_eq(
               0,
               registry.pull_count(),
               "pull_count starts at zero",
           );
           ctx.result()
       }
   }

   crate::conformance_test! {
       name: "pull_failure_registry_returns_err",
       adapter: "registry",
       category: EdgeCase,
       |ctx| {
           let registry = MockRegistry::new().with_pull_failure();
           let result = rt().block_on(registry.pull_image(&alpine()));
           ctx.assert_err(
               result,
               "pull_failure registry must return Err",
           );
           ctx.result()
       }
   }

   crate::conformance_test! {
       name: "pull_count_incremented_on_failure",
       adapter: "registry",
       category: EdgeCase,
       |ctx| {
           let registry = MockRegistry::new().with_pull_failure();
           let _ = rt().block_on(registry.pull_image(&alpine()));
           ctx.assert_eq(
               1,
               registry.pull_count(),
               "pull_count incremented even on failure",
           );
           ctx.result()
       }
   }
   ```

2. Remove the `pub fn all()` from `registry.rs` — it is no longer
   needed.

3. Verify the 6 registry tests still appear in inventory and pass:

   ```
   cargo nextest run -p minibox-testsuite  -> all green
   ```

4. Commit: `git commit -m "refactor(testsuite): convert registry adapter to conformance_test! macro"`

---

### Task 8: Convert remaining 21 adapter modules

**Crate**: `minibox-testsuite`
**File(s)**: all files in `crates/minibox-testsuite/src/adapters/`
except `registry.rs` and `mod.rs`
**Run**: `cargo nextest run -p minibox-testsuite`

1. For each module, apply the same pattern as Task 7:
   - Replace each struct + `impl ConformanceTest` with a
     `crate::conformance_test! { ... }` invocation
   - Add `capability:` field where the test exercises a specific
     `BackendCapability` (e.g., `capability: Commit` for
     `container_committer`, `capability: Exec` for `exec_runtime`,
     `capability: Network` for `network`, etc.)
   - Remove the module's `pub fn all()` function
   - Keep helper functions (like `alpine()`, `rt()`) as module-level
     fns

2. Capability mapping for each module:

   | Module | Capability |
   |--------|-----------|
   | `container_committer` | `Commit` |
   | `image_builder` | `BuildFromContext` |
   | `image_pusher` | `PushToRegistry` |
   | `vm_checkpoint` | `Checkpoint` |
   | `filesystem` | `Filesystem` |
   | `exec_runtime` | `Exec` |
   | `network` | `Network` |
   | `pty` | `Pty` |
   | `metrics` | `Metrics` |
   | `registry_router` | `RegistryRouter` |
   | `image_loader` | `ImageLoader` |
   | `registry` | (none — already done) |
   | `runtime` | (none — general runtime) |
   | `limiter` | (none — general resource) |
   | `state` | (none — state management) |
   | `pause_resume` | (none — lifecycle) |
   | `list` | (none — listing) |
   | `policy` | (none — policy) |
   | `container_id` | (none — ID generation) |
   | `logs` | (none — log retrieval) |
   | `remove` | (none — cleanup) |
   | `stop_handler` | (none — lifecycle) |

3. After converting all modules, verify:

   ```
   cargo nextest run -p minibox-testsuite  -> all green
   cargo clippy -p minibox-testsuite -- -D warnings  -> zero
   ```

4. Commit: `git commit -m "refactor(testsuite): convert all adapter modules to conformance_test! macro"`

---

### Task 9: Remove `adapters::all()` and `SpokeRegistry`

**Crate**: `minibox-testsuite`
**File(s)**:
- `crates/minibox-testsuite/src/adapters/mod.rs`
- `crates/minibox-testsuite/src/spoke.rs`
- `crates/minibox-testsuite/src/lib.rs`
**Run**: `cargo nextest run -p minibox-testsuite`

1. In `adapters/mod.rs`, remove the `all()` function entirely. Keep
   the `pub mod` declarations (modules still exist, they just register
   via inventory now instead of returning `Vec`s).

   Remove the `use crate::harness::ConformanceTest;` import if no
   longer needed.

2. Delete `crates/minibox-testsuite/src/spoke.rs`.

3. In `lib.rs`, remove `pub mod spoke;`. Update the prelude if it
   re-exports spoke types.

4. Verify:

   ```
   cargo nextest run -p minibox-testsuite  -> all green
   cargo clippy -p minibox-testsuite -- -D warnings  -> zero
   ```

5. Commit: `git commit -m "refactor(testsuite): remove SpokeRegistry and adapters::all()"`

---

### Task 10: Update binaries to use `collect_inventory`

**Crate**: `minibox-testsuite`
**File(s)**:
- `crates/minibox-testsuite/src/bin/run_conformance.rs`
- `crates/minibox-testsuite/src/bin/generate_report.rs`
**Run**: `cargo run -p minibox-testsuite --bin run-conformance`

1. Rewrite `run_conformance.rs`:

   ```rust
   //! `run-conformance` -- execute all conformance tests and report.
   #![allow(clippy::expect_used)]

   use minibox_testsuite::harness::{
       ReportConfig, ReportGenerator, TestRunner,
   };

   fn main() {
       let adapter_filter =
           std::env::var("CONFORMANCE_ADAPTER").ok();
       let verbose =
           std::env::var("CONFORMANCE_VERBOSE")
               .is_ok_and(|v| v == "1");

       let runner = TestRunner::collect_inventory();

       let runner = if let Some(ref name) = adapter_filter {
           runner.filter_adapter(name)
       } else {
           runner
       };

       eprintln!(
           "Running {} conformance tests...",
           runner.filtered_count(),
       );

       let summary = runner.run();

       let cfg = ReportConfig {
           verbose,
           summary_only: false,
           show_timing: true,
       };
       let mut stdout = std::io::stdout();
       ReportGenerator::text(&mut stdout, &summary, &cfg)
           .expect("write report");

       if std::env::var("GITHUB_ACTIONS").is_ok() {
           ReportGenerator::github_actions(&mut stdout, &summary)
               .expect("write GH annotations");
       }

       if !summary.is_success() {
           std::process::exit(1);
       }
   }
   ```

2. Rewrite `generate_report.rs`:

   ```rust
   //! `generate-report` -- run all conformance tests and write
   //! JSON + JUnit XML reports.
   #![allow(clippy::expect_used)]

   use std::fs;
   use std::path::PathBuf;

   use minibox_testsuite::harness::{
       ReportConfig, ReportGenerator, TestRunner,
   };

   fn main() {
       let artifact_dir =
           std::env::var("CONFORMANCE_ARTIFACT_DIR")
               .map_or_else(
                   |_| PathBuf::from("artifacts/conformance"),
                   PathBuf::from,
               );

       fs::create_dir_all(&artifact_dir)
           .expect("create artifact dir");

       let runner = TestRunner::collect_inventory();

       eprintln!(
           "Running {} conformance tests...",
           runner.count(),
       );
       let summary = runner.run();

       let cfg = ReportConfig {
           verbose: true,
           summary_only: false,
           show_timing: true,
       };
       let mut text_out = Vec::new();
       ReportGenerator::text(&mut text_out, &summary, &cfg)
           .expect("text report");
       eprint!("{}", String::from_utf8_lossy(&text_out));

       let json_path = artifact_dir.join("conformance.json");
       let mut f = fs::File::create(&json_path)
           .expect("create json report");
       ReportGenerator::json(&mut f, &summary)
           .expect("write json");
       println!("conformance:json={}", json_path.display());

       let junit_path = artifact_dir.join("conformance.xml");
       let mut f = fs::File::create(&junit_path)
           .expect("create junit report");
       ReportGenerator::junit_xml(&mut f, &summary)
           .expect("write junit");
       println!("conformance:junit={}", junit_path.display());

       let md_path = artifact_dir.join("conformance.md");
       let mut f = fs::File::create(&md_path)
           .expect("create markdown report");
       ReportGenerator::markdown(&mut f, &summary)
           .expect("write markdown");
       println!(
           "conformance:markdown={}",
           md_path.display(),
       );

       println!(
           "conformance:summary {}/{} passed, \
            {} failed, {} skipped in {}ms",
           summary.passed,
           summary.total,
           summary.failed,
           summary.skipped,
           summary.duration_ms,
       );

       if !summary.is_success() {
           std::process::exit(1);
       }
   }
   ```

3. Verify both binaries build and run:

   ```
   cargo run -p minibox-testsuite --bin run-conformance
   cargo run -p minibox-testsuite --bin generate-report
   ```

4. Commit: `git commit -m "refactor(testsuite): update binaries to use collect_inventory"`

---

### Task 11: Update external crate conformance tests

**Crate**: `smolbox`, `winbox`, `macbox`, `minibox`
**File(s)**: any `tests/conformance_*.rs` files that reference
`SpokeRegistry` or `adapters::all()`
**Run**: `cargo check --workspace`

1. Search for all references to `SpokeRegistry` or `spoke::` outside
   `minibox-testsuite`:

   ```
   rg "SpokeRegistry|spoke::" crates/ --type rust \
     --glob '!**/minibox-testsuite/**'
   ```

2. For each hit, decide:
   - If the test file was registering spoke tests, convert to
     `conformance_test!` macro invocations
   - If the test file was consuming `adapters::all()`, switch to
     `TestRunner::collect_inventory()`

3. Verify:

   ```
   cargo check --workspace  -> all green
   cargo nextest run --workspace  -> all green
   ```

4. Commit: `git commit -m "refactor: update external crate conformance tests for inventory collection"`

---

### Task 12: Final verification and cleanup

**Run**: `cargo xtask verify`

1. Run the full local gate:

   ```
   cargo xtask verify
   ```

2. Run the conformance binary end-to-end:

   ```
   cargo run -p minibox-testsuite --bin run-conformance -- \
     CONFORMANCE_VERBOSE=1
   ```

3. Verify test counts match expectations. The total test count should
   be unchanged (same tests, just registered differently).

4. Remove any dead imports, unused `pub fn all()` remnants, or stale
   spoke references found by clippy.

5. Commit: `git commit -m "chore(testsuite): final cleanup after conformance macro migration"`

## Risk

- [x] Breaking API: `TestContext` gains lifetime. All test impls must
  update. Contained to `publish = false` test crates.
- [x] New deps: `inventory` (0 transitive), `paste` (0 transitive).
  Both are stable, widely used.
- [ ] Feature flag: no new flags. `inventory` in `minibox-core` uses
  existing `test-utils`.
- [ ] Semver: no production API changes. All changes are in test infra.
