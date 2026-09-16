---
source_sha: f0fd2661aa36411f42a73ed1f44ce49455acd7d1
sources:
  - xtask/src/main.rs
  - xtask/schema/cli.schema.json
  - xtask/src/context.rs
  - xtask/src/context/model.rs
  - xtask/src/context/collect.rs
  - xtask/src/context/manifest.rs
  - xtask/src/context/test_inventory.rs
  - xtask/src/context/output.rs
  - xtask/context.toml
generated: 2026-09-12
---

# xtask CLI Reference

Last updated: 2026-09-16

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
| `lint` | `--sarif <path>` | Validate frontmatter + status values under the legacy `docs/superpowers/{plans,specs}/` paths. If those directories are absent, zero files are checked. |
| `update-date` | — | Rewrite the Last-updated stamp in `FEATURE_MATRIX.mbx.md`. |
| `sync-adapters` | — | Idempotently regenerate only the marked adapter-suite and capability blocks from `xtask/context.toml`. |

Deprecated aliases: `docs-audit`, `lint-docs`, `update-feature-matrix-date`.

### `info <target> [flags...]`

Bare `cargo xtask info` prints the target list.

| Target | Flags | Notes |
|---|---|---|
| `metrics` | `--save` | Aggregate crate count, test count, source lines. `--save` persists the snapshot to disk. |
| `context` | `--save` `--strict` `--validate-all` `--evidence-dir <path>` | Emit the evidence-ledger context snapshot v3; persistence and validation policy are explicit. |
| `changes` | `[<base-ref>]` | Classify changed paths; emits GitHub Actions step outputs. Default base ref: `HEAD^`. |

Deprecated aliases: `collect-metrics`, `context`, `detect-changes`.

#### Context snapshot v3

`cargo xtask info context` emits one formatted JSON document on stdout. Snapshot v3 replaces snapshot v2 rather than extending its ambiguous summary fields. The top-level sections are `identity`, `environment`, `workspace`, `adapters`, `tests`, `context_map`, `evidence`, and `diagnostics`.

Options:

- `--save` validates first, then atomically replaces `artifacts/context/snapshot.json`, appends one compact v3 record to `history.jsonl`, and persists only exact-key current profile evidence. Without this flag the command is read-only.
- `--strict` applies the required native-profile threshold. JSON is emitted before a strict validation error produces a nonzero exit.
- `--validate-all` attempts every profile declared in `xtask/context.toml`. By itself it reports failures in JSON and exits zero.
- `--validate-all --strict` requires current validated evidence for every profile marked `required_in_ci`; optional profile failures remain diagnostics.
- `--evidence-dir <path>` imports CI profile evidence. Exact-key records can satisfy required profiles; stale records remain visible for diagnostics but contribute no executable tests.

`cargo xtask docs sync-adapters` idempotently rewrites only the explicitly marked adapter-suite and capability tables in `docs/core/FEATURE_MATRIX.mbx.md` from `xtask/context.toml`.

##### Evidence and validation semantics

Facts carry separate `declared`, `observed`, and `validation` values. `declared` records policy or manifest intent, `observed` records runtime or source evidence, and `validation.state` distinguishes `match`, `mismatch`, `declared_only`, `observed_only`, `unavailable`, and `not_applicable`. Missing observations are never silently converted to empty strings, zero tests, or successful validation.

The repository identity includes the full commit, branch, changed paths, and a dirty-worktree fingerprint. The fingerprint covers tracked and untracked changed-path state and changed content while excluding ignored files and content outside the repository.

Profile cache freshness requires an exact match across commit, dirty-worktree fingerprint, manifest SHA-256, target triple, sorted feature set, default-feature mode, Cargo version, rustc version, and cargo-nextest version. A complete match is current evidence. Any mismatch is stale evidence. A profile with `unavailable` status is not equivalent to a validated profile containing zero tests.

Executable tests are listed per named target/profile from cargo-nextest JSON. Cross-profile counts deduplicate stable package/binary/test identities. Static Rust declarations remain separate because `#[cfg]`, target selection, required features, and macro expansion can change executable inventory.

##### Migration from v2

