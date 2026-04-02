//! `minibox events` — stream container lifecycle events as JSON-lines to stdout.

use anyhow::Result;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::path::Path;

/// Execute the `events` command: subscribe to the daemon event stream and
/// print each [`ContainerEvent`] as a JSON line until interrupted or the
/// daemon closes the connection.
pub async fn execute(socket_path: &Path) -> Result<()> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client.call(DaemonRequest::SubscribeEvents).await?;

    loop {
        match stream.next().await? {
            Some(DaemonResponse::Event { event }) => {
                let line = serde_json::to_string(&event)?;
                println!("{line}");
            }
            Some(DaemonResponse::Error { message }) => {
                anyhow::bail!("{message}");
            }
            Some(_) => {
                // Ignore unexpected response types.
            }
            None => {
                // Server closed the connection.
                break;
            }
        }
    }

    Ok(())
}
