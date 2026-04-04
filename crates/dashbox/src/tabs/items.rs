// dashbox/src/tabs/items.rs
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::items::ItemsSource;

pub struct ItemsTab {
    source: CachedSource<ItemsSource>,
    table_state: TableState,
}

impl ItemsTab {
    pub fn new() -> Self {
        let mut tab = Self {
            source: CachedSource::new(ItemsSource::new("minibox"), 30),
            table_state: TableState::default(),
        };
        tab.table_state.select(Some(0));
        tab
    }
}

fn priority_color(p: &str) -> Color {
    match p {
        "P0" => Color::Red,
        "P1" => Color::Yellow,
        _ => Color::Cyan,
    }
}

fn status_color(s: &str) -> Color {
    match s {
        "open" => Color::Green,
        "blocked" => Color::Red,
        "parked" => Color::DarkGray,
        "done" => Color::DarkGray,
        _ => Color::White,
    }
}

impl TabRenderer for ItemsTab {
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

        // Header + list + detail pane
        let chunks =
            Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(area);

        // Header summary
        let header_line = Line::from(vec![
            Span::styled(
                "Items",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} open", data.open),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} blocked", data.blocked),
                Style::default().fg(Color::Red),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} done", data.done),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(header_line), chunks[0]);

        // Split content area: list on left, detail on right
        let selected = self.table_state.selected().unwrap_or(0);
        let has_detail = data.items.get(selected).and_then(|i| i.description.as_deref()).is_some();
        let content_chunks = if has_detail {
            Layout::horizontal([Constraint::Fill(2), Constraint::Fill(1)]).split(chunks[1])
        } else {
            Layout::horizontal([Constraint::Fill(1)]).split(chunks[1])
        };

        // Table
        let col_header = Row::new(["ID", "P", "Status", "Title"])
            .style(Style::default().fg(Color::DarkGray));
        let widths = [
            Constraint::Length(12),
            Constraint::Length(4),
            Constraint::Length(9),
            Constraint::Fill(1),
        ];

        let rows: Vec<Row> = data.items.iter().map(|item| {
            let title: String = item.title.chars().take(80).collect();
            Row::new([
                Cell::from(item.handoff_id.as_str())
                    .style(Style::default().fg(Color::DarkGray)),
                Cell::from(item.priority.as_str())
                    .style(Style::default().fg(priority_color(&item.priority))),
                Cell::from(item.status.as_str())
                    .style(Style::default().fg(status_color(&item.status))),
                Cell::from(title),
            ])
        }).collect();

        let table = Table::new(rows, widths)
            .header(col_header)
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .block(Block::default().borders(Borders::NONE));
        frame.render_stateful_widget(table, content_chunks[0], &mut self.table_state);

        // Detail pane for selected item
        if has_detail {
            if let Some(item) = data.items.get(selected) {
                let desc = item.description.as_deref().unwrap_or("");
                let mut lines: Vec<Line> = Vec::new();
                lines.push(Line::from(Span::styled(
                    item.title.as_str(),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::raw(""));
                for line in desc.lines() {
                    lines.push(Line::raw(line.to_string()));
                }
                if !item.files.is_empty() {
                    lines.push(Line::raw(""));
                    lines.push(Line::from(Span::styled(
                        "Files:",
                        Style::default().fg(Color::DarkGray),
                    )));
                    for f in &item.files {
                        lines.push(Line::from(Span::styled(
                            format!("  {f}"),
                            Style::default().fg(Color::Cyan),
                        )));
                    }
                }
                frame.render_widget(
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: false })
                        .block(
                            Block::default()
                                .borders(Borders::LEFT)
                                .border_style(Style::default().fg(Color::DarkGray)),
                        ),
                    content_chunks[1],
                );
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
        "j/k:scroll  r:refresh"
    }
}
