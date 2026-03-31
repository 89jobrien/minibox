// dashbox/src/tabs/git.rs
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::git::GitSource;

pub struct GitTab {
    source: CachedSource<GitSource>,
    table_state: TableState,
}

impl GitTab {
    pub fn new() -> Self {
        Self {
            source: CachedSource::new(GitSource::new(), 10),
            table_state: TableState::default(),
        }
    }
}

impl TabRenderer for GitTab {
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

        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(6),
        ])
        .split(area);

        // Branch info
        let clean_indicator = if data.is_clean { "clean" } else { "dirty" };
        let clean_color = if data.is_clean {
            Color::Green
        } else {
            Color::Yellow
        };
        let branch_line = Line::from(vec![
            Span::styled(
                &data.branch,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(clean_indicator, Style::default().fg(clean_color)),
            if data.ahead > 0 {
                Span::styled(
                    format!("  +{} ahead", data.ahead),
                    Style::default().fg(Color::Green),
                )
            } else {
                Span::raw("")
            },
            if data.behind > 0 {
                Span::styled(
                    format!("  -{} behind", data.behind),
                    Style::default().fg(Color::Red),
                )
            } else {
                Span::raw("")
            },
        ]);
        frame.render_widget(
            Paragraph::new(branch_line).block(Block::default().borders(Borders::BOTTOM)),
            chunks[0],
        );

        // Commits table
        let header = Row::new(["Hash", "Author", "Age", "Message"])
            .style(Style::default().fg(Color::DarkGray));
        let widths = [
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Length(16),
            Constraint::Fill(1),
        ];
        let rows: Vec<Row> = data
            .commits
            .iter()
            .map(|c| {
                Row::new([
                    Cell::from(c.hash.as_str()).style(Style::default().fg(Color::Yellow)),
                    Cell::from(c.author.as_str()),
                    Cell::from(c.age.as_str()).style(Style::default().fg(Color::DarkGray)),
                    Cell::from(c.message.as_str()),
                ])
            })
            .collect();
        let table = Table::new(rows, widths)
            .header(header)
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(table, chunks[1], &mut self.table_state);

        // Changed files
        if !data.changed_files.is_empty() {
            let file_lines: Vec<Line> = data
                .changed_files
                .iter()
                .map(|f| {
                    let color = match f.status.as_str() {
                        "M" => Color::Yellow,
                        "A" | "?" => Color::Green,
                        "D" => Color::Red,
                        _ => Color::White,
                    };
                    Line::from(vec![
                        Span::styled(format!("{:>2} ", f.status), Style::default().fg(color)),
                        Span::raw(&f.path),
                    ])
                })
                .collect();
            frame.render_widget(
                Paragraph::new(file_lines)
                    .block(Block::default().borders(Borders::TOP).title("Changed")),
                chunks[2],
            );
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
        "j/k:scroll  r:refresh"
    }
}
