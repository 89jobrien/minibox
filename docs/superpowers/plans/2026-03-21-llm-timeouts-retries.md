# minibox-llm Timeouts & Retries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add HTTP timeouts, transient error retry with exponential backoff, and ergonomic macros to the minibox-llm crate.

**Architecture:** Reqwest timeouts live in providers (HTTP concern). Retry/backoff lives in a `RetryingProvider` wrapper (cross-cutting concern). `provide!` macro eliminates env var boilerplate. `invoke!`/`ainvoke!` macros provide ergonomic request dispatch.

**Tech Stack:** Rust, reqwest 0.12, tokio, async-trait, serde_json, tracing

**Spec:** `docs/superpowers/specs/2026-03-21-llm-timeouts-retries-design.md`

**Quality gates:** `cargo xtask pre-commit` (fmt + clippy + release build) must pass after every commit. Run `cargo test -p minibox-llm` after every implementation step.

---

## File Structure

| File | Responsibility |
|------|---------------|
| `crates/minibox-llm/src/types.rs` | Add `timeout`, `max_retries` fields + manual `Default` impl for `CompletionRequest` |
| `crates/minibox-llm/src/error.rs` | Add `HttpStatusError` struct + `is_transient()` method |
| `crates/minibox-llm/src/provider.rs` | Add `ProviderConfig` struct |
| `crates/minibox-llm/src/retry.rs` (new) | `RetryConfig` + `RetryingProvider<P>` wrapper |
| `crates/minibox-llm/src/anthropic.rs` | `with_config()` constructor, explicit status checking, per-request timeout, remove old `from_env` methods |
| `crates/minibox-llm/src/openai.rs` | Same pattern as anthropic |
| `crates/minibox-llm/src/gemini.rs` | Same pattern as anthropic |
| `crates/minibox-llm/src/chain.rs` | `from_env_with_config()`, wrap providers in `RetryingProvider` |
| `crates/minibox-llm/src/lib.rs` | `provide!` macro, `invoke!`/`ainvoke!` macros, module + re-export declarations |

---

### Task 1: Add `HttpStatusError` and `is_transient()` to error types

**Files:**
- Modify: `crates/minibox-llm/src/error.rs`

- [ ] **Step 1: Write test for `is_transient()` classification**

Add to `crates/minibox-llm/src/error.rs` at the bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn http_error(status: u16) -> LlmError {
        LlmError::ProviderError {
            provider: "test".to_string(),
            source: Box::new(HttpStatusError {
                status,
                body: "test".to_string(),
            }),
        }
    }

    #[test]
    fn transient_http_statuses() {
        assert!(http_error(429).is_transient());
        assert!(http_error(500).is_transient());
        assert!(http_error(502).is_transient());
        assert!(http_error(503).is_transient());
        assert!(http_error(504).is_transient());
    }

    #[test]
    fn permanent_http_statuses() {
        assert!(!http_error(400).is_transient());
        assert!(!http_error(401).is_transient());
        assert!(!http_error(403).is_transient());
        assert!(!http_error(404).is_transient());
    }

    #[test]
    fn schema_parse_error_is_permanent() {
        let e = LlmError::SchemaParseError("bad".to_string());
        assert!(!e.is_transient());
    }

    #[test]
    fn all_providers_failed_is_permanent() {
        let e = LlmError::AllProvidersFailed("all failed".to_string());
        assert!(!e.is_transient());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox-llm error::tests -- --nocapture`
Expected: FAIL — `HttpStatusError` and `is_transient()` don't exist yet.

- [ ] **Step 3: Add `HttpStatusError` struct**

Add above the `LlmError` enum in `crates/minibox-llm/src/error.rs` (before line 3):

```rust
/// Typed HTTP error for status code classification via downcasting.
#[derive(Debug)]
pub struct HttpStatusError {
    pub status: u16,
    pub body: String,
}

impl std::fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.status, self.body)
    }
}

