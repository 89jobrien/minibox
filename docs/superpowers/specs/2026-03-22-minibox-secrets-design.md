# minibox-secrets — Design Spec

**Date:** 2026-03-22
**Status:** Draft

---

## Overview

`minibox-secrets` is a local-only credential validation and passthrough library. It fetches
credentials from local providers (env vars, OS keychain, 1Password CLI, Bitwarden CLI),
validates their structure, and returns typed structs safe for use in downstream code.

No network calls are made by the crate itself. Provider CLIs (`op`, `bw`) manage their
own vault communication.

---

## Goals

- Typed, validated credential structs with SHA-256 hash for cache invalidation and audit logging
- `CredentialProvider` async trait (port) — swap providers without touching callers
- `CredentialProviderChain` — ranked fallback across providers with expiry-aware caching
- All secret values wrapped in `secrecy::SecretString` — never logged, zeroed on drop
- Local-only: `Env → Keyring → OnePassword → Bitwarden`

## Non-Goals

- No user-facing authentication (no `UserPass` credential type)
- No direct network calls or remote secret manager SDKs
- No Dockerfile / container credential injection

---

## Architecture

Hexagonal architecture: domain layer has zero external deps beyond `secrecy`, `sha2`, `hex`,
`chrono`, `thiserror`; adapters implement the port. `anyhow` is for adapter internals only
and must not appear in domain types.

```
┌─────────────────────────────────────────────┐
│            Composition Root                 │
│  (caller wires chain: Env→Keyring→Op→Bw)   │
└──────────────┬──────────────────────────────┘
               │
    ┌──────────┴──────────┐
    │                     │
┌───▼──────────────┐  ┌───▼──────────────────────────────────────┐
│  Domain Layer    │  │  Adapters                                │
│  (domain.rs)     │  │                                          │
│                  │  │  EnvProvider          (always)           │
│  Credential      │  │  KeyringProvider      feature=keyring    │
│  ApiKey          │  │  OnePasswordProvider  feature=onepassword│
│  Token           │  │  BitwardenProvider    feature=bitwarden  │
│  DatabaseUrl     │  │  InMemoryProvider     always (tests)     │
│  SshKey          │  │                                          │
│                  │  │  CredentialProviderChain                 │
│  CredentialRef   │  │   (impl CredentialProvider)              │
│  FetchedCred     │  └──────────────────────────────────────────┘
│  CacheHint       │
│                  │
│  Validate trait  │
│  CredentialProvider trait (async)           │
└──────────────────┘
```

Dependencies point inward: adapters → domain.

---

## Crate Layout

```
crates/minibox-secrets/
├── Cargo.toml
└── src/
    ├── lib.rs              # pub re-exports only
    ├── domain.rs           # traits, credential types, errors
    ├── validation.rs       # Validate impls per credential kind
    └── adapters/
        ├── mod.rs
        ├── in_memory.rs    # InMemoryProvider (HashMap)
        ├── env.rs          # EnvProvider
        ├── keyring.rs      # KeyringProvider (feature = "keyring")
        ├── onepassword.rs  # OnePasswordProvider (feature = "onepassword")
        └── bitwarden.rs    # BitwardenProvider (feature = "bitwarden")
```

---

## Domain Layer

### Credential types

Each struct stores the secret value and a SHA-256 hex hash of that value. The hash is safe
to log, compare, and use for cache invalidation without exposing the secret.

The hash is computed from the raw value before wrapping in `SecretString`:

```rust
pub struct ApiKey {
    pub key: SecretString,   // e.g. "sk-abc123..."
    pub hash: String,        // SHA-256 hex of key — safe to log
}

pub struct Token {
    pub token: SecretString,
    pub hash: String,
    pub expires_at: Option<DateTime<Utc>>,
}

pub struct DatabaseUrl {
    pub url: SecretString,   // "postgres://user:pass@host/db"
    pub hash: String,
}

pub struct SshKey {
    pub private_key: SecretString,  // PEM block (encrypted or plaintext)
    pub hash: String,
}
```

**No `Serialize`/`Deserialize` derives on any credential type.** `secrecy::SecretString`
intentionally prevents serialization; `Credential` types must not derive `serde::Serialize`
either. `Deserialize` is opt-in only if a specific struct explicitly needs it (e.g. loading
from a config file), and only with the `#[serde(deny_unknown_fields)]` guard.

Each type exposes a constructor:

```rust
impl ApiKey {
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let hash = sha256_hex(raw.as_bytes()); // pure fn in domain.rs
        Self { key: SecretString::new(raw.into()), hash }
    }
}
```

Unified enum returned by providers:

```rust
pub enum Credential {
    ApiKey(ApiKey),
    Token(Token),
    DatabaseUrl(DatabaseUrl),
    SshKey(SshKey),
}
```

