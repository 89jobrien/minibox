---
name: whatidid
description: >
  Generate a daily activity report from Claude Code session data. Harvests
  human/assistant turns from ~/.claude/projects/*/*.jsonl, analyzes them with
  claude-haiku-4-5 via the Anthropic API, and renders an HTML report with KPI
  cards, goals table, and effort/leverage breakdown.
argument-hint: "[YYYY-MM-DD]"
---

# What Did I Do — Daily Activity Report

## Overview

Produces a structured "what did I do today" report by reading Claude Code session
JSONL files, extracting user instructions and tool summaries, calling the Anthropic
API for goal/task analysis, and rendering an HTML report with KPI cards, goals table,
and token cost breakdown.

The report answers: what goals were pursued, what tasks were completed, how much time
was spent, and what the ROI was relative to the seat cost.

## When to Use

- At the end of a workday to generate a standup/status report
- When you need an objective log of what was worked on for a given date
- When asked to "report on today", "what did I do", or given a date argument

**Not for:** Real-time session introspection, cross-repo analysis beyond git metadata,
or project planning.

## Helpers & References

| File                                       | Purpose                                                       |
| ------------------------------------------ | ------------------------------------------------------------- |
| `helpers/whatidid.rs`                      | Entrypoint — orchestrates harvest → analyze → report          |
| `helpers/harvest.rs`                       | Scans `~/.claude/projects/*/*.jsonl` for today's sessions     |
| `helpers/analyze.rs`                       | Calls Anthropic API; caches result to `cache/YYYY-MM-DD.json` |
| `helpers/report.rs`                        | Renders HTML with KPI cards and goals table; opens in browser |
| `references/architecture.whatidid.md`      | Data flow, session format, token cost model                   |
| `references/effort-estimation.whatidid.md` | Effort estimation heuristics                                  |

---

## Steps

### 1. Resolve the target date

Use `$ARGUMENTS` if provided (format: `YYYY-MM-DD`). Otherwise default to today.

### 2. Harvest session data

Run `helpers/harvest.rs [YYYY-MM-DD]` via `rust-script`.

Scans all `~/.claude/projects/<project-slug>/*.jsonl` files. Each JSONL line has a
`timestamp` field (ISO 8601). Events with a matching date prefix are included.

Key event types extracted:

- `type: "human"` → `message.content[].text` (user instructions; noise-filtered)
- `type: "assistant"` → `message.content[]` text blocks + `tool_use` names
- Metadata from every line: `cwd`, `gitBranch`, `sessionId`, `slug`

Noise filtered from human turns: single-word confirmations ("ok", "yes", "act"),
`<system-reminder>` injections, `<current_datetime>` tags.

Output: JSON array of session records with `messages[]`, `tool_calls`, `read_calls`,
`edit_calls`, `bash_calls`.

### 3. Analyze with Anthropic API

Run `helpers/analyze.rs <sessions.json> [YYYY-MM-DD]` via `rust-script`.

Requires `ANTHROPIC_API_KEY` in environment. Inject via:

```nu
op run --env-file=$HOME/dev/.env -- rust-script helpers/whatidid.rs
```

Builds a transcript from harvested sessions (cwd, branch, SIGNALS block with tool
counts, then interleaved human/assistant turns). Sends to `claude-haiku-4-5-20251001`.

Model prompt lives in `prompts/analysis.whatidid.txt`. Returns structured JSON digest:
`goals[]` each with `tasks[]`, `human_hours`, `domain_skills`, `tech_skills`.

Caches result to `cache/YYYY-MM-DD.json` — re-runs skip the API call.

### 4. Compute KPIs

```text
total_human_hours  = sum(goal.human_hours for goal in goals)
human_value        = total_human_hours × 72          # $72/hr blended rate
seat_cost_per_mo   = 39                              # Claude Code Pro seat
leverage           = human_value / seat_cost_per_mo
```

Token cost model: prefix-matched against `model_pricing.json`.
Fallback: $3.00 input / $15.00 output per 1M tokens.

### 5. Generate HTML report

Run `helpers/report.rs <digest.json> [YYYY-MM-DD]` via `rust-script`.

Layout:

```text
Header bar   — date, primary_focus, session count
Narrative    — day_narrative (2 sentences from digest)
KPI cards    — Hours | Human value | Leverage
Goals table  — Goal | Tasks (bullet list) | Hours | Skills
```

Written to `/tmp/whatidid-YYYY-MM-DD.html`. Opened automatically via `open`.

### 6. Report to user

```text
[1/3] harvesting sessions for 2026-05-17...
[2/3] analyzing with Claude...
[3/3] rendering report...
Date: 2026-05-17  |  Goals: 5  |  Hours: 6.5h
Human value: $468  |  Leverage: 12x
Report: /tmp/whatidid-2026-05-17.html
```

---

## Token Cost Model

`model_pricing.json` is the canonical prefix-to-rate table. Unknown models use
its `fallback` input/output rates; do not duplicate rates in this document.

Update `model_pricing.json` when rates change.

## Common Mistakes

- **Including injected context as user instructions** — filter `<current_datetime>`,
  `<system-reminder>`, and single-word approval messages ("ok", "yes", "go") before
  building the transcript.
- **Double-counting sessions** — a single working session may span midnight; use
  `created_at` (session start) not `updated_at` as the date key.
- **Calling the API without a cached result check** — always check
  `cache/YYYY-MM-DD.json` before hitting the Anthropic API; the API has rate limits.
- **Hardcoding model names** — add prefix entries to `model_pricing.json` so new model
  variants use explicit rates instead of the fallback.
