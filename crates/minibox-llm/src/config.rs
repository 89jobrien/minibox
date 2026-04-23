/// Provider factory — detects which LLM backend to use from the environment.
///
/// Auto-detection order: Anthropic → OpenAI → Gemini → Ollama (`local` feature).
/// Returns the first provider whose required environment variable(s) are set.
/// Returns `None` if no provider is available.
use crate::provider::{LlmProvider, ProviderConfig};

/// Create the best available [`LlmProvider`] by inspecting environment variables.
///
/// Detection order:
/// 1. `ANTHROPIC_API_KEY` → [`AnthropicProvider`](crate::anthropic::AnthropicProvider)
///    (requires `anthropic` feature)
/// 2. `OPENAI_API_KEY` → [`OpenAiProvider`](crate::openai::OpenAiProvider)
///    (requires `openai` feature)
/// 3. `GEMINI_API_KEY` → [`GeminiProvider`](crate::gemini::GeminiProvider)
///    (requires `gemini` feature)
/// 4. Ollama fallback → [`OllamaProvider`](crate::local::OllamaProvider)
///    (requires `local` feature)
///
/// Returns `None` if no applicable feature is compiled in or no key is set.
pub fn create_provider(config: &ProviderConfig) -> Option<Box<dyn LlmProvider>> {
    #[cfg(feature = "anthropic")]
    if let Some(p) = crate::anthropic::AnthropicProvider::from_env_with_config(config) {
        return Some(Box::new(p));
    }

    #[cfg(feature = "openai")]
    if let Some(p) = crate::openai::OpenAiProvider::from_env_with_config(config) {
        return Some(Box::new(p));
    }

    #[cfg(feature = "gemini")]
    if let Some(p) = crate::gemini::GeminiProvider::from_env_with_config(config) {
        return Some(Box::new(p));
    }

    #[cfg(feature = "local")]
    {
        return Some(Box::new(crate::local::OllamaProvider::from_env()));
    }

    #[allow(unreachable_code)]
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialises environment-variable mutations across parallel tests.
    // SAFETY: Rust 2024 requires unsafe for set_var/remove_var. Mutex ensures
    // single-threaded env access.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn create_provider_returns_anthropic_when_key_set() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // SAFETY: ENV_MUTEX serialises access to the environment.
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "test-key");
        }
        let _provider = create_provider(&ProviderConfig::default());
        // SAFETY: cleanup
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
        }
        // With the `anthropic` feature (default), Anthropic should be picked first.
        #[cfg(feature = "anthropic")]
        assert!(_provider.is_some());
    }

    #[test]
    fn create_provider_without_any_cloud_key_falls_back_to_ollama_when_local_enabled() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // Remove all cloud keys.
        // SAFETY: ENV_MUTEX serialises access.
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("GEMINI_API_KEY");
        }
        let _provider = create_provider(&ProviderConfig::default());
        // With the `local` feature the Ollama fallback should always be present.
        #[cfg(feature = "local")]
        assert!(_provider.is_some());
        // Without any feature (and no keys) None is expected — but this branch
        // is unlikely in practice given the `default` features.
        #[cfg(not(any(
            feature = "anthropic",
            feature = "openai",
            feature = "gemini",
            feature = "local"
        )))]
        assert!(_provider.is_none());
    }
}
