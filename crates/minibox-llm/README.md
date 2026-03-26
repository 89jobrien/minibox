# minibox-llm

Multi-provider LLM client with structured output support and fallback chains.

## Providers

| Provider | Feature | Models |
|----------|---------|--------|
| Claude (Anthropic) | `anthropic` | claude-opus-4-6, claude-sonnet-4-6, claude-haiku-4-5 |
| OpenAI | `openai` | gpt-4-turbo, gpt-4o, etc. |
| Google Gemini | `gemini` | gemini-2.0-flash, etc. |

## Usage

```rust
use minibox_llm::LlmClient;

let client = LlmClient::anthropic("your-api-key");
let response = client.complete("Explain containers").await?;
```

## Structured Output

Use `with_schema()` to define JSON schemas for responses, enabling type-safe extraction:

```rust
let schema = json!({"type": "object", "properties": {...}});
let response = client.with_schema(schema).complete(prompt).await?;
```

## Fallback Chains

Stack providers for automatic fallback:

```rust
let chain = vec![
    Box::new(LlmClient::anthropic(key1)),
    Box::new(LlmClient::openai(key2)),
];
```

Default features include all three providers. Disable individually as needed.
