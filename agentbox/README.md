# agentbox

Go agent runtime for the minibox workspace. Provides multi-role code review
(council), parallel task decomposition (meta-agent), and AI-assisted commit
message generation — all backed by the Claude Agent SDK.

## Binaries

### `agentbox`

Subcommand CLI with two modes:

```
agentbox council [--base main] [--mode core|extensive] [--no-synthesis]
agentbox meta-agent [--no-synthesis] <task description>
```

**council** — Multi-role code review against a base branch. Core mode runs 3
roles (strict critic, creative explorer, general analyst); extensive adds
security reviewer and performance analyst. Each role analyses the branch diff,
then a synthesis agent merges findings into a weighted verdict with health
scores.

**meta-agent** — Takes a freeform task description, spawns a designer agent
that decomposes it into 2-5 parallel agents with distinct concerns, executes
them concurrently, and synthesises the results.

### `mbx-commit-msg`

Standalone commit message generator.

```
mbx-commit-msg [-a] [-c] [-y]
```

| Flag | Effect                                              |
| ---- | --------------------------------------------------- |
| `-a` | Stage all changes (`git add -A`) before generating  |
| `-c` | Prompt to commit with the generated message         |
| `-y` | Skip confirmation and commit immediately (implies `-c`) |

Produces conventional commit messages (`type(scope): description`) matching the
repository's existing style.

## Architecture

```
cmd/
  agentbox/         CLI entry point (council + meta-agent subcommands)
  mbx-commit-msg/   Standalone commit message tool
internal/
  domain/           Core types and interfaces (AgentRunner, LlmProvider,
                    MessageBroker, ContextProvider, ResultWriter)
  agent/            Claude Agent SDK runner (subprocess-based)
  llm/              Anthropic provider, fallback chain, retry with backoff
  orchestrator/     Council roles + synthesis, meta-agent design/spawn/synth
  context/          Git context provider (branch, diff, project rules, structure)
  output/           Dual writer — JSONL telemetry + markdown reports
  pubsub/           In-process channel-based message broker
  tools/            Commit message generation tool
```

Domain interfaces decouple agent execution, LLM access, context gathering, and
output persistence. The `agent` package implements `AgentRunner` via the Claude
Agent SDK Go wrapper (`severity1/claude-agent-sdk-go`), which shells out to the
`claude` CLI. The `llm` package provides a direct Anthropic SDK client with
retry and provider-chain fallback for standalone completions.

## Output

All runs write telemetry and reports to `~/.minibox/`:

| Path                            | Format  | Content                        |
| ------------------------------- | ------- | ------------------------------ |
| `~/.minibox/agent-runs.jsonl`   | JSONL   | Run records (id, timing, args) |
| `~/.minibox/ai-logs/<sha>-*.md` | Markdown | Full role outputs + synthesis  |

## Requirements

- Go 1.26+
- `claude` CLI on PATH (Claude Agent SDK runs agents as subprocesses)
- `ANTHROPIC_API_KEY` in environment (for direct LLM provider usage)
- Git repository context (runs `git diff`, `git log`, etc.)

## Build

```
cd agentbox
go build -o bin/agentbox ./cmd/agentbox
go build -o bin/mbx-commit-msg ./cmd/mbx-commit-msg
```

## Test

```
go test ./...
```
