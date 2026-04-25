use async_trait::async_trait;

use crate::error::LlmError;
use crate::types::{
    CompletionRequest, CompletionResponse, ContentBlock, InferenceRequest, InferenceResponse,
};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Multi-turn inference with tool support. Default wraps complete().
    async fn infer(&self, req: &InferenceRequest) -> Result<InferenceResponse, LlmError> {
        let prompt = req
            .messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let completion = self
            .complete(&CompletionRequest {
                prompt,
                system: req.system.clone(),
                max_tokens: req.max_tokens,
                schema: None,
            })
            .await?;
        Ok(InferenceResponse {
            content: vec![ContentBlock::Text {
                text: completion.text,
            }],
        })
    }
}
