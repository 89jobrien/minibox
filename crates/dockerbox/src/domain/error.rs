#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("minibox error: {0}")]
    Minibox(#[from] anyhow::Error),
    #[error("client error: {0}")]
    Client(#[from] minibox_client::ClientError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
