use crate::error::{ClientError, Result};
use minibox_core::protocol::{DaemonRequest, DaemonResponse, decode_response};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub struct DaemonClient {
    socket_path: std::path::PathBuf,
}

impl DaemonClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            socket_path: crate::default_socket_path(),
        })
    }

    pub fn with_socket(path: impl AsRef<Path>) -> Self {
        Self {
            socket_path: path.as_ref().to_path_buf(),
        }
    }

    pub async fn call(&self, request: DaemonRequest) -> Result<DaemonResponseStream> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(ClientError::ConnectionFailed)?;

        // Send request
        let payload = serde_json::to_string(&request)?;
        stream
            .write_all(format!("{}\n", payload).as_bytes())
            .await
            .map_err(ClientError::ConnectionFailed)?;
        stream
            .flush()
            .await
            .map_err(ClientError::ConnectionFailed)?;

        Ok(DaemonResponseStream {
            stream: BufReader::new(stream),
        })
    }
}

pub struct DaemonResponseStream {
    stream: BufReader<UnixStream>,
}

impl DaemonResponseStream {
    pub async fn next(&mut self) -> Result<Option<DaemonResponse>> {
        let mut line = String::new();
        let n = self
            .stream
            .read_line(&mut line)
            .await
            .map_err(ClientError::ConnectionFailed)?;

        if n == 0 {
            return Ok(None);
        }

        let response =
            decode_response(line.as_bytes()).map_err(|e| ClientError::FrameError(e.to_string()))?;

        Ok(Some(response))
    }

    pub async fn try_collect(mut self) -> Result<Vec<DaemonResponse>> {
        let mut responses = Vec::new();
        while let Some(resp) = self.next().await? {
            responses.push(resp);
        }
        Ok(responses)
    }
}

/// A write-only connection that sends a single [`DaemonRequest`] and discards the response.
///
/// Used for fire-and-forget fire-and-forget messages like `SendInput` and `ResizePty` where the
/// caller does not need to inspect the daemon's `Success`/`Error` reply.
pub struct DaemonWriter {
    socket_path: std::path::PathBuf,
}

impl DaemonWriter {
    pub fn with_socket(path: impl AsRef<Path>) -> Self {
        Self {
            socket_path: path.as_ref().to_path_buf(),
        }
    }

    /// Send `request` to the daemon and return immediately without reading the response.
    pub async fn send(&self, request: DaemonRequest) -> Result<()> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(ClientError::ConnectionFailed)?;
        let payload = serde_json::to_string(&request)?;
        stream
            .write_all(format!("{}\n", payload).as_bytes())
            .await
            .map_err(ClientError::ConnectionFailed)?;
        stream
            .flush()
            .await
            .map_err(ClientError::ConnectionFailed)?;
        Ok(())
    }
}

impl Default for DaemonClient {
    fn default() -> Self {
        Self::new().expect("failed to create default client")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = DaemonClient::new();
        assert!(client.is_ok());
    }

    #[test]
    fn test_client_with_socket() {
        let client = DaemonClient::with_socket("/tmp/test.sock");
        assert_eq!(
            client.socket_path,
            std::path::PathBuf::from("/tmp/test.sock")
        );
    }

    #[test]
    fn test_client_default() {
        let client = DaemonClient::default();
        assert!(client.socket_path.as_os_str().len() > 0);
    }
}
