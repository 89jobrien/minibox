// dashbox/src/tabs/metrics.rs
use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use std::time::SystemTime;

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::metrics::{LiveMetrics, MetricsData, MetricsSource};

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

impl TabRenderer for MetricsTab {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.source.ensure_fresh();

        match self.source.get() {
            None => {
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
                render_offline(frame, area, &self.addr);
            }
            Some(Ok(MetricsData::Live(live))) => {
                let addr = self.addr.clone();
                render_live(frame, area, &addr, live, false, None);
            }
            Some(Ok(MetricsData::Stale(live, written_at))) => {
                let addr = self.addr.clone();
                render_live(frame, area, &addr, live, true, Some(*written_at));
            }
        }
    }

    fn handle_key(&mut self, _key: KeyEvent) -> TabAction {
        TabAction::None
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
        Span::styled(
            "OFFLINE",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(addr, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(status_line), chunks[0]);

    let msg = Paragraph::new(
        "miniboxd is not running or metrics endpoint is unreachable.\nStart the daemon: sudo miniboxd",
    )
    .style(Style::default().fg(Color::DarkGray))
    .block(Block::default().borders(Borders::ALL).title("Metrics"));
    frame.render_widget(msg, chunks[1]);
}

fn render_live(
    frame: &mut Frame,
    area: Rect,
    addr: &str,
    live: &LiveMetrics,
    stale: bool,
    written_at: Option<SystemTime>,
) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // status bar
        Constraint::Min(0),    // content
    ])
    .split(area);

    let status_line = if stale {
        let age = written_at
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|d| {
                let secs = d.as_secs();
                if secs < 60 {
                    format!("{secs}s ago")
                } else {
                    format!("{}m ago", secs / 60)
                }
            })
            .unwrap_or_else(|| "unknown age".to_string());
        Line::from(vec![
            Span::styled(
                "STALE",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(addr, Style::default().fg(Color::DarkGray)),
            Span::raw("  "),
            Span::styled(
                format!("(snapshot from {age})"),
                Style::default().fg(Color::DarkGray),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                "LIVE",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(addr, Style::default().fg(Color::DarkGray)),
        ])
    };
    frame.render_widget(Paragraph::new(status_line), chunks[0]);

    let content =
        Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(chunks[1]);

    let top =
        Layout::horizontal([Constraint::Ratio(3, 10), Constraint::Ratio(7, 10)]).split(content[0]);

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
    let mut ops: Vec<String> = live
        .ops_counters
        .keys()
        .map(|(op, _)| op.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    ops.sort();

    let header =
        Row::new(["Op", "OK", "Error", "Total"]).style(Style::default().fg(Color::DarkGray));
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
            let err_color = if err > 0.0 {
                Color::Red
            } else {
                Color::DarkGray
            };
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

    let header = Row::new(["Op", "p50", "p95"]).style(Style::default().fg(Color::DarkGray));
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
