# Plan: derived-doc-freshness-gate

## Goal

One sentence. What does this implement and why.

## Architecture

- Crates affected:
- New traits/types:
- Data flow: source → transform → sink

## Tech Stack

- Rust edition:
- New dependencies:

## Tasks

### Task 1: <name>

**Crate**: `<crate-name>`
**File(s)**: `crates/<crate>/src/<file>.rs`
**Run**: `cargo nextest run -p <crate>`

1. Write failing test and confirm FAIL.
2. Implement minimum code to pass. Confirm GREEN.
3. `cargo clippy -p <crate> -- -D warnings` — zero warnings.
4. `git commit -m "feat(<crate>): <summary>"`
