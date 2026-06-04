---
status: done
---

# Plan: Rustqual Bulk Sweep — Production Crates

## Goal

Reduce rustqual findings in production source files from 533 to <150 by fixing
actionable categories (error handling, magic numbers, long functions, dead code,
duplicates, SRP params, IOSP violations) across `minibox`, `minibox-core`,
`miniboxd`, and `minibox-cli`. Deferred: UNSAFE (already documented), test
quality (TQ_*), FRAGMENT, BOILERPLATE.

## Architecture

- Crates affected: `minibox`, `minibox-core`, `miniboxd`, `minibox-cli`
- No new traits/types — this is a refactoring-only sweep
- No public API changes — all extractions are internal helpers
- Data flow: unchanged

## Tech Stack

- Rust 2024, anyhow for error context
- No new dependencies

## Subagent Partition

Three parallel subagents, one per crate group. Each runs in a worktree branch.

| Slot | Crate(s) | Branch | Est. findings |
|------|----------|--------|---------------|
| 1 | `minibox-core` | `qual/minibox-core` | ~100 |
| 2 | `minibox` | `qual/minibox` | ~240 |
| 3 | `miniboxd` + `minibox-cli` | `qual/miniboxd` | ~30 |

Slot 2 is largest. Slot 3 is small enough to combine.

---

## Tasks — Slot 1: minibox-core

### Task 1.1: Error handling — replace unwrap/panic in mocks

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/adapters/mocks.rs`
**Run**: `cargo check -p minibox-core`

Findings: 10 `unwrap/panic/todo` in MockRegistry, MockFilesystem, MockLimiter,
MockRuntime.

1. Replace each `.unwrap()` with `.expect("mock: <context>")` — mocks are test
   infrastructure so `expect` is appropriate (not `.context()?` since mock trait
   impls often can't return Result).
2. For any mock fn that returns `Result`, use `.context("mock: <description>")?`
   instead.
3. Verify: `cargo check -p minibox-core`

### Task 1.2: Magic numbers — extract named constants

**Crate**: `minibox-core`
**File(s)**:
- `crates/minibox-core/src/image/layer.rs` (8 magic numbers)
- `crates/minibox-core/src/image/registry.rs` (12 magic numbers)
- `crates/minibox-core/src/image/mod.rs` (3 magic numbers)
- `crates/minibox-core/src/protocol.rs` (4 magic numbers)
- `crates/minibox-core/src/domain.rs` (2 magic numbers)
- `crates/minibox-core/src/preflight.rs` (3 magic numbers)

1. For each magic number, add a `const` at module or impl scope with a
   descriptive name. Examples:
   - `1024 * 1024` -> `const MAX_LAYER_SIZE: u64 = 1024 * 1024;`
   - Buffer sizes, retry counts, timeouts -> named constants
   - Port numbers, HTTP status codes -> named constants
2. Replace all inline literals with the named constant.
3. Verify: `cargo check -p minibox-core && cargo clippy -p minibox-core -- -D warnings`

### Task 1.3: Long functions — extract helpers

**Crate**: `minibox-core`
**File(s)**:
- `crates/minibox-core/src/image/layer.rs:144` — `extract_layer` (156 lines)
- `crates/minibox-core/src/image/mod.rs:236` — `store_layer_verified` (94 lines)
- `crates/minibox-core/src/image/mod.rs:367` — `image_dir` (64 lines)
- `crates/minibox-core/src/image/registry.rs:365` — `get_manifest_inner` (88 lines)
- `crates/minibox-core/src/image/registry.rs:552` — `pull_image` (258 lines)
- `crates/minibox-core/src/domain/execution_policy.rs:63` — `evaluate` (79 lines)
- `crates/minibox-core/src/trace.rs:135` — `list` (63 lines)

1. Read each function. Identify logical blocks that can become private helpers.
2. Extract helpers with descriptive names. Each helper should be <40 lines.
3. For `pull_image` (258 lines) — split into: `resolve_manifest`,
   `download_layers`, `assemble_image` (or similar based on actual logic).
4. For `extract_layer` (156 lines) — split into: `validate_entry`,
   `extract_file_entry`, `extract_symlink_entry` (or similar).
5. Verify: `cargo nextest run -p minibox-core`

### Task 1.4: Dead code — remove or cfg-gate

**Crate**: `minibox-core`
**File(s)**:
- `crates/minibox-core/src/adapters/mocks.rs` (testonly fns)
- `crates/minibox-core/src/adapters/conformance.rs` (testonly)
- `crates/minibox-core/src/preflight.rs` (uncalled fns)
- `crates/minibox-core/src/image/lease.rs`
- `crates/minibox-core/src/domain.rs`

1. For `testonly` findings: add `#[cfg(test)]` or `#[cfg(any(test, feature = "testing"))]`
   if used by other crates' tests.
2. For `uncalled` findings: verify with Grep that the function is truly unused
   across the workspace. If unused, delete it. If used only in tests, cfg-gate.
3. Verify: `cargo check --workspace`

