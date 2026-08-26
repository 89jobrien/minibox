//! Errors returned by the typed minibox daemon client.

use thiserror::Error;

#[derive(Error, Debug)]
/// Failures produced while connecting to or communicating with the daemon.
pub enum ClientError {
    /// The Unix socket could not be connected to or used.
    #[error("failed to connect to daemon: {0}")]
    ConnectionFailed(#[from] std::io::Error),

    /// The daemon returned an application-level error.
    #[error("daemon error: {0}")]
    DaemonError(String),

    /// A response frame was malformed.
    #[error("frame error: {0}")]
    FrameError(String),

    /// No daemon socket path could be resolved.
    #[error("socket path not found")]
    SocketPathNotFound,

    /// Request or response JSON could not be encoded or decoded.
    #[error("json error: {0}")]
    JsonError(#[from] serde_json::Error),
}

/// Result type returned by daemon client operations.
pub type Result<T> = std::result::Result<T, ClientError>;
