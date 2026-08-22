# Testing Guide

Full test strategy for minibox. Tests follow a six-dimension progression —
each dimension unlocks confidence at a different stage. Applying the wrong
dimension at the wrong stage wastes effort; skipping one leaves a gap.

## The Six Dimensions

```
Idea -> Unit -> Property -> Fuzz -> Conformance -> Integration -> Regression
         |          |        |           |              |              |
      always   non-trivial  |        new impl        wiring        bug fixed
               input space  |        complete
                         unsafe/parser/
                         protocol boundary
```

| Dimension   | When to apply                          | Question it answers                                    |
| ----------- | -------------------------------------- | ------------------------------------------------------ |
| Unit        | Function exists                        | Does this function do what I think it does?            |
| Property    | Input space is non-trivial             | Does this invariant hold for all valid inputs?         |
| Fuzz        | Unsafe, parser, or protocol boundary   | Does this code survive malformed or adversarial input? |
| Conformance | Trait contract defined, impl complete  | Does this impl satisfy the contract the trait promises?|
| Integration | Components wired together              | Do these parts work correctly when connected?          |
| Regression  | Bug reproduced and fixed               | Will this specific failure mode ever recur?            |

---

## Dimension 1: Unit — always, first

Write unit tests the moment a function exists. A function without a unit
test has no verified behaviour.

- Scope: one function, pure logic
- Fakes over mocks — pass a `Vec`, not a `MockRepository`
- Live in `#[cfg(test)]` in the same file
- Name: `fn <thing>_<scenario>_<expected>()`
- Use `expect("reason")`, never bare `.unwrap()`

### Platform gating

Gate Linux-only tests explicitly. macOS `cargo check` does not validate
`#[cfg(target_os = "linux")]` paths.

```rust
#[cfg(target_os = "linux")]
#[test]
fn test_cgroup_limits() { ... }
```

### Environment mutation

`std::env::set_var` and `remove_var` are `unsafe` in Rust 2024. Serialize
any test that mutates the environment with a shared mutex.

```rust
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_env_var() {
    let _guard = ENV_LOCK.lock().unwrap();
    unsafe { std::env::set_var("KEY", "val") };
    // ...
}
```

### Commands

```bash
cargo xtask test-unit          # lib + select integration + conformance
just test-unit                 # equivalent shorthand
```

---

## Dimension 2: Property — when the input space is non-trivial

If a function accepts strings, integers, collections, or any type with many
possible values, a unit test only proves it works for the inputs you thought
of. Property tests prove invariants hold for inputs you didn't think of.

- Use `proptest`; strategies via `prop_compose!` for domain types
- Test invariants, not specific values: "output is always sorted",
  "round-trip is lossless"
- Commit `proptest-regressions/` — these are found counterexamples, never
  delete them
- Good candidates: parsers, graph operations, serialisation, arithmetic

### Commands

```bash
cargo xtask test-property      # ~46 proptest tests
```

Not in CI — run manually before pushing to `next`.

---

## Dimension 3: Fuzz — for unsafe, parsers, and protocol boundaries

If a function touches raw bytes, parses external input, handles untrusted
data, or contains `unsafe`, fuzz it. Property tests generate structured
inputs; fuzz tests generate arbitrary byte sequences.

- Use `cargo-fuzz` (libFuzzer); targets in `fuzz/fuzz_targets/`
- Seed corpus in `fuzz/corpus/<target>/` — commit representative inputs
- Run locally: `cargo xtask fuzz` or
  `cargo fuzz run <target> -- -max_total_time=60`
- Good candidates: tar extraction, OCI manifest parsing, socket framing,
  path validation, any function that calls `unsafe`
- Fuzz seeds, corpus, and artifacts are gitignored

```rust
// fuzz/fuzz_targets/fuzz_parse_manifest.rs
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = minibox_core::image::parse_manifest(s);
    }
});
```

---

## Dimension 4: Conformance — for every new `impl Trait`

A trait defines a contract. An impl that compiles is not necessarily correct.
Conformance tests verify the impl satisfies the semantic invariants the trait
promises — one suite per trait, reusable across all impls.

```rust
fn assert_port_contract<T: MyPort>(impl_under_test: T) {
    // assert every invariant the trait doc promises
}

#[test]
fn my_adapter_satisfies_port_contract() {
    assert_port_contract(MyAdapter::new());
}
```

### Commands

```bash
cargo xtask test-conformance          # run + report (Markdown/JSON)
cargo xtask test-krun-conformance     # krun-specific variant
```

### Borrow-reasoning fixtures

Standalone must-pass/must-fail Rust snippets that verify borrow-checker
expectations. Not runtime tests — they validate that certain patterns
compile (or correctly fail to compile).

```bash
cargo xtask borrow-fixtures           # 19 fixtures
```

---

## Dimension 5: Integration — when wiring is complete

Integration tests verify that components work correctly when connected.
They are slower and more expensive than unit tests — write them after the
components are individually verified, not before.

- Live in `tests/` (separate compilation unit)
- Use real I/O boundaries where the test is specifically about that boundary
- Do not duplicate unit test coverage — test the wiring, not the logic

### CLI subprocess tests

Do not use `Command::cargo_bin()`. Use the existing `find_minibox()` helper
or set `MINIBOX_TEST_BIN_DIR`.

