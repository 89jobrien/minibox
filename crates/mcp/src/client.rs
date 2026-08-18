//! Typed minibox daemon client adapter for MCP tools.

use crate::error::{McpServerError, Result};
use crate::policy::DEFAULT_MAX_OUTPUT_BYTES;
use minibox_core::client::{ClientError, DaemonClient, default_socket_path};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use serde_json::Value;
use std::path::PathBuf;

/// Thin adapter over [`DaemonClient`] with terminal-aware response collection.
#[derive(Clone, Debug)]
pub struct MiniboxDaemonClient {
    /// Unix socket path used to connect to `miniboxd`.
    pub socket_path: PathBuf,
}

impl MiniboxDaemonClient {
    /// Create a daemon client for an explicit socket path.
    #[must_use]
    pub const fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Create a daemon client using minibox's existing socket environment rules.
    #[must_use]
    pub fn from_env() -> Self {
        Self::new(default_socket_path())
    }

    /// Send a request and collect daemon responses using a default output limit.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket call, response decoding, daemon handling,
    /// or output accounting fails.
    pub async fn call(&self, request: DaemonRequest) -> Result<DaemonCallResult> {
        self.call_limited(request, DEFAULT_MAX_OUTPUT_BYTES).await
    }

    /// Send a request and collect daemon responses until a terminal response or stream close.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket call, response decoding, daemon handling,
    /// or output accounting fails.
    pub async fn call_limited(
        &self,
        request: DaemonRequest,
        max_output_bytes: usize,
    ) -> Result<DaemonCallResult> {
        let client = DaemonClient::with_socket(&self.socket_path);
        let mut stream = client.call(request).await.map_err(map_client_error)?;

        let mut responses = Vec::new();
        let mut raw_responses = Vec::new();
        let mut total_bytes = 0usize;
        let mut terminal_type = None;

        while let Some(response) = stream.next().await.map_err(map_client_error)? {
            let is_terminal = response.is_terminal();
            let response_type = response_type(&response);
            let raw = serde_json::to_value(&response)?;
            total_bytes = total_bytes.saturating_add(raw.to_string().len());
            // TODO(review): this hard-errors on overflow, but normalize_run_output()
            // (containers.rs) has its own graceful truncation path with a `truncated` flag.
            // The hard error here fires first, so large-output runs fail instead of
            // returning truncated stdout/stderr. Reconcile the two strategies.
            if total_bytes > max_output_bytes {
                return Err(McpServerError::OutputLimitExceeded);
            }
            if let DaemonResponse::Error { message } = &response {
                return Err(McpServerError::Daemon(message.clone()));
            }
            responses.push(response);
            raw_responses.push(raw);
            if is_terminal {
                terminal_type = Some(response_type);
                break;
            }
        }

        Ok(DaemonCallResult {
            responses,
            raw_responses,
            terminal_type,
        })
    }
}

/// Responses returned by a daemon call.
#[derive(Debug, Clone)]
pub struct DaemonCallResult {
    /// Typed daemon responses in order.
    pub responses: Vec<DaemonResponse>,
    /// JSON daemon responses in order, suitable for MCP structured output.
    pub raw_responses: Vec<Value>,
    /// Terminal response type, if collection stopped on a terminal response.
    pub terminal_type: Option<String>,
}

fn map_client_error(error: ClientError) -> McpServerError {
    match error {
        ClientError::ConnectionFailed(e) => McpServerError::DaemonConnection(e.to_string()),
        ClientError::DaemonError(message) => McpServerError::Daemon(message),
        ClientError::FrameError(message) => McpServerError::ProtocolError(message),
        ClientError::SocketPathNotFound => {
            McpServerError::DaemonConnection(ClientError::SocketPathNotFound.to_string())
        }
        ClientError::JsonError(e) => McpServerError::Json(e),
    }
}

/// Return the daemon response variant name.
///
/// Falls back to `"Unknown"` with a warning when the response cannot be
/// serialized or carries no `type` tag — either indicates protocol drift
/// between this crate and `minibox-core::protocol::DaemonResponse`.
#[must_use]
pub fn response_type(response: &DaemonResponse) -> String {
    serde_json::to_value(response)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .map(std::string::ToString::to_string)
        })
        .unwrap_or_else(|| {
            tracing::warn!(
                response = ?response,
                "client: daemon response missing type tag; possible protocol drift"
            );
            "Unknown".to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_type_reads_type_tag() {
        let response = DaemonResponse::Success {
            message: "ok".to_string(),
        };

        assert_eq!(response_type(&response), "Success");
    }
}
