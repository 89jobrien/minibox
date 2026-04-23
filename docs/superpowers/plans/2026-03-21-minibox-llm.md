---
status: done
completed: "2026-03-21"
branch: main
---

# minibox-llm Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create a provider-agnostic LLM crate with async trait, fallback chain, structured JSON output, and three built-in providers (Anthropic, OpenAI, Gemini).

**Architecture:** Hexagonal — `LlmProvider` trait as the port, concrete provider structs as adapters, `FallbackChain` as the composition utility. Async-first with a sync wrapper using `OnceLock<Runtime>`.

**Tech Stack:** Rust, reqwest, serde/serde_json, tokio, async-trait, thiserror, tracing

**Spec:** `docs/superpowers/specs/2026-03-21-minibox-llm-design.md`

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `crates/minibox-llm/Cargo.toml` | Crate manifest with feature flags |
| Create | `crates/minibox-llm/src/lib.rs` | Re-exports, public API surface |
| Create | `crates/minibox-llm/src/types.rs` | CompletionRequest, CompletionResponse, JsonSchema, Usage |
| Create | `crates/minibox-llm/src/provider.rs` | LlmProvider trait definition |
| Create | `crates/minibox-llm/src/error.rs` | LlmError enum |
| Create | `crates/minibox-llm/src/chain.rs` | FallbackChain, sync wrapper |
| Create | `crates/minibox-llm/src/anthropic.rs` | AnthropicProvider (feature-gated) |
| Create | `crates/minibox-llm/src/openai.rs` | OpenAiProvider (feature-gated) |
| Create | `crates/minibox-llm/src/gemini.rs` | GeminiProvider (feature-gated) |
| Modify | `Cargo.toml` | Add `crates/minibox-llm` to workspace members |
| Modify | `CLAUDE.md` | Add minibox-llm to clippy/fmt quality gates |

---

### Task 1: Scaffold crate and types

**Files:**
- Create: `crates/minibox-llm/Cargo.toml`
- Create: `crates/minibox-llm/src/lib.rs`
- Create: `crates/minibox-llm/src/types.rs`
- Create: `crates/minibox-llm/src/error.rs`
- Create: `crates/minibox-llm/src/provider.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "minibox-llm"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[features]
default = ["anthropic", "openai", "gemini"]
anthropic = ["dep:reqwest"]
openai = ["dep:reqwest"]
gemini = ["dep:reqwest"]

[dependencies]
async-trait = { workspace = true }
reqwest = { workspace = true, optional = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
```

- [ ] **Step 2: Create `src/types.rs`**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prompt: String,
    pub system: Option<String>,
    pub max_tokens: u32,
    pub schema: Option<JsonSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub text: String,
    pub provider: String,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct JsonSchema {
    pub name: String,
    pub schema: serde_json::Value,
}
```

- [ ] **Step 3: Create `src/error.rs`**

```rust
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
```

- [ ] **Step 4: Create `src/provider.rs`**

```rust
use async_trait::async_trait;
use crate::error::LlmError;
use crate::types::{CompletionRequest, CompletionResponse};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError>;
}
```

- [ ] **Step 5: Create `src/lib.rs`**

```rust
pub mod error;
pub mod provider;
pub mod types;

