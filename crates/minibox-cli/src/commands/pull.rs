//! `minibox pull` — pull an image from Docker Hub.

use anyhow::Result;
use linuxbox::protocol::{DaemonRequest, DaemonResponse};

use crate::commands::send_request;

/// Execute the `pull` subcommand.
///
/// Downloads the image layers to the daemon's local image store.  Because
/// layer downloads can take time, prints a "Pulling…" indicator before
/// sending the request and waits for the (potentially slow) daemon response.
pub async fn execute(image: String, tag: String) -> Result<()> {
    eprintln!("Pulling {image}:{tag}…");

    let request = DaemonRequest::Pull {
        image: image.clone(),
        tag: Some(tag.clone()),
    };

    match send_request(&request).await? {
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
}
