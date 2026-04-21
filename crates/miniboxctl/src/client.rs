//! Thin daemon client for miniboxctl.
//!
//! Connects to the miniboxd Unix socket and exchanges JSON-over-newline
//! protocol messages.  This is a minimal client embedded directly in miniboxctl
//! rather than depending on a separate `minibox-client` crate.

use anyhow::{Context, Result};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;

/// A connection to the minibox daemon over a Unix socket.
pub struct DaemonClient {
    socket_path: String,
}

impl DaemonClient {
    /// Create a new client targeting the given socket path.
    pub fn new(socket_path: String) -> Self {
        Self { socket_path }
    }

    /// Resolve socket path from environment or platform default.
    pub fn from_env() -> Self {
        let path = std::env::var("MINIBOX_SOCKET_PATH").unwrap_or_else(|_| {
            #[cfg(target_os = "macos")]
            {
                "/tmp/minibox/miniboxd.sock".to_string()
            }
            #[cfg(not(target_os = "macos"))]
            {
                "/run/minibox/miniboxd.sock".to_string()
            }
        });
        Self::new(path)
    }

    /// Open a streaming connection to the daemon.
    ///
    /// Sends the request and returns a [`ResponseStream`] that yields
    /// responses one at a time (for ephemeral/streaming runs, multiple
    /// responses arrive on the same connection).
    pub async fn call(&self, request: DaemonRequest) -> Result<ResponseStream> {
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .with_context(|| format!("connecting to daemon at {}", self.socket_path))?;

        let (read_half, write_half) = stream.into_split();
        let mut writer = BufWriter::new(write_half);

        let mut payload = serde_json::to_string(&request).context("serialising request")?;
        payload.push('\n');

        writer
            .write_all(payload.as_bytes())
            .await
            .context("writing request")?;
        writer.flush().await.context("flushing request")?;

        Ok(ResponseStream {
            reader: BufReader::new(read_half),
            _writer: writer,
        })
    }
}

/// An iterator-like handle that reads daemon responses from the socket.
pub struct ResponseStream {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    /// Keep the write half alive so the socket isn't half-closed.
    _writer: BufWriter<tokio::net::unix::OwnedWriteHalf>,
}

impl ResponseStream {
    /// Read the next response from the daemon, or `None` if the connection
    /// closed.
    pub async fn next(&mut self) -> Result<Option<DaemonResponse>> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .await
            .context("reading response")?;
        if n == 0 {
            return Ok(None);
        }
        let resp: DaemonResponse = serde_json::from_str(line.trim()).context("parsing response")?;
        Ok(Some(resp))
    }
}
