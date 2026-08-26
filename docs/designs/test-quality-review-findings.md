# Test Quality Review — Findings

Companion findings report for the test-quality review plan
(`.ctx/tasks/test-quality-review.md`, local/gitignored). Read-only review —
no production or test code is modified as part of this report; any mutation
applied during the Mutation section is reverted immediately after each
check and never committed.

## Sweep: No-op / trivial assertions

- `assert!(true)`: **0 matches** across `crates/**/*.rs`. Clean.
- Empty `#[test] fn test_...() {}` bodies: **0 matches**. Clean.
- `assert!(expr.is_ok())` as the _only_ assertion in a test: **67 matches**
  across 26 files, no file concentrated (max ~4 per file — e.g.
  `crates/minibox/src/adapters/network/none.rs`,
  `crates/minibox/tests/adapter_failure_injection_tests.rs`).
- `assert!(expr.is_err())` as the _only_ assertion in a test: **71 matches**,
  similarly spread thin (max ~3 per file — e.g.
  `crates/minibox/tests/conformance_snapshot.rs`,
  `crates/minibox/src/nesting.rs`).

**Verdict**: not a concentrated problem, but a real pattern worth naming.
Spot-checked `fs_util.rs::nonexistent_source_returns_error` — `is_err()` is
the right assertion there (only the error variant matters, path doesn't
exist by construction). Spot-checked
`adapters/network/none.rs::noop_network_setup_returns_empty_string` — the
test name claims the return value is checked ("returns*empty_string") but
the body only asserts `.is_ok()`, never asserts the returned string actually
\_is* empty. That's a real gap: a regression that made `NoopNetwork::setup`
return a non-empty garbage string would pass this test. This is likely
representative of a chunk of the 67 `is_ok()`-only hits — the assertion is
weaker than the test name promises. Not filed as a blocking finding on its
own since `none.rs` is a no-op stub adapter (low blast radius), but the
pattern ("test name implies value-checking, body only checks the Result
variant") is worth a targeted second pass beyond this review's scope.

## Sweep: Over-mocked tests

Searched for `call_count`/`times_called`/`was_called`/`call_log`/`invocations`
assertions across `crates/**/*.rs` (10 files with any hit). Spot-checked the
three highest-signal cases:

- `crates/minibox/tests/daemon_handler_image_tests.rs:993` —
  `mock_committer.call_count() == 1` is paired with a real assertion on the
  handler's `DaemonResponse::Success` message content (line 989-992). Not
  over-mocked: the test would fail if the commit handler produced the wrong
  response even with the mock called correctly. **Clean.**
- `crates/minibox/tests/daemon_conformance_tests.rs:1412`
  (`noop_gc_prune_is_idempotent`) — `gc.prune_call_count()` assertion is
  paired with `report.freed_bytes == 0` and `report.removed.is_empty()`
  checks per iteration. **Clean.**
- `crates/minibox/tests/security_regression.rs:780`
  (`mutation_audit_peercred_guard_called_in_handler`) — not a mock-based
  test at all, but structurally the same failure mode Task 2 is looking
  for: it does `include_str!("../src/daemon/server.rs")` and asserts
  `source.matches("is_authorized(").count() >= 2` and
  `source.contains("!is_authorized(")`. This is a **source-text grep
  masquerading as a behavioral security test** — it verifies the string
  `is_authorized(` appears twice in the file, not that unauthorized
  connections are actually rejected at runtime. A refactor that renamed the
  call, wrapped it in a helper, or introduced the same substring in an
  unrelated comment would silently pass or fail this test for the wrong
  reason. The docstring says "The behavioral tests in
  `daemon_security_regression.rs` verify the function's logic" — so real
  behavioral coverage may exist elsewhere; this test's own value is limited
  to catching wholesale removal of the call site.

**Verdict**: mock-call-count tests in this codebase are generally well-formed
(paired with domain-state assertions). The concerning pattern is a
different one — text-based "mutation audit" tests in `security_regression.rs`
that assert on source-code strings rather than behavior. Flagging as a
candidate for the mutation-check phase (Task 5-7 targets don't currently
include this file, but it's the same class of risk: a security invariant
whose test can pass without the invariant actually holding at runtime).

## Sweep: Swallowed failures in tests

Searched for `let _ = ...unwrap()/.expect()` (0 matches) and `.ok();` as a
statement terminator (59 matches workspace-wide) in `crates/**/*.rs`, then
narrowed to files under `tests/` (5 files: `miniboxd/tests/cgroup_tests.rs`,
`minibox/tests/daemon_conformance_tests.rs`,
`minibox/tests/daemon_handler_lifecycle_tests.rs`,
`minibox/tests/daemon_state_persistence_tests.rs`, and
`minibox/tests/conformance_report.rs`) and inspected each hit with context.

- `cgroup_tests.rs:87` — `std::env::var("MINIBOX_CGROUP_ROOT").ok()` is a
  fixture save/restore read, not a discarded assertion. **Clean.**
- The remaining ~10 hits across `daemon_conformance_tests.rs`,
  `daemon_handler_lifecycle_tests.rs`, and `daemon_state_persistence_tests.rs`
  are all the same shape: `state.update_container_state(&id, ...).await.ok();`
  used in the **arrange** phase to force a container into a known state
  before exercising the handler under test — not discarding the result of
  the function actually being tested. In `daemon_state_persistence_tests.rs`
  the surrounding comment ("must not panic") makes the intent explicit: the
  call is expected to possibly fail for a nonexistent container, and only
  panic-freedom is being verified, so discarding the `Result` there is
  correct, not a masked failure.

**Verdict**: no swallowed-failure anti-pattern found in test code. The only
workspace-wide occurrences of `.ok()`-in-tests are legitimate
fixture-arrangement calls. (The remaining ~49 `.ok();` hits are in
production `src/` files — out of scope for this test-quality review, though
worth noting `crates/minibox/src/daemon/handler/{lifecycle,stop,run}.rs`
each have at least one; per `rust-patterns.md`'s own anti-pattern table
`.ok()` swallowing a fallible call is called out as a known risk — those
call sites weren't audited here since they're production code, not tests.)

## Sweep: Stale snapshots

`crates/**/*.snap.new` (unaccepted snapshot updates): **0 matches**.
`insta::assert*_snapshot!` call sites: **3**, matching **3** committed
`.snap` files 1:1. **Clean** — no orphaned or unreviewed snapshots.

## Sweep summary

| Category                                                         | Found                                                   | Verdict                                                                                                                                                                                                                                 |
| ---------------------------------------------------------------- | ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| No-op / trivial assertions (`assert!(true)`, empty test bodies)  | 0                                                       | Clean                                                                                                                                                                                                                                   |
| `is_ok()`/`is_err()`-only assertions                             | 67 + 71                                                 | Not concentrated; spot-check found one real gap class — test names implying value checks that the body doesn't perform (see `none.rs::noop_network_setup_returns_empty_string`). Not previously tracked in `mistakes.md`.               |
| Over-mocked (call-count-only) tests                              | ~10 files w/ call-count assertions                      | Clean — all paired with domain-state assertions, except one non-mock structural analog: `security_regression.rs`'s source-text "mutation audit" test (see below)                                                                        |
| Swallowed failures in test code (`.ok()`, `let _ = ...unwrap()`) | 0 in test arrange-phase beyond legitimate fixture setup | Clean in tests. Known in production code — see `.ctx/memory-bank/mistakes.md`'s "Handler Error Swallowing (2 occurrences)" entry, which already tracks this exact class (`net.attach`, spawn-failure cleanup) for `src/`, not `tests/`. |
| Stale/unreviewed snapshots                                       | 0                                                       | Clean                                                                                                                                                                                                                                   |

**New pattern surfaced by this sweep, not previously in `mistakes.md`**:
source-text "mutation audit" tests (`security_regression.rs`, e.g.
`mutation_audit_peercred_guard_called_in_handler`) that `include_str!` a
source file and assert on substring counts rather than exercising runtime
behavior. This is a distinct risk class from the tracked "Handler Error
Swallowing" pattern — it affects test _validity_ (can a passing test still
mean the invariant is broken?) rather than production error handling. Fed
into the Mutation section below since it's the same underlying question the
mutation checks are designed to answer.

