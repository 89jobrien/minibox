// dashbox/src/main.rs
mod app;
mod command;
mod data;
mod diagram;
mod diagrams;
mod palette;
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

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = io::stdout().execute(LeaveAlternateScreen);
    }
}

fn main() -> Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let _guard = TerminalGuard;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    run(&mut terminal)
}

fn dispatch_action(app: &mut App, action: TabAction) {
    match action {
        TabAction::RunInline { cmd, args } => {
            if app.inline_cmd.is_none() {
                app.inline_cmd = InlineCommand::spawn(&cmd, &args).ok();
            }
        }
        TabAction::RunBackground { cmd, args, label } => {
            if app.bg_cmd.is_none() {
                app.notification = Some((format!("{label}..."), std::time::Instant::now()));
                app.bg_cmd = BackgroundCommand::spawn(&cmd, &args, label).ok();
            }
        }
        TabAction::Quit => app.should_quit = true,
        TabAction::OpenUrl(url) => {
            let opener = if std::env::consts::OS == "macos" {
                "open"
            } else {
                "xdg-open"
            };
            let _ = std::process::Command::new(opener).arg(&url).spawn();
        }
        TabAction::None => {}
    }
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = App::new();

    loop {
        terminal.draw(|f| ui::render(f, &mut app))?;

        app.poll_commands();

        if app.pending_refresh {
            app.active_tab_renderer().refresh();
            app.pending_refresh = false;
        }

        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Esc: close palette first, then inline pane, then quit
                if key.code == KeyCode::Esc {
                    if app.palette.is_some() {
                        app.close_palette();
                        continue;
                    } else if app.inline_cmd.is_some() {
                        app.inline_cmd = None;
                        continue;
                    } else {
                        app.should_quit = true;
                        continue;
                    }
                }

                // If palette is open, route all keys into it
                if app.palette.is_some() {
                    match key.code {
                        KeyCode::Down | KeyCode::Char('j') => {
                            if let Some(ref mut p) = app.palette {
                                p.select_next();
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if let Some(ref mut p) = app.palette {
                                p.select_previous();
                            }
                        }
                        KeyCode::Char(' ') => {
                            app.close_palette();
                        }
                        KeyCode::Enter => {
                            if let Some(palette) = app.palette.take() {
                                if let Some(action) = palette.handle_key(None, true) {
                                    dispatch_action(&mut app, action);
                                }
                            }
                        }
                        KeyCode::Char(c) => {
                            if let Some(palette) = app.palette.take() {
                                if let Some(action) = palette.handle_key(Some(c), false) {
                                    dispatch_action(&mut app, action);
                                }
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Global keys (not captured by tabs when inline is open)
                if app.inline_cmd.is_none() {
                    match key.code {
                        KeyCode::Char('q') => {
                            app.should_quit = true;
                            continue;
                        }
                        KeyCode::Char(' ') => {
                            app.open_palette();
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
                            app.select_tab(Tab::Items);
                            continue;
                        }
                        KeyCode::Char('7') => {
                            app.select_tab(Tab::Ci);
                            continue;
                        }
                        KeyCode::Char('8') => {
                            app.select_tab(Tab::Diagrams);
                            continue;
                        }
                        KeyCode::Char('9') => {
                            app.select_tab(Tab::Metrics);
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
                dispatch_action(&mut app, action);
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
