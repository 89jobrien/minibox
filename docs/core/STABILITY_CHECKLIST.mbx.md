---
source_sha: 9da04a4b3b8fdc49254c873302d344de579e0375
sources:
  - .github/workflows
  - xtask/src/main.rs
  - CONTRIBUTING.md
generated: 2026-08-22
---

# Stability Checklist

Gates and review prompts for adding new Core or Platform crates, or promoting an Experimental
crate. See `docs/core/SUPPORT_TIERS.mbx.md` for the full support-tier definitions and promotion
criteria.

Last updated: 2026-08-22

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

**Lifted 2026-08-18.** The net-new surface freeze declared 2026-05-14 under issue
#127 (`CONTRIBUTING.md` "Feature Freeze", commit b23575db) is lifted. Lift
evidence, per the conditions this section previously stated:

- All six mandatory gates green simultaneously on `develop`: Stability Gates,
  Conformance, and Merge workflows all passing on commits bc01b5f5 and a9940738
  (2026-08-18), including the protocol-drift gate after its lock regeneration.
- Handler coverage 92.41% against the 80% threshold (Gate 2).
- Linux integration/e2e evidence: conformance suite 123/123 on the pre-push gate
  and self-hosted Linux CI; `native_adapter_isolation_tests` plus the new
  `native_adapter_lifecycle_failure_tests` (#74) green on a Linux VM under
  root + cgroup v2 (2026-08-18).

Normal contribution rules resume. New crates and public surface follow the
Stabilization Policy in `docs/core/CRATE_TIERS.mbx.md` — the gate criteria are
now a standing promotion bar, not a freeze. Chain I issues are unblocked.

Issue #127 records the lift decision; the original freeze declaration is
preserved in git history and in `CONTRIBUTING.md`.

---

## CI Enforcement

The following jobs enforce checklist items in GitHub Actions (issue #133):

| Enforces  | CI Job                                        | Command                                        | Workflow            |
| --------- | --------------------------------------------- | ---------------------------------------------- | ------------------- |
| Gate 1    | protocol-drift (core contract hash check)     | `cargo xtask check-protocol-drift`             | protocol-drift.yml  |
| Gate 1    | check-protocol-sites                          | `cargo xtask check-protocol-sites`             | stability-gates.yml |
| Gate 2    | handler-coverage (>=80% function coverage)    | `cargo xtask coverage-check`                   | stability-gates.yml |
| Gate 3    | adapter-integration-tests (all five adapters) | `cargo xtask check adapter-coverage`           | stability-gates.yml |
| Gate 5    | test-unit                                     | `cargo xtask test unit`                        | pr.yml / merge.yml  |
| Gate 6    | deny + audit                                  | `cargo deny check` / `cargo audit`             | pr.yml / merge.yml  |
| A2        | no-unwrap-in-prod (enforced as hard job)      | `cargo xtask check-no-unwrap --strict`         | stability-gates.yml |
| doc sync  | doc-sync (docs audit + FEATURE_MATRIX age)    | `cargo xtask docs audit --full --strict`       | stability-gates.yml |
| doc names | check-stale-names                             | `cargo xtask check-stale-names`                | stability-gates.yml |
| compile   | stability-compile (check + clippy)            | `cargo check --workspace` + targeted clippy    | stability-gates.yml |

Known gaps (tracked, not yet CI-enforced):

- Gate 4 (`cargo xtask pre-commit` on macOS) has no CI job — all stability jobs run on
  `ubuntu-latest`. It remains a local gate.
- Gates 5 and 6 run in `pr.yml`/`merge.yml`, not in the `stability-gates.yml` fan-in, so the
  six gates are not verified green as a single unit.
- Advisory items A1, A3, and A4 are review-time only; A2 is the only automated advisory.
