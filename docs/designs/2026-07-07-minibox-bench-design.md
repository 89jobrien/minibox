# Design: minibox-bench — dedicated benchmark crate

## Goal

Replace the ad-hoc benches in `crates/minibox/benches/` with a dedicated `crates/minibox-bench`
crate that benchmarks the real hot paths (layer extraction, image pull, overlay setup, container
spawn, cgroup ops) and make results actionable via baseline regression checking in
`cargo xtask bench`, with canonical numbers from a nightly job on the self-hosted Linux runner.

## Approved Approach

"Dedicated bench crate" from brainstorm: one workspace member owns all criterion targets and
shared fixture builders; nightly self-hosted CI produces canonical numbers; full redesign of both
the coverage axis and the regression-infra axis.

## Crate Ownership

- **Owner crate**: `minibox-bench` (new, `crates/minibox-bench`, `publish = false`) — benchmark
  targets and fixture builders have no production consumers and need `test-utils` features that
  must not leak into lib crates.
- **Affected crates**:
  - `minibox-core` — one visibility change (see Integration Points)
  - `minibox` — deletes `benches/` and its two `[[bench]]` sections
  - `xtask` — `bench.rs` gains baseline comparison; dispatch gains flags
- No crate imports `minibox-bench`; it is a leaf.

## Public API

### minibox-bench fixtures (`src/lib.rs`, `src/fixtures/`)

```rust
/// Shape of a synthetic OCI layer for extraction/pull benches.
pub struct LayerSpec {
    pub file_count: usize,
    pub file_size_bytes: usize,
    pub dir_depth: usize,
}

/// Deterministic gzipped tar built from the spec (seeded content, no randomness).
pub fn build_layer_tar_gz(spec: &LayerSpec) -> Vec<u8>;

pub fn sha256_digest(bytes: &[u8]) -> String;

/// Wiremock-backed OCI registry serving one image, mirroring the private
/// harness in minibox-core registry tests.
pub struct BenchRegistry {
    server: wiremock::MockServer,
}

impl BenchRegistry {
    pub async fn serve(image: &str, tag: &str, layers: Vec<Vec<u8>>) -> anyhow::Result<Self>;
    /// RegistryClient pointed at the mock server (linux/amd64 pinned).
    pub fn client(&self) -> anyhow::Result<minibox_core::image::RegistryClient>;
}

/// Runtime guard for root-required Linux benches (mirrors integration tests).
pub fn is_root() -> bool;
```

Dispatch/state benches reuse `minibox::testing::helpers::daemon` builders (`make_mock_deps`,
`make_mock_state_with_n_containers`) via the `test-utils` feature — no duplication.

### Bench targets (`benches/`, all `harness = false`, criterion 0.7)

| Target            | Measures                                                        | Gating              |
| ----------------- | --------------------------------------------------------------- | ------------------- |
| `protocol_codec`  | moved verbatim from `crates/minibox/benches/protocol_codec.rs`  | none                |
| `layer_extract`   | `extract_layer()` over `LayerSpec` grid (10 MiB / 100 MiB; many-small vs few-large files) | none |
| `image_pull`      | `RegistryClient::pull_image()` end-to-end against `BenchRegistry` (async_tokio) | none |
| `daemon_dispatch` | moved: `state_reconcile` (n = 10/100/500), `handler_pipeline_list`, `handler_pipeline_run_mock_image_miss`, `handler_pipeline_pause_not_found` | none |
| `trait_dispatch`  | slimmed: the four direct-vs-`dyn` pairs (registry, filesystem, limiter, runtime); `arc_clone` and `downcast_to_concrete` are deleted | none |
| `linux_rootfs`    | `OverlayFilesystem::setup_rootfs()` / `cleanup()` with real extracted layers | `cfg(target_os = "linux")`, root: skip at runtime |
| `linux_cgroup`    | `CgroupV2Limiter::create()` / `add_process()` / `cleanup()`     | linux + root skip   |
| `linux_spawn`     | `LinuxNamespaceRuntime::spawn_process()` + `wait_for_exit()` for a trivial command | linux + root skip |

### xtask (`xtask/src/bench.rs`)

```rust
pub struct BenchOpts {
    pub skip_bench: bool,
    pub check: bool,          // compare latest vs bench/baseline.<env>.json, exit 1 on regression
    pub save_baseline: bool,  // promote latest run to bench/baseline.<env>.json
    pub threshold_pct: Option<f64>, // default 15.0
    pub env: String,          // baseline namespace: "local" (default) | "selfhosted" | "hosted"
}

pub fn bench(sh: &Shell, root: &Path, opts: &BenchOpts) -> Result<()>;

struct BaselineDelta {
    scenario: String,
    group: String,
    baseline_mean_ns: f64,
    current_mean_ns: f64,
    delta_pct: f64,
    regressed: bool,
}

fn compare_to_baseline(latest: &RunRecord, baseline: &RunRecord, threshold_pct: f64)
    -> Vec<BaselineDelta>;
```

- `cargo bench` invocation changes from `-p minibox` to `-p minibox-bench`.
- `BenchMetrics` drops the always-zero `memory_peak_bytes` field (old history files still
  deserialize; serde ignores the extra field).
