//! Input handling and background daemon polling for the minibox TUI.

use std::time::Duration;

use crossterm::event::{Event as CtEvent, EventStream, KeyCode, KeyEventKind};
use minibox_core::client::DaemonClient;
use minibox_core::events::ContainerEvent;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::app::App;

const TICK: Duration = Duration::from_secs(1);

/// Messages fed into the main loop from input, the poll timer, and the daemon event stream.
pub enum Message {
    Quit,
    SelectNext,
    SelectPrev,
    Tick,
    ContainerList(Vec<minibox_core::protocol::ContainerInfo>),
    DaemonEvent(ContainerEvent),
    DaemonError(String),
}

/// Spawn a background task that subscribes to the daemon's event stream and forwards
/// each [`ContainerEvent`] to the main loop via `tx`. Runs for the lifetime of the app.
pub fn spawn_event_subscriber(tx: mpsc::UnboundedSender<Message>) {
    tokio::spawn(async move {
        let client = DaemonClient::new();
        let mut stream = match client.call(DaemonRequest::SubscribeEvents).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(Message::DaemonError(format!("subscribe failed: {e}")));
                return;
            }
        };
        loop {
            match stream.next().await {
                Ok(Some(DaemonResponse::Event { event })) => {
                    if tx.send(Message::DaemonEvent(event)).is_err() {
                        return;
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => return,
                Err(e) => {
                    let _ = tx.send(Message::DaemonError(format!("event stream error: {e}")));
                    return;
                }
            }
        }
    });
}

/// Spawn a periodic timer that pushes [`Message::Tick`] onto `tx` every [`TICK`].
pub fn spawn_ticker(tx: mpsc::UnboundedSender<Message>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK);
        loop {
            interval.tick().await;
            if tx.send(Message::Tick).is_err() {
                return;
            }
        }
    });
}

/// Spawn a task translating terminal input into [`Message`]s.
pub fn spawn_input_reader(tx: mpsc::UnboundedSender<Message>) {
    tokio::spawn(async move {
        let mut events = EventStream::new();
        while let Some(Ok(ev)) = events.next().await {
            let msg = match ev {
                CtEvent::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => Some(Message::Quit),
                    KeyCode::Char('j') | KeyCode::Down => Some(Message::SelectNext),
                    KeyCode::Char('k') | KeyCode::Up => Some(Message::SelectPrev),
                    _ => None,
                },
                _ => None,
            };
            if let Some(msg) = msg {
                if tx.send(msg).is_err() {
                    return;
                }
            }
        }
    });
}

/// Fetch the current container list from the daemon and send it as a [`Message`].
pub async fn refresh_containers(tx: &mpsc::UnboundedSender<Message>) {
    let client = DaemonClient::new();
    let result = async {
        let mut stream = client.call(DaemonRequest::List).await?;
        match stream.next().await? {
            Some(DaemonResponse::ContainerList { containers }) => Ok(containers),
            Some(DaemonResponse::Error { message }) => {
                Err(minibox_core::client::ClientError::DaemonError(message))
            }
            _ => Ok(Vec::new()),
        }
    }
    .await;

    match result {
        Ok(containers) => {
            let _ = tx.send(Message::ContainerList(containers));
        }
        Err(e) => {
            let _ = tx.send(Message::DaemonError(format!("list failed: {e}")));
        }
    }
}

/// Apply a [`Message`] to `app`, returning `true` if the app should keep running.
pub fn apply(app: &mut App, msg: Message) -> bool {
    match msg {
        Message::Quit => return false,
        Message::SelectNext => app.select_next(),
        Message::SelectPrev => app.select_prev(),
        Message::Tick => {}
        Message::ContainerList(containers) => {
            app.last_error = None;
            app.set_containers(containers);
        }
        Message::DaemonEvent(event) => app.push_event(&event),
        Message::DaemonError(e) => app.last_error = Some(e),
    }
    true
}
