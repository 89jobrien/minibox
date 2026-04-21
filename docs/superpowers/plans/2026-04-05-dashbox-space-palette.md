---
status: done
---

# Dashbox Space-Leader Command Palette Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace all direct-letter action keys in dashbox tabs with a Space-triggered floating
command palette overlay, keeping navigation keys (`j/k`, `↑/↓`, `Enter`, `1-9`, `r`, `q`, `Esc`)
as direct bindings.

**Architecture:** A `CommandPalette` struct lives on `App`. Pressing `Space` collects
`palette_actions() -> Vec<PaletteEntry>` from the active tab and opens the palette. The palette
renders as a centered floating overlay in `ui.rs`. A key press matching any `PaletteEntry.key`
fires its `TabAction`; `Esc`/`Space` dismisses. All existing direct action keys (`t`, `b`, `c`,
`o`, `d`, `D`, `B`) are removed from `handle_key`.

**Tech Stack:** Rust 2024, ratatui 0.29, crossterm 0.28. No new dependencies.

---

## File Map

| File | Change |
|------|--------|
| `crates/dashbox/src/tabs/mod.rs` | Add `PaletteEntry` type; add `palette_actions()` to `TabRenderer` |
| `crates/dashbox/src/palette.rs` | New — `CommandPalette` struct and render logic |
| `crates/dashbox/src/app.rs` | Add `palette: Option<CommandPalette>` field |
| `crates/dashbox/src/ui.rs` | Render palette overlay; update status bar hints |
| `crates/dashbox/src/main.rs` | Route `Space` to open palette; route keys through palette |
| `crates/dashbox/src/tabs/bench.rs` | Remove `t/b/B` from `handle_key`; add `palette_actions` |
| `crates/dashbox/src/tabs/ci.rs` | Remove `o` from `handle_key`; add `palette_actions` |
| `crates/dashbox/src/tabs/items.rs` | Remove `c/o/b` from `handle_key`; add `palette_actions` |
| `crates/dashbox/src/tabs/todos.rs` | Remove `c/o` from `handle_key`; add `palette_actions` |
| `crates/dashbox/src/tabs/diagrams.rs` | Remove `d/D` from `handle_key`; add `palette_actions` |
| `crates/dashbox/src/tabs/agents.rs` | `palette_actions` empty (Enter detail stays direct) |
| `crates/dashbox/src/tabs/git.rs` | No change needed |
| `crates/dashbox/src/tabs/history.rs` | No change needed |
| `crates/dashbox/src/tabs/metrics.rs` | Remove `r` from `handle_key` (global `r` covers it) |

---

## Task 1: Add `PaletteEntry` type and `palette_actions()` to `TabRenderer`

**Files:**
- Modify: `crates/dashbox/src/tabs/mod.rs`

- [ ] **Step 1: Add `PaletteEntry` and update `TabRenderer`**

Replace the contents of `crates/dashbox/src/tabs/mod.rs` with:

```rust
// dashbox/src/tabs/mod.rs
pub mod agents;
pub mod bench;
pub mod ci;
pub mod diagrams;
pub mod git;
pub mod history;
pub mod items;
pub mod metrics;
pub mod todos;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;

#[allow(dead_code)]
pub enum TabAction {
    None,
    Quit,
    RunInline {
        cmd: String,
        args: Vec<String>,
    },
    RunBackground {
        cmd: String,
        args: Vec<String>,
        label: String,
    },
    OpenUrl(String),
}

/// One entry in the command palette for a tab.
pub struct PaletteEntry {
    /// Single character the user presses to trigger this action.
    pub key: char,
    /// Human-readable label shown in the palette overlay.
    pub label: &'static str,
    /// Action to fire when this entry is selected.
    pub action: TabAction,
}

pub trait TabRenderer {
    fn render(&mut self, frame: &mut Frame, area: Rect);
    fn handle_key(&mut self, key: KeyEvent) -> TabAction;
    fn refresh(&mut self);
    fn status_keys(&self) -> &'static str;
    /// Returns the actions available in the command palette for this tab.
    /// Default: no actions.
    fn palette_actions(&mut self) -> Vec<PaletteEntry> {
        vec![]
    }
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p dashbox 2>&1 | tail -5
```

