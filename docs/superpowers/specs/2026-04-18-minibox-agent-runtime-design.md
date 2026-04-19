# minibox-agent Runtime Design

**Date**: 2026-04-18
**Status**: Approved
**Scope**: Loop-driven agent runtime in `crates/minibox-agent/` — extends crux-agentic foundation

## Overview

`minibox-agent` gains a full agent execution runtime: a step-machine loop driving Think → Act →
Observe cycles, symmetric redaction guards, bidirectional MCP integration, a pluggable event
pipeline, and pluggable context compaction. All components follow the hexagonal architecture
already established in this workspace (ports as traits, adapters as structs).

The crux-agentic foundation (`FallbackChainAdapter`, `CruxLlmStep`, `CruxCtx`) added in commit
`73beaf9` is preserved and extended — not replaced.

## Module Layout

```
crates/minibox-agent/src/
├── lib.rs              — re-exports public surface
├── error.rs            — AgentError (extend with new variants)
├── provider.rs         — FallbackChainAdapter (existing, unchanged)
├── step.rs             — CruxLlmStep (existing, unchanged)
├── context/
│   └── mod.rs          — AgentContext: wraps CruxCtx + history + TokenBudget
├── agent_loop/
│   └── mod.rs          — AgentLoop: step-machine driver + builder
├── step_kind/
│   └── mod.rs          — Step enum: Think, Act, Observe, Done
├── guard/
│   ├── mod.rs          — PreToolGuard + PostToolGuard traits
│   └── redaction.rs    — RedactionGuard impl (symmetric pre+post)
├── mcp/
│   ├── mod.rs
│   ├── client.rs       — McpClient: consume external MCP tools
│   └── server.rs       — McpServer: expose agent tools as MCP server
├── event/
│   └── mod.rs          — EventPipeline trait + ChannelPipeline impl + AgentEvent enum
└── compaction/
    ├── mod.rs          — CompactionStrategy trait + TokenBudget type
    ├── sliding_window.rs — SlidingWindow(n) impl
    └── summarize.rs    — SummarizeAndReplace impl (uses CruxLlmStep)
```

## Step Machine

The core loop transitions through four states:

```
Think → Act → Observe → Think → ... → Done
          ↑               ↓
       PreGuard        PostGuard
       redact_in       redact_out
       event emit      event emit
```

`Step` enum:

```rust
pub enum Step {
    Think { prompt: String },
    Act   { tool: String, input: serde_json::Value },
    Observe { output: serde_json::Value },
    Done  { result: String },
}
```

`AgentLoop` drives the machine:

```rust
pub struct AgentLoop {
    llm:          CruxLlmStep,
    tools:        Vec<Box<dyn Tool>>,
    pre_guards:   Vec<Box<dyn PreToolGuard>>,
    post_guards:  Vec<Box<dyn PostToolGuard>>,
    compaction:   Box<dyn CompactionStrategy>,
    events:       Box<dyn EventPipeline>,
    mcp_client:   Option<McpClient>,
    mcp_server:   Option<McpServer>,
}
```

The loop runs until `Step::Done` or budget exhaustion. On each `Act` transition: run all
`PreToolGuard`s (including redaction), call the tool (local registry or MCP), run all
`PostToolGuard`s (including redaction), emit events, check compaction budget.

## AgentContext

Wraps `CruxCtx` (crux replay + budget) and owns the conversation history:

```rust
pub struct AgentContext {
    pub crux:    CruxCtx,
    pub history: Vec<Turn>,
    pub budget:  TokenBudget,
}
```

`TokenBudget` tracks used/max tokens and triggers compaction when `used > threshold * max`
(default threshold: 0.8). `Turn` holds role (`user`/`assistant`/`tool`) + content string.

## Guards

```rust
pub trait PreToolGuard: Send + Sync {
    fn before(&self, tool: &str, input: &mut serde_json::Value) -> Result<(), AgentError>;
}

pub trait PostToolGuard: Send + Sync {
    fn after(&self, tool: &str, output: &mut serde_json::Value) -> Result<(), AgentError>;
}
```

`RedactionGuard` implements both. It holds a `Box<dyn Redactor>`:

```rust
pub trait Redactor: Send + Sync {
    fn redact(&self, text: &str) -> String;
}
```

