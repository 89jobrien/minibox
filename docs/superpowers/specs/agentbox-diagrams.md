# Agentbox Architecture Diagrams

**Date**: 2026-03-26
**Companion to**: `2026-03-25-agentbox-design.md`
**Scope**: Data flow, component relationships, and conventions for the Tier A implementation

---

## 1. System Overview

Shows the four-layer module structure of the `agentbox/` Go module: Binary (CLI entry points), Orchestration (council/meta-agent logic + tool adapters + SDK runner), Infrastructure (LLM provider, git context, JSONL output, pub/sub broker), and Domain (shared interfaces and types). Each layer depends only on layers below it; the Domain layer has no outward dependencies.

```
┌───────────────────────────────────────────────────────────────────────┐
│                          agentbox/ Go module                          │
│                                                                       │
│  ┌──────────────────────────────────────────────────────────────────┐ │
│  │                        Binary Layer                              │ │
│  │  ┌───────────────────────┐  ┌─────────────────────────────────┐  │ │
│  │  │   cmd/agentbox/       │  │   cmd/minibox-commit-msg/           │  │ │
│  │  │   • council           │  │   • standalone commit-msg tool  │  │ │
│  │  │   • meta-agent        │  │                                 │  │ │
│  │  └──────────┬────────────┘  └──────────────┬──────────────────┘  │ │
│  └─────────────┼──────────────────────────────┼─────────────────────┘ │
│                │                              │                       │
│  ┌─────────────▼──────────────────────────────▼─────────────────────┐ │
│  │                     Orchestration Layer                          │ │
│  │  ┌─────────────────────┐  ┌──────────────────┐  ┌──────────────┐ │ │
│  │  │ orchestrator/       │  │ tools/           │  │ agent/       │ │ │
│  │  │ • Council           │  │ • CommitMsg      │  │ • SDKRunner  │ │ │
│  │  │ • MetaAgent         │  │                  │  │              │ │ │
│  │  └────────┬────────────┘  └────────┬─────────┘  └───────┬──────┘ │ │
│  └───────────┼────────────────────────┼────────────────────┼────────┘ │
│              │                        │                    │          │
│  ┌───────────▼────────────────────────▼────────────────────▼────────┐ │
│  │                     Infrastructure Layer                         │ │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────────┐  │ │
│  │  │ llm/     │  │ context/ │  │ output/  │  │ pubsub/          │  │ │
│  │  │ Anthropic│  │ Git CLI  │  │ JSONL    │  │ ChannelBroker    │  │ │
│  │  │ Chain    │  │ Rules    │  │ Report   │  │ (Go channels)    │  │ │
│  │  │ Retry    │  │ Diff     │  │ Dual     │  │                  │  │ │
│  │  └──────────┘  └──────────┘  └──────────┘  └──────────────────┘  │ │
│  └──────────────────────────────────────────────────────────────────┘ │
│                                                                       │
│  ┌──────────────────────────────────────────────────────────────────┐ │
│  │                        Domain Layer                              │ │
│  │  domain/types.go     Message, AgentConfig, AgentResult, AgentRun │ │
│  │  domain/interfaces.go AgentRunner, LlmProvider, MessageBroker,   │ │
│  │                       ContextProvider, ResultWriter              │ │
│  └──────────────────────────────────────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────┘
```

---

## 2. Hexagonal Architecture — Ports and Adapters

Shows the ports-and-adapters (hexagonal) design. The Domain Core defines five Go interfaces (ports). Driving adapters on the left (`cmd/agentbox`, `cmd/minibox-commit-msg`, future HTTP/gRPC) invoke the core. Driven adapters on the right implement each port: `ClaudeSDKRunner` for agent execution, `AnthropicProvider` for LLM calls, `ChannelBroker` for pub/sub, `GitProvider` for branch context, and `DualWriter` for JSONL + Markdown output. Swapping any adapter requires no changes to the domain core.

