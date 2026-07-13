# Design: Declarative Conformance Test Macro with Auto-Collection

## Goal

Replace hand-written `ConformanceTest` struct boilerplate with a
`conformance_test!` macro that auto-generates struct + trait impl +
`inventory` registration, extend `BackendDescriptor` to cover all 12
`BackendCapability` variants, wire capability-based auto-skip through
`TestContext`, and remove `SpokeRegistry` in favor of `inventory`-based
cross-crate collection.

## Approved Approach

- **A+B**: declarative spec replacing boilerplate + full-capability
  descriptor
- **C2**: `macro_rules!` macro + `inventory` for auto-collection
- **D3**: hybrid descriptor (3 named fields + `extras` map)
- **E1**: `TestContext` carries `&BackendDescriptor`
- **F1**: replace `SpokeRegistry` with `inventory`

## Crate Ownership

| Change | Owner crate | Reason |
|--------|-------------|--------|
| `conformance_test!` macro | `minibox-testsuite` | test infra crate, `macro_rules!` needs no proc-macro crate |
| `BackendDescriptor` expansion | `minibox-core` | descriptor already lives here under `test-utils` feature |
| `TestContext` descriptor field | `minibox-testsuite` | context is owned by the harness |
| `inventory` integration | `minibox-testsuite` + `minibox-core` | both need the dep |
| `SpokeRegistry` removal | `minibox-testsuite` | sole owner |

**Affected crates** (consumers that register tests):
`minibox-testsuite` (built-in adapters), `smolbox`, `winbox`,
`macbox`, `minibox` (integration tests that use spoke pattern today).

## Public API

### `minibox-core::adapters::conformance` changes

```rust
use std::any::Any;
use std::collections::HashMap;

/// Type-erased factory for capabilities beyond the 3 named fields.
///
/// Keyed by `BackendCapability`. Values are `Box<dyn Any + Send + Sync>`
/// wrapping a `Box<dyn Fn() -> DynTrait + Send + Sync>` for the
/// corresponding trait object.
pub type CapabilityExtras =
    HashMap<BackendCapability, Box<dyn Any + Send + Sync>>;

// Existing BackendDescriptor gains one new field:
pub struct BackendDescriptor {
    pub name: &'static str,
    pub capabilities: BackendCapabilitySet,

    // --- existing named fields (unchanged) ---
    pub make_committer:
        Option<Box<dyn Fn() -> DynContainerCommitter + Send + Sync>>,
    pub make_builder:
        Option<Box<dyn Fn() -> DynImageBuilder + Send + Sync>>,
    pub make_pusher:
        Option<Box<dyn Fn() -> DynImagePusher + Send + Sync>>,

    // --- new ---
    pub extras: CapabilityExtras,
}

impl BackendDescriptor {
    /// Register a type-erased factory for any capability.
    pub fn with_extra<T: Send + Sync + 'static>(
        self,
        cap: BackendCapability,
        factory: Box<dyn Fn() -> T + Send + Sync>,
    ) -> Self;

    /// Retrieve a typed factory from extras. Returns `None` if the
    /// capability is absent or the type doesn't match.
    pub fn extra<T: 'static>(
        &self,
        cap: BackendCapability,
    ) -> Option<&(dyn Fn() -> T + Send + Sync)>;
}
```

### `minibox-testsuite::harness::context` changes

```rust
use minibox_core::adapters::conformance::BackendDescriptor;

pub struct TestContext<'d> {
    failures: Vec<String>,
    log: Vec<LogEntry>,
    descriptor: Option<&'d BackendDescriptor>,
}

impl<'d> TestContext<'d> {
    /// Create a context with no descriptor (backward compat for
    /// standalone tests).
    pub const fn new() -> Self;

    /// Create a context bound to a backend descriptor.
    pub const fn with_descriptor(
        descriptor: &'d BackendDescriptor,
    ) -> Self;

    /// Access the backend descriptor. Panics if none was set
    /// (programming error in harness, not in test).
    pub fn descriptor(&self) -> &BackendDescriptor;

    /// Check if the backend supports a capability. Returns false
    /// (and logs skip reason) if no descriptor is set.
    pub fn supports(
        &self,
        cap: minibox_core::domain::BackendCapability,
    ) -> bool;

    // ... existing assert_* methods unchanged ...
}
```

### `minibox-testsuite::harness::traits` changes

```rust
use super::context::TestContext;

/// Updated trait — lifetime parameter on TestContext.
pub trait ConformanceTest: Send + Sync {
    fn name(&self) -> &str;
    fn adapter(&self) -> &str;
    fn category(&self) -> TestCategory;

    /// Optional: declare required capability for auto-skip.
    /// Default: `None` (always runs).
    fn required_capability(
        &self,
    ) -> Option<minibox_core::domain::BackendCapability> {
        None
    }

    fn run_sync(&self, ctx: &mut TestContext<'_>) -> TestResult;

    fn id(&self) -> String {
        format!("{}::{}", self.adapter(), self.name())
    }
}
```

### `minibox-testsuite` — the macro

