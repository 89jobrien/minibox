# minibox-conformance

Conformance test harness for minibox adapter trait contracts.

## Purpose

This crate verifies that each adapter (registry, runtime, limiter, state) correctly implements
its domain port contract as defined in `minibox-core/src/domain.rs`. Tests use mock adapters
from `minibox::testing::mocks` — no kernel interaction, network calls, or daemon process required.

## Running the suite

Run the full suite and print a pass/fail summary:

```
cargo run -p minibox-conformance --bin run-conformance
```

Generate JSON and JUnit XML reports in `artifacts/`:

```
cargo run -p minibox-conformance --bin generate-report
```

Both binaries exit `0` on success and `1` on any test failure.

## Test count and categories

28 conformance tests across four adapter modules:

| Adapter    | Tests | Notes                                           |
| ---------- | ----- | ----------------------------------------------- |
| `limiter`  | 7     | `ResourceLimiter` — cgroup lifecycle contract   |
| `registry` | 6     | `ImageRegistry` — pull count and has_image      |
| `runtime`  | 8     | `ContainerRuntime` — spawn, PIDs, sync/async    |
| `state`    | 7     | `DaemonState` — add/remove/list/persist/name    |

Categories used in the harness:

- `unit` — single trait method or invariant in isolation
- `integration` — interactions between multiple trait implementations
- `edge_case` — boundary conditions, empty inputs, and error paths

## Structure

```
crates/minibox-conformance/
  src/
    harness/          ConformanceTest trait, TestContext, TestRunner, ReportGenerator
    adapters/         per-adapter test modules (registry, runtime, limiter, state)
    bin/
      run_conformance.rs     CLI: run all tests, exit 1 on failure
      generate_report.rs     CLI: run tests, write JSON + JUnit reports to artifacts/
```

## Adding a new conformance test

1. Add a struct in the relevant `src/adapters/<adapter>.rs` file.
2. Implement `ConformanceTest` — provide `name()`, `adapter()`, `category()`, and `run_sync()`.
3. Append `Box::new(YourStruct)` to the `all()` function in that file.
4. Verify with `cargo run -p minibox-conformance --bin run-conformance`.

Example skeleton:

```rust
pub struct MyNewTest;
impl ConformanceTest for MyNewTest {
    fn name(&self) -> &str { "my_new_test" }
    fn adapter(&self) -> &str { "runtime" }
    fn category(&self) -> TestCategory { TestCategory::Unit }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let runtime = MockRuntime::new();
        // ... drive the mock, call ctx.assert_* methods ...
        ctx.result()
    }
}
```

## Relation to other test categories

| Category          | Command                                        | Requires root/Linux |
| ----------------- | ---------------------------------------------- | ------------------- |
| Conformance       | `cargo run -p minibox-conformance --bin run-conformance` | No      |
| Unit              | `cargo xtask test-unit`                        | No                  |
| Integration       | `just test-integration`                        | Yes (cgroups)       |
| E2E               | `just test-e2e`                                | Yes (daemon)        |

Conformance tests are the fastest gate and safe to run on any platform.
