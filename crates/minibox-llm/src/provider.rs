use crate::error::LlmError;
use crate::types::{CompletionRequest, CompletionResponse};
use async_trait::async_trait;
use std::time::Duration;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError>;
}

/// HTTP-level configuration for LLM providers.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(60),
        }
    }
}
