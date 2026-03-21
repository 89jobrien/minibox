# minibox-llm: HTTP Timeouts & Transient Retry Design

**Date**: 2026-03-21
**Status**: Draft
**Scope**: `crates/minibox-llm`

## Problem

All three LLM providers (`AnthropicProvider`, `OpenAiProvider`, `GeminiProvider`) construct `reqwest::Client::new()` with no timeout configuration. Reqwest 0.12 has no default timeout, so a stalled API call hangs indefinitely and the fallback chain cannot advance. There is no retry logic for transient failures (timeouts, 429, 503).

## Design Decisions

1. **Hybrid architecture** — reqwest timeouts live in providers (HTTP concern), retry/backoff lives in a `RetryingProvider` wrapper (cross-cutting concern). Clean layer separation following existing hexagonal patterns.
2. **Transient vs. permanent error classification** — retry transient errors (timeouts, connection failures, 429/5xx) on the same provider with exponential backoff before falling through. Permanent errors (401, 400, schema parse) fall through immediately.
3. **Per-request overrides** — `CompletionRequest` gains optional `timeout` and `max_retries` fields. Provider defaults apply when unset.
4. **`provide!` macro** — eliminates duplicated `from_env` / `from_env_with_key` boilerplate across all three providers.
5. **`invoke!` / `ainvoke!` macros** — ergonomic request construction and dispatch. `invoke!` is sync (blocking), `ainvoke!` is async.

## New Types

### `ProviderConfig`

HTTP-level defaults set at provider construction time.

```rust
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub connect_timeout: Duration,  // default: 10s
    pub request_timeout: Duration,  // default: 60s
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

### `RetryConfig`

Retry behavior for the `RetryingProvider` wrapper.

```rust
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,        // default: 2
    pub backoff_base: Duration,  // default: 1s (exponential: 1s, 2s)
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 2,
            backoff_base: Duration::from_secs(1),
        }
    }
}
```

### `HttpStatusError`

Typed error for HTTP status codes, enabling `is_transient()` classification via downcasting.

```rust
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

### `CompletionRequest` additions

```rust
pub struct CompletionRequest {
    pub prompt: String,
    pub system: Option<String>,
    pub max_tokens: u32,
    pub schema: Option<JsonSchema>,
    pub timeout: Option<Duration>,     // overrides provider's request_timeout
    pub max_retries: Option<u32>,      // overrides RetryConfig max_retries
}
```

Both fields are `Option` — callers that don't care about overrides change nothing.

## Transient Error Classification

New method on `LlmError`:

```rust
impl LlmError {
    pub fn is_transient(&self) -> bool {
        match self {
            LlmError::ProviderError { source, .. } => {
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

`reqwest_err.is_request()` is broad but safe — our URLs are hardcoded constants, so it only fires on genuine network issues in practice.

## Provider Changes

### `provide!` macro

Replaces hand-written `from_env()` / `from_env_with_key()` across all three providers:

```rust
macro_rules! provide {
    ($provider:ty, $env_var:expr, $default_model:expr) => {
        impl $provider {
            pub fn from_env() -> Option<Self> {
                Self::from_env_with_config(
                    std::env::var($env_var).ok(),
                    &ProviderConfig::default(),
                )
            }

            pub fn from_env_with_config(
                key: Option<String>,
                config: &ProviderConfig,
            ) -> Option<Self> {
                key.map(|k| Self::with_config(k, $default_model.to_string(), config))
            }
        }
    };
}

provide!(AnthropicProvider, "ANTHROPIC_API_KEY", "claude-sonnet-4-6");
provide!(OpenAiProvider, "OPENAI_API_KEY", "gpt-4.1");
provide!(GeminiProvider, "GEMINI_API_KEY", "gemini-2.5-flash");
```

### Constructor changes

Each provider gains `with_config()` that builds a timeout-configured reqwest client. Existing `new()` becomes a convenience:

```rust
impl AnthropicProvider {
    pub fn new(key: String, model: String) -> Self {
        Self::with_config(key, model, &ProviderConfig::default())
    }

