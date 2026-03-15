//! `minibox stop` — stop a running container.

use anyhow::Result;
use minibox_lib::protocol::{DaemonRequest, DaemonResponse};

use crate::commands::send_request;

/// Execute the `stop` subcommand.
///
/// Sends `SIGTERM` to the container process; the daemon escalates to
/// `SIGKILL` after 10 seconds if the process does not exit.
pub async fn execute(id: String) -> Result<()> {
    let request = DaemonRequest::Stop { id: id.clone() };

    match send_request(&request).await? {
        DaemonResponse::Success { message } => {
            println!("{}", message);
            Ok(())
        }
        DaemonResponse::Error { message } => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        other => {
            eprintln!("unexpected response: {:?}", other);
            std::process::exit(1);
        }
    }
}
