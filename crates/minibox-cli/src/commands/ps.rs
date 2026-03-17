//! `minibox ps` — list all containers.

use anyhow::Result;
use minibox_lib::protocol::{DaemonRequest, DaemonResponse};

use crate::commands::send_request;

/// Column widths for the table output.
const COL_ID: usize = 14;
const COL_IMAGE: usize = 20;
const COL_COMMAND: usize = 20;
const COL_STATE: usize = 10;
const COL_CREATED: usize = 25;
const COL_PID: usize = 8;

/// Execute the `ps` subcommand.
///
/// Prints a formatted table of all containers known to the daemon.
pub async fn execute() -> Result<()> {
    let request = DaemonRequest::List;

    match send_request(&request).await? {
        DaemonResponse::ContainerList { containers } => {
            // Header
            println!(
                "{:<width_id$}  {:<width_image$}  {:<width_cmd$}  {:<width_state$}  {:<width_created$}  {:<width_pid$}",
                "CONTAINER ID",
                "IMAGE",
                "COMMAND",
                "STATE",
                "CREATED",
                "PID",
                width_id = COL_ID,
                width_image = COL_IMAGE,
                width_cmd = COL_COMMAND,
                width_state = COL_STATE,
                width_created = COL_CREATED,
                width_pid = COL_PID,
            );

            // Separator
            println!(
                "{}",
                "-".repeat(
                    COL_ID + COL_IMAGE + COL_COMMAND + COL_STATE + COL_CREATED + COL_PID + 10
                )
            );

            for c in &containers {
                let pid_str = c
                    .pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string());

                // Truncate long fields to keep the table tidy.
                let image = truncate(&c.image, COL_IMAGE);
                let command = truncate(&c.command, COL_COMMAND);
                let created = truncate(&c.created_at, COL_CREATED);

                println!(
                    "{:<width_id$}  {:<width_image$}  {:<width_cmd$}  {:<width_state$}  {:<width_created$}  {:<width_pid$}",
                    c.id,
                    image,
                    command,
                    c.state,
                    created,
                    pid_str,
                    width_id = COL_ID,
                    width_image = COL_IMAGE,
                    width_cmd = COL_COMMAND,
                    width_state = COL_STATE,
                    width_created = COL_CREATED,
                    width_pid = COL_PID,
                );
            }

            if containers.is_empty() {
                println!("(no containers)");
            }

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

/// Return a string slice of at most `max` characters, appending "…" if
/// truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
