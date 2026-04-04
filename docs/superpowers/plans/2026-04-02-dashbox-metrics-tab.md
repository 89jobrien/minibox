# Dashbox Metrics Tab + Grafana Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a live Metrics tab to the dashbox TUI that polls the miniboxd Prometheus endpoint, and ship a Grafana dashboard JSON for external visualization.

**Architecture:** A new `MetricsSource` implementing the existing `DataSource` trait fetches and parses Prometheus text exposition format via a blocking HTTP GET (ureq). The parsed data flows into a `MetricsTab` rendering three sections: status bar, counters table, and duration table. A separate `grafana/minibox-dashboard.json` provides an importable Grafana dashboard with four panels.

**Tech Stack:** Rust, ratatui 0.29, ureq (blocking HTTP), Prometheus text format (hand-parsed), Grafana dashboard JSON v1.

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `crates/dashbox/Cargo.toml` | Modify | Add `ureq = "2"` dependency |
| `crates/dashbox/src/data/metrics.rs` | Create | `MetricsSource`, `MetricsData`, all parsing logic |
| `crates/dashbox/src/data/mod.rs` | Modify | Add `pub mod metrics` |
| `crates/dashbox/src/tabs/metrics.rs` | Create | `MetricsTab` implementing `TabRenderer` |
| `crates/dashbox/src/tabs/mod.rs` | Modify | Add `pub mod metrics` |
| `crates/dashbox/src/app.rs` | Modify | Add `Tab::Metrics`, wire `MetricsTab::new()` |
| `grafana/minibox-dashboard.json` | Create | Importable Grafana dashboard |

---

## Task 1: Add ureq dependency

**Files:**
- Modify: `crates/dashbox/Cargo.toml`

- [ ] **Step 1: Add ureq to Cargo.toml**

In `crates/dashbox/Cargo.toml`, add after the `dirs` line:

```toml
ureq = { version = "2", features = [] }
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p dashbox
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add crates/dashbox/Cargo.toml
git commit -m "chore(dashbox): add ureq for metrics HTTP polling"
```

---

## Task 2: Implement MetricsSource and data types

**Files:**
- Create: `crates/dashbox/src/data/metrics.rs`
- Modify: `crates/dashbox/src/data/mod.rs`

- [ ] **Step 1: Write the unit tests first**

Create `crates/dashbox/src/data/metrics.rs` with tests only:

