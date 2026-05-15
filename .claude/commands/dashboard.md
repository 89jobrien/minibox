---
name: dashboard
description: >
  Show agent run history and benchmark results as Nu tables. Reads
  ~/.minibox/agent-runs.jsonl and bench/results/latest.json.
argument-hint: "[--agents] [--bench]"
---

# dashboard

Displays agent run history and the latest benchmark summary.

```nu
nu scripts/dashboard.nu           # show both agent history and bench results
nu scripts/dashboard.nu --agents  # agent run history only
nu scripts/dashboard.nu --bench   # benchmark results only
```

Data sources:
- Agent history: `~/.minibox/agent-runs.jsonl`
- Bench results: `bench/results/latest.json`
