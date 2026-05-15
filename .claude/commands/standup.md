---
name: standup
description: >
  Generate a time-blocked standup report from git activity across ~/dev/ repos.
  Use at the start of the day or to summarise recent work.
argument-hint: "[--hours <n>] [--vault <path>] [--no-sessions]"
agent: atelier:herald
allowed-tools: [Bash, Read, Write]
---

Scan `~/dev/` repos for git activity and produce a time-blocked standup report.

Parse `$ARGUMENTS` for: `--hours <n>` (default: 24), `--vault <path>`, `--no-sessions`.

**Steps:**

1. For each repo under `~/dev/`, run:
   `git -C <repo> log --since="<n> hours ago" --oneline --format="%h %ai %s" 2>/dev/null`
2. Group commits by repo, then by inferred work block (contiguous commits within 30 min)
3. For each work block, identify: what changed, which crates/modules, likely intent
4. Produce a time-ordered standup report:
   - Header: date range covered
   - Per-repo sections with bullet points per work block
   - Timestamp each block

Unless `--no-sessions`, also scan recent Claude session logs for additional context.

If `--vault <path>` provided: write the report as a daily note to
`<vault>/Daily/<YYYY-MM-DD>.md` (append if file exists, create if not).

Output format: markdown, suitable for pasting into Slack or a standup tool.
