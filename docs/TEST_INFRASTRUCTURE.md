# Test Infrastructure Report

> Generated 2026-04-27 from automated codebase analysis.
> Updated 2026-05-06: date refresh; no structural changes.

## Test Counts

| Location                                  | Tests (est.) |
| ----------------------------------------- | ------------ |
| Integration test files (crates/\*/tests/) | ~739         |
| Inline (#[cfg(test)] in src/)             | ~728         |
| **Grand total**                           | **~1,467**   |

Note: `cargo nextest` on macOS reports ~760 — that's the cross-platform
subset (lib tests + included integration files). Linux-only, feature-gated,
and root-required tests add ~700 more.

---

## Tests by Crate

| Crate          | Integration files | Integration tests | Inline tests |
| -------------- | ----------------- | ----------------- | ------------ |
| minibox        | 35                | ~479              | ~255         |
| minibox-core   | 7                 | ~126              | ~285         |
| miniboxd       | 6                 | ~72               | ~24          |
| mbx            | 2                 | ~32               | ~96          |
| macbox         | 3                 | ~30               | ~63          |
| winbox         | 0                 | 0                 | ~5           |
| minibox-macros | 0                 | 0                 | 0            |
| xtask          | 0                 | 0                 | 0            |

---

## Test Categories

| Category                                     | Tests (est.) | Platform    | Root?  | In CI?      |
| -------------------------------------------- | ------------ | ----------- | ------ | ----------- |
| Unit (inline lib)                            | ~728         | any         | no     | yes         |
| Handler + daemon conformance                 | ~209         | any         | no     | partial     |
| minibox-core conformance                     | 126          | any         | no     | yes         |
| Adapter isolation (colima/gke/native/smolvm) | ~66          | varies      | varies | partial     |
| Property tests (proptest)                    | ~46          | any         | no     | **no**      |
| Security regression                          | ~19          | any         | no     | yes         |
| Cgroup integration                           | 16           | Linux       | yes    | next/stable |
| E2E daemon+CLI                               | 15           | Linux       | yes    | next/stable |
| Sandbox                                      | ~17          | Linux       | yes    | **no**      |
| CLI subprocess                               | 30           | any         | no     | **no**      |
| krun conformance                             | ~29          | macOS/Linux | no     | **no**      |
| VZ smoke                                     | ~1           | macOS       | no     | **no**      |
| Protocol evolution                           | 11           | any         | no     | yes         |

---

## CI Workflows

8 workflows in `.github/workflows/`:

| Workflow                | Trigger                                          | Key jobs                                                                                             |
| ----------------------- | ------------------------------------------------ | ---------------------------------------------------------------------------------------------------- |
| `ci.yml` (Quality)      | all pushes + PRs                                 | lint, build-test-archive, test-unit, test-archive, audit/deny/machete (next/stable), geiger (stable) |
| `stability-gates.yml`   | all pushes + PRs                                 | doc-sync, adapter-integration-tests, no-unwrap-in-prod, stability-compile                            |
| `conformance.yml`       | next/stable + dispatch                           | `cargo xtask test-conformance` on self-hosted Linux                                                  |
| `bench-regression.yml`  | next/stable + dispatch                           | `cargo xtask bench` + 10% regression check                                                           |
| `protocol-drift.yml`    | pushes touching protocol.rs/handler.rs/server.rs | variant count + handler coverage check                                                               |
| `nightly.yml`           | daily cron                                       | `cargo geiger` unsafe audit (informational)                                                          |
| `release.yml`           | `v*` tag                                         | crates.io publish + musl cross-compile + GitHub release                                              |
| `phased-deployment.yml` | dispatch only                                    | **disabled** (prints message)                                                                        |

---

## CI Coverage Gaps

### Not tested in any CI workflow

| Test category      | Command                             | Tests missed                     |
| ------------------ | ----------------------------------- | -------------------------------- |
| Property tests     | `cargo xtask test-property`         | ~46 proptest tests               |
| Sandbox tests      | `cargo xtask test-sandbox`          | ~17 sandbox tests                |
| CLI subprocess     | `just test-cli-subprocess`          | 30 CLI e2e tests                 |
| krun conformance   | `cargo xtask test-krun-conformance` | ~29 tests                        |
| VZ smoke/isolation | `just test-vz-isolation`            | ~1 test                          |
| Coverage gate      | `cargo xtask coverage-check`        | handler.rs fn coverage threshold |

### Scope mismatches

- CI `test-unit` job runs `nextest --workspace --lib` (lib tests only).
  `cargo xtask test-unit` also includes daemon_conformance_tests,
  colima_conformance, gke_isolation, lifecycle_failure — these only run
  in `test-archive` on Ubuntu, a different job and environment.

- `test-all-features` CI job excludes `macbox` and `miniboxd`. macbox
  inline tests (63 tests) only run on the self-hosted mac runner.

---

## xtask Commands

| Command                 | What                                                   |
| ----------------------- | ------------------------------------------------------ |
| `pre-commit`            | fmt-check + clippy + release build + docs-lint         |
| `prepush`               | nextest + llvm-cov + ai-review (non-fatal)             |
| `test-unit`             | lib + select integration tests + conformance           |
| `test-conformance`      | commit/build/push/report conformance suite             |
| `test-krun-conformance` | krun-specific conformance                              |
| `test-property`         | proptest suites                                        |
| `test-integration`      | cgroup tests (Linux+root)                              |
| `test-e2e-suite`        | daemon+CLI e2e (Linux+root)                            |
| `test-sandbox`          | sandbox tests (Linux+root)                             |
| `coverage-check`        | handler.rs fn coverage >= 80% gate                     |
| `bench`                 | criterion benchmarks (trait_overhead + protocol_codec) |
| `bench-vps`             | benchmarks on VPS (optional --commit --push)           |
| `build-vm-image`        | Alpine kernel + minibox agent for VZ                   |
| `nuke-test-state`       | kill orphans, unmount overlays, clean cgroups          |

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

- `mocks.rs` — cross-platform mock adapters (duplicates minibox mocks)
- `test_fixtures.rs` — shared fixtures
- `conformance.rs` — conformance harness
