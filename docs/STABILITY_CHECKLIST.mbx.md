# Stability Checklist

Gates and review prompts for pre-release validation, PR merges, and crate promotion. See
`docs/CRATE_TIERS.md` for the full crate stabilization policy.

Last updated: 2026-05-14

---

## Quick Reference: Mandatory Gates vs Advisory Items

**Mandatory gates** are hard blockers. A PR, release, or crate promotion cannot proceed until
every mandatory gate passes. CI enforces most gates automatically; the remainder require explicit
reviewer sign-off before merge is approved. There are no exceptions without a tracked issue and
an explicit maintainer override recorded in that issue.

**Advisory items** are review prompts. They represent best practices that are strongly encouraged
but context-dependent. A reviewer may approve a PR with an unmet advisory item provided the PR
description includes an "ADVISORY acknowledged" comment explaining the rationale or linking a
follow-up issue. Silently ignoring advisory items is not acceptable.

---

## Legend

| Tag            | Meaning                                                                               |
| -------------- | ------------------------------------------------------------------------------------- |
| **[GATE]**     | Mandatory. CI enforces this automatically or a reviewer must verify it before merge.  |
|                | A failing gate **blocks promotion and release**.                                      |
| **[ADVISORY]** | Review prompt. Context-dependent. Does not block merge, but **must be acknowledged**  |
|                | with a rationale comment in the PR when the item is not met.                          |

---

## Mandatory Gates

Each gate lists the exact `cargo xtask` command that enforces it and the CI job that runs it.

### Gate 1 — Code Format

| Field   | Value                                                                          |
| ------- | ------------------------------------------------------------------------------ |
| **Tag** | [GATE]                                                                         |
| **Why** | Consistent formatting is required for readable diffs and automated tooling.    |
| **Cmd** | `cargo xtask pre-commit` (runs `cargo fmt --check` as part of the gate)        |
| **CI**  | `pr.yml` — `pre-commit` job                                                    |

A PR with unformatted code will fail the pre-commit job and cannot merge.

---

### Gate 2 — Clippy (warnings denied)

| Field   | Value                                                                          |
| ------- | ------------------------------------------------------------------------------ |
| **Tag** | [GATE]                                                                         |
| **Why** | Clippy with `-D warnings` catches real bugs and enforces idiom consistency.    |
| **Cmd** | `cargo xtask pre-commit` (runs `cargo clippy --workspace -- -D warnings`)      |
| **CI**  | `pr.yml` — `pre-commit` job                                                    |

Any new clippy warning introduced by a PR fails the gate. Fix it; do not suppress with
`#[allow]` without a justification comment.

---

### Gate 3 — Nextest (unit and conformance)

| Field   | Value                                                                          |
| ------- | ------------------------------------------------------------------------------ |
| **Tag** | [GATE]                                                                         |
| **Why** | All cross-platform unit and conformance tests must pass on every PR.           |
| **Cmd** | `cargo xtask test-unit`                                                        |
| **CI**  | `pr.yml` — `test-unit` job                                                     |

Run `cargo xtask prepush` for the full nextest suite (requires Linux for some tests).

---

### Gate 4 — Coverage check >= 80%

| Field   | Value                                                                          |
| ------- | ------------------------------------------------------------------------------ |
| **Tag** | [GATE]                                                                         |
| **Why** | Handler function coverage must not regress below 80% (`handler.rs`).          |
| **Cmd** | `cargo xtask coverage-check`                                                   |
| **CI**  | `merge.yml` — `coverage` job (Linux runner, llvm-cov)                         |

This gate runs on the `next` branch merge job, not on every PR, to avoid requiring a Linux
runner for every contributor. PRs that touch handler code should be self-verified locally
before merge.

---

### Gate 5 — Protocol drift check

| Field   | Value                                                                               |
| ------- | ----------------------------------------------------------------------------------- |
| **Tag** | [GATE]                                                                              |
| **Why** | Detects unintended changes to the wire protocol (`DaemonRequest`/`DaemonResponse`). |
| **Cmd** | `cargo xtask check-protocol-drift`                                                  |
| **Cmd** | `cargo xtask check-protocol-drift --update` — update baseline after intentional     |
|         | protocol changes (requires a companion protocol changelog entry)                    |
| **CI**  | `pr.yml` — `protocol-drift` job                                                     |

When the drift check fails, either the change is unintentional (revert it) or intentional
(update the baseline, add a changelog entry, and get protocol-owner review).

---

### Gate 6 — Stale name check

