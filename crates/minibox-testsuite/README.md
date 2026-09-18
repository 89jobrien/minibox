# minibox-testsuite

Conformance test harness for minibox adapter trait contracts.

## Purpose

This crate verifies that each adapter (registry, runtime, limiter, state) correctly implements
its domain port contract as defined in `minibox-domain/src/`. Tests use mock adapters
from `minibox::testing::mocks` — no kernel interaction, network calls, or daemon process required.

## Running the suite

Run the full suite and print a pass/fail summary:

```
cargo run -p minibox-testsuite --bin run-conformance
```

Generate JSON and JUnit XML reports in `artifacts/`:

```
cargo run -p minibox-testsuite --bin generate-report
```

Both binaries exit `0` on success and `1` on any test failure.

## Test count and categories

The current inventory contains 123 tests across 22 adapter and port categories.
The runner reports the authoritative count at startup and pins a minimum of 123
tests so dropped inventory registrations cannot silently shrink the suite.

Categories used in the harness:

- `unit` — single trait method or invariant in isolation
- `integration` — interactions between multiple trait implementations
- `edge_case` — boundary conditions, empty inputs, and error paths

## Structure

```
crates/minibox-testsuite/
  src/
    harness/          ConformanceTest trait, TestContext, TestRunner, ReportGenerator
    adapters/         per-port contract modules (registry, runtime, limiter, state, etc.)
    bin/
      run_conformance.rs     CLI: run all tests, exit 1 on failure
      generate_report.rs     CLI: run tests, write JSON + JUnit reports to artifacts/
```

## Adding a new conformance test

1. Add a `conformance_test!` invocation in the relevant `src/adapters/<adapter>.rs` file.
2. Declare the test name, adapter category, optional required capability, and test body.
3. Verify with `cargo run -p minibox-testsuite --bin run-conformance`.

Example skeleton:

```rust,ignore
crate::conformance_test! {
    name: "my_new_test",
    adapter: "runtime",
    category: Unit,
    |ctx| {
        // Drive the mock and call ctx.assert_* methods.
        ctx.result()
    }
}
```

## Relation to other test categories

| Category    | Command                                                | Requires root/Linux |
| ----------- | ------------------------------------------------------ | ------------------- |
| Conformance | `cargo run -p minibox-testsuite --bin run-conformance` | No                  |
| Unit        | `cargo xtask test unit`                                | No                  |
| Integration | `just test-integration`                                | Yes (cgroups)       |
| E2E         | `just test-e2e`                                        | No                  |

Conformance tests are the fastest gate and safe to run on any platform.
