// dashbox/src/app.rs
use std::time::Instant;

use crate::command::{BackgroundCommand, InlineCommand};
use crate::tabs::TabRenderer;
use crate::tabs::agents::AgentsTab;
use crate::tabs::bench::BenchTab;
use crate::tabs::ci::CiTab;
use crate::tabs::diagrams::DiagramsTab;
use crate::tabs::git::GitTab;
use crate::tabs::history::HistoryTab;
use crate::tabs::todos::TodosTab;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Agents,
    Bench,
    History,
    Git,
    Todos,
    Ci,
    Diagrams,
}

impl Tab {
    pub const ALL: [Tab; 7] = [
        Tab::Agents,
        Tab::Bench,
        Tab::History,
        Tab::Git,
        Tab::Todos,
        Tab::Ci,
        Tab::Diagrams,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Tab::Agents => "1 Agents",
            Tab::Bench => "2 Bench",
            Tab::History => "3 History",
            Tab::Git => "4 Git",
            Tab::Todos => "5 Todos",
            Tab::Ci => "6 CI",
            Tab::Diagrams => "7 Diagrams",
        }
    }

    pub fn index(&self) -> usize {
        match self {
            Tab::Agents => 0,
            Tab::Bench => 1,
            Tab::History => 2,
            Tab::Git => 3,
            Tab::Todos => 4,
            Tab::Ci => 5,
            Tab::Diagrams => 6,
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
}

impl App {
    pub fn new() -> Self {
        Self {
            active_tab: Tab::Agents,
            should_quit: false,
            last_refresh: Instant::now(),
            tabs: vec![
                Box::new(AgentsTab::new()),
                Box::new(BenchTab::new()),
                Box::new(HistoryTab::new()),
                Box::new(GitTab::new()),
                Box::new(TodosTab::new()),
                Box::new(CiTab::new()),
                Box::new(DiagramsTab::new(
                    crate::diagram::source::load_user_diagrams(),
                )),
            ],
            inline_cmd: None,
            bg_cmd: None,
            notification: None,
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

    pub fn poll_commands(&mut self) {
        if let Some(ref mut cmd) = self.inline_cmd {
            cmd.poll();
        }
        if let Some(ref mut cmd) = self.bg_cmd {
            cmd.poll();
            if cmd.finished {
                let msg = if cmd.exit_code == Some(0) {
                    format!("{} complete", cmd.label)
                } else {
                    format!(
                        "{} failed (exit {})",
                        cmd.label,
                        cmd.exit_code.unwrap_or(-1)
                    )
                };
                self.notification = Some((msg, Instant::now()));
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
