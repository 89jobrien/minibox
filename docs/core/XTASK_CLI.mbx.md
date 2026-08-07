# xtask CLI Reference

Last updated: 2026-08-07

Full command surface of `cargo xtask`, rendered from `xtask/schema/cli.schema.json`
(the machine-readable source of truth — regenerate this doc by hand alongside the
schema when `xtask/src/main.rs`'s match arms change). Every entry below is verified
directly against the current `main()` dispatcher in `xtask/src/main.rs`.

Two entries — `available` and `lint-paths` — exist as top-level match arms but are
**not** listed in `cargo xtask`'s own `print_help()` output. They're documented here
for completeness but are effectively undiscoverable from `--help`.

---

## Subcommand groups

### `test <suite>`

Bare `cargo xtask test` prints the suite list and exits 0.

| Suite | Notes |
|---|---|
| `unit` | Unit + conformance tests, any platform |
| `conformance` | Commit+build+push conformance suite + reports |
| `krun-conformance` | krun adapter conformance (HVF/KVM) |
| `turmoil` | Turmoil network simulation tests |
| `shuttle` | Shuttle concurrency tests |
| `property` | Property-based tests (proptest) |
| `quickcheck` | Quickcheck property tests |
| `integration` | cgroup + integration tests (Linux, root) |
| `e2e` | Protocol e2e tests, any platform |
| `system-suite` / `e2e-suite` | Full-stack system tests (Linux, root) — both names dispatch identically |
| `sandbox` | Sandbox contract tests (Linux, root) |
| `gke-profile` | GKE profile unit tests |
| `gke-adapter` | GKE adapter integration tests |
| `cgroup` | Alias reaching `cgroup_tests::run_cgroup_tests` directly (not in the printed suite list) |

Deprecated aliases (each prints a one-time deprecation note, then dispatches to the
equivalent `test <suite>` call): `test-unit`, `test-conformance`, `test-krun-conformance`,
`test-turmoil`, `test-shuttle`, `test-property`, `test-quickcheck`, `test-integration`,
`test-e2e`, `test-system-suite`, `test-e2e-suite`, `test-sandbox`, `test-gke-profile`,
`test-gke-adapter`.

### `check <target> [flags...]`

Bare `cargo xtask check` prints the target list.

| Target | Flags | Notes |
|---|---|---|
| `stale-names` | — | Audit for banned old crate/binary names |
| `protocol-drift` | `--update` `--warn-only` `--hook` `--sarif <path>` | Verify core contract hashes against `xtask/protocol-drift.lock`. `--update` writes new hashes instead of comparing. `--hook` reads a Claude Code PostToolUse hook JSON payload from stdin and only checks if the edited file is a tracked surface. |
| `protocol-sites` | `[<file>]` `--expected N` `--warn-only` | Verify `HandlerDependencies` construction-site count. Default file: `crates/miniboxd/src/main.rs`. Default expected: `4`. |
| `protocol-variants` | — | Scan for `DaemonRequest`/`DaemonResponse` variants with no handler sites |
| `adapter-coverage` | — | Verify each adapter has integration test files |
| `no-unwrap` | `--strict` | Scan production code for `.unwrap()`. Without `--strict`, prints a warning and exits 0; with it, exits non-zero on any hit. |
| `repo-clean` | — | Warn (non-fatal) if generated artifacts are tracked by git |

Deprecated aliases: `check-stale-names`, `check-protocol-drift`, `check-protocol-sites`,
`check-adapter-coverage`, `check-no-unwrap`, `check-repo-clean`.

### `docs <action> [flags...]`

Bare `cargo xtask docs` prints the action list.

| Action | Flags | Notes |
|---|---|---|
| `audit` | `--full` `--strict` | Audit `docs/core/` facts against code. `--full` runs full mode; `--strict` only affects Quick mode (the default when `--full` is absent). |
| `lint` | `--sarif <path>` | Validate frontmatter + status values across doc files. |
| `update-date` | — | Rewrite the Last-updated stamp in `FEATURE_MATRIX.mbx.md`. |

Deprecated aliases: `docs-audit`, `lint-docs`, `update-feature-matrix-date`.

### `info <target> [flags...]`

Bare `cargo xtask info` prints the target list.

| Target | Flags | Notes |
|---|---|---|
| `metrics` | `--save` | Aggregate crate count, test count, source lines. `--save` persists the snapshot to disk. |
| `context` | `--save` | Machine-readable repo context snapshot. |
| `changes` | `[<base-ref>]` | Classify changed paths; emits GitHub Actions step outputs. Default base ref: `HEAD^`. |

Deprecated aliases: `collect-metrics`, `context`, `detect-changes`.

---

## Quality gates

| Command | Mutates files? | Description |
|---|---|---|
| `verify` | No | fmt --check, `cargo check --workspace`, clippy `-D warnings`, borrow-reasoning fixtures, docs lint. Checkpointed — skipped if the tree hash is unchanged since the last pass. |
| `lint` | No | fmt --check + clippy `-D warnings` + `cargo check --workspace` across the full adapter matrix. Checkpointed. |
| `fix` | **Yes** | `cargo fmt --all`, re-stage, version bump, `clippy --fix --allow-dirty --allow-staged`, re-stage again. Only runs the mutating steps if Rust files are currently staged. |
| `pre-commit` | Validation-only | The git pre-commit hook's gate: fmt+clippy on staged Rust files, agentlint on staged agent-config files, actionlint on staged workflow files, docs-lint, FEATURE_MATRIX date stamp refresh, repo-cleanliness warning. Never runs a release build or the conformance suite. |
| `prepush` | No | Release build (`miniboxd`, `minibox-core`, `mbx`, `minibox`, `minibox-macros`) + nextest (release profile) + conformance suite. Skipped entirely if no Rust files are in the push range. Checkpointed. |
| `agentlint [--all]` | No | Lint agent config files (`.claude/`, `.codex/`, `.agents/`, `.cursor/`). Without `--all`, only staged files are linted. |
| `coverage [--open] [--lcov-only] [--html-only]` | No | Generate a coverage report; `--open` opens the HTML report afterward. |
| `coverage-check` | No | Handler module function coverage gate. |

---

## Build / VM

| Command | Flags | Description |
|---|---|---|
| `build-test-image` | `--force` | Cross-compile the test binaries and package an OCI tarball for VM-based testing. `--force` rebuilds even if a cached image exists. |
| `setup-test-vm` | `--force` | Build/refresh a persistent smolvm VM with a Rust toolchain (macOS-side testing). |
| `test-in-vm` | `--skip-build` `--keep` `--smolfile <path>` | Dual-backend (native minibox + smolvm) test run inside a VM. `--keep` skips teardown. |
| `test-linux` | — | Cross-compile (zigbuild) + build a CPIO initramfs + run tests inside a smolvm QEMU VM. All configuration comes from `XConfig::load`, not flags. |

---

## Cleanup

| Command | Description |
|---|---|
| `clean-artifacts` | Remove non-critical build outputs. |
| `nuke-test-state` | Kill orphaned test processes, unmount leaked overlay mounts, clean stale cgroups/temp state. |

---

## CI / promotion

| Command | Flags | Description |
|---|---|---|
| `bump` | `[patch\|minor\|major]` (default `patch`) | Bump the workspace version. |
| `preflight` | — | Verify required tools are on PATH (`cargo`, `cargo-nextest`, `gh`). |
| `doctor` | — | Full preflight diagnostics — same underlying probe as `mbx doctor`. |
| `promote` | `--from <tier>` `--to <tier>` `--dry-run` | Cascade-merge one stability tier into the next (`develop -> next -> staging -> main`), gated on CI green for the source branch. `--from`/`--to` infer sensible defaults if omitted. |
| `ci-watch` | `--branch <name>` | Watch the most recent GitHub Actions run with job-level detail; defaults to the current branch. |
| `daily-orchestration` | `--ci` `--dry-run` | Run the daily maintenance orchestration pass. Unlike most xtask parsers, unrecognized flags here cause a hard usage error rather than a warning. |
| `council` | `--base <ref>` (default `main`) `--mode core\|extended` (default `core`) `--no-synthesis` `--prod` | Run devloop council analysis against a base ref. |

---

## Misc standalone

| Command | Flags | Description |
|---|---|---|
| `bench` | `--skip-bench` `--check` `--save-baseline` `--threshold <pct>` (default `15.0`) `--env <label>` (default `local`) | Run Criterion benchmarks, save results to `bench/results/`. `--check` compares against a saved baseline instead of running+saving. Unrecognized flags print a warning and are ignored (not a hard error). |
| `fuzz` | — | Run libFuzzer protocol targets. |
| `demo` | `--adapter <name>` (default `smolvm`) | Short end-to-end demonstration: pull + run against the named adapter. Advisory — exits 0 even if the underlying commands fail. |
| `borrow-fixtures` | — | Run borrow-reasoning must-pass/must-fail fixtures. |
| `clippy-sarif` | `[<path>]` (default `clippy.sarif`) | Run clippy and write results as a SARIF report. |
| `run-cgroup-tests` | — | cgroup v2 integration tests (Linux, root). |
| `clean-artifacts` | — | See Cleanup above. |
| `nuke-test-state` | — | See Cleanup above. |
| `cas-add` | `<file>` `--ref <name>` | Add a file to the content-addressed overlay store, with an optional named reference. |
| `cas-check` | — | Verify overlay refs match their CAS objects. |
| `run` | `<script> [args...]` | Run `scripts/<script>.nu`, forwarding remaining args. Errors with the available script list if not found. |
| `lint-paths` *(undocumented)* | — | Lint path-handling code. Not listed in `print_help()`. |
| `available` *(undocumented)* | — | Check whether the xtask binary itself is available/buildable. Not listed in `print_help()`. |

---

## Notes on the schema file

`xtask/schema/cli.schema.json` is a JSON Schema (draft 2020-12) describing this same
command surface as a `oneOf` over one object schema per command, each with a
`command` const and an `args` object matching the flags above. It's hand-maintained
against `xtask/src/main.rs` — there's no build-time generator, so schema and this doc
can drift from source if `main.rs`'s match arms change without a corresponding update
here.
