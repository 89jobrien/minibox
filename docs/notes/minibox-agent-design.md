# minibox-agent Design

> **ARCHIVED:** This document describes removed functionality. The
> `minibox-agent` and `minibox-llm` crates were removed during the
> consolidation in sessions 29-31 (2026-04-21 to 2026-04-26). See
> `docs/superpowers/plans/2026-04-28-minibox-agent-reland.md` for the
> current reland plan.

`feat/minibox-agent-error` implemented a structured agentic loop in `crates/minibox-agent/`.
It was reverted because it depended on a richer `minibox-llm` API (`InferenceRequest`,
`Message`, `ContentBlock`, `ToolDefinition`, `.infer()`) that was stripped down to the current
`CompletionRequest`/`complete()` interface before the branch was merged.

This document captures the design so the work can be ported forward when the LLM API is extended.

---

## What Was Implemented

### `AgentError` — domain error type (`src/error.rs`)

A typed error enum that wraps infrastructure errors without stringifying them:

```rust
pub enum AgentError {
    Llm(#[from] minibox_llm::LlmError),
    Step(#[from] cruxai_core::types::error::CruxErr),
    BudgetExceeded(String),
    Other(String),
}
```

Key design choice: source errors are preserved via `#[from]` so callers can downcast,
pattern-match, or inspect `std::error::Error::source()`. `LlmError` and `CruxErr` both
convert automatically. This is the right approach for a domain error type — it keeps the
domain clean while not discarding diagnostic information.

### `Agent` — multi-turn agentic loop (`src/agent.rs`)

Drives a tool-use conversation loop against `LlmProvider` and `ToolExecutor` ports:

```rust
pub struct Agent {
    llm: Box<dyn LlmProvider>,       // async inference port
    tools: Box<dyn ToolExecutor>,    // tool execution port
    events: EventManager,            // pre/post hook firing
    system: Option<String>,          // system prompt
    tool_defs: Vec<ToolDefinition>,  // tools exposed to the model
}
```

The loop (in `run_turn`):

1. Build `InferenceRequest { messages, tools, system, max_tokens: 4096 }`
2. Call `llm.infer(&req).await` — returns `InferenceResponse { content: Vec<ContentBlock>, .. }`
3. Append assistant message to history
4. If no tool calls in response → return `TurnResult { text, observations }`
5. For each `ContentBlock::ToolUse { id, name, input }`:
    - Fire `Event::PreToolUse`
    - Call `tools.execute(ToolInput { name, args })`
    - Fire `Event::PostToolUse`
    - Record `Observation` (session_id, turn, input, output)
    - Collect `ContentBlock::ToolResult { tool_use_id, content }`
6. Append tool results as a user turn and loop
7. Hard limit: `MAX_TOOL_ROUNDS = 50` → `AgentLoopError::MaxRoundsExceeded`

Both ports are `Box<dyn …>` — no generic parameters at call sites.

### `AgentLoopError` vs `AgentError`

Two separate error types were introduced:

- `AgentLoopError` — scoped to `run_turn()`: `Llm(String)`, `Tool(String)`, `MaxRoundsExceeded`
- `AgentError` — the public domain error: wraps `LlmError` and `CruxErr` by value

The intent was to reconcile these in a follow-up. `AgentLoopError` stringifies errors (loses
type) — should be replaced with `AgentError` throughout.

---

## The LLM API Gap

The branch was built against a richer `minibox-llm` interface that was removed before merge.
What the agent needs vs what currently exists:

| Needed by agent                                                      | Current `minibox-llm`                                      | Gap                                     |
| -------------------------------------------------------------------- | ---------------------------------------------------------- | --------------------------------------- |
| `InferenceRequest { messages, tools, system, max_tokens }`           | `CompletionRequest { prompt, system, max_tokens, schema }` | No message history, no tool definitions |
| `InferenceResponse { content: Vec<ContentBlock> }`                   | `CompletionResponse { text: String }`                      | No structured content blocks            |
| `ContentBlock::Text`, `ToolUse`, `ToolResult`                        | —                                                          | Missing entirely                        |
| `Message::user()`, `Message::assistant()`, `Message::tool_results()` | —                                                          | Missing entirely                        |
| `Role` enum                                                          | —                                                          | Missing                                 |
| `ToolDefinition { name, description, schema }`                       | —                                                          | Missing                                 |
| `LlmProvider::infer(&InferenceRequest)`                              | `LlmProvider::complete(&CompletionRequest)`                | Different method, different types       |

The `CompletionRequest`/`complete()` API is designed for single-turn completions with optional
structured output. The agent loop requires multi-turn conversation history and tool-use blocks.
These are genuinely different use cases and need separate methods or a unified request type.

---

## Porting Plan

To re-land `feat/minibox-agent-error` cleanly:

### Option A — Extend `minibox-llm` with an inference API alongside `complete()`

Add to `LlmProvider`:

```rust
async fn infer(&self, req: &InferenceRequest) -> Result<InferenceResponse, LlmError>;
```

Where `InferenceRequest` and `InferenceResponse` are new types in `minibox-llm/src/types.rs`
covering message history, content blocks, and tool definitions. `complete()` stays as-is for
single-turn use cases. Anthropic and OpenAI providers implement both.

### Option B — Replace `complete()` with `infer()` everywhere

`CompletionRequest`/`CompletionResponse` become a thin wrapper or alias over the richer types.
The `ainvoke!` macro adapts. Higher churn but a single unified API.

### Option C — Keep `minibox-agent` self-contained

`minibox-agent` carries its own `InferenceRequest`/`ContentBlock` types and adapts them to
`CompletionRequest` internally. Avoids changing `minibox-llm` but leaks abstractions upward.
Not recommended.

**Recommended: Option A.** Add `infer()` to `LlmProvider` as a second method. The two use cases
(single-turn structured output vs multi-turn tool-use) are distinct enough to warrant separate
paths. Implement it in the Anthropic provider first (the branch's tests use `ScriptedLlm` so
they'll pass immediately once the types exist).

---

## Files to Recover

When porting forward, recover these from `feat/minibox-agent-error`:

| File                                       | What to keep                                             |
| ------------------------------------------ | -------------------------------------------------------- |
| `crates/minibox-agent/src/error.rs`        | `AgentError` as written — no changes needed              |
| `crates/minibox-agent/src/agent.rs`        | `Agent`, `TurnResult`, `AgentLoopError`, full test suite |
| `crates/minibox-agent/src/conversation.rs` | Conversation history management using `Message` types    |

The `AgentError` from `error.rs` is already correct and can be committed independently of
the LLM API work — it compiles fine with the current `minibox-llm` since it only references
`LlmError`, which exists.

---

## Related

- `crates/minibox-llm/src/provider.rs` — `LlmProvider` trait (add `infer()` here)
- `crates/minibox-llm/src/types.rs` — add `InferenceRequest`, `InferenceResponse`,
  `ContentBlock`, `Message`, `Role`, `ToolDefinition`
- `feat/minibox-agent-error` branch — preserved, not deleted; contains full implementation
