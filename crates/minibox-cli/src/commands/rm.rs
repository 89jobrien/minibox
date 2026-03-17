//! `minibox rm` — remove a stopped container.

use anyhow::Result;
use minibox_lib::protocol::{DaemonRequest, DaemonResponse};

use crate::commands::send_request;

/// Execute the `rm` subcommand.
///
/// The container must already be in the `Stopped` state; use `minibox stop`
/// first if it is still running.
pub async fn execute(id: String) -> Result<()> {
    let request = DaemonRequest::Remove { id: id.clone() };

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
