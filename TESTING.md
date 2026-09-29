---
source_sha: a72281f338bd3ea9b790b77145de108c97281f20
sources:
  - xtask/src/main.rs
  - Justfile
  - fuzz
  - crates/minibox-testsuite
  - crates/minibox/src/testing
  - crates/minibox-core/src/adapters
  - .github/workflows
generated: 2026-09-16
---

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
cargo xtask test unit          # workspace library tests
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
cargo xtask test property      # proptest suites
```

This suite runs on Linux and macOS in `conformance.yml`.

---

## Dimension 3: Fuzz — for unsafe, parsers, and protocol boundaries

If a function touches raw bytes, parses external input, handles untrusted
data, or contains `unsafe`, fuzz it. Property tests generate structured
inputs; fuzz tests generate arbitrary byte sequences.

- Use `cargo-fuzz` (libFuzzer); targets in `fuzz/fuzz_targets/`
- Seed corpus in `fuzz/corpus/<target>/` — local only; corpora, seeds, and artifacts are ignored
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
cargo xtask test conformance          # run + report (JSON/JUnit)
cargo xtask test krun-conformance     # krun-specific variant
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

# Protocol E2E — any platform, no root
just test-e2e

# Full-stack daemon+CLI — Linux + root
just test-system            # local suite
just test-e2e-vps           # run suite on VPS via SSH

# Adapter swap tests — cross-platform
just test-adapters
just test-cli-subprocess    # 30 CLI subprocess tests

# Sandbox — Linux + root
cargo xtask test sandbox
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

The current tree contains **96 integration test files** under `crates/*/tests/`, with
approximately **1,060 integration-test annotations** and **1,228 inline test annotations**.
Counts are source annotations rather than a claim that every platform executes every test:
Linux-only, root-required, ignored, and feature-gated tests are selected by separate suites.

---

## Test Helpers

All helpers live behind the `test-utils` feature flag.

**`minibox::testing`** — enabled with `--features test-utils`; handler tests target the split
`crates/minibox/src/daemon/handler/` module rather than an obsolete monolithic `handler.rs`:

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

CI responsibilities are listed by workflow name rather than a hard-coded workflow count:

| Workflow | Primary purpose |
| --- | --- |
| `ci.yml` | Core checks for main/staging/release and PRs to main |
| `pr.yml` | PR lint, unit, protocol, and docs jobs |
| `merge.yml` | Push/merge-group matrix across active workflow branches |
| `macos.yml` | macOS `cargo fmt --all --check` |
| `conformance.yml` | Conformance, property, krun, CLI, borrow, and quickcheck suites |
| `context-snapshot.yml` | Cross-platform v3 context evidence collection and aggregate validation |
| `stability-gates.yml` | docs, adapter coverage, no-unwrap, compile, and handler coverage |
| `protocol-drift.yml` | Protocol hash/variant drift |
| `protocol-sites.yml` | Handler dependency construction-site drift |
| `rust-clippy.yml` | Clippy review workflow |
| `nightly.yml` | Scheduled audits and coverage check |
| `promote.yml` | Stability-branch promotion |
| `release.yml` | crates and binary release workflow |
| `publish-mbx.yml` | CLI package publishing |
| `summary.yml` | Issue summary automation |

### Remaining CI gaps

- Sandbox tests: `cargo xtask test sandbox` (Linux + root)
- Feature-gated CLI subprocess suite: `just test-cli-subprocess`

Property tests, borrow fixtures, krun conformance, and the handler coverage gate all have CI jobs.

---

## Coverage

```bash
just coverage                # HTML report at target/llvm-cov/html/
cargo xtask coverage-check   # gate: daemon handler module function coverage >= 80%
```

## Cleaning Test State

```bash
cargo xtask nuke-test-state   # kill orphans, unmount overlays, clean cgroups
just clean-test               # remove test binaries
```
