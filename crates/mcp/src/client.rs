//! Typed minibox daemon client adapter for MCP tools.

use crate::error::{McpServerError, Result};
use crate::policy::{Authorized, DEFAULT_MAX_OUTPUT_BYTES};
use base64::Engine as _;
use minibox_core::client::{ClientError, DaemonClient, default_socket_path};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use serde_json::Value;
use std::path::PathBuf;

const MAX_RUN_METADATA_BYTES: usize = 64 * 1024;

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
        ensure_read_only(&request)?;
        Ok(self
            .call_raw(request, max_output_bytes, OutputAccounting::Serialized)
            .await?
            .result)
    }

    pub(crate) async fn call_authorized(
        &self,
        request: Authorized<DaemonRequest>,
        max_output_bytes: usize,
    ) -> Result<(DaemonCallResult, bool)> {
        let request = request.into_inner();
        let accounting = if matches!(request, DaemonRequest::Run { .. }) {
            OutputAccounting::TruncateContainerOutput
        } else {
            OutputAccounting::Serialized
        };
        let collected = self.call_raw(request, max_output_bytes, accounting).await?;
        Ok((collected.result, collected.output_truncated))
    }

    async fn call_raw(
        &self,
        request: DaemonRequest,
        max_output_bytes: usize,
        accounting: OutputAccounting,
    ) -> Result<CollectedCallResult> {
        let client = DaemonClient::with_socket(&self.socket_path);
        let mut stream = client.call(request).await.map_err(map_client_error)?;

        let mut responses = Vec::new();
        let mut raw_responses = Vec::new();
        let mut total_bytes = 0usize;
        let mut output_bytes = 0usize;
        let mut output_truncated = false;
        let mut terminal_type = None;

        while let Some(mut response) = stream.next().await.map_err(map_client_error)? {
            let is_container_output = matches!(response, DaemonResponse::ContainerOutput { .. });
            if accounting == OutputAccounting::TruncateContainerOutput {
                if let DaemonResponse::ContainerOutput { data, .. } = &mut response {
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(&*data)
                        .map_err(|error| McpServerError::ProtocolError(error.to_string()))?;
                    let remaining = max_output_bytes.saturating_sub(output_bytes);
                    let take = decoded.len().min(remaining);
                    output_bytes = output_bytes.saturating_add(take);
                    if take < decoded.len() {
                        output_truncated = true;
                    }
                    if take == 0 {
                        continue;
                    }
                    *data = base64::engine::general_purpose::STANDARD.encode(&decoded[..take]);
                }
            }

            let is_terminal = response.is_terminal();
            let response_type = response_type(&response);
            let raw = serde_json::to_value(&response)?;
            let serialized_bytes = raw.to_string().len();
            match accounting {
                OutputAccounting::Serialized => {
                    total_bytes = total_bytes.saturating_add(serialized_bytes);
                    if total_bytes > max_output_bytes {
                        return Err(McpServerError::OutputLimitExceeded);
                    }
                }
                OutputAccounting::TruncateContainerOutput if !is_container_output => {
                    total_bytes = total_bytes.saturating_add(serialized_bytes);
                    if total_bytes > MAX_RUN_METADATA_BYTES {
                        return Err(McpServerError::OutputLimitExceeded);
                    }
                }
                OutputAccounting::TruncateContainerOutput => {}
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

        Ok(CollectedCallResult {
            result: DaemonCallResult {
                responses,
                raw_responses,
                terminal_type,
            },
            output_truncated,
        })
    }
}

struct CollectedCallResult {
    result: DaemonCallResult,
    output_truncated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputAccounting {
    Serialized,
    TruncateContainerOutput,
}

fn ensure_read_only(request: &DaemonRequest) -> Result<()> {
    if matches!(
        request,
        DaemonRequest::List
            | DaemonRequest::SubscribeEvents
            | DaemonRequest::ListImages
            | DaemonRequest::ContainerLogs { .. }
            | DaemonRequest::ListSnapshots { .. }
            | DaemonRequest::GetManifest { .. }
            | DaemonRequest::VerifyManifest { .. }
            | DaemonRequest::ListPipelines { .. }
            | DaemonRequest::ShowPipeline { .. }
    ) {
        Ok(())
    } else {
        Err(McpServerError::PolicyDenied {
            tool: "daemon_call",
            reason: format!("{} requires policy authorization", request.type_tag()),
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

#[cfg(test)]
mod authorization_tests {
    use super::*;

    #[tokio::test]
    async fn unrestricted_client_call_cannot_bypass_mutation_policy() {
        let client = MiniboxDaemonClient::new("/nonexistent/minibox.sock".into());

        let result = client
            .call_limited(
                DaemonRequest::Stop {
                    id: "abc123".to_string(),
                },
                DEFAULT_MAX_OUTPUT_BYTES,
            )
            .await;

        assert!(matches!(
            result,
            Err(McpServerError::PolicyDenied {
                tool: "daemon_call",
                ..
            })
        ));
    }
}