Machine consumers must branch on `snapshot_version` and replace their v2 field access:

| Removed v2 field | v3 replacement |
|---|---|
| top-level `commit`, `branch`, `timestamp` | `identity` plus `environment.generated_at` |
| `crates` and `crates[].deps` | `workspace.packages` and typed `dependencies` |
| `tests.total` and per-crate `test_count` | `tests.profiles` plus `validated_unique_tests` |
| `ci_workflows` and `recent_commits` | Removed; they were not validated context evidence |
| `context_map.crate_assignments` | `context_map.crates` |
| `context_map.file_assignments` | `context_map.files` and `changed_files` |
| `context_map.task_slices` | `context_map.collector_tasks` with executed statuses and evidence IDs |

The v3 schema rejects the removed `crate_assignments`, `file_assignments`, and `task_slices` fields and unknown object properties.

---

## Quality gates

| Command | Mutates files? | Description |
|---|---|---|
| `verify` | No | fmt check, workspace check, targeted clippy, architecture guard, borrow fixtures, docs lint, and quick docs audit. Checkpointed. |
| `lint` | No | fmt check, targeted clippy (`minibox`, domain, macros, CLI, core, macbox, miniboxd, winbox, ail), workspace check, and architecture guard. Checkpointed. |
| `fix` | **Yes** | `cargo fmt --all`, re-stage, version bump, `clippy --fix --allow-dirty --allow-staged`, re-stage again. Only runs the mutating steps if Rust files are currently staged. |
| `pre-commit` | **Yes, conditionally** | With staged Rust, runs `cargo fmt --all`, re-stages tracked `.rs` changes, targeted clippy, and architecture. Also conditionally runs agentlint/actionlint, always lints docs, refreshes the FEATURE_MATRIX date, and warns on tracked generated artifacts. No release build or conformance. |
| `prepush` | No | If Rust is in the push range: starts with `musl-check`, then release build (`miniboxd`, `minibox-core`, `minibox-cli`, `minibox`, `minibox-macros`), release library nextest, and conformance. `SKIP_PHASE_2=1` only skips conformance locally. |
| `musl-check` | No | Cross-build `miniboxd` and `minibox-cli` for `x86_64-unknown-linux-musl`; warns/skips when prerequisites are absent unless `MINIBOX_REQUIRE_MUSL_CHECK=1`. |
| `agentlint [--all]` | No | Lint agent config files (`.claude/`, `.codex/`, `.agents/`, `.cursor/`). Without `--all`, only staged files are linted. |
| `coverage [--open] [--lcov-only] [--html-only]` | No | Generate a coverage report; `--open` opens the HTML report afterward. |
| `coverage-check` | No | Handler module function coverage gate. |
| `architecture` | No | Enforce dependency rings and canonical type ownership. |

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
| `promote` | `--from <tier>` `--to <tier>` `--dry-run` `--skip-ci-check` | Cascade-merge one stability tier into the next (`develop -> staging -> release -> main`), gated on CI green unless explicitly overridden. |
| `ci-watch` | `--branch <name>` | Watch the most recent GitHub Actions run with job-level detail; defaults to the current branch. |
| `daily-orchestration` | `--ci` `--dry-run` | Run the daily maintenance orchestration pass. Unlike most xtask parsers, unrecognized flags here cause a hard usage error rather than a warning. |
| `council` | `--base <ref>` (default `main`) `--mode core\|extended` (default `core`) `--no-synthesis` `--prod` | Run devloop council analysis against a base ref. |

---

## Misc standalone

| Command | Flags | Description |
|---|---|---|
| `bench` | `--skip-bench` `--check` `--save-baseline` `--threshold <pct>` (default `15.0`) `--env <label>` (default `local`) | Run Criterion benchmarks, save results to `bench/results/`. `--check` compares against a saved baseline instead of running+saving. Unrecognized flags print a warning and are ignored (not a hard error). |
| `fuzz` | — | Run libFuzzer protocol targets. |
| `demo` | `--adapter <name>` (default `smolvm`) `--filter <name>` `--strict` | Run showcase scenarios, optionally filtered; strict mode propagates failures. |
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
