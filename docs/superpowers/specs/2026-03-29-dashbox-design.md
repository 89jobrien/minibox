# Dashbox — Ratatui TUI Dashboard

**Date:** 2026-03-29
**Crate:** `dashbox` (new workspace member)
**Binary:** `dashbox`

## Overview

Interactive terminal dashboard for minibox project health. Six tabs displaying agent runs, benchmarks, bench history trends, git status, doob todos, and GitHub Actions CI status. Supports running tests and benchmarks from within the TUI. Built with Ratatui + Crossterm.

## Tabs

### 1. Agents

Displays data from `~/.mbx/agent-runs.jsonl`.

- **Header panel:** total runs, complete/running/crashed counts (colored)
- **Summary table:** per-script stats (runs, avg duration, last run, last output)
- **History table:** recent 20 runs with status indicator, duration, output preview
- **Actions:** `Enter` expands selected run to show full output

### 2. Benchmarks

Displays data from `bench/results/latest.json` + `bench/results/bench.jsonl`.

- **Header panel:** git SHA, hostname, timestamp, regression count, VPS run count
- **Results table:** suite, test, avg, p95, min, iterations, delta vs previous run
- Delta coloring: >10% red, <-10% green, else dim
- **Actions:**
  - `t` — run `cargo xtask test-unit` (inline pane, streaming output)
  - `b` — run `cargo xtask bench` locally (inline pane, streaming output)
  - `B` — run `cargo xtask bench-vps` (background, notification on completion)

### 3. Bench History

Displays trend data from `bench/results/bench.jsonl` (VPS runs only).

- **Sparkline chart** per test showing avg duration across last N runs (ratatui Sparkline widget)
- **Table** with test name, current avg, best avg, worst avg, trend direction (arrow)
- Selectable test list on left, chart on right (two-pane layout)

### 4. Git

Data from `git` CLI commands.

- **Branch info:** current branch, ahead/behind origin, clean/dirty status
- **Recent commits:** last 15 commits (hash, author, age, message)
- **Changed files:** unstaged + staged file list with modification type (M/A/D/R)

### 5. Todos

Data from `doob todo list --json --project minibox`.

- **Pending table:** priority, content (truncated), tags, created date
- **Counts:** pending/completed/total in header
- Sorted by priority descending

### 6. CI

Data from `gh run list --json conclusion,status,headBranch,createdAt,name,databaseId --limit 10`.

- **Workflow runs table:** workflow name, branch, status/conclusion, timestamp, duration
- Status coloring: success=green, failure=red, in_progress=yellow, cancelled=dim
- **Header:** overall health indicator (last N green/red ratio)
- **Actions:** `o` opens selected run URL in browser

## Command Execution

Two modes for running commands from the TUI:

### Inline Pane (short commands)

For tests and local benchmarks. Splits the current tab vertically — table in top half, streaming command output in bottom half. Output scrolls automatically. `Esc` closes the pane.

Used by: `t` (test-unit), `b` (bench local)

Implementation: spawn child process with piped stdout/stderr, read in a background thread via `mpsc::channel`, render captured lines in the bottom pane on each draw tick.

### Background + Notification (long commands)

For VPS benchmarks. Command runs in a background thread. Status bar at bottom shows "VPS bench running..." with elapsed time. On completion, flashes a notification ("VPS bench complete: 3 regressions" or "VPS bench complete: all clear") and auto-refreshes the bench tab data.

Used by: `B` (bench-vps)

Implementation: spawn thread, store `JoinHandle` + `Arc<AtomicBool>` for completion flag. Main loop checks flag each tick.

## Architecture

