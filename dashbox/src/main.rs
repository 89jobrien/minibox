// dashbox/src/main.rs
mod app;
mod command;
mod data;
mod tabs;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::prelude::CrosstermBackend;

use app::{App, Tab};
use command::{BackgroundCommand, InlineCommand};
use tabs::TabAction;

fn main() -> Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let result = run(&mut terminal);

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();

    loop {
        terminal.draw(|f| ui::render(f, &mut app))?;

        app.poll_commands();

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Esc: close inline pane if open, otherwise quit
                if key.code == KeyCode::Esc {
                    if app.inline_cmd.is_some() {
                        app.inline_cmd = None;
                        continue;
                    } else {
                        app.should_quit = true;
                        continue;
                    }
                }

                // Global keys (not captured by tabs when inline is open)
                if app.inline_cmd.is_none() {
                    match key.code {
                        KeyCode::Char('q') => {
                            app.should_quit = true;
                            continue;
                        }
                        KeyCode::Char('1') => {
                            app.select_tab(Tab::Agents);
                            continue;
                        }
                        KeyCode::Char('2') => {
                            app.select_tab(Tab::Bench);
                            continue;
                        }
                        KeyCode::Char('3') => {
                            app.select_tab(Tab::History);
                            continue;
                        }
                        KeyCode::Char('4') => {
                            app.select_tab(Tab::Git);
                            continue;
                        }
                        KeyCode::Char('5') => {
                            app.select_tab(Tab::Todos);
                            continue;
                        }
                        KeyCode::Char('6') => {
                            app.select_tab(Tab::Ci);
                            continue;
                        }
                        KeyCode::Left => {
                            app.prev_tab();
                            continue;
                        }
                        KeyCode::Right => {
                            app.next_tab();
                            continue;
                        }
                        KeyCode::Char('r') => {
                            app.active_tab_renderer().refresh();
                            continue;
                        }
                        _ => {}
                    }
                }

                // Forward to active tab
                let action = app.active_tab_renderer().handle_key(key);
                match action {
                    TabAction::RunInline { cmd, args } => {
                        if app.inline_cmd.is_none() {
                            app.inline_cmd = InlineCommand::spawn(&cmd, &args).ok();
                        }
                    }
                    TabAction::RunBackground { cmd, args, label } => {
                        if app.bg_cmd.is_none() {
                            app.bg_cmd = BackgroundCommand::spawn(&cmd, &args, label).ok();
                        }
                    }
                    TabAction::Quit => app.should_quit = true,
                    TabAction::OpenUrl(_url) => {
                        // Future: open in browser
                    }
                    TabAction::None => {}
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
