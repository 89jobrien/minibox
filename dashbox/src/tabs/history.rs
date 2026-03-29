// dashbox/src/tabs/history.rs
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Sparkline, Table, TableState};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::bench::BenchSource;

pub struct HistoryTab {
    source: CachedSource<BenchSource>,
    table_state: TableState,
    selected_test: Option<(String, String)>,
}

impl HistoryTab {
    pub fn new() -> Self {
        Self {
            source: CachedSource::new(BenchSource::new(), 10),
            table_state: TableState::default().with_selected(Some(0)),
            selected_test: None,
        }
    }
}

impl TabRenderer for HistoryTab {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.source.ensure_fresh();
        let data = match self.source.get() {
            Some(Ok(d)) => d,
            Some(Err(e)) => {
                frame.render_widget(
                    Paragraph::new(format!("Error: {e}")).style(Style::default().fg(Color::Red)),
                    area,
                );
                return;
            }
            None => return,
        };

        let vps_runs: Vec<_> = data
            .history
            .iter()
            .filter(|r| r.metadata.hostname == "jobrien")
            .collect();

        if vps_runs.is_empty() {
            frame.render_widget(
                Paragraph::new("No VPS bench history yet.")
                    .block(Block::default().borders(Borders::ALL).title("History")),
                area,
            );
            return;
        }

        // Collect all unique (suite, test) pairs
        let mut test_keys: Vec<(String, String)> = Vec::new();
        if let Some(last) = vps_runs.last() {
            for suite in &last.suites {
                for test in &suite.tests {
                    if test.iterations > 0 {
                        test_keys.push((suite.name.clone(), test.name.clone()));
                    }
                }
            }
        }

        // Auto-select first test if none selected
        if self.selected_test.is_none() && !test_keys.is_empty() {
            self.selected_test = Some(test_keys[0].clone());
        }

        let chunks = Layout::horizontal([Constraint::Length(30), Constraint::Min(0)]).split(area);

        // Left: test list
        let header = Row::new(["Suite", "Test"]).style(Style::default().fg(Color::DarkGray));
        let rows: Vec<Row> = test_keys
            .iter()
            .map(|(suite, test)| {
                Row::new([
                    Cell::from(suite.as_str()).style(Style::default().fg(Color::Magenta)),
                    Cell::from(test.as_str()),
                ])
            })
            .collect();
        let table = Table::new(rows, [Constraint::Length(10), Constraint::Fill(1)])
            .header(header)
            .block(Block::default().borders(Borders::RIGHT))
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(table, chunks[0], &mut self.table_state);

        // Right: sparkline for selected test
        if let Some((ref suite_name, ref test_name)) = self.selected_test {
            let values: Vec<u64> = vps_runs
                .iter()
                .map(|run| {
                    run.suites
                        .iter()
                        .find(|s| &s.name == suite_name)
                        .and_then(|s| s.tests.iter().find(|t| &t.name == test_name))
                        .and_then(|t| t.avg_us())
                        .map(|v| v as u64)
                        .unwrap_or(0)
                })
                .collect();

            let right_chunks = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(5),
                Constraint::Min(0),
            ])
            .split(chunks[1]);

            let title = format!(" {suite_name}/{test_name} — last {} runs", values.len());
            frame.render_widget(
                Paragraph::new(title).style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                right_chunks[0],
            );

            let sparkline = Sparkline::default()
                .data(&values)
                .style(Style::default().fg(Color::Cyan))
                .block(Block::default().borders(Borders::ALL));
            frame.render_widget(sparkline, right_chunks[1]);

            // Stats summary
            if !values.is_empty() {
                let non_zero: Vec<u64> = values.iter().copied().filter(|&v| v > 0).collect();
                if !non_zero.is_empty() {
                    let current = non_zero.last().copied().unwrap_or(0) as f64;
                    let best = *non_zero.iter().min().unwrap_or(&0) as f64;
                    let worst = *non_zero.iter().max().unwrap_or(&0) as f64;
                    let trend = if values.len() >= 2 {
                        let last = *values.last().unwrap_or(&0) as f64;
                        let prev = *values.get(values.len() - 2).unwrap_or(&0) as f64;
                        if prev == 0.0 {
                            "?"
                        } else if last < prev * 0.95 {
                            "improving"
                        } else if last > prev * 1.05 {
                            "regressing"
                        } else {
                            "stable"
                        }
                    } else {
                        "insufficient data"
                    };

                    let stats = format!(
                        " Current: {current:.1}us  Best: {best:.1}us  Worst: {worst:.1}us  Trend: {trend}",
                    );
                    frame.render_widget(
                        Paragraph::new(stats).style(Style::default().fg(Color::DarkGray)),
                        right_chunks[2],
                    );
                }
            }
        }

        // Update selected_test based on table selection
        if let Some(idx) = self.table_state.selected() {
            if let Some(key) = test_keys.get(idx) {
                self.selected_test = Some(key.clone());
            }
        }
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
            _ => TabAction::None,
        }
    }

    fn refresh(&mut self) {
        self.source.refresh();
    }

    fn status_keys(&self) -> &'static str {
        "j/k:select test  r:refresh"
    }
}
