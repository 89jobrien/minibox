# Personalized Usage Profile: Joe

### At a Glance

| Metric                          | Value                                |
| ------------------------------- | ------------------------------------ |
| Account created                 | 2025-11-23                           |
| First Claude Code token         | 2026-01-12                           |
| First startup (current install) | 2026-03-12                           |
| Total startups                  | **916** (~20/day over 45 days)       |
| Prompt queue uses               | **2,358** (~2.6 prompts per startup) |
| Current version                 | 2.1.74                               |
| Billing                         | Stripe subscription (admin)          |
| Verbose mode                    | Enabled                              |

### Workspace Footprint

- **44 project directories** tracked (34 under `~/dev/`)
- **28 GitHub repos** linked (26 personal `89jobrien/*`, 2 employer `toptal/*`)
- Primary language: **Rust** across ~25 crates/workspaces
- Secondary: Go (maestro), Nushell (scripts/hooks)

### MCP Integrations

Connected to 5 cloud MCP servers: Gmail, Google Calendar, Linear, Notion, Google Drive. Per CLAUDE.md, these are **never called unless explicitly asked** -- local tools are preferred.

### Skill Ecosystem

**180 registered skills** -- an extraordinarily deep customization layer. Includes custom plugins (atelier, sanctum, orca-strait, hand), superpowers skills, and project-specific workflows. Skill counts all show 0, suggesting usage is tracked elsewhere or the counter was reset.

### Feature Flags / Growth Config

265 `tengu_*` feature flags cached. Notable enabled features:

- `tengu_streaming_tool_execution2` -- streaming tool calls
- `tengu_ultraplan_config` -- plan mode
- `tengu_code_diff_cli` -- diff display
- `tengu_review_bughunter_config` -- bug hunter fleet (5 agents, Opus 4.7, $5-20/run)
- `tengu_cobalt_compass`, `tengu_harbor`, `tengu_birch_compass` -- various UI/UX flags

### Usage Pattern

- **Power user**: 916 startups, 2,358 prompt queue uses, verbose mode, custom hooks, custom plugins, and a 3-layer CLAUDE.md hierarchy (global + workspace + per-project)
- **Hook-heavy workflow**: RTK token rewriting, course-correction blocking, cargo fmt/check auto-runs, secret redaction, memory sync to Obsidian
- **Multi-agent oriented**: subagent guardrails, worktree isolation, orca-strait TDD agents, parallel dispatch patterns
- **Rejected 5 API keys** (custom key attempts that didn't stick)
- **Plan mode active**: last used recently (timestamp in data)

---

## JSON Schema of `~/.claude.json`

Moved to @`../rootDotClaudeDotJsonSchema.json`
