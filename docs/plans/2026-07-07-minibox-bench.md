# Plan: minibox-bench — dedicated benchmark crate

## Goal

Implement the approved design at `docs/designs/2026-07-07-minibox-bench-design.md`: a new
`crates/minibox-bench` leaf crate owning all criterion targets and fixtures, real hot-path
benches, baseline regression checking in `cargo xtask bench`, and a nightly CI job that
prefers the self-hosted runner and falls back to GitHub-hosted.

## Architecture

- Crates affected: `minibox-bench` (new), `minibox-core` (one visibility change), `minibox`
  (bench removal), `xtask` (regression tooling). Non-crate: `Justfile`, `.gitignore`,
  `.github/workflows/nightly.yml`, docs.
- New types: `LayerSpec`, `BenchRegistry` (minibox-bench); `BenchOpts`, `BaselineDelta` (xtask).
- Data flow: bench targets → criterion → `target/criterion/` → `xtask bench` parses into
  `RunRecord` → `bench/results/` + optional compare vs tracked `bench/baseline.<env>.json`.

## Tech Stack

Rust 2024, criterion 0.7 (`async_tokio`), wiremock/tar/flate2/sha2/tempfile/tokio — all
already `[workspace.dependencies]`. No new external dependencies.

## Execution constraints

- All work on `develop`. Run `git branch --show-current` before any commit; stop if not
  `develop`.
- The repo pre-commit hook runs `git add -u`, sweeping every modified tracked file into any
  commit. Subagents therefore DO NOT commit; the parent session makes one umbrella commit per
  wave. Per-task "Commit" lines below name the logical unit for the umbrella message only.
- Trust code over this plan's line numbers. Where a signature is sketched, confirm against the
  file first.
- Verification uses package-scoped commands; the canonical clippy gate is lib-target
  (`cargo clippy -p <crate> -- -D warnings`). `--all-targets` has known pre-existing test-lint
  noise — do not chase it.

## Wave A — crate, fixtures, portable benches

### Task 1: Scaffold crates/minibox-bench

**Crate**: `minibox-bench` (new)
**File(s)**: `Cargo.toml` (workspace members), `crates/minibox-bench/Cargo.toml`,
`crates/minibox-bench/src/lib.rs`, `.gitignore`
**Run**: `cargo check -p minibox-bench`

1. Root `Cargo.toml` members: add `"crates/minibox-bench",` after `"crates/ail",`.
2. `crates/minibox-bench/Cargo.toml`:

   ```toml
   [package]
   name = "minibox-bench"
   version.workspace = true
   edition.workspace = true
   license.workspace = true
   rust-version.workspace = true
   repository.workspace = true
   publish = false

   [dependencies]
   anyhow.workspace = true
   flate2.workspace = true
   minibox = { workspace = true, features = ["test-utils"] }
   minibox-core = { workspace = true, features = ["test-utils"] }
   sha2.workspace = true
   tar.workspace = true
   tempfile.workspace = true
   tokio.workspace = true
   wiremock.workspace = true

   [dev-dependencies]
   criterion.workspace = true
   minibox-macros.workspace = true
   ```

   Bench targets are declared in later tasks as each bench file lands (criterion needs
   `harness = false` per target).

3. `src/lib.rs`:

   ```rust
   //! Benchmark fixtures and harness support for the minibox workspace.
   //!
   //! This crate is a leaf: nothing depends on it, and it is the only place
   //! where `test-utils` features of the lib crates are enabled.

   pub mod fixtures;

   /// Runtime guard for root-required Linux benches.
   #[must_use]
   pub fn is_root() -> bool {
       #[cfg(unix)]
       // SAFETY: geteuid has no preconditions and cannot fail.
       unsafe {
           libc::geteuid() == 0
       }
       #[cfg(not(unix))]
       false
   }
   ```

   If `libc` is not a workspace dep path you want here, use
   `nix::unistd::geteuid().is_root()` (nix is a workspace dep) — prefer nix to avoid a direct
   libc dependency; adjust Cargo.toml accordingly.