```rust
// dashbox/src/data/metrics.rs
use anyhow::Result;
use std::collections::HashMap;

use super::DataSource;

/// Parsed result from a single poll of the /metrics endpoint.
#[derive(Debug, Clone)]
pub enum MetricsData {
    /// Daemon is unreachable.
    Offline,
    /// Successfully parsed metrics.
    Live(LiveMetrics),
}

/// Fully parsed live metrics snapshot.
#[derive(Debug, Clone, Default)]
pub struct LiveMetrics {
    /// Value of minibox_active_containers gauge.
    pub active_containers: f64,
    /// Ops counters keyed by (op, status) → count.
    pub ops_counters: HashMap<(String, String), f64>,
    /// Duration p50/p95 keyed by op name.
    pub durations: HashMap<String, DurationSummary>,
}

/// p50 and p95 latency in seconds for a given op.
#[derive(Debug, Clone)]
pub struct DurationSummary {
    pub p50: f64,
    pub p95: f64,
}

pub struct MetricsSource {
    pub addr: String,
}

impl MetricsSource {
    pub fn new() -> Self {
        let addr = std::env::var("MINIBOX_METRICS_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9090".to_string());
        Self { addr }
    }
}

impl DataSource for MetricsSource {
    type Data = MetricsData;

    fn load(&self) -> Result<MetricsData> {
        let url = format!("http://{}/metrics", self.addr);
        let body = match ureq::get(&url).call() {
            Ok(resp) => resp.into_string()?,
            Err(ureq::Error::Transport(t))
                if t.kind() == ureq::ErrorKind::ConnectionFailed =>
            {
                return Ok(MetricsData::Offline);
            }
            Err(e) => return Err(e.into()),
        };
        Ok(MetricsData::Live(parse_metrics(&body)))
    }
}

/// Parse Prometheus text exposition format into LiveMetrics.
pub fn parse_metrics(input: &str) -> LiveMetrics {
    let mut result = LiveMetrics::default();
    // bucket_data: op → vec of (le, cumulative_count)
    let mut bucket_data: HashMap<String, Vec<(f64, f64)>> = HashMap::new();

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (name_and_labels, value_str) = match line.rsplit_once(' ') {
            Some(parts) => parts,
            None => continue,
        };
        let value: f64 = match value_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let (name, labels) = parse_name_and_labels(name_and_labels);

        if name == "minibox_active_containers" {
            result.active_containers = value;
        } else if name == "minibox_container_ops_total" {
            let op = labels.get("op").cloned().unwrap_or_default();
            let status = labels.get("status").cloned().unwrap_or_default();
            *result.ops_counters.entry((op, status)).or_insert(0.0) += value;
        } else if name == "minibox_container_op_duration_seconds_bucket" {
            let op = labels.get("op").cloned().unwrap_or_default();
            let le_str = labels.get("le").map(|s| s.as_str()).unwrap_or("+Inf");
            let le: f64 = if le_str == "+Inf" {
                f64::INFINITY
            } else {
                le_str.parse().unwrap_or(f64::INFINITY)
            };
            bucket_data.entry(op).or_default().push((le, value));
        }
    }

    // Derive p50/p95 from bucket data
    for (op, mut buckets) in bucket_data {
        buckets.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let total = buckets.last().map(|(_, c)| *c).unwrap_or(0.0);
        if total == 0.0 {
            continue;
        }
        let p50 = interpolate_quantile(&buckets, 0.50, total);
        let p95 = interpolate_quantile(&buckets, 0.95, total);
        result.durations.insert(op, DurationSummary { p50, p95 });
    }

    result
}

/// Linear interpolation of a quantile from sorted (le, cumulative_count) pairs.
fn interpolate_quantile(buckets: &[(f64, f64)], q: f64, total: f64) -> f64 {
    let target = q * total;
    let mut prev_le = 0.0_f64;
    let mut prev_count = 0.0_f64;
    for &(le, count) in buckets {
        if count >= target {
            if count == prev_count {
                return prev_le;
            }
            // Linear interpolation within bucket
            let fraction = (target - prev_count) / (count - prev_count);
            return prev_le + fraction * (le - prev_le);
        }
        prev_le = le;
        prev_count = count;
    }
    prev_le
}

/// Split "metric_name{k=\"v\",k2=\"v2\"}" into (name, labels_map).
/// Also handles plain "metric_name" with no labels.
fn parse_name_and_labels(s: &str) -> (&str, HashMap<String, String>) {
    let mut labels = HashMap::new();
    match s.find('{') {
        None => (s, labels),
        Some(brace) => {
            let name = &s[..brace];
            let rest = &s[brace + 1..];
            let rest = rest.trim_end_matches('}');
            for pair in rest.split(',') {
                if let Some((k, v)) = pair.split_once('=') {
                    let v = v.trim_matches('"');
                    labels.insert(k.to_string(), v.to_string());
                }
            }
            (name, labels)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# HELP minibox_active_containers Number of active containers
# TYPE minibox_active_containers gauge
minibox_active_containers 3
# HELP minibox_container_ops_total Total container operations
# TYPE minibox_container_ops_total counter
minibox_container_ops_total{op="start",adapter="daemon",status="ok"} 42
minibox_container_ops_total{op="start",adapter="daemon",status="error"} 2
minibox_container_ops_total{op="stop",adapter="daemon",status="ok"} 10
# HELP minibox_container_op_duration_seconds Duration histogram
# TYPE minibox_container_op_duration_seconds histogram
minibox_container_op_duration_seconds_bucket{op="start",adapter="daemon",le="0.001"} 0
minibox_container_op_duration_seconds_bucket{op="start",adapter="daemon",le="0.002"} 5
minibox_container_op_duration_seconds_bucket{op="start",adapter="daemon",le="0.004"} 21
minibox_container_op_duration_seconds_bucket{op="start",adapter="daemon",le="0.008"} 40
minibox_container_op_duration_seconds_bucket{op="start",adapter="daemon",le="+Inf"} 44
"#;

    #[test]
    fn test_parse_active_containers() {
        let m = parse_metrics(SAMPLE);
        assert_eq!(m.active_containers, 3.0);
    }

    #[test]
    fn test_parse_ops_counters() {
        let m = parse_metrics(SAMPLE);
        assert_eq!(
            m.ops_counters.get(&("start".to_string(), "ok".to_string())),
            Some(&42.0)
        );
        assert_eq!(
            m.ops_counters.get(&("start".to_string(), "error".to_string())),
            Some(&2.0)
        );
        assert_eq!(
            m.ops_counters.get(&("stop".to_string(), "ok".to_string())),
            Some(&10.0)
        );
    }

    #[test]
    fn test_parse_durations_p50_within_range() {
        let m = parse_metrics(SAMPLE);
        let d = m.durations.get("start").expect("start duration missing");
        // p50 of 44 total = 22nd obs; bucket [0.004,0.008] contains obs 22-40
        assert!(d.p50 >= 0.004 && d.p50 <= 0.008, "p50={}", d.p50);
    }

    #[test]
    fn test_parse_durations_p95_within_range() {
        let m = parse_metrics(SAMPLE);
        let d = m.durations.get("start").expect("start duration missing");
        // p95 of 44 total = 41.8th obs; bucket [0.008,+Inf] contains obs 41-44
        assert!(d.p95 >= 0.008, "p95={}", d.p95);
    }

    #[test]
    fn test_empty_input() {
        let m = parse_metrics("");
        assert_eq!(m.active_containers, 0.0);
        assert!(m.ops_counters.is_empty());
        assert!(m.durations.is_empty());
    }

    #[test]
    fn test_parse_name_no_labels() {
        let (name, labels) = parse_name_and_labels("minibox_active_containers");
        assert_eq!(name, "minibox_active_containers");
        assert!(labels.is_empty());
    }

    #[test]
    fn test_parse_name_with_labels() {
        let (name, labels) =
            parse_name_and_labels(r#"minibox_container_ops_total{op="start",status="ok"}"#);
        assert_eq!(name, "minibox_container_ops_total");
        assert_eq!(labels.get("op").map(|s| s.as_str()), Some("start"));
        assert_eq!(labels.get("status").map(|s| s.as_str()), Some("ok"));
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/dashbox/src/data/mod.rs`, add after the existing `pub mod todos;` line:

