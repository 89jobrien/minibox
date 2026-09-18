# xtask

Cargo xtask dev automation. Provides quality gates, CI tasks, benchmark orchestration, VM image
builds, and test infrastructure as a compiled Rust binary (not shell scripts).

## Commands

### Quality Gates

```bash
cargo xtask pre-commit          # staged fmt/clippy plus architecture/config/docs checks
cargo xtask prepush             # musl check, release build, nextest, conformance
cargo xtask test unit           # workspace library tests
cargo xtask test conformance    # adapter and port conformance suite
cargo xtask test krun-conformance
cargo xtask test property       # property-based tests (any platform)
cargo xtask test integration    # cgroup + integration tests (Linux, root)
cargo xtask test e2e            # protocol e2e tests (any platform)
cargo xtask test system-suite   # full-stack tests (Linux, root, cgroups v2)
cargo xtask test sandbox        # sandbox contract tests (Linux, root, Docker Hub)
cargo xtask coverage-check    # llvm-cov minibox; fail if handler/ functions fall below 80%
cargo xtask check repo-clean  # warn if generated artifacts are tracked
```

### Benchmarks

All criterion benches live in `crates/minibox-bench`. Results go to `bench/results/`
(gitignored); tracked per-env baselines live at `bench/baseline.{local,selfhosted,hosted}.json`.

```bash
cargo xtask bench                  # run criterion benchmarks, save to bench/results/
cargo xtask bench --check          # compare against the per-env baseline, fail on regression
cargo xtask bench --save-baseline  # save results as the new per-env baseline
cargo xtask bench --skip-bench     # re-parse existing criterion output without re-running
cargo xtask bench --env <name>     # baseline environment: local (default), selfhosted, hosted
cargo xtask bench --threshold <pct> # regression threshold percentage for --check
```

### VM and Test Images

```bash
cargo xtask build-test-image          # build the cached OCI test image
cargo xtask setup-test-vm             # prepare the smolvm test environment
cargo xtask setup-test-vm --force     # rebuild cached VM assets
cargo xtask test-in-vm                # run tests through the configured VM backend
cargo xtask test-linux                # run the Linux dogfood pipeline
```

### Test Infrastructure

```bash
cargo xtask build-test-image  # cross-compile test binaries + assemble OCI tarball
cargo xtask test-linux        # build image + load into minibox + run tests in container
cargo xtask run-cgroup-tests  # cgroup v2 integration tests in delegated hierarchy (Linux, root)
```

### CAS Overlay Store

```bash
cargo xtask cas-add <file> [--ref <name>]  # add file to CAS overlay (~/.minibox/vm/overlay/cas/)
cargo xtask cas-check                      # verify all overlay refs match their CAS objects
```

### Utilities

```bash
cargo xtask bump [patch|minor|major]  # bump workspace version in Cargo.toml
cargo xtask preflight                 # check required tools are on PATH and functional
cargo xtask doctor                    # full preflight: tools + CARGO_TARGET_DIR + Linux system checks
cargo xtask available                 # verify cargo xtask is runnable (real capability check)
cargo xtask docs lint                 # validate supported plan/spec frontmatter
cargo xtask info context [--save]     # dump machine-readable repo context snapshot (JSON)
cargo xtask check stale-names         # audit workspace for banned old crate/binary names
cargo xtask check protocol-drift [--update] [--warn-only] [--hook]
                                      # verify core contract hashes
cargo xtask check protocol-sites [<file>] [--expected N] [--warn-only]
                                      # verify HandlerDependencies construction site count
```

### Cleanup

```bash
cargo xtask nuke-test-state   # kill orphans, unmount overlays, clean cgroups/tmp
cargo xtask clean-artifacts   # remove non-critical build outputs
```

## Modules

| Module              | Responsibility                                                |
| ------------------- | ------------------------------------------------------------- |
| `gates.rs`          | Quality gate implementations (fmt, clippy, nextest, coverage) |
| `bench.rs`          | Benchmark run, result persistence, per-env baseline checking  |
| `setup_test_vm.rs`  | Prepare the smolvm test environment                           |
| `test_in_vm.rs`     | Execute the configured VM test profiles                       |
| `test_image.rs`     | OCI test image build and Linux dogfood test runner            |
| `test_linux.rs`     | Run test suite inside a minibox container                     |
| `cgroup_tests.rs`   | cgroup v2 integration test runner (delegated hierarchy)       |
| `cas.rs`            | Content-addressed overlay store operations                    |
| `bump.rs`           | Workspace version bumping                                     |
| `preflight.rs`      | Tool availability probing and doctor checks                   |
| `docs_lint.rs`      | Frontmatter and status validation for docs/superpowers/       |
| `protocol_drift.rs` | Core contract hash drift checker                              |
| `protocol_sites.rs` | HandlerDependencies construction site counter                 |
| `stale_names.rs`    | Audit for banned old crate/binary names                       |
| `context.rs`        | Repo context snapshot (JSON)                                  |
| `cleanup.rs`        | Test state cleanup (cgroups, overlays, orphan processes)      |
