//! Conformance tests for the [`NetworkProvider`] trait contract.
//!
//! All tests use `MockNetwork` — no real network namespaces are created.

use minibox::testing::mocks::network::MockNetwork;
use minibox_core::domain::{NetworkConfig, NetworkProvider};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn default_config() -> NetworkConfig {
    NetworkConfig::default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// setup returns a non-empty netns path.
crate::conformance_test! {
    name: "setup_returns_netns_path",
    adapter: "network",
    capability: Network,
    category: Unit,
    |ctx| {
        let mock = MockNetwork::new();
        let result = rt().block_on(mock.setup("ctr-net-001", &default_config()));
        if let Some(path) = ctx.assert_ok(result, "network setup should succeed") {
            ctx.assert_true(!path.is_empty(), "returned netns path must be non-empty");
        }
        ctx.result()
    }
}

// setup increments the call count.
crate::conformance_test! {
    name: "setup_increments_count",
    adapter: "network",
    capability: Network,
    category: Unit,
    |ctx| {
        let mock = MockNetwork::new();
        let _ = rt().block_on(mock.setup("ctr-net-002", &default_config()));
        ctx.assert_eq(1, mock.setup_count(), "setup_count after one call");
        ctx.result()
    }
}

// cleanup increments the cleanup count.
crate::conformance_test! {
    name: "cleanup_increments_count",
    adapter: "network",
    capability: Network,
    category: Unit,
    |ctx| {
        let mock = MockNetwork::new();
        let _ = rt().block_on(mock.cleanup("ctr-net-003"));
        ctx.assert_eq(1, mock.cleanup_count(), "cleanup_count after one call");
        ctx.result()
    }
}

// setup returns Err when configured to fail.
crate::conformance_test! {
    name: "setup_failure_returns_err",
    adapter: "network",
    capability: Network,
    category: EdgeCase,
    |ctx| {
        let mock = MockNetwork::new().with_setup_failure();
        let result = rt().block_on(mock.setup("ctr-net-004", &default_config()));
        ctx.assert_err(
            result,
            "network setup with failure configured must return Err",
        );
        ctx.result()
    }
}

// cleanup returns Err when configured to fail.
crate::conformance_test! {
    name: "cleanup_failure_returns_err",
    adapter: "network",
    capability: Network,
    category: EdgeCase,
    |ctx| {
        let mock = MockNetwork::new().with_cleanup_failure();
        let result = rt().block_on(mock.cleanup("ctr-net-005"));
        ctx.assert_err(
            result,
            "network cleanup with failure configured must return Err",
        );
        ctx.result()
    }
}
