//! `minibox stop` — stop a running container.

use anyhow::Context;
use linuxbox::protocol::{DaemonRequest, DaemonResponse};
use minibox_client::DaemonClient;

/// Execute the `stop` subcommand.
///
/// Sends a `Stop` request to the daemon, which is responsible for signalling
/// the container process.  Prints the daemon's confirmation message on success
/// or an error description on failure.
pub async fn execute(id: String) -> anyhow::Result<()> {
    let request = DaemonRequest::Stop { id: id.clone() };

    let client = DaemonClient::new().context("failed to create daemon client")?;
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::Success { message } => {
                println!("{message}");
                Ok(())
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("no response from daemon");
        std::process::exit(1);
    }
}
