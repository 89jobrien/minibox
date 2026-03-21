use crate::error::LlmError;
use crate::types::{CompletionRequest, CompletionResponse};
use async_trait::async_trait;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse, LlmError>;
}
