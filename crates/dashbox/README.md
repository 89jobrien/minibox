# dashbox

Ratatui TUI dashboard for minibox. Provides a live terminal interface for monitoring containers, benchmarks, CI status, git activity, todos, and architecture diagrams.

## Tabs

| Tab | Description |
|-----|-------------|
| Agents | AI agent run history from `~/.mbx/agent-runs.jsonl` |
| Bench | Benchmark results from `bench/results/` |
| History | Git commit timeline |
| Git | Working tree status and branch info |
| Todos | Doob todo list for the current project |
| CI | GitHub Actions run status |
| Diagrams | Mermaid architecture diagrams (built-in + user-defined `.mmd` files) |

## Usage

```bash
just dash
# or
./target/release/dashbox
```

## Diagrams

Built-in diagrams are embedded as `.mmd` source files in `src/diagrams/`. User-defined diagrams are loaded from `~/.mbx/diagrams/*.mmd` at startup.
