use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("failed to connect to daemon: {0}")]
    ConnectionFailed(#[from] std::io::Error),

    #[error("daemon error: {0}")]
    DaemonError(String),

    #[error("frame error: {0}")]
    FrameError(String),

    #[error("socket path not found")]
    SocketPathNotFound,

    #[error("json error: {0}")]
    JsonError(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ClientError>;
