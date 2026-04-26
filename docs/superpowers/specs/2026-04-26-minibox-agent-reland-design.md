# minibox-agent Re-land: infer() API Integration

**Date**: 2026-04-26
**Status**: Draft
**Scope**: `crates/minibox-agent/` (re-create from git history) +
`crates/minibox-llm/src/provider.rs` (no changes needed)
**Handoff item**: minibox-agent-llm-api

## Overview

Re-land the minibox-agent crate that was reverted in commit `6fbd81d`. The original
revert happened because minibox-llm had been stripped to a single-turn
`CompletionRequest`/`complete()` API, breaking the agent's dependency on multi-turn
`InferenceRequest`/`infer()`.

**The API gap is now closed.** Commit `e36bedf` restored all required types to
minibox-llm: `Message`, `ContentBlock`, `ToolDefinition`, `InferenceRequest`,
`InferenceResponse`, and the `infer()` method on `LlmProvider`. The agent code
from commit `70cf113` can be ported forward with minimal adaptation.

## What Exists Today

### minibox-llm (ready -- no changes needed)

| Type/Method           | Status       | Location                           |
| --------------------- | ------------ | ---------------------------------- |
| `Role`                | Implemented  | `types.rs`                         |
| `ContentBlock`        | Implemented  | `types.rs` (Text, ToolUse, ToolResult) |
| `Message`             | Implemented  | `types.rs` (user/assistant/tool_results builders) |
| `ToolDefinition`      | Implemented  | `types.rs`                         |
| `InferenceRequest`    | Implemented  | `types.rs`                         |
| `InferenceResponse`   | Implemented  | `types.rs` (.text(), .has_tool_calls()) |
| `LlmProvider::infer()`| Implemented | `provider.rs` (default wraps complete()) |
| `FallbackChain`       | Implemented  | `chain.rs`                         |
| `OllamaProvider`      | Implemented  | `local.rs` (feature-gated)         |

### minibox-agent (reverted -- needs re-creation)

Source available in git history at commit `70cf113`. Modules:

| Module              | Purpose                           | Port complexity |
| ------------------- | --------------------------------- | --------------- |
| `error.rs`          | AgentError (Llm, Step variants)   | Trivial         |
| `agent.rs`          | Agent loop (infer + tool execute) | Medium          |
| `conversation.rs`   | Message history + @file resolve   | Trivial         |
| `events.rs`         | Event, EventManager, EventHandler | Trivial         |
| `observation.rs`    | Observation types                 | Trivial         |
| `tools.rs`          | ToolExecutor, ToolInput/Output    | Trivial         |
| `provider.rs`       | FallbackChainAdapter (crux bridge)| Medium -- assess if still needed |
| `step.rs`           | CruxLlmStep (crux integration)   | Medium -- assess if still needed |
| `session_log.rs`    | Session logging                   | Trivial         |
| `trace.rs`          | Trace infrastructure              | Trivial         |
| `hooks.rs`          | Lifecycle hooks                   | Trivial         |

## Strategy

**Option A (recommended)**: Cherry-pick from `70cf113`, resolve conflicts, drop
crux dependencies. The agent runtime spec
(`2026-04-18-minibox-agent-runtime-design.md`) supersedes the crux-based design
with a standalone step-machine loop. Port the core agent loop and conversation
management; leave crux bridge modules (`provider.rs`, `step.rs`) behind.

**Option B**: Extract files manually from git history. More control but more
manual work.

## Module Plan

### Phase 1: Core agent (this spec)

Re-create `crates/minibox-agent/` with the minimal viable surface:

```
crates/minibox-agent/src/
+-- lib.rs              -- re-exports
+-- error.rs            -- AgentError { Llm, Tool, MaxRounds }
+-- agent.rs            -- Agent struct, run_turn() loop
+-- conversation.rs     -- Conversation history, @file resolution
+-- tools.rs            -- ToolExecutor trait, ToolInput, ToolOutput
```