    pub fn with_config(key: String, model: String, config: &ProviderConfig) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .build()
            .expect("failed to build reqwest client");
        Self {
            key,
            model,
            display_name: format!("anthropic/{model}"),
            client,
        }
    }
}
```

Same pattern for `OpenAiProvider` and `GeminiProvider`.

### Per-request timeout override

Each provider's `complete()` applies the request-level timeout override on the `RequestBuilder`:

```rust
let mut req = self.client.post(url).headers(headers).json(&body);
if let Some(t) = request.timeout {
    req = req.timeout(t);
}
```

### Explicit HTTP status checking

Providers replace `error_for_status()` with explicit status checking to produce typed `HttpStatusError`:

```rust
let status = response.status();
if !status.is_success() {
    let body = response.text().await.unwrap_or_default();
    return Err(LlmError::ProviderError {
        provider: self.name().to_string(),
        source: Box::new(HttpStatusError {
            status: status.as_u16(),
            body,
        }),
    });
}
```

## `RetryingProvider` Wrapper

New file: `retry.rs`

```rust
pub struct RetryingProvider<P: LlmProvider> {
    inner: P,
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
        unreachable!("loop always returns")
    }
}
```

Backoff is capped at 30s via `.min(backoff_cap)` and uses `saturating_pow` to prevent overflow.

## Chain Wiring

`FallbackChain::from_env()` wraps each provider in `RetryingProvider`:

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
        if let Some(p) = AnthropicProvider::from_env_with_config(
            std::env::var("ANTHROPIC_API_KEY").ok(),
            &provider_config,
        ) {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(RetryingProvider::new(p, retry_config.clone())));
        } else {
            tracing::warn!(provider = "anthropic", "llm: provider skipped (no key)");
        }

        #[cfg(feature = "openai")]
        if let Some(p) = OpenAiProvider::from_env_with_config(
            std::env::var("OPENAI_API_KEY").ok(),
            &provider_config,
        ) {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(RetryingProvider::new(p, retry_config.clone())));
        } else {
            tracing::warn!(provider = "openai", "llm: provider skipped (no key)");
        }

        #[cfg(feature = "gemini")]
        if let Some(p) = GeminiProvider::from_env_with_config(
            std::env::var("GEMINI_API_KEY").ok(),
            &provider_config,
        ) {
            tracing::info!(provider = p.name(), "llm: provider available");
            providers.push(Box::new(RetryingProvider::new(p, retry_config.clone())));
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

## `invoke!` / `ainvoke!` Macros

Ergonomic macros for constructing and dispatching LLM requests. Both share the same request-building logic internally.

### `ainvoke!` (async)

```rust
// Minimal — just prompt, defaults for everything else
let resp = ainvoke!(chain, "Summarize this code").await?;

// With options
let resp = ainvoke!(chain, "Summarize this code",
    system: "You are a code reviewer",
    max_tokens: 2048,
    timeout: Duration::from_secs(90),
    max_retries: 1,
    schema: my_schema,
).await?;
```

### `invoke!` (sync, blocking)

```rust
let resp = invoke!(chain, "Summarize this code")?;

let resp = invoke!(chain, "What is this?",
    system: "Be concise",
    max_tokens: 512,
)?;
```

### Macro expansion

Both macros expand to `CompletionRequest` construction with defaults for omitted fields:

```rust
macro_rules! ainvoke {
    ($chain:expr, $prompt:expr $(, $key:ident : $val:expr)* $(,)?) => {{
        let request = $crate::CompletionRequest {
            prompt: $prompt.into(),
            system: None,
            max_tokens: 1024,
            schema: None,
            timeout: None,
            max_retries: None,
            $( $key: ainvoke!(@field $key $val), )*
        };
        $chain.complete(&request)
    }};
    (@field system $val:expr) => { Some($val.into()) };
    (@field schema $val:expr) => { Some($val) };
    (@field timeout $val:expr) => { Some($val) };
    (@field max_retries $val:expr) => { Some($val) };
    (@field max_tokens $val:expr) => { $val };
}
```

`invoke!` is identical but calls `$chain.complete_sync(&request)` instead.

### Defaults when omitted

| Field | Default |
|-------|---------|
| `system` | `None` |
| `max_tokens` | `1024` |
| `schema` | `None` |
| `timeout` | `None` (use provider default) |
| `max_retries` | `None` (use retry config default) |

## File Change Summary

| File | Change |
|------|--------|
| `types.rs` | Add `timeout: Option<Duration>`, `max_retries: Option<u32>` to `CompletionRequest` |
| `error.rs` | Add `HttpStatusError` struct, `is_transient()` method on `LlmError` |
| `provider.rs` | Add `ProviderConfig`, `RetryConfig` types with `Default` impls |
| `retry.rs` (new) | `RetryingProvider<P>` wrapper implementing `LlmProvider` |
| `anthropic.rs` | `with_config()` constructor, explicit status checking, per-request timeout |
| `openai.rs` | Same pattern |
| `gemini.rs` | Same pattern |
| `chain.rs` | `from_env_with_config()`, wraps providers in `RetryingProvider` |
| `lib.rs` | `provide!`, `invoke!`, `ainvoke!` macro definitions, export new public types, declare `retry` module |

## Worst-Case Latency

With defaults (60s timeout, 2 retries per provider, 3 providers):

- Per provider: 60s + 1s + 60s + 2s + 60s = ~183s worst case
- Full chain: ~549s (~9 min) if every attempt on every provider times out

This is by design — callers opting into 3 providers with retries accept the tail latency. Per-request `timeout` and `max_retries` overrides allow callers to bound this for latency-sensitive paths.

## Testing Strategy

- **Unit tests**: Mock `LlmProvider` that returns transient errors N times then succeeds — verify retry count and that backoff delays are applied
- **Classification tests**: Construct `LlmError::ProviderError` with `HttpStatusError` at various status codes — verify `is_transient()` returns correctly
- **Integration with chain**: Mock providers where first returns permanent error, second returns transient-then-success — verify fallback + retry interaction
- **Existing tests**: Update `CompletionRequest` construction to include `timeout: None, max_retries: None`
