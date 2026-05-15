---
name: standup
description: >
  Generate a time-blocked standup report from git activity across ~/dev/ repos.
  Use at the start of the day or to summarise recent work.
argument-hint: "[--hours <n>] [--vault <path>] [--no-sessions]"
---

# standup

Scans git logs across repos and produces a standup-style activity report.

```nu
nu scripts/standup.nu                          # last 24h, all repos
nu scripts/standup.nu --hours 48               # last 48h
nu scripts/standup.nu --vault "/path/to/vault" # write report to Obsidian vault
nu scripts/standup.nu --no-sessions            # skip Claude session log analysis
```