### CredentialRef

A typed, parsed reference to a credential. The constructor validates the scheme prefix at
parse time; a malformed ref fails early rather than at `get()`.

```rust
pub struct CredentialRef {
    pub scheme: CredentialScheme,
    pub path: String,              // everything after the scheme prefix
    pub kind: CredentialKind,      // expected credential type
}

pub enum CredentialScheme {
    Env,
    Keyring,
    OnePassword,
    Bitwarden,
    InMemory,
}

pub enum CredentialKind {
    ApiKey,
    Token,
    DatabaseUrl,
    SshKey,
}

impl CredentialRef {
    /// Parse a ref string. Validates scheme prefix and kind suffix.
    ///
    /// Format: `<scheme>:<path>:<kind>`
    /// Examples:
    ///   `env:MY_API_KEY:api_key`
    ///   `keyring:service/username:api_key`
    ///   `op://vault/item/field:api_key`
    ///   `bw:item-name/field:api_key`
    pub fn parse(s: &str) -> Result<Self, CredentialError> { ... }
}
```

The `kind` suffix determines which `Credential` variant the provider constructs. This
eliminates the ambiguity of a provider always returning `ApiKey`.

### CacheHint and FetchedCredential

```rust
pub enum CacheHint {
    Never,               // always re-fetch (env vars)
    Session,             // valid until process exits
    Until(DateTime<Utc>),
}

/// Wrapped in Arc so it can be cached and returned without Clone on SecretString.
pub type FetchedCredential = Arc<FetchedCredentialInner>;

pub struct FetchedCredentialInner {
    pub credential: Credential,
    pub cache: CacheHint,
}
```

`Arc` allows the cache to hold a `FetchedCredential` and return a clone of the `Arc` to
callers — no `Clone` on `SecretString` required.

### Traits

**`CredentialProvider` is async.** The workspace uses Tokio throughout; blocking CLI calls
(`Command::output()`) must not block a tokio worker. Adapters call
`tokio::task::spawn_blocking` internally; callers `.await` the trait method.

```rust
use async_trait::async_trait;

#[async_trait]
pub trait CredentialProvider: Send + Sync {
    async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError>;
}

pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}
```

### Error types

`CredentialError::Expired` is removed. Token expiry flows through
`Validation(ValidationError::Expired)` only — no duplicate path.

```rust
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("field `{field}` is empty")]
    Empty { field: &'static str },
    #[error("credential has expired")]
    Expired,
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("credential not found: {0}")]
    NotFound(String),
    #[error("provider unavailable: {0}")]   // CLI not installed, vault locked
    ProviderUnavailable(String),
    #[error("invalid format: {0}")]         // fetched but unparseable
    InvalidFormat(String),
    #[error("validation failed: {0}")]
    Validation(#[from] ValidationError),
}
```

---

## Validation Rules

| Type          | Rules                                                                               |
|---------------|-------------------------------------------------------------------------------------|
| `ApiKey`      | Non-empty, ≥ 8 chars, no ASCII whitespace                                          |
| `Token`       | Non-empty; if `expires_at` is set and in the past → `ValidationError::Expired`    |
| `DatabaseUrl` | Parses as URL; scheme ∈ `{postgres, postgresql, mysql, sqlite, redis}`             |
| `SshKey`      | Starts with `-----BEGIN`, contains `PRIVATE KEY` (encrypted keys are accepted;     |
|               | decryption is the caller's responsibility)                                         |

Validation is called by the chain after each successful `get()`. A credential that fails
validation causes the chain to try the next provider.

---

## CredentialProviderChain

```rust
pub struct CredentialProviderChain {
    providers: Vec<Box<dyn CredentialProvider>>,
    cache: tokio::sync::Mutex<HashMap<String, (FetchedCredential, Instant)>>,
}
```

`cache` key is the raw ref string. The cache stores `Arc<FetchedCredentialInner>` (cheap
to clone and return).

Algorithm:

1. **Cache check**: look up the ref string.
   - If `CacheHint::Never` → skip cache, go to step 2.
   - If `CacheHint::Session` → return cached value if present.
   - If `CacheHint::Until(t)` → return cached value if `Utc::now() < t`; evict and
     continue otherwise.
2. **Provider loop**: try each provider in order.
   - On `NotFound` or `ProviderUnavailable` → try next.
   - On `InvalidFormat` or `Validation(_)` → try next (log at `warn!` level).
   - On first success: validate the credential (call `validate()`); if validation fails,
     treat as `Validation` error and try next.
3. **Cache store**: if `CacheHint` is not `Never`, insert into cache.
4. **Result**: return the first success, or the last non-`NotFound` error, or `NotFound`
   if all providers returned `NotFound`.

Default chain construction helper:

```rust
impl CredentialProviderChain {
    /// Env → Keyring → OnePassword → Bitwarden
    pub fn default_local() -> Self { ... }
}
```

---

## Adapters

All CLI-based adapters use `tokio::task::spawn_blocking(|| Command::new(...).output())`.

### EnvProvider

- Ref format: `env:<VAR_NAME>:<kind>`
- `std::env::var(VAR_NAME)`, maps `CredentialKind` to the correct `Credential` variant
- `CacheHint::Never`

### KeyringProvider (`feature = "keyring"`)

- Ref format: `keyring:<service>/<username>:<kind>`
- `keyring::Entry::new(service, username).get_password()`
- Maps `CredentialKind` to the correct `Credential` variant
- `CacheHint::Session`
- `spawn_blocking` wraps the blocking `get_password()` call

### OnePasswordProvider (`feature = "onepassword"`)

- Ref format: `op://vault/item/field:<kind>` (the `op://` portion is passed verbatim to
  `op read`)
