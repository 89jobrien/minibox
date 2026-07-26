//! `minibox rm` — remove a stopped container.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Execute the `rm` subcommand.
///
/// The container must already be in the `Stopped` state; use `minibox stop`
/// first if it is still running.
pub async fn execute(id: String, socket_path: &std::path::Path) -> anyhow::Result<()> {
    super::send_request(DaemonRequest::Remove { id }, socket_path).await
}

/// Remove all stopped containers.
pub async fn execute_all(socket_path: &std::path::Path) -> anyhow::Result<()> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::List)
        .await
        .context("failed to call daemon")?;

    let containers = if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ContainerList { containers } => containers,
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
    };

    let stopped: Vec<_> = containers.iter().filter(|c| c.state == "Stopped").collect();

    if stopped.is_empty() {
        println!("No stopped containers to remove.");
        return Ok(());
    }

    let count = stopped.len();
    let mut removed = 0u32;
    for c in &stopped {
        let client = DaemonClient::with_socket(socket_path);
        let mut stream = client
            .call(DaemonRequest::Remove { id: c.id.clone() })
            .await
            .context("failed to call daemon")?;

        if let Some(response) = stream.next().await.context("stream error")? {
            match response {
                DaemonResponse::Success { .. } => {
                    removed += 1;
                }
                DaemonResponse::Error { message } => {
                    eprintln!("warn: failed to remove {}: {message}", c.id);
                }
                _ => {}
            }
        }
    }

    println!("Removed {removed}/{count} stopped containers.");
    Ok(())
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