```rust
/// Declare a conformance test with auto-registration.
///
/// # Usage
///
/// ```rust,ignore
/// conformance_test! {
///     name: "pull_increments_count",
///     adapter: "registry",
///     category: Unit,
///     |ctx| {
///         let registry = MockRegistry::new();
///         // ... test logic ...
///         ctx.result()
///     }
/// }
///
/// // With capability-gated auto-skip:
/// conformance_test! {
///     name: "commit_roundtrip",
///     adapter: "container_committer",
///     capability: Commit,
///     category: Unit,
///     |ctx| {
///         // Only runs if backend declares Commit capability.
///         // Auto-skipped otherwise.
///         ctx.result()
///     }
/// }
/// ```
///
/// Expands to:
/// 1. A unit struct with a name derived from adapter + test name
/// 2. `impl ConformanceTest for ...`
/// 3. `inventory::submit!(...)`
#[macro_export]
macro_rules! conformance_test {
    // Variant with capability
    (
        name: $name:expr,
        adapter: $adapter:expr,
        capability: $cap:ident,
        category: $cat:ident,
        |$ctx:ident| $body:block
    ) => { ... };

    // Variant without capability (always runs)
    (
        name: $name:expr,
        adapter: $adapter:expr,
        category: $cat:ident,
        |$ctx:ident| $body:block
    ) => { ... };
}
```

### `minibox-testsuite::harness::runner` changes

```rust
impl TestRunner {
    /// Collect all tests registered via `inventory::iter`.
    pub fn collect_inventory() -> Self;

    /// Set the backend descriptor used for capability auto-skip.
    /// The runner injects this into each `TestContext`.
    pub fn with_descriptor(
        self,
        descriptor: BackendDescriptor,
    ) -> Self;
}
```

### Inventory registration type

```rust
/// Wrapper submitted to `inventory` by each `conformance_test!`
/// invocation. Holds a constructor function rather than the test
/// itself to avoid static initialization order issues.
pub struct ConformanceTestEntry {
    pub make: fn() -> Box<dyn ConformanceTest>,
}

inventory::collect!(ConformanceTestEntry);
```

### Removals

```rust
// DELETED: minibox-testsuite/src/spoke.rs (entire module)
// DELETED: `pub mod spoke;` from lib.rs
// DELETED: `adapters::all()` function (replaced by inventory)
// DELETED: per-module `pub fn all()` in each adapters/*.rs
```

## Data Flow

1. **Registration** (compile time): each `conformance_test!` invocation
   emits an `inventory::submit!(ConformanceTestEntry { make: ... })`.
2. **Collection** (runtime): `TestRunner::collect_inventory()` iterates
   `inventory::iter::<ConformanceTestEntry>` and calls each `make()`.
3. **Filtering**: runner applies `RunnerFilter` (adapter, category, name).
4. **Execution**: for each test, runner creates
   `TestContext::with_descriptor(&descriptor)`. If the test declares
   `required_capability()`, the macro-generated `run_sync` checks
   `ctx.supports(cap)` and returns `Skipped` before entering the
   user closure.
5. **Reporting**: unchanged — `TestSummary` flows to `ReportGenerator`.

## Hexagonal Boundaries

No new hexagonal boundaries. This is internal test infrastructure.
`BackendDescriptor` remains a test-utils type behind the `test-utils`
feature flag in `minibox-core`.

## Migration Path

1. Add `inventory` dep to `minibox-testsuite` and `minibox-core`.
2. Expand `BackendDescriptor` with `extras` field + helpers.
3. Add `descriptor` field to `TestContext`, update lifetime.
4. Add `required_capability()` default method to `ConformanceTest`.
5. Update `TestRunner` with `collect_inventory()` +
   `with_descriptor()`.
6. Define `ConformanceTestEntry` and `conformance_test!` macro.
7. Convert one adapter module (e.g., `registry.rs`) as proof.
8. Convert remaining 21 adapter modules.
9. Remove `SpokeRegistry`, `adapters::all()`, per-module `all()`.
10. Update `run_conformance.rs` and `generate_report.rs` binaries.

## Out of Scope

- BAML schema codegen for test fixtures
- `cargo xtask` template scaffolding
- Parallel test execution in `TestRunner`
- Converting tests outside `minibox-testsuite` (e.g., crate-level
  `tests/conformance_*.rs` files) — those are separate integration
  tests, not part of the harness

## Risk

- [x] Breaking API changes: yes — `TestContext` gains a lifetime
  parameter. All `ConformanceTest` impls and callers of
  `TestContext::new()` must update. Contained to test code only
  (no production API change). `minibox-testsuite` is `publish = false`.
- [x] New external dependency: yes — `inventory` (zero transitive deps,
  widely used, stable). Only needed in test infrastructure crates.
- [ ] Feature flag required: no — `inventory` is unconditional in
  `minibox-testsuite`; in `minibox-core` it goes under existing
  `test-utils` feature.

## Alternatives Considered

| Alternative | Why rejected |
|-------------|-------------|
| Proc macro | Extra crate, compile-time cost, overkill for struct gen |
| `linkme` instead of `inventory` | Less portable (linker-section tricks), `inventory` is simpler |
| Flat 12-field descriptor | Too verbose, most backends only use 3-4 capabilities |
| `HashMap` only descriptor | Loses type safety on the common-path fields |