### Phase 2: Runtime extensions (separate spec, already written)

Per `2026-04-18-minibox-agent-runtime-design.md`:
- Step machine (Think/Act/Observe/Done)
- Guards (pre/post tool redaction)
- MCP client/server
- Event pipeline
- Context compaction

## Detailed Design

### error.rs

```rust
use minibox_llm::LlmError;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),

    #[error("tool execution failed: {tool}: {message}")]
    Tool { tool: String, message: String },

    #[error("max rounds exceeded ({0})")]
    MaxRoundsExceeded(u32),
}
```

Design note: `AgentError` preserves the source error via `#[from]` -- no
stringification. This was the correct pattern from the reverted branch.

### tools.rs

```rust
use async_trait::async_trait;
use serde_json::Value;

pub struct ToolInput {
    pub name: String,
    pub input: Value,
}

pub struct ToolOutput {
    pub content: String,
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, String>;
    fn available_tools(&self) -> Vec<minibox_llm::ToolDefinition>;
}
```

### conversation.rs

```rust
use minibox_llm::Message;
use std::path::Path;

pub struct Conversation {
    messages: Vec<Message>,
}

impl Conversation {
    pub fn new() -> Self { Self { messages: Vec::new() } }

    pub fn push_user(&mut self, content: &str) {
        // Resolve @filename tokens before appending
        let resolved = resolve_file_refs(content);
        self.messages.push(Message::user(resolved));
    }

    pub fn push_assistant(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn push_tool_results(&mut self, results: Vec<(String, String)>) {
        self.messages.push(Message::tool_results(results));
    }
}

/// Replace `@path/to/file` tokens with file contents.
fn resolve_file_refs(input: &str) -> String {
    // Regex: @followed by a non-whitespace path
    // Read file, inline contents, or leave token if file not found
    ...
}
```

### agent.rs

```rust
use crate::conversation::Conversation;
use crate::error::AgentError;
use crate::tools::{ToolExecutor, ToolInput};
use minibox_llm::{ContentBlock, InferenceRequest, LlmProvider, ToolDefinition};
use std::sync::Arc;

const MAX_ROUNDS: u32 = 50;

pub struct Agent {
    llm: Arc<dyn LlmProvider>,
    tools: Arc<dyn ToolExecutor>,
    system: Option<String>,
    max_rounds: u32,
}

pub struct TurnResult {
    pub text: Option<String>,
    pub rounds: u32,
}

impl Agent {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        tools: Arc<dyn ToolExecutor>,
    ) -> Self {
        Self { llm, tools, system: None, max_rounds: MAX_ROUNDS }
    }

    pub fn with_system(mut self, system: String) -> Self {
        self.system = Some(system);
        self
    }

    pub fn with_max_rounds(mut self, max: u32) -> Self {
        self.max_rounds = max;
        self
    }

    /// Run a single user turn through the agent loop.
    ///
    /// Calls infer(), checks for tool_calls, executes tools, feeds results
    /// back, repeats until the model produces a text-only response or
    /// max_rounds is hit.
    pub async fn run_turn(
        &self,
        conversation: &mut Conversation,
    ) -> Result<TurnResult, AgentError> {
        let tool_defs = self.tools.available_tools();
        let mut rounds = 0u32;

        loop {
            rounds += 1;
            if rounds > self.max_rounds {
                return Err(AgentError::MaxRoundsExceeded(self.max_rounds));
            }

            let request = InferenceRequest {
                messages: conversation.messages().to_vec(),
                tools: if tool_defs.is_empty() { vec![] }
                       else { tool_defs.clone() },
                system: self.system.clone(),
                max_tokens: None,
            };

            let response = self.llm.infer(&request).await?;

            // Append the full assistant response (may contain tool_use blocks)
            conversation.push_assistant(
                minibox_llm::Message::assistant(response.content.clone())
            );

            // If no tool calls, we're done
            if !response.has_tool_calls() {
                return Ok(TurnResult {
                    text: response.text(),
                    rounds,
                });
            }

            // Execute each tool call
            let mut results = Vec::new();
            for block in &response.content {
                if let ContentBlock::ToolUse { id, name, input } = block {
                    let tool_input = ToolInput {
                        name: name.clone(),
                        input: input.clone(),
                    };
                    let output = self.tools.execute(tool_input).await
                        .map_err(|msg| AgentError::Tool {
                            tool: name.clone(),
                            message: msg,
                        })?;
                    results.push((id.clone(), output.content));
                }
            }

            // Feed tool results back into conversation
            conversation.push_tool_results(results);
        }
    }
}
```

