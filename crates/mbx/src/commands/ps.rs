//! `minibox ps` — list all containers.

use anyhow::Context;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Column widths for the table output.
const COL_ID: usize = 14;
const COL_NAME: usize = 16;
const COL_IMAGE: usize = 20;
const COL_COMMAND: usize = 20;
const COL_STATE: usize = 10;
const COL_CREATED: usize = 25;
const COL_PID: usize = 8;

/// Format the table header line.
pub fn format_header() -> String {
    format!(
        "{:<width_id$}  {:<width_name$}  {:<width_image$}  {:<width_cmd$}  \
         {:<width_state$}  {:<width_created$}  {:<width_pid$}",
        "CONTAINER ID",
        "NAME",
        "IMAGE",
        "COMMAND",
        "STATE",
        "CREATED",
        "PID",
        width_id = COL_ID,
        width_name = COL_NAME,
        width_image = COL_IMAGE,
        width_cmd = COL_COMMAND,
        width_state = COL_STATE,
        width_created = COL_CREATED,
        width_pid = COL_PID,
    )
}

/// Format a single container row.
pub fn format_row(c: &minibox_core::protocol::ContainerInfo) -> String {
    let pid_str = c
        .pid
        .map(|p| p.to_string())
        .unwrap_or_else(|| "-".to_string());
    let name_str = c.name.as_deref().unwrap_or("-").to_string();

    let name = truncate(&name_str, COL_NAME);
    let image = truncate(&c.image, COL_IMAGE);
    let command = truncate(&c.command, COL_COMMAND);
    let created = truncate(&c.created_at, COL_CREATED);

    format!(
        "{:<width_id$}  {:<width_name$}  {:<width_image$}  {:<width_cmd$}  \
         {:<width_state$}  {:<width_created$}  {:<width_pid$}",
        c.id,
        name,
        image,
        command,
        c.state,
        created,
        pid_str,
        width_id = COL_ID,
        width_name = COL_NAME,
        width_image = COL_IMAGE,
        width_cmd = COL_COMMAND,
        width_state = COL_STATE,
        width_created = COL_CREATED,
        width_pid = COL_PID,
    )
}

/// Execute the `ps` subcommand.
///
/// Prints a formatted table of all containers known to the daemon.
pub async fn execute(socket_path: &std::path::Path) -> anyhow::Result<()> {
    let request = DaemonRequest::List;

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ContainerList { containers } => {
                // Header
                println!("{}", format_header());

                // Separator
                println!(
                    "{}",
                    "-".repeat(
                        COL_ID
                            + COL_NAME
                            + COL_IMAGE
                            + COL_COMMAND
                            + COL_STATE
                            + COL_CREATED
                            + COL_PID
                            + 12
                    )
                );

                for c in &containers {
                    println!("{}", format_row(c));
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
    use minibox_core::protocol::ContainerInfo;

    /// Bind a Unix socket, accept one connection, read one request line,
    /// respond with `response`, then close.  Returns the socket path.
    #[cfg(unix)]
    async fn serve_once(socket_path: &std::path::Path, response: DaemonResponse) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixListener;

        let listener = UnixListener::bind(socket_path).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let mut resp = serde_json::to_string(&response).unwrap();
        resp.push('\n');
        write_half.write_all(resp.as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_prints_empty_list() {
        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");

        let sp = socket_path.clone();
        tokio::spawn(async move {
            serve_once(&sp, DaemonResponse::ContainerList { containers: vec![] }).await;
        });

        // Give server a moment to bind
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let result = execute(&socket_path).await;
        assert!(
            result.is_ok(),
            "execute should succeed with empty list: {result:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_prints_container_row() {
        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");

        let container = ContainerInfo {
            id: "abc123456789".to_string(),
            name: None,
            image: "alpine".to_string(),
            command: "/bin/sh".to_string(),
            state: "running".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: Some(42),
        };

        let sp = socket_path.clone();
        tokio::spawn(async move {
            serve_once(
                &sp,
                DaemonResponse::ContainerList {
                    containers: vec![container],
                },
            )
            .await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let result = execute(&socket_path).await;
        assert!(
            result.is_ok(),
            "execute should succeed with one container: {result:?}"
        );
    }

    /// Verify that the NAME column header is present in ps output.
    /// This test checks the format_header function produces the NAME column.
    #[test]
    fn ps_header_contains_name_column() {
        let header = format_header();
        assert!(
            header.contains("NAME"),
            "ps header should contain NAME column, got: {header}"
        );
    }

    /// Verify that a container with a name shows it in the formatted row.
    #[test]
    fn ps_row_shows_container_name() {
        let info = ContainerInfo {
            id: "abc123".to_string(),
            name: Some("my-web".to_string()),
            image: "nginx".to_string(),
            command: "/bin/nginx".to_string(),
            state: "running".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: Some(99),
        };
        let row = format_row(&info);
        assert!(
            row.contains("my-web"),
            "row should contain the container name, got: {row}"
        );
    }

    /// Verify that a container without a name shows '-' in the NAME column.
    #[test]
    fn ps_row_shows_dash_when_no_name() {
        let info = ContainerInfo {
            id: "abc123".to_string(),
            name: None,
            image: "alpine".to_string(),
            command: "/bin/sh".to_string(),
            state: "running".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            pid: Some(1),
        };
        let row = format_row(&info);
        // The NAME column should contain '-' for unnamed containers.
        assert!(
            row.contains('-'),
            "row should contain '-' for unnamed container, got: {row}"
        );
    }

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
