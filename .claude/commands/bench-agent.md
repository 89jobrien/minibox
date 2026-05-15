---
name: bench-agent
description: >
  Benchmark analysis agent: report latest results, compare runs, detect
  regressions, clean up stale files, or trigger a new bench run.
argument-hint: "<report|compare|regress|cleanup|trigger> [options]"
agent: atelier:minion
allowed-tools: [Bash, Read, Write]
---

Subcommand-driven benchmark analysis. Parse subcommand and options from `$ARGUMENTS`.

All subcommands accept `--max-turns <n>` (default: 15) to cap agent iterations.

**Subcommands:**

`report`
- Read `bench/results/latest.json`
- Summarise per-suite timing and p95 latencies
- Display as table: suite | test | avg | p95 | iters

`compare [sha…]`
- Read named runs from `bench/results/bench.jsonl`
- Diff timing between the specified SHAs (or last 2 if none given)
- Highlight improvements (green) and regressions (red)

`regress [--threshold <pct>]`
- Scan `bench/results/bench.jsonl` for regressions exceeding threshold (default: 10%)
- List regressed tests with: test name | baseline | current | delta%

`cleanup [--dry-run]`
- List result files older than the 5 most recent runs
- Without `--dry-run`: delete them

`trigger [--suite <name>] [--vps]`
- Run `cargo bench` (optionally filtered to `--suite`)
- If `--vps`: run on remote minibox VPS via SSH
- After completion: run `report`

Suites: `codec`, `adapter`, `pull`, `run`, `exec`, `e2e`.
