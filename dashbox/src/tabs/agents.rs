// dashbox/src/tabs/agents.rs
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::agents::{AgentsData, AgentsSource};

pub struct AgentsTab {
    source: CachedSource<AgentsSource>,
    table_state: TableState,
}

impl AgentsTab {
    pub fn new() -> Self {
        Self {
            source: CachedSource::new(AgentsSource::new(), 10),
            table_state: TableState::default(),
        }
    }

    fn render_header(&self, data: &AgentsData) -> Paragraph<'_> {
        let line = Line::from(vec![
            Span::styled(
                "Agents",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} total", data.total),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} complete", data.complete),
                Style::default().fg(Color::Green),
            ),
            if data.running > 0 {
                Span::styled(
                    format!("  {} running", data.running),
                    Style::default().fg(Color::Yellow),
                )
            } else {
                Span::raw("")
            },
            if data.crashed > 0 {
                Span::styled(
                    format!("  {} crashed", data.crashed),
                    Style::default().fg(Color::Red),
                )
            } else {
                Span::raw("")
            },
        ]);
        Paragraph::new(line)
    }
}

impl TabRenderer for AgentsTab {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.source.ensure_fresh();
        let data = match self.source.get() {
            Some(Ok(d)) => d,
            Some(Err(e)) => {
                let msg = Paragraph::new(format!("Error: {e}"))
                    .style(Style::default().fg(Color::Red))
                    .block(Block::default().borders(Borders::ALL).title("Agents"));
                frame.render_widget(msg, area);
                return;
            }
            None => return,
        };

        let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(area);

        frame.render_widget(self.render_header(data), chunks[0]);

        let header = Row::new(["Time", "Script", "Status", "Dur", "Output"])
            .style(Style::default().fg(Color::DarkGray));
        let widths = [
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Fill(1),
        ];

        let rows: Vec<Row> = data
            .runs
            .iter()
            .take(30)
            .map(|run| {
                let time = if run.run_id.len() >= 16 {
                    &run.run_id[..16]
                } else {
                    &run.run_id
                };
                let (status_text, status_color) = match run.status.as_str() {
                    "complete" => ("done", Color::Green),
                    "crashed" => ("crash", Color::Red),
                    "running" => ("live", Color::Yellow),
                    _ => ("?", Color::DarkGray),
                };
                let dur = run
                    .duration_s
                    .map(|d| format!("{d:.1}s"))
                    .unwrap_or_else(|| "-".to_string());
                let output = run
                    .output
                    .as_deref()
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>();

                Row::new([
                    Cell::from(time.replace('T', " ")),
                    Cell::from(run.script.as_str()),
                    Cell::from(status_text).style(Style::default().fg(status_color)),
                    Cell::from(dur),
                    Cell::from(output),
                ])
            })
            .collect();

        let table = Table::new(rows, widths)
            .header(header)
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
            _ => TabAction::None,
        }
    }

    fn refresh(&mut self) {
        self.source.refresh();
    }

    fn status_keys(&self) -> &'static str {
        "j/k:scroll  Enter:expand  r:refresh"
    }
}
