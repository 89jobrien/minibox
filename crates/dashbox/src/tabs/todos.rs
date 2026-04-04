// dashbox/src/tabs/todos.rs
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::todos::{TodosData, TodosSource};

pub struct TodosTab {
    source: CachedSource<TodosSource>,
    table_state: TableState,
    pub cached_data: Option<TodosData>,
}

impl TodosTab {
    pub fn new() -> Self {
        Self {
            source: CachedSource::new(TodosSource::new(), 10),
            table_state: TableState::default(),
            cached_data: None,
        }
    }
}

impl TabRenderer for TodosTab {
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
        self.cached_data = Some(data.clone());

        let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(area);

        // Header
        let header_line = Line::from(vec![
            Span::styled(
                "Todos",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} pending", data.pending),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} completed", data.completed),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} total", data.total),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(header_line), chunks[0]);

        let header = Row::new(["P", "Status", "Content", "Tags"])
            .style(Style::default().fg(Color::DarkGray));
        let widths = [
            Constraint::Length(4),
            Constraint::Length(10),
            Constraint::Fill(1),
            Constraint::Length(20),
        ];

        let rows: Vec<Row> = data
            .todos
            .iter()
            .filter(|t| t.status == "pending")
            .map(|t| {
                let content: String = t.content.chars().take(60).collect();
                let tags = t.tags.join(", ");
                Row::new([
                    Cell::from(t.priority.to_string()).style(Style::default().fg(Color::Cyan)),
                    Cell::from("pending").style(Style::default().fg(Color::Yellow)),
                    Cell::from(content),
                    Cell::from(tags).style(Style::default().fg(Color::DarkGray)),
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
            KeyCode::Char('c') => {
                let idx = match self.table_state.selected() {
                    Some(i) => i,
                    None => return TabAction::None,
                };
                // Visible rows are pending todos only — find the idx-th pending todo.
                let uuid = self.cached_data.as_ref().and_then(|d| {
                    d.todos
                        .iter()
                        .filter(|t| t.status == "pending")
                        .nth(idx)
                        .map(|t| t.doob_uuid.clone())
                });
                match uuid {
                    Some(u) if !u.is_empty() => TabAction::RunBackground {
                        cmd: "doob".into(),
                        args: vec!["todo".into(), "complete".into(), u],
                        label: "complete todo".into(),
                    },
                    _ => TabAction::None,
                }
            }
            KeyCode::Char('o') => {
                let idx = match self.table_state.selected() {
                    Some(i) => i,
                    None => return TabAction::None,
                };
                let tag = self.cached_data.as_ref().and_then(|d| {
                    d.todos
                        .iter()
                        .filter(|t| t.status == "pending")
                        .nth(idx)
                        .and_then(|t| t.tags.first().cloned())
                });
                match tag {
                    Some(t) if !t.is_empty() => TabAction::RunBackground {
                        cmd: "open".into(),
                        args: vec![format!(
                            "https://github.com/89jobrien/minibox/issues/{}",
                            t.trim_start_matches("minibox-")
                        )],
                        label: "open in browser".into(),
                    },
                    _ => TabAction::None,
                }
            }
            _ => TabAction::None,
        }
    }

    fn refresh(&mut self) {
        self.source.refresh();
    }

    fn status_keys(&self) -> &'static str {
        "j/k:scroll  c:complete  o:open-browser  r:refresh"
    }
}