### Task 1.5: Duplicates — deduplicate

**Crate**: `minibox-core`
**File(s)**:
- `crates/minibox-core/src/adapters/mocks.rs:488` — `spawn_process_sync`
- `crates/minibox-core/src/adapters/test_fixtures.rs` — `MockAdapterBuilder::build`,
  `TempContainerFixture::new`
- `crates/minibox-core/src/preflight.rs` — `format_report`, `probe_kernel_version`,
  `parse_kernel_version`

1. For test_fixtures duplicates: check if `minibox-core` and `minibox` have
   identical copies. If so, re-export from one location.
2. For preflight duplicates: check if `minibox-core::preflight` and
   `minibox::preflight` share code. Extract shared logic to `minibox-core` and
   import in `minibox`.
3. Verify: `cargo check --workspace`

### Task 1.6: VIOLATION — separate logic from calls

**Crate**: `minibox-core`
**File(s)**:
- `crates/minibox-core/src/image/manifest.rs:214` — `TargetPlatform::parse`
- `crates/minibox-core/src/image/mod.rs:236` — `store_layer_verified`
- `crates/minibox-core/src/image/registry.rs:365` — `get_manifest_inner`

1. For each VIOLATION (logic + calls mixed): extract the pure-logic portion
   into a separate function that takes data in and returns data out, leaving
   the I/O calls in the original function.
2. This overlaps with Task 1.3 long-fn extractions — coordinate to avoid
   double-editing.
3. Verify: `cargo nextest run -p minibox-core`

### Task 1.7: Final verify + commit

**Run**: `cargo nextest run -p minibox-core && cargo clippy -p minibox-core -- -D warnings`

1. Run full suite for minibox-core.
2. Run `rustqual --suggestions 2>&1 | grep "minibox-core/" | grep -v "/tests/" | wc -l`
   to confirm reduction.
3. Commit: `git commit -m "refactor(minibox-core): rustqual bulk sweep — error handling, magic numbers, long fns, dead code, duplicates"`

---

## Tasks — Slot 2: minibox

### Task 2.1: Error handling — replace unwrap/panic in prod paths

**Crate**: `minibox`
**File(s)**:
- `crates/minibox/src/adapters/colima.rs` (6 findings)
- `crates/minibox/src/adapters/mocks.rs` (5 findings)
- `crates/minibox/src/daemon/handler/run.rs` (3 findings)
- `crates/minibox/src/container/process.rs` (2 findings)
- `crates/minibox/src/daemon/server.rs` (2 findings)
- Other handler files (scattered)

1. In adapter/daemon code: replace `.unwrap()` with `.context("description")?`.
2. In mock code: replace with `.expect("mock: description")`.
3. For `todo!()` macros: replace with `bail!("not yet implemented: <feature>")`.
4. Verify: `cargo check -p minibox`

### Task 2.2: Magic numbers — extract named constants

**Crate**: `minibox`
**File(s)**:
- `crates/minibox/src/adapters/colima.rs` (8 magic numbers)
- `crates/minibox/src/adapters/network/bridge.rs` (10 magic numbers)
- `crates/minibox/src/container/process.rs` (5 magic numbers)
- `crates/minibox/src/daemon/handler/run.rs` (4 magic numbers)
- `crates/minibox/src/daemon/server.rs` (3 magic numbers)
- `crates/minibox/benches/*.rs` (20+ magic numbers)
- Other scattered files

1. Module-level `const` for each. Group related constants (e.g., all network
   constants together in bridge.rs).
2. For bench files: constants at top of file.
3. Verify: `cargo check -p minibox`

### Task 2.3: Long functions — extract helpers

**Crate**: `minibox`
**File(s)**: (49 findings — focus on worst offenders >100 lines)
- `src/adapters/builder.rs:86` — `build_image` (286 lines)
- `src/daemon/handler/run.rs:366` — `prepare_run` (272 lines)
- `src/daemon/handler/run.rs:153` — `handle_run_streaming` (175 lines)
- `src/daemon/handler/pipeline.rs:41` — `handle_pipeline` (232 lines)
- `src/daemon/handler/update.rs:30` — `handle_update` (216 lines)
- `src/daemon/server.rs:415` — `dispatch` (257 lines)
- `src/adapters/push.rs:50` — `push_image` (138 lines)
- `src/container/filesystem.rs:195` — `pivot_root_to` (117 lines)
- `src/container/filesystem.rs:341` — `apply_one_bind_mount` (99 lines)
- `src/adapters/exec.rs:262` — `run_pty_exec` (97 lines)
- `examples/showcase.rs:255` — `main` (271 lines)

1. Read each function. Extract logical blocks into private helpers.
2. Priority: functions >150 lines first (`build_image`, `prepare_run`,
   `handle_pipeline`, `dispatch`, `handle_update`, `showcase::main`,
   `handle_run_streaming`).
3. Functions 60-100 lines: extract only if there's a clear seam. Don't force it.
4. Verify after each file: `cargo check -p minibox`

### Task 2.4: Dead code — remove or cfg-gate

