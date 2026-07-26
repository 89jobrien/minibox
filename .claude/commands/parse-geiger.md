---
name: parse-geiger
description: >
  Parse a cargo-geiger report into structured Nu records. Filter by unsafe
  status, sort by expression count, or export to JSON.
argument-hint: "<report-file>"
allowed-tools: [Read]
---

Parse a cargo-geiger report into a structured table.

Usage: `/parse-geiger <report-file>`

The report file is the first argument in `$ARGUMENTS`. Error if not provided or not found.

Data lines match: `<fn> <expr> <impl> <traits> <methods> <status> <dep-tree>`
where each metric is `used/total`, status is `!` (unsafe), `:)` (forbids_unsafe), or `?` (no_forbid).

For each data line:
1. Strip ANSI codes
2. Parse the 5 `used/total` ratios and the status symbol
3. Extract crate name and version from the dep-tree column (last token = version, rest = name)
4. Output a row: name | version | status | fn_used | expr_used | expr_total

Display as a table. Suggest follow-on filters:
- `where status == "unsafe" | sort-by expr_used desc` for highest-risk crates
