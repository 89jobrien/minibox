//! `minibox stop` — stop a running container.

use anyhow::Result;
use minibox_lib::protocol::{DaemonRequest, DaemonResponse};

use crate::commands::send_request;

/// Execute the `stop` subcommand.
///
/// Sends a `Stop` request to the daemon, which is responsible for signalling
/// the container process.  Prints the daemon's confirmation message on success
/// or an error description on failure.
pub async fn execute(id: String) -> Result<()> {
    let request = DaemonRequest::Stop { id: id.clone() };

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
