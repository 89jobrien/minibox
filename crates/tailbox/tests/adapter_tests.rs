//! Unit tests for TailnetNetwork adapter.
//! Integration tests requiring a real tailnet are marked #[ignore].

use minibox_core::domain::{NetworkConfig, NetworkMode, NetworkProvider, TailnetMode};
use tailbox::{TailnetConfig, TailnetNetwork};

/// attach() must always return Ok(()) — it is a no-op.
#[tokio::test]
async fn attach_is_noop() {
    let net = TailnetNetwork::new(TailnetConfig::default()).await.unwrap();
    let result = net.attach("test-container-id", 12345).await;
    assert!(result.is_ok(), "attach should always succeed: {result:?}");
}

/// stats() must always return Ok(NetworkStats::default()) — no stats API in v0.2.
#[tokio::test]
async fn stats_returns_default() {
    use minibox_core::domain::NetworkStats;
    let net = TailnetNetwork::new(TailnetConfig::default()).await.unwrap();
    let stats = net.stats("test-container-id").await.unwrap();
    assert_eq!(stats.rx_bytes, 0);
    assert_eq!(stats.tx_bytes, 0);
}

/// cleanup() on an unknown container_id must not error.
#[tokio::test]
async fn cleanup_unknown_container_is_ok() {
    let net = TailnetNetwork::new(TailnetConfig::default()).await.unwrap();
    let result = net.cleanup("never-existed").await;
    assert!(
        result.is_ok(),
        "cleanup of unknown container should be ok: {result:?}"
    );
}

/// Gateway mode integration test — requires real tailnet.
#[tokio::test]
#[ignore = "requires TAILSCALE_AUTH_KEY and network access"]
async fn setup_gateway_mode_returns_valid_context() {
    let key = std::env::var("TAILSCALE_AUTH_KEY")
        .expect("TAILSCALE_AUTH_KEY must be set for integration test");

    let config = NetworkConfig {
        mode: NetworkMode::Tailnet,
        tailnet_mode: TailnetMode::Gateway,
        tailnet_auth_key: Some(key),
        ..Default::default()
    };

    let net = TailnetNetwork::new(TailnetConfig::default()).await.unwrap();
    let ctx_json = net.setup("test-gw-container", &config).await.unwrap();
    let ctx: serde_json::Value = serde_json::from_str(&ctx_json).unwrap();

    assert_eq!(ctx["mode"].as_str().unwrap(), "gateway");
    assert!(ctx["tailnet_ip"].as_str().is_some(), "must have tailnet_ip");

    net.cleanup("test-gw-container").await.unwrap();
}

/// Per-container mode integration test — requires real tailnet.
#[tokio::test]
#[ignore = "requires TAILSCALE_AUTH_KEY and network access"]
async fn setup_per_container_mode_returns_own_ip() {
    let key = std::env::var("TAILSCALE_AUTH_KEY")
        .expect("TAILSCALE_AUTH_KEY must be set for integration test");

    let config = NetworkConfig {
        mode: NetworkMode::Tailnet,
        tailnet_mode: TailnetMode::PerContainer,
        tailnet_auth_key: Some(key),
        ..Default::default()
    };

    let net = TailnetNetwork::new(TailnetConfig::default()).await.unwrap();
    let ctx_json = net.setup("test-pc-container", &config).await.unwrap();
    let ctx: serde_json::Value = serde_json::from_str(&ctx_json).unwrap();

    assert_eq!(ctx["mode"].as_str().unwrap(), "per_container");
    assert!(ctx["tailnet_ip"].as_str().is_some());

    net.cleanup("test-pc-container").await.unwrap();
}