| Field   | Value                                                                               |
| ------- | ----------------------------------------------------------------------------------- |
| **Tag** | [GATE]                                                                              |
| **Why** | Prevents re-introduction of banned old crate/binary names (`linuxbox`, `mbx`, etc). |
| **Cmd** | `cargo xtask check-stale-names`                                                     |
| **CI**  | `pr.yml` — `stale-names` job                                                        |

The banned name list lives in `xtask/src/stale_names.rs`. Add new banned names there when
performing renames.

---

### Gate 7 — Protocol construction sites check

| Field   | Value                                                                               |
| ------- | ----------------------------------------------------------------------------------- |
| **Tag** | [GATE]                                                                              |
| **Why** | Verifies that the expected number of `HandlerDependencies` construction sites exist  |
|         | in `miniboxd/src/main.rs` so new adapters cannot silently omit required wiring.     |
| **Cmd** | `cargo xtask check-protocol-sites`                                                  |
| **Cmd** | `cargo xtask check-protocol-sites --expected N` — assert a specific site count      |
| **CI**  | `pr.yml` — `protocol-sites` job                                                     |

When adding a new adapter, update `--expected` in the CI job to the new count.

---

## Advisory Items

Advisory items do not block merge but must be acknowledged when not met.

| #  | Item                                                             | Enforcement                        |
| -- | ---------------------------------------------------------------- | ---------------------------------- |
| A1 | Docs lint passes (`cargo xtask lint-docs`)                       | `cargo xtask lint-docs`            |
| A2 | No `.unwrap()` in new production code (`cargo xtask check-no-unwrap`) | `cargo xtask check-no-unwrap` |
| A3 | Working tree is clean of generated artifacts (`cargo xtask check-repo-clean`) | `cargo xtask check-repo-clean` |
| A4 | Borrow-reasoning fixtures pass (`cargo xtask borrow-fixtures`)   | `cargo xtask borrow-fixtures`      |
| A5 | New domain trait has an in-memory mock double in tests           | Reviewer sign-off                  |
| A6 | Tracing events use structured fields, not message strings        | Reviewer sign-off                  |
| A7 | New `unsafe` blocks include a SAFETY comment                     | Reviewer sign-off                  |

### Running advisory checks locally

```bash
# A1: docs lint
cargo xtask lint-docs

# A2: unwrap scan (advisory by default; --strict makes it fatal)
cargo xtask check-no-unwrap

# A3: repo cleanliness
cargo xtask check-repo-clean

# A4: borrow-reasoning fixtures
cargo xtask borrow-fixtures
```

---

## How to Add a New Mandatory Gate

Follow these steps to wire a new check into xtask and CI so it enforces uniformly.

### Step 1 — Implement the check in xtask

Add a new source file under `xtask/src/` (e.g., `xtask/src/my_check.rs`) that implements the
check logic and returns `anyhow::Result<()>`. Register it in `xtask/src/main.rs`:

```rust
// In xtask/src/main.rs
mod my_check;

// In the match arm:
Some("check-my-feature") => my_check::run(root),
```

Update the help text block in `main.rs` to document the new subcommand.

### Step 2 — Add the gate to `gates.rs` (if part of a compound gate)

If the check should run as part of `cargo xtask pre-commit` or `cargo xtask prepush`, add a
call inside the relevant function in `xtask/src/gates.rs`:

```rust
pub fn pre_commit(sh: &Shell) -> Result<()> {
    // ... existing gates ...
    my_check::run(root)?;
    Ok(())
}
```

### Step 3 — Add a CI job in `.github/workflows/pr.yml`

Add a new job that runs the check on every PR:

```yaml
my-feature-check:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - run: cargo xtask check-my-feature
```

If the check is Linux-only, keep `runs-on: ubuntu-latest`. If macOS-safe, it can run on
`macos-latest` as well.

### Step 4 — Add it to this checklist

Add a new entry to the Mandatory Gates section above with:
- The [GATE] tag
- Why it matters
- The exact `cargo xtask` command
- Which CI job enforces it

### Step 5 — Update `DEVELOPMENT.md`

Add the new command to the "CI Gates" section of `DEVELOPMENT.md` so contributors know to run
it locally before pushing.

---

## Freeze Status

The stabilization freeze (issues #117 and #127) applies to net-new Core and Platform crates.
The freeze lifts when all mandatory gates above are verified green on the `next` branch.

Gate 4 (handler coverage >= 80%) is the primary remaining blocker. See issue #158 for tracking.
