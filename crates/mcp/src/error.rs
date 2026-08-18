//! Error types for the minibox MCP server.

/// Result type used by the MCP crate.
pub type Result<T> = std::result::Result<T, McpServerError>;
use miette::Diagnostic;
use thiserror::Error;

/// Errors raised while handling MCP tool calls.
#[derive(Debug, Error, Diagnostic)]
pub enum McpServerError {
    /// The daemon socket could not be reached.
    #[error("daemon connection failed: {0}")]
    #[diagnostic(
        code(minibox::mcp::daemon_connection),
        help("ensure miniboxd is running and MINIBOX_SOCKET_PATH points to its socket")
    )]
    DaemonConnection(String),
    /// The daemon returned an explicit error response.
    #[error("daemon returned error: {0}")]
    #[diagnostic(code(minibox::mcp::daemon_error))]
    Daemon(String),
    /// The daemon is reachable but sent frames the client could not decode.
    #[error("daemon protocol error: {0}")]
    #[diagnostic(
        code(minibox::mcp::protocol_error),
        help(
            "the daemon is running but sent unparseable frames; check for version skew between the mcp binary and miniboxd"
        )
    )]
    ProtocolError(String),
    /// Tool parameters were invalid.
    #[error("invalid tool input: {0}")]
    #[diagnostic(code(minibox::mcp::invalid_input))]
    InvalidInput(String),
    /// Agent policy rejected the requested operation.
    #[error("policy denied {tool}: {reason}")]
    #[diagnostic(code(minibox::mcp::policy_denied))]
    PolicyDenied {
        /// Tool name.
        tool: &'static str,
        /// Human-readable denial reason.
        reason: String,
    },
    /// The daemon returned a response variant the tool did not expect.
    #[error("unexpected daemon response for {tool}: {response}")]
    #[diagnostic(code(minibox::mcp::unexpected_response))]
    UnexpectedResponse {
        /// Tool name.
        tool: &'static str,
        /// Debug representation of the unexpected response.
        response: String,
    },
    /// The daemon response stream exceeded the configured output limit.
    #[error("response stream exceeded configured output limit")]
    #[diagnostic(
        code(minibox::mcp::output_limit_exceeded),
        help("increase MINIBOX_MCP_MAX_OUTPUT_BYTES if the output is expected")
    )]
    OutputLimitExceeded,
    /// JSON serialization or deserialization failed.
    #[error(transparent)]
    #[diagnostic(code(minibox::mcp::json))]
    Json(#[from] serde_json::Error),
}

impl McpServerError {
    /// Stable diagnostic code carried in the MCP error payload.
    #[must_use]
    pub const fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::DaemonConnection(_) => "minibox::mcp::daemon_connection",
            Self::Daemon(_) => "minibox::mcp::daemon_error",
            Self::ProtocolError(_) => "minibox::mcp::protocol_error",
            Self::InvalidInput(_) => "minibox::mcp::invalid_input",
            Self::PolicyDenied { .. } => "minibox::mcp::policy_denied",
            Self::UnexpectedResponse { .. } => "minibox::mcp::unexpected_response",
            Self::OutputLimitExceeded => "minibox::mcp::output_limit_exceeded",
            Self::Json(_) => "minibox::mcp::json",
        }
    }

    /// Whether retrying the identical call can plausibly succeed.
    ///
    /// Connection and daemon-side failures are transient; policy denials and
    /// invalid input are not and require a changed request or configuration.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::DaemonConnection(_) | Self::Daemon(_))
    }
}

/// Preserve variant structure through to the MCP error payload so a calling
/// agent can distinguish "retry later" from "denied, don't retry" from "bad
/// input, fix and retry" instead of receiving one opaque string.
impl From<McpServerError> for rmcp::ErrorData {
    fn from(value: McpServerError) -> Self {
        let data = Some(serde_json::json!({
            "code": value.diagnostic_code(),
            "retryable": value.retryable(),
        }));
        let message = value.to_string();
        match value {
            McpServerError::InvalidInput(_) | McpServerError::Json(_) => {
                Self::invalid_params(message, data)
            }
            McpServerError::PolicyDenied { .. } => Self::invalid_request(message, data),
            McpServerError::DaemonConnection(_)
            | McpServerError::Daemon(_)
            | McpServerError::ProtocolError(_)
            | McpServerError::UnexpectedResponse { .. }
            | McpServerError::OutputLimitExceeded => Self::internal_error(message, data),
        }
    }
}