```
                    ┌──────────────────────────────────┐
                    │         Domain Ports             │
                    │       (Go interfaces)            │
                    └──────────┬───────────────────────┘
                               │
         ┌─────────────────────┼──────────────────────┐
         │                     │                      │
    ┌────▼──────┐         ┌────▼───────┐         ┌────▼─────────┐
    │  Driving  │         │  Driving   │         │   Driving    │
    │  Adapter  │         │  Adapter   │         │   Adapter    │
    │           │         │            │         │              │
    │ cmd/      │         │ cmd/       │         │  (future)    │
    │ agentbox  │         │ minibox-       │         │  HTTP API    │
    │           │         │ commit-msg │         │  gRPC        │
    └────┬──────┘         └─────┬──────┘         └──────────────┘
         │                      │
         └───────────┬──────────┘
                     │
              ┌──────▼─────────────────────────────────────────────┐
              │              Domain Core                           │
              │                                                    │
              │  AgentRunner ─── Run(ctx, AgentConfig) → Result    │
              │  LlmProvider ── Complete(ctx, Request) → Response  │
              │  MessageBroker ─ Publish / Subscribe / Close       │
              │  ContextProvider ─ GitLog / Diff / BranchContext   │
              │  ResultWriter ── WriteRun / WriteReport            │
              └─────┬──────────────────────────────────────────────┘
                    │
         ┌──────────┼──────────┬──────────────┬────────────┐
         │          │          │              │            │
    ┌────▼────┐ ┌───▼─────┐ ┌──▼──────┐ ┌─────▼───┐ ┌──────▼───┐
    │ Driven  │ │ Driven  │ │ Driven  │ │ Driven  │ │  Driven  │
    │ Adapter │ │ Adapter │ │ Adapter │ │ Adapter │ │  Adapter │
    │         │ │         │ │         │ │         │ │          │
    │ Claude  │ │Anthropic│ │ Channel │ │  Git    │ │   Dual   │
    │ SDK     │ │  SDK    │ │ Broker  │ │ Provider│ │  Writer  │
    │ Runner  │ │  +      │ │         │ │  (CLI)  │ │ JSONL +  │
    │         │ │ Chain   │ │ (future │ │         │ │ Markdown │
    │         │ │ +Retry  │ │  NATS)  │ │         │ │          │
    └────┬────┘ └───┬─────┘ └─┬───────┘ └───┬─────┘ └────┬─────┘
         │          │         │             │            │
         ▼          ▼         ▼             ▼            ▼
      claude      Anthropic  Go          git CLI     ~/.minibox/
      CLI         Messages   channels                ├─ agent-runs.jsonl
      subprocess  API                                └─ ai-logs/*.md
```

---

## 3. Council Data Flow

End-to-end sequence for `agentbox council`. After parsing flags, three adapters are wired in parallel: SDK runner, git context provider, and output writer. `BranchContext()` runs `git log`/`git diff` to build the review payload. Three reviewer roles (Strict Critic, Creative Explorer, General Analyst) each receive the same branch context and run sequentially as independent SDK queries with read-only tool access. Their scored outputs feed a final `RunSynthesis` call that produces a weighted verdict, which is written to both the JSONL telemetry log and a per-SHA Markdown report.

```
                          User: agentbox council --base main --mode core
                                          │
                                          ▼
                              ┌───────────────────────┐
                              │   Parse CLI Flags     │
                              │   base=main mode=core │
                              └───────────┬───────────┘
                                          │
                    ┌─────────────────────┼───────────────────────┐
                    │                     │                       │
                    ▼                     ▼                       ▼
          ┌─────────────────┐  ┌───────────────────┐   ┌──────────────────┐
          │ ClaudeSDKRunner │  │  GitProvider      │   │   DualWriter     │
          │ (AgentRunner)   │  │  (ContextProvider)│   │  (ResultWriter)  │
          └─────────────────┘  └────────┬──────────┘   └──────────────────┘
                                        │
                                        ▼
                              ┌──────────────────────┐
                              │  BranchContext()     │
                              │  git log main...HEAD │
                              │  git diff main...HEAD│
                              │  git rev-parse HEAD  │
                              └─────────┬────────────┘
                                        │
                           ┌────────────▼────────────┐
                           │   WriteRun("running")   │──→ ~/.minibox/agent-runs.jsonl
                           └────────────┬────────────┘
                                        │
                                        ▼
              ┌──────────────────────────────────────────────────┐
              │              RunRoles (sequential)               │
              │                                                  │
              │   ┌──────────────┐  ┌───────────────────────┐    │
              │   │ Strict       │  │ role.Prompt +         │    │
              │   │ Critic       │─▶│ "Analyse this branch" │───▶│ SDK
              │   │              │  │ + branchContext       │    │ Query
              │   └──────────────┘  └───────────────────────┘    │
              │                                                  │
              │   ┌──────────────┐  ┌───────────────────────┐    │
              │   │ Creative     │  │ role.Prompt +         │    │
              │   │ Explorer     │─▶│ "Analyse this branch" │───▶│ SDK
              │   │              │  │ + branchContext       │    │ Query
              │   └──────────────┘  └───────────────────────┘    │
              │                                                  │
              │   ┌──────────────┐  ┌───────────────────────┐    │
              │   │ General      │  │ role.Prompt +         │    │
              │   │ Analyst      │─▶│ "Analyse this branch" │───▶│ SDK
              │   │              │  │ + branchContext       │    │ Query
              │   └──────────────┘  └───────────────────────┘    │
              │                                                  │
              │   Tools: ["Read", "Glob", "Grep"] (read-only)    │
              └──────────────────────┬───────────────────────────┘
                                     │
                                     ▼
                        map[string]string {
                          "strict-critic":    "Score: 0.72 ...",
                          "creative-explorer": "Score: 0.91 ...",
                          "general-analyst":   "Score: 0.83 ...",
                        }
                                     │
                                     ▼
              ┌──────────────────────────────────────────────────┐
              │              RunSynthesis                        │
              │                                                  │
              │  SynthesisPrompt(roleOutputs, branchCtx)         │
              │  ┌────────────────────────────────────────────┐  │
              │  │ "You are synthesising a multi-role council │  │
              │  │  code review into a final verdict."        │  │
              │  │                                            │  │
              │  │  Required sections:                        │  │
              │  │  • Health Scores (weighted 1.5× critic)    │  │
              │  │  • Areas of Consensus                      │  │
              │  │  • Areas of Tension (dialectic format)     │  │
              │  │  • Balanced Recommendations (top 3–5)      │  │
              │  │  • Branch Health verdict                   │  │
              │  └────────────────────────────────────────────┘  │
              └──────────────────────┬───────────────────────────┘
                                     │
                        ┌────────────▼────────────┐
                        │  WriteRun("complete")   │──→ ~/.minibox/agent-runs.jsonl
                        │  WriteReport(council)   │──→ ~/.minibox/ai-logs/{sha}-council-core.md
                        └─────────────────────────┘
```