impl std::error::Error for HttpStatusError {}
```

- [ ] **Step 4: Add `is_transient()` method on `LlmError`**

Add after the `LlmError` enum definition:

```rust
impl LlmError {
    /// Returns `true` for errors that may succeed on retry (timeouts, 429, 5xx).
    pub fn is_transient(&self) -> bool {
        match self {
            LlmError::ProviderError { source, .. } => {
                // Check for reqwest errors (timeout, connect, request).
                // reqwest is feature-gated, so use cfg to avoid referencing the type
                // when no provider features are enabled.
                #[cfg(any(feature = "anthropic", feature = "openai", feature = "gemini"))]
                if let Some(reqwest_err) = source.downcast_ref::<reqwest::Error>() {
                    return reqwest_err.is_timeout()
                        || reqwest_err.is_connect()
                        || reqwest_err.is_request();
                }
                if let Some(status_err) = source.downcast_ref::<HttpStatusError>() {
                    return matches!(status_err.status, 429 | 500 | 502 | 503 | 504);
                }
                false
            }
            LlmError::SchemaParseError(_) => false,
            LlmError::AllProvidersFailed(_) => false,
        }
    }
}
```

The `#[cfg]` attribute on the `if let` branch avoids referencing `reqwest::Error` when no provider features are enabled. No `use reqwest;` import needed — the type is referenced by full path in the downcast.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p minibox-llm error::tests -- --nocapture`
Expected: All 4 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-llm/src/error.rs
git commit -m "feat(minibox-llm): add HttpStatusError and is_transient() classification"
```

---

### Task 2: Add `ProviderConfig` and update `CompletionRequest`

**Files:**
- Modify: `crates/minibox-llm/src/provider.rs`
- Modify: `crates/minibox-llm/src/types.rs`

- [ ] **Step 1: Add `ProviderConfig` to `provider.rs`**

Add after the `LlmProvider` trait definition in `crates/minibox-llm/src/provider.rs` (after line 10):

```rust
use std::time::Duration;

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
```

- [ ] **Step 2: Update `CompletionRequest` with new fields and `Default` impl**

In `crates/minibox-llm/src/types.rs`, replace the `CompletionRequest` struct (lines 3-9) with:

```rust
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prompt: String,
    pub system: Option<String>,
    pub max_tokens: u32,
    pub schema: Option<JsonSchema>,
    pub timeout: Option<Duration>,
    pub max_retries: Option<u32>,
}

