# minibox-llm — LLM abstraction crate

**Date:** 2026-03-21
**Status:** Draft

## Purpose

Provider-agnostic LLM interface with async-first design, fallback chain, and structured JSONL output. Designed for minibox tooling (CI agent) but cleanly extractable for reuse across projects.

## Core trait

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;  // e.g. "anthropic/claude-sonnet-4-6"

    async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse>;
}
```

## Types

```rust
pub struct CompletionRequest {
    pub prompt: String,
    pub system: Option<String>,      // system prompt (mapped per-provider)
    pub max_tokens: u32,
    pub schema: Option<JsonSchema>,  // when set, provider returns structured JSON
}

pub struct CompletionResponse {
    pub text: String,
    pub provider: String,
    pub usage: Option<Usage>,
}

pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

pub struct JsonSchema {
    pub name: String,
    pub schema: serde_json::Value,  // JSON Schema object
}
```

## Providers

Three built-in providers, each behind a feature flag:

| Feature | Provider | Model | Auth | Structured output |
|---------|----------|-------|------|-------------------|
| `anthropic` | Anthropic | Claude Sonnet 4.6 | `x-api-key` header | tool_use with schema |
| `openai` | OpenAI | GPT-4.1 | `Bearer` auth | `response_format` with json_schema |
| `gemini` | Google | Gemini 2.5 Flash | `x-goog-api-key` header | `response_mime_type` + schema |

Each provider reads its API key from an environment variable (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`). Constructors return `None` if the key is not set — this is how the fallback chain knows which providers are available.

### Provider implementation pattern

Each provider module exposes:

```rust
pub struct AnthropicProvider { key: String, model: String }

impl AnthropicProvider {
    pub fn from_env() -> Option<Self>;  // None if ANTHROPIC_API_KEY unset
    pub fn new(key: String, model: String) -> Self;
}

impl LlmProvider for AnthropicProvider { ... }
```

### Structured output mapping

When `request.schema` is `Some(schema)`:

- **Anthropic:** Sends a single tool definition with the schema, forces tool use via `tool_choice: { type: "tool", name: schema.name }`. Extracts JSON from the tool call result.
- **OpenAI:** Sets `response_format: { type: "json_schema", json_schema: { name, schema, strict: true } }`. Extracts JSON from `choices[0].message.content`.
- **Gemini:** Sets `generationConfig.responseMimeType: "application/json"` and `generationConfig.responseSchema` to the schema. Extracts JSON from `candidates[0].content.parts[0].text`. **Note:** Gemini's `responseSchema` accepts a subset of JSON Schema (no `$schema`, `$ref`, `additionalProperties`, or other advanced keywords). The Gemini provider must sanitize the input `JsonSchema.schema` by stripping unsupported fields before sending.

When `request.schema` is `None`, all providers return plain text.

### System prompt mapping

When `request.system` is `Some(text)`:

- **Anthropic:** Sent as top-level `system` field (separate from `messages[]`).
- **OpenAI:** Sent as a `{ role: "system", content: text }` message prepended to `messages[]`.
- **Gemini:** Sent as `systemInstruction.parts[0].text`.

## Fallback chain

```rust
pub struct FallbackChain {
    providers: Vec<Box<dyn LlmProvider>>,
}

impl FallbackChain {
    /// Auto-discovers available providers by checking env vars.
    /// Order: Anthropic → OpenAI → Gemini. Providers without API keys
    /// are skipped (not added to the chain).
    pub fn from_env() -> Self;

    /// Uses a specific set of providers.
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self;

    /// Tries each provider in order. Logs and continues on failure.
    /// Returns AllProvidersFailed if none succeed.
    pub async fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse>;
}
```

### Error type

```rust
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("all providers failed: {0}")]
    AllProvidersFailed(String),

    #[error("provider {provider} failed: {source}")]
    ProviderError {
        provider: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("structured output failed to parse: {0}")]
    SchemaParseError(String),
}
```

## Sync wrapper

```rust
impl FallbackChain {
    /// Blocking wrapper for callers not in an async context.
    /// Uses a shared OnceLock<Runtime> to avoid creating a runtime per call.
    pub fn complete_sync(&self, request: &CompletionRequest) -> Result<CompletionResponse>;
}
```

Uses the same `OnceLock<Runtime>` pattern as the proptest suite in daemonbox. The `OnceLock<Runtime>` must be a module-level `static`, not a field on `FallbackChain`, to survive across calls from different chain instances.

## Dependencies

- `reqwest` — HTTP client (already in workspace)
- `serde`, `serde_json` — serialization, structured output schemas
- `tokio` — async runtime (already in workspace)
- `tracing` — provider skip/fail logging (already in workspace)
- `thiserror` — error types (already in workspace)
- `async-trait` — async trait support

All behind feature flags except `serde`, `tracing`, `thiserror` which are always on.

## Crate structure

```
crates/minibox-llm/
├── Cargo.toml
└── src/
    ├── lib.rs          // re-exports, public API surface
    ├── types.rs        // CompletionRequest, CompletionResponse, JsonSchema, Usage
    ├── provider.rs     // LlmProvider trait definition
    ├── anthropic.rs    // feature = "anthropic"
    ├── openai.rs       // feature = "openai"
    ├── gemini.rs       // feature = "gemini"
    └── chain.rs        // FallbackChain, sync wrapper, error types
```

## Feature flags

```toml
[features]
default = ["anthropic", "openai", "gemini"]
anthropic = ["dep:reqwest"]
openai = ["dep:reqwest"]
gemini = ["dep:reqwest"]

[dependencies]
reqwest = { workspace = true, optional = true }
```

Feature flags gate compilation of provider modules. `reqwest` is behind a `providers` feature (on by default) so the trait and types can be consumed without the HTTP dependency. When all provider features are disabled, only the trait, types, and chain logic are compiled — useful for consumers that supply their own `LlmProvider` implementation.

## Tracing contract

Follows minibox tracing conventions:

| Level | Event |
|-------|-------|
| `info!` | Provider selected, completion succeeded (provider name, token usage) |
| `warn!` | Provider skipped (no key), provider failed (error), falling back |
| `debug!` | Request details (model, max_tokens, schema name), response timing |

Fields: `provider`, `model`, `input_tokens`, `output_tokens`, `elapsed_ms`, `error`.

## Testing strategy

- **Unit tests per provider:** Mock HTTP responses using a test helper that implements `LlmProvider` returning canned responses. Verify request serialization by capturing the built request body.
- **Fallback chain tests:** Compose chains with mock providers that fail/succeed in various orders. Verify skip-on-no-key, retry-on-error, all-failed error.
- **Structured output tests:** Verify each provider correctly wraps/unwraps JSON schema in its provider-specific format.
- **Sync wrapper test:** Verify `complete_sync` works from a non-async context.

No live API calls in tests — all mocked.

## Workspace integration

Add to `Cargo.toml` workspace members. Add to clippy and fmt targets in CLAUDE.md macOS quality gates. The crate is a library — no binary.

New `Cargo.toml` must inherit workspace settings: `edition.workspace = true`, `version.workspace = true`, `license.workspace = true` (required for `cargo deny check` CI gate).

Provider implementations must catch `reqwest::Error` internally and wrap it in `ProviderError { provider, source: Box::new(e) }` — HTTP errors should never leak as bare errors, so the chain always knows which provider failed.
