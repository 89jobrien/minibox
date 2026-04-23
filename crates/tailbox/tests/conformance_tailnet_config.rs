//! Conformance tests for `TailnetConfig`.
//!
//! Verifies the externally-visible contract:
//! - `Default` produces documented default values.
//! - `key_secret_name` defaults to `"tailscale-auth-key"`.
//! - `auth_key` defaults to `None`.
//! - `Clone` produces an equal value.
//! - `Debug` output is non-empty.

use tailbox::TailnetConfig;

// default_key_secret_name and default_auth_key tests live in proptest_suite.rs
// to avoid duplication — see the "Default value contract" section there.

#[test]
fn clone_produces_equal_key_secret_name() {
    let cfg = TailnetConfig {
        auth_key: None,
        key_secret_name: "my-secret".to_string(),
    };
    let cloned = cfg.clone();
    assert_eq!(cfg.key_secret_name, cloned.key_secret_name);
}

#[test]
fn clone_produces_equal_auth_key() {
    let cfg = TailnetConfig {
        auth_key: Some("tskey-abc".to_string()),
        key_secret_name: "k".to_string(),
    };
    let cloned = cfg.clone();
    assert_eq!(cfg.auth_key, cloned.auth_key);
}

#[test]
fn debug_output_is_non_empty() {
    let cfg = TailnetConfig::default();
    let debug = format!("{cfg:?}");
    assert!(!debug.is_empty());
}

#[test]
fn key_secret_name_survives_roundtrip_through_clone_chain() {
    // Verify no surprising mutation across multiple clones.
    let original = TailnetConfig {
        auth_key: Some("tskey-xyz".to_string()),
        key_secret_name: "original-key".to_string(),
    };
    let c1 = original.clone();
    let c2 = c1.clone();
    assert_eq!(original.key_secret_name, c2.key_secret_name);
    assert_eq!(original.auth_key, c2.auth_key);
}
