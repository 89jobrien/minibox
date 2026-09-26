//! `mbx images` — list cached images.
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

use super::RequestError;
use anyhow::Context as _;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::path::Path;

/// Column width for the single-column image table.
const COL_IMAGE: usize = 40;

/// Format the table header line.
#[must_use]
pub fn format_header() -> String {
    format!("{:<COL_IMAGE$}", "IMAGE")
}

/// Format a single image row.
///
/// Image references are never truncated — a truncated reference cannot be fed
/// back to `mbx run`/`mbx push`/`mbx rmi`, so overflow is allowed instead.
#[must_use]
pub fn format_row(image_ref: &str) -> String {
    format!("{image_ref:<COL_IMAGE$}")
}

/// Execute the `images` subcommand.
///
/// Sends `DaemonRequest::ListImages` to the daemon and prints the returned
/// references as a single-column table, following the same header/separator
/// layout as `mbx ps`.
pub async fn execute(socket_path: &Path) -> anyhow::Result<()> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::ListImages)
        .await
        .context("failed to call daemon")?;

    let Some(response) = stream.next().await.context("stream error")? else {
        return Err(RequestError::NoResponse.into());
    };

    match response {
        DaemonResponse::ImageList { images } => {
            println!("{}", format_header());
            println!("{}", "-".repeat(COL_IMAGE));
            if images.is_empty() {
                println!("(no images)");
            }
            for image_ref in &images {
                println!("{}", format_row(image_ref));
            }
            Ok(())
        }
        DaemonResponse::Error { message } => Err(RequestError::DaemonError { message }.into()),
        other => Err(RequestError::UnexpectedResponse {
            response: format!("{other:?}"),
        }
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::setup;
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    #[test]
    fn format_header_contains_image_column() {
        let header = format_header();
        assert!(
            header.contains("IMAGE"),
            "images header should contain IMAGE column, got: {header:?}"
        );
    }

    #[test]
    fn format_row_returns_image_ref() {
        let row = format_row("alpine:latest");
        assert_eq!(row.trim(), "alpine:latest");
    }

    #[test]
    fn format_row_for_multiple_refs_preserves_each_on_its_own_line() {
        let rows: Vec<String> = ["alpine:latest", "nginx:stable"]
            .iter()
            .map(|r| format_row(r))
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].trim(), "alpine:latest");
        assert_eq!(rows[1].trim(), "nginx:stable");
    }

    #[test]
    fn format_row_does_not_truncate_long_refs() {
        let long = "registry.example.com/team/subproject/service-component:v1.2.3-rc1";
        let row = format_row(long);
        assert!(
            row.contains(long),
            "long image refs must survive formatting intact, got: {row:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_sends_list_images_request_and_prints_refs() {
        let temp_dir = tempfile::TempDir::new().expect("create temp dir");
        let socket_path = temp_dir.path().join("images.sock");
        let server_socket = socket_path.clone();
        let server = tokio::spawn(async move {
            let listener = UnixListener::bind(server_socket).expect("bind test socket");
            let (stream, _) = listener.accept().await.expect("accept connection");
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut reader = BufReader::new(read_half);
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read request");
            let mut encoded = serde_json::to_string(&DaemonResponse::ImageList {
                images: vec!["alpine:latest".to_string(), "nginx:stable".to_string()],
            })
            .expect("encode response");
            encoded.push('\n');
            write_half
                .write_all(encoded.as_bytes())
                .await
                .expect("write response");
            write_half.flush().await.expect("flush response");
            serde_json::from_str::<DaemonRequest>(&line).expect("decode request")
        });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let result = execute(&socket_path).await;
        assert!(result.is_ok(), "images should succeed: {result:?}");

        let request = server.await.expect("server task");
        assert!(
            matches!(request, DaemonRequest::ListImages),
            "expected ListImages request, got: {request:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_succeeds_on_empty_image_list() {
        let (_tmp, socket_path) = setup(DaemonResponse::ImageList { images: vec![] }).await;
        let result = execute(&socket_path).await;
        assert!(
            result.is_ok(),
            "empty image list should succeed: {result:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_propagates_daemon_error() {
        let (_tmp, socket_path) = setup(DaemonResponse::Error {
            message: "image store unavailable".to_string(),
        })
        .await;
        let result = execute(&socket_path).await;
        let error = result.expect_err("daemon error should propagate");
        assert!(
            error.to_string().contains("image store unavailable"),
            "error should carry daemon message, got: {error:#}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_rejects_unexpected_response() {
        let (_tmp, socket_path) = setup(DaemonResponse::Success {
            message: "unexpected".to_string(),
        })
        .await;
        let result = execute(&socket_path).await;
        let error = result.expect_err("unexpected response should fail");
        assert!(
            error.to_string().contains("unexpected response"),
            "unexpected error: {error:#}"
        );
    }
}