---

## 4. Meta-Agent Data Flow

Three-phase pipeline for `agentbox meta-agent`. Phase 1 (Design): a designer SDK query reads the repo and outputs a JSON array of 2–5 specialized sub-agents, each with a distinct role, prompt, and tool list; malformed JSON falls back to a single analyst. Phase 2 (Parallel Execution): each sub-agent runs in its own goroutine with an independent SDK query; results are collected via a channel and `WaitGroup`. Phase 3 (Synthesis): all sub-agent outputs are merged by a final SDK call that produces a deduplicated, ranked report, then written to disk.

```
          User: agentbox meta-agent "Find memory leaks in async handlers"
                                     │
                                     ▼
  ┌──────────────────────────────────────────────────────────────────────┐
  │ PHASE 1: DESIGN                                                      │
  │                                                                      │
  │   ┌───────────────────────────────────────────────────────────────┐  │
  │   │ DesignerPrompt(task, repoContext)                             │  │
  │   │                                                               │  │
  │   │ "You are a meta-agent designer. Given a task, repo context,   │  │
  │   │  design 2–5 parallel agents with distinct concerns.           │  │
  │   │  Output ONLY valid JSON array."                               │  │
  │   │                                                               │  │
  │   │ → SDK Query (tools: Read, Glob, Grep)                         │  │
  │   └──────────────────────────┬────────────────────────────────────┘  │
  │                              │                                       │
  │                              ▼                                       │
  │   ┌───────────────────────────────────────────────────────────────┐  │
  │   │ ParseAgentPlan(json_output)                                   │  │
  │   │                                                               │  │
  │   │ [                                                             │  │
  │   │   {"name":"async-tracer",  "role":"Trace async lifetimes",    │  │
  │   │    "prompt":"...", "tools":["Read","Glob","Grep"]},           │  │
  │   │   {"name":"cgroup-checker","role":"Check resource limits",    │  │
  │   │    "prompt":"...", "tools":["Read","Glob","Grep"]},           │  │
  │   │   {"name":"test-scanner", "role":"Find missing tests",        │  │
  │   │    "prompt":"...", "tools":["Read","Glob","Grep"]}            │  │
  │   │ ]                                                             │  │
  │   │                                                               │  │
  │   │ Fallback: if parse fails → single "analyst" agent             │  │
  │   └──────────────────────────┬────────────────────────────────────┘  │
  └──────────────────────────────┼───────────────────────────────────────┘
                                 │
                                 ▼
  ┌──────────────────────────────────────────────────────────────────────┐
  │ PHASE 2: PARALLEL EXECUTION                                          │
  │                                                                      │
  │   ┌──────────────────┐  ┌──────────────────┐  ┌───────────────────┐  │
  │   │   goroutine 1    │  │   goroutine 2    │  │   goroutine 3     │  │
  │   │                  │  │                  │  │                   │  │
  │   │  async-tracer    │  │  cgroup-checker  │  │  test-scanner     │  │
  │   │  ┌────────────┐  │  │  ┌────────────┐  │  │  ┌────────────┐   │  │
  │   │  │ SDK Query  │  │  │  │ SDK Query  │  │  │  │ SDK Query  │   │  │
  │   │  │ Read,Glob  │  │  │  │ Read,Glob  │  │  │  │ Read,Glob  │   │  │
  │   │  │ Grep       │  │  │  │ Grep       │  │  │  │ Grep       │   │  │
  │   │  └─────┬──────┘  │  │  └─────┬──────┘  │  │  └─────┬──────┘   │  │
  │   │        │         │  │        │         │  │        │          │  │
  │   └────────┼─────────┘  └────────┼─────────┘  └────────┼──────────┘  │
  │            │                     │                     │             │
  │            └─────────────────────┼─────────────────────┘             │
  │                                  │                                   │
  │                       ┌──────────▼──────────┐                        │
  │                       │  results channel    │                        │
  │                       │  + WaitGroup.Wait() │                        │
  │                       └──────────┬──────────┘                        │
  └──────────────────────────────────┼───────────────────────────────────┘
                                     │
                                     ▼
                         map[string]string {
                           "async-tracer":   "Found 3 lifetime issues...",
                           "cgroup-checker": "All limits validated...",
                           "test-scanner":   "Missing 5 test cases...",
                         }
                                     │
                                     ▼
  ┌──────────────────────────────────────────────────────────────────────┐
  │ PHASE 3: SYNTHESIS                                                   │
  │                                                                      │
  │  MetaSynthesisPrompt(task, agentOutputs)                             │
  │  ┌────────────────────────────────────────────────────────────────┐  │
  │  │ "Synthesize outputs from multiple parallel agents into a       │  │
  │  │  single coherent report."                                      │  │
  │  │                                                                │  │
  │  │  Required sections:                                            │  │
  │  │  • Summary (2–3 sentences)                                     │  │
  │  │  • Key Findings (deduplicated, grouped by theme)               │  │
  │  │  • Recommended Actions (ranked, who/what/why)                  │  │
  │  │  • Open Questions                                              │  │
  │  └────────────────────────────────────────────────────────────────┘  │
  │                                                                      │
  │  → SDK Query → Final synthesized report                              │
  └──────────────────────────────────┬───────────────────────────────────┘
                                     │
                        ┌────────────▼────────────┐
                        │  WriteRun("complete")   │──→ ~/.minibox/agent-runs.jsonl
                        │  WriteReport(meta)      │──→ ~/.minibox/ai-logs/{sha}-meta-agent.md
                        └─────────────────────────┘
```

