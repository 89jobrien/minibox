// dashbox/src/ui.rs
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};

use crate::app::{App, Tab};

pub fn render(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(3), // tab bar
        Constraint::Min(0),    // content
        Constraint::Length(1), // status bar
    ])
    .split(frame.area());

    // Tab bar
    let titles: Vec<Line> = Tab::ALL.iter().map(|t| Line::from(t.title())).collect();
    let tabs = Tabs::new(titles)
        .select(app.active_tab.index())
        .highlight_style(Style::default().fg(Color::Cyan).bold());
    frame.render_widget(tabs, chunks[0]);

    // Content area — split if inline command is active
    if let Some(ref inline) = app.inline_cmd {
        let content_chunks =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(chunks[1]);

        // Top: tab content
        let idx = app.active_tab.index();
        app.tabs[idx].render(frame, content_chunks[0]);

        // Bottom: inline command output
        let visible_start = if inline.lines.len() > content_chunks[1].height as usize {
            inline.lines.len() - content_chunks[1].height as usize + 2
        } else {
            0
        };
        let visible_lines: Vec<Line> = inline.lines[visible_start..]
            .iter()
            .map(|l| Line::from(l.as_str()))
            .collect();
        let title = if inline.finished {
            "Output (done)"
        } else {
            "Output (running...)"
        };
        let output_widget = Paragraph::new(visible_lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .style(Style::default().fg(Color::White));
        frame.render_widget(output_widget, content_chunks[1]);
    } else {
        // Full content area for active tab
        let idx = app.active_tab.index();
        app.tabs[idx].render(frame, chunks[1]);
    }

    // Palette overlay — drawn on top of tab content
    if let Some(ref mut palette) = app.palette {
        palette.render(frame, chunks[1]);
    }

    // Status bar
    let tab_keys = app.tabs[app.active_tab.index()].status_keys();
    let left_text = if app.palette.is_some() {
        " j/k:select  Enter:run  Esc:close".to_string()
    } else if app.inline_cmd.is_some() {
        format!(" Esc:close pane  {tab_keys}")
    } else {
        format!(" q:quit  1-9:tab  r:refresh  Space:actions  {tab_keys}")
    };

    let right_text = if let Some(ref cmd) = app.bg_cmd {
        format!("{} {}s... ", cmd.label, cmd.elapsed_secs())
    } else if let Some((ref msg, _)) = app.notification {
        format!("{msg} ")
    } else {
        String::new()
    };

    // Render left-aligned status
    let status_left = Paragraph::new(left_text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(status_left, chunks[2]);

    // Render right-aligned status
    if !right_text.is_empty() {
        let right_color = if app.bg_cmd.is_some() {
            Color::Yellow
        } else {
            Color::Green
        };
        let status_right = Paragraph::new(right_text)
            .style(Style::default().fg(right_color))
            .alignment(Alignment::Right);
        frame.render_widget(status_right, chunks[2]);
    }
}
