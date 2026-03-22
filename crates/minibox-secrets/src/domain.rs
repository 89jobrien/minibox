use chrono::{DateTime, Utc};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("field `{field}` is empty")]
    Empty { field: &'static str },
    #[error("credential has expired")]
    Expired,
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("credential not found: {0}")]
    NotFound(String),
    /// Provider CLI not installed, vault locked, session expired.
    #[error("provider unavailable: {0}")]
    ProviderUnavailable(String),
    /// Value fetched but could not be parsed into the expected type.
    #[error("invalid format: {0}")]
    InvalidFormat(String),
    #[error("validation failed: {0}")]
    Validation(#[from] ValidationError),
}

// ---------------------------------------------------------------------------
// CredentialKind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    ApiKey,
    Token,
    DatabaseUrl,
    SshKey,
}

impl CredentialKind {
    pub fn from_str(s: &str) -> Result<Self, CredentialError> {
        match s {
            "api_key" => Ok(Self::ApiKey),
            "token" => Ok(Self::Token),
            "database_url" => Ok(Self::DatabaseUrl),
            "ssh_key" => Ok(Self::SshKey),
            other => Err(CredentialError::InvalidFormat(format!(
                "unknown credential kind `{other}`; expected api_key, token, database_url, ssh_key"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// CredentialScheme
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialScheme {
    Env,
    Keyring,
    OnePassword,
    Bitwarden,
    InMemory,
}

// ---------------------------------------------------------------------------
// CredentialRef
// ---------------------------------------------------------------------------

/// A parsed, validated reference to a credential.
///
/// Format: `<scheme>:<path>:<kind>`
///
/// Examples:
/// - `env:MY_API_KEY:api_key`
/// - `keyring:my-service/username:token`
/// - `op://vault/item/field:api_key`
/// - `bw:item-name/field:database_url`
/// - `inmemory:my-key:ssh_key`
#[derive(Debug, Clone)]
pub struct CredentialRef {
    pub scheme: CredentialScheme,
    /// Everything between the scheme prefix and the `:kind` suffix.
    pub path: String,
    pub kind: CredentialKind,
    /// Original raw string, preserved for cache keying.
    raw: String,
}

impl CredentialRef {
    /// Parse and validate a credential reference string.
    pub fn parse(s: &str) -> Result<Self, CredentialError> {
        // The kind is always the last colon-delimited segment.
        let (prefix, kind_str) = s.rsplit_once(':').ok_or_else(|| {
            CredentialError::InvalidFormat(format!(
                "missing kind suffix in ref `{s}`; expected `<scheme>:<path>:<kind>`"
            ))
        })?;

        let kind = CredentialKind::from_str(kind_str)?;

        let (scheme, path) = if let Some(p) = prefix.strip_prefix("op://") {
            (CredentialScheme::OnePassword, p.to_string())
        } else if let Some(p) = prefix.strip_prefix("env:") {
            (CredentialScheme::Env, p.to_string())
        } else if let Some(p) = prefix.strip_prefix("keyring:") {
            (CredentialScheme::Keyring, p.to_string())
        } else if let Some(p) = prefix.strip_prefix("bw:") {
            (CredentialScheme::Bitwarden, p.to_string())
        } else if let Some(p) = prefix.strip_prefix("inmemory:") {
            (CredentialScheme::InMemory, p.to_string())
        } else {
            return Err(CredentialError::InvalidFormat(format!(
                "unknown scheme in ref `{prefix}`; expected env:, keyring:, op://, bw:, inmemory:"
            )));
        };

        Ok(Self {
            scheme,
            path,
            kind,
            raw: s.to_string(),
        })
    }

    /// The original string, used as a cache key.
    pub fn as_str(&self) -> &str {
        &self.raw
    }
}

// ---------------------------------------------------------------------------
// Credential types
// ---------------------------------------------------------------------------

/// SHA-256 hex digest of a raw secret value. Safe to log and compare.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub struct ApiKey {
    pub key: SecretString,
    /// SHA-256 hex of `key` — safe to log.
    pub hash: String,
}

impl ApiKey {
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let hash = sha256_hex(raw.as_bytes());
        Self {
            key: SecretString::from(raw),
            hash,
        }
    }
}

pub struct Token {
    pub token: SecretString,
    pub hash: String,
    pub expires_at: Option<DateTime<Utc>>,
}

impl Token {
    pub fn new(raw: impl Into<String>, expires_at: Option<DateTime<Utc>>) -> Self {
        let raw = raw.into();
        let hash = sha256_hex(raw.as_bytes());
        Self {
            token: SecretString::from(raw),
            hash,
            expires_at,
        }
    }
}

pub struct DatabaseUrl {
    pub url: SecretString,
    pub hash: String,
}

impl DatabaseUrl {
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let hash = sha256_hex(raw.as_bytes());
        Self {
            url: SecretString::from(raw),
            hash,
        }
    }
}

pub struct SshKey {
    pub private_key: SecretString,
    pub hash: String,
}

impl SshKey {
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let hash = sha256_hex(raw.as_bytes());
        Self {
            private_key: SecretString::from(raw),
            hash,
        }
    }
}

/// Unified credential enum returned by all providers.
pub enum Credential {
    ApiKey(ApiKey),
    Token(Token),
    DatabaseUrl(DatabaseUrl),
    SshKey(SshKey),
}

/// Construct a `Credential` of the given kind from a raw string value.
/// Pure function — unit-testable without I/O.
pub fn build_credential(kind: CredentialKind, raw: String) -> Credential {
    match kind {
        CredentialKind::ApiKey => Credential::ApiKey(ApiKey::new(raw)),
        CredentialKind::Token => Credential::Token(Token::new(raw, None)),
        CredentialKind::DatabaseUrl => Credential::DatabaseUrl(DatabaseUrl::new(raw)),
        CredentialKind::SshKey => Credential::SshKey(SshKey::new(raw)),
    }
}

// ---------------------------------------------------------------------------
// CacheHint + FetchedCredential
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CacheHint {
    /// Never cache — always re-fetch (e.g. env vars).
    Never,
    /// Valid for the lifetime of the process.
    Session,
    /// Valid until the given timestamp.
    Until(DateTime<Utc>),
}

pub struct FetchedCredentialInner {
    pub credential: Credential,
    pub cache: CacheHint,
}

/// `Arc` allows the chain cache to hold and return values without `Clone` on `SecretString`.
pub type FetchedCredential = Arc<FetchedCredentialInner>;

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}

#[async_trait::async_trait]
pub trait CredentialProvider: Send + Sync {
    async fn get(&self, r: &CredentialRef) -> Result<FetchedCredential, CredentialError>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_ref() {
        let r = CredentialRef::parse("env:MY_KEY:api_key").unwrap();
        assert_eq!(r.scheme, CredentialScheme::Env);
        assert_eq!(r.path, "MY_KEY");
        assert_eq!(r.kind, CredentialKind::ApiKey);
    }