Expected: `Finished` with no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/dashbox/src/tabs/mod.rs
git commit -m "feat(dashbox): add PaletteEntry type and palette_actions() to TabRenderer"
```

---

## Task 2: Create `CommandPalette` in `palette.rs`

**Files:**
- Create: `crates/dashbox/src/palette.rs`

- [ ] **Step 1: Write the test first**

Create `crates/dashbox/src/palette.rs` with tests at the bottom:

```rust
// dashbox/src/palette.rs
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Row, Table, TableState};

use crate::tabs::{PaletteEntry, TabAction};

pub struct CommandPalette {
    entries: Vec<PaletteEntry>,
    state: TableState,
}

impl CommandPalette {
    pub fn new(entries: Vec<PaletteEntry>) -> Self {
        let mut state = TableState::default();
        if !entries.is_empty() {
            state.select(Some(0));
        }
        Self { entries, state }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn select_next(&mut self) {
        self.state.select_next();
    }

    pub fn select_previous(&mut self) {
        self.state.select_previous();
    }

    /// If `ch` matches any entry key, return that entry's action (consuming self).
    /// If Enter is pressed, fire the currently selected entry.
    /// Returns None if no match.
    pub fn handle_key(mut self, ch: Option<char>, enter: bool) -> Option<TabAction> {
        if enter {
            if let Some(idx) = self.state.selected() {
                if let Some(entry) = self.entries.into_iter().nth(idx) {
                    return Some(entry.action);
                }
            }
            return None;
        }
        if let Some(c) = ch {
            for entry in self.entries {
                if entry.key == c {
                    return Some(entry.action);
                }
            }
        }
        None
    }

    /// Render the palette as a centered floating overlay.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let label_width = self
            .entries
            .iter()
            .map(|e| e.label.len())
            .max()
            .unwrap_or(10) as u16;
        // box width: 2 (border) + 2 (key col) + 2 (gap) + label_width + 1 (padding)
        let box_w = (label_width + 7).max(20);
        let box_h = self.entries.len() as u16 + 2; // border top+bottom

        // Centre the box
        let x = area.x + area.width.saturating_sub(box_w) / 2;
        let y = area.y + area.height.saturating_sub(box_h) / 2;
        let popup_area = Rect { x, y, width: box_w.min(area.width), height: box_h.min(area.height) };

        // Clear background so the overlay is opaque
        frame.render_widget(Clear, popup_area);

        let widths = [Constraint::Length(2), Constraint::Fill(1)];
        let rows: Vec<Row> = self
            .entries
            .iter()
            .map(|e| {
                Row::new([
                    ratatui::widgets::Cell::from(Span::styled(
                        e.key.to_string(),
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    )),
                    ratatui::widgets::Cell::from(e.label),
                ])
            })
            .collect();

        let table = Table::new(rows, widths)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Actions ")
                    .title_alignment(Alignment::Center)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_stateful_widget(table, popup_area, &mut self.state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: char, label: &'static str) -> PaletteEntry {
        PaletteEntry { key, label, action: TabAction::None }
    }

    #[test]
    fn key_match_fires_action() {
        let palette = CommandPalette::new(vec![entry('t', "run tests")]);
        let result = palette.handle_key(Some('t'), false);
        assert!(matches!(result, Some(TabAction::None)));
    }

    #[test]
    fn no_match_returns_none() {
        let palette = CommandPalette::new(vec![entry('t', "run tests")]);
        let result = palette.handle_key(Some('x'), false);
        assert!(result.is_none());
    }

    #[test]
    fn enter_fires_selected() {
        let palette = CommandPalette::new(vec![entry('t', "run tests")]);
        let result = palette.handle_key(None, true);
        assert!(matches!(result, Some(TabAction::None)));
    }

    #[test]
    fn empty_palette_enter_returns_none() {
        let palette = CommandPalette::new(vec![]);
        let result = palette.handle_key(None, true);
        assert!(result.is_none());
    }
}
```

- [ ] **Step 2: Register the module in `main.rs`**

In `crates/dashbox/src/main.rs`, add `mod palette;` after the existing `mod` declarations:

```rust
mod app;
mod command;
mod data;
mod diagram;
mod diagrams;
mod palette;   // add this line
mod tabs;
mod ui;
```

- [ ] **Step 3: Run the tests**

```bash
cargo test -p dashbox palette 2>&1 | tail -15
```

Expected: `test result: ok. 4 passed`.

- [ ] **Step 4: Commit**

```bash
git add crates/dashbox/src/palette.rs crates/dashbox/src/main.rs
git commit -m "feat(dashbox): add CommandPalette struct with render and key handling"
```

---

## Task 3: Wire `CommandPalette` into `App`

**Files:**
- Modify: `crates/dashbox/src/app.rs`

- [ ] **Step 1: Add `palette` field and `open_palette`/`close_palette` methods**

In `crates/dashbox/src/app.rs`, add the import and field:

```rust
use crate::palette::CommandPalette;
```

Add `palette: Option<CommandPalette>` to the `App` struct after `pending_refresh`:

```rust
pub struct App {
    pub active_tab: Tab,
    pub should_quit: bool,
    #[allow(dead_code)]
    pub last_refresh: Instant,
    pub tabs: Vec<Box<dyn TabRenderer>>,
    pub inline_cmd: Option<InlineCommand>,
    pub bg_cmd: Option<BackgroundCommand>,
    pub notification: Option<(String, Instant)>,
    pub pending_refresh: bool,
    pub palette: Option<CommandPalette>,
}
```

In `App::new()`, add `palette: None` to the struct initialiser.

Add two methods at the end of the `impl App` block (before the `poll_commands` impl):

```rust
/// Open the command palette for the currently active tab.
/// Does nothing if the tab has no palette actions.
pub fn open_palette(&mut self) {
    let actions = self.active_tab_renderer().palette_actions();
    if !actions.is_empty() {
        self.palette = Some(CommandPalette::new(actions));
    }
}

/// Close the command palette without firing any action.
pub fn close_palette(&mut self) {
    self.palette = None;
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p dashbox 2>&1 | tail -5
```

Expected: `Finished`.

- [ ] **Step 3: Commit**

```bash
git add crates/dashbox/src/app.rs
git commit -m "feat(dashbox): wire CommandPalette into App"
```

---

## Task 4: Update `ui.rs` to render the palette overlay and update status bar

**Files:**
- Modify: `crates/dashbox/src/ui.rs`

- [ ] **Step 1: Render the palette overlay**

In `crates/dashbox/src/ui.rs`, after the tab content is rendered (end of the `if let Some(ref inline)` / `else` block), add:

```rust
// Palette overlay — drawn on top of tab content
if let Some(ref mut palette) = app.palette {
    palette.render(frame, chunks[1]);
}
```

- [ ] **Step 2: Update status bar hints**

Replace the `left_text` construction so it shows palette hint when palette is open, and `Space:actions` when it isn't:

```rust
let tab_keys = app.tabs[app.active_tab.index()].status_keys();
let left_text = if app.palette.is_some() {
    " j/k:select  Enter:run  Esc:close".to_string()
} else if app.inline_cmd.is_some() {
    format!(" Esc:close pane  {tab_keys}")
} else {
    format!(" q:quit  1-9:tab  r:refresh  Space:actions  {tab_keys}")
};
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo check -p dashbox 2>&1 | tail -5
```

Expected: `Finished`.

- [ ] **Step 4: Commit**

```bash
git add crates/dashbox/src/ui.rs
git commit -m "feat(dashbox): render palette overlay and update status bar hints"
```

---

## Task 5: Route `Space` through the palette in `main.rs`

**Files:**
- Modify: `crates/dashbox/src/main.rs`

- [ ] **Step 1: Add palette key routing**

In the `run` function in `main.rs`, the key handling section currently looks like:

```rust
if key.code == KeyCode::Esc {
    ...
}

// Global keys (not captured by tabs when inline is open)
if app.inline_cmd.is_none() {
    match key.code {
        KeyCode::Char('q') => { ... }
        ...
    }
}

// Forward to active tab
let action = app.active_tab_renderer().handle_key(key);
```

Replace the entire key-handling block (from `if key.code == KeyCode::Esc` through the tab forward) with:

```rust
// Esc: close palette first, then inline pane, then quit
if key.code == KeyCode::Esc {
    if app.palette.is_some() {
        app.close_palette();
        continue;
    } else if app.inline_cmd.is_some() {
        app.inline_cmd = None;
        continue;
    } else {
        app.should_quit = true;
        continue;
    }
}

// If palette is open, route keys into it
if app.palette.is_some() {
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(ref mut p) = app.palette { p.select_next(); }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(ref mut p) = app.palette { p.select_previous(); }
        }
        KeyCode::Char(' ') => {
            app.close_palette();
        }
        KeyCode::Enter => {
            if let Some(palette) = app.palette.take() {
                if let Some(action) = palette.handle_key(None, true) {
                    dispatch_action(&mut app, action);
                }
            }
        }
        KeyCode::Char(c) => {
            if let Some(palette) = app.palette.take() {
                if let Some(action) = palette.handle_key(Some(c), false) {
                    dispatch_action(&mut app, action);
                }
                // If no match, palette was consumed (closed) — that's fine
            }
        }
        _ => {}
    }
    continue;
}

