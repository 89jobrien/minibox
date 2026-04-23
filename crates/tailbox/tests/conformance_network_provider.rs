//! Conformance tests for `TailnetNetwork` as a `NetworkProvider` port implementation.
//!
//! Tests that can run without a real Tailscale daemon (no auth key needed):
//! - `attach()` is always a no-op — returns `Ok(())`.
//! - `cleanup()` on an unknown container is idempotent — returns `Ok(())`.
//! - `stats()` always returns `Ok(NetworkStats::default())` (no stats API in v0.2).
//! - `NetworkStats::default()` has all-zero fields.
//!
//! Tests that require live tailnet access are `#[ignore]`-gated and not run in CI.

use minibox_core::domain::{NetworkProvider, NetworkStats};
use tailbox::{TailnetConfig, TailnetNetwork};

// ---------------------------------------------------------------------------
// attach — always a no-op
// ---------------------------------------------------------------------------

#[tokio::test]
async fn attach_with_valid_id_and_pid_returns_ok() {
    let net = TailnetNetwork::new(TailnetConfig::default())
        .await
        .expect("TailnetNetwork::new must not fail");
    let result = net.attach("container-abc123", 12345).await;
    assert!(result.is_ok(), "attach must always succeed: {result:?}");
}

#[tokio::test]
async fn attach_with_empty_id_returns_ok() {
    let net = TailnetNetwork::new(TailnetConfig::default())
        .await
        .expect("TailnetNetwork::new must not fail");
    let result = net.attach("", 0).await;
    assert!(result.is_ok(), "attach must succeed even with empty id");
}

#[tokio::test]
async fn attach_twice_returns_ok_both_times() {
    let net = TailnetNetwork::new(TailnetConfig::default())
        .await
        .expect("TailnetNetwork::new must not fail");
    assert!(net.attach("c1", 100).await.is_ok());
    assert!(net.attach("c1", 100).await.is_ok());
}

// ---------------------------------------------------------------------------
// cleanup — idempotent on unknown container
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cleanup_unknown_container_is_ok() {
    let net = TailnetNetwork::new(TailnetConfig::default())
        .await
        .expect("TailnetNetwork::new must not fail");
    let result = net.cleanup("never-existed").await;
    assert!(
        result.is_ok(),
        "cleanup on unknown container must succeed: {result:?}"
    );
}

#[tokio::test]
async fn cleanup_called_twice_on_same_id_is_ok() {
    let net = TailnetNetwork::new(TailnetConfig::default())
        .await
        .expect("TailnetNetwork::new must not fail");
    assert!(net.cleanup("ctr-1").await.is_ok());
    assert!(net.cleanup("ctr-1").await.is_ok(), "double cleanup must be idempotent");
}

// ---------------------------------------------------------------------------
// stats — always returns NetworkStats::default()
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stats_returns_ok() {
    let net = TailnetNetwork::new(TailnetConfig::default())
        .await
        .expect("TailnetNetwork::new must not fail");
    let result = net.stats("any-container").await;
    assert!(result.is_ok(), "stats must always return Ok: {result:?}");
}

#[tokio::test]
async fn stats_rx_bytes_is_zero() {
    let net = TailnetNetwork::new(TailnetConfig::default())
        .await
        .expect("TailnetNetwork::new must not fail");
    let stats = net.stats("ctr").await.unwrap();
    assert_eq!(stats.rx_bytes, 0, "rx_bytes must be 0 in v0.2");
}

#[tokio::test]
async fn stats_tx_bytes_is_zero() {
    let net = TailnetNetwork::new(TailnetConfig::default())
        .await
        .expect("TailnetNetwork::new must not fail");
    let stats = net.stats("ctr").await.unwrap();
    assert_eq!(stats.tx_bytes, 0, "tx_bytes must be 0 in v0.2");
}

#[tokio::test]
async fn stats_all_fields_are_zero() {
    let net = TailnetNetwork::new(TailnetConfig::default())
        .await
        .expect("TailnetNetwork::new must not fail");
    let stats = net.stats("ctr").await.unwrap();
    let expected = NetworkStats::default();
    assert_eq!(stats.rx_bytes, expected.rx_bytes);
    assert_eq!(stats.tx_bytes, expected.tx_bytes);
    assert_eq!(stats.rx_packets, expected.rx_packets);
    assert_eq!(stats.tx_packets, expected.tx_packets);
    assert_eq!(stats.rx_errors, expected.rx_errors);
    assert_eq!(stats.tx_errors, expected.tx_errors);
    assert_eq!(stats.rx_dropped, expected.rx_dropped);
    assert_eq!(stats.tx_dropped, expected.tx_dropped);
}

#[tokio::test]
async fn stats_on_unknown_container_returns_default() {
    let net = TailnetNetwork::new(TailnetConfig::default())
        .await
        .expect("TailnetNetwork::new must not fail");
    let stats = net.stats("does-not-exist").await.unwrap();
    assert_eq!(stats.rx_bytes, 0);
}

// ---------------------------------------------------------------------------
// NetworkStats::default() contract
// ---------------------------------------------------------------------------

#[test]
fn network_stats_default_all_zero() {
    let s = NetworkStats::default();
    assert_eq!(s.rx_bytes, 0);
    assert_eq!(s.tx_bytes, 0);
    assert_eq!(s.rx_packets, 0);
    assert_eq!(s.tx_packets, 0);
    assert_eq!(s.rx_errors, 0);
    assert_eq!(s.tx_errors, 0);
    assert_eq!(s.rx_dropped, 0);
    assert_eq!(s.tx_dropped, 0);
}
