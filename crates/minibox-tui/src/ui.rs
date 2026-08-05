//! Rendering for the minibox TUI.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table};

use crate::app::App;

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Percentage(40),
            Constraint::Length(1),
        ])
        .split(frame.area());

    draw_containers(frame, app, chunks[0]);
    draw_events(frame, app, chunks[1]);
    draw_status_bar(frame, app, chunks[2]);
}

fn draw_containers(frame: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(["ID", "NAME", "IMAGE", "STATE", "PID"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let rows = app.containers.iter().map(|c| {
        let state_style = match c.state.as_str() {
            "running" => Style::default().fg(Color::Green),
            "stopped" => Style::default().fg(Color::Red),
            _ => Style::default().fg(Color::Yellow),
        };
        Row::new(vec![
            Cell::from(c.id.clone()),
            Cell::from(c.name.clone().unwrap_or_else(|| "-".to_string())),
            Cell::from(c.image.clone()),
            Cell::from(Span::styled(c.state.clone(), state_style)),
            Cell::from(c.pid.map_or_else(|| "-".to_string(), |p| p.to_string())),
        ])
    });

    let widths = [
        Constraint::Length(14),
        Constraint::Length(16),
        Constraint::Percentage(30),
        Constraint::Length(10),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" containers ({}) ", app.containers.len())),
        )
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ratatui::widgets::TableState::default();
    if !app.containers.is_empty() {
        state.select(Some(app.selected));
    }
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_events(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .events
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .map(|e| ListItem::new(e.as_str()))
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(" events "));
    frame.render_widget(list, area);
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let text = if let Some(err) = &app.last_error {
        Line::from(Span::styled(
            format!(" error: {err}"),
            Style::default().fg(Color::Red),
        ))
    } else {
        Line::from(" q: quit  |  j/k, ↑/↓: select  |  live-updating ps + events ")
    };
    frame.render_widget(Paragraph::new(text), area);
}
