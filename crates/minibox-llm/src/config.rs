pub async fn create_provider() -> Option<Box<dyn crate::provider::LlmProvider>> {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        return None; // Anthropic provider not yet implemented
    }
    if std::env::var("OPENAI_API_KEY").is_ok() {
        return None; // OpenAI provider not yet implemented
    }
    #[cfg(feature = "local")]
    {
        let ollama = crate::local::OllamaProvider::from_env();
        if ollama.is_available().await {
            return Some(Box::new(ollama));
        }
    }
    None
}
