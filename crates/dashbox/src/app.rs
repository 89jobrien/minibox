// dashbox/src/app.rs
use std::time::Instant;

use crate::command::{BackgroundCommand, InlineCommand};
use crate::data::agents::{JsonlFileSource, MultiSourceLog};
use crate::palette::CommandPalette;
use crate::tabs::TabRenderer;
use crate::tabs::agents::AgentsTab;
use crate::tabs::bench::BenchTab;
use crate::tabs::ci::CiTab;
use crate::tabs::diagrams::DiagramsTab;
use crate::tabs::git::GitTab;
use crate::tabs::history::HistoryTab;
use crate::tabs::items::ItemsTab;
use crate::tabs::metrics::MetricsTab;
use crate::tabs::todos::TodosTab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Agents,
    Bench,
    History,
    Git,
    Todos,
    Items,
    Ci,
    Diagrams,
    Metrics,
}

impl Tab {
    pub const ALL: [Tab; 9] = [
        Tab::Agents,
        Tab::Bench,
        Tab::History,
        Tab::Git,
        Tab::Todos,
        Tab::Items,
        Tab::Ci,
        Tab::Diagrams,
        Tab::Metrics,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Agents => "1 Agents",
            Tab::Bench => "2 Bench",
            Tab::History => "3 History",
            Tab::Git => "4 Git",
            Tab::Todos => "5 Todos",
            Tab::Items => "6 Items",
            Tab::Ci => "7 CI",
            Tab::Diagrams => "8 Diagrams",
            Tab::Metrics => "9 Metrics",
        }
    }

    pub fn index(&self) -> usize {
        match self {
            Tab::Agents => 0,
            Tab::Bench => 1,
            Tab::History => 2,
            Tab::Git => 3,
            Tab::Todos => 4,
            Tab::Items => 5,
            Tab::Ci => 6,
            Tab::Diagrams => 7,
            Tab::Metrics => 8,
        }
    }

    pub fn from_index(i: usize) -> Option<Tab> {
        Tab::ALL.get(i).copied()
    }
}

pub struct App {
    pub active_tab: Tab,
    pub should_quit: bool,
    #[allow(dead_code)]
    pub last_refresh: Instant,
    pub tabs: Vec<Box<dyn TabRenderer>>,
    pub inline_cmd: Option<InlineCommand>,
    pub bg_cmd: Option<BackgroundCommand>,
    pub notification: Option<(String, Instant)>,
    pub pending_refresh: bool,
    pub palette: Option<CommandPalette>,
}

impl App {
    pub fn new() -> Self {
        Self {
            active_tab: Tab::Agents,
            should_quit: false,
            last_refresh: Instant::now(),
            tabs: vec![
                Box::new(AgentsTab::new(Box::new(MultiSourceLog::new(vec![
                    // Primary: Claude agent runs (meta-agent, council, ai-review, etc.)
                    Box::new(JsonlFileSource::default_agent_log()),
                    // Secondary: hook/skill run log (protocol-drift, vps-health, etc.)
                    Box::new(JsonlFileSource::new(
                        dirs::home_dir()
                            .expect("home dir")
                            .join(".mbx/automation-runs.jsonl"),
                    )),
                ])))),
                Box::new(BenchTab::new()),
                Box::new(HistoryTab::new()),
                Box::new(GitTab::new()),
                Box::new(TodosTab::new()),
                Box::new(ItemsTab::new()),
                Box::new(CiTab::new()),
                Box::new(DiagramsTab::new(
                    crate::diagram::source::load_user_diagrams(),
                )),
                Box::new(MetricsTab::new()),
            ],
            inline_cmd: None,
            bg_cmd: None,
            notification: None,
            pending_refresh: false,
            palette: None,
        }
    }

    pub fn select_tab(&mut self, tab: Tab) {
        self.active_tab = tab;
    }

    pub fn next_tab(&mut self) {
        let next = (self.active_tab.index() + 1) % Tab::ALL.len();
        self.active_tab = Tab::from_index(next).unwrap_or(Tab::Agents);
    }

    pub fn prev_tab(&mut self) {
        let prev = (self.active_tab.index() + Tab::ALL.len() - 1) % Tab::ALL.len();
        self.active_tab = Tab::from_index(prev).unwrap_or(Tab::Agents);
    }

    pub fn active_tab_renderer(&mut self) -> &mut dyn TabRenderer {
        &mut *self.tabs[self.active_tab.index()]
    }

    /// Open the command palette for the currently active tab.
    /// Does nothing if the tab has no palette actions.
    pub fn open_palette(&mut self) {
        let actions = self.active_tab_renderer().palette_actions();
        if !actions.is_empty() {
            self.palette = Some(CommandPalette::new(actions));
        }
    }

    /// Close the command palette without firing any action.
    pub fn close_palette(&mut self) {
        self.palette = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::BackgroundCommand;

    #[test]
    fn test_poll_commands_sets_pending_refresh_on_success() {
        let mut app = App::new();
        let cmd = BackgroundCommand::spawn("true", &[], "test op".to_string()).expect("spawn true");
        app.bg_cmd = Some(cmd);

        for _ in 0..50 {
            app.poll_commands();
            if app.bg_cmd.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(app.bg_cmd.is_none(), "bg_cmd should be cleared");
        assert!(
            app.pending_refresh,
            "pending_refresh should be true after exit 0"
        );
    }
}

impl App {
    pub fn poll_commands(&mut self) {
        if let Some(ref mut cmd) = self.inline_cmd {
            cmd.poll();
        }
        if let Some(ref mut cmd) = self.bg_cmd {
            cmd.poll();
            if cmd.finished {
                let success = cmd.exit_code == Some(0);
                let msg = if success {
                    format!("{} complete", cmd.label)
                } else {
                    let stderr = cmd.stderr_tail.as_deref().unwrap_or("").trim();
                    if stderr.is_empty() {
                        format!(
                            "{} failed (exit {})",
                            cmd.label,
                            cmd.exit_code.unwrap_or(-1)
                        )
                    } else {
                        format!(
                            "{} failed (exit {}): {}",
                            cmd.label,
                            cmd.exit_code.unwrap_or(-1),
                            stderr
                        )
                    }
                };
                self.notification = Some((msg, Instant::now()));
                if success {
                    self.pending_refresh = true;
                }
                self.bg_cmd = None;
            }
        }
        // Clear notification after 5s
        if let Some((_, when)) = &self.notification {
            if when.elapsed().as_secs() >= 5 {
                self.notification = None;
            }
        }
    }
}