## Mutation: path validation

**Target**: `has_parent_dir_component()` in
`crates/minibox-core/src/image/layer.rs:65`, the pre-check used by
`validate_tar_entry_path()` (the Zip Slip / path-traversal guard for OCI
layer extraction).

**Mutation applied**: replaced the body with `false` unconditionally —
i.e. simulated the `..`-detection check being silently disabled while
leaving everything else (absolute-path rejection, canonicalize-and-prefix
check) intact.

**Run**: `cargo test -p minibox-core --lib image::layer::`

**Result**: 5 of 37 tests in the module **failed** —
`exhaustive_has_parent_dir_component_true_cases`, `dotdot_in_middle_rejected`,
`exhaustive_validate_tar_entry_path_table`,
`absolute_symlink_with_parent_traversal_rejected`, and the property test
`proptest_tests::dotdot_paths_always_rejected` (which found a minimal
failing case, `"a/../../a"`, and would have shrunk further on a real bug).
Mutation reverted via `git checkout --` immediately after the run; working
tree confirmed clean.

**Verdict**: **CONFIRMED GOOD** — this is exactly what a healthy test suite
looks like for a security-critical function. Multiple independent test
styles (direct unit tests, a table-driven exhaustive test, and a property
test) all caught the same regression, including the property test deriving
a novel failing input rather than replaying a fixed fixture. No follow-up
needed here; this module is a positive example, not a gap.

