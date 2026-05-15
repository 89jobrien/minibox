//! Conformance tests for `ContainerPolicy` enforcement.
//!
//! Tests use `validate_policy` directly — no daemon socket required.

use minibox::daemon::handler::{ContainerPolicy, validate_policy};
use minibox_core::domain::BindMount;

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

fn a_bind_mount() -> BindMount {
    BindMount {
        host_path: std::path::PathBuf::from("/host/data"),
        container_path: std::path::PathBuf::from("/data"),
        read_only: false,
    }
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

pub struct RunWithBindMountWhenDeniedReturnsError;
impl ConformanceTest for RunWithBindMountWhenDeniedReturnsError {
    fn name(&self) -> &str {
        "run_with_bind_mount_when_denied_returns_error"
    }
    fn adapter(&self) -> &str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: false,
        };
        let mounts = vec![a_bind_mount()];
        let result = validate_policy(&mounts, false, &policy);
        ctx.assert_err(result, "bind mount denied when allow_bind_mounts=false");
        ctx.result()
    }
}

pub struct RunPrivilegedWhenDeniedReturnsError;
impl ConformanceTest for RunPrivilegedWhenDeniedReturnsError {
    fn name(&self) -> &str {
        "run_privileged_when_denied_returns_error"
    }
    fn adapter(&self) -> &str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: false,
        };
        let result = validate_policy(&[], true, &policy);
        ctx.assert_err(result, "privileged denied when allow_privileged=false");
        ctx.result()
    }
}

pub struct RunWithBindMountWhenAllowedSucceeds;
impl ConformanceTest for RunWithBindMountWhenAllowedSucceeds {
    fn name(&self) -> &str {
        "run_with_bind_mount_when_allowed_succeeds"
    }
    fn adapter(&self) -> &str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: false,
        };
        let mounts = vec![a_bind_mount()];
        let result = validate_policy(&mounts, false, &policy);
        ctx.assert_ok(result, "bind mount allowed when allow_bind_mounts=true");
        ctx.result()
    }
}

pub struct RunPrivilegedWhenAllowedSucceeds;
impl ConformanceTest for RunPrivilegedWhenAllowedSucceeds {
    fn name(&self) -> &str {
        "run_privileged_when_allowed_succeeds"
    }
    fn adapter(&self) -> &str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: true,
        };
        let result = validate_policy(&[], true, &policy);
        ctx.assert_ok(result, "privileged allowed when allow_privileged=true");
        ctx.result()
    }
}

pub struct DefaultPolicyDeniesBothCapabilities;
impl ConformanceTest for DefaultPolicyDeniesBothCapabilities {
    fn name(&self) -> &str {
        "default_policy_denies_bind_mounts"
    }
    fn adapter(&self) -> &str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy::default();
        ctx.assert_false(policy.allow_bind_mounts, "default denies bind mounts");
        ctx.assert_false(policy.allow_privileged, "default denies privileged mode");
        let mounts = vec![a_bind_mount()];
        ctx.assert_err(
            validate_policy(&mounts, false, &policy),
            "validate_policy rejects bind mounts under default policy",
        );
        ctx.assert_err(
            validate_policy(&[], true, &policy),
            "validate_policy rejects privileged under default policy",
        );
        ctx.result()
    }
}

/// Return all policy conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(RunWithBindMountWhenDeniedReturnsError),
        Box::new(RunPrivilegedWhenDeniedReturnsError),
        Box::new(RunWithBindMountWhenAllowedSucceeds),
        Box::new(RunPrivilegedWhenAllowedSucceeds),
        Box::new(DefaultPolicyDeniesBothCapabilities),
    ]
}
