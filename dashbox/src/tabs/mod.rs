// dashbox/src/tabs/mod.rs
pub mod agents;
pub mod bench;
pub mod ci;
pub mod diagrams;
pub mod git;
pub mod history;
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

pub trait TabRenderer {
    fn render(&mut self, frame: &mut Frame, area: Rect);
    fn handle_key(&mut self, key: KeyEvent) -> TabAction;
    fn refresh(&mut self);
    fn status_keys(&self) -> &'static str;
}
