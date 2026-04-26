//! `minibox rm` — remove a stopped container.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Execute the `rm` subcommand.
///
/// The container must already be in the `Stopped` state; use `minibox stop`
/// first if it is still running.
pub async fn execute(id: String, socket_path: &std::path::Path) -> anyhow::Result<()> {
    let request = DaemonRequest::Remove { id: id.clone() };

    let client = DaemonClient::with_socket(socket_path);
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

#[cfg(test)]
mod tests {
    use super::super::test_helpers::setup;
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_succeeds_on_success_response() {
        let (_tmp, socket_path) = setup(DaemonResponse::Success {
            message: "removed".to_string(),
        })
        .await;
        let result = execute("abc123".to_string(), &socket_path).await;
        assert!(result.is_ok(), "execute should succeed: {result:?}");
    }
}
