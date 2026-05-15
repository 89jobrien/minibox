---
name: bench-agent
description: >
  Benchmark analysis agent: report latest results, compare runs, detect
  regressions, clean up stale files, or trigger a new bench run.
argument-hint: "<report|compare|regress|cleanup|trigger> [options]"
---

# bench-agent

AI-assisted benchmark analysis with subcommands.

```nu
nu scripts/bench-agent.nu report                          # summarise latest results
nu scripts/bench-agent.nu compare abc1234 def5678         # compare two SHAs
nu scripts/bench-agent.nu regress                         # detect regressions (default 10% threshold)
nu scripts/bench-agent.nu regress --threshold 5.0         # tighter threshold
nu scripts/bench-agent.nu cleanup --dry-run               # list stale files without deleting
nu scripts/bench-agent.nu trigger                         # run benchmarks + analyse
nu scripts/bench-agent.nu trigger --suite codec --vps     # specific suite on VPS
```

Suites: `codec`, `adapter`, `pull`, `run`, `exec`, `e2e`.

All subcommands accept `--max-turns <n>` (default: 15) to cap agent iterations.