4. `.gitignore`: add a `bench/results/` line (verify `grep -n '^bench' .gitignore` first).
5. Verify: `cargo check -p minibox-bench` clean; `cargo clippy -p minibox-bench -- -D warnings`
   clean.
6. Umbrella unit: `chore(minibox-bench): scaffold dedicated benchmark crate`.

### Task 2: Promote RegistryClient::for_test behind test-utils

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/image/registry.rs` (fn at ~line 395)
**Run**: `cargo nextest run -p minibox-core`

1. Change:

   ```rust
   pub(crate) fn for_test(auth_url: &str, registry_base: &str) -> anyhow::Result<Self> {
   ```

   to:

   ```rust
   #[cfg(any(test, feature = "test-utils"))]
   pub fn for_test(auth_url: &str, registry_base: &str) -> anyhow::Result<Self> {
   ```

   Add a doc comment: `/// Test/bench-only constructor pointing at a mock registry.`
   The `platform` field is set directly by the in-crate test harness
   (`client.platform = ...`); external callers cannot do that, so also add (same cfg gate):

   ```rust
   #[cfg(any(test, feature = "test-utils"))]
   #[must_use]
   pub fn with_pinned_platform(
       mut self,
       platform: crate::image::manifest::TargetPlatform,
   ) -> Self {
       self.platform = platform;
       self
   }
   ```

2. Verify: `cargo nextest run -p minibox-core` green;
   `cargo clippy -p minibox-core -- -D warnings` clean;
   `cargo xtask check protocol-drift` — if the surface hash moved, re-run with `--update` and
   include the lock in the wave commit.
3. Umbrella unit: `feat(minibox-core): expose for_test registry constructor to bench crate`.

### Task 3: Layer fixtures

**Crate**: `minibox-bench`
**File(s)**: `crates/minibox-bench/src/fixtures/mod.rs`, `crates/minibox-bench/src/fixtures/layer.rs`
**Run**: `cargo nextest run -p minibox-bench`

1. `fixtures/mod.rs`:

   ```rust
   pub mod layer;
   pub mod registry;

   pub use layer::{LayerSpec, build_layer_tar_gz, sha256_digest};
   pub use registry::BenchRegistry;
   ```

   (`registry` module lands in Task 4; stub it with an empty file if implementing 3 first.)

2. `fixtures/layer.rs` — deterministic content, no `Date::now`/randomness:

   ```rust
   use flate2::{Compression, write::GzEncoder};
   use sha2::{Digest, Sha256};
   use std::io::Write;

   /// Shape of a synthetic OCI layer for extraction/pull benches.
   #[derive(Debug, Clone, Copy)]
   pub struct LayerSpec {
       pub file_count: usize,
       pub file_size_bytes: usize,
       pub dir_depth: usize,
   }

   /// Deterministic gzipped tar built from the spec.
   #[must_use]
   pub fn build_layer_tar_gz(spec: &LayerSpec) -> Vec<u8> {
       let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
       let payload = vec![0xA5u8; spec.file_size_bytes];
       for i in 0..spec.file_count {
           let dir: String = (0..spec.dir_depth).map(|d| format!("d{d}/")).collect();
           let path = format!("{dir}file-{i:06}.bin");
           let mut header = tar::Header::new_gnu();
           header.set_size(spec.file_size_bytes as u64);
           header.set_mode(0o644);
           header.set_cksum();
           builder
               .append_data(&mut header, path, payload.as_slice())
               .expect("append tar entry");
       }
       builder
           .into_inner()
           .expect("finish tar")
           .finish()
           .expect("finish gzip")
   }

   #[must_use]
   pub fn sha256_digest(bytes: &[u8]) -> String {
       format!("sha256:{:x}", Sha256::digest(bytes))
   }
   ```

   Note the workspace denies `expect_used` — fixtures are bench-support code where panicking
   on broken fixtures is correct; add `#![allow(clippy::expect_used)]` at the top of
   `fixtures/layer.rs` (and `registry.rs`) with a one-line comment, mirroring
   `crates/minibox/benches/trait_overhead.rs`.

3. Tests (inline `#[cfg(test)]`): build a small spec (3 files, 1 KiB, depth 2), assert the
   output gunzips and `tar::Archive::entries` count == 3; assert `extract_layer` from
   minibox-core extracts it into a `TempDir` successfully (round-trip against the real
   consumer).
4. Verify: `cargo nextest run -p minibox-bench` green; clippy clean.
5. Umbrella unit: `feat(minibox-bench): deterministic layer fixtures`.

### Task 4: BenchRegistry wiremock harness

**Crate**: `minibox-bench`
**File(s)**: `crates/minibox-bench/src/fixtures/registry.rs`
**Run**: `cargo nextest run -p minibox-bench`

1. Mirror the in-crate harness at `crates/minibox-core/src/image/registry.rs` tests
   (`test_client` at ~line 1207 and the pull-test mock setups near it — read them first;
   they define the exact token/manifest/blob endpoint shapes `RegistryClient` expects).

   ```rust
   use anyhow::Result;
   use minibox_core::image::RegistryClient;
   use wiremock::MockServer;

   /// Wiremock-backed OCI registry serving one image.
   pub struct BenchRegistry {
       server: MockServer,
   }

   impl BenchRegistry {
       /// Serve `image:tag` composed of the given gzipped tar layers.
       pub async fn serve(image: &str, tag: &str, layers: Vec<Vec<u8>>) -> Result<Self>;

       /// RegistryClient pointed at the mock server, pinned to linux/amd64.
       pub fn client(&self) -> Result<RegistryClient>;
   }
   ```

   `serve` registers: token endpoint, `/v2/<image>/manifests/<tag>` returning an
   image-manifest (or index) JSON with the layers' digests/sizes, and one
   `/v2/<image>/blobs/<digest>` mock per layer — copy the JSON bodies from the existing pull
   tests, parameterized by digest/size. `client` uses
   `RegistryClient::for_test(&format!("{}/token", uri), &format!("{}/v2", uri))?
   .with_pinned_platform(TargetPlatform::linux_amd64())` (Task 2 API).

2. Test: `#[tokio::test]` — `BenchRegistry::serve("bench/img", "latest", vec![layer])` with a
   Task 3 fixture layer, then `client().pull_image("bench/img", "latest", &store)` into an
   `ImageStore::new(tempdir)` succeeds and the extracted layer dir is non-empty.
3. Verify: `cargo nextest run -p minibox-bench` green; clippy clean.
4. Umbrella unit: `feat(minibox-bench): wiremock BenchRegistry fixture`.

### Task 5: Move protocol_codec bench

**Crate**: `minibox-bench`, `minibox`
**File(s)**: `crates/minibox-bench/benches/protocol_codec.rs` (moved from
`crates/minibox/benches/protocol_codec.rs`), both Cargo.tomls
**Run**: `cargo bench -p minibox-bench --bench protocol_codec -- --test`

1. `git mv crates/minibox/benches/protocol_codec.rs crates/minibox-bench/benches/protocol_codec.rs`
   — imports already target `minibox_core::protocol` and `minibox_macros`; no code changes
   expected. Add to minibox-bench Cargo.toml:

   ```toml
   [[bench]]
   name = "protocol_codec"
   harness = false
   ```

2. Remove the `[[bench]] name = "protocol_codec"` section from `crates/minibox/Cargo.toml`.
3. Verify: `cargo bench -p minibox-bench --bench protocol_codec -- --test` (criterion test
   mode, fast) exits 0.
4. Umbrella unit: `refactor(minibox-bench): move protocol_codec bench`.

### Task 6: Port dispatch/state benches; prune trivia

**Crate**: `minibox-bench`, `minibox`
**File(s)**: `crates/minibox-bench/benches/daemon_dispatch.rs`,
`crates/minibox-bench/benches/trait_dispatch.rs` (both derived from
`crates/minibox/benches/trait_overhead.rs`), both Cargo.tomls
**Run**: `cargo bench -p minibox-bench --bench daemon_dispatch -- --test`

1. Split `trait_overhead.rs` into two bench files in minibox-bench:
   - `daemon_dispatch.rs`: `state_reconcile` group (n = 10/100/500), `handler_pipeline_list`,
     `handler_pipeline_run_mock_image_miss`, `handler_pipeline_pause_not_found` — moved as-is
     (imports change from crate-relative to `minibox::...`; they already use
     `minibox::daemon::handler::RunParams` and `minibox::testing` helpers, available via the
     dep's `test-utils` feature).
   - `trait_dispatch.rs`: the four direct-vs-trait-object pairs (registry, filesystem,
     limiter, runtime). DELETE `arc_clone` and `downcast_to_concrete` entirely.
   Update `criterion_group!`/`criterion_main!` registrations to match each file's contents.
2. Delete `crates/minibox/benches/trait_overhead.rs`, its `[[bench]]` section (including
   `required-features`), and the criterion entry in `[dev-dependencies]` of
   `crates/minibox/Cargo.toml`. `crates/minibox/benches/` must now be gone entirely.
3. Add both `[[bench]]` sections (`harness = false`) to minibox-bench Cargo.toml.
4. Verify:

   ```
   cargo bench -p minibox-bench --bench daemon_dispatch -- --test   → exit 0
   cargo bench -p minibox-bench --bench trait_dispatch -- --test    → exit 0
   cargo check -p minibox --all-targets                             → clean (no bench refs)
   grep -rn 'trait_overhead' --include='*.toml' --include='*.rs' .  → no hits outside docs
   ```

5. Umbrella unit: `refactor(minibox-bench): port dispatch benches, prune arc/downcast trivia`.

### Task 7: layer_extract bench

**Crate**: `minibox-bench`
**File(s)**: `crates/minibox-bench/benches/layer_extract.rs`, minibox-bench Cargo.toml
**Run**: `cargo bench -p minibox-bench --bench layer_extract -- --test`

1. Bench `minibox_core::image::layer::extract_layer(reader, dest)` (confirm the `pub use`
   path via `grep -rn 'pub use.*extract_layer\|pub fn extract_layer' crates/minibox-core/src/`)
   over a spec grid, pre-building each layer once outside the timing loop and extracting into
   a fresh `TempDir` per iteration (`iter_batched` with `BatchSize::PerIteration`):

   | scenario                | spec                                              |
   | ----------------------- | ------------------------------------------------- |
   | `extract_small_many`    | 1024 files x 4 KiB, depth 4 (~4 MiB)              |
   | `extract_large_few`     | 4 files x 8 MiB, depth 1 (~32 MiB)                |
   | `extract_deep_tree`     | 512 files x 16 KiB, depth 16 (~8 MiB)             |

   Group name: `layer_extract`. `[[bench]] name = "layer_extract"`, `harness = false`.
2. Verify: bench test-mode run exits 0; clippy clean.
3. Umbrella unit: `feat(minibox-bench): layer extraction hot-path bench`.

### Task 8: image_pull bench

**Crate**: `minibox-bench`
**File(s)**: `crates/minibox-bench/benches/image_pull.rs`, minibox-bench Cargo.toml
**Run**: `cargo bench -p minibox-bench --bench image_pull -- --test`

1. Async criterion bench (`criterion::async_executor::AsyncExecutor` with a tokio runtime,
   matching the `async_tokio` pattern already used in `trait_overhead.rs` handler benches):
   start one `BenchRegistry` outside the loop; per iteration pull into a fresh
   `ImageStore::new(TempDir)`. Scenarios: `pull_1_layer_4mib`, `pull_4_layers_4mib`,
   `pull_1_layer_32mib`. Group: `image_pull`. `[[bench]]` section added, `harness = false`.
2. Verify: bench test-mode run exits 0; clippy clean.
3. Umbrella unit: `feat(minibox-bench): end-to-end image pull bench`.

**Wave A gate** (parent session): `cargo check --workspace --all-targets`,
`cargo nextest run -p minibox-bench -p minibox-core`, `cargo xtask verify`, then umbrella
commit `feat(minibox-bench): dedicated bench crate with hot-path coverage (wave A)`.

## Wave B — Linux-gated benches

All three files start with `#![cfg(target_os = "linux")]` and skip at runtime when
`!minibox_bench::is_root()` (register an empty criterion group so `--test` mode still
passes on macOS/non-root). macOS validation gate for every task:
`cargo check -p minibox-bench --all-targets --target aarch64-unknown-linux-gnu`.

### Task 9: linux_rootfs bench

**File(s)**: `crates/minibox-bench/benches/linux_rootfs.rs`, minibox-bench Cargo.toml
**Run**: `cargo check -p minibox-bench --all-targets --target aarch64-unknown-linux-gnu`

`OverlayFilesystem::new_with_base(tempdir)` (adapters/filesystem.rs:69): per iteration,
`setup_rootfs(&layers, &container_dir)` then `cleanup(&container_dir)`, with 1/4/16
pre-extracted fixture layers. minibox-bench gains a target-gated dep:

```toml
[target.'cfg(target_os = "linux")'.dependencies]
# (only if the bench needs types not re-exported through `minibox`)
```

— check first: `OverlayFilesystem` is exported via `minibox::adapters` (confirm with
`grep -rn 'pub use.*OverlayFilesystem\|pub mod adapters' crates/minibox/src/lib.rs`); if so,
no new dep is needed. Umbrella unit: `feat(minibox-bench): overlay rootfs bench (linux)`.

### Task 10: linux_cgroup bench

**File(s)**: `crates/minibox-bench/benches/linux_cgroup.rs`, minibox-bench Cargo.toml
**Run**: `cargo check -p minibox-bench --all-targets --target aarch64-unknown-linux-gnu`

`CgroupV2Limiter` (adapters/limiter.rs:64): per iteration `create(&id, &config)` →
`cleanup(&id)` with `ResourceConfig { memory_limit_bytes: Some(64 << 20), cpu_weight:
Some(100), pids_max: Some(64), io_max_bytes_per_sec: None }`; unique id per iteration via a
counter (no randomness). `add_process` is excluded (needs a live child; covered by Task 11).
Umbrella unit: `feat(minibox-bench): cgroup v2 limiter bench (linux)`.

### Task 11: linux_spawn bench

**File(s)**: `crates/minibox-bench/benches/linux_spawn.rs`, minibox-bench Cargo.toml
**Run**: `cargo check -p minibox-bench --all-targets --target aarch64-unknown-linux-gnu`

`LinuxNamespaceRuntime` (adapters/runtime.rs:84): per iteration `spawn_process(&config)` then
`wait_for_exit(...)` for `/bin/true` on a pre-extracted busybox-like rootfs built from
fixtures (one static binary copied in). `ContainerSpawnConfig` (domain.rs:1169) fields:
`rootfs`/`cgroup_path` are `InternalPath`, `command: "/bin/true"`, empty `args`/`env`/`mounts`,
`hostname: "bench"`, `capture_output: false`, `hooks: ContainerHooks::default()` (confirm
Default exists; otherwise construct empty), `skip_network_namespace: true` (cheaper, network
cost is not the subject), `privileged: false`, `image_ref: None`. This is async
(`spawn_process` is `async fn`) — same async-executor pattern as Task 8. Confirm exact
construction against the Linux e2e tests (`grep -rn 'ContainerSpawnConfig {'
crates/minibox/tests/ crates/miniboxd/tests/`). Umbrella unit:
`feat(minibox-bench): container spawn lifecycle bench (linux)`.

**Wave B gate** (parent session): Linux-target check clean; optionally run the suite on
jobrien-vm as root for a smoke pass. Umbrella commit
`feat(minibox-bench): linux hot-path benches (wave B)`.

## Wave C — xtask regression tooling, Justfile, CI

### Task 12: xtask BenchOpts + flag parsing + retarget

**Crate**: `xtask`
**File(s)**: `xtask/src/bench.rs`, `xtask/src/main.rs`
**Run**: `cargo nextest run -p xtask`

1. TDD: add a pure parser + tests in bench.rs mirroring the `dispatch_args_tests` style in
   main.rs (Wave 1 of the moa plan):

   ```rust
   pub struct BenchOpts {
       pub skip_bench: bool,
       pub check: bool,
       pub save_baseline: bool,
       pub threshold_pct: Option<f64>,
       pub env: String,
   }

   pub fn parse_bench_args(rest: &[String]) -> BenchOpts;
   ```

   Tests: default env is `"local"`; `--check --env hosted --threshold 20` parses; unknown
   flags are ignored with a warning (consistent with existing xtask arg style).
2. `pub fn bench(sh: &Shell, root: &Path, opts: &BenchOpts) -> Result<()>` (was
   `(sh, root)`); main.rs dispatch arm (line ~202) becomes
   `Some("bench") => bench::bench(&sh, root, &bench::parse_bench_args(&argv[2..]))`.
3. Line ~85: `cargo bench -p minibox -- --noplot` → `cargo bench -p minibox-bench -- --noplot`.
4. Remove `memory_peak_bytes` from `BenchMetrics` and every write site (grep the file); old
   history JSON still parses (serde ignores unknown fields by default — confirm no
   `deny_unknown_fields` on these structs).
5. Verify: `cargo nextest run -p xtask` green; `cargo run -p xtask -- bench --skip-bench`
   exits 0 against existing criterion output (or cleanly reports none).
6. Umbrella unit: `feat(xtask): BenchOpts parsing; retarget bench to minibox-bench`.

### Task 13: xtask baseline comparison

**Crate**: `xtask`
**File(s)**: `xtask/src/bench.rs`
**Run**: `cargo nextest run -p xtask -- baseline`

1. TDD with pure functions over `RunRecord` values (no filesystem in unit tests):

   ```rust
   #[derive(Debug)]
   struct BaselineDelta {
       scenario: String,
       group: String,
       baseline_mean_ns: f64,
       current_mean_ns: f64,
       delta_pct: f64,
       regressed: bool,
   }

   fn compare_to_baseline(
       latest: &RunRecord,
       baseline: &RunRecord,
       threshold_pct: f64,
   ) -> Vec<BaselineDelta>;
   ```

   Rules (tests for each): per-scenario effective threshold
   `t = threshold_pct.max(2.0 * (baseline.std_dev_ns / baseline.mean_ns) * 100.0)`;
   `regressed = current_mean_ns > baseline_mean_ns * (1.0 + t / 100.0)`; scenario in baseline
   but missing from latest → synthetic `regressed = true` delta (inventory collapse); scenario
   only in latest → not regressed (informational).
2. Wire into `bench()`: after writing results, when `opts.check`, load
   `bench/baseline.{env}.json`; if absent, print bootstrap notice and pass; else print a delta
   table and `bail!` listing regressed scenarios if any. When `opts.save_baseline`, copy the
   just-written `latest.json` to `bench/baseline.{env}.json`.
3. Verify: `cargo nextest run -p xtask` green; clippy clean.
4. Umbrella unit: `feat(xtask): per-env baseline regression checking for bench`.

### Task 14: Justfile targets

**File(s)**: `Justfile` (lines ~126-142)
**Run**: `just --list`

1. Delete the `bench-sync` and `flamegraph` targets (their xtask commands never existed).
2. Add:

   ```make
   bench-check:
       cargo xtask bench --check

   bench-baseline:
       cargo xtask bench --save-baseline
   ```

3. Verify: `just --list` shows bench/bench-check/bench-baseline and no dangling targets;
   `grep -n 'bench-sync\|flamegraph' Justfile` → no hits.
4. Umbrella unit: `chore(just): bench-check/bench-baseline; drop dangling targets`.

### Task 15: nightly workflow bench pipeline

**File(s)**: `.github/workflows/nightly.yml`
**Run**: `actionlint .github/workflows/nightly.yml`

Use Bash heredoc-style edits if the Edit tool is blocked for workflow files. Append three
jobs modeled on the existing nightly jobs' checkout/toolchain steps:

```yaml
bench-runner-check:
  runs-on: ubuntu-latest
  outputs:
    selfhosted: ${{ steps.probe.outputs.selfhosted }}
  steps:
    - id: probe
      env:
        RUNNER_TOKEN: ${{ secrets.ACTIONS_RUNNER_READ_TOKEN }}
      run: |
        ok=false
        if [ -n "$RUNNER_TOKEN" ]; then
          online=$(curl -sf -H "Authorization: Bearer $RUNNER_TOKEN" \
            "https://api.github.com/repos/${{ github.repository }}/actions/runners" \
            | jq '[.runners[] | select(.status == "online")
                   | select(.labels[].name == "minibox")] | length' || echo 0)
          [ "${online:-0}" -gt 0 ] && ok=true
        fi
        echo "selfhosted=$ok" >> "$GITHUB_OUTPUT"

bench-selfhosted:
  needs: bench-runner-check
  if: needs.bench-runner-check.outputs.selfhosted == 'true'
  runs-on: [self-hosted, minibox]
  steps:
    - uses: actions/checkout@v4
    - run: ~/.local/bin/mise exec -- cargo xtask bench --check --env selfhosted
    - uses: actions/upload-artifact@v4
      if: always()
      with: { name: bench-results-selfhosted, path: bench/results/ }

bench-hosted:
  needs: bench-runner-check
  if: needs.bench-runner-check.outputs.selfhosted != 'true'
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable   # match the toolchain step used by existing jobs
    - run: cargo xtask bench --check --env hosted
    - uses: actions/upload-artifact@v4
      if: always()
      with: { name: bench-results-hosted, path: bench/results/ }
```

Match the concrete checkout/toolchain/cache steps to whatever the existing nightly jobs use
(read the file first) rather than the sketch above. Verify with actionlint. Document the
`ACTIONS_RUNNER_READ_TOKEN` secret requirement in the workflow file as a comment. Umbrella
unit: `ci(nightly): bench job with self-hosted preference and hosted fallback`.

### Task 16: docs sweep

**File(s)**: `DEVELOPMENT.md`, `docs/` files referencing benches, `xtask/README.md` (if any)
**Run**: `grep -rn 'trait_overhead\|bench-sync\|flamegraph' docs/ DEVELOPMENT.md USAGE.md`

Update every stale reference (survey found hits in USAGE.md, DEVELOPMENT.md, xtask README):
bench home is `crates/minibox-bench`, commands are `just bench` / `just bench-check` /
`just bench-baseline`, baselines are per-env. Verify grep returns no stale hits. Umbrella
unit: `docs: point bench docs at minibox-bench`.

**Wave C gate** (parent session): `cargo xtask verify`, `cargo nextest run --workspace`
(known exclusion: smolvm e2e), `actionlint`, one real `cargo xtask bench` run to seed
`bench/results/` locally, then umbrella commit
`feat(bench): regression tooling, Justfile, nightly CI (wave C)`.

## Execution order

- Wave A: Task 1 → 2 → 3 → 4 sequential (each builds on the previous); Tasks 5-8 parallel-safe
  after 4 (disjoint bench files; Cargo.toml `[[bench]]` merges are append-only — if run as
  parallel agents, the parent resolves the Cargo.toml union).
- Wave B: Tasks 9-11 parallel-safe after Wave A commits.
- Wave C: Task 12 → 13 sequential; 14-16 parallel-safe alongside 13.

## Deferred (tracked in the design doc, not here)

Memory profiling, flamegraph integration, PR-time bench smoke, VM-adapter benches, running
root-gated benches in CI.