    #[test]
    fn parse_op_ref() {
        let r = CredentialRef::parse("op://vault/item/field:token").unwrap();
        assert_eq!(r.scheme, CredentialScheme::OnePassword);
        assert_eq!(r.path, "vault/item/field");
        assert_eq!(r.kind, CredentialKind::Token);
    }

    #[test]
    fn parse_bw_ref() {
        let r = CredentialRef::parse("bw:my-item/password:database_url").unwrap();
        assert_eq!(r.scheme, CredentialScheme::Bitwarden);
        assert_eq!(r.kind, CredentialKind::DatabaseUrl);
    }

    #[test]
    fn parse_keyring_ref() {
        let r = CredentialRef::parse("keyring:my-service/user:ssh_key").unwrap();
        assert_eq!(r.scheme, CredentialScheme::Keyring);
        assert_eq!(r.kind, CredentialKind::SshKey);
    }

    #[test]
    fn parse_missing_kind_fails() {
        assert!(CredentialRef::parse("env:MY_KEY").is_err());
    }

    #[test]
    fn parse_unknown_scheme_fails() {
        assert!(CredentialRef::parse("vault:MY_KEY:api_key").is_err());
    }

    #[test]
    fn parse_unknown_kind_fails() {
        assert!(CredentialRef::parse("env:MY_KEY:password").is_err());
    }

    #[test]
    fn build_credential_api_key() {
        let c = build_credential(CredentialKind::ApiKey, "sk-test1234".into());
        assert!(matches!(c, Credential::ApiKey(_)));
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        assert_eq!(sha256_hex(b"hello"), sha256_hex(b"hello"));
        assert_ne!(sha256_hex(b"hello"), sha256_hex(b"world"));
    }

    #[test]
    fn api_key_hash_stored() {
        let k = ApiKey::new("sk-12345678");
        assert_eq!(k.hash, sha256_hex(b"sk-12345678"));
    }
}
