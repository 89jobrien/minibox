---
name: collect-metrics
description: >
  Run cargo tests across workspace crates and display results as Nu tables.
  Optionally saves a timestamped JSONL run record. Also shows bench summary.
argument-hint: "[--save] [--crates <list>] [--reports-dir <path>]"
---

# collect-metrics

Runs tests for each workspace crate, renders a pass/fail table, and optionally
saves a timestamped run record.

```nu
nu scripts/collect-metrics.nu                          # run all default crates
nu scripts/collect-metrics.nu --save                   # save run record to artifacts/reports/
nu scripts/collect-metrics.nu --crates minibox,miniboxd # specific crates only
nu scripts/collect-metrics.nu --save --reports-dir /tmp/runs
```

Default crates: `minibox`, `minibox-macros`, `mbx`, `miniboxd`.

Exits non-zero if any test fails.
