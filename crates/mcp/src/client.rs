//! Typed minibox daemon client adapter for MCP tools.

use crate::error::{McpServerError, Result};
use crate::policy::AuthorizedDaemonCall;
use base64::Engine as _;
use minibox_core::client::{ClientError, DaemonClient, default_socket_path};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use serde_json::Value;
use std::path::PathBuf;

/// Thin adapter over [`DaemonClient`] with terminal-aware response collection.
///
/// Daemon calls cannot be made from an unvalidated request:
///
/// ```compile_fail
/// use mcp::client::MiniboxDaemonClient;
/// use minibox_core::protocol::DaemonRequest;
///
/// # async fn omitted_policy(client: &MiniboxDaemonClient) {
/// let _ = client.call(DaemonRequest::List).await;
/// # }
/// ```
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

    /// Send a policy-authorized request and collect daemon responses.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket call, response decoding, daemon handling,
    /// or output accounting fails.
    pub async fn call(&self, authorized: AuthorizedDaemonCall) -> Result<DaemonCallResult> {
        let (request, max_output_bytes) = authorized.into_parts();
        let gracefully_truncate_output = matches!(request, DaemonRequest::Run { .. });
        let client = DaemonClient::with_socket(&self.socket_path);
        let mut stream = client.call(request).await.map_err(map_client_error)?;

        let mut responses = Vec::new();
        let mut raw_responses = Vec::new();
        let mut total_bytes = 0usize;
        let mut retained_output_bytes = 0usize;
        let mut output_truncated = false;
        let mut terminal_type = None;

        while let Some(mut response) = stream.next().await.map_err(map_client_error)? {
            let is_terminal = response.is_terminal();
            let response_type = response_type(&response);
            if let DaemonResponse::Error { message } = &response {
                return Err(McpServerError::Daemon(message.clone()));
            }

            if gracefully_truncate_output
                && truncate_container_output(
                    &mut response,
                    max_output_bytes,
                    &mut retained_output_bytes,
                    &mut output_truncated,
                )?
            {
                continue;
            }

            let raw = serde_json::to_value(&response)?;
            if !gracefully_truncate_output {
                total_bytes = total_bytes.saturating_add(raw.to_string().len());
                if total_bytes > max_output_bytes {
                    return Err(McpServerError::OutputLimitExceeded);
                }
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
            output_truncated,
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
    /// Whether streamed container output was truncated while collecting responses.
    pub output_truncated: bool,
}

fn truncate_container_output(
    response: &mut DaemonResponse,
    max_output_bytes: usize,
    retained_output_bytes: &mut usize,
    output_truncated: &mut bool,
) -> Result<bool> {
    let DaemonResponse::ContainerOutput { data, .. } = response else {
        return Ok(false);
    };
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|error| McpServerError::ProtocolError(error.to_string()))?;
    let remaining = max_output_bytes.saturating_sub(*retained_output_bytes);
    let retained = decoded.len().min(remaining);
    *retained_output_bytes = retained_output_bytes.saturating_add(retained);
    if retained < decoded.len() {
        *output_truncated = true;
    }
    if retained == 0 {
        return Ok(true);
    }
    if retained < decoded.len() {
        *data = base64::engine::general_purpose::STANDARD.encode(&decoded[..retained]);
    }
    Ok(false)
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
