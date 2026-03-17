//! CLI command modules.

pub mod ps;
pub mod pull;
pub mod rm;
pub mod run;
pub mod stop;

use anyhow::{Context, Result};
use minibox_lib::protocol::{DaemonRequest, DaemonResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;
use tracing::debug;

/// Default Unix socket path of the running daemon.
const DEFAULT_SOCKET_PATH: &str = "/run/minibox/miniboxd.sock";

/// Resolve the daemon socket path.
///
/// Checks `MINIBOX_SOCKET_PATH` env var first, falls back to default.
fn socket_path() -> String {
    std::env::var("MINIBOX_SOCKET_PATH").unwrap_or_else(|_| DEFAULT_SOCKET_PATH.to_string())
}

/// Open a connection to the daemon, send one request, and return the response.
///
/// The protocol is a single JSON line → single JSON line.
pub async fn send_request(request: &DaemonRequest) -> Result<DaemonResponse> {
    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .await
        .with_context(|| format!("connecting to daemon at {}", path))?;

    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);

    // Serialise request as a single JSON line.
    let mut payload = serde_json::to_string(request).context("serialising request")?;
    payload.push('\n');

    debug!("sending: {}", payload.trim());

    writer
        .write_all(payload.as_bytes())
        .await
        .context("writing request")?;
    writer.flush().await.context("flushing request")?;

    // Read one response line.
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .context("reading response")?;

    debug!("received: {}", line.trim());

    let response: DaemonResponse = serde_json::from_str(line.trim()).context("parsing response")?;
    Ok(response)
}