## Mutation: cgroup limits

**Target**: the memory-minimum bounds check in `CgroupManager::create()`,
`crates/minibox/src/container/cgroups.rs:140` (`if mem < MIN_MEMORY_BYTES`),
guarding against a caller-supplied memory limit below the kernel's ~4KB
floor.

**Mutation applied**: `if mem < MIN_MEMORY_BYTES` → `if false && mem <
MIN_MEMORY_BYTES` (permanently disabled the check while leaving the rest of
`create()` untouched).

**Run**: `cargo test -p miniboxd --test cgroup_tests` (targeting
`test_cgroup_rejects_invalid_memory_limit`, the test that directly exercises
this bound per the sweep).

**Result**: the entire `cgroup_tests.rs` file is gated
`#![cfg(target_os = "linux")]` — on this macOS dev machine it compiles to
**0 tests run**, mutated or not. Confirmed via a baseline run before
mutating (`running 0 tests`) and again after (`cargo build -p miniboxd
--tests` exits 0, meaning the mutated, security-check-disabled code
compiles and links cleanly). No test anywhere in the workspace outside this
Linux-gated file exercises `CgroupManager::create()`'s bounds checks — the
in-module `#[cfg(test)] mod tests` block in `cgroups.rs` itself only tests
path-construction helpers (`validate_cgroup_parent`, `with_root`,
`delegation_paths`), never `create()`. Mutation reverted via `git checkout
--`; working tree confirmed clean.

**Update (follow-up fix applied)**: extracted the bounds check into a new
cross-platform, filesystem-independent function
`minibox::resource_limits::validate_resource_limits()`
(`crates/minibox/src/resource_limits.rs`), called from
`CgroupManager::create()` before any filesystem work. The `container`
module (including `cgroups.rs`) is gated `#[cfg(target_os = "linux")]` at
`crates/minibox/src/lib.rs:57` — the extraction had to live in a new,
ungated top-level module (`pub mod resource_limits;`), not inside
`container::cgroups` itself, or it would still fail to compile on macOS.
Re-ran the identical mutation (disabling the memory-minimum check) against
the new location: `cargo test -p minibox --lib resource_limits::` now
**fails 1 of 7 tests** (`rejects_memory_below_minimum`) on this macOS
machine, where previously the equivalent check produced zero test signal
anywhere in the toolchain. `cargo check --workspace`, `cargo clippy -p
minibox --all-targets -- -D warnings`, and `cargo fmt --all --check` all
pass; full `cargo test -p minibox --lib` run is green (281 passed, 1
ignored). Mutation reverted before this note was written; working tree
verified clean of it.

**Original verdict** (superseded by the fix above, kept for record):
**CANNOT CONFIRM — coverage gap, not a test-quality gap**. This
isn't the same finding as the path-validation case: the _test itself_
(`test_cgroup_rejects_invalid_memory_limit`) looks correctly written (single
call, `assert!(result.is_err())` on a config just below the boundary) — the
problem is it structurally cannot run in this environment, and per
`TEST_INFRASTRUCTURE.mbx.md` cgroup integration tests only run in `next`/
`stable` CI (self-hosted Linux runner), not on every PR. A developer on
macOS (the primary dev machine per `.claude.local.md`) who breaks this
security check locally gets zero signal — `cargo check`/`clippy`/`cargo
test` all pass clean. The only way this bug would be caught is a push that
happens to trigger the `next`/`stable` gate, which per the CI docs is not
every merge. Recommend as a follow-up (not fixed in this review): a
`#[cfg(not(target_os = "linux"))]` unit test using a mockable/injectable
write path (or extracting the bounds-validation logic into a pure function
independent of the real cgroupfs write) so the security-critical bounds
checks get cross-platform coverage instead of being bundled with the
filesystem-writing integration test.

## Mutation: protocol terminal-response classification