- Regression rule per scenario: fail when `current_mean_ns > baseline_mean_ns * (1 + t/100)`
  where `t = max(threshold_pct, 2 * baseline_cv_pct)` and `baseline_cv_pct` is the baseline's
  coefficient of variation — noisy scenarios get proportionally wider bands.
- Scenario present in baseline but missing from the latest run → hard fail (inventory collapse);
  new scenario absent from baseline → informational only. `--check` with no baseline file for
  the selected env → informational pass (bootstrap run).
- Baselines are per environment (`bench/baseline.local.json`, `bench/baseline.selfhosted.json`,
  `bench/baseline.hosted.json`) because self-hosted and GitHub-hosted numbers are not
  comparable.
- CLI: `cargo xtask bench [--skip-bench] [--check] [--save-baseline] [--threshold <pct>]
  [--env <name>]`.

## Data Flow

1. Source: bench targets in `minibox-bench` call real entry points (`extract_layer`,
   `pull_image`, adapters) against fixtures built by `LayerSpec`/`BenchRegistry`.
2. Transform: criterion writes `target/criterion/`; `xtask bench` parses estimates into
   `RunRecord` and, with `--check`, computes `BaselineDelta`s against `bench/baseline.json`.
3. Sink: `bench/results/{latest.json,latest.csv,history/,index.html}` (gitignored) plus the
   tracked `bench/baseline.json`; nightly CI fails the job on any `regressed` delta.

## Hexagonal Boundaries

- Benches consume existing ports (`ImageRegistry`, `RootfsSetup`, `ResourceLimiter`,
  `ContainerRuntime`) and exercise the real adapters (`OverlayFilesystem`, `CgroupV2Limiter`,
  `LinuxNamespaceRuntime`); mocks are used only where dispatch overhead itself is the subject.
- The fixture builders are adapters over `wiremock`/`tar`/`flate2`; no new ports.

## Integration Points

- `minibox-core`: `RegistryClient::for_test` (registry.rs:395) is promoted from `pub(crate)` to
  `#[cfg(any(test, feature = "test-utils"))] pub` so `BenchRegistry::client()` can build a
  mock-pointed client. Feature-gated — invisible to production consumers.
- Workspace `Cargo.toml`: add `crates/minibox-bench` to members.
- `crates/minibox/Cargo.toml`: remove both `[[bench]]` sections and the criterion dev-dep;
  delete `crates/minibox/benches/`.
- `Justfile`: `bench` stays (`cargo xtask bench`); `bench-check` added (`--check`); dangling
  `bench-sync` and `flamegraph` targets are deleted (their xtask commands never existed).
- `.github/workflows/nightly.yml`: three-job bench pipeline with runner fallback — prefer the
  self-hosted runner when online, else GitHub-hosted:
  - `bench-runner-check` (ubuntu-latest): queries the repo runners API for an online runner
    labeled `minibox`. Uses the `ACTIONS_RUNNER_READ_TOKEN` secret (fine-grained PAT,
    Administration: read) because `GITHUB_TOKEN` cannot read runner status. If the secret is
    missing or the call fails, outputs `selfhosted=false` (safe fallback).
  - `bench-selfhosted` (`if: selfhosted == 'true'`, runs-on `[self-hosted, minibox]`):
    `cargo xtask bench --check --env selfhosted`.
  - `bench-hosted` (`if: selfhosted != 'true'`, runs-on `ubuntu-latest`):
    `cargo xtask bench --check --env hosted`.
  Both bench jobs upload `bench/results/` as artifact and run non-root, so root-gated benches
  skip in CI; they remain runnable manually via sudo on Linux hosts.
- `.gitignore`: add `bench/results/`; the `bench/baseline.<env>.json` files are tracked and
  updated only via `cargo xtask bench --save-baseline --env <name>` from a nightly-job
  artifact of the matching environment.

## Out of Scope

- Memory profiling (`memory_peak_bytes` replacement via tracking allocator) — deferred; the
  stub field is deleted rather than implemented.
- Flamegraph/profiling integration in xtask — removed from Justfile, not reimplemented.
- PR-time bench smoke job — nightly only, per brainstorm decision.
- Windows/macOS adapter benches (smolvm, krun, colima) — VM boot time dominates; not comparable.

## Risk

- [x] Breaking API changes: **no** published-API breaks. Internal: `minibox` loses its bench
      targets; `BenchMetrics` schema drops one field; `xtask bench` gains flags (existing
      invocation unchanged).
- [x] New external dependency: **no** new crates — wiremock, tar, flate2, sha2, criterion,
      tokio are all already workspace dependencies; minibox-bench only adds workspace refs.
- [x] Feature flag required: minibox-bench depends on `minibox`/`minibox-core` with
      `test-utils` enabled — confined to the leaf crate, satisfying the no-leak constraint.
- [ ] Four crates touched (limit guideline is 3): minibox-bench (new), minibox-core (one-line
      visibility change), minibox (deletion only), xtask (real changes) — accepted because two
      of the four are mechanical.
- [ ] Root-gated benches produce no canonical numbers until the job runs them as root;
      overlay/cgroup/spawn coverage on CI is skip-by-default initially.
- [ ] GitHub-hosted runners have run-to-run hardware variance (shared tenancy); the
      `2 x CV` component of the regression threshold absorbs some of it, but flaky
      regressions may need threshold tuning after the first weeks of history.
