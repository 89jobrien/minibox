//! `mbx monitor` — live status dashboard for daemon and containers.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::doc_markdown,
        clippy::unwrap_in_result
    )
)]

use anyhow::Context as _;
use minibox_core::client::DaemonClient;
use minibox_core::domain::CapabilityMatrix;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// Refresh interval for the monitor display.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// Column widths for the container table output.
const COL_ID: usize = 14;
const COL_NAME: usize = 16;
const COL_IMAGE: usize = 20;
const COL_STATE: usize = 10;
const COL_PID: usize = 8;
const COL_UPTIME: usize = 12;

/// Render a clear-screen escape sequence.
const fn clear_screen() -> &'static str {
    "\x1B[2J\x1B[H"
}

/// Format the header section with daemon status.
fn format_header(
    is_running: bool,
    adapter_name: &str,
    matrix: &Option<CapabilityMatrix>,
) -> String {
    use std::fmt::Write;

    let status = if is_running { "running" } else { "offline" };
    let status_color = if is_running { "\x1B[32m" } else { "\x1B[31m" };
    let reset = "\x1B[0m";

    let mut output = String::new();
    let _ = writeln!(
        output,
        "minibox monitor — daemon {status_color}{status}{reset}"
    );
    let _ = writeln!(output, "  adapter:    {adapter_name}");

    if let Some(m) = matrix {
        let backends = m
            .backends
            .iter()
            .map(|b| b.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(output, "  backends:   {backends}");
    }

    let _ = writeln!(output);
    output
}

/// Format the container table header.
fn format_container_header() -> String {
    format!(
        "{:<width_id$}  {:<width_name$}  {:<width_image$}  \
         {:<width_state$}  {:<width_pid$}  {:<width_uptime$}",
        "CONTAINER ID",
        "NAME",
        "IMAGE",
        "STATE",
        "PID",
        "UPTIME",
        width_id = COL_ID,
        width_name = COL_NAME,
        width_image = COL_IMAGE,
        width_state = COL_STATE,
        width_pid = COL_PID,
        width_uptime = COL_UPTIME,
    )
}

/// Truncate a string to fit within a column width.
fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() > width {
        let truncated: String = s.chars().take(width).collect();
        format!("{truncated}…")
    } else {
        s.to_string()
    }
}

/// Format a single container row.
fn format_container_row(c: &minibox_core::protocol::ContainerInfo) -> String {
    let pid_str = c.pid.map_or_else(|| "-".to_string(), |p| p.to_string());
    let name_str = c.name.as_deref().unwrap_or("-").to_string();

    // Compute uptime based on created_at string (ISO 8601 format).
    // This is approximate; for a real implementation we'd parse the timestamp.
    let uptime = if c.state == "running" {
        // Placeholder: would parse created_at to compute actual uptime
        "N/A".to_string()
    } else {
        "-".to_string()
    };

    let name = truncate(&name_str, COL_NAME);
    let image = truncate(&c.image, COL_IMAGE);

    format!(
        "{:<width_id$}  {:<width_name$}  {:<width_image$}  \
         {:<width_state$}  {:<width_pid$}  {:<width_uptime$}",
        c.id,
        name,
        image,
        c.state,
        pid_str,
        uptime,
        width_id = COL_ID,
        width_name = COL_NAME,
        width_image = COL_IMAGE,
        width_state = COL_STATE,
        width_pid = COL_PID,
        width_uptime = COL_UPTIME,
    )
}

/// Fetch daemon status and capabilities.
async fn fetch_daemon_status(socket_path: &Path) -> (bool, String, Option<CapabilityMatrix>) {
    // Try to get capabilities
    let client = DaemonClient::with_socket(socket_path);
    match client.call(DaemonRequest::GetCapabilities).await {
        Ok(mut stream) => match stream.next().await {
            Ok(Some(DaemonResponse::CapabilityMatrix { matrix })) => {
                let adapter = matrix
                    .backends
                    .first()
                    .map_or("unknown", |b| b.as_str())
                    .to_string();
                (true, adapter, Some(matrix))
            }
            _ => (true, "unknown".to_string(), None),
        },
        Err(_) => (false, "unknown".to_string(), None),
    }
}