impl Default for CompletionRequest {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            system: None,
            max_tokens: 1024,
            schema: None,
            timeout: None,
            max_retries: None,
        }
    }
}
```

- [ ] **Step 3: Update all `CompletionRequest` construction sites**

Every place that constructs a `CompletionRequest` needs the two new fields. Search for `CompletionRequest {` in the crate:

In `crates/minibox-llm/src/chain.rs` test helper `fn request()` (line ~105-112), add:

```rust
timeout: None,
max_retries: None,
```

- [ ] **Step 4: Update exports in `lib.rs`**

In `crates/minibox-llm/src/lib.rs`, update re-exports:

```rust
pub use error::{HttpStatusError, LlmError};
pub use provider::{LlmProvider, ProviderConfig};
```

- [ ] **Step 5: Run all tests to verify nothing broke**

Run: `cargo test -p minibox-llm -- --nocapture`
Expected: All 13 existing tests + 4 new error tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-llm/src/provider.rs crates/minibox-llm/src/types.rs crates/minibox-llm/src/chain.rs crates/minibox-llm/src/lib.rs
git commit -m "feat(minibox-llm): add ProviderConfig and CompletionRequest timeout/retry fields"
```

---

### Task 3: Add `RetryConfig` and `RetryingProvider` wrapper

**Files:**
- Create: `crates/minibox-llm/src/retry.rs`
- Modify: `crates/minibox-llm/src/lib.rs`

- [ ] **Step 1: Write tests for `RetryingProvider`**

Create `crates/minibox-llm/src/retry.rs` with tests first:

```rust
use std::time::Duration;

use async_trait::async_trait;

use crate::error::LlmError;
use crate::provider::LlmProvider;
use crate::types::{CompletionRequest, CompletionResponse};

/// Retry configuration for transient error handling.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub backoff_base: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            backoff_base: Duration::from_secs(1),
        }
    }
}

// RetryingProvider will go here

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::HttpStatusError;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingProvider {
        call_count: AtomicU32,
        fail_times: u32,
        transient: bool,
    }

    impl CountingProvider {
        fn new(fail_times: u32, transient: bool) -> Self {
            Self {
                call_count: AtomicU32::new(0),
                fail_times,
                transient,
            }
        }

        fn calls(&self) -> u32 {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl LlmProvider for CountingProvider {
        fn name(&self) -> &str {
            "counting"
        }

        async fn complete(
            &self,
            _request: &CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_times {
                let status = if self.transient { 503 } else { 401 };
                Err(LlmError::ProviderError {
                    provider: "counting".to_string(),
                    source: Box::new(HttpStatusError {
                        status,
                        body: "fail".to_string(),
                    }),
                })
            } else {
                Ok(CompletionResponse {
                    text: "ok".to_string(),
                    provider: "counting".to_string(),
                    usage: None,
                })
            }
        }
    }

    fn req() -> CompletionRequest {
        CompletionRequest {
            prompt: "test".to_string(),
            ..CompletionRequest::default()
        }
    }

    #[tokio::test]
    async fn no_retry_on_success() {
        let provider = CountingProvider::new(0, true);
        let retrying = RetryingProvider::new(provider, RetryConfig {
            max_retries: 2,
            backoff_base: Duration::from_millis(1),
        });
        let resp = retrying.complete(&req()).await.unwrap();
        assert_eq!(resp.text, "ok");
        assert_eq!(retrying.inner.calls(), 1);
    }

    #[tokio::test]
    async fn retries_transient_errors() {
        let provider = CountingProvider::new(2, true); // fail twice, succeed third
        let retrying = RetryingProvider::new(provider, RetryConfig {
            max_retries: 2,
            backoff_base: Duration::from_millis(1),
        });
        let resp = retrying.complete(&req()).await.unwrap();
        assert_eq!(resp.text, "ok");
        assert_eq!(retrying.inner.calls(), 3);
    }

    #[tokio::test]
    async fn no_retry_on_permanent_error() {
        let provider = CountingProvider::new(1, false); // permanent error
        let retrying = RetryingProvider::new(provider, RetryConfig {
            max_retries: 2,
            backoff_base: Duration::from_millis(1),
        });
        let result = retrying.complete(&req()).await;
        assert!(result.is_err());
        assert_eq!(retrying.inner.calls(), 1); // no retry
    }

    #[tokio::test]
    async fn exhausts_retries_on_persistent_transient_error() {
        let provider = CountingProvider::new(10, true); // always fails
        let retrying = RetryingProvider::new(provider, RetryConfig {
            max_retries: 2,
            backoff_base: Duration::from_millis(1),
        });
        let result = retrying.complete(&req()).await;
        assert!(result.is_err());
        assert_eq!(retrying.inner.calls(), 3); // 1 initial + 2 retries
    }

    #[tokio::test]
    async fn request_level_retry_override() {
        let provider = CountingProvider::new(10, true);
        let retrying = RetryingProvider::new(provider, RetryConfig {
            max_retries: 5,
            backoff_base: Duration::from_millis(1),
        });
        let mut r = req();
        r.max_retries = Some(1); // override: only 1 retry
        let result = retrying.complete(&r).await;
        assert!(result.is_err());
        assert_eq!(retrying.inner.calls(), 2); // 1 initial + 1 retry
    }

    #[tokio::test]
    async fn name_delegates_to_inner() {
        let provider = CountingProvider::new(0, true);
        let retrying = RetryingProvider::new(provider, RetryConfig::default());
        assert_eq!(retrying.name(), "counting");
    }
}
```

- [ ] **Step 2: Declare `retry` module in `lib.rs`**

In `crates/minibox-llm/src/lib.rs`, add the module declaration (not feature-gated since `RetryConfig` is a public type):

```rust
mod retry;
```

And add to re-exports:

```rust
pub use retry::{RetryConfig, RetryingProvider};
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p minibox-llm retry::tests -- --nocapture`
Expected: FAIL — `RetryingProvider` doesn't exist yet.

- [ ] **Step 4: Implement `RetryingProvider`**

Add in `crates/minibox-llm/src/retry.rs` between `RetryConfig` and `#[cfg(test)]`:

```rust
/// Wraps any `LlmProvider` with retry logic for transient errors.
pub struct RetryingProvider<P: LlmProvider> {
    pub(crate) inner: P,
    config: RetryConfig,
}

impl<P: LlmProvider> RetryingProvider<P> {
    pub fn new(inner: P, config: RetryConfig) -> Self {
        Self { inner, config }
    }
}

#[async_trait]
impl<P: LlmProvider> LlmProvider for RetryingProvider<P> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let max = request.max_retries.unwrap_or(self.config.max_retries);
        let backoff_cap = Duration::from_secs(30);

        for attempt in 0..=max {
            match self.inner.complete(request).await {
                Ok(resp) => return Ok(resp),
                Err(e) if e.is_transient() && attempt < max => {
                    let delay = (self.config.backoff_base * 2u32.saturating_pow(attempt))
                        .min(backoff_cap);
                    tracing::warn!(
                        provider = self.inner.name(),
                        attempt = attempt + 1,
                        delay_ms = delay.as_millis() as u64,
                        error = %e,
                        "llm: transient error, retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("loop always returns: 0..=max covers all attempts")
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p minibox-llm retry::tests -- --nocapture`
Expected: All 6 tests PASS.

- [ ] **Step 6: Run full test suite**

Run: `cargo test -p minibox-llm -- --nocapture`
Expected: All tests PASS (13 existing + 4 error + 6 retry = 23).

- [ ] **Step 7: Commit**

```bash
git add crates/minibox-llm/src/retry.rs crates/minibox-llm/src/lib.rs
git commit -m "feat(minibox-llm): add RetryingProvider wrapper with exponential backoff"
```

---

### Task 4: Update providers with `with_config()` and explicit status checking

**Files:**
- Modify: `crates/minibox-llm/src/anthropic.rs`
- Modify: `crates/minibox-llm/src/openai.rs`
- Modify: `crates/minibox-llm/src/gemini.rs`
- Modify: `crates/minibox-llm/src/lib.rs`

- [ ] **Step 1: Add `provide!` macro to `lib.rs`**

In `crates/minibox-llm/src/lib.rs`, add the `provide!` macro before module declarations. This is a crate-internal macro (no `#[macro_export]`):

```rust
/// Generates `from_env()`, `from_env_with_config()`, and test-only `from_key()` for a provider.
macro_rules! provide {
    ($provider:ty, $env_var:expr, $default_model:expr) => {
        impl $provider {
            pub fn from_env() -> Option<Self> {
                Self::from_env_with_config(&$crate::ProviderConfig::default())
            }

            pub fn from_env_with_config(config: &$crate::ProviderConfig) -> Option<Self> {
                std::env::var($env_var)
                    .ok()
                    .map(|k| Self::with_config(k, $default_model.to_string(), config))
            }

            /// Test helper — inject a key without reading the environment.
            #[cfg(test)]
            pub(crate) fn from_key(key: String) -> Self {
                Self::new(key, $default_model.to_string())
            }
        }
    };
}
```

Note: The macro must appear before the module declarations that use it (Rust requires macros to be defined before use within the same crate).

- [ ] **Step 2: Update `AnthropicProvider`**

In `crates/minibox-llm/src/anthropic.rs`:

**a) Replace constructor and env methods (lines 14-31).** Remove `from_env()` and `from_env_with_key()` entirely — the `provide!` macro generates them. Replace the `impl` block with:

```rust
impl AnthropicProvider {
    pub fn new(key: String, model: String) -> Self {
        Self::with_config(key, model, &crate::ProviderConfig::default())
    }

    pub fn with_config(key: String, model: String, config: &crate::ProviderConfig) -> Self {
        let display_name = format!("anthropic/{model}");
        let client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .expect("failed to build reqwest client");
        Self {
            key,
            model,
            display_name,
            client,
        }
    }
}

provide!(AnthropicProvider, "ANTHROPIC_API_KEY", "claude-sonnet-4-6");
```

**b) Restructure `complete()` method response handling (lines 72-101).** The current code calls `resp.json()` first then checks status, but `resp.json()` and `resp.text()` both consume the body. Restructure to check status *before* parsing JSON:

Replace lines 72-101 with:

```rust
        let mut req = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body);

        if let Some(t) = request.timeout {
            req = req.timeout(t);
        }

        let resp = req.send().await.map_err(|e| LlmError::ProviderError {
            provider: self.name().to_string(),
            source: Box::new(e),
        })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(String::from))
                .unwrap_or_else(|| body.clone());
            return Err(LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(crate::error::HttpStatusError {
                    status: status.as_u16(),
                    body: message,
                }),
            });
        }

        let resp_body: serde_json::Value =
            resp.json().await.map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;
```

