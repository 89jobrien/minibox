# Stability Checklist

Gates and review prompts for adding new Core or Platform crates, or promoting an Experimental
crate. See `docs/CRATE_TIERS.md` for the full stabilization policy.

Last updated: 2026-05-06

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

## Gates

| #   | Item                                                       | Tag        | Status  | Evidence                                   |
| --- | ---------------------------------------------------------- | ---------- | ------- | ------------------------------------------ |
| 1   | Protocol types have a single source of truth               | [GATE]     | Met     | `minibox-core/src/protocol.rs` (#122/#128) |
| 2   | Handler coverage >= 80% function coverage                  | [GATE]     | Not met | Current ~67.5% (`handler.rs`)              |
| 3   | All wired adapters have at least one integration test      | [GATE]     | Met     | native, gke, colima, smolvm, krun all tested |
| 4   | `cargo xtask pre-commit` passes on macOS                   | [GATE]     | Met     | fmt + clippy + release build               |
| 5   | `cargo xtask test-unit` passes                             | [GATE]     | Met     | ~506 tests (macOS cross-platform subset)   |
| 6   | `cargo deny check` passes                                  | [GATE]     | Met     | License + advisory audit in CI             |
| 7   | New domain trait has an in-memory mock double in tests     | [ADVISORY] | —       | Required for hexagonal port compliance     |
| 8   | No `.unwrap()` in production paths of new code             | [ADVISORY] | —       | See rust-patterns.md rule 1                |
| 9   | Tracing events use structured fields, not message strings  | [ADVISORY] | —       | See rust-patterns.md tracing rules         |
| 10  | New `unsafe` blocks include a SAFETY comment               | [ADVISORY] | —       | See rust-patterns.md rule 6                |

---

## How to Verify

### [GATE] items

```bash
# Gate 1: protocol snapshot tests
cargo test -p minibox-core -- protocol

# Gate 2: handler coverage (requires Linux + llvm-cov)
cargo xtask prepush  # generates coverage report

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

### [ADVISORY] items

Advisory items are checked during PR review. Reviewers annotate with "ADVISORY: acknowledged —
\<rationale\>" when a prompt does not apply or is deferred with a tracked follow-up issue.

---

## Freeze Status

The stabilization freeze (issues #117 and #127) applies to **net-new Core and Platform crates**.
The freeze lifts when all **[GATE]** items above are verified green on the `next` branch.

Gate 2 (handler coverage) is the primary remaining blocker. See
[GH #158](https://github.com/89jobrien/minibox/issues/158) for tracking.
---

## CI Enforcement

The following xtask gates are enforced in GitHub Actions (
and ):

| Gate                       | CI Job                        | Workflow                |
| -------------------------- | ----------------------------- | ----------------------- |
| coverage-check             | handler coverage gate (>=80%) | stability-gates.yml     |
| check-protocol-drift       | core contract hash check      | protocol-drift.yml      |
| check-stale-names          | stale crate/binary name audit | stability-gates.yml     |
| check-protocol-sites       | HandlerDependencies site count| stability-gates.yml     |

Gates 1-6 in the table above are enforced via pre-commit () locally and
the jobs listed here in CI. All four xtask-based gates (#133) were added in
.