/// Fetch the current container list.
async fn fetch_containers(
    socket_path: &Path,
) -> anyhow::Result<Vec<minibox_core::protocol::ContainerInfo>> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::List)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ContainerList { containers } => Ok(containers),
            _ => Ok(Vec::new()),
        }
    } else {
        Ok(Vec::new())
    }
}

/// Render a single update of the monitor display.
async fn render_update(socket_path: &Path) -> anyhow::Result<()> {
    // Fetch daemon status before acquiring stdout lock to avoid !Send issues across await
    let (is_running, adapter, matrix) = fetch_daemon_status(socket_path).await;

    // Fetch containers before acquiring stdout lock
    let containers = if is_running {
        fetch_containers(socket_path).await.ok()
    } else {
        None
    };

    // Now we can safely lock stdout and render everything
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    // Clear screen
    write!(out, "{}", clear_screen())?;

    // Print header
    write!(out, "{}", format_header(is_running, &adapter, &matrix))?;

    if let Some(containers) = containers {
        if containers.is_empty() {
            writeln!(out, "No containers running")?;
        } else {
            writeln!(out, "Containers:")?;
            writeln!(out, "{}", format_container_header())?;
            writeln!(
                out,
                "{}",
                "-".repeat(COL_ID + COL_NAME + COL_IMAGE + COL_STATE + COL_PID + COL_UPTIME + 30)
            )?;
            for container in containers {
                writeln!(out, "{}", format_container_row(&container))?;
            }
        }
    } else if is_running {
        writeln!(out, "error fetching containers")?;
    } else {
        writeln!(out, "daemon is not responding")?;
    }

    // Footer with timestamp (using system time without external crate)
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs() % 86400; // seconds in a day
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs_part = secs % 60;

    writeln!(out)?;
    writeln!(
        out,
        "last updated: {hours:02}:{mins:02}:{secs_part:02} | refresh every 2s"
    )?;
    writeln!(out, "press Ctrl+C to exit")?;

    out.flush()?;
    Ok(())
}

/// Execute the `monitor` subcommand.
///
/// Continuously displays daemon and container status, refreshing every 2 seconds.
/// Runs until interrupted (Ctrl+C).
pub async fn execute(socket_path: &Path) -> anyhow::Result<()> {
    // Set up signal handler for graceful shutdown
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("failed to set up signal handler")?;

    loop {
        render_update(socket_path).await?;

        tokio::select! {
            Some(()) = sigint.recv() => {
                // Clear screen before exiting for cleanliness
                println!("{}", clear_screen());
                break;
            }
            () = tokio::time::sleep(REFRESH_INTERVAL) => {
                // Continue to next iteration
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_header_shows_running_status() {
        let header = format_header(true, "smolvm", &None);
        assert!(header.contains("running"));
        assert!(header.contains("smolvm"));
    }

    #[test]
    fn format_header_shows_offline_status() {
        let header = format_header(false, "smolvm", &None);
        assert!(header.contains("offline"));
    }

    #[test]
    fn format_container_header_includes_all_columns() {
        let header = format_container_header();
        assert!(header.contains("CONTAINER ID"));
        assert!(header.contains("IMAGE"));
        assert!(header.contains("STATE"));
        assert!(header.contains("PID"));
    }

    #[test]
    fn truncate_respects_width() {
        let long = "this-is-a-very-long-container-name-that-exceeds-width";
        let truncated = truncate(long, 10);
        assert!(truncated.chars().count() <= 11); // width + ellipsis
    }

    #[test]
    fn truncate_preserves_short_strings() {
        let short = "short";
        let truncated = truncate(short, 10);
        assert_eq!(truncated, "short");
    }
}