The rest of the method (text extraction, usage, response) stays unchanged.

**c) Replace tests (lines 141-157):**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_without_key() {
        // from_env reads ANTHROPIC_API_KEY; in test env it's not set
        // This test relies on the env var not being present in CI/dev
        let provider = AnthropicProvider::from_env();
        assert!(provider.is_none());
    }

    #[test]
    fn from_key_creates_provider_with_default_model() {
        let provider = AnthropicProvider::from_key("sk-test".to_string());
        assert_eq!(provider.name(), "anthropic/claude-sonnet-4-6");
    }
}
```

- [ ] **Step 3: Update `OpenAiProvider`**

In `crates/minibox-llm/src/openai.rs`, apply the same three changes:

**a) Replace constructor (lines 14-31):**

```rust
impl OpenAiProvider {
    pub fn new(key: String, model: String) -> Self {
        Self::with_config(key, model, &crate::ProviderConfig::default())
    }

    pub fn with_config(key: String, model: String, config: &crate::ProviderConfig) -> Self {
        let display_name = format!("openai/{model}");
        let client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .expect("failed to build reqwest client");
        Self {
            key,
            model,
            display_name,
            client,
        }
    }
}

provide!(OpenAiProvider, "OPENAI_API_KEY", "gpt-4.1");
```

**b) Restructure `complete()` response handling (lines 73-101).** Same pattern — build request with timeout, check status before JSON parse:

```rust
        let mut req = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.key))
            .header("content-type", "application/json")
            .json(&body);

        if let Some(t) = request.timeout {
            req = req.timeout(t);
        }

        let resp = req.send().await.map_err(|e| LlmError::ProviderError {
            provider: self.name().to_string(),
            source: Box::new(e),
        })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(String::from))
                .unwrap_or_else(|| body.clone());
            return Err(LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(crate::error::HttpStatusError {
                    status: status.as_u16(),
                    body: message,
                }),
            });
        }

        let resp_body: serde_json::Value =
            resp.json().await.map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;
```

**c) Replace tests (lines 135-151):**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_without_key() {
        let provider = OpenAiProvider::from_env();
        assert!(provider.is_none());
    }

    #[test]
    fn from_key_creates_provider_with_default_model() {
        let provider = OpenAiProvider::from_key("sk-test".to_string());
        assert_eq!(provider.name(), "openai/gpt-4.1");
    }
}
```

- [ ] **Step 4: Update `GeminiProvider`**

In `crates/minibox-llm/src/gemini.rs`, apply the same three changes:

**a) Replace constructor (lines 47-64):**

```rust
impl GeminiProvider {
    pub fn new(key: String, model: String) -> Self {
        Self::with_config(key, model, &crate::ProviderConfig::default())
    }

    pub fn with_config(key: String, model: String, config: &crate::ProviderConfig) -> Self {
        let display_name = format!("google/{model}");
        let client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .expect("failed to build reqwest client");
        Self {
            key,
            model,
            display_name,
            client,
        }
    }
}

provide!(GeminiProvider, "GEMINI_API_KEY", "gemini-2.5-flash");
```

**b) Restructure `complete()` response handling.** Same pattern — build request with timeout, check status before JSON parse. In Gemini the URL is constructed dynamically:

```rust
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.model
        );

        let mut req = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.key)
            .header("content-type", "application/json")
            .json(&body);

        if let Some(t) = request.timeout {
            req = req.timeout(t);
        }

        let resp = req.send().await.map_err(|e| LlmError::ProviderError {
            provider: self.name().to_string(),
            source: Box::new(e),
        })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(String::from))
                .unwrap_or_else(|| body.clone());
            return Err(LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(crate::error::HttpStatusError {
                    status: status.as_u16(),
                    body: message,
                }),
            });
        }

        let resp_body: serde_json::Value =
            resp.json().await.map_err(|e| LlmError::ProviderError {
                provider: self.name().to_string(),
                source: Box::new(e),
            })?;
```

