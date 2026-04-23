# minibox-agent

Crux-backed agent execution layer for minibox.

Bridges `minibox-llm`'s `FallbackChain` into [`crux-agentic`](https://crates.io/crates/crux-agentic)'s
`LlmProvider` / `LlmStep` model, giving minibox agents replay, budget tracking, and structured
tracing for free.

## Architecture

```text
minibox-agent
├── Port:    crux_agentic::LlmProvider    (from crux-agentic)
├── Adapter: FallbackChainAdapter         (wraps FallbackChain as LlmProvider)
├── Step:    CruxLlmStep                  (newtype over LlmStep<FallbackChainAdapter>)
└── Error:   AgentError                   (maps LlmError + CruxErr into a domain error)
```

## Modules

| Module        | Description                                                         |
| ------------- | ------------------------------------------------------------------- |
| `agent`       | `InferenceLlmProvider` — multi-turn inference with tool use         |
| `conversation`| `Conversation` — stateful multi-turn message accumulator            |
| `error`       | `AgentError` — domain error type preserving source errors           |
| `events`      | Agent lifecycle events (start, step, complete, error)               |
| `hooks`       | Pre/post-step hook traits for observability integrations            |
| `message`     | `Message`, `ContentBlock`, `Role`, `ToolDefinition` domain types    |
| `observation` | Structured observation records emitted during agent steps           |
| `provider`    | `FallbackChainAdapter` — wraps `FallbackChain` as `LlmProvider`     |
| `session_log` | Per-session structured log writer                                   |
| `step`        | `CruxLlmStep` — newtype over `LlmStep<FallbackChainAdapter>`        |
| `tools`       | Tool registry and tool-call dispatch                                |
| `trace`       | `FileTraceStore` — file-based `TraceStore` adapter                  |
| `trajectory`  | ATIF v1.6 trajectory writer (`~/.minibox/trajectories/*.yaml`)      |

## Quick start

```rust
use minibox_agent::{AgentError, CruxLlmStep, LlmRequest, CruxCtx};

async fn summarize(ctx: &mut CruxCtx, input: String) -> Result<String, AgentError> {
    let step = CruxLlmStep::from_env();
    let resp = step.invoke(ctx, "summarize.call", LlmRequest {
        prompt: input,
        ..Default::default()
    }).await?;
    Ok(resp.text)
}
```

## Error handling

`AgentError` preserves the original source error so callers can downcast:

```rust
match err {
    AgentError::Llm(llm_err) => { /* inspect LlmError variants */ }
    AgentError::Step(crux_err) => { /* inspect CruxErr */ }
}
```

## ATIF trajectory format

Trajectories are written in [ATIF v1.6](https://atif.dev) YAML format to
`~/.minibox/trajectories/<session_id>.yaml`. The format supports multimodal content
(`ContentPart`, `ImageSource`) and aggregate metrics.
