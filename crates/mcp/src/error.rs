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

// TODO(review): this collapses every McpServerError variant (DaemonConnection,
// PolicyDenied, InvalidInput, UnexpectedResponse, etc.) into a single opaque string at
// the tool boundary (see server.rs `.map_err(Into::into)`), discarding the Diagnostic
// structure and preventing a calling agent from distinguishing "retry later" from
// "denied, don't retry" from "bad input, fix and retry". Preserve variant/code structure
// through to the MCP error payload instead (e.g. implement From<McpServerError> for
// rmcp's ErrorData with a distinct code per variant).
impl From<McpServerError> for String {
    fn from(value: McpServerError) -> Self {
        value.to_string()
    }
}