**c) Replace tests (lines 171-206) — keep the `sanitize_strips_unsupported_keywords` test:**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_without_key() {
        let provider = GeminiProvider::from_env();
        assert!(provider.is_none());
    }

    #[test]
    fn from_key_creates_provider_with_default_model() {
        let provider = GeminiProvider::from_key("test-key".to_string());
        assert_eq!(provider.name(), "google/gemini-2.5-flash");
    }

    #[test]
    fn sanitize_strips_unsupported_keywords() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        });
        let sanitized = sanitize_schema(&schema);
        assert!(sanitized.get("additionalProperties").is_none());
        assert!(sanitized.get("$schema").is_none());
        assert!(sanitized.get("type").is_some());
        assert!(sanitized.get("properties").is_some());
    }
}
```

- [ ] **Step 5: Run full test suite**

Run: `cargo test -p minibox-llm -- --nocapture`
Expected: All tests PASS. Provider tests now use `from_key()` instead of `from_env_with_key()`.

- [ ] **Step 6: Run quality gates**

Run: `cargo xtask pre-commit`
Expected: fmt + clippy + release build all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/minibox-llm/src/anthropic.rs crates/minibox-llm/src/openai.rs crates/minibox-llm/src/gemini.rs crates/minibox-llm/src/lib.rs
git commit -m "feat(minibox-llm): add provider timeouts, provide! macro, explicit HTTP status errors"
```

---

### Task 5: Wire `RetryingProvider` into `FallbackChain`

**Files:**
- Modify: `crates/minibox-llm/src/chain.rs`

- [ ] **Step 1: Write test for retry + fallback interaction**

Add to the existing test module in `crates/minibox-llm/src/chain.rs`:

```rust
#[tokio::test]
async fn retrying_chain_retries_transient_then_succeeds() {
    use crate::error::HttpStatusError;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TransientThenOk {
        calls: AtomicU32,
    }

    #[async_trait]
    impl LlmProvider for TransientThenOk {
        fn name(&self) -> &str { "transient-ok" }
        async fn complete(&self, _r: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Err(LlmError::ProviderError {
                    provider: "transient-ok".to_string(),
                    source: Box::new(HttpStatusError { status: 503, body: "down".to_string() }),
                })
            } else {
                Ok(CompletionResponse {
                    text: "recovered".to_string(),
                    provider: "transient-ok".to_string(),
                    usage: None,
                })
            }
        }
    }

    let provider = TransientThenOk { calls: AtomicU32::new(0) };
    let retrying = crate::RetryingProvider::new(provider, crate::RetryConfig {
        max_retries: 2,
        backoff_base: std::time::Duration::from_millis(1),
    });
    let chain = FallbackChain::new(vec![Box::new(retrying)]);
    let resp = chain.complete(&request("test")).await.unwrap();
    assert_eq!(resp.text, "recovered");
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test -p minibox-llm chain::tests::retrying_chain -- --nocapture`
Expected: PASS — the `RetryingProvider` wrapper already implements `LlmProvider`, so the chain handles it naturally.

- [ ] **Step 3: Update `from_env()` to use `from_env_with_config()`**

Replace the `from_env` impl block in `crates/minibox-llm/src/chain.rs` (lines 40-75) with:

