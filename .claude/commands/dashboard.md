---
name: dashboard
description: >
  Show agent run history and benchmark results as Nu tables. Reads
  ~/.minibox/agent-runs.jsonl and bench/results/latest.json.
argument-hint: "[--agents] [--bench]"
allowed-tools: [Bash, Read]
---

Display agent run history and benchmark results.

Parse `$ARGUMENTS` for: `--agents`, `--bench`.

**Agent history** (shown by default or with `--agents`):

1. Read `~/.minibox/agent-runs.jsonl` (JSONL). If absent, print "no agent runs found".
2. Deduplicate by `run_id`: for each run_id keep the `complete` entry if present, else latest.
3. Display two tables:
   - **Agent Summary**: script | runs | avg_s | last_run | last_output_preview
   - **Recent Runs (last 20)**: time | script | status | duration | output_preview

**Bench results** (shown by default or with `--bench`):

1. Read `bench/results/latest.json`. If absent, print "no bench results".
2. Header: git SHA (first 8 chars) + hostname + timestamp
3. Table: suite | test | avg | p95 | iters
4. If `bench/results/bench.jsonl` exists, print run count and file size.
