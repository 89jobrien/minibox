//! `minibox load` — load an image from a local OCI tar archive.

use anyhow::Context;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Execute the `load` subcommand.
///
/// Sends a `LoadImage` request to the daemon with the given tar path, name,
/// and tag.  Prints `"Loaded image: <image>"` on success.
pub async fn execute(
    path: String,
    name: String,
    tag: String,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    let request = DaemonRequest::LoadImage {
        path: path.clone(),
        name: name.clone(),
        tag: tag.clone(),
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ImageLoaded { image } => {
                println!("Loaded image: {image}");
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

/// Derive an image name from a file path by stripping the directory and
/// known archive extensions (`.tar`, `.tar.gz`, `.tgz`).
pub fn name_from_path(path: &str) -> String {
    let stem = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path);

    // Strip known archive extensions.
    for ext in &[".tar.gz", ".tgz", ".tar"] {
        if let Some(stripped) = stem.strip_suffix(ext) {
            return stripped.to_string();
        }
    }
    stem.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_from_path_strips_tar_gz() {
        assert_eq!(name_from_path("/tmp/myimage.tar.gz"), "myimage");
    }

    #[test]
    fn name_from_path_strips_tgz() {
        assert_eq!(name_from_path("myimage.tgz"), "myimage");
    }

    #[test]
    fn name_from_path_strips_tar() {
        assert_eq!(name_from_path("myimage.tar"), "myimage");
    }

    #[test]
    fn name_from_path_no_extension() {
        assert_eq!(name_from_path("myimage"), "myimage");
    }

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
    async fn execute_succeeds_on_image_loaded_response() {
        let tmp = tempfile::TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let sp = socket_path.clone();
        tokio::spawn(async move {
            serve_once(
                &sp,
                DaemonResponse::ImageLoaded {
                    image: "myimage:latest".to_string(),
                },
            )
            .await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let result = execute(
            "/tmp/myimage.tar".to_string(),
            "myimage".to_string(),
            "latest".to_string(),
            &socket_path,
        )
        .await;
        assert!(result.is_ok(), "execute should succeed: {result:?}");
    }
}