---

## 5. LLM Provider Stack

Shows the two-layer resilience wrapper around `AnthropicProvider`. `RetryingProvider` wraps any inner provider and retries with exponential backoff (1s, 2s, …, max 30s) before giving up. `FallbackChain` tries a prioritized list of providers in order, returning the first success and surfacing a combined error only if all fail. `AnthropicProvider` is the concrete leaf: it calls `anthropic.Client.Messages.New()`, extracts text blocks, and returns a `CompletionResponse{Text, Provider}`.

```
                          CompletionRequest
                          {Prompt, System, MaxTokens}
                                  │
                                  ▼
                      ┌───────────────────────┐
                      │   RetryingProvider    │
                      │                       │
                      │   attempt 0 ──────────│──→ inner.Complete()
                      │     if err:           │         │
                      │   wait 1s             │         │ fail
                      │   attempt 1 ──────────│──→ inner.Complete()
                      │     if err:           │         │
                      │   wait 2s             │         │ fail
                      │   attempt 2 ──────────│──→ inner.Complete()
                      │     if err:           │         │
                      │   "retries exhausted" │         │ fail
                      │                       │         │
                      │   max backoff: 30s    │         │
                      │   max retries: config │         ▼
                      └───────────┬───────────┘   ┌───────────┐
                                  │               │  success  │
                                  ▼               └───────────┘
                      ┌───────────────────────┐
                      │    FallbackChain      │
                      │                       │
                      │  providers[0] ────────│──→ Complete()
                      │    if ok: return      │         │
                      │    if err:            │         │ fail
                      │  providers[1] ────────│──→ Complete()
                      │    if ok: return      │         │
                      │    if err:            │         │ fail
                      │  providers[N] ────────│──→ Complete()
                      │    if all err:        │
                      │  "all providers       │
                      │   failed: a; b; ..."  │
                      └───────────┬───────────┘
                                  │
                                  ▼
                      ┌───────────────────────┐
                      │  AnthropicProvider    │
                      │                       │
                      │  model: claude-sonnet │
                      │  default max: 1024    │
                      │                       │
                      │  anthropic.Client     │
                      │    .Messages.New()    │
                      │                       │
                      │  Extract text blocks  │──→ CompletionResponse
                      │  from response        │    {Text, Provider}
                      └───────────────────────┘
```

---

## 6. Agent SDK Execution Model

