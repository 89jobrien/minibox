// dashbox/src/tabs/bench.rs
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::bench::BenchSource;

fn local_hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub struct BenchTab {
    source: CachedSource<BenchSource>,
    table_state: TableState,
    hostname: String,
}

impl BenchTab {
    pub fn new() -> Self {
        Self {
            source: CachedSource::new(BenchSource::new(), 10),
            table_state: TableState::default(),
            hostname: local_hostname(),
        }
    }
}

fn format_duration(us: f64) -> String {
    if us < 1.0 {
        format!("{:.0}ns", us * 1000.0)
    } else if us < 1000.0 {
        format!("{us:.1}us")
    } else if us < 1_000_000.0 {
        format!("{:.1}ms", us / 1000.0)
    } else {
        format!("{:.2}s", us / 1_000_000.0)
    }
}

impl TabRenderer for BenchTab {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.source.ensure_fresh();
        let data = match self.source.get() {
            Some(Ok(d)) => d,
            Some(Err(e)) => {
                let msg = Paragraph::new(format!("Error: {e}"))
                    .style(Style::default().fg(Color::Red))
                    .block(Block::default().borders(Borders::ALL).title("Bench"));
                frame.render_widget(msg, area);
                return;
            }
            None => return,
        };

        let latest = match &data.latest {
            Some(l) => l,
            None => {
                frame.render_widget(
                    Paragraph::new("No bench results. Run: cargo xtask bench")
                        .block(Block::default().borders(Borders::ALL).title("Bench")),
                    area,
                );
                return;
            }
        };

        let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(area);

        // Header
        let sha_end = 8.min(latest.metadata.git_sha.len());
        let header_line = Line::from(vec![
            Span::styled(
                "Benchmarks",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                &latest.metadata.git_sha[..sha_end],
                Style::default().fg(Color::Cyan),
            ),
            Span::raw("  "),
            Span::styled(
                &latest.metadata.hostname,
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("  "),
            Span::styled(
                latest
                    .metadata
                    .timestamp
                    .get(..16)
                    .unwrap_or(&latest.metadata.timestamp),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("  "),
            Span::styled(
                format!(
                    "{} VPS runs",
                    data.history
                        .iter()
                        .filter(|r| r.metadata.hostname == self.hostname)
                        .count()
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(header_line), chunks[0]);

        // Get previous run for deltas
        let vps_runs: Vec<_> = data
            .history
            .iter()
            .filter(|r| r.metadata.hostname == self.hostname)
            .collect();
        let prev = if vps_runs.len() >= 2 {
            Some(&vps_runs[vps_runs.len() - 2])
        } else {
            None
        };

        let header_row = Row::new(["Suite", "Test", "Avg", "P95", "Min", "Iter", "Delta"])
            .style(Style::default().fg(Color::DarkGray));
        let widths = [
            Constraint::Length(10),
            Constraint::Length(16),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Length(10),
        ];

        let mut rows = Vec::new();
        for suite in &latest.suites {
            for test in &suite.tests {
                if test.iterations == 0 {
                    continue;
                }
                let avg = test
                    .avg_us()
                    .map(format_duration)
                    .unwrap_or_else(|| "-".to_string());
                let p95 = test
                    .p95_us()
                    .map(format_duration)
                    .unwrap_or_else(|| "-".to_string());
                let min = test
                    .min_us()
                    .map(format_duration)
                    .unwrap_or_else(|| "-".to_string());

                let (delta_text, delta_color) =
                    if let (Some(prev_run), Some(curr_avg)) = (prev, test.avg_us()) {
                        let prev_test = prev_run
                            .suites
                            .iter()
                            .find(|s| s.name == suite.name)
                            .and_then(|s| s.tests.iter().find(|t| t.name == test.name));
                        if let Some(prev_avg) = prev_test.and_then(|t| t.avg_us()) {
                            let pct = ((curr_avg - prev_avg) / prev_avg) * 100.0;
                            let sign = if pct > 0.0 { "+" } else { "" };
                            let color = if pct > 10.0 {
                                Color::Red
                            } else if pct < -10.0 {
                                Color::Green
                            } else {
                                Color::DarkGray
                            };
                            (format!("{sign}{pct:.1}%"), color)
                        } else {
                            ("-".to_string(), Color::DarkGray)
                        }
                    } else {
                        ("-".to_string(), Color::DarkGray)
                    };

                rows.push(Row::new([
                    Cell::from(suite.name.as_str()).style(Style::default().fg(Color::Magenta)),
                    Cell::from(test.name.as_str()),
                    Cell::from(avg),
                    Cell::from(p95),
                    Cell::from(min),
                    Cell::from(test.iterations.to_string()),
                    Cell::from(delta_text).style(Style::default().fg(delta_color)),
                ]));
            }
        }

        let table = Table::new(rows, widths)
            .header(header_row)
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(table, chunks[1], &mut self.table_state);
    }

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
            KeyCode::Char('t') => TabAction::RunInline {
                cmd: "cargo".to_string(),
                args: vec!["xtask".to_string(), "test-unit".to_string()],
            },
            KeyCode::Char('b') => TabAction::RunInline {
                cmd: "cargo".to_string(),
                args: vec!["xtask".to_string(), "bench".to_string()],
            },
            KeyCode::Char('B') => TabAction::RunBackground {
                cmd: "cargo".to_string(),
                args: vec!["xtask".to_string(), "bench-vps".to_string()],
                label: "VPS bench".to_string(),
            },
            _ => TabAction::None,
        }
    }

    fn refresh(&mut self) {
        self.source.refresh();
    }

    fn status_keys(&self) -> &'static str {
        "j/k:scroll  t:test  b:bench  B:vps-bench  r:refresh"
    }
}
