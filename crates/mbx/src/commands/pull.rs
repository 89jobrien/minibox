//! `minibox pull` — pull an image from Docker Hub.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Execute the `pull` subcommand.
///
/// Downloads the image layers to the daemon's local image store.  Because
/// layer downloads can take time, prints a "Pulling…" indicator before
/// sending the request and waits for the (potentially slow) daemon response.
pub async fn execute(
    image: String,
    tag: String,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    eprintln!("Pulling {image}:{tag}…");

    let request = DaemonRequest::Pull {
        image: image.clone(),
        tag: Some(tag.clone()),
        platform: None,
    };

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
            message: "pulled".to_string(),
        })
        .await;
        let result = execute("alpine".to_string(), "latest".to_string(), &socket_path).await;
        assert!(result.is_ok(), "execute should succeed: {result:?}");
    }
}