Illustrates how `ClaudeSDKRunner` translates an `AgentConfig` into a running Claude Code subprocess. `configToQueryOptions` maps the config's tool list and system prompt into SDK query options; `claudecode.Query()` spawns the `claude` CLI as a child process communicating over NDJSON stdin/stdout. The model runs its own agentic loop (think → tool use → observe → respond). The runner iterates the message stream, collecting `ResultMessage` values until `ErrNoMore`, and returns a single `AgentResult`.

```
                         AgentConfig
                         {Name, Prompt, Tools, SystemPrompt}
                                │
                                ▼
                    ┌───────────────────────┐
                    │   ClaudeSDKRunner     │
                    │                       │
                    │  configToQueryOptions │
                    │  ├─ WithAllowedTools  │
                    │  └─ WithSystemPrompt  │
                    │                       │
                    │  claudecode.Query()   │
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │  claude CLI process   │  ← spawned as subprocess
                    │                       │
                    │  NDJSON stdin/stdout  │
                    │                       │
                    │  Claude model         │
                    │  ┌─────────────────┐  │
                    │  │ Agent loop:     │  │
                    │  │ 1. Think        │  │
                    │  │ 2. Use tool     │──│──→ Read file
                    │  │ 3. Observe      │  │    Glob pattern
                    │  │ 4. Think again  │  │    Grep content
                    │  │ 5. Use tool     │──│──→ Bash command
                    │  │ 6. Respond      │  │    Write/Edit file
                    │  └─────────────────┘  │
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │  Message iterator     │
                    │                       │
                    │  for msg in iter:     │
                    │    if ResultMessage:  │
                    │      collect .Result  │
                    │  until ErrNoMore      │
                    └───────────┬───────────┘
                                │
                                ▼
                         AgentResult
                         {Name, Output}
```

---

## 7. Output Pipeline

Shows the dual-write fan-out at the end of every orchestrator run. `WriteRun` appends a JSONL record (start + completion with `duration_s` and full `output`) to the append-only `~/.minibox/agent-runs.jsonl` file consumed by `dashboard.py` and standup scripts. `WriteReport` writes a human-readable Markdown file per run to `~/.minibox/ai-logs/` named by commit SHA and script type. Both paths are wired through `DualWriter`, which implements `ResultWriter` and delegates to `JSONLWriter` and `ReportWriter` respectively.

```
  Orchestrator completes
         │
         ├──────────────────────────────────────┐
         │                                      │
         ▼                                      ▼
  ┌─────────────────────────────┐  ┌──────────────────────────────────┐
  │     WriteRun (telemetry)    │  │    WriteReport (human-readable)  │
  └──────────┬──────────────────┘  └──────────────┬───────────────────┘
             │                                    │
             ▼                                    ▼
  ┌──────────────────────────┐      ┌──────────────────────────────────┐
  │ ~/.minibox/agent-runs.jsonl  │      │ ~/.minibox/ai-logs/                  │
  │                          │      │                                  │
  │ Append-only JSONL:       │      │ One file per run:                │
  │                          │      │ {sha}-council-core.md            │
  │ {"run_id":"2026-03-26T.."│      │ {sha}-council-extensive.md       │
  │  "script":"council",     │      │ {sha}-meta-agent.md              │
  │  "status":"running",...} │      │                                  │
  │ {"run_id":"2026-03-26T.."│      │ Format:                          │
  │  "script":"council",     │      │ ┌──────────────────────────────┐ │
  │  "status":"complete",    │      │ │ # council · abc1234          │ │
  │  "duration_s":120.5,     │      │ │                              │ │
  │  "output":"..."}         │      │ │ - **base**: main             │ │
  │                          │      │ │ - **mode**: core             │ │
  │ Read by:                 │      │ │ - **date**: 2026-03-26       │ │
  │ • dashboard.py           │      │ │                              │ │
  │ • standup scripts        │      │ │ ---                          │ │
  │                          │      │ │                              │ │
  └──────────────────────────┘      │ │ [full markdown report]       │ │
                                    │ └──────────────────────────────┘ │
                                    └──────────────────────────────────┘


  DualWriter wiring:

    ┌──────────────┐
    │  DualWriter  │
    │              │
    │  WriteRun()  │──→ JSONLWriter.WriteRun()  ──→ agent-runs.jsonl
    │  WriteReport │──→ ReportWriter.WriteReport──→ ai-logs/{sha}-{script}.md
    │              │
    └──────────────┘
```

---

## 8. Pub/Sub Topology (Tier A — In-Process)

Describes `ChannelBroker`, the Tier A `MessageBroker` implementation. It maintains a `topics` map of string → `[]chan Message`. `Publish` is non-blocking: it attempts a channel send and silently drops messages to slow subscribers rather than blocking the publisher. `Subscribe` appends a new buffered channel (size 64) to a topic. `Close` closes all channels and clears the map. The lower half shows the Tier B upgrade path: replacing `ChannelBroker` with `NATSBroker` behind the same `domain.MessageBroker` interface requires no changes to orchestrators.