Built-in: `ObfsckRedactor` — applies obfsck pattern set (reuses patterns already in this repo).
Guards run in registration order. A guard returning `Err` aborts the tool call and transitions
to `Done` with an error result.

## MCP Integration

### Consumer (`McpClient`)

Wraps `rmcp` (the Rust MCP SDK). At loop startup, `McpClient::connect(transport)` discovers
available tools via `list_tools()` and registers them in the loop's tool registry alongside
local tools. Tool calls route to `McpClient::call_tool(name, input)` for remote tools.

Transports supported: stdio (for local MCP servers like doob, devloop, gkg), SSE (for remote).

### Producer (`McpServer`)

Exposes the agent's local tool registry as an MCP server. External hosts (Claude Code, other
agents) call in. Transport: stdio or SSE. `McpServer::serve(transport)` is non-blocking —
spawns a Tokio task alongside the agent loop.

Tools exposed via `McpServer::register(name, handler)`. An `#[mcp_tool]` attribute macro is a
future ergonomic improvement — v1 uses manual registration only.

## Event Pipeline

```rust
pub trait EventPipeline: Send + Sync {
    fn emit(&self, event: AgentEvent);
}

pub enum AgentEvent {
    StepStarted   { step: Step },
    StepCompleted { step: Step, elapsed_ms: u64 },
    ToolCalled    { tool: String, input: serde_json::Value },
    ToolResult    { tool: String, output: serde_json::Value },
    BudgetWarning { used: u32, max: u32 },
    LoopDone      { result: String, total_steps: u32 },
}
```

`ChannelPipeline` wraps `tokio::sync::broadcast::Sender<AgentEvent>`. Consumers subscribe via
`ChannelPipeline::subscribe() -> Receiver<AgentEvent>`. `NoopPipeline` for tests.

Emission is fire-and-forget (`emit` is sync, non-blocking). Slow consumers drop events via
`broadcast` semantics — this is acceptable for observability; it must not stall the agent loop.

## Context Compaction

```rust
pub trait CompactionStrategy: Send + Sync {
    fn compact(
        &self,
        history: &mut Vec<Turn>,
        budget:  &TokenBudget,
        llm:     &CruxLlmStep,
        ctx:     &mut AgentContext,
    ) -> impl Future<Output = Result<(), AgentError>> + Send;
}
```

Two built-ins:

| Strategy | Behaviour |
|---|---|
| `SlidingWindow(n)` | Keeps the last `n` turns, drops the oldest. No LLM call. |
| `SummarizeAndReplace` | When turns exceed budget, calls `CruxLlmStep` to summarize dropped turns; prepends summary as a `system` turn. |

`AgentLoop` calls `compact` after each `Observe` step when `budget.used > budget.threshold`.

## Error Handling

New `AgentError` variants (added to `error.rs`):

```rust
pub enum AgentError {
    // existing
    Llm(String),
    Step(String),
    BudgetExceeded,
    Other(String),
    // new
    GuardRejected { guard: String, reason: String },
    McpError(String),
    CompactionFailed(String),
    EventPipelineError(String),
}
```

All new variants implement `From<...>` conversions where applicable.

## Testing Strategy

- **Unit**: each module tested in isolation with mock implementations of all traits.
  `MockTool`, `MockGuard`, `MockPipeline`, `MockCompaction` live behind `#[cfg(test)]` or a
  `test-utils` Cargo feature.
- **Step-machine**: property tests (proptest) over arbitrary `Step` sequences — verify guards
  always run, events always emit, loop always terminates.
- **Integration**: `AgentLoop` with `McpClient` connected to a local stdio MCP server (doob
  or a test stub). Gated on `#[cfg(feature = "integration-tests")]`.
- **Redaction**: snapshot tests (insta) over known secret patterns — verify `ObfsckRedactor`
  scrubs expected values.

## Build & Feature Flags

| Feature | Controls |
|---|---|
| `mcp` (default off) | Compiles `mcp/` module; pulls in `rmcp` dep |
| `redaction` (default on) | Compiles `guard/redaction.rs` + `ObfsckRedactor` |
| `test-utils` | Exports mock impls for downstream test use |

`AgentLoop` is always available. MCP is opt-in to avoid pulling in `rmcp` for consumers that
don't need it.

## Open Questions

None — all design decisions resolved during brainstorm session 2026-04-18.
