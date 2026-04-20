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
`73beaf9` is preserved and extended — not replaced. `AgentLoop` is intentionally coupled to
`CruxLlmStep`/`CruxCtx`: crux is the framework providing replay and budget tracking, not
infrastructure to abstract away.

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
    Think   { prompt: String },
    Act     { tool: String, input: serde_json::Value },
    Observe { output: serde_json::Value },
    Done    { result: String },
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
    mcp_server:   Option<McpServerHandle>,
}
```

`McpServerHandle` (see MCP section) holds the `JoinHandle` for the server background task and
is cancelled on `Drop`.

The loop runs until `Step::Done` or budget exhaustion. On each `Act` transition: run all
`PreToolGuard`s (including redaction), call the tool (local registry or MCP), run all
`PostToolGuard`s (including redaction), emit events, check compaction budget.

Event emission uses `Arc<Step>` — the loop clones the `Arc` cheaply before emitting so the
step value remains accessible after the emit call. See Event Pipeline section.

## AgentContext

Wraps `CruxCtx` (crux replay + budget) and owns the conversation history:

```rust
pub struct AgentContext {
    pub crux:    CruxCtx,
    pub history: Vec<Turn>,
    pub budget:  TokenBudget,
}
```

`TokenBudget` tracks used/max tokens using `u64` (conventional for token counts) and triggers
compaction when `used > threshold * max` (default threshold: 0.8). `Turn` holds role
(`user`/`assistant`/`tool`) + content string.

## Guards

```rust
pub trait PreToolGuard: Send + Sync {
    fn before(&self, tool: &str, input: &mut serde_json::Value) -> Result<(), AgentError>;
}

pub trait PostToolGuard: Send + Sync {
    fn after(&self, tool: &str, output: &mut serde_json::Value) -> Result<(), AgentError>;
}
```

`RedactionGuard` implements both traits. To register one instance in both guard vecs, wrap it
in `Arc` and clone the `Arc` into each vec:

```rust
let guard = Arc::new(RedactionGuard::new(Arc::new(ObfsckRedactor::new())));
loop_builder
    .pre_guard(Arc::clone(&guard) as Arc<dyn PreToolGuard>)
    .post_guard(Arc::clone(&guard) as Arc<dyn PostToolGuard>);
```

`RedactionGuard` holds `Arc<dyn Redactor>`:

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

**v1 limitation**: tool list is discovered once at startup. Dynamic tool list changes (e.g. a
doob MCP server adding tools at runtime) are not reflected until the loop restarts. A
`McpClient::refresh()` method is out of scope for v1.

### Producer (`McpServer` + `McpServerHandle`)

Exposes the agent's local tool registry as an MCP server. External hosts (Claude Code, other
agents) call in. Transport: stdio or SSE.

`McpServer::serve(transport)` spawns a Tokio task and returns a `McpServerHandle`:

```rust
pub struct McpServerHandle {
    _cancel: tokio_util::sync::CancellationToken,
    task:    tokio::task::JoinHandle<()>,
}

impl Drop for McpServerHandle {
    fn drop(&mut self) {
        self._cancel.cancel();
        // task will terminate on next poll
    }
}
```

`AgentLoop` stores `Option<McpServerHandle>` — dropping the loop cancels the server task.

Tools exposed via `McpServer::register(name, handler)`. An `#[mcp_tool]` attribute macro is a
future ergonomic improvement — v1 uses manual registration only.

## Event Pipeline

```rust
pub trait EventPipeline: Send + Sync {
    fn emit(&self, event: AgentEvent);
}

pub enum AgentEvent {
    StepStarted   { step: Arc<Step> },
    StepCompleted { step: Arc<Step>, elapsed_ms: u64 },
    ToolCalled    { tool: String, input: serde_json::Value },
    ToolResult    { tool: String, output: serde_json::Value },
    BudgetWarning { used: u64, max: u64 },
    LoopDone      { result: String, total_steps: u32 },
}
```

`Step` is wrapped in `Arc` in event variants: the loop wraps the current step in `Arc` before
emitting, then retains the `Arc` for continued use in the loop body. No clone of the step data
is needed.

`ChannelPipeline` wraps `tokio::sync::broadcast::Sender<AgentEvent>`. Consumers subscribe via
`ChannelPipeline::subscribe() -> Receiver<AgentEvent>`. `NoopPipeline` for tests.

Emission is fire-and-forget (`emit` is sync, non-blocking). `broadcast` uses a fixed-capacity
ring buffer — when full, the oldest unread messages are silently dropped and subscribers
receive `RecvError::Lagged` on next poll. **All consumers must handle `RecvError::Lagged`** —
treat it as a signal that events were dropped and continue receiving. This is by design:
observability must not stall the agent loop.

## Context Compaction

```rust
use async_trait::async_trait;

#[async_trait]
pub trait CompactionStrategy: Send + Sync {
    async fn compact(
        &self,
        history: &mut Vec<Turn>,
        budget:  &TokenBudget,
        llm:     &CruxLlmStep,
        ctx:     &mut AgentContext,
    ) -> Result<(), AgentError>;
}
```

`async_trait` is used to make the trait object-safe for `Box<dyn CompactionStrategy>` storage
in `AgentLoop`. RPITIT (`-> impl Future`) is not object-safe in Rust 2024.

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
}
```

`EventPipelineError` is not included — `emit` is fire-and-forget with no return value, so
pipeline errors are unobservable at the call site and the variant would be dead code.

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
| `mcp` (default off) | Compiles `mcp/` module; pulls in `rmcp` + `tokio-util` deps |
| `redaction` (default on) | Compiles `guard/redaction.rs` + `ObfsckRedactor` |
| `test-utils` | Exports mock impls for downstream test use |

`AgentLoop` is always available. MCP is opt-in to avoid pulling in `rmcp` for consumers that
don't need it. `tokio-util` (for `CancellationToken`) is gated behind `mcp` as well.

## Open Questions

None — all design decisions resolved during brainstorm session 2026-04-18. Sentinel review
applied 2026-04-18: object-safety fix for `CompactionStrategy`, `Arc<Step>` for event
ownership, `Arc<RedactionGuard>` dual-registration pattern, `McpServerHandle` drop semantics,
`RecvError::Lagged` documented, `EventPipelineError` removed, `u64` token budget,
`CruxLlmStep` coupling rationale documented.