- `Command::new("op").args(["read", op_ref])` inside `spawn_blocking`
- Stderr containing "not signed in" or "unauthorized" → `ProviderUnavailable`
- Non-zero exit, item not found → `NotFound`
- Strips trailing whitespace from stdout; constructs `Credential` from `CredentialKind`
- `CacheHint::Session`
- Type detection is driven entirely by the `kind` field in the parsed `CredentialRef`,
  not by field name heuristics. A pure `fn build_credential(kind, raw) -> Credential`
  handles construction and is unit-testable without shelling out.

### BitwardenProvider (`feature = "bitwarden"`)

- Ref format: `bw:<item-name>/<field>:<kind>`
- `Command::new("bw").args(["get", "item", item_name])` inside `spawn_blocking`
- Exit code 1 with "Vault is locked" in stderr → `ProviderUnavailable`
- Exit code 1 with "Not found" in stderr → `NotFound`
- Parses stdout as JSON (`serde_json::Value`):
  1. Look for `fields` array; find entry where `name == field`; use `.value`
  2. If no `fields` array or field not found, fall back to `login.password`
  3. If neither present → `InvalidFormat`
- Constructs `Credential` from `CredentialKind` (same pure `build_credential` fn)
- `BW_SESSION` must be set in the environment by the caller; the adapter does not manage
  Bitwarden session tokens
- `CacheHint::Session`

### InMemoryProvider

- `HashMap<String, FetchedCredential>` keyed by ref string
- Used in tests; callers insert entries directly
- Always returns `CacheHint::Never`

---

## Workspace Integration

### `Cargo.toml` additions

```toml
# workspace root [workspace.dependencies]
minibox-secrets = { path = "crates/minibox-secrets" }
secrecy  = "0.10"
keyring  = { version = "3", optional = true }
```

```toml
# crates/minibox-secrets/Cargo.toml
[dependencies]
secrecy       = { workspace = true }
sha2          = { workspace = true }
hex           = { workspace = true }
chrono        = { workspace = true }
thiserror     = { workspace = true }
tokio         = { workspace = true }      # for spawn_blocking + Mutex in chain
async-trait   = { workspace = true }
serde_json    = { workspace = true }      # bitwarden JSON parsing

# anyhow is for adapter internals only — not used in domain types or errors
anyhow        = { workspace = true }

[dependencies.keyring]
workspace = true
optional  = true

[features]
default      = []
keyring      = ["dep:keyring"]
onepassword  = []
bitwarden    = []
```

---

## Testing Strategy

- **Unit tests** (`validation.rs`): each `Validate` impl with valid and invalid inputs,
  including empty string, expired token, bad URL scheme, truncated PEM, encrypted PEM
- **`build_credential` pure fn** (`domain.rs`): unit-tested for every `CredentialKind`,
  no I/O required
- **`CredentialRef::parse`**: unit-tested for valid refs, bad schemes, missing kind suffix
- **Chain tests** (`in_memory.rs`): resolution order, `CacheHint::Never` bypass, `Session`
  hit, `Until` expiry eviction, all-`NotFound` propagation
- **Provider smoke tests** (`#[ignore]`): require live `op` / `bw` / keyring entry; skipped
  in CI, run manually
- No mocking of `std::process::Command` — integration tests against real CLIs only

---

## Security Notes

- `SecretString` prevents `Debug`/`Display` leaks; `.expose_secret()` is the only access
  point and is grep-able for audit
- SHA-256 hash computed from raw value before wrapping — hex string stored, safe to log
- No credential value is written to disk by this crate
- `Credential` types must not derive `Serialize` (see Domain Layer section)
- `BW_SESSION` managed by caller, not this crate
- `op` relies on the system agent / biometric unlock, not managed by this crate
- Encrypted SSH private keys pass validation; decryption is the caller's responsibility
