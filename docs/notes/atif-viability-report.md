# ATIF Viability Report: Minibox Agent Trajectory Logging

> **STALE:** All implementation targets below reference removed crates
> (`minibox-agent`, `minibox-llm`). Update targets before acting on
> this report.

| Field       | Value         |
| :---------- | :------------ |
| **Status**  | Draft         |
| **Date**    | 2026-04-23    |
| **RFC Ref** | ATIF v1.6     |
| **Scope**   | minibox-agent |

---

## Summary

This report evaluates the viability of adopting the Agent Trajectory Interchange Format (ATIF v1.6)
as the standard trajectory logging format for minibox's agent runtime (`minibox-agent` crate and
`agentbox` Go tooling). The format is well-suited to minibox's use case with minor gaps noted below.

---

## What ATIF Provides

ATIF is a JSON-based schema for logging complete LLM agent interaction histories. It captures:

- **Dialogue turns** — system prompts, user messages, agent responses
- **Tool calls** — structured function invocations with arguments and correlation IDs
- **Observations** — environment feedback keyed to tool call IDs
- **Metrics** — per-step token counts, costs, logprobs, and optional token ID arrays for RL
- **Multi-agent support** — subagent trajectory references for hierarchical agent architectures
- **Multimodal content** — image references via relative paths (v1.6+)

The format explicitly targets three downstream consumers: debugging/visualization, Supervised
Fine-Tuning (SFT), and Reinforcement Learning (RL).

---

## Fit Assessment

### Strong alignment

| Minibox concern                          | ATIF support                                                              |
| :--------------------------------------- | :------------------------------------------------------------------------ |
| Multi-step agentic tasks                 | Sequential `steps` array with `step_id` ordering                          |
| Tool-calling agents (crux/minibox-agent) | `tool_calls` + `observation.results` with `source_call_id` correlation    |
| Parallel tool calls                      | Multiple entries in `tool_calls` per step; native pattern in spec         |
| Subagent delegation (meta-agent)         | `SubagentTrajectoryRef` with `session_id` + optional `trajectory_path`    |
| Cost tracking                            | `cost_usd` per step, `total_cost_usd` in `final_metrics`                  |
| RL training pipeline                     | `completion_token_ids`, `logprobs`, `prompt_token_ids` in `MetricsSchema` |
| Anthropic-specific extras                | `metrics.extra` for `cache_creation_input_tokens` etc.                    |

### Gaps and considerations

1. **No Rust reference implementation.** The reference implementation is Python/Pydantic only.
   Minibox would need to implement ATIF serialization in Rust (`minibox-agent`) and Go
   (`agentbox`). This is straightforward — the schema is pure JSON — but not zero effort.

2. **No streaming step support.** ATIF captures steps as complete objects. Minibox's streaming
   protocol emits `ContainerOutput` tokens incrementally. Trajectories must be assembled after
   completion, not written token-by-token. This is consistent with current agent practice but
   requires buffering during execution.

3. **No native error/abort step type.** `source` only allows `"system"`, `"user"`, or `"agent"`.
   Agent failures or aborts can be expressed in `observation.results[].content` or `extra` but
   are not first-class. Consider encoding errors in `step.extra` with a convention like
   `{"abort": true, "reason": "..."}`.

4. **Subagent ref is file/URL-based.** `trajectory_path` is a string reference (file path or S3
   URL). For in-memory or database-backed trajectory stores, the path convention needs a
   documented scheme. Recommend using `minibox://session/<session_id>` as an internal URI scheme.

5. **Image storage is by convention, not enforcement.** The `images/` subdirectory convention
   is advisory; no schema field enforces co-location. For minibox agent runs that capture
   terminal screenshots or diff visualizations, agree on a storage layout up front.

---

## Integration Points

### `minibox-agent` (Rust)

The agent runtime should:

1. Construct a `Trajectory` root object at session start with a UUID `session_id`.
2. Append a `StepObject` for each agent turn — user messages, LLM responses, tool invocations,
   and observations.
3. Populate `MetricsSchema` from the `minibox-llm` response metadata (token counts, cost, logprobs
   if available).
4. Write completed trajectories to `~/.minibox/trajectories/<session_id>.json` on task completion.
5. Reference the path convention in `agent-runs.jsonl` so the dashboard can link to trajectories.

### `agentbox` (Go)

The council and meta-agent binaries should:

1. Emit ATIF-compatible trajectories for each agent role (council) or subagent (meta-agent).
2. Use `SubagentTrajectoryRef` in the orchestrator trajectory to link child trajectories.
3. Write to the same `~/.minibox/trajectories/` directory.

### `dashbox` TUI

The Agents tab can be extended to:

1. Load trajectory JSON files from `~/.minibox/trajectories/`.
2. Display step-level breakdowns (token counts, tool call summaries, cost).
3. Link to subagent trajectories for hierarchical drill-down.

---

## Implementation Plan

### Phase 1 — Rust types (minibox-agent)

- Define `Trajectory`, `StepObject`, `ToolCall`, `Observation`, `Metrics`, `FinalMetrics` structs
  in `crates/minibox-agent/src/trajectory.rs`.
- Implement `serde::Serialize`/`Deserialize` for JSON I/O.
- Wire trajectory construction into the agent execution loop.
- Write to `~/.minibox/trajectories/` on completion.

Estimated effort: **2–3 days**.

### Phase 2 — Go types (agentbox)

- Define equivalent structs in `agentbox/internal/atif/`.
- Integrate into council and meta-agent output.
- Emit `SubagentTrajectoryRef` entries from the orchestrator.

Estimated effort: **1–2 days**.

### Phase 3 — Dashboard integration

- Extend dashbox Agents tab to load and render trajectory files.
- Add step-level token/cost breakdown view.

Estimated effort: **1–2 days**.

---

## Verdict

**Adopt ATIF.** The format is well-designed, covers minibox's agent use cases, and is explicitly
built for the SFT/RL pipelines that minibox's agent layer is targeting. The gaps (no Rust impl,
no streaming steps, weak error typing) are implementation concerns — not design flaws — and are
all resolvable within the existing schema using `extra` fields and agreed conventions.

Priority: Phase 1 (Rust types) should be included in the next agent runtime milestone.

---

## Open Questions

1. Should minibox use `minibox://session/<id>` as the URI scheme for `trajectory_path`, or
   plain relative file paths?
2. Should `agent.name` be `"minibox-agent"` for all Rust-side trajectories, or encode the
   adapter suite (e.g., `"minibox-agent/native"`)?
3. Do we need a trajectory retention policy (auto-prune after N days) in the daemon or xtask?