// Global keys (not captured by tabs when inline is open)
if app.inline_cmd.is_none() {
    match key.code {
        KeyCode::Char('q') => {
            app.should_quit = true;
            continue;
        }
        KeyCode::Char('1') => { app.select_tab(Tab::Agents); continue; }
        KeyCode::Char('2') => { app.select_tab(Tab::Bench); continue; }
        KeyCode::Char('3') => { app.select_tab(Tab::History); continue; }
        KeyCode::Char('4') => { app.select_tab(Tab::Git); continue; }
        KeyCode::Char('5') => { app.select_tab(Tab::Todos); continue; }
        KeyCode::Char('6') => { app.select_tab(Tab::Items); continue; }
        KeyCode::Char('7') => { app.select_tab(Tab::Ci); continue; }
        KeyCode::Char('8') => { app.select_tab(Tab::Diagrams); continue; }
        KeyCode::Char('9') => { app.select_tab(Tab::Metrics); continue; }
        KeyCode::Left => { app.prev_tab(); continue; }
        KeyCode::Right => { app.next_tab(); continue; }
        KeyCode::Char('r') => { app.active_tab_renderer().refresh(); continue; }
        KeyCode::Char(' ') => {
            app.open_palette();
            continue;
        }
        _ => {}
    }
}

// Forward to active tab
let action = app.active_tab_renderer().handle_key(key);
dispatch_action(&mut app, action);
```

- [ ] **Step 2: Extract `dispatch_action` helper**

The action dispatch logic (currently inline after `handle_key`) needs to be a function so it can be
called from both the palette path and the tab-forward path. Add this function **outside** `run`,
before or after it:

```rust
fn dispatch_action(app: &mut App, action: TabAction) {
    match action {
        TabAction::RunInline { cmd, args } => {
            if app.inline_cmd.is_none() {
                app.inline_cmd = InlineCommand::spawn(&cmd, &args).ok();
            }
        }
        TabAction::RunBackground { cmd, args, label } => {
            if app.bg_cmd.is_none() {
                app.notification = Some((format!("{label}..."), std::time::Instant::now()));
                app.bg_cmd = BackgroundCommand::spawn(&cmd, &args, label).ok();
            }
        }
        TabAction::Quit => app.should_quit = true,
        TabAction::OpenUrl(url) => {
            let opener = if std::env::consts::OS == "macos" { "open" } else { "xdg-open" };
            let _ = std::process::Command::new(opener).arg(&url).spawn();
        }
        TabAction::None => {}
    }
}
```

Remove the old inline `match action { ... }` block that follows the tab-forward call.

- [ ] **Step 3: Verify it compiles**

```bash
cargo check -p dashbox 2>&1 | tail -5
```

Expected: `Finished`.

- [ ] **Step 4: Commit**

```bash
git add crates/dashbox/src/main.rs
git commit -m "feat(dashbox): route Space to palette, extract dispatch_action helper"
```

---

## Task 6: Migrate `BenchTab` — remove direct keys, add `palette_actions`

**Files:**
- Modify: `crates/dashbox/src/tabs/bench.rs`

- [ ] **Step 1: Update `handle_key` and add `palette_actions`**

In `crates/dashbox/src/tabs/bench.rs`:

Replace the `handle_key` implementation (remove `t`, `b`, `B` arms):

```rust
fn handle_key(&mut self, key: KeyEvent) -> TabAction {
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            self.table_state.select_next();
            TabAction::None
        }
        KeyCode::Up | KeyCode::Char('k') => {
            self.table_state.select_previous();
            TabAction::None
        }
        _ => TabAction::None,
    }
}
```

Add `palette_actions` implementation:

```rust
fn palette_actions(&mut self) -> Vec<crate::tabs::PaletteEntry> {
    vec![
        crate::tabs::PaletteEntry {
            key: 't',
            label: "run tests",
            action: TabAction::RunInline {
                cmd: "cargo".to_string(),
                args: vec!["xtask".to_string(), "test-unit".to_string()],
            },
        },
        crate::tabs::PaletteEntry {
            key: 'b',
            label: "bench local",
            action: TabAction::RunInline {
                cmd: "cargo".to_string(),
                args: vec!["xtask".to_string(), "bench".to_string()],
            },
        },
        crate::tabs::PaletteEntry {
            key: 'B',
            label: "bench VPS",
            action: TabAction::RunBackground {
                cmd: "cargo".to_string(),
                args: vec!["xtask".to_string(), "bench-vps".to_string()],
                label: "VPS bench".to_string(),
            },
        },
    ]
}
```

Update `status_keys`:

```rust
fn status_keys(&self) -> &'static str {
    "j/k:scroll  r:refresh"
}
```

- [ ] **Step 2: Verify**

```bash
cargo check -p dashbox 2>&1 | tail -5
```

- [ ] **Step 3: Commit**

```bash
git add crates/dashbox/src/tabs/bench.rs
git commit -m "feat(dashbox): move bench actions to palette (t/b/B)"
```

---

## Task 7: Migrate `CiTab`, `ItemsTab`, `TodosTab`

**Files:**
- Modify: `crates/dashbox/src/tabs/ci.rs`
- Modify: `crates/dashbox/src/tabs/items.rs`
- Modify: `crates/dashbox/src/tabs/todos.rs`

- [ ] **Step 1: Update `CiTab`**

In `ci.rs`, remove the `KeyCode::Char('o')` arm from `handle_key` (keep `j/k`). Add:

```rust
fn palette_actions(&mut self) -> Vec<crate::tabs::PaletteEntry> {
    let idx = match self.table_state.selected() {
        Some(i) => i,
        None => return vec![],
    };
    let url = self
        .cached_data
        .as_ref()
        .and_then(|d| d.runs.get(idx))
        .map(|r| r.url.clone())
        .unwrap_or_default();
    if url.is_empty() {
        return vec![];
    }
    vec![crate::tabs::PaletteEntry {
        key: 'o',
        label: "open in browser",
        action: TabAction::OpenUrl(url),
    }]
}
```

Update `status_keys`:

```rust
fn status_keys(&self) -> &'static str {
    "j/k:scroll  r:refresh"
}
```

- [ ] **Step 2: Update `ItemsTab`**

In `items.rs`, remove the `KeyCode::Char('c') | KeyCode::Char('o') | KeyCode::Char('b')` arm from
`handle_key` (keep `j/k`). Add:

```rust
fn palette_actions(&mut self) -> Vec<crate::tabs::PaletteEntry> {
    let idx = match self.table_state.selected() {
        Some(i) => i,
        None => return vec![],
    };
    let uuid = self
        .cached_data
        .as_ref()
        .and_then(|d| d.items.get(idx))
        .map(|i| i.doob_uuid.clone())
        .unwrap_or_default();
    if uuid.is_empty() {
        return vec![];
    }
    vec![
        crate::tabs::PaletteEntry {
            key: 'c',
            label: "mark done",
            action: TabAction::RunBackground {
                cmd: "doob".into(),
                args: vec!["handoff".into(), "update-status".into(), uuid.clone(), "done".into()],
                label: "mark done".into(),
            },
        },
        crate::tabs::PaletteEntry {
            key: 'o',
            label: "mark open",
            action: TabAction::RunBackground {
                cmd: "doob".into(),
                args: vec!["handoff".into(), "update-status".into(), uuid.clone(), "open".into()],
                label: "mark open".into(),
            },
        },
        crate::tabs::PaletteEntry {
            key: 'b',
            label: "mark blocked",
            action: TabAction::RunBackground {
                cmd: "doob".into(),
                args: vec!["handoff".into(), "update-status".into(), uuid, "blocked".into()],
                label: "mark blocked".into(),
            },
        },
    ]
}
```

Update `status_keys`:

```rust
fn status_keys(&self) -> &'static str {
    "j/k:scroll  r:refresh"
}
```

- [ ] **Step 3: Update `TodosTab`**

In `todos.rs`, remove the `KeyCode::Char('c')` and `KeyCode::Char('o')` arms from `handle_key`
(keep `j/k`). The `TodosTab` needs a `cached_data` field to resolve UUIDs at palette-open time.

Add the field to the struct:

```rust
pub struct TodosTab {
    source: CachedSource<TodosSource>,
    table_state: TableState,
    cached_data: Option<crate::data::todos::TodosData>,
}
```

In `new()`, add `cached_data: None`.

In `render`, after loading data, add:

```rust
self.cached_data = Some(data.clone());
```

(Add this line immediately after `let data = match self.source.get() { Some(Ok(d)) => d, ... }`)

Add `palette_actions`:

```rust
fn palette_actions(&mut self) -> Vec<crate::tabs::PaletteEntry> {
    let idx = match self.table_state.selected() {
        Some(i) => i,
        None => return vec![],
    };
    // Only show palette for visible pending todos
    let pending: Vec<_> = self
        .cached_data
        .as_ref()
        .map(|d| d.todos.iter().filter(|t| t.status == "pending").collect())
        .unwrap_or_default();
    let todo = match pending.get(idx) {
        Some(t) => t,
        None => return vec![],
    };
    let uuid = todo.doob_uuid.clone();
    if uuid.is_empty() {
        return vec![];
    }
    vec![
        crate::tabs::PaletteEntry {
            key: 'c',
            label: "complete",
            action: TabAction::RunBackground {
                cmd: "doob".into(),
                args: vec!["todo".into(), "complete".into(), uuid],
                label: "complete todo".into(),
            },
        },
    ]
}
```

Update `status_keys`:

```rust
fn status_keys(&self) -> &'static str {
    "j/k:scroll  r:refresh"
}
```

- [ ] **Step 4: Verify all three compile**

```bash
cargo check -p dashbox 2>&1 | tail -5
```

- [ ] **Step 5: Commit**

```bash
git add crates/dashbox/src/tabs/ci.rs crates/dashbox/src/tabs/items.rs crates/dashbox/src/tabs/todos.rs
git commit -m "feat(dashbox): move ci/items/todos actions to palette"
```

---

## Task 8: Migrate `DiagramsTab`, clean up `MetricsTab`

**Files:**
- Modify: `crates/dashbox/src/tabs/diagrams.rs`
- Modify: `crates/dashbox/src/tabs/metrics.rs`

- [ ] **Step 1: Update `DiagramsTab`**

In `diagrams.rs`, remove `KeyCode::Char('d')` and `KeyCode::Char('D')` arms from `handle_key`
(keep `h/l/j/k` navigation and `Tab`/`BackTab`). Add:

```rust
fn palette_actions(&mut self) -> Vec<crate::tabs::PaletteEntry> {
    vec![
        crate::tabs::PaletteEntry {
            key: 'd',
            label: "next diagram",
            action: TabAction::None, // handled specially — see note below
        },
        crate::tabs::PaletteEntry {
            key: 'D',
            label: "prev diagram",
            action: TabAction::None,
        },
    ]
}
```

> **Note:** `next_diagram`/`prev_diagram` mutate `self` so they can't be expressed as a pure
> `TabAction`. For this tab, keep `d/D` in `handle_key` as well (they don't conflict with global
> keys). The palette entries provide discoverability; pressing `d` directly still works. This is
> the one exception to the "remove direct keys" rule.

Actually — revert: keep `d/D` in `handle_key` for DiagramsTab, and have `palette_actions` return
them as `TabAction::None` entries (display-only discovery). The main loop will see `TabAction::None`
from the palette and the diagram won't advance. Instead, close the palette on selection and let the
user press `d` directly.

Simpler approach: just add diagram switch actions using the inline command workaround. But that
adds unnecessary complexity. **Final decision:** DiagramsTab keeps `d/D` in `handle_key`; its
`palette_actions` lists them as informational entries that fire `TabAction::None` and closes the
palette, leaving the user to press `d` directly for actual navigation.

Update `status_keys`:

```rust
fn status_keys(&self) -> &'static str {
    "h/j/k/l:navigate  Tab:next-node  d/D:diagram  r:refresh"
}
```

- [ ] **Step 2: Update `MetricsTab`**

In `metrics.rs`, remove the `KeyCode::Char('r')` arm from `handle_key` (global `r` already handles
refresh). The `handle_key` becomes:

```rust
fn handle_key(&mut self, key: KeyEvent) -> TabAction {
    TabAction::None
}
```

Update `status_keys`:

```rust
fn status_keys(&self) -> &'static str {
    "r:refresh"
}
```

- [ ] **Step 3: Verify**

```bash
cargo check -p dashbox 2>&1 | tail -5
```

- [ ] **Step 4: Commit**

```bash
git add crates/dashbox/src/tabs/diagrams.rs crates/dashbox/src/tabs/metrics.rs
git commit -m "feat(dashbox): update diagrams/metrics tabs for palette refactor"
```

---

## Task 9: Fix existing tests broken by action key removal

**Files:**
- Modify: `crates/dashbox/src/tabs/items.rs` (test section)
- Modify: `crates/dashbox/src/tabs/ci.rs` (test section)
- Modify: `crates/dashbox/src/tabs/todos.rs` (test section, if any)

- [ ] **Step 1: Run the test suite to see what breaks**

```bash
cargo test -p dashbox 2>&1 | tail -30
```

Any test that does `tab.handle_key(make_key(KeyCode::Char('c')))` and expects `RunBackground` will
now get `None`. These tests must be rewritten to call `tab.palette_actions()` instead.

- [ ] **Step 2: Rewrite `items.rs` tests**

Replace the two action tests in `crates/dashbox/src/tabs/items.rs`:

```rust
#[test]
fn test_items_tab_palette_c_key_emits_run_background() {
    let mut tab = tab_with_data(vec![make_item("uuid-abc", "open")]);
    tab.table_state.select(Some(0));
    // Manually set cached_data so palette_actions can resolve UUID
    let actions = tab.palette_actions();
    let entry = actions.into_iter().find(|e| e.key == 'c');
    assert!(entry.is_some(), "expected 'c' palette entry");
    match entry.unwrap().action {
        TabAction::RunBackground { cmd, args, label } => {
            assert_eq!(cmd, "doob");
            assert!(args.contains(&"uuid-abc".to_string()), "args: {args:?}");
            assert!(args.contains(&"done".to_string()), "args: {args:?}");
            assert_eq!(label, "mark done");
        }
        other => panic!("expected RunBackground, got {:?}", std::mem::discriminant(&other)),
    }
}

