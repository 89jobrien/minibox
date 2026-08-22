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
test name claims the return value is checked ("returns_empty_string") but
the body only asserts `.is_ok()`, never asserts the returned string actually
_is_ empty. That's a real gap: a regression that made `NoopNetwork::setup`
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

(pending)

## Mutation: cgroup limits

(pending)

## Mutation: protocol terminal-response classification

(pending)

## Overall verdict

(pending)
