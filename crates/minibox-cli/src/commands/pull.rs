//! `minibox pull` — pull an image from Docker Hub.

use anyhow::Context;
use linuxbox::protocol::{DaemonRequest, DaemonResponse};
use minibox_client::DaemonClient;

/// Execute the `pull` subcommand.
///
/// Downloads the image layers to the daemon's local image store.  Because
/// layer downloads can take time, prints a "Pulling…" indicator before
/// sending the request and waits for the (potentially slow) daemon response.
pub async fn execute(image: String, tag: String) -> anyhow::Result<()> {
    eprintln!("Pulling {image}:{tag}…");

    let request = DaemonRequest::Pull {
        image: image.clone(),
        tag: Some(tag.clone()),
    };

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
