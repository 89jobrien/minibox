//! `minibox pull` — pull an image from Docker Hub.

use anyhow::Context;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Execute the `pull` subcommand.
///
/// Downloads the image layers to the daemon's local image store.  Because
/// layer downloads can take time, prints a "Pulling…" indicator before
/// sending the request and waits for the (potentially slow) daemon response.
pub async fn execute(
    image: String,
    tag: String,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    eprintln!("Pulling {image}:{tag}…");

    let request = DaemonRequest::Pull {
        image: image.clone(),
        tag: Some(tag.clone()),
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
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
    } else {
        eprintln!("no response from daemon");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    async fn execute_succeeds_on_success_response() {
        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let sp = socket_path.clone();
        tokio::spawn(async move {
            serve_once(
                &sp,
                DaemonResponse::Success {
                    message: "pulled".to_string(),
                },
            )
            .await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let result = execute("alpine".to_string(), "latest".to_string(), &socket_path).await;
        assert!(result.is_ok(), "execute should succeed: {result:?}");
    }
}
