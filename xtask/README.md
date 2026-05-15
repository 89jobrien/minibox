# xtask

Cargo xtask dev automation. Provides quality gates, CI tasks, benchmark orchestration, VM image
builds, and test infrastructure as a compiled Rust binary (not shell scripts).

## Commands

### Quality Gates

```bash
cargo xtask pre-commit        # fmt-check + lint + release build (macOS-safe)
cargo xtask prepush           # fast lib tests (debug, incremental)
cargo xtask test-unit         # all unit + conformance tests
cargo xtask test-conformance  # commit+build+push conformance suite + artifact reports
cargo xtask test-krun-conformance  # krun adapter conformance (HVF/KVM, sets MINIBOX_KRUN_TESTS=1)
cargo xtask test-property     # property-based tests (proptest, any platform)
cargo xtask test-integration  # cgroup + integration tests (Linux, root)
cargo xtask test-e2e          # protocol e2e tests (any platform, no root required)
cargo xtask test-system-suite # full-stack system tests (Linux, root, cgroups v2)
cargo xtask test-e2e-suite    # alias for test-system-suite (backward compat)
cargo xtask test-sandbox      # sandbox contract tests (Linux, root, Docker Hub)
cargo xtask coverage-check    # llvm-cov minibox; fail if handler.rs fns < 80%
cargo xtask check-repo-clean  # warn if generated artifacts (target/, traces/, *.profraw) are tracked
```

### Benchmarks

```bash
cargo xtask bench             # run criterion benchmarks, save to bench/results/
```

### VM Image (macOS/vz adapter)

```bash
cargo xtask build-vm-image          # download Alpine kernel/rootfs, cross-compile agent (cached)
cargo xtask build-vm-image --force  # force re-download and recompile
cargo xtask run-vm                  # boot VM with interactive shell (QEMU HVF, Ctrl-A X to exit)
cargo xtask test-vm                 # build musl test binaries + run in VM, stream results
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
cargo xtask lint-docs                 # validate frontmatter + status values in docs/superpowers/
cargo xtask context [--save]          # dump machine-readable repo context snapshot (JSON)
cargo xtask check-stale-names         # audit workspace for banned old crate/binary names
cargo xtask check-protocol-drift [--update] [--warn-only] [--hook]
                                      # verify core contract hashes
cargo xtask check-protocol-sites [<file>] [--expected N] [--warn-only]
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
| `bench.rs`          | Benchmark run and result persistence                          |
| `vm_image.rs`       | Alpine download, agent cross-compile, VM image assembly       |
| `vm_run.rs`         | VM boot: interactive shell or test execution under QEMU       |
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