**Target**: `DaemonResponse::is_terminal()` in
`crates/minibox-core/src/protocol.rs:753`, the `matches!` arm list deciding
which response variants end a request/response exchange.

**Mutation applied**: removed `Self::ContainerPaused { .. }` from the
terminal-variant list — a genuinely-terminal variant silently reclassified
as non-terminal (a real regression class per `progress.mbx.md`'s note that
"most other response variants end request streaming" and per the daemon's
handler contract).

**Run**: `cargo test -p minibox-core --lib protocol::`

**Result**: 1 of 70 tests **failed** —
`protocol::tests::is_terminal_matches_canonical_table` — with a precise
diagnostic: `unexpected terminal status for variant: ContainerPaused { id:
"abc" }, left: false, right: true`. Mutation reverted via `git checkout
--`; working tree confirmed clean.

**Verdict**: **CONFIRMED GOOD**. This test is explicitly designed for
exactly this failure mode — its own doc comment states it's "the canonical
terminal-variant table formerly hand-rolled in
`minibox::daemon::server::is_terminal_response`" and that the `match` guard
is written so it "won't compile until both this test and `is_terminal` are
updated" when a new variant is added. It caught the mutation on the first
run with a variant-level error message, not just a generic assertion
failure. Best-designed test found across all three mutation targets in this
review — a good model for the cgroup coverage gap identified above (an
exhaustive table test that stays in sync with the enum by construction).

## Overall verdict

**Sweep phase**: clean on every mechanical anti-pattern checked (no no-op
assertions, no over-mocked call-count-only tests, no swallowed failures in
test code, no stale snapshots). Two real but non-blocking observations
surfaced: a "test name promises more than the assertion checks" gap in
`adapters/network/none.rs`, and a source-text "mutation audit" test pattern
in `security_regression.rs` (`mutation_audit_peercred_guard_called_in_handler`)
that verifies string occurrences in a file rather than runtime behavior —
neither was in `.ctx/memory-bank/mistakes.md` before this review.

**Mutation phase — the real signal**: 2 of 3 targets had the mutation
**caught immediately** by well-designed tests (path validation: 5
independent failures including a property test deriving a novel input;
protocol terminal classification: caught by an exhaustive canonical-table
test built to fail-to-compile on drift). The third target — **cgroup
resource-limit bounds (memory minimum, CPU weight range)** — is the one
place a security-relevant regression would ship silently on this team's
primary dev platform: the test that exists for it is well-written, but it
lives in a file gated `#![cfg(target_os = "linux")]` that produces zero
compiled tests on macOS, and per `TEST_INFRASTRUCTURE.mbx.md` cgroup
integration tests only run in `next`/`stable` CI, not on every PR/push to
`develop`. `cargo check`, `clippy`, and `cargo test` all pass clean with the
security check fully disabled.

**Top 3 follow-ups** (not fixed here — this review is read-only against
production code):

1. **DONE — Cross-platform unit coverage for cgroup bounds validation.**
   Extracted the `MIN_MEMORY_BYTES`/`MAX_CPU_WEIGHT` range checks out of
   `CgroupManager::create()` into `minibox::resource_limits::validate_resource_limits()`
   (`crates/minibox/src/resource_limits.rs`, new ungated top-level module —
   not inside `container::cgroups`, since the whole `container` module is
   gated `#[cfg(target_os = "linux")]`), with 7 unit tests that now run on
   every platform. Re-running the original mutation against the new
   location confirmed it's now caught (1/7 tests fail) where it previously
   produced zero signal. See the updated "Mutation: cgroup limits" section
   above for the full before/after.
2. **Retire or relabel the source-text "mutation audit" test in
   `security_regression.rs`.** Either replace
   `mutation_audit_peercred_guard_called_in_handler` with a real behavioral
   test that opens a socket with a mismatched peer UID and asserts the
   connection is rejected, or rename/re-document it clearly as a
   "call-site-exists" smoke check so it isn't mistaken for behavioral
   coverage of Security Invariant 7.
3. **Spot-audit the `is_ok()`/`is_err()`-only assertion population (138
   hits total) for the "test name promises a value check" pattern** found
   in `none.rs::noop_network_setup_returns_empty_string`. Not urgent (low
   blast radius on the sampled case) but likely has more instances given
   the count; a grep for test names containing `_returns_` or `_returns_a_`
   cross-referenced against assertion bodies would narrow the candidate
   list quickly.

Both mutation-phase working-tree diffs were confirmed empty via `git status
--porcelain` after each `git checkout --` — no mutation was left in place
or committed.
