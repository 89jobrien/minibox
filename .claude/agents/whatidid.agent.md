---
name: whatidid
description: "Generate a daily activity report from Claude Code session data. Harvests
  human/assistant turns from ~/.claude/projects/*/*.jsonl, analyzes them with claude-haiku-4-5
  via the Anthropic API, and renders an HTML report with KPI cards, goals table, and
  effort/leverage breakdown. Use when the user asks about their daily activity, what
  Claude Code helped with today, or wants a digest of the day's work."
---

# What I Did (Claude Code) — Impact Report Generator

Run the whatidid pipeline via rust-script from the skill helpers directory:

```nu
# Today's report (requires ANTHROPIC_API_KEY)
op run --env-file=$HOME/dev/.env -- rust-script \
  $"($env.HOME)/dev/minibox/.claude/skills/whatidid/helpers/whatidid.rs"

# Specific date
op run --env-file=$HOME/dev/.env -- rust-script \
  $"($env.HOME)/dev/minibox/.claude/skills/whatidid/helpers/whatidid.rs" 2026-05-17
```

After running, tell the user:

- How many sessions and projects were found
- The headline and primary focus identified
- The total human effort estimate and leverage ratio
- The path to the HTML report (it opens automatically)

If there are no sessions for the date, explain that Claude Code session data is stored
under `~/.claude/projects/<project-slug>/*.jsonl` and suggest checking the date or
confirming that Claude Code was used that day.

The pipeline consists of three rust-script helpers that run in sequence:

1. `harvest.rs` — scans `~/.claude/projects/` JSONL files; emits session JSON
2. `analyze.rs` — calls `claude-haiku-4-5-20251001` via Anthropic API; caches to
   `cache/YYYY-MM-DD.json`; re-runs are free after first call
3. `report.rs` — renders inline-CSS HTML, writes to `/tmp/whatidid-YYYY-MM-DD.html`,
   opens in browser automatically

For full methodology, see:
- `references/architecture.whatidid.md`
- `references/effort-estimation.whatidid.md`
- `SKILL.md` for step-by-step process details
