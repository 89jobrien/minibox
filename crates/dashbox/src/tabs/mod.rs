// dashbox/src/tabs/mod.rs
pub mod agents;
pub mod bench;
pub mod ci;
pub mod diagrams;
pub mod git;
pub mod history;
pub mod items;
pub mod metrics;
pub mod todos;

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::Rect;

#[allow(dead_code)]
pub enum TabAction {
    None,
    Quit,
    RunInline {
        cmd: String,
        args: Vec<String>,
    },
    RunBackground {
        cmd: String,
        args: Vec<String>,
        label: String,
    },
    OpenUrl(String),
}

/// One entry in the command palette for a tab.
pub struct PaletteEntry {
    /// Single character the user presses to trigger this action.
    pub key: char,
    /// Human-readable label shown in the palette overlay.
    pub label: &'static str,
    /// Action to fire when this entry is selected.
    pub action: TabAction,
}

pub trait TabRenderer {
    fn render(&mut self, frame: &mut Frame, area: Rect);
    fn handle_key(&mut self, key: KeyEvent) -> TabAction;
    fn refresh(&mut self);
    fn status_keys(&self) -> &'static str;
    /// Returns the actions available in the command palette for this tab.
    /// Default: no actions.
    fn palette_actions(&mut self) -> Vec<PaletteEntry> {
        vec![]
    }
}