### Cargo.toml

```toml
[package]
name = "minibox-agent"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
minibox-llm = { path = "../minibox-llm" }
async-trait.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio = { workspace = true, features = ["fs"] }
tracing.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

**Note**: No `crux-agentic` or `cruxai-core` dependency. The Phase 2 runtime
spec (step machine, guards, MCP) replaces the crux integration entirely.

## Testing Strategy

### Unit tests (in each module)

- **agent.rs**: `ScriptedLlm` mock that returns pre-defined responses.
  `InMemoryToolExecutor` that records calls and returns canned outputs.
  - Test: single text response (no tools) -- 1 round
  - Test: tool call + result + final text -- 2 rounds
  - Test: max_rounds exceeded -- returns error
  - Test: tool execution error -- returns AgentError::Tool

- **conversation.rs**: @file resolution tests
  - Test: `@/tmp/test.txt` replaced with file contents
  - Test: `@nonexistent` left as-is
  - Test: multiple @refs in one message

- **tools.rs**: ToolExecutor trait object construction
  - Test: available_tools returns correct definitions

### Integration tests (gated)

- `#[cfg(feature = "integration-tests")]`: Agent + OllamaProvider end-to-end
  (requires local Ollama instance)

## Migration Path from Reverted Code

Files to recover from commit `70cf113`:

| Reverted file         | Action                                     |
| --------------------- | ------------------------------------------ |
| `error.rs`            | Port directly -- only 2 variants needed    |
| `agent.rs`            | Rewrite -- drop crux refs, use infer() API |
| `conversation.rs`     | Port directly -- @file resolution intact   |
| `tools.rs`            | Port directly -- trait unchanged            |
| `events.rs`           | Skip -- Phase 2 (event pipeline)           |
| `observation.rs`      | Skip -- Phase 2                            |
| `provider.rs`         | Skip -- crux bridge no longer needed       |
| `step.rs`             | Skip -- crux bridge no longer needed       |
| `session_log.rs`      | Skip -- Phase 2                            |
| `trace.rs`            | Skip -- Phase 2                            |
| `hooks.rs`            | Skip -- Phase 2                            |

## What Does Not Change

- `minibox-llm` -- the API gap is closed, no modifications needed
- `minibox-core` -- no agent-specific types
- `daemonbox` / `miniboxd` -- agent is a library, not wired into the daemon
- `linuxbox` -- no changes

## Relationship to Runtime Spec

The `2026-04-18-minibox-agent-runtime-design.md` spec describes the full Phase 2
runtime: step machine, guards, MCP integration, event pipeline, compaction. This
spec covers only Phase 1 -- the minimal agent loop that exercises the
minibox-llm `infer()` API and proves the integration works.

Phase 2 builds on top of the types introduced here. `Agent.run_turn()` evolves
into `AgentLoop` with the Think/Act/Observe/Done state machine. `ToolExecutor`
becomes the tool registry. `Conversation` evolves into `AgentContext`.

## Open Questions

None. The API surface is proven (existed in the reverted branch), the LLM types
are restored, and the design doc at `docs/minibox-agent-design.md` resolves all
architectural decisions.