### Protocol changes

Changes to `crates/minibox-core/src/protocol.rs` must be accompanied by
updates to handlers, CLI paths, and snapshot tests in the same commit. New
request fields use `#[serde(default)]` for wire compatibility.

### Snapshot tests

After `cargo nextest` runs, check for `.snap.new` files:

```bash
cargo insta review
```

### Commands

```bash
# Cgroup integration — Linux + root
just test-integration

# E2E daemon+CLI — Linux + root
just test-e2e               # single lifecycle scenario
just test-e2e-suite         # full suite (15 scenarios)
just test-e2e-vps           # run suite on VPS via SSH

# Adapter swap tests — cross-platform
just test-adapters
just test-cli-subprocess    # 30 CLI subprocess tests

# Sandbox — Linux + root
cargo xtask test-sandbox
```

---

## Dimension 6: Regression — after every bug fix

Every bug that reaches production or a failing test represents a gap in the
test suite. Close that gap permanently.

1. Reproduce the bug with a minimal failing test
2. Fix the bug
3. Verify the test now passes
4. If found by a property test, commit the `proptest-regressions/` file

Never fix a bug without a regression test.

### Naming

```text
regression_gh_NNN_short_description
```

Where `NNN` is the GitHub issue number. Examples:

- `regression_gh_42_symlink_escape_in_tar`
- `regression_gh_108_overlay_cleanup_on_cgroup_error`

### Rules

- Never delete a regression test. If the code it covers is removed, mark it
  `#[ignore]` with a comment explaining why.
- Regression tests live alongside related unit/integration tests in the
  appropriate crate, not in a separate file.
- If the bug was found by a property test, commit the counterexample file.

---

## Test Counts

| Category              | Count (est.) | Platform    | Root? | CI?         |
| --------------------- | ------------ | ----------- | ----- | ----------- |
| Unit (inline lib)     | ~728         | any         | no    | yes         |
| Handler + conformance | ~209         | any         | no    | partial     |
| minibox-core conform. | 126          | any         | no    | yes         |
| Adapter isolation     | ~66          | varies      | varies| partial     |
| Property (proptest)   | ~46          | any         | no    | no          |
| Security regression   | ~19          | any         | no    | yes         |
| CLI subprocess        | 30           | any         | no    | no          |
| krun conformance      | ~29          | macOS/Linux | no    | no          |
| Sandbox               | ~17          | Linux       | yes   | no          |
| E2E daemon+CLI        | 15           | Linux       | yes   | next/staging|
| Cgroup integration    | 16           | Linux       | yes   | next/staging|
| Protocol evolution    | 11           | any         | no    | yes         |

`cargo nextest` on macOS reports ~506 tests — that is the cross-platform
subset. Linux-only, feature-gated, and root-required tests add ~700 more.
Total estimate: ~1,467.

---

## Test Helpers

All helpers live behind the `test-utils` feature flag.

**`minibox::testing`** — enabled with `--features test-utils`:

- `mocks/` — `MockRegistry`, `MockFilesystem`, `MockLimiter`,
  `MockRuntime`, `MockNetwork`, `MockExecRuntime`, `MockImagePusher`,
  `MockContainerCommitter`, `MockImageBuilder`
- `fixtures/` — `ContainerFixture`, `ImageFixture`,
  `BuildContextFixture`, `PushTargetFixture`, `UpperDirFixture`
- `helpers/` — `create_test_deps_with_dir`, GC helpers, daemon helpers
- `backend/` — `BackendCapability`, `BackendDescriptor` (conformance)

**`minibox-core::adapters`** — enabled with `--features test-utils`:

- `mocks.rs` — cross-platform mock adapters
- `test_fixtures.rs` — shared fixtures
- `conformance.rs` — conformance harness

---

## CI Coverage

Eight workflows run in GitHub Actions:

| Workflow              | Tests run                                             | Branches           |
| --------------------- | ----------------------------------------------------- | ------------------ |
| `ci.yml`              | lint, unit, archive integration, audit/deny           | all                |
| `stability-gates.yml` | doc-sync, adapter integration, no-unwrap, compile     | all                |
| `conformance.yml`     | `cargo xtask test-conformance`                        | next/staging       |
| `bench-regression.yml`| criterion + 10% regression gate                       | next/staging       |
| `protocol-drift.yml`  | variant count + handler coverage (protocol.rs changes)| all                |
| `nightly.yml`         | `cargo geiger` unsafe audit (informational)           | daily cron         |
| `release.yml`         | musl cross-compile + publish                          | `v*` tags          |

### CI gaps (not covered by any workflow)

The following require manual runs before merging to `next`:

- Property tests: `cargo xtask test-property`
- Sandbox tests: `cargo xtask test-sandbox`
- CLI subprocess: `just test-cli-subprocess`
- krun conformance: `cargo xtask test-krun-conformance`
- Coverage gate: `cargo xtask coverage-check`

---

## Coverage

```bash
just coverage                # HTML report at target/llvm-cov/html/
cargo xtask coverage-check   # gate: handler.rs function coverage >= 80%
```

## Cleaning Test State

```bash
cargo xtask nuke-test-state   # kill orphans, unmount overlays, clean cgroups
just clean-test               # remove test binaries
```
