use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("all providers failed: {0}")]
    AllProvidersFailed(String),

    #[error("provider {provider} failed: {source}")]
    ProviderError {
        provider: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("structured output failed to parse: {0}")]
    SchemaParseError(String),
}