pub use error::LlmError;
pub use provider::LlmProvider;
pub use types::{CompletionRequest, CompletionResponse, JsonSchema, Usage};
```

- [ ] **Step 6: Add to workspace `Cargo.toml`**

Add `"crates/minibox-llm"` to the `members` array. Do **not** add to `[workspace.dependencies]` yet — no other crate depends on it until `minibox-ci` is built.

- [ ] **Step 7: Verify it compiles**

Run: `cargo check -p minibox-llm`
Expected: compiles with no errors

- [ ] **Step 8: Commit**

```bash
git add crates/minibox-llm/ Cargo.toml
git commit -m "feat(minibox-llm): scaffold crate with types, trait, and error types"
```

---

### Task 2: FallbackChain and sync wrapper

**Files:**
- Create: `crates/minibox-llm/src/chain.rs`
- Modify: `crates/minibox-llm/src/lib.rs`

- [ ] **Step 1: Write failing test for empty chain**

Add to bottom of `chain.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CompletionRequest;

    fn request(prompt: &str) -> CompletionRequest {
        CompletionRequest {
            prompt: prompt.to_string(),
            system: None,
            max_tokens: 100,
            schema: None,
        }
    }

    #[tokio::test]
    async fn empty_chain_returns_all_providers_failed() {
        let chain = FallbackChain::new(vec![]);
        let result = chain.complete(&request("hello")).await;
        assert!(matches!(result, Err(LlmError::AllProvidersFailed(_))));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox-llm chain::tests::empty_chain_returns_all_providers_failed`
Expected: FAIL — `FallbackChain` doesn't exist yet

- [ ] **Step 3: Implement FallbackChain**

```rust
use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse};
use std::sync::OnceLock;

pub struct FallbackChain {
    providers: Vec<Box<dyn LlmProvider>>,
}

impl FallbackChain {
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        Self { providers }
    }

    pub async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let mut errors = Vec::new();
        for provider in &self.providers {
            match provider.complete(request).await {
                Ok(response) => {
                    tracing::info!(
                        provider = provider.name(),
                        "llm: completion succeeded"
                    );
                    return Ok(response);
                }
                Err(e) => {
                    tracing::warn!(
                        provider = provider.name(),
                        error = %e,
                        "llm: provider failed, trying next"
                    );
                    errors.push(format!("{}: {e}", provider.name()));
                }
            }
        }
        Err(LlmError::AllProvidersFailed(errors.join("; ")))
    }
}

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

impl FallbackChain {
    /// Blocking wrapper for callers not in an async context.
    ///
    /// # Panics
    ///
    /// Panics if called from within an existing Tokio runtime (e.g. inside
    /// `spawn_blocking`). Use `complete()` directly in async contexts.
    pub fn complete_sync(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let rt = RUNTIME.get_or_init(|| {
            tokio::runtime::Runtime::new().expect("failed to create tokio runtime")
        });
        rt.block_on(self.complete(request))
    }
}
```

- [ ] **Step 4: Add `pub mod chain;` and re-export in `lib.rs`**

Add:
```rust
pub mod chain;
pub use chain::FallbackChain;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p minibox-llm chain::tests::empty_chain_returns_all_providers_failed`
Expected: PASS

- [ ] **Step 6: Write test for single provider success**

```rust
    use crate::types::CompletionResponse;

    struct MockProvider {
        response: Result<String, String>,
    }

    #[async_trait::async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &str { "mock" }
        async fn complete(
            &self,
            _request: &CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            match &self.response {
                Ok(text) => Ok(CompletionResponse {
                    text: text.clone(),
                    provider: "mock".to_string(),
                    usage: None,
                }),
                Err(msg) => Err(LlmError::ProviderError {
                    provider: "mock".to_string(),
                    source: msg.clone().into(),
                }),
            }
        }
    }

    #[tokio::test]
    async fn single_provider_returns_response() {
        let chain = FallbackChain::new(vec![
            Box::new(MockProvider { response: Ok("hello".to_string()) }),
        ]);
        let result = chain.complete(&request("test")).await.unwrap();
        assert_eq!(result.text, "hello");
        assert_eq!(result.provider, "mock");
    }
```

- [ ] **Step 7: Run test — should pass immediately**

Run: `cargo test -p minibox-llm chain::tests::single_provider_returns_response`
Expected: PASS

- [ ] **Step 8: Write test for fallback on failure**

```rust
    #[tokio::test]
    async fn falls_back_to_second_provider_on_failure() {
        let chain = FallbackChain::new(vec![
            Box::new(MockProvider { response: Err("down".to_string()) }),
            Box::new(MockProvider { response: Ok("fallback".to_string()) }),
        ]);
        let result = chain.complete(&request("test")).await.unwrap();
        assert_eq!(result.text, "fallback");
    }

    #[tokio::test]
    async fn all_fail_returns_error_with_details() {
        let chain = FallbackChain::new(vec![
            Box::new(MockProvider { response: Err("err1".to_string()) }),
            Box::new(MockProvider { response: Err("err2".to_string()) }),
        ]);
        let result = chain.complete(&request("test")).await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("err1"), "error should contain first failure");
        assert!(err.contains("err2"), "error should contain second failure");
    }
