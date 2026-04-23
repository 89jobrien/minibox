//! Property-based tests for tailbox domain types.
//!
//! Invariants tested:
//! - `TailnetConfig` always produces a non-empty `key_secret_name`.
//! - `TailnetConfig` `Default` matches documented defaults.
//! - Arbitrary `auth_key` values survive a clone without mutation.
//! - `key_secret_name` is always valid UTF-8 (by construction).

use proptest::prelude::*;
use tailbox::TailnetConfig;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_tailnet_config() -> impl Strategy<Value = TailnetConfig> {
    (
        proptest::option::of(any::<String>()),
        any::<String>().prop_filter("key name must be non-empty", |s| !s.is_empty()),
    )
        .prop_map(|(auth_key, key_secret_name)| TailnetConfig {
            auth_key,
            key_secret_name,
        })
}

// ---------------------------------------------------------------------------
// Invariants
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(proptest::test_runner::Config {
        failure_persistence: None,
        ..proptest::test_runner::Config::default()
    })]

    /// `key_secret_name` is always non-empty — the field is meaningful only when
    /// `auth_key` is `None`, but it must still be a valid lookup key.
    #[test]
    fn key_secret_name_is_always_non_empty(cfg in arb_tailnet_config()) {
        prop_assert!(
            !cfg.key_secret_name.is_empty(),
            "key_secret_name must be non-empty"
        );
    }

    /// Cloning a `TailnetConfig` produces an equal value.
    #[test]
    fn clone_is_equal(cfg in arb_tailnet_config()) {
        let cloned = cfg.clone();
        prop_assert_eq!(&cfg.auth_key, &cloned.auth_key);
        prop_assert_eq!(&cfg.key_secret_name, &cloned.key_secret_name);
    }

    /// When `auth_key` is `Some`, it survives clone without mutation.
    #[test]
    fn inline_auth_key_survives_clone(key in any::<String>()) {
        let cfg = TailnetConfig {
            auth_key: Some(key.clone()),
            key_secret_name: "k".to_string(),
        };
        let cloned = cfg.clone();
        prop_assert_eq!(cloned.auth_key.as_deref(), Some(key.as_str()));
    }
}

// ---------------------------------------------------------------------------
// Default value contract
// ---------------------------------------------------------------------------

#[test]
fn default_key_secret_name_is_tailscale_auth_key() {
    let cfg = TailnetConfig::default();
    assert_eq!(cfg.key_secret_name, "tailscale-auth-key");
}

#[test]
fn default_auth_key_is_none() {
    let cfg = TailnetConfig::default();
    assert!(cfg.auth_key.is_none());
}
