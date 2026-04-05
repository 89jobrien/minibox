// dashbox/src/palette.rs
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Clear, Row, Table, TableState};

use crate::tabs::{PaletteEntry, TabAction};

pub struct CommandPalette {
    entries: Vec<PaletteEntry>,
    state: TableState,
}

impl CommandPalette {
    pub fn new(entries: Vec<PaletteEntry>) -> Self {
        let mut state = TableState::default();
        if !entries.is_empty() {
            state.select(Some(0));
        }
        Self { entries, state }
    }

    pub fn select_next(&mut self) {
        self.state.select_next();
    }

    pub fn select_previous(&mut self) {
        self.state.select_previous();
    }

    /// If `ch` matches any entry key, return that entry's action (consuming self).
    /// If Enter is pressed, fire the currently selected entry.
    /// Returns None if no match.
    pub fn handle_key(self, ch: Option<char>, enter: bool) -> Option<TabAction> {
        if enter {
            if let Some(idx) = self.state.selected() {
                if let Some(entry) = self.entries.into_iter().nth(idx) {
                    return Some(entry.action);
                }
            }
            return None;
        }
        if let Some(c) = ch {
            for entry in self.entries {
                if entry.key == c {
                    return Some(entry.action);
                }
            }
        }
        None
    }

    /// Render the palette as a centered floating overlay.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let label_width = self
            .entries
            .iter()
            .map(|e| e.label.len())
            .max()
            .unwrap_or(10) as u16;
        // box width: 2 (border) + 2 (key col) + 2 (gap) + label_width + 1 (padding)
        let box_w = (label_width + 7).max(20);
        let box_h = self.entries.len() as u16 + 2; // border top+bottom

        // Centre the box
        let x = area.x + area.width.saturating_sub(box_w) / 2;
        let y = area.y + area.height.saturating_sub(box_h) / 2;
        let popup_area = Rect {
            x,
            y,
            width: box_w.min(area.width),
            height: box_h.min(area.height),
        };

        // Clear background so the overlay is opaque
        frame.render_widget(Clear, popup_area);

        let widths = [Constraint::Length(2), Constraint::Fill(1)];
        let rows: Vec<Row> = self
            .entries
            .iter()
            .map(|e| {
                Row::new([
                    Cell::from(Span::styled(
                        e.key.to_string(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Cell::from(e.label),
                ])
            })
            .collect();

        let table = Table::new(rows, widths)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Actions ")
                    .title_alignment(Alignment::Center)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_stateful_widget(table, popup_area, &mut self.state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: char, label: &'static str) -> PaletteEntry {
        PaletteEntry {
            key,
            label,
            action: TabAction::None,
        }
    }

    #[test]
    fn key_match_fires_action() {
        let palette = CommandPalette::new(vec![entry('t', "run tests")]);
        let result = palette.handle_key(Some('t'), false);
        assert!(matches!(result, Some(TabAction::None)));
    }

    #[test]
    fn no_match_returns_none() {
        let palette = CommandPalette::new(vec![entry('t', "run tests")]);
        let result = palette.handle_key(Some('x'), false);
        assert!(result.is_none());
    }

    #[test]
    fn enter_fires_selected() {
        let palette = CommandPalette::new(vec![entry('t', "run tests")]);
        let result = palette.handle_key(None, true);
        assert!(matches!(result, Some(TabAction::None)));
    }

    #[test]
    fn empty_palette_enter_returns_none() {
        let palette = CommandPalette::new(vec![]);
        let result = palette.handle_key(None, true);
        assert!(result.is_none());
    }
}
