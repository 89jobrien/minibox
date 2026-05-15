---
name: parse-geiger
description: >
  Parse a cargo-geiger report into structured Nu records. Filter by unsafe
  status, sort by expression count, or export to JSON.
argument-hint: "<report-file>"
---

# parse-geiger

Parses a `cargo geiger` text report into structured records.

```nu
nu scripts/parse-geiger.nu geiger-report.txt
nu scripts/parse-geiger.nu geiger-report.txt | to json
nu scripts/parse-geiger.nu geiger-report.txt | where status == "unsafe" | sort-by expressions_used -r
nu scripts/parse-geiger.nu geiger-report.txt | where status == "unsafe" | select name version expressions_used
```

Generate a report with:
```sh
cargo geiger 2>/dev/null > geiger-report.txt
```

Output columns: `name`, `version`, `status`, `functions_used/total`,
`expressions_used/total`, `impls_used/total`, `traits_used/total`,
`methods_used/total`.

Status values: `unsafe`, `forbids_unsafe`, `no_forbid`.