```rust
impl FallbackChain {
    pub fn from_env() -> Self {
        Self::from_env_with_config(ProviderConfig::default(), RetryConfig::default())
    }

    pub fn from_env_with_config(
        provider_config: ProviderConfig,
        retry_config: RetryConfig,
    ) -> Self {
        let mut providers: Vec<Box<dyn LlmProvider>> = Vec::new();

        #[cfg(feature = "anthropic")]
        if let Some(p) = crate::anthropic::AnthropicProvider::from_env_with_config(&provider_config)
        {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(
                crate::retry::RetryingProvider::new(p, retry_config.clone()),
            ));
        } else {
            tracing::warn!(provider = "anthropic", "llm: provider skipped (no key)");
        }

        #[cfg(feature = "openai")]
        if let Some(p) = crate::openai::OpenAiProvider::from_env_with_config(&provider_config) {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(
                crate::retry::RetryingProvider::new(p, retry_config.clone()),
            ));
        } else {
            tracing::warn!(provider = "openai", "llm: provider skipped (no key)");
        }

        #[cfg(feature = "gemini")]
        if let Some(p) = crate::gemini::GeminiProvider::from_env_with_config(&provider_config) {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(
                crate::retry::RetryingProvider::new(p, retry_config.clone()),
            ));
        } else {
            tracing::warn!(provider = "gemini", "llm: provider skipped (no key)");
        }

        if providers.is_empty() {
            tracing::warn!("llm: no providers available — all API keys missing");
        }

        Self { providers }
    }
}
```

Add necessary imports at the top of `chain.rs`:

```rust
use crate::provider::ProviderConfig;
use crate::retry::RetryConfig;
```

- [ ] **Step 4: Run full test suite**

Run: `cargo test -p minibox-llm -- --nocapture`
Expected: All tests PASS.

- [ ] **Step 5: Run quality gates**

Run: `cargo xtask pre-commit`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-llm/src/chain.rs
git commit -m "feat(minibox-llm): wire RetryingProvider into FallbackChain"
```

---

### Task 6: Add `invoke!` and `ainvoke!` macros

**Files:**
- Modify: `crates/minibox-llm/src/lib.rs`

- [ ] **Step 1: Add `ainvoke!` macro**

In `crates/minibox-llm/src/lib.rs`, add:

```rust
/// Async LLM invocation. Returns a future that resolves to `Result<CompletionResponse, LlmError>`.
///
/// ```ignore
/// let resp = ainvoke!(chain, "Summarize this").await?;
/// let resp = ainvoke!(chain, "Summarize", system: "Be concise", max_tokens: 512).await?;
/// ```
#[macro_export]
macro_rules! ainvoke {
    ($chain:expr, $prompt:expr $(, $key:ident : $val:expr)* $(,)?) => {{
        let request = $crate::CompletionRequest {
            prompt: $prompt.into(),
            $( $key: $crate::ainvoke!(@wrap $key $val), )*
            ..$crate::CompletionRequest::default()
        };
        $chain.complete(&request)
    }};
    (@wrap system $val:expr) => { Some($val.into()) };
    (@wrap schema $val:expr) => { Some($val) };
    (@wrap timeout $val:expr) => { Some($val) };
    (@wrap max_retries $val:expr) => { Some($val) };
    (@wrap max_tokens $val:expr) => { $val };
}
```

- [ ] **Step 2: Add `invoke!` macro**

```rust
/// Sync (blocking) LLM invocation. Only works with `FallbackChain`.
///
/// ```ignore
/// let resp = invoke!(chain, "Summarize this")?;
/// let resp = invoke!(chain, "Summarize", system: "Be concise", max_tokens: 512)?;
/// ```
#[macro_export]
macro_rules! invoke {
    ($chain:expr, $prompt:expr $(, $key:ident : $val:expr)* $(,)?) => {{
        let request = $crate::CompletionRequest {
            prompt: $prompt.into(),
            $( $key: $crate::invoke!(@wrap $key $val), )*
            ..$crate::CompletionRequest::default()
        };
        $chain.complete_sync(&request)
    }};
    (@wrap system $val:expr) => { Some($val.into()) };
    (@wrap schema $val:expr) => { Some($val) };
    (@wrap timeout $val:expr) => { Some($val) };
    (@wrap max_retries $val:expr) => { Some($val) };
    (@wrap max_tokens $val:expr) => { $val };
}
```

- [ ] **Step 3: Write macro tests**

Add a test module in `crates/minibox-llm/src/lib.rs` (or add to chain.rs tests since they have mock providers):

Add to `crates/minibox-llm/src/chain.rs` test module:

```rust
#[tokio::test]
async fn ainvoke_macro_minimal() {
    let chain = FallbackChain::new(vec![Box::new(MockProvider {
        response: Ok("macro works".to_string()),
    })]);
    let resp = ainvoke!(chain, "test prompt").await.unwrap();
    assert_eq!(resp.text, "macro works");
}