#[test]
fn test_items_tab_no_selection_palette_is_empty() {
    let mut tab = ItemsTab::new();
    tab.table_state.select(None);
    let actions = tab.palette_actions();
    assert!(actions.is_empty());
}
```

- [ ] **Step 3: Rewrite `ci.rs` tests**

Replace the `test_ci_tab_o_key_emits_open_url` test:

```rust
#[test]
fn test_ci_tab_palette_o_key_emits_open_url() {
    let url = "https://github.com/89jobrien/minibox/actions/runs/1";
    let mut tab = tab_with_run(run_with_url(url));
    tab.table_state.select(Some(0));
    let actions = tab.palette_actions();
    let entry = actions.into_iter().find(|e| e.key == 'o');
    assert!(entry.is_some(), "expected 'o' palette entry");
    match entry.unwrap().action {
        TabAction::OpenUrl(u) => assert_eq!(u, url),
        other => panic!("expected OpenUrl, got {:?}", std::mem::discriminant(&other)),
    }
}
```

- [ ] **Step 4: Run all tests**

```bash
cargo test -p dashbox 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/dashbox/src/tabs/items.rs crates/dashbox/src/tabs/ci.rs
git commit -m "test(dashbox): update action tests to use palette_actions()"
```

---

## Task 10: Final gate — pre-commit and smoke check

- [ ] **Step 1: Run pre-commit gate**

```bash
cargo xtask pre-commit 2>&1 | tail -20
```

Expected: `pre-commit checks passed`.

- [ ] **Step 2: Run all dashbox tests**

```bash
cargo test -p dashbox 2>&1 | tail -10
```

Expected: all pass.

- [ ] **Step 3: Verify no direct action keys remain (audit)**

```bash
grep -n "Char('t')\|Char('b')\|Char('B')\|Char('c')\|Char('o')\|Char('D')" \
  crates/dashbox/src/tabs/bench.rs \
  crates/dashbox/src/tabs/ci.rs \
  crates/dashbox/src/tabs/items.rs \
  crates/dashbox/src/tabs/todos.rs
