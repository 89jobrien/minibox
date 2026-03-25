# Agentbox Design Spec

**Date**: 2026-03-25
**Status**: Draft
**Scope**: Go agent runtime for minibox — compiled AI agent binaries shippable in container images

## Overview

Agentbox is a Go-based agent subsystem for minibox that compiles AI agents into shippable binaries. Agents use the Claude Agent SDK (CLI subprocess) for agentic workflows and minibox-llm (direct API) for simple completions. Communication flows through a pub/sub core (Go channels for Tier A, upgradeable to NATS for Tier B).

Three deployment tiers, designed upfront, built sequentially:

- **Tier A**: In-container agent binaries (orchestration-first)
- **Tier B**: Agent runtime daemon alongside miniboxd
- **Tier C**: Discoverable tool agents with capability manifests

## LLM Access Layer

Dual-mode LLM access. Agents choose mode per-task based on these rules:

- **Use Agent SDK** when the task requires tool use, file editing, multi-turn reasoning, MCP servers, or code execution. The agent needs to act on the environment.
- **Use minibox-llm** when the task is a single-shot completion: scoring, classification, structured JSON output, summarization. The agent needs an answer, not an action.

| Mode | When | How |
|------|------|-----|
| Agent SDK (CLI subprocess) | Agentic workflows: tool use, multi-turn, MCP, code generation | Go Agent SDK wraps `claude` CLI, NDJSON over stdin/stdout |
| minibox-llm (direct API) | Simple completions: scoring, classification, structured output | Rust HTTP service (existing crate, ~600 lines) |

### Agent SDK Integration

Use a community Go port of the Claude Agent SDK. Container images ship with the `claude` CLI binary alongside the Go agent binary. The SDK spawns `claude` as a child process and communicates via NDJSON over stdin/stdout.

Candidate SDKs evaluated:

| Project | Differentiator |
|---------|---------------|
| character-ai/claude-agent-sdk-go | Direct API Agent, type-safe tool registration via generics |
| severity1/claude-agent-sdk-go | 100% Python SDK compat, dual API (Query + Client) |
| M1n9X/claude-agent-sdk-go | Complete 204-feature parity with Python SDK |
| partio-io/claude-agent-sdk-go | Multi-turn, tool use, hooks, MCP, subagents |

Final SDK selection deferred to implementation phase — evaluate against these criteria:
1. CLI subprocess stability (NDJSON parsing robustness)
2. Multi-turn session management
3. Tool use / MCP support
4. Active maintenance
5. Minimal dependencies

### minibox-llm (Direct API)

The existing Rust crate (`crates/minibox-llm/`) provides multi-provider LLM access with fallback chains, retry logic, and structured output. Go agents call it for simple completions where full agentic capabilities are unnecessary.

For Tier A, agents call minibox-llm either:
- Via a lightweight HTTP/UDS sidecar (Axum, ~100 lines of Rust wrapper)
- Or via a Go-native reimplementation wrapping official SDKs (`anthropic-sdk-go`, `openai-go`, `go-genai`) — ~700 lines of Go mirroring the same `FallbackChain` + `RetryingProvider` pattern