#[tokio::test]
async fn ainvoke_macro_with_options() {
    let chain = FallbackChain::new(vec![Box::new(MockProvider {
        response: Ok("with opts".to_string()),
    })]);
    let resp = ainvoke!(chain, "test",
        system: "be helpful",
        max_tokens: 2048,
    ).await.unwrap();
    assert_eq!(resp.text, "with opts");
}

#[test]
fn invoke_macro_sync() {
    let chain = FallbackChain::new(vec![Box::new(MockProvider {
        response: Ok("sync macro".to_string()),
    })]);
    let resp = invoke!(chain, "test prompt").unwrap();
    assert_eq!(resp.text, "sync macro");
}
```

- [ ] **Step 4: Run tests to verify macros compile and work**

Run: `cargo test -p minibox-llm -- --nocapture`
Expected: All tests PASS including new macro tests.

- [ ] **Step 5: Run quality gates**

Run: `cargo xtask pre-commit`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/minibox-llm/src/lib.rs crates/minibox-llm/src/chain.rs
git commit -m "feat(minibox-llm): add invoke! and ainvoke! ergonomic macros"
```

---

### Task 7: Final quality gate and workspace check

**Files:**
- No new changes — verification only

- [ ] **Step 1: Run full crate tests**

Run: `cargo test -p minibox-llm -- --nocapture`
Expected: All tests PASS.

- [ ] **Step 2: Run workspace quality gates**

Run: `cargo xtask pre-commit`
Expected: fmt + clippy + release build all pass.

- [ ] **Step 3: Run unit test suite**

Run: `cargo xtask test-unit`
Expected: All unit tests across workspace pass. The new `CompletionRequest` fields don't break anything outside minibox-llm since no other crate constructs them directly.

- [ ] **Step 4: Verify test count**

Run: `cargo test -p minibox-llm -- --list 2>&1 | grep "test " | wc -l`
Expected: 27 tests (13 original + 4 error + 6 retry + 1 chain retry + 3 macro).

- [ ] **Step 5: Verify final `lib.rs` state**

The final `crates/minibox-llm/src/lib.rs` should look like this (verify by reading the file):

```rust
/// Generates `from_env()`, `from_env_with_config()`, and test-only `from_key()` for a provider.
macro_rules! provide {
    ($provider:ty, $env_var:expr, $default_model:expr) => {
        impl $provider {
            pub fn from_env() -> Option<Self> {
                Self::from_env_with_config(&$crate::ProviderConfig::default())
            }

            pub fn from_env_with_config(config: &$crate::ProviderConfig) -> Option<Self> {
                std::env::var($env_var)
                    .ok()
                    .map(|k| Self::with_config(k, $default_model.to_string(), config))
            }

            #[cfg(test)]
            pub(crate) fn from_key(key: String) -> Self {
                Self::new(key, $default_model.to_string())
            }
        }
    };
}

#[cfg(feature = "anthropic")]
pub mod anthropic;
pub mod chain;
pub mod error;
#[cfg(feature = "gemini")]
pub mod gemini;
#[cfg(feature = "openai")]
pub mod openai;
pub mod provider;
pub mod retry;
pub mod types;

pub use chain::FallbackChain;
pub use error::{HttpStatusError, LlmError};
pub use provider::{LlmProvider, ProviderConfig};
pub use retry::{RetryConfig, RetryingProvider};
pub use types::{CompletionRequest, CompletionResponse, JsonSchema, Usage};

/// Async LLM invocation macro (see Task 6 for definition)
// ainvoke! and invoke! macros defined via #[macro_export] above
```

- [ ] **Step 6: Commit spec status update**

Update the spec's status from "Draft" to "Implemented":

```bash
sed -i '' 's/^**Status**: Draft/**Status**: Implemented/' docs/superpowers/specs/2026-03-21-llm-timeouts-retries-design.md
git add docs/superpowers/specs/2026-03-21-llm-timeouts-retries-design.md
git commit -m "docs: mark llm timeouts spec as implemented"
```