```

Expected: no output (all removed).

- [ ] **Step 4: Verify `Space` is wired in main**

```bash
grep -n "Char(' ')" crates/dashbox/src/main.rs
```

Expected: two matches — one in palette routing block, one in global keys block.

- [ ] **Step 5: Final commit if anything was touched**

```bash
git status
# only commit if there are changes
git add -A && git commit -m "chore(dashbox): post-refactor cleanup"
```

---

## Self-Review

**Spec coverage:**
- ✅ Space opens palette
- ✅ `j/k`/Enter/Esc navigate/fire/dismiss palette
- ✅ Centered floating overlay with border
- ✅ Navigation keys stay direct
- ✅ All action keys removed from `handle_key`
- ✅ `status_keys` updated to remove stale hints
- ✅ Existing tests rewritten for palette path
- ✅ DiagramsTab exception documented (d/D stay direct, palette is informational)

**Placeholder scan:** No TBDs. All code is complete.

**Type consistency:**
- `PaletteEntry` defined in Task 1, used identically in Tasks 6-8
- `CommandPalette::handle_key` signature: `(self, ch: Option<char>, enter: bool) -> Option<TabAction>` — used consistently in Task 5
- `App::open_palette` / `App::close_palette` defined in Task 3, called in Task 5
- `dispatch_action(app: &mut App, action: TabAction)` defined and called in Task 5
