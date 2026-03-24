//! `minibox ps` — list all containers.

use anyhow::Context;
use linuxbox::protocol::{DaemonRequest, DaemonResponse};
use minibox_client::DaemonClient;

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
pub async fn execute() -> anyhow::Result<()> {
    let request = DaemonRequest::List;

    let client = DaemonClient::new().context("failed to create daemon client")?;
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
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
    } else {
        eprintln!("no response from daemon");
        std::process::exit(1);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_empty_string_unchanged() {
        assert_eq!(truncate("", 10), "");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_exact_length_unchanged() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_one_over_limit_adds_ellipsis() {
        // 6 chars, max 5 → take 4 + "…"
        assert_eq!(truncate("hello!", 5), "hell…");
    }

    #[test]
    fn truncate_long_string_adds_ellipsis() {
        assert_eq!(truncate("hello world", 8), "hello w…");
    }

    #[test]
    fn truncate_max_one_produces_single_ellipsis() {
        // max=1, saturating_sub(1)=0 → take 0 chars, push "…"
        assert_eq!(truncate("ab", 1), "…");
    }

    #[test]
    fn truncate_counts_unicode_chars_not_bytes() {
        // "café" is 4 chars but 5 bytes; should be treated as 4 chars
        assert_eq!(truncate("café", 4), "café");
        assert_eq!(truncate("café!", 4), "caf…");
    }
}
