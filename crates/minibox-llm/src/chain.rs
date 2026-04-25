use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse, InferenceRequest, InferenceResponse};

pub struct FallbackChain {
    providers: Vec<Box<dyn LlmProvider>>,
}

impl FallbackChain {
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        Self { providers }
    }

    pub async fn infer(&self, req: &InferenceRequest) -> Result<InferenceResponse, LlmError> {
        for provider in &self.providers {
            match provider.infer(req).await {
                Ok(resp) => return Ok(resp),
                Err(_) => continue,
            }
        }
        Err(LlmError::AllProvidersFailed)
    }

    pub async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        for provider in &self.providers {
            match provider.complete(req).await {
                Ok(resp) => return Ok(resp),
                Err(_) => continue,
            }
        }
        Err(LlmError::AllProvidersFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentBlock, Message};
    use async_trait::async_trait;

    struct SuccessProvider {
        response: String,
    }

    #[async_trait]
    impl LlmProvider for SuccessProvider {
        async fn complete(&self, _req: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                text: self.response.clone(),
            })
        }

        async fn infer(&self, _req: &InferenceRequest) -> Result<InferenceResponse, LlmError> {
            Ok(InferenceResponse {
                content: vec![ContentBlock::Text {
                    text: self.response.clone(),
                }],
            })
        }
    }

    struct FailProvider;

    #[async_trait]
    impl LlmProvider for FailProvider {
        async fn complete(&self, _req: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Provider("fail".to_string()))
        }

        async fn infer(&self, _req: &InferenceRequest) -> Result<InferenceResponse, LlmError> {
            Err(LlmError::Provider("fail".to_string()))
        }
    }

    fn make_infer_req() -> InferenceRequest {
        InferenceRequest {
            messages: vec![Message::user("hello")],
            tools: vec![],
            system: None,
            max_tokens: 100,
        }
    }

    fn make_complete_req() -> CompletionRequest {
        CompletionRequest {
            prompt: "hello".to_string(),
            system: None,
            max_tokens: 100,
            schema: None,
        }
    }

    #[tokio::test]
    async fn fallback_chain_uses_first_success() {
        let chain = FallbackChain::new(vec![
            Box::new(SuccessProvider {
                response: "first".to_string(),
            }),
            Box::new(SuccessProvider {
                response: "second".to_string(),
            }),
        ]);
        let resp = chain.complete(&make_complete_req()).await.unwrap();
        assert_eq!(resp.text, "first");
    }

    #[tokio::test]
    async fn fallback_chain_skips_failures() {
        let chain = FallbackChain::new(vec![
            Box::new(FailProvider),
            Box::new(SuccessProvider {
                response: "fallback".to_string(),
            }),
        ]);
        let resp = chain.complete(&make_complete_req()).await.unwrap();
        assert_eq!(resp.text, "fallback");
    }

    #[tokio::test]
    async fn fallback_chain_all_fail_returns_error() {
        let chain = FallbackChain::new(vec![Box::new(FailProvider), Box::new(FailProvider)]);
        let result = chain.complete(&make_complete_req()).await;
        assert!(matches!(result, Err(LlmError::AllProvidersFailed)));
    }

    #[tokio::test]
    async fn fallback_chain_infer_uses_first_success() {
        let chain = FallbackChain::new(vec![
            Box::new(FailProvider),
            Box::new(SuccessProvider {
                response: "ok".to_string(),
            }),
        ]);
        let resp = chain.infer(&make_infer_req()).await.unwrap();
        assert_eq!(resp.text(), "ok");
    }
}
