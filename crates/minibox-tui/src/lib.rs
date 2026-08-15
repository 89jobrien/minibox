//! Read-only terminal dashboard for the minibox daemon.
//!
//! Polls `DaemonRequest::List` on a timer for the container table and subscribes to
//! `DaemonRequest::SubscribeEvents` for a live-tailing lifecycle event log. No mutating
//! requests are sent from this crate — run/stop/exec stay in `mbx`'s existing subcommands.

pub mod app;
pub mod event;
pub mod ui;

use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use miette::{IntoDiagnostic, Result, WrapErr};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use app::App;
use event::Message;

/// Run the TUI event loop until the user quits. Restores the terminal on exit,
/// including on error, so a panic or daemon-connection failure never leaves the
/// shell in raw/alternate-screen mode.
pub async fn run() -> Result<()> {
    enable_raw_mode()
        .into_diagnostic()
        .wrap_err("enable raw mode")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)
        .into_diagnostic()
        .wrap_err("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)
        .into_diagnostic()
        .wrap_err("init terminal")?;

    let result = run_loop(&mut terminal).await;

    disable_raw_mode()
        .into_diagnostic()
        .wrap_err("disable raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .into_diagnostic()
        .wrap_err("leave alternate screen")?;

    result
}

async fn run_loop(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    event::spawn_input_reader(tx.clone());
    event::spawn_ticker(tx.clone());
    event::spawn_event_subscriber(tx.clone());

    let mut app = App::new();
    event::refresh_containers(&tx).await;

    loop {
        terminal
            .draw(|frame| ui::draw(frame, &app))
            .into_diagnostic()
            .wrap_err("draw frame")?;

        let Some(msg) = rx.recv().await else {
            break;
        };

        let is_tick = matches!(msg, Message::Tick);
        if !event::apply(&mut app, msg) {
            break;
        }
        if is_tick {
            event::refresh_containers(&tx).await;
        }
    }

    Ok(())
}
