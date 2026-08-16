# Stability Checklist

Gates and review prompts for adding new Core or Platform crates, or promoting an Experimental
crate. See `docs/core/SUPPORT_TIERS.mbx.md` for the full support-tier definitions and promotion
criteria.

Last updated: 2026-08-16

---

## Quick Reference: Gates vs Advisory

**[GATE] items are hard blockers.** A PR or crate promotion cannot merge until every [GATE] item
passes. CI enforces most gates automatically; the remainder require explicit reviewer sign-off in
the PR before merge is approved. There are no exceptions without a tracked issue and explicit
maintainer override.

**[ADVISORY] items are review prompts.** They represent best practices that are strongly
encouraged but context-dependent. A reviewer may approve a PR with an unmet [ADVISORY] item
provided the PR description includes an "ADVISORY acknowledged" comment explaining the rationale
or linking a follow-up issue. Silently ignoring advisory items is not acceptable.

---

## Legend

| Tag              | Meaning                                                                            |
| ---------------- | ---------------------------------------------------------------------------------- |
| **[GATE]**       | Mandatory merge gate. CI enforces this automatically or a reviewer must verify     |
|                  | it explicitly before merging. A failing GATE item **blocks promotion**.            |
| **[ADVISORY]**   | Review prompt. Best-effort or context-dependent. Failing an ADVISORY item does     |
|                  | not block merge, but **must be acknowledged** with a rationale comment in the PR.  |

---

## Mandatory Gates

These block promotion/merge. All six must be green simultaneously on the promotion path (`develop` -> `staging` -> `release`).

| #   | Item                                                        | Status  | Evidence                                               |
| --- | ------------------------------------------------------------ | ------- | ------------------------------------------------------- |
| 1   | Protocol types have a single source of truth                | Met     | `crates/minibox-core/src/protocol.rs` (#122/#128)        |
| 2   | Handler coverage >= 80% function coverage                   | Met     | 92.41% (207/224 functions, 2026-08-10)                   |
| 3   | All wired adapters have at least one integration test       | Met     | native, gke, colima, smolvm, krun all tested             |
| 4   | `cargo xtask pre-commit` passes on macOS                    | Met     | staged fmt/clippy + config/docs checks                   |
| 5   | `cargo xtask test-unit` passes                               | Met     | ~506 tests (macOS cross-platform subset)                 |
| 6   | `cargo deny check` passes                                    | Met     | License + advisory audit in CI                           |

## Advisory Items

These are review prompts, not merge blockers. Failing one must be acknowledged in the PR
description (see "ADVISORY acknowledged" note above), not silently ignored.

| #   | Item                                                        | Evidence                            |
| --- | ------------------------------------------------------------ | ------------------------------------ |
| A1  | New domain trait has an in-memory mock double in tests       | Required for hexagonal port compliance |
| A2  | No `.unwrap()` in production paths of new code                | See rust-patterns.md rule 1          |
| A3  | Tracing events use structured fields, not message strings     | See rust-patterns.md tracing rules   |
| A4  | New `unsafe` blocks include a SAFETY comment                   | See rust-patterns.md rule 6          |

---

## How to Verify

### [GATE] items

```bash
# Gate 1: protocol snapshot tests
cargo test -p minibox-core -- protocol

# Gate 2: handler coverage (requires Linux + llvm-cov)
cargo xtask coverage-check

# Gate 3: adapter integration tests
just test-integration  # Linux + root
just test-adapters     # Colima + handler adapter swap

# Gate 4: pre-commit gate
cargo xtask pre-commit

# Gate 5: unit test suite
cargo xtask test-unit

# Gate 6: deny + audit
cargo deny check
cargo audit
```

### Advisory items

Advisory items (A1-A4) are checked during PR review, not by CI. Reviewers annotate with
"ADVISORY: acknowledged — \<rationale\>" when a prompt does not apply or is deferred with a
tracked follow-up issue.

---

## Freeze Status

A narrow Core/Platform freeze applies until every mandatory gate is verified green
on the promotion path. It freezes new public `minibox-core` protocol/domain API,
native-platform capability expansion, and newly wired adapters.

Bug and security fixes, coverage work, documentation, compatibility-safe refactors,
and isolated Tier 2/3 experiments remain permitted. An exception requires explicit
maintainer approval and a tracking issue; it must state why the work cannot remain
isolated from frozen Tier 1 contracts.

The freeze lifts only after the 80% handler-coverage gate, promotion-path CI, Linux
integration/e2e evidence, and supporting documentation are simultaneously current.
Issue #127 records the final lift decision.

---

## CI Enforcement

The following xtask gates are enforced in GitHub Actions (`stability-gates.yml` and
`protocol-drift.yml`):

| Gate                       | CI Job                        | Workflow                |
| -------------------------- | ----------------------------- | ----------------------- |
| coverage-check             | handler coverage gate (>=80%) | stability-gates.yml     |
| check-protocol-drift       | core contract hash check      | protocol-drift.yml      |
| check-stale-names          | stale crate/binary name audit | stability-gates.yml     |
| check-protocol-sites       | HandlerDependencies site count| stability-gates.yml     |

Gates 1-6 in the table above are enforced via `cargo xtask pre-commit` locally and the jobs
listed here in CI. All four xtask-based gates were added under issue #133.
