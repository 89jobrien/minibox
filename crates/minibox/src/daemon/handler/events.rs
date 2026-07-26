//! Event subscription handler.

use minibox_core::protocol::DaemonResponse;
use std::sync::Arc;
use tracing::warn;

/// Stream container lifecycle events to a client.
///
/// Subscribes to the event broker and forwards each [`ContainerEvent`] as a
/// [`DaemonResponse::Event`] message until the client disconnects (channel
/// send fails) or the broker is shut down.
pub async fn handle_subscribe_events(
    event_source: Arc<dyn minibox_core::events::EventSource>,
    tx: tokio::sync::mpsc::Sender<DaemonResponse>,
) {
    let mut rx = event_source.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) => {
                if tx.send(DaemonResponse::Event { event }).await.is_err() {
                    // Client disconnected.
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!(skipped = n, "events: subscriber lagged, skipping events");
                // Continue — don't break on lag.
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}