**Crate**: `minibox`
**File(s)**:
- `crates/minibox/src/adapters/colima.rs` (testonly fns)
- `crates/minibox/src/adapters/mocks.rs` (testonly)
- `crates/minibox/src/preflight.rs` (uncalled)
- `crates/minibox/src/testing/` (testonly helpers)
- Various scattered `pub` fns only used in tests

1. Same approach as Task 1.4: cfg-gate testonly, delete truly unused.
2. For `crates/minibox/src/testing/` module: ensure it's behind
   `#[cfg(any(test, feature = "testing"))]` at the mod declaration.
3. Verify: `cargo check --workspace`

### Task 2.5: Duplicates — deduplicate colima lima_exec and mock builders

**Crate**: `minibox`
**File(s)**:
- `crates/minibox/src/adapters/colima.rs` — 4x `lima_exec` duplicates
  (lines 150, 380, 558, 737)
- `crates/minibox/src/daemon/handler/lifecycle.rs` — `handle_pause` /
  `handle_resume` duplicate
- `crates/minibox/src/testing/fixtures/container.rs` — `MockAdapterBuilder::build`,
  `TempContainerFixture::new` (duplicated from minibox-core)

1. For `lima_exec`: extract a shared `fn lima_exec(args, cwd)` at module level
   or in a `colima_common` submodule. Each adapter struct calls the shared fn.
2. For pause/resume: extract common lock-lookup-update pattern into a helper.
3. For test fixtures: import from minibox-core instead of duplicating.
4. Verify: `cargo nextest run -p minibox`

### Task 2.6: SRP params — introduce config structs

**Crate**: `minibox`
**File(s)**:
- Functions with 6+ parameters (18 findings across minibox + minibox-core)

1. For each function with 6+ params: group related params into a config struct.
   Examples: `RunConfig`, `BuildConfig`, `NetworkConfig`.
2. Only create a struct if 3+ params are logically related. Don't force it for
   functions where params are genuinely independent.
3. Verify: `cargo check -p minibox`

### Task 2.7: VIOLATION — separate logic from calls

**Crate**: `minibox`
**File(s)**:
- `src/adapters/gke.rs:239` — `ProotRuntime::from_env`
- `src/adapters/network/bridge.rs` — setup, attach, cleanup, stats

1. Extract pure validation/computation into helper fns.
2. Keep I/O (Command::new, fs::write, etc.) in the original fn.
3. Verify: `cargo check -p minibox`

### Task 2.8: Final verify + commit

**Run**: `cargo nextest run -p minibox && cargo clippy -p minibox -- -D warnings`

1. Full suite.
2. Confirm rustqual reduction.
3. Commit: `git commit -m "refactor(minibox): rustqual bulk sweep — error handling, magic numbers, long fns, dead code, duplicates, SRP"`

---

## Tasks — Slot 3: miniboxd + minibox-cli

### Task 3.1: All categories — miniboxd

**Crate**: `miniboxd`
**File(s)**:
- `crates/miniboxd/src/main.rs` — 3 LONG_FN (`run_daemon` 223 lines,
  `build_handler_deps` 63 lines, `build_native_handler_dependencies` 68 lines),
  magic numbers, dead code
- `crates/miniboxd/src/adapter_registry.rs` — 8 findings
- `crates/miniboxd/src/config.rs` — 1 VIOLATION

1. Extract `run_daemon` into: `setup_tracing`, `bind_socket`, `accept_loop`
   (or similar based on actual structure).
2. Replace magic numbers with named constants.
3. Fix error handling (unwrap -> context).
4. Separate logic from calls in `DaemonConfig::load`.
5. Verify: `cargo check -p miniboxd`

### Task 3.2: Final verify + commit

**Run**: `cargo nextest run -p miniboxd && cargo clippy -p miniboxd -- -D warnings`

1. Full suite.
2. Commit: `git commit -m "refactor(miniboxd): rustqual bulk sweep"`

---

## Integration

After all three slots complete:

1. Merge branches sequentially: `qual/minibox-core` -> `qual/minibox` ->
   `qual/miniboxd` (core first since others depend on it).
2. Run: `cargo nextest run --workspace && cargo clippy --workspace -- -D warnings`
3. Run: `rustqual --suggestions` and compare before/after.
4. Squash-merge to develop.

## Out of Scope

- Test file findings (1830 findings — separate effort)
- UNSAFE findings (already documented per project rules)
- FRAGMENT findings (low value, high churn)
- BOILERPLATE findings (low value)
- TQ_UNTESTED (adding tests is a separate effort)
- Adding new tests for extracted helpers (separate effort)
- macbox crate findings (not in scope)

## Risk

- Long function extraction may change behavior if error handling paths differ
  after extraction. Mitigated by running full test suite per slot.
- Dead code removal may break downstream crates. Mitigated by
  `cargo check --workspace` after each removal.
- Duplicate deduplication (test fixtures) may require adding `pub` visibility
  or a `testing` feature flag. Confirm approach before changing.
