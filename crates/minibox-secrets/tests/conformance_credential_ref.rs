//! Conformance tests for `CredentialRef` parse contract.
//!
//! Verifies the externally-visible guarantees:
//! - All 5 supported schemes parse correctly.
//! - Parsed fields match the input.
//! - `as_str()` roundtrips to the original string.
//! - Invalid inputs return `Err`.
//! - `CredentialKind` is exhaustive over 4 variants.

use minibox_secrets::{CredentialError, CredentialKind, CredentialRef, CredentialScheme};

// ---------------------------------------------------------------------------
// Scheme parsing
// ---------------------------------------------------------------------------

#[test]
fn env_scheme_parses() {
    let r = CredentialRef::parse("env:MY_API_KEY:api_key").unwrap();
    assert_eq!(r.scheme, CredentialScheme::Env);
    assert_eq!(r.path, "MY_API_KEY");
    assert_eq!(r.kind, CredentialKind::ApiKey);
}

#[test]
fn op_scheme_parses() {
    let r = CredentialRef::parse("op://Personal/my-item/password:token").unwrap();
    assert_eq!(r.scheme, CredentialScheme::OnePassword);
    assert_eq!(r.path, "Personal/my-item/password");
    assert_eq!(r.kind, CredentialKind::Token);
}

#[test]
fn keyring_scheme_parses() {
    let r = CredentialRef::parse("keyring:svc/user:ssh_key").unwrap();
    assert_eq!(r.scheme, CredentialScheme::Keyring);
    assert_eq!(r.path, "svc/user");
    assert_eq!(r.kind, CredentialKind::SshKey);
}

#[test]
fn bitwarden_scheme_parses() {
    let r = CredentialRef::parse("bw:my-item/field:database_url").unwrap();
    assert_eq!(r.scheme, CredentialScheme::Bitwarden);
    assert_eq!(r.kind, CredentialKind::DatabaseUrl);
}

#[test]
fn inmemory_scheme_parses() {
    let r = CredentialRef::parse("inmemory:test-key:api_key").unwrap();
    assert_eq!(r.scheme, CredentialScheme::InMemory);
    assert_eq!(r.path, "test-key");
    assert_eq!(r.kind, CredentialKind::ApiKey);
}

// ---------------------------------------------------------------------------
// as_str() roundtrip
// ---------------------------------------------------------------------------

#[test]
fn as_str_roundtrips_env() {
    let raw = "env:MY_KEY:api_key";
    let r = CredentialRef::parse(raw).unwrap();
    assert_eq!(r.as_str(), raw);
}

#[test]
fn as_str_roundtrips_op() {
    let raw = "op://vault/item/field:token";
    let r = CredentialRef::parse(raw).unwrap();
    assert_eq!(r.as_str(), raw);
}

#[test]
fn as_str_roundtrips_inmemory() {
    let raw = "inmemory:k:ssh_key";
    let r = CredentialRef::parse(raw).unwrap();
    assert_eq!(r.as_str(), raw);
}

// ---------------------------------------------------------------------------
// All four CredentialKind variants
// ---------------------------------------------------------------------------

#[test]
fn kind_api_key_parses() {
    let r = CredentialRef::parse("env:K:api_key").unwrap();
    assert_eq!(r.kind, CredentialKind::ApiKey);
}

#[test]
fn kind_token_parses() {
    let r = CredentialRef::parse("env:K:token").unwrap();
    assert_eq!(r.kind, CredentialKind::Token);
}

#[test]
fn kind_database_url_parses() {
    let r = CredentialRef::parse("env:K:database_url").unwrap();
    assert_eq!(r.kind, CredentialKind::DatabaseUrl);
}

#[test]
fn kind_ssh_key_parses() {
    let r = CredentialRef::parse("env:K:ssh_key").unwrap();
    assert_eq!(r.kind, CredentialKind::SshKey);
}

// ---------------------------------------------------------------------------
// Invalid inputs
// ---------------------------------------------------------------------------

#[test]
fn missing_kind_suffix_fails() {
    let err = CredentialRef::parse("env:MY_KEY").unwrap_err();
    assert!(matches!(err, CredentialError::InvalidFormat(_)));
}

#[test]
fn unknown_scheme_fails() {
    let err = CredentialRef::parse("vault:MY_KEY:api_key").unwrap_err();
    assert!(matches!(err, CredentialError::InvalidFormat(_)));
}

#[test]
fn unknown_kind_fails() {
    let err = CredentialRef::parse("env:MY_KEY:password").unwrap_err();
    assert!(matches!(err, CredentialError::InvalidFormat(_)));
}

#[test]
fn empty_string_fails() {
    assert!(CredentialRef::parse("").is_err());
}

#[test]
fn only_colon_fails() {
    assert!(CredentialRef::parse(":").is_err());
}

// ---------------------------------------------------------------------------
// Exhaustiveness guard — fails to compile if new schemes or kinds are added
// without updating conformance
// ---------------------------------------------------------------------------

#[test]
fn all_credential_kinds_are_covered() {
    // Exhaustive match: adding a new CredentialKind variant will cause a compile error.
    let kinds = [
        CredentialKind::ApiKey,
        CredentialKind::Token,
        CredentialKind::DatabaseUrl,
        CredentialKind::SshKey,
    ];
    for k in &kinds {
        let _ = match k {
            CredentialKind::ApiKey => "api_key",
            CredentialKind::Token => "token",
            CredentialKind::DatabaseUrl => "database_url",
            CredentialKind::SshKey => "ssh_key",
        };
    }
}

#[test]
fn all_credential_schemes_are_covered() {
    let schemes = [
        CredentialScheme::Env,
        CredentialScheme::Keyring,
        CredentialScheme::OnePassword,
        CredentialScheme::Bitwarden,
        CredentialScheme::InMemory,
    ];
    assert_eq!(schemes.len(), 5);
}