```rust
pub mod metrics;
```

- [ ] **Step 3: Run the tests to verify they fail (implementation missing)**

```bash
cargo test -p dashbox data::metrics 2>&1 | head -30
```

Expected: compile error — `ureq` calls won't compile until ureq is added (already done in Task 1). Tests for parse functions should compile and some may pass trivially.

- [ ] **Step 4: Run all metric parsing tests**

```bash
cargo test -p dashbox metrics
```

Expected: all 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/dashbox/src/data/metrics.rs crates/dashbox/src/data/mod.rs
git commit -m "feat(dashbox): add MetricsSource with Prometheus text parser"
```

---

## Task 3: Implement MetricsTab

**Files:**
- Create: `crates/dashbox/src/tabs/metrics.rs`
- Modify: `crates/dashbox/src/tabs/mod.rs`

- [ ] **Step 1: Create the tab**

Create `crates/dashbox/src/tabs/metrics.rs`:

```rust
// dashbox/src/tabs/metrics.rs
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::metrics::{LiveMetrics, MetricsData, MetricsSource};

pub struct MetricsTab {
    source: CachedSource<MetricsSource>,
}

impl MetricsTab {
    pub fn new() -> Self {
        Self {
            source: CachedSource::new(MetricsSource::new(), 5),
        }
    }
}

impl TabRenderer for MetricsTab {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.source.ensure_fresh();

        let addr = self.source.source_addr();