```
  Current state: ChannelBroker (Go channels, buffered 64)

  ┌──────────────────────────────────────────────────┐
  │              ChannelBroker                       │
  │                                                  │
  │  topics map:                                     │
  │                                                  │
  │  "agent.output"  ──→ [ch1, ch2]                  │
  │  "task.complete" ──→ [ch3]                       │
  │  "state.update"  ──→ [ch4, ch5, ch6]             │
  │                                                  │
  │  Publish("agent.output", msg)                    │
  │  ├─ RLock                                        │
  │  ├─ for each ch in topics["agent.output"]:       │
  │  │   select:                                     │
  │  │     case ch <- msg: ✓ delivered               │
  │  │     default:        ✗ dropped (non-blocking)  │
  │  └─ RUnlock                                      │
  │                                                  │
  │  Subscribe("agent.output")                       │
  │  ├─ Lock                                         │
  │  ├─ ch := make(chan Message, 64)                 │
  │  ├─ topics["agent.output"] = append(..., ch)     │
  │  └─ return ch                                    │
  │                                                  │
  │  Close()                                         │
  │  ├─ Lock                                         │
  │  ├─ close all channels                           │
  │  └─ clear topics                                 │
  └──────────────────────────────────────────────────┘


  Tier B upgrade path (future):

  ┌──────────────────┐         ┌──────────────────┐
  │  agentbox        │         │  NATS server     │
  │  (container A)   │         │  (sidecar)       │
  │                  │         │                  │
  │  NATSBroker      │◀──nats─▶│  subjects:       │
  │  (implements     │         │  agent.output    │
  │   MessageBroker) │         │  task.complete   │
  │                  │         │  state.update    │
  └──────────────────┘         └──────────────────┘
         │                              ▲
         │                              │
         └───── same interface ─────────┘
               domain.MessageBroker
```

---

## 9. Tool Allowlist Convention

Documents which tools each orchestrator grants to SDK agents and how tool safety is enforced. All read-only orchestrators (council, commit-msg) are limited to `Read`, `Glob`, `Grep`. Meta-agent spawned sub-agents default to read-only but the designer LLM may grant `Bash`, `Write`, `Edit` for modifier agents. After `parseAgentPlan` parses the designer's JSON output, each requested tool is validated against a static allowlist; unknown tools are rejected and empty tool lists fall back to the read-only default. The sanitized tool list is passed to `ClaudeSDKRunner` via `WithAllowedTools`, which the `claude` CLI enforces at the subprocess level.

```
  Orchestrator                 Tools Granted           Permission Level
  ─────────────────────────── ────────────────────── ──────────────────
  Council (all roles)          Read, Glob, Grep       READ-ONLY
  Council (synthesis)          Read, Glob, Grep       READ-ONLY
  Meta-agent (designer)        Read, Glob, Grep       READ-ONLY
  Meta-agent (spawned agents)  Read, Glob, Grep       READ-ONLY (default)
                               + Bash, Write, Edit    MODIFY (if designer grants)
  Meta-agent (synthesis)       Read, Glob, Grep       READ-ONLY
  CommitMsg                    Read, Glob, Grep       READ-ONLY
  Diagnose (future)            Bash, Read, Glob       READ + EXECUTE

  Tool Safety Flow:

  Designer LLM output
        │
        ▼
  parseAgentPlan()
        │
        ├─ validate each tool against allowlist:
        │   {"Read", "Glob", "Grep", "Bash", "Write", "Edit"}
        │
        ├─ unknown tools → rejected
        ├─ empty tools   → default to ["Read", "Glob", "Grep"]
        │
        ▼
  sanitized AgentSpec.Tools
        │
        ▼
  ClaudeSDKRunner
        │
        ▼
  WithAllowedTools(...)  → claude CLI enforces
```

---

## 10. Commit Message Flow

End-to-end sequence for `minibox-commit-msg`. The tool first verifies there is staged content (`git diff --cached`), collecting diff, stat, branch name, recent log, and working-tree status. If the diff exceeds 64 KB, only the stat summary is sent to reduce token cost. `CommitMsg.Generate()` calls the SDK with a conventional-commit prompt (type(scope): description, ≤72 chars, imperative mood). The generated message is printed to stdout; with `-c -y`, a `Co-Authored-By` trailer is appended and `git commit` runs automatically.