Provider priority: Claude (primary) → OpenAI (fallback) → Gemini (fallback). API keys read from environment (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`).

### FFI Decision

No FFI. The minibox-llm crate is ~600 lines of HTTP wrappers + retry logic. Official Go SDKs exist for all three providers. Pure Go reimplementation is simpler than any FFI approach (cgo, UniFFI, purego, WASM). The Rust crate continues to serve Rust consumers; Go agents get a native Go implementation of the same patterns.

## Pub/Sub Core

### Tier A: Go Channels

In-process pub/sub using Go channels and goroutines. Zero overhead for goroutine-based agent orchestration.

### Message Format

Structured JSONL with envelope:

```json
{"source": "council-critic", "timestamp": "2026-03-25T12:00:00Z", "topic": "result.council.abc123", "schema_version": 1, "payload": {"score": 0.82, "findings": [...]}}
```

### Topic Convention

- `task.<agent>.<id>` — task assignments from parent to child
- `result.<agent>.<id>` — completed results from child to parent
- `state.<agent>.<id>` — state updates (progress, health)

### Behavior

- Dynamic topic creation — agents subscribe on startup, topics created on first publish
- Context-aware retry — failed message delivery retried based on error type
- Fail-out after configurable attempts (default: 3)
- Interface contract designed so Go channels can be swapped for NATS in Tier B without rewriting agent code

### Tier B Upgrade Path (NATS)

When cross-container routing is needed, the `MessageBroker` interface swaps from `ChannelBroker` to `NATSBroker`. Same topic convention, same JSONL messages, same subscribe/publish API. Embedded NATS server runs inside the agent runtime daemon. JetStream provides message persistence for replay.

## Binary Structure

Hybrid: one orchestration binary + standalone tool binaries.

### `agentbox` (orchestration binary)

Single binary with subcommands for agents that spawn other agents:

```
agentbox meta-agent <task>              # design + spawn parallel agents
agentbox council [--mode core|extensive] # multi-role analysis
agentbox review [--base main]           # security/correctness review
agentbox bench <subcmd>                 # performance analysis
```

### Standalone tool binaries

Each tool agent is a separate binary. Independently shippable, smaller attack surface:

```
mbx-commit-msg    # AI commit message generation
mbx-diagnose      # container failure diagnosis
mbx-sync-check    # git sync with conflict resolution
mbx-gen-tests     # test scaffolding
mbx-standup       # activity summarization
```

### Container Image Layout

Container images include:
- `agentbox` binary (orchestration)
- Tool binaries as needed (per-image selection)
- `claude` CLI binary (for Agent SDK subprocess)
- No Rust toolchain or Python runtime required

## Output and Logging

Dual output during migration, then deprecate direct file writes once pub/sub logger is stable.

### Pub/Sub Output

Results published to `result.<agent>.<id>` topics. Parent agents (meta-agent) subscribe and synthesize.

### File Output (backward compatible)

- `~/.mbx/agent-runs.jsonl` — run metadata (start time, duration, agent type, exit status, token usage)
- `~/.mbx/ai-logs/<sha>-<agent>.md` — human-readable reports

Compatible with existing `dashboard.py` — no changes needed during migration.

## Directory Structure

```
agentbox/
├── go.mod
├── go.sum
├── cmd/
│   ├── agentbox/          # orchestration binary
│   │   └── main.go
│   ├── mbx-commit-msg/    # tool binary
│   │   └── main.go
│   ├── mbx-diagnose/
│   │   └── main.go
│   ├── mbx-sync-check/
│   │   └── main.go
│   ├── mbx-gen-tests/
│   │   └── main.go
│   └── mbx-standup/
│       └── main.go
├── internal/
│   ├── agent/             # agent SDK wrapper
│   │   ├── sdk.go         # Claude Agent SDK client
│   │   └── session.go     # session management
│   ├── llm/               # direct API client
│   │   ├── provider.go    # LlmProvider interface
│   │   ├── anthropic.go   # Claude (primary)
│   │   ├── openai.go      # fallback
│   │   ├── gemini.go      # fallback
│   │   ├── chain.go       # FallbackChain
│   │   └── retry.go       # RetryingProvider
│   ├── pubsub/            # pub/sub core
│   │   ├── broker.go      # in-process broker (Go channels)
│   │   ├── message.go     # JSONL message envelope
│   │   └── topic.go       # dynamic topic registry
│   ├── orchestrator/      # orchestration agent implementations
│   │   ├── meta.go        # meta-agent: design + spawn
│   │   ├── council.go     # multi-role council
│   │   ├── review.go      # code review
│   │   └── bench.go       # benchmark analysis
│   ├── tools/             # tool agent implementations
│   │   ├── commitmsg.go
│   │   ├── diagnose.go
│   │   ├── synccheck.go
│   │   ├── gentests.go
│   │   └── standup.go
│   ├── context/           # repo context discovery
│   │   ├── git.go         # git log, diff, branch info
│   │   ├── project.go     # CLAUDE.md, README, structure
│   │   └── discover.go    # dynamic context collection
│   └── output/            # result persistence
│       ├── jsonl.go       # ~/.mbx/agent-runs.jsonl writer
│       ├── report.go      # ~/.mbx/ai-logs/ markdown writer
│       └── dual.go        # pub/sub + file dual writer
├── pkg/
│   └── manifest/          # Tier C: capability manifests
│       ├── schema.go      # manifest JSON schema
│       └── registry.go    # discovery registry
```

## Hexagonal Architecture

Following minibox's existing SOLID patterns. Domain interfaces define ports; adapters implement them; composition root wires everything.

### Domain Layer (interfaces)

```go
// AgentRunner — execute an agent with config, return results
type AgentRunner interface {
    Run(ctx context.Context, config AgentConfig) (AgentResult, error)
}

// LlmProvider — single LLM completion
type LlmProvider interface {
    Name() string
    Complete(ctx context.Context, req CompletionRequest) (CompletionResponse, error)
}

// MessageBroker — pub/sub messaging
type MessageBroker interface {
    Publish(ctx context.Context, topic string, msg Message) error
    Subscribe(ctx context.Context, topic string) (<-chan Message, error)
    Close() error
}

// ContextProvider — gather repo context
type ContextProvider interface {
    GitLog(ctx context.Context, n int) ([]Commit, error)
    Diff(ctx context.Context, base string) (string, error)
    ProjectRules(ctx context.Context) (ProjectContext, error)
}

// ResultWriter — persist agent results
type ResultWriter interface {
    WriteRun(ctx context.Context, run AgentRun) error
    WriteReport(ctx context.Context, report AgentReport) error
}
```

### Adapters

| Interface | Adapter | Notes |
|-----------|---------|-------|
| AgentRunner | ClaudeSDKRunner | CLI subprocess, NDJSON |
| LlmProvider | AnthropicProvider | `anthropic-sdk-go` (primary) |
| LlmProvider | OpenAIProvider | `openai-go` (fallback) |
| LlmProvider | GeminiProvider | `go-genai` (fallback) |
| MessageBroker | ChannelBroker | Go channels (Tier A) |
| MessageBroker | NATSBroker | Embedded NATS (Tier B) |
| ContextProvider | GitContextProvider | `git` CLI subprocess |
| ResultWriter | DualResultWriter | pub/sub + `~/.mbx/` files |

### Composition Root

`cmd/agentbox/main.go` wires interfaces to concrete adapters:

```go
func main() {
    // Read config from env / flags
    broker := pubsub.NewChannelBroker()
    llm := llm.NewFallbackChain(
        llm.NewAnthropicFromEnv(),
        llm.NewOpenAIFromEnv(),
        llm.NewGeminiFromEnv(),
    )
    sdk := agent.NewClaudeSDK()
    ctx := context.NewGitProvider()
    out := output.NewDualWriter(broker)

    // Dispatch subcommand
    switch subcommand {
    case "meta-agent":
        orchestrator.RunMetaAgent(broker, sdk, llm, ctx, out, task)
    case "council":
        orchestrator.RunCouncil(broker, sdk, llm, ctx, out, mode)
    // ...
    }
}
```

## Tier B: Agent Runtime Daemon

Future phase. Go daemon (`agentboxd`) running alongside miniboxd.

### Responsibilities

- Receive task requests via pub/sub (NATS) or HTTP API
- Spawn agent containers via minibox CLI/API
- Mount context (repo checkout, config) into containers
- Monitor agent health: heartbeat checks, timeout enforcement
- Manage resource limits: CPU/memory per agent via cgroups (inherited from minibox)
- Enforce concurrent agent limits with queue overflow handling
- Persist message history via NATS JetStream for replay

### Container Integration

Each agent task runs in an isolated minibox container:
- Agent binary + `claude` CLI copied/mounted in
- Environment variables injected (API keys, config)
- Results streamed back via pub/sub or container stdout
- Container cleaned up on completion or timeout

## Tier C: Discoverable Tool Agents

Future phase. Tool agents advertise capabilities via manifests.

### Agent Manifest

Each tool binary supports `--manifest` flag:

```json
{
  "name": "mbx-commit-msg",
  "version": "0.1.0",
  "description": "Generate conventional commit messages from staged diffs",
  "inputs": {
    "diff": {"type": "string", "description": "Git diff to summarize"},
    "history": {"type": "string", "description": "Recent commit history", "optional": true}
  },
  "outputs": {
    "message": {"type": "string", "description": "Generated commit message"}
  },
  "capabilities": ["git", "commit-message", "conventional-commits"],
  "requires": {
    "tools": ["git"],
    "env": ["ANTHROPIC_API_KEY"]
  }
}
```

### Discovery Registry

- Scans container images for agent binaries
- Reads manifests via `<binary> --manifest`
- Builds capability index
- Exposes query API: "find agent with capability X"
- Callers invoke by capability; registry resolves to binary + container image
- Agent runtime (Tier B) handles execution

## Agent Implementations

### Meta-Agent (orchestration)

Port of `scripts/meta-agent.py`. Workflow:
1. Collect repo context (CLAUDE.md, git log, code structure)
2. Designer agent (via Claude Agent SDK) → JSON agent plan (2-5 independent tasks)
3. Spawn child agents concurrently via pub/sub
4. Collect results from `result.<child>.<id>` topics
5. Synthesizer agent → final report
6. Write to `~/.mbx/ai-logs/<sha>-meta-agent.md`

### Council (orchestration)

Port of `scripts/council.py`. Modes:
- **Core**: Strict Critic, Creative Explorer, General Analyst (3 roles)
- **Extensive**: + Security Reviewer, Performance Analyst (5 roles)

Each role runs as a concurrent Agent SDK call with its own system prompt. Roles score branch health 0.0–1.0 via structured output (minibox-llm direct API with JSON schema). Synthesis produces consensus areas, dialectic tensions, and balanced recommendations.

### Review Agent (analysis)

Port of `scripts/ai-review.py`. Security + correctness review of diffs against a base branch. Produces structured findings with file paths, line numbers, severity levels.

### Bench Agent (analysis)

Port of `scripts/bench-agent.py`. Subcommands: `report`, `compare`, `regress`, `cleanup`, `trigger`. Reads benchmark data from `bench/results/`, produces analysis via LLM.

### Tool Agents

| Agent | Port of | Core Logic |
|-------|---------|-----------|
| mbx-commit-msg | `scripts/commit-msg.py` | Read staged diff + recent history → generate conventional commit message |
| mbx-diagnose | `scripts/diagnose.py` | Read container logs + cgroup state → diagnose failure |
| mbx-sync-check | `scripts/sync-check.py` | Fetch + rebase onto origin/main, auto-resolve obvious conflicts |
| mbx-gen-tests | `scripts/gen-tests.py` | Read trait definition → scaffold unit tests for adapter |
| mbx-standup | `scripts/standup.py` | Read git activity across repos → time-block standup summary |

## Migration Plan

1. Build `agentbox` with meta-agent + council (Tier A, orchestration-first)
2. Port review + bench agents
3. Port tool agents as standalone binaries
4. Existing Python scripts remain functional throughout — no breaking changes
5. Dashboard reads from same `~/.mbx/` paths
6. Update Justfile/mise.toml to offer both Python and Go invocations
7. Deprecate Python scripts once Go versions are stable and tested
8. Tier B (agent runtime daemon) after Tier A agents are proven
9. Tier C (discovery registry) after Tier B daemon is operational

## Testing Strategy

### Unit Tests

- Mock `AgentRunner`, `LlmProvider`, `MessageBroker` interfaces
- Test orchestration logic (meta-agent plan parsing, council synthesis) with canned responses
- Test pub/sub message routing with `ChannelBroker`
- Test retry/fallback chain with simulated failures

### Integration Tests

- Agent SDK integration: spawn real `claude` CLI, verify NDJSON exchange
- LLM provider integration: call real APIs with test prompts (gated behind env vars)
- Pub/sub: verify message delivery ordering and fail-out behavior

### E2E Tests

- Meta-agent: full workflow from task → design → spawn → synthesize
- Council: full multi-role analysis on a test branch
- Tool agents: verify output format matches Python script output

## Dependencies

### Go Modules (expected)

- Claude Agent SDK Go port (TBD — evaluate during implementation)
- `github.com/anthropics/anthropic-sdk-go` — official Anthropic SDK
- `github.com/openai/openai-go` — official OpenAI SDK
- `github.com/googleapis/go-genai` — official Google GenAI SDK
- Standard library (`context`, `encoding/json`, `os/exec`, `net`)

### Runtime Dependencies

- `claude` CLI binary (for Agent SDK subprocess)
- API keys: `ANTHROPIC_API_KEY` (required), `OPENAI_API_KEY` (optional), `GEMINI_API_KEY` (optional)
- `git` CLI (for context discovery)

### No Rust Toolchain Required

Go build is fully independent of the Rust workspace. No FFI, no cgo, no shared libraries.
