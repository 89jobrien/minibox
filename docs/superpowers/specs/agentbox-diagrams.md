# Agentbox Architecture Diagrams

**Date**: 2026-03-26
**Companion to**: `2026-03-25-agentbox-design.md`
**Scope**: Data flow, component relationships, and conventions for the Tier A implementation

---

## 1. System Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          agentbox/ Go module                           │
│                                                                        │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │                        Binary Layer                                │ │
│  │  ┌──────────────────────┐  ┌───────────────────────────────────┐  │ │
│  │  │   cmd/agentbox/      │  │   cmd/mbx-commit-msg/             │  │ │
│  │  │   • council           │  │   • standalone commit-msg tool    │  │ │
│  │  │   • meta-agent        │  │                                   │  │ │
│  │  └──────────┬───────────┘  └──────────────┬────────────────────┘  │ │
│  └─────────────┼─────────────────────────────┼───────────────────────┘ │
│                │                             │                         │
│  ┌─────────────▼─────────────────────────────▼───────────────────────┐ │
│  │                     Orchestration Layer                            │ │
│  │  ┌─────────────────────┐  ┌──────────────────┐  ┌──────────────┐ │ │
│  │  │ orchestrator/       │  │ tools/            │  │ agent/       │ │ │
│  │  │ • Council           │  │ • CommitMsg       │  │ • SDKRunner  │ │ │
│  │  │ • MetaAgent         │  │                   │  │              │ │ │
│  │  └────────┬────────────┘  └────────┬─────────┘  └──────┬───────┘ │ │
│  └───────────┼────────────────────────┼────────────────────┼─────────┘ │
│              │                        │                    │           │
│  ┌───────────▼────────────────────────▼────────────────────▼─────────┐ │
│  │                     Infrastructure Layer                          │ │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────────┐ │ │
│  │  │ llm/     │  │ context/ │  │ output/  │  │ pubsub/          │ │ │
│  │  │ Anthropic│  │ Git CLI  │  │ JSONL    │  │ ChannelBroker    │ │ │
│  │  │ Chain    │  │ Rules    │  │ Report   │  │ (Go channels)    │ │ │
│  │  │ Retry    │  │ Diff     │  │ Dual     │  │                  │ │ │
│  │  └──────────┘  └──────────┘  └──────────┘  └──────────────────┘ │ │
│  └───────────────────────────────────────────────────────────────────┘ │
│                                                                        │
│  ┌────────────────────────────────────────────────────────────────────┐ │
│  │                        Domain Layer                                │ │
│  │  domain/types.go      Message, AgentConfig, AgentResult, AgentRun │ │
│  │  domain/interfaces.go AgentRunner, LlmProvider, MessageBroker,    │ │
│  │                       ContextProvider, ResultWriter               │ │
│  └────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Hexagonal Architecture — Ports and Adapters

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
    │ agentbox  │         │ mbx-       │         │  HTTP API    │
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
      claude      Anthropic  Go          git CLI     ~/.mbx/
      CLI         Messages   channels                ├─ agent-runs.jsonl
      subprocess  API                                └─ ai-logs/*.md
```

---

## 3. Council Data Flow

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
                           │   WriteRun("running")   │──→ ~/.mbx/agent-runs.jsonl
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
                        │  WriteRun("complete")   │──→ ~/.mbx/agent-runs.jsonl
                        │  WriteReport(council)   │──→ ~/.mbx/ai-logs/{sha}-council-core.md
                        └─────────────────────────┘
```

---

## 4. Meta-Agent Data Flow

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
                        │  WriteRun("complete")   │──→ ~/.mbx/agent-runs.jsonl
                        │  WriteReport(meta)      │──→ ~/.mbx/ai-logs/{sha}-meta-agent.md
                        └─────────────────────────┘
```

---

## 5. LLM Provider Stack

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
  │ ~/.mbx/agent-runs.jsonl  │      │ ~/.mbx/ai-logs/                  │
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

```
  User: mbx-commit-msg -a -c -y
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
       └──→ cmd/mbx-commit-msg/   (flag, fmt, os, os/exec, time, bufio, strings)


  Internal Dependency Graph:

  cmd/agentbox ──────┐
  cmd/mbx-commit-msg ┤
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

```
  ┌────────────────────────────────────────────────────────────────────┐
  │ TIER A (current): In-Container Agents                              │
  │                                                                    │
  │  ┌─────────────┐                                                   │
  │  │ Container   │  agentbox binary + claude CLI                     │
  │  │             │  Go channels pub/sub                              │
  │  │  agentbox   │  ~/.mbx/ output files                             │
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
