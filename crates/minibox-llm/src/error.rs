#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("all providers failed")]
    AllProvidersFailed,
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("deserialization error: {0}")]
    Deserialization(String),
}
