// dashbox/src/tabs/items.rs
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::items::{ItemsData, ItemsSource};

pub struct ItemsTab {
    source: CachedSource<ItemsSource>,
    table_state: TableState,
    pub cached_data: Option<ItemsData>,
}

impl ItemsTab {
    pub fn new() -> Self {
        let mut tab = Self {
            source: CachedSource::new(ItemsSource::new("minibox"), 30),
            table_state: TableState::default(),
            cached_data: None,
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
        self.cached_data = Some(data.clone());

        // Header + list + detail pane
        let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(area);

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
        let has_detail = data
            .items
            .get(selected)
            .and_then(|i| i.description.as_deref())
            .is_some();
        let content_chunks = if has_detail {
            Layout::horizontal([Constraint::Fill(2), Constraint::Fill(1)]).split(chunks[1])
        } else {
            Layout::horizontal([Constraint::Fill(1)]).split(chunks[1])
        };

        // Table
        let col_header =
            Row::new(["ID", "P", "Status", "Title"]).style(Style::default().fg(Color::DarkGray));
        let widths = [
            Constraint::Length(12),
            Constraint::Length(4),
            Constraint::Length(9),
            Constraint::Fill(1),
        ];

        let rows: Vec<Row> = data
            .items
            .iter()
            .map(|item| {
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
            })
            .collect();

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
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
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
                    Paragraph::new(lines).wrap(Wrap { trim: false }).block(
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
                    args: vec![
                        "handoff".into(),
                        "update-status".into(),
                        uuid.clone(),
                        "done".into(),
                    ],
                    label: "mark done".into(),
                },
            },
            crate::tabs::PaletteEntry {
                key: 'o',
                label: "mark open",
                action: TabAction::RunBackground {
                    cmd: "doob".into(),
                    args: vec![
                        "handoff".into(),
                        "update-status".into(),
                        uuid.clone(),
                        "open".into(),
                    ],
                    label: "mark open".into(),
                },
            },
            crate::tabs::PaletteEntry {
                key: 'b',
                label: "mark blocked",
                action: TabAction::RunBackground {
                    cmd: "doob".into(),
                    args: vec![
                        "handoff".into(),
                        "update-status".into(),
                        uuid,
                        "blocked".into(),
                    ],
                    label: "mark blocked".into(),
                },
            },
        ]
    }

    fn refresh(&mut self) {
        self.source.refresh();
    }

    fn status_keys(&self) -> &'static str {
        "j/k:scroll  r:refresh"
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::data::items::{HandoffItem, ItemsData};

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn make_item(doob_uuid: &str, status: &str) -> HandoffItem {
        HandoffItem {
            handoff_id: format!("minibox-{doob_uuid}"),
            title: "Test".into(),
            description: None,
            priority: "P1".into(),
            status: status.into(),
            files: vec![],
            doob_uuid: doob_uuid.into(),
        }
    }

    fn tab_with_data(items: Vec<HandoffItem>) -> ItemsTab {
        let open = items.iter().filter(|i| i.status == "open").count();
        let done = items.iter().filter(|i| i.status == "done").count();
        let blocked = items.iter().filter(|i| i.status == "blocked").count();
        let data = ItemsData {
            items,
            open,
            done,
            blocked,
        };
        let mut tab = ItemsTab::new();
        tab.cached_data = Some(data);
        tab.table_state.select(Some(0));
        tab
    }

    #[test]
    fn test_items_tab_palette_actions_mark_done() {
        let mut tab = tab_with_data(vec![make_item("uuid-abc", "open")]);
        let actions = tab.palette_actions();
        let done_entry = actions.iter().find(|e| e.key == 'c').expect("no 'c' entry");
        match &done_entry.action {
            TabAction::RunBackground { cmd, args, label } => {
                assert_eq!(cmd, "doob");
                assert!(args.contains(&"uuid-abc".to_string()), "args: {args:?}");
                assert!(args.contains(&"done".to_string()), "args: {args:?}");
                assert_eq!(label, "mark done");
            }
            other => panic!(
                "expected RunBackground, got {:?}",
                std::mem::discriminant(other)
            ),
        }
    }

    #[test]
    fn test_items_tab_no_selection_palette_actions_is_empty() {
        let mut tab = ItemsTab::new();
        tab.table_state.select(None);
        let actions = tab.palette_actions();
        assert!(actions.is_empty());
    }
}
