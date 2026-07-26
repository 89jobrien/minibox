//! Conformance tests for the `ResourceLimiter` trait contract.
//!
//! All tests use `MockLimiter` — no kernel/cgroup interaction.

use minibox::testing::mocks::limiter::MockLimiter;
use minibox_core::domain::{ResourceConfig, ResourceLimiter};

const fn default_config() -> ResourceConfig {
    ResourceConfig {
        memory_limit_bytes: Some(128 * 1024 * 1024),
        cpu_weight: Some(512),
        pids_max: None,
        io_max_bytes_per_sec: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "create_returns_cgroup_path",
    adapter: "limiter",
    category: Unit,
    |ctx| {
        let limiter = MockLimiter::new();
        let path = ctx.assert_ok(
            limiter.create("testcontainer01", &default_config()),
            "create",
        );
        if let Some(p) = path {
            ctx.assert_contains(&p, "testcontainer01", "path contains container_id");
        }
        ctx.result()
    }
}

crate::conformance_test! {
    name: "create_increments_count",
    adapter: "limiter",
    category: Unit,
    |ctx| {
        let limiter = MockLimiter::new();
        limiter
            .create("counttest01", &default_config())
            .expect("create");
        ctx.assert_eq(1, limiter.create_count(), "create_count after one create");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "create_failure_returns_err",
    adapter: "limiter",
    category: EdgeCase,
    |ctx| {
        let limiter = MockLimiter::new().with_create_failure();
        ctx.assert_err(
            limiter.create("failtest01", &default_config()),
            "failure mock returns Err",
        );
        ctx.result()
    }
}

crate::conformance_test! {
    name: "create_failure_increments_count",
    adapter: "limiter",
    category: EdgeCase,
    |ctx| {
        let limiter = MockLimiter::new().with_create_failure();
        let _ = limiter.create("failtest02", &default_config());
        ctx.assert_eq(1, limiter.create_count(), "failed create still counted");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "add_process_succeeds_by_default",
    adapter: "limiter",
    category: Unit,
    |ctx| {
        let limiter = MockLimiter::new();
        ctx.assert_ok(
            limiter.add_process("aptest01", 12345),
            "add_process succeeds",
        );
        ctx.result()
    }
}

crate::conformance_test! {
    name: "cleanup_increments_count",
    adapter: "limiter",
    category: Unit,
    |ctx| {
        let limiter = MockLimiter::new();
        limiter.cleanup("cleanuptest01").expect("cleanup");
        ctx.assert_eq(
            1,
            limiter.cleanup_count(),
            "cleanup_count after one cleanup",
        );
        ctx.result()
    }
}

crate::conformance_test! {
    name: "create_then_cleanup_round_trip",
    adapter: "limiter",
    category: Integration,
    |ctx| {
        let limiter = MockLimiter::new();
        ctx.assert_ok(limiter.create("roundtrip01", &default_config()), "create");
        ctx.assert_ok(limiter.cleanup("roundtrip01"), "cleanup");
        ctx.assert_eq(1, limiter.create_count(), "create_count == 1");
        ctx.assert_eq(1, limiter.cleanup_count(), "cleanup_count == 1");
        ctx.result()
    }
}