```

- [ ] **Step 9: Run all chain tests**

Run: `cargo test -p minibox-llm chain::tests`
Expected: all PASS

- [ ] **Step 10: Write test for `complete_sync`**

```rust
    #[test]
    fn sync_wrapper_works_outside_async() {
        let chain = FallbackChain::new(vec![
            Box::new(MockProvider { response: Ok("sync".to_string()) }),
        ]);
        let result = chain.complete_sync(&request("test")).unwrap();
        assert_eq!(result.text, "sync");
    }
```

- [ ] **Step 11: Run all chain tests**

Run: `cargo test -p minibox-llm chain::tests`
Expected: all PASS (5 tests)

- [ ] **Step 12: Commit**

```bash
git add crates/minibox-llm/src/chain.rs crates/minibox-llm/src/lib.rs
git commit -m "feat(minibox-llm): add FallbackChain with fallback logic and sync wrapper"
```

---

### Task 3: Anthropic provider

**Files:**
- Create: `crates/minibox-llm/src/anthropic.rs`
- Modify: `crates/minibox-llm/src/lib.rs`

- [ ] **Step 1: Write test for `from_env` returning None without key**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_without_key() {
        // Safe because we're reading, not setting
        let provider = AnthropicProvider::from_env_with_key(None);
        assert!(provider.is_none());
    }

    #[test]
    fn from_env_returns_some_with_key() {
        let provider = AnthropicProvider::from_env_with_key(Some("sk-test".to_string()));
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().name(), "anthropic/claude-sonnet-4-6");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox-llm anthropic::tests`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement AnthropicProvider**

```rust
use async_trait::async_trait;
use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse, Usage};

pub struct AnthropicProvider {
    key: String,
    model: String,
    display_name: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(key: String, model: String) -> Self {
        let display_name = format!("anthropic/{model}");
        Self {
            key,
            model,
            display_name,
            client: reqwest::Client::new(),
        }
    }

    pub fn from_env() -> Option<Self> {
        Self::from_env_with_key(std::env::var("ANTHROPIC_API_KEY").ok())
    }

    pub(crate) fn from_env_with_key(key: Option<String>) -> Option<Self> {
        key.map(|k| Self::new(k, "claude-sonnet-4-6".to_string()))
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.display_name
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "messages": [{"role": "user", "content": &request.prompt}],
        });

        if let Some(system) = &request.system {
            body["system"] = serde_json::json!(system);
        }

        if let Some(schema) = &request.schema {
            body["tools"] = serde_json::json!([{
                "name": &schema.name,
                "description": "Respond with structured output",
                "input_schema": &schema.schema,
            }]);
            body["tool_choice"] = serde_json::json!({
                "type": "tool",
                "name": &schema.name,
            });
        }

        let start = std::time::Instant::now();
        tracing::debug!(
            provider = self.name(),
            model = %self.model,
            max_tokens = request.max_tokens,
            schema = request.schema.as_ref().map(|s| s.name.as_str()),
            "llm: sending request"
        );

        let resp = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;

        let status = resp.status();
        let resp_body: serde_json::Value = resp.json().await.map_err(|e| {
            LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            }
        })?;

        if !status.is_success() {
            let msg = resp_body["error"]["message"]
                .as_str()
                .unwrap_or("unknown API error");
            return Err(LlmError::ProviderError {
                provider: self.name().to_string(),
                source: msg.to_string().into(),
            });
        }

        let text = if request.schema.is_some() {
            // Extract from tool_use content block
            resp_body["content"]
                .as_array()
                .and_then(|blocks| {
                    blocks.iter().find(|b| b["type"] == "tool_use")
                })
                .map(|b| b["input"].to_string())
                .ok_or_else(|| {
                    LlmError::SchemaParseError(
                        "no tool_use block in response".to_string(),
                    )
                })?
        } else {
            resp_body["content"]
                .as_array()
                .and_then(|blocks| {
                    blocks.iter().find(|b| b["type"] == "text")
                })
                .and_then(|b| b["text"].as_str())
                .unwrap_or("")
                .to_string()
        };

        let usage = resp_body["usage"].as_object().map(|u| Usage {
            input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
        });

        tracing::debug!(
            provider = self.name(),
            elapsed_ms = start.elapsed().as_millis() as u64,
            input_tokens = usage.as_ref().map(|u| u.input_tokens),
            output_tokens = usage.as_ref().map(|u| u.output_tokens),
            "llm: response received"
        );

        Ok(CompletionResponse {
            text,
            provider: self.name().to_string(),
            usage,
        })
    }
}
```