```
  User: minibox-commit-msg -a -c -y
         │
         ├─ -a: git add -A
         ├─ -c: commit after generating
         └─ -y: auto-confirm
                │
                ▼
  ┌─────────────────────────────────────────────┐
  │  Stage Check                                │
  │                                             │
  │  git diff --cached → stagedDiff             │
  │  git diff --cached --stat → stagedStat      │
  │                                             │
  │  if empty:                                  │
  │    "Nothing staged." or "Working tree clean"│
  │    exit(1)                                  │
  └─────────────────────┬───────────────────────┘
                        │
                        ▼
  ┌─────────────────────────────────────────────┐
  │  Context Collection                         │
  │                                             │
  │  Branch:      git rev-parse --abbrev-ref    │
  │  RecentLog:   git log -8 --oneline          │
  │  Status:      git status --short            │
  │  UnstagedStat: git diff --stat              │
  │                                             │
  │  if stagedDiff > 64KB:                      │
  │    "(diff too large — XXX KB; stat only)"   │
  └─────────────────────┬───────────────────────┘
                        │
                        ▼
  ┌─────────────────────────────────────────────┐
  │  CommitMsg.Generate()                       │
  │                                             │
  │  Prompt rules:                              │
  │  • Follow existing commit style             │
  │  • Conventional: type(scope): description   │
  │  • ≤72 chars, imperative, no period         │
  │  • NO Co-Authored-By (added separately)     │
  │                                             │
  │  → SDK Query → commit message               │
  └─────────────────────┬───────────────────────┘
                        │
                        ▼
  ┌─────────────────────────────────────────────┐
  │  Output                                     │
  │                                             │
  │  ─────────────────────────────────────────  │
  │  feat(agentbox): add council orchestrator   │
  │                                             │
  │  Add multi-role analysis with 5 reviewer    │
  │  perspectives and weighted synthesis.       │
  │  ─────────────────────────────────────────  │
  │                                             │
  │  if -c and -y:                              │
  │    append Co-Authored-By                    │
  │    git commit -m "{msg}"                    │
  │    "Committed."                             │
  └─────────────────────────────────────────────┘
```

---

## 11. Telemetry Record Format

Shows the two-record lifecycle for each agent run in `agent-runs.jsonl`. A "running" record is written immediately at start (allowing crash detection by looking for runs with no matching "complete" record). A "complete" record is appended on success with `duration_s` and the full `output` text. The lower section shows the companion per-run Markdown report format written to `ai-logs/`: a frontmatter header (base, mode, date) followed by the full multi-role analysis and synthesis narrative.

```
  JSONL Record Lifecycle (agent-runs.jsonl):

  Run Start:
  ┌──────────────────────────────────────────────────────────────────┐
  │ {"run_id":"2026-03-26T14:30:45Z",                                │
  │  "script":"council",                                             │
  │  "args":{"base":"main","mode":"core"},                           │
  │  "status":"running"}                                             │
  └──────────────────────────────────────────────────────────────────┘

  Run Complete:
  ┌──────────────────────────────────────────────────────────────────┐
  │ {"run_id":"2026-03-26T14:30:45Z",                                │
  │  "script":"council",                                             │
  │  "args":{"base":"main","mode":"core"},                           │
  │  "status":"complete",                                            │
  │  "duration_s":120.5,                                             │
  │  "output":"## Strict Critic\nScore: 0.72\n..."}                  │
  └──────────────────────────────────────────────────────────────────┘


  Markdown Report Format (ai-logs/{sha}-{script}.md):

  ┌──────────────────────────────────────────────────────────────────┐
  │ # council · abc1234                                              │
  │                                                                  │
  │ - **base**: main                                                 │
  │ - **mode**: core                                                 │
  │ - **date**: 2026-03-26 14:30                                     │
  │                                                                  │
  │ ---                                                              │
  │                                                                  │
  │ ## Strict Critic                                                 │
  │ **Health Score**: 0.72                                           │
  │ ...                                                              │
  │                                                                  │
  │ ## Creative Explorer                                             │
  │ **Health Score**: 0.91                                           │
  │ ...                                                              │
  │                                                                  │
  │ ## Synthesis                                                     │
  │ **Health Scores**: ...weighted average...                        │
  │ **Branch Health**: Good                                          │
  └──────────────────────────────────────────────────────────────────┘
```

---

## 12. Dependency Graph

Maps all import relationships across the module. Two external dependencies: `anthropic-sdk-go` (used only in `internal/llm/`) and `claude-agent-sdk-go` (used only in `internal/agent/`). All other packages use only the Go standard library. The internal graph shows a strict DAG: both binaries import from the orchestration and infrastructure packages, which all converge downward on `internal/domain` — the only package with no outward dependencies. This structure prevents import cycles and ensures domain types are never coupled to infrastructure.

