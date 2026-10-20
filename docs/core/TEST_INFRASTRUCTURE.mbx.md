---
source_sha: f5481a9482fbb04690db6b7a52ee8eca9c5fe5e9
sources:
  - crates/minibox
  - crates/minibox-core
  - crates/minibox-domain
  - crates/miniboxd
  - crates/mbx
  - crates/macbox
  - crates/minibox-testsuite
  - crates/smolbox
  - crates/winbox
  - crates/minibox-macros
  - crates/minibox-crux-plugin
  - crates/minibox-bench
  - crates/minibox-cni
  - crates/minibox-tui
  - crates/mcp
  - crates/ail
  - xtask/src/main.rs
  - .github/workflows
generated: 2026-09-16
---

# Test Infrastructure Report

> Generated 2026-04-27 from automated codebase analysis.
> Updated 2026-05-07: macOS nextest cross-platform count updated (506, was ~760).
> Updated 2026-06-14: macOS nextest count updated to 789 (789 passed, 1 skipped). minibox
> integration test file count updated to 54. minibox-testsuite and smolbox crates added to
> the crate table.
> Updated 2026-07-25: integration test file counts corrected against current
> crates/*/tests/ contents (minibox 54->56, minibox-core 7->10, miniboxd 6->13,
> mbx 2->3, smolbox 0->2, winbox 0->1). Per-crate/category test-count estimates
> not re-derived — file counts only.

## Test Counts

| Location                                  | Tests (est.) |
| ----------------------------------------- | ------------ |
| Integration test files (`crates/*/tests/*.rs`) | 96 files / ~1,060 annotations |
| Inline (`#[cfg(test)]` in `src/`)              | ~1,228 annotations            |
| **Source annotations**                         | **more than 2,200**            |

These are source-level annotations, not a single-platform executed-test count. Linux-only,
feature-gated, ignored, and root-required tests are selected by dedicated suites.

---

## Tests by Crate

| Crate              | Integration files | Integration tests | Inline tests |
| ------------------ | ----------------- | ----------------- | ------------ |
| minibox            | 56                | ~479              | ~255         |
| minibox-core       | 10                | ~126              | ~285         |
| miniboxd           | 13                | ~72               | ~24          |
| minibox-cli        | 3                 | ~32               | ~96          |
| macbox             | 5                 | ~30               | ~63          |
| minibox-testsuite  | 0                 | 0                 | ~24          |
| smolbox            | 2                 | 0                 | ~3           |
| winbox             | 1                 | 0                 | ~5           |
| minibox-macros     | 0                 | 0                 | 0            |
| minibox-domain     | inline            | —                 | included     |
| minibox-crux-plugin| integration       | —                 | included     |
| minibox-mcp        | integration       | —                 | included     |
| minibox-bench      | benches           | —                 | fixture tests|
| minibox-cni        | integration       | —                 | included     |
| minibox-tui        | inline            | —                 | included     |
| ail                | 0                 | 0                 | 0            |
| xtask              | 0                 | 0                 | 0            |

---

## Test Categories

| Category                                     | Tests (est.) | Platform    | Root?  | In CI?      |
| -------------------------------------------- | ------------ | ----------- | ------ | ----------- |
| Unit (inline lib)                            | ~1,228 annotations | any    | no     | yes         |
| Handler + daemon conformance                 | ~209         | any         | no     | partial     |
| minibox-core conformance                     | 126          | any         | no     | yes         |
| Adapter isolation (colima/gke/native/smolvm) | ~66          | varies      | varies | partial     |
| Property tests (proptest)                    | ~46          | any         | no     | yes         |
| Borrow-reasoning fixtures                    | 19           | any         | no     | yes         |
| Security regression                          | ~19          | any         | no     | yes         |
| Cgroup integration                           | 16           | Linux       | yes    | promotion branches |
| Protocol E2E                                 | varies       | any         | no     | promotion branches |
| Full-stack daemon+CLI system                 | 15           | Linux       | yes    | promotion branches |
| Sandbox                                      | ~17          | Linux       | yes    | **no**      |
| CLI subprocess                               | 30           | any         | no     | **no**      |
| krun conformance                             | ~29          | macOS/Linux | no     | yes         |
| Protocol evolution                           | 11           | any         | no     | yes         |

---

## CI Workflows

CI responsibilities are listed by workflow name rather than a hard-coded count:

| Workflow              | Trigger                                          | Key jobs                                                                                                                                                   |
| --------------------- | ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ci.yml`, `merge.yml` | Main CI, PR, and push/merge-group gates; clippy, unit, e2e, docs, audit, deny, machete, actionlint, and the `promote-staging`/`promote-release` promotions |
| `macos.yml` | macOS formatting |
| `conformance.yml` | conformance, property, krun, CLI, borrow, quickcheck |
| `stability-gates.yml` | doc-sync, adapter/no-unwrap/compile, stale names, construction-site count |
| `protocol-drift.yml` | protocol hash and variant drift |
| `nightly.yml` | scheduled audits, handler `coverage-check`, all-features, shuttle, turmoil, archive tests, bench baselines |
| `release.yml`, `publish-mbx.yml` | release and CLI publishing |
| `pages.yml` | publishes `site/` to GitHub Pages |
| `summary.yml` | issue summary automation |

`pr.yml`, `rust-clippy.yml`, `protocol-sites.yml`, `promote.yml`, and
`context-snapshot.yml` no longer exist; their responsibilities now live in the
workflows listed above. Handler function coverage is a `nightly.yml` job
(`coverage-check`, `continue-on-error`), not a per-push gate.

---

## CI Coverage Gaps

### Not tested in any CI workflow

| Test category    | Command                             | Tests missed                     |
| ---------------- | ----------------------------------- | -------------------------------- |
| Sandbox tests    | `cargo xtask test sandbox`          | ~17 sandbox tests                |
| CLI subprocess   | `just test-cli-subprocess`          | 30 CLI e2e tests                 |

---

## xtask Commands

| Command                 | What                                                     |
| ----------------------- | -------------------------------------------------------- |
| `verify`                | read-only fmt/check/clippy + borrow fixtures + docs-lint |
| `borrow-fixtures`       | standalone Rust borrow must-pass/must-fail fixtures      |
| `pre-commit`            | conditional fmt/restage + clippy/architecture + docs/date checks |
| `prepush`               | musl-check + release build/library tests + conformance   |
| `test unit`             | workspace library tests                                  |
| `test conformance`      | commit/build/push/report conformance suite               |
| `test krun-conformance` | krun-specific conformance                                |
| `test property`         | proptest suites                                          |
| `test integration`      | cgroup tests (Linux+root)                                |
| `test system-suite`     | daemon+CLI e2e (Linux+root)                              |
| `test sandbox`          | sandbox tests (Linux+root)                               |
| `coverage-check`        | handler module function coverage >= 80% gate             |
| `architecture`          | dependency-ring and canonical-owner guard                |
| `musl-check`            | release cross-build for Linux musl targets               |
| `bench`                 | criterion benchmarks in crates/minibox-bench (8 targets) |
| `check stale-names`     | audit workspace for banned old crate/binary names        |
| `nuke-test-state`       | kill orphans, unmount overlays, clean cgroups            |

---

## Test Helpers

**`minibox::testing`** (behind `test-utils` feature):

- `mocks/` — MockRegistry, MockFilesystem, MockLimiter, MockRuntime,
  MockNetwork, MockExecRuntime, MockImagePusher, MockContainerCommitter,
  MockImageBuilder
- `fixtures/` — ContainerFixture, ImageFixture, BuildContextFixture,
  PushTargetFixture, UpperDirFixture
- `helpers/` — `create_test_deps_with_dir`, GC helpers, daemon helpers
- `backend/` — BackendCapability, BackendDescriptor (conformance)

**`minibox-core::adapters`** (behind `test-utils`):

- `mocks.rs` — canonical cross-platform mock adapters; `minibox::testing::mocks` re-exports them
- `test_fixtures.rs` — shared fixtures
- `conformance.rs` — conformance harness