Note: The same `start`/`debug!` pattern should be applied to OpenAI and Gemini providers — add `let start = std::time::Instant::now();` before the HTTP call and the elapsed `debug!` after parsing the response. Omitted from the plan for brevity but required by the spec's tracing contract.

- [ ] **Step 4: Add to `lib.rs`**

```rust
#[cfg(feature = "anthropic")]
pub mod anthropic;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p minibox-llm anthropic::tests`
Expected: PASS (2 tests)

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-llm/src/anthropic.rs crates/minibox-llm/src/lib.rs
git commit -m "feat(minibox-llm): add Anthropic provider with structured output via tool_use"
```

---

### Task 4: OpenAI provider

**Files:**
- Create: `crates/minibox-llm/src/openai.rs`
- Modify: `crates/minibox-llm/src/lib.rs`

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_without_key() {
        let provider = OpenAiProvider::from_env_with_key(None);
        assert!(provider.is_none());
    }

    #[test]
    fn from_env_returns_some_with_key() {
        let provider = OpenAiProvider::from_env_with_key(Some("sk-test".to_string()));
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().name(), "openai/gpt-4.1");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox-llm openai::tests`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement OpenAiProvider**

```rust
use async_trait::async_trait;
use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse, Usage};

pub struct OpenAiProvider {
    key: String,
    model: String,
    display_name: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(key: String, model: String) -> Self {
        let display_name = format!("openai/{model}");
        Self {
            key,
            model,
            display_name,
            client: reqwest::Client::new(),
        }
    }

    pub fn from_env() -> Option<Self> {
        Self::from_env_with_key(std::env::var("OPENAI_API_KEY").ok())
    }

    pub(crate) fn from_env_with_key(key: Option<String>) -> Option<Self> {
        key.map(|k| Self::new(k, "gpt-4.1".to_string()))
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.display_name
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let mut messages = Vec::new();

        if let Some(system) = &request.system {
            messages.push(serde_json::json!({"role": "system", "content": system}));
        }
        messages.push(serde_json::json!({"role": "user", "content": &request.prompt}));

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "messages": messages,
        });

        if let Some(schema) = &request.schema {
            body["response_format"] = serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": &schema.name,
                    "schema": &schema.schema,
                    "strict": true,
                },
            });
        }

        let resp = self.client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.key))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;

        let status = resp.status();
        let resp_body: serde_json::Value = resp.json().await.map_err(|e| {
            LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            }
        })?;

        if !status.is_success() {
            let msg = resp_body["error"]["message"]
                .as_str()
                .unwrap_or("unknown API error");
            return Err(LlmError::ProviderError {
                provider: self.name().to_string(),
                source: msg.to_string().into(),
            });
        }

        let text = resp_body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = resp_body["usage"].as_object().map(|u| Usage {
            input_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
        });

        Ok(CompletionResponse {
            text,
            provider: self.name().to_string(),
            usage,
        })
    }
}
```

- [ ] **Step 4: Add to `lib.rs`**

