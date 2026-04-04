// dashbox/src/tabs/ci.rs
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use super::{TabAction, TabRenderer};
use crate::data::CachedSource;
use crate::data::ci::{CiData, CiSource};

pub struct CiTab {
    source: CachedSource<CiSource>,
    table_state: TableState,
    pub cached_data: Option<CiData>,
}

impl CiTab {
    pub fn new() -> Self {
        Self {
            source: CachedSource::new(CiSource::new(), 10),
            table_state: TableState::default(),
            cached_data: None,
        }
    }
}

impl TabRenderer for CiTab {
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

        // Header with health indicator
        let health_color = if data.success_rate >= 80.0 {
            Color::Green
        } else if data.success_rate >= 50.0 {
            Color::Yellow
        } else {
            Color::Red
        };
        let header_line = Line::from(vec![
            Span::styled(
                "CI",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{:.0}% success rate", data.success_rate),
                Style::default().fg(health_color),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} runs", data.runs.len()),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(header_line), chunks[0]);

        let header = Row::new(["Workflow", "Branch", "Status", "Time"])
            .style(Style::default().fg(Color::DarkGray));
        let widths = [
            Constraint::Fill(1),
            Constraint::Length(16),
            Constraint::Length(14),
            Constraint::Length(20),
        ];

        let rows: Vec<Row> = data
            .runs
            .iter()
            .map(|run| {
                let (status_text, status_color) = if run.status == "completed" {
                    match run.conclusion.as_str() {
                        "success" => ("success", Color::Green),
                        "failure" => ("failure", Color::Red),
                        "cancelled" => ("cancelled", Color::DarkGray),
                        _ => ("unknown", Color::DarkGray),
                    }
                } else {
                    match run.status.as_str() {
                        "in_progress" => ("running", Color::Yellow),
                        "queued" => ("queued", Color::DarkGray),
                        _ => ("unknown", Color::DarkGray),
                    }
                };
                let time = run
                    .created_at
                    .get(..16)
                    .unwrap_or(&run.created_at)
                    .replace('T', " ");

                Row::new([
                    Cell::from(run.name.as_str()),
                    Cell::from(run.head_branch.as_str()).style(Style::default().fg(Color::Cyan)),
                    Cell::from(status_text).style(Style::default().fg(status_color)),
                    Cell::from(time),
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
            KeyCode::Char('o') => {
                let idx = match self.table_state.selected() {
                    Some(i) => i,
                    None => return TabAction::None,
                };
                let url = self
                    .cached_data
                    .as_ref()
                    .and_then(|d| d.runs.get(idx))
                    .map(|r| r.url.clone())
                    .unwrap_or_default();
                if url.is_empty() {
                    TabAction::None
                } else {
                    TabAction::OpenUrl(url)
                }
            }
            _ => TabAction::None,
        }
    }

    fn refresh(&mut self) {
        self.source.refresh();
    }

    fn status_keys(&self) -> &'static str {
        "j/k:scroll  o:open  r:refresh"
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::data::ci::{CiData, CiRun};

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn run_with_url(url: &str) -> CiRun {
        CiRun {
            name: "CI".into(),
            head_branch: "main".into(),
            status: "completed".into(),
            conclusion: "success".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            database_id: 1,
            url: url.into(),
        }
    }

    fn tab_with_run(run: CiRun) -> CiTab {
        let mut tab = CiTab::new();
        tab.cached_data = Some(CiData {
            runs: vec![run],
            success_rate: 100.0,
        });
        tab.table_state.select(Some(0));
        tab
    }

    #[test]
    fn test_ci_tab_o_key_emits_open_url() {
        let url = "https://github.com/89jobrien/minibox/actions/runs/1";
        let mut tab = tab_with_run(run_with_url(url));
        let action = tab.handle_key(make_key(KeyCode::Char('o')));
        match action {
            TabAction::OpenUrl(u) => assert_eq!(u, url),
            other => panic!(
                "expected OpenUrl, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }
}
