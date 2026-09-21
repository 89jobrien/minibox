# Architecture

## Data flow

```text
~/.claude/projects/<project-slug>/*.jsonl
           |
           v
       harvest.rs
  - Selects sessions by their first timestamp
  - Extracts human/assistant turns and tool names
  - Captures cwd, branch, slug, and model token metrics
           |
           v
       analyze.rs
  - Builds a structured transcript
  - Calls the Anthropic Messages API with claude-haiku-4-5
  - Returns goals, tasks, skills, hours, and narrative JSON
  - Caches cache/YYYY-MM-DD.json
           |
           v
       report.rs
  - Loads model_pricing.json
  - Renders the HTML KPI and goals report
```

## Session file format

Claude Code stores JSONL sessions under `~/.claude/projects/<project-slug>/`.
The harvester reads human and assistant message events, tool-use blocks, session
metadata (`cwd`, `gitBranch`, `slug`, timestamps), and per-model token usage.

## Token cost model

Harvested model metrics retain model name plus input, output, and cache-read tokens,
so prefix-based pricing can be applied automatically.

Pricing is defined in `model_pricing.json` with prefix-matched model names and
explicit fallback rates. Update that JSON file when rates change; `report.rs`
loads it rather than maintaining a second table.

## Leverage metric

```text
human_value    = total_human_hours × HOURLY_RATE   ($72/hr blended rate)
seat_cost/mo   = $39/mo enterprise plan
leverage       = human_value / seat_cost_per_month
```

Example: 29h × $72 = $2,088 human value ÷ $39/mo seat = **54×**

This measures estimated return on the configured coding-agent seat cost per day used.

## Anthropic API

- Endpoint: `https://api.anthropic.com/v1/messages`
- Auth: `ANTHROPIC_API_KEY`
- Model: `claude-haiku-4-5-20251001`
