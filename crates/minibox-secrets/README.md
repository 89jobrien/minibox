# minibox-secrets

Typed credential store with a provider chain, validation, and SHA-256 audit hashes. Secrets are wrapped in `secrecy::SecretString` — never exposed in logs or `Debug` output.

## Credential Kinds

| Kind           | Type          | Validation                                            |
| -------------- | ------------- | ----------------------------------------------------- |
| `api_key`      | `ApiKey`      | Non-empty, ≥8 chars, no whitespace                    |
| `token`        | `Token`       | Non-empty, optional expiry check                      |
| `database_url` | `DatabaseUrl` | Non-empty, scheme must be postgres/mysql/sqlite/redis |
| `ssh_key`      | `SshKey`      | PEM header with `PRIVATE KEY`                         |

Every credential stores a SHA-256 hex digest (`hash` field) that is safe to log, compare, and persist for audit trails.

## Providers

| Provider              | Scheme      | Ref format                      | Feature flag      |
| --------------------- | ----------- | ------------------------------- | ----------------- |
| `EnvProvider`         | `env:`      | `env:VAR_NAME:kind`             | default           |
| `KeyringProvider`     | `keyring:`  | `keyring:service/username:kind` | `keyring`         |
| `OnePasswordProvider` | `op://`     | `op://vault/item/field:kind`    | `onepassword`     |
| `BitwardenProvider`   | `bw:`       | `bw:item-name/field:kind`       | `bitwarden`       |
| `InMemoryProvider`    | `inmemory:` | `inmemory:key:kind`             | default (testing) |

All providers implement `CredentialProvider` — an async trait with a single method:

```rust
async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError>;
```

### Provider details

- **Env** — reads `std::env::var`; cache hint is `Never` (always re-reads).
- **1Password** — shells out to `op read`; requires an active session (biometric or `op signin`). 10s timeout, configurable. Cache hint: `Session`.
- **Bitwarden** — shells out to `bw get item`; requires `BW_SESSION` env var. Resolves custom `fields[]` first, falls back to `login.password`. 10s timeout, configurable. Cache hint: `Session`.
- **Keyring** — uses the `keyring` crate (macOS Keychain, libsecret, Windows Credential Store). Cache hint: `Session`.

## Provider Chain

`CredentialProviderChain` tries providers in order, returning the first successful + valid result:

```rust
use minibox_secrets::{CredentialProviderChain, CredentialRef};
use minibox_secrets::adapters::env::EnvProvider;

let chain = CredentialProviderChain::new(vec![
    Box::new(EnvProvider),
    // add more providers as fallbacks
]);

let cred_ref = CredentialRef::parse("env:MY_API_KEY:api_key").unwrap();
let fetched = chain.get(&cred_ref).await?;
```

### Cache behaviour

| `CacheHint` | Behaviour                                           |
| ----------- | --------------------------------------------------- |
| `Never`     | Always re-fetches (env vars)                        |
| `Session`   | Cached for process lifetime                         |
| `Until(t)`  | Cached until timestamp, then evicted and re-fetched |

Use `chain.invalidate("ref-string")` to evict a single entry or `chain.clear()` to flush the entire cache.

### Fallback logic

1. If a provider returns `NotFound`, try the next silently.
2. If a provider returns a credential that fails validation, log a warning and try the next.
3. If a provider is unavailable (CLI missing, vault locked), log a warning and try the next.
4. If all providers return `NotFound`, the chain returns `NotFound`.
5. If any provider returned a non-`NotFound` error, the chain returns the last such error.

## Feature Flags

```toml
[dependencies]
minibox-secrets = { path = "../minibox-secrets" }                        # env + in-memory only
minibox-secrets = { path = "../minibox-secrets", features = ["keyring"] } # + OS keychain
minibox-secrets = { path = "../minibox-secrets", features = ["onepassword", "bitwarden"] }
```
