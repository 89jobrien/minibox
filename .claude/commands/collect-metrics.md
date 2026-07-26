---
name: collect-metrics
description: >
  Run cargo tests across workspace crates and display results as Nu tables.
  Optionally saves a timestamped JSONL run record. Also shows bench summary.
argument-hint: "[--save] [--crates <list>] [--reports-dir <path>]"
allowed-tools: [Bash, Write]
---

Run tests for each workspace crate and display a pass/fail table.

Parse `$ARGUMENTS` for: `--save`, `--crates <csv>`, `--reports-dir <path>`.

Default crates: `minibox`, `minibox-macros`, `mbx`, `miniboxd`.

For each crate:
1. Run: `cargo test -p <crate> --lib -- --format json -Z unstable-options 2>/dev/null`
2. Parse JSON event lines: collect `type=test` events, count `ok`/`failed`/`ignored`
3. Collect names of failed tests

Display a table: crate | total | ok | failed | ignored
Print totals row. List any failed test names.

If `bench/results/latest.json` exists, also display a bench summary table:
suite | test | avg | p95 | iters

If `--save`: write `{timestamp, git_sha, branch, results}` to
`<reports-dir>/<timestamp>/meta.json` (default reports-dir: `artifacts/reports`).

Exit non-zero if any test failed.