```
dashbox/
├── Cargo.toml
└── src/
    ├── main.rs          # Entry point, event loop, terminal setup/teardown
    ├── app.rs           # App state, tab switching, tick handling, command state
    ├── ui.rs            # Top-level layout: tab bar + active tab content + status bar
    ├── command.rs       # Command runner: inline pane + background execution
    ├── tabs/
    │   ├── mod.rs       # Tab trait definition
    │   ├── agents.rs    # Tab 1
    │   ├── bench.rs     # Tab 2
    │   ├── history.rs   # Tab 3
    │   ├── git.rs       # Tab 4
    │   ├── todos.rs     # Tab 5
    │   └── ci.rs        # Tab 6
    └── data/
        ├── mod.rs       # DataSource trait definition
        ├── agents.rs    # Parse agent-runs.jsonl
        ├── bench.rs     # Parse bench JSONL + latest.json
        ├── git.rs       # Run git commands, parse output
        ├── todos.rs     # Run doob, parse JSON
        └── ci.rs        # Run gh, parse JSON
```

### Hexagonal Structure

**Domain traits (ports):**

- `DataSource<T>` — trait for loading tab data. Methods: `load() -> Result<T>`, `is_stale() -> bool`. Implemented by each `data/` module.
- `TabRenderer` — trait for rendering a tab. Methods: `render(frame, area, state)`, `handle_key(key) -> TabAction`, `title() -> &str`. Implemented by each `tabs/` module.
- `TabAction` — enum returned by key handlers: `None`, `Scroll(direction)`, `RunInline(Command)`, `RunBackground(Command)`, `OpenUrl(String)`, `ExpandRow`.

**Adapters:**
- `data/*.rs` — concrete `DataSource` implementations (file readers, CLI wrappers)
- `tabs/*.rs` — concrete `TabRenderer` implementations (ratatui widgets)

**Composition root:** `main.rs` wires data sources to tab renderers via `App`.

## Dependencies

```toml
[dependencies]
ratatui = "0.29"
crossterm = "0.28"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
dirs = "6"
```

No async runtime. Background commands use `std::thread` + `mpsc::channel`. Data loading uses `std::process::Command` and `std::fs`.

## Event Loop

```
loop {
    terminal.draw(|f| ui::render(f, &app))?;
    if crossterm::event::poll(Duration::from_millis(250))? {
        match crossterm::event::read()? {
            Key('q') | Key(Esc)  => app.handle_quit_or_close(),
            Key('1'..='6')       => app.select_tab(n),
            Key(Left)            => app.prev_tab(),
            Key(Right)           => app.next_tab(),
            Key(Up/Down/j/k)     => app.scroll(direction),
            Key('r')             => app.refresh_current_tab(),
            other                => app.forward_to_tab(other),
        }
    }
    app.poll_commands();  // check inline/background command channels
    if app.should_refresh() {
        app.refresh_current_tab();
    }
}
```

`handle_quit_or_close`: if inline pane is open, Esc closes it. If no pane, q/Esc quits.

## Tab Bar

Rendered as a `Tabs` widget at the top of the screen. Active tab highlighted. Format:

```
 [1 Agents] [2 Bench] [3 History] [4 Git] [5 Todos] [6 CI]
```

## Status Bar

Bottom row of the screen. Shows:
- Left: available keys for current tab (e.g., `t:test  b:bench  B:vps-bench  r:refresh`)
- Right: background task status if running (e.g., `VPS bench 42s...`)
- Notification flash: on background completion, highlight for 5s then fade

## Data Refresh Strategy

- **On tab switch:** refresh that tab's data if stale (>10s since last load)
- **Timer tick:** refresh active tab every 10s
- **Manual:** `r` forces immediate refresh
- **Startup:** load all tabs eagerly (fast enough for file reads + 3 CLI calls)
- **Post-command:** auto-refresh relevant tab when inline/background command completes

## Scrolling

Tabs with tables support vertical scrolling via `TableState`. Up/Down/j/k move selection. The bench history tab additionally supports left/right to select different tests.

## Error Handling

CLI commands that fail (doob not installed, gh not authed) show an inline error message in the tab content area rather than crashing. Each `data/` module returns `Result<T>` — tabs render the error string on failure.

## Integration

- **Justfile:** `just dash` recipe
- **mise.toml:** `all:dash` task
- Workspace member in root `Cargo.toml`
- Binary output: `target/release/dashbox`