```rust
#[cfg(feature = "openai")]
pub mod openai;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p minibox-llm openai::tests`
Expected: PASS (2 tests)

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-llm/src/openai.rs crates/minibox-llm/src/lib.rs
git commit -m "feat(minibox-llm): add OpenAI provider with structured output via json_schema"
```

---

### Task 5: Gemini provider

**Files:**
- Create: `crates/minibox-llm/src/gemini.rs`
- Modify: `crates/minibox-llm/src/lib.rs`

- [ ] **Step 1: Write tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_without_key() {
        let provider = GeminiProvider::from_env_with_key(None);
        assert!(provider.is_none());
    }

    #[test]
    fn from_env_returns_some_with_key() {
        let provider = GeminiProvider::from_env_with_key(Some("key-test".to_string()));
        assert!(provider.is_some());
        assert_eq!(provider.unwrap().name(), "google/gemini-2.5-flash");
    }

    #[test]
    fn sanitize_strips_unsupported_keywords() {
        let schema = serde_json::json!({
            "type": "object",
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$ref": "#/definitions/Foo",
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string" }
            }
        });
        let sanitized = sanitize_schema(&schema);
        assert!(sanitized.get("$schema").is_none());
        assert!(sanitized.get("$ref").is_none());
        assert!(sanitized.get("additionalProperties").is_none());
        assert!(sanitized.get("type").is_some());
        assert!(sanitized.get("properties").is_some());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox-llm gemini::tests`
Expected: FAIL — module doesn't exist

- [ ] **Step 3: Implement GeminiProvider with schema sanitization**

```rust
use async_trait::async_trait;
use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse, Usage};

const UNSUPPORTED_KEYWORDS: &[&str] = &[
    "$schema", "$ref", "$id", "$comment",
    "additionalProperties", "patternProperties",
    "if", "then", "else", "allOf", "anyOf", "oneOf", "not",
];

pub(crate) fn sanitize_schema(schema: &serde_json::Value) -> serde_json::Value {
    match schema {
        serde_json::Value::Object(map) => {
            let filtered: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter(|(k, _)| !UNSUPPORTED_KEYWORDS.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), sanitize_schema(v)))
                .collect();
            serde_json::Value::Object(filtered)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(sanitize_schema).collect())
        }
        other => other.clone(),
    }
}

pub struct GeminiProvider {
    key: String,
    model: String,
    display_name: String,
    client: reqwest::Client,
}

impl GeminiProvider {
    pub fn new(key: String, model: String) -> Self {
        let display_name = format!("google/{model}");
        Self {
            key,
            model,
            display_name,
            client: reqwest::Client::new(),
        }
    }

    pub fn from_env() -> Option<Self> {
        Self::from_env_with_key(std::env::var("GEMINI_API_KEY").ok())
    }

    pub(crate) fn from_env_with_key(key: Option<String>) -> Option<Self> {
        key.map(|k| Self::new(k, "gemini-2.5-flash".to_string()))
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn name(&self) -> &str {
        &self.display_name
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model,
        );

        let mut body = serde_json::json!({
            "contents": [{"parts": [{"text": &request.prompt}]}],
        });

        if let Some(system) = &request.system {
            body["systemInstruction"] = serde_json::json!({
                "parts": [{"text": system}],
            });
        }

        let mut generation_config = serde_json::json!({
            "maxOutputTokens": request.max_tokens,
        });

        if let Some(schema) = &request.schema {
            generation_config["responseMimeType"] = "application/json".into();
            generation_config["responseSchema"] = sanitize_schema(&schema.schema);
        }

        body["generationConfig"] = generation_config;

        let resp = self.client
            .post(&url)
            .header("x-goog-api-key", &self.key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;

        let status = resp.status();
        let resp_body: serde_json::Value = resp.json().await.map_err(|e| {
            LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            }
        })?;

        if !status.is_success() {
            let msg = resp_body["error"]["message"]
                .as_str()
                .unwrap_or("unknown API error");
            return Err(LlmError::ProviderError {
                provider: self.name().to_string(),
                source: msg.to_string().into(),
            });
        }

        let text = resp_body["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = resp_body["usageMetadata"].as_object().map(|u| Usage {
            input_tokens: u["promptTokenCount"].as_u64().unwrap_or(0) as u32,
            output_tokens: u["candidatesTokenCount"].as_u64().unwrap_or(0) as u32,
        });

        Ok(CompletionResponse {
            text,
            provider: self.name().to_string(),
            usage,
        })
    }
}
```

