//! Unit tests for tailnet auth key resolution.
//! Tests use std::env mutations — serialised with ENV_LOCK.

use minibox_core::domain::{NetworkConfig, NetworkMode};
use tailbox::auth::resolve_auth_key;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn tailnet_config() -> NetworkConfig {
    NetworkConfig {
        mode: NetworkMode::Tailnet,
        ..Default::default()
    }
}

#[tokio::test]
async fn inline_key_takes_precedence() {
    let _g = ENV_LOCK.lock().unwrap();
    // SAFETY: serialised by ENV_LOCK; no concurrent readers of TAILSCALE_AUTH_KEY.
    unsafe { std::env::remove_var("TAILSCALE_AUTH_KEY") };

    let mut cfg = tailnet_config();
    cfg.tailnet_auth_key = Some("tskey-inline".to_string());

    let key = resolve_auth_key(&cfg, "tailscale-auth-key").await.unwrap();
    assert_eq!(key, "tskey-inline");
}

#[tokio::test]
async fn env_var_fallback() {
    let _g = ENV_LOCK.lock().unwrap();
    // SAFETY: serialised by ENV_LOCK.
    unsafe {
        std::env::set_var("TAILSCALE_AUTH_KEY", "tskey-from-env");
    }

    let cfg = tailnet_config(); // tailnet_auth_key = None

    let key = resolve_auth_key(&cfg, "tailscale-auth-key").await.unwrap();
    assert_eq!(key, "tskey-from-env");

    unsafe { std::env::remove_var("TAILSCALE_AUTH_KEY") };
}

#[tokio::test]
async fn no_key_returns_error() {
    let _g = ENV_LOCK.lock().unwrap();
    // SAFETY: serialised by ENV_LOCK.
    unsafe {
        std::env::remove_var("TAILSCALE_AUTH_KEY");
    }

    let cfg = tailnet_config();
    let result = resolve_auth_key(&cfg, "tailscale-auth-key").await;
    assert!(result.is_err(), "expected error when no key available");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no auth key"), "error should mention 'no auth key', got: {err}");
}
