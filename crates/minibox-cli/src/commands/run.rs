//! `minibox run` — create and start a container.

use anyhow::Result;
use minibox_lib::protocol::{DaemonRequest, DaemonResponse};

use crate::commands::send_request;

/// Execute the `run` subcommand.
///
/// Sends a `DaemonRequest::Run` to the daemon and prints the new container ID
/// on success.
pub async fn execute(
    image: String,
    tag: String,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
) -> Result<()> {
    let request = DaemonRequest::Run {
        image: image.clone(),
        tag: Some(tag.clone()),
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral: false,
    };

    match send_request(&request).await? {
        DaemonResponse::ContainerCreated { id } => {
            println!("{id}");
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