        match self.source.get() {
            None => {
                // First load in progress — show loading state
                let msg = Paragraph::new("Loading metrics…")
                    .block(Block::default().borders(Borders::ALL).title("Metrics"));
                frame.render_widget(msg, area);
            }
            Some(Err(e)) => {
                let msg = Paragraph::new(format!("Error: {e}"))
                    .style(Style::default().fg(Color::Red))
                    .block(Block::default().borders(Borders::ALL).title("Metrics"));
                frame.render_widget(msg, area);
            }
            Some(Ok(MetricsData::Offline)) => {
                render_offline(frame, area, &addr);
            }
            Some(Ok(MetricsData::Live(live))) => {
                render_live(frame, area, &addr, live);
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> TabAction {
        match key.code {
            KeyCode::Char('r') => {
                self.source.refresh();
                TabAction::None
            }
            _ => TabAction::None,
        }
    }

    fn refresh(&mut self) {
        self.source.refresh();
    }

    fn status_keys(&self) -> &'static str {
        "r:refresh"
    }
}

fn render_offline(frame: &mut Frame, area: Rect, addr: &str) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    let status_line = Line::from(vec![
        Span::styled("OFFLINE", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(addr, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(status_line), chunks[0]);

    let msg = Paragraph::new("miniboxd is not running or metrics endpoint is unreachable.\nStart the daemon: sudo miniboxd")
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL).title("Metrics"));
    frame.render_widget(msg, chunks[1]);
}

fn render_live(frame: &mut Frame, area: Rect, addr: &str, live: &LiveMetrics) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // status bar
        Constraint::Min(0),    // content
    ])
    .split(area);

    // Status bar
    let status_line = Line::from(vec![
        Span::styled("LIVE", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(addr, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(status_line), chunks[0]);

    // Content area: top row (gauge + counters) | bottom row (durations)
    let content = Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
        .split(chunks[1]);

    // Top row: gauge (30%) | counters (70%)
    let top = Layout::horizontal([Constraint::Ratio(3, 10), Constraint::Ratio(7, 10)])
        .split(content[0]);

    render_gauge(frame, top[0], live.active_containers);
    render_counters(frame, top[1], live);
    render_durations(frame, content[1], live);
}

fn render_gauge(frame: &mut Frame, area: Rect, value: f64) {
    let text = format!("{}", value as u64);
    let paragraph = Paragraph::new(Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Active Containers"),
    );
    frame.render_widget(paragraph, area);
}

fn render_counters(frame: &mut Frame, area: Rect, live: &LiveMetrics) {
    // Collect unique ops
    let mut ops: Vec<String> = live
        .ops_counters
        .keys()
        .map(|(op, _)| op.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    ops.sort();

    let header = Row::new(["Op", "OK", "Error", "Total"])
        .style(Style::default().fg(Color::DarkGray));
    let widths = [
        Constraint::Length(12),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
    ];

    let rows: Vec<Row> = ops
        .iter()
        .map(|op| {
            let ok = live
                .ops_counters
                .get(&(op.clone(), "ok".to_string()))
                .copied()
                .unwrap_or(0.0);
            let err = live
                .ops_counters
                .get(&(op.clone(), "error".to_string()))
                .copied()
                .unwrap_or(0.0);
            let total = ok + err;
            let err_color = if err > 0.0 { Color::Red } else { Color::DarkGray };
            Row::new([
                Cell::from(op.as_str()).style(Style::default().fg(Color::White)),
                Cell::from(format!("{ok:.0}")),
                Cell::from(format!("{err:.0}")).style(Style::default().fg(err_color)),
                Cell::from(format!("{total:.0}")),
            ])
        })
        .collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Ops Counters"));
    frame.render_widget(table, area);
}

fn render_durations(frame: &mut Frame, area: Rect, live: &LiveMetrics) {
    let mut ops: Vec<String> = live.durations.keys().cloned().collect();
    ops.sort();

    let header =
        Row::new(["Op", "p50", "p95"]).style(Style::default().fg(Color::DarkGray));
    let widths = [
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(12),
    ];

    let rows: Vec<Row> = ops
        .iter()
        .map(|op| {
            let d = &live.durations[op];
            Row::new([
                Cell::from(op.as_str()).style(Style::default().fg(Color::White)),
                Cell::from(fmt_duration(d.p50)),
                Cell::from(fmt_duration(d.p95)),
            ])
        })
        .collect();

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Op Durations"));
    frame.render_widget(table, area);
}

fn fmt_duration(secs: f64) -> String {
    if secs < 0.001 {
        format!("{:.0}µs", secs * 1_000_000.0)
    } else if secs < 1.0 {
        format!("{:.1}ms", secs * 1000.0)
    } else {
        format!("{:.2}s", secs)
    }
}
```

- [ ] **Step 2: Add `source_addr()` helper to `CachedSource`**

`MetricsTab` calls `self.source.source_addr()` — add this to `CachedSource` in `crates/dashbox/src/data/mod.rs`. Also add the `MetricsSource` address field access. The cleanest approach: add a `source_addr()` method specific to `MetricsSource`, not on the generic `CachedSource`. Update `metrics.rs` to expose the addr and update the tab to call it directly:

In `crates/dashbox/src/data/metrics.rs`, add a public field accessor — `MetricsSource` already has `pub addr: String`, so in the tab, access it via `self.source` is private. Instead, store addr separately in the tab.

Update `MetricsTab` in `tabs/metrics.rs` to store the addr at construction:

```rust
pub struct MetricsTab {
    source: CachedSource<MetricsSource>,
    addr: String,
}

impl MetricsTab {
    pub fn new() -> Self {
        let ms = MetricsSource::new();
        let addr = ms.addr.clone();
        Self {
            source: CachedSource::new(ms, 5),
            addr,
        }
    }
}
```

Then replace `self.source.source_addr()` calls in `render` with `&self.addr`.

- [ ] **Step 3: Register the module in tabs/mod.rs**

In `crates/dashbox/src/tabs/mod.rs`, add after `pub mod todos;`:

```rust
pub mod metrics;
```

- [ ] **Step 4: Verify it compiles**

```bash
cargo check -p dashbox
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add crates/dashbox/src/tabs/metrics.rs crates/dashbox/src/tabs/mod.rs
git commit -m "feat(dashbox): add MetricsTab with offline/live rendering"
```

---

## Task 4: Wire Metrics tab into App

**Files:**
- Modify: `crates/dashbox/src/app.rs`

- [ ] **Step 1: Update app.rs**

Replace the contents of `crates/dashbox/src/app.rs` with:

```rust
// dashbox/src/app.rs
use std::time::Instant;

use crate::command::{BackgroundCommand, InlineCommand};
use crate::tabs::TabRenderer;
use crate::tabs::agents::AgentsTab;
use crate::tabs::bench::BenchTab;
use crate::tabs::ci::CiTab;
use crate::tabs::diagrams::DiagramsTab;
use crate::tabs::git::GitTab;
use crate::tabs::history::HistoryTab;
use crate::tabs::metrics::MetricsTab;
use crate::tabs::todos::TodosTab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Agents,
    Bench,
    History,
    Git,
    Todos,
    Ci,
    Diagrams,
    Metrics,
}

impl Tab {
    pub const ALL: [Tab; 8] = [
        Tab::Agents,
        Tab::Bench,
        Tab::History,
        Tab::Git,
        Tab::Todos,
        Tab::Ci,
        Tab::Diagrams,
        Tab::Metrics,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Agents => "1 Agents",
            Tab::Bench => "2 Bench",
            Tab::History => "3 History",
            Tab::Git => "4 Git",
            Tab::Todos => "5 Todos",
            Tab::Ci => "6 CI",
            Tab::Diagrams => "7 Diagrams",
            Tab::Metrics => "8 Metrics",
        }
    }

    pub fn index(&self) -> usize {
        match self {
            Tab::Agents => 0,
            Tab::Bench => 1,
            Tab::History => 2,
            Tab::Git => 3,
            Tab::Todos => 4,
            Tab::Ci => 5,
            Tab::Diagrams => 6,
            Tab::Metrics => 7,
        }
    }

    pub fn from_index(i: usize) -> Option<Tab> {
        Tab::ALL.get(i).copied()
    }
}

pub struct App {
    pub active_tab: Tab,
    pub should_quit: bool,
    #[allow(dead_code)]
    pub last_refresh: Instant,
    pub tabs: Vec<Box<dyn TabRenderer>>,
    pub inline_cmd: Option<InlineCommand>,
    pub bg_cmd: Option<BackgroundCommand>,
    pub notification: Option<(String, Instant)>,
}

impl App {
    pub fn new() -> Self {
        Self {
            active_tab: Tab::Agents,
            should_quit: false,
            last_refresh: Instant::now(),
            tabs: vec![
                Box::new(AgentsTab::new()),
                Box::new(BenchTab::new()),
                Box::new(HistoryTab::new()),
                Box::new(GitTab::new()),
                Box::new(TodosTab::new()),
                Box::new(CiTab::new()),
                Box::new(DiagramsTab::new(
                    crate::diagram::source::load_user_diagrams(),
                )),
                Box::new(MetricsTab::new()),
            ],
            inline_cmd: None,
            bg_cmd: None,
            notification: None,
        }
    }

    pub fn select_tab(&mut self, tab: Tab) {
        self.active_tab = tab;
    }

    pub fn next_tab(&mut self) {
        let next = (self.active_tab.index() + 1) % Tab::ALL.len();
        self.active_tab = Tab::from_index(next).unwrap_or(Tab::Agents);
    }

    pub fn prev_tab(&mut self) {
        let prev = (self.active_tab.index() + Tab::ALL.len() - 1) % Tab::ALL.len();
        self.active_tab = Tab::from_index(prev).unwrap_or(Tab::Agents);
    }

    pub fn active_tab_renderer(&mut self) -> &mut dyn TabRenderer {
        &mut *self.tabs[self.active_tab.index()]
    }

    pub fn poll_commands(&mut self) {
        if let Some(ref mut cmd) = self.inline_cmd {
            cmd.poll();
        }
        if let Some(ref mut cmd) = self.bg_cmd {
            cmd.poll();
            if cmd.finished {
                let msg = if cmd.exit_code == Some(0) {
                    format!("{} complete", cmd.label)
                } else {
                    format!(
                        "{} failed (exit {})",
                        cmd.label,
                        cmd.exit_code.unwrap_or(-1)
                    )
                };
                self.notification = Some((msg, Instant::now()));
                self.bg_cmd = None;
            }
        }
        // Clear notification after 5s
        if let Some((_, when)) = &self.notification {
            if when.elapsed().as_secs() >= 5 {
                self.notification = None;
            }
        }
    }
}
```

- [ ] **Step 2: Check if main.rs handles key '8' for tab navigation**

```bash
grep -n "KeyCode::Char\|select_tab\|Tab::" crates/dashbox/src/main.rs | head -40
```

If '8' is handled via pattern like `'1'..='7'` mapping to tab index, update the range to `'1'..='8'` or add an explicit `'8'` arm. Read `main.rs` to find the exact pattern.

- [ ] **Step 3: Verify full build**

```bash
cargo build -p dashbox
```

Expected: builds cleanly, no warnings.

- [ ] **Step 4: Run tests**

```bash
cargo test -p dashbox
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/dashbox/src/app.rs
git commit -m "feat(dashbox): wire Metrics tab as tab 8"
```

---

## Task 5: Check and fix key binding for tab 8

**Files:**
- Modify: `crates/dashbox/src/main.rs` (if needed)

- [ ] **Step 1: Read main.rs to find tab key handling**

Read `crates/dashbox/src/main.rs` in full. Look for how digit keys select tabs.

- [ ] **Step 2: Update key binding range if needed**

If you find something like:

```rust
KeyCode::Char(c) if c >= '1' && c <= '7' => {
    let idx = (c as usize) - ('1' as usize);
    app.select_tab(Tab::from_index(idx).unwrap_or(Tab::Agents));
}
```

Change `'7'` to `'8'`:

```rust
KeyCode::Char(c) if c >= '1' && c <= '8' => {
    let idx = (c as usize) - ('1' as usize);
    app.select_tab(Tab::from_index(idx).unwrap_or(Tab::Agents));
}
```

- [ ] **Step 3: Build and verify**

```bash
cargo build -p dashbox
```

- [ ] **Step 4: Commit if changed**

```bash
git add crates/dashbox/src/main.rs
git commit -m "fix(dashbox): extend tab key binding to include tab 8 (Metrics)"
```

---

## Task 6: Create Grafana dashboard JSON

**Files:**
- Create: `grafana/minibox-dashboard.json`

- [ ] **Step 1: Create the grafana directory and dashboard file**

```bash
mkdir -p grafana
```

Create `grafana/minibox-dashboard.json`:

```json
{
  "__inputs": [
    {
      "name": "DS_PROMETHEUS",
      "label": "Prometheus",
      "description": "Prometheus datasource scraping miniboxd /metrics",
      "type": "datasource",
      "pluginId": "prometheus",
      "pluginName": "Prometheus"
    }
  ],
  "__requires": [
    {
      "type": "grafana",
      "id": "grafana",
      "name": "Grafana",
      "version": "10.0.0"
    },
    {
      "type": "datasource",
      "id": "prometheus",
      "name": "Prometheus",
      "version": "1.0.0"
    },
    {
      "type": "panel",
      "id": "stat",
      "name": "Stat",
      "version": ""
    },
    {
      "type": "panel",
      "id": "timeseries",
      "name": "Time series",
      "version": ""
    }
  ],
  "annotations": {
    "list": []
  },
  "description": "miniboxd container runtime — ops counters, durations, active containers",
  "editable": true,
  "fiscalYearStartMonth": 0,
  "graphTooltip": 1,
  "id": null,
  "links": [],
  "panels": [
    {
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "thresholds" },
          "mappings": [],
          "thresholds": {
            "mode": "absolute",
            "steps": [
              { "color": "green", "value": null },
              { "color": "yellow", "value": 10 },
              { "color": "red", "value": 50 }
            ]
          },
          "unit": "short"
        },
        "overrides": []
      },
      "gridPos": { "h": 4, "w": 4, "x": 0, "y": 0 },
      "id": 1,
      "options": {
        "colorMode": "value",
        "graphMode": "none",
        "justifyMode": "center",
        "orientation": "auto",
        "reduceOptions": { "calcs": ["lastNotNull"], "fields": "", "values": false },
        "textMode": "auto"
      },
      "title": "Active Containers",
      "type": "stat",
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
          "expr": "minibox_active_containers",
          "legendFormat": "active",
          "refId": "A"
        }
      ]
    },
    {
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "palette-classic" },
          "custom": {
            "axisCenteredZero": false,
            "axisColorMode": "text",
            "axisLabel": "ops/s",
            "axisPlacement": "auto",
            "barAlignment": 0,
            "drawStyle": "line",
            "fillOpacity": 10,
            "gradientMode": "none",
            "hideFrom": { "legend": false, "tooltip": false, "viz": false },
            "lineInterpolation": "linear",
            "lineWidth": 1,
            "pointSize": 5,
            "scaleDistribution": { "type": "linear" },
            "showPoints": "never",
            "spanNulls": false,
            "stacking": { "group": "A", "mode": "none" },
            "thresholdsStyle": { "mode": "off" }
          },
          "mappings": [],
          "thresholds": { "mode": "absolute", "steps": [{ "color": "green", "value": null }] },
          "unit": "ops"
        },
        "overrides": []
      },
      "gridPos": { "h": 8, "w": 10, "x": 4, "y": 0 },
      "id": 2,
      "options": {
        "legend": { "calcs": ["mean", "max"], "displayMode": "table", "placement": "bottom" },
        "tooltip": { "mode": "multi", "sort": "none" }
      },
      "title": "Op Rate (ops/s)",
      "type": "timeseries",
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
          "expr": "sum by (op) (rate(minibox_container_ops_total{status=\"ok\"}[2m]))",
          "legendFormat": "{{op}} ok",
          "refId": "A"
        }
      ]
    },
    {
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "palette-classic" },
          "custom": {
            "drawStyle": "line",
            "fillOpacity": 10,
            "lineWidth": 1,
            "showPoints": "never",
            "spanNulls": false
          },
          "unit": "percentunit"
        },
        "overrides": []
      },
      "gridPos": { "h": 8, "w": 10, "x": 14, "y": 0 },
      "id": 3,
      "options": {
        "legend": { "calcs": ["mean"], "displayMode": "table", "placement": "bottom" },
        "tooltip": { "mode": "multi", "sort": "none" }
      },
      "title": "Error Rate by Op",
      "type": "timeseries",
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
          "expr": "sum by (op) (rate(minibox_container_ops_total{status=\"error\"}[2m])) / sum by (op) (rate(minibox_container_ops_total[2m]))",
          "legendFormat": "{{op}}",
          "refId": "A"
        }
      ]
    },
    {
      "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
      "fieldConfig": {
        "defaults": {
          "color": { "mode": "palette-classic" },
          "custom": {
            "drawStyle": "line",
            "fillOpacity": 10,
            "lineWidth": 1,
            "showPoints": "never",
            "spanNulls": false
          },
          "unit": "s"
        },
        "overrides": []
      },
      "gridPos": { "h": 8, "w": 24, "x": 0, "y": 8 },
      "id": 4,
      "options": {
        "legend": { "calcs": ["mean", "max"], "displayMode": "table", "placement": "bottom" },
        "tooltip": { "mode": "multi", "sort": "none" }
      },
      "title": "Op Duration p95 (seconds)",
      "type": "timeseries",
      "targets": [
        {
          "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
          "expr": "histogram_quantile(0.95, sum by (op, le) (rate(minibox_container_op_duration_seconds_bucket[2m])))",
          "legendFormat": "{{op}} p95",
          "refId": "A"
        },
        {
          "datasource": { "type": "prometheus", "uid": "${DS_PROMETHEUS}" },
          "expr": "histogram_quantile(0.50, sum by (op, le) (rate(minibox_container_op_duration_seconds_bucket[2m])))",
          "legendFormat": "{{op}} p50",
          "refId": "B"
        }
      ]
    }
  ],
  "refresh": "10s",
  "schemaVersion": 38,
  "tags": ["minibox", "containers"],
  "templating": {
    "list": [
      {
        "current": {},
        "hide": 0,
        "includeAll": false,
        "multi": false,
        "name": "DS_PROMETHEUS",
        "options": [],
        "query": "prometheus",
        "refresh": 1,
        "type": "datasource",
        "label": "Prometheus Datasource"
      }
    ]
  },
  "time": { "from": "now-1h", "to": "now" },
  "timepicker": {},
  "timezone": "browser",
  "title": "Minibox Container Runtime",
  "uid": "minibox-runtime-v1",
  "version": 1,
  "weekStart": ""
}
```

- [ ] **Step 2: Validate JSON is well-formed**

```bash
python3 -c "import json; json.load(open('grafana/minibox-dashboard.json')); print('valid')"
```

Expected: `valid`

- [ ] **Step 3: Commit**

```bash
git add grafana/minibox-dashboard.json
git commit -m "feat(grafana): add importable Minibox Container Runtime dashboard"
```

---

## Task 7: Final verification

- [ ] **Step 1: Run all dashbox tests**

```bash
cargo test -p dashbox
```

Expected: all tests pass (including the 7 metrics parsing tests from Task 2).

- [ ] **Step 2: Run full workspace check**

```bash
cargo check --workspace
```

Expected: no errors.

- [ ] **Step 3: Run clippy**

```bash
cargo clippy -p dashbox -- -D warnings
```

Fix any warnings before proceeding.

- [ ] **Step 4: Build release binary**

```bash
cargo build -p dashbox --release
```

Expected: clean build.

- [ ] **Step 5: Smoke test (optional, if miniboxd is running)**

```bash
./target/release/dashbox
```

Press `8` to navigate to the Metrics tab. With daemon offline: should show "OFFLINE" badge and instructions. With daemon running: should show "LIVE" badge, active containers gauge, ops counters table, duration table.

- [ ] **Step 6: Final commit**

```bash
git add -A
git commit -m "feat(dashbox): Metrics tab + Grafana dashboard — complete"
```

---

## Notes for Implementer

- `ureq` v2 uses `ureq::Error::Transport` for connection failures; `ErrorKind::ConnectionFailed` covers `ECONNREFUSED`. If the daemon is up but `/metrics` returns non-200, the `Err` branch handles it via the red error paragraph.
- The Prometheus text parser in `metrics.rs` is intentionally simple — it handles the three metric families miniboxd emits. It does not need to handle all Prometheus edge cases.
- `Tab::ALL` length changed from 7 to 8. The `from_index` bounds are handled by `.get(i)` returning `None` for out-of-range — no overflow risk.
- Grafana dashboard `uid` is `"minibox-runtime-v1"` — change this if importing multiple times to avoid collision.
