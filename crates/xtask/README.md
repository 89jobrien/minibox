# xtask

Cargo xtask dev automation. Provides quality gates, CI tasks, benchmark orchestration, VM image builds, and test infrastructure as a compiled Rust binary (not shell scripts).

## Commands

### Quality Gates

```bash
cargo xtask pre-commit        # fmt-check + clippy + release build (macOS-safe)
cargo xtask prepush           # nextest + llvm-cov coverage report (Linux)
cargo xtask test-unit         # all unit + conformance tests
cargo xtask test-property     # proptest property-based tests
cargo xtask test-e2e-suite    # daemon+CLI e2e tests (Linux, root)
```

### Benchmarks

```bash
cargo xtask bench             # run benchmarks locally, save to bench/results/
cargo xtask bench-vps         # run on VPS, fetch results
cargo xtask bench-vps --commit          # ... and commit results
cargo xtask bench-vps --commit --push   # ... and push to remote
```

### VM Image (macOS/vz adapter)

```bash
cargo xtask build-vm-image          # download Alpine + cross-compile agent (cached)
cargo xtask build-vm-image --force  # force re-download and recompile
```

### Test Infrastructure

```bash
cargo xtask build-test-image      # build OCI test image for Linux dogfooding
cargo xtask test-linux            # run test suite inside minibox container
```

### Cleanup

```bash
cargo xtask nuke-test-state       # kill orphans, unmount overlays, clean cgroups/tmp
cargo xtask clean-artifacts       # remove non-critical build outputs
```

## Modules

| Module          | Responsibility                                                |
| --------------- | ------------------------------------------------------------- |
| `gates.rs`      | Quality gate implementations (fmt, clippy, nextest, coverage) |
| `bench.rs`      | Benchmark run, result persistence, VPS orchestration          |
| `vm_image.rs`   | Alpine download, agent cross-compile, VM image assembly       |
| `test_image.rs` | OCI test image build and Linux dogfood test runner            |
| `flamegraph.rs` | samply/flamegraph profiling integration                       |
| `cleanup.rs`    | Test state cleanup (cgroups, overlays, orphan processes)      |