- [ ] **Step 4: Add to `lib.rs`**

```rust
#[cfg(feature = "gemini")]
pub mod gemini;
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p minibox-llm gemini::tests`
Expected: PASS (3 tests)

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-llm/src/gemini.rs crates/minibox-llm/src/lib.rs
git commit -m "feat(minibox-llm): add Gemini provider with schema sanitization"
```

---

### Task 6: `FallbackChain::from_env` and integration

**Files:**
- Modify: `crates/minibox-llm/src/chain.rs`

- [ ] **Step 1: Write test for `from_env`**

Add to `chain::tests`:

```rust
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn from_env_with_no_keys_creates_empty_chain() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // Clear any API keys that might be set in the test environment.
        // unsafe: Rust 2024 requires unsafe for env mutation. Mutex serializes access.
        unsafe {
            std::env::remove_var("ANTHROPIC_API_KEY");
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("GEMINI_API_KEY");
        }
        let chain = FallbackChain::from_env();
        let result = chain.complete_sync(&request("test"));
        assert!(matches!(result, Err(LlmError::AllProvidersFailed(_))));
    }
```

- [ ] **Step 2: Implement `from_env`**

Add to `chain.rs`:

```rust
impl FallbackChain {
    pub fn from_env() -> Self {
        let mut providers: Vec<Box<dyn LlmProvider>> = Vec::new();

        #[cfg(feature = "anthropic")]
        if let Some(p) = crate::anthropic::AnthropicProvider::from_env() {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(p));
        }

        #[cfg(feature = "openai")]
        if let Some(p) = crate::openai::OpenAiProvider::from_env() {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(p));
        }

        #[cfg(feature = "gemini")]
        if let Some(p) = crate::gemini::GeminiProvider::from_env() {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(p));
        }

        if providers.is_empty() {
            tracing::warn!("llm: no providers available — all API keys missing");
        }

        Self { providers }
    }
}
```

- [ ] **Step 3: Run all tests**

Run: `cargo test -p minibox-llm`
Expected: all PASS (12+ tests)

- [ ] **Step 4: Commit**

```bash
git add crates/minibox-llm/src/chain.rs
git commit -m "feat(minibox-llm): add FallbackChain::from_env with auto-discovery"
```

---

### Task 7: Quality gates and docs

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add minibox-llm to clippy target in CLAUDE.md**

Find the `cargo clippy` line in the macOS quality gates section and add `-p minibox-llm`.

- [ ] **Step 2: Run quality gates**

Run:
```bash
cargo fmt --all --check
cargo clippy -p minibox-llm -- -D warnings
cargo test -p minibox-llm
```
Expected: all pass, no warnings

- [ ] **Step 3: Fix any clippy/fmt issues**

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: add minibox-llm to quality gates"
```

---

### Task 8: Verify no-default-features builds

**Files:** None (verification only)

- [ ] **Step 1: Build with no provider features**

Run: `cargo check -p minibox-llm --no-default-features`
Expected: compiles — only trait, types, chain, error are built. No reqwest dependency.

- [ ] **Step 2: Build with single provider**

Run: `cargo check -p minibox-llm --no-default-features --features anthropic`
Expected: compiles — only anthropic provider + reqwest

- [ ] **Step 3: Fix any conditional compilation issues**

Ensure `from_env` compiles when provider features are disabled (the `#[cfg(feature = "...")]` blocks should handle this).

- [ ] **Step 4: Commit if fixes were needed**

```bash
git commit -am "fix(minibox-llm): conditional compilation for feature flags"
```
