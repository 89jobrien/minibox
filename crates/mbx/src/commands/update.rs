//! `mbx update` — re-pull cached images to check for newer versions.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Execute the `update` subcommand.
///
/// Validates that at least one target is specified (`images`, `--all`, or
/// `--containers`), sends `DaemonRequest::Update` to the daemon, and streams
/// `UpdateProgress` lines until a terminal `Success` or `Error` is received.
pub async fn execute(
    images: Vec<String>,
    all: bool,
    containers: bool,
    restart: bool,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    if images.is_empty() && !all && !containers {
        eprintln!("error: specify at least one image, --all, or --containers");
        std::process::exit(1);
    }

    let request = DaemonRequest::Update {
        images,
        all,
        containers,
        restart,
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    loop {
        match stream.next().await.context("stream error")? {
            Some(DaemonResponse::UpdateProgress { image, status }) => {
                println!("{image}: {status}");
            }
            Some(DaemonResponse::Success { message }) => {
                println!("{message}");
                return Ok(());
            }
            Some(DaemonResponse::Error { message }) => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            Some(other) => {
                eprintln!("unexpected response: {other:?}");
                std::process::exit(1);
            }
            None => {
                eprintln!("no response from daemon");
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::{setup, setup_multi};
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_streams_progress_then_success() {
        let (_tmp, socket_path) = setup_multi(vec![
            DaemonResponse::UpdateProgress {
                image: "alpine:latest".to_string(),
                status: "updated".to_string(),
            },
            DaemonResponse::Success {
                message: "update complete".to_string(),
            },
        ])
        .await;
        let result = execute(
            vec!["alpine:latest".to_string()],
            false,
            false,
            false,
            &socket_path,
        )
        .await;
        assert!(result.is_ok(), "execute should succeed: {result:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_with_all_flag_succeeds() {
        let (_tmp, socket_path) = setup(DaemonResponse::Success {
            message: "nothing to update".to_string(),
        })
        .await;
        let result = execute(vec![], true, false, false, &socket_path).await;
        assert!(
            result.is_ok(),
            "execute with --all should succeed: {result:?}"
        );
    }
}
