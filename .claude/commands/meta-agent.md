---
name: meta-agent
description: >
  Design and spawn parallel Claude agents from a task description. Fetches and
  caches Claude Agent SDK docs (24h TTL), designs 2–5 agents, runs concurrently,
  and synthesises results.
argument-hint: "<task> [--no-synthesis] [--refresh-docs]"
agent: atelier:forge
allowed-tools: [Bash, Read, Agent]
---

Decompose a task into parallel sub-agents, run them, and synthesise results.

Task description comes from `$ARGUMENTS` or stdin. Parse flags:
`--no-synthesis`, `--refresh-docs`.

**Steps:**

1. If `--refresh-docs` or SDK docs cache is older than 24h: fetch Claude Agent SDK docs
   and cache to `~/.minibox/cache/agent-sdk-docs.md`
2. Read the task description
3. Design 2–5 independent sub-agents with distinct roles (e.g. research, implement,
   test, review, document) — choose the minimum needed, not the maximum
4. For each sub-agent, define: role, scope, inputs, expected output, allowed tools
5. Dispatch all sub-agents in parallel via the Agent tool
6. Collect results as they complete
7. Unless `--no-synthesis`: synthesise all results into a unified response that
   resolves conflicts and surfaces the most actionable findings

Flags:
- `--no-synthesis` — print each agent's output separately, skip final synthesis
- `--refresh-docs` — force re-fetch of Claude Agent SDK docs even if cache is fresh