```
  External Dependencies:

  github.com/anthropics/anthropic-sdk-go
       │
       └──→ internal/llm/anthropic.go (AnthropicProvider)

  github.com/severity1/claude-agent-sdk-go
       │
       └──→ internal/agent/sdk.go (ClaudeSDKRunner)

  standard library only:
       │
       ├──→ internal/domain/       (encoding/json, time, context)
       ├──→ internal/pubsub/       (sync, context, encoding/json)
       ├──→ internal/output/       (os, path/filepath, encoding/json, fmt)
       ├──→ internal/context/      (os, os/exec, path/filepath, strings, fmt)
       ├──→ internal/orchestrator/ (context, fmt, strings, sync, encoding/json)
       ├──→ internal/tools/        (context, fmt)
       ├──→ cmd/agentbox/          (flag, fmt, os, time)
       └──→ cmd/minibox-commit-msg/   (flag, fmt, os, os/exec, time, bufio, strings)


  Internal Dependency Graph:

  cmd/agentbox ──────┐
  cmd/minibox-commit-msg ┤
                     │
                     ├──→ internal/orchestrator
                     │    ├──→ internal/domain
                     │    └──→ (uses AgentRunner interface)
                     │
                     ├──→ internal/tools
                     │    └──→ internal/domain
                     │
                     ├──→ internal/agent
                     │    ├──→ internal/domain
                     │    └──→ claude-agent-sdk-go
                     │
                     ├──→ internal/llm
                     │    ├──→ internal/domain
                     │    └──→ anthropic-sdk-go
                     │
                     ├──→ internal/context
                     │    └──→ internal/domain
                     │
                     ├──→ internal/output
                     │    └──→ internal/domain
                     │
                     └──→ internal/pubsub
                          └──→ internal/domain
```

---

## 13. Tier A → B → C Evolution

Roadmap for agentbox's three deployment tiers. Tier A (current): a single Go binary runs all agents in-process using Go channels; output goes to `~/.minibox/` files. Tier B (future): `agentboxd` becomes a standalone daemon communicating with `miniboxd` over NATS; individual council/review/commit agents are separate processes dispatched by the daemon. Tier C (future): a capability registry stores agent manifests (inputs, outputs, tool allowlists) that can be auto-discovered via OCI labels or a NATS service registry, enabling dynamic agent composition without code changes.

```
  ┌────────────────────────────────────────────────────────────────────┐
  │ TIER A (current): In-Container Agents                              │
  │                                                                    │
  │  ┌─────────────┐                                                   │
  │  │ Container   │  agentbox binary + claude CLI                     │
  │  │             │  Go channels pub/sub                              │
  │  │  agentbox   │  ~/.minibox/ output files                             │
  │  │  council    │                                                   │
  │  │             │  All in one process                               │
  │  └─────────────┘                                                   │
  └────────────────────────────────────────────────────────────────────┘
                          │
                          ▼
  ┌──────────────────────────────────────────────────────────────────┐
  │ TIER B (future): Agent Runtime Daemon                            │
  │                                                                  │
  │  ┌─────────────┐    NATS    ┌─────────────┐                      │
  │  │ miniboxd    │◀──────────▶│ agentboxd   │                      │
  │  │ (container  │            │ (agent      │                      │
  │  │  runtime)   │            │  runtime)   │                      │
  │  └─────────────┘            └──────┬──────┘                      │
  │                                    │                             │
  │                         ┌──────────┼──────────┐                  │
  │                         ▼          ▼          ▼                  │
  │                    ┌─────────┐ ┌────────┐ ┌────────┐             │
  │                    │ council │ │ review │ │ commit │             │
  │                    │ agent   │ │ agent  │ │ agent  │             │
  │                    └─────────┘ └────────┘ └────────┘             │
  └──────────────────────────────────────────────────────────────────┘
                          │
                          ▼
  ┌───────────────────────────────────────────────────────────────────┐
  │ TIER C (future): Discoverable Tool Agents                         │
  │                                                                   │
  │  ┌─────────────┐         ┌─────────────────────────┐              │
  │  │ Registry    │         │ Capability Manifests    │              │
  │  │             │         │                         │              │
  │  │ agents:     │◀───────▶│ council:                │              │
  │  │  council    │         │   inputs: [base, mode]  │              │
  │  │  review     │         │   outputs: [report]     │              │
  │  │  diagnose   │         │   tools: [Read,Glob]    │              │
  │  │  commit-msg │         │                         │              │
  │  │  ...        │         │ review:                 │              │
  │  │             │         │   inputs: [diff]        │              │
  │  └─────────────┘         │   outputs: [findings]   │              │
  │                          │   tools: [Read,Glob]    │              │
  │  Auto-discovery via      └─────────────────────────┘              │
  │  OCI labels or NATS                                               │
  │  service registry                                                 │
  └───────────────────────────────────────────────────────────────────┘
```
