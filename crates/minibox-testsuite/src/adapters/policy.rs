//! Conformance tests for `ContainerPolicy` enforcement.
//!
//! Tests use `validate_policy` directly — no daemon socket required.

use minibox::daemon::handler::{ContainerPolicy, PolicyOverride, validate_policy};
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
    fn name(&self) -> &'static str {
        "run_with_bind_mount_when_denied_returns_error"
    }
    fn adapter(&self) -> &'static str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: false,
            ..Default::default()
        };
        let mounts = vec![a_bind_mount()];
        let result = validate_policy(&mounts, false, None, &policy);
        ctx.assert_err(result, "bind mount denied when allow_bind_mounts=false");
        ctx.result()
    }
}

pub struct RunPrivilegedWhenDeniedReturnsError;
impl ConformanceTest for RunPrivilegedWhenDeniedReturnsError {
    fn name(&self) -> &'static str {
        "run_privileged_when_denied_returns_error"
    }
    fn adapter(&self) -> &'static str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: false,
            ..Default::default()
        };
        let result = validate_policy(&[], true, None, &policy);
        ctx.assert_err(result, "privileged denied when allow_privileged=false");
        ctx.result()
    }
}

pub struct RunWithBindMountWhenAllowedSucceeds;
impl ConformanceTest for RunWithBindMountWhenAllowedSucceeds {
    fn name(&self) -> &'static str {
        "run_with_bind_mount_when_allowed_succeeds"
    }
    fn adapter(&self) -> &'static str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: true,
            allow_privileged: false,
            ..Default::default()
        };
        let mounts = vec![a_bind_mount()];
        let result = validate_policy(&mounts, false, None, &policy);
        ctx.assert_ok(result, "bind mount allowed when allow_bind_mounts=true");
        ctx.result()
    }
}

pub struct RunPrivilegedWhenAllowedSucceeds;
impl ConformanceTest for RunPrivilegedWhenAllowedSucceeds {
    fn name(&self) -> &'static str {
        "run_privileged_when_allowed_succeeds"
    }
    fn adapter(&self) -> &'static str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: true,
            ..Default::default()
        };
        let result = validate_policy(&[], true, None, &policy);
        ctx.assert_ok(result, "privileged allowed when allow_privileged=true");
        ctx.result()
    }
}

pub struct DefaultPolicyDeniesBothCapabilities;
impl ConformanceTest for DefaultPolicyDeniesBothCapabilities {
    fn name(&self) -> &'static str {
        "default_policy_denies_bind_mounts"
    }
    fn adapter(&self) -> &'static str {
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
            validate_policy(&mounts, false, None, &policy),
            "validate_policy rejects bind mounts under default policy",
        );
        ctx.assert_err(
            validate_policy(&[], true, None, &policy),
            "validate_policy rejects privileged under default policy",
        );
        ctx.result()
    }
}

pub struct PriorityBelowMinimumReturnsError;
impl ConformanceTest for PriorityBelowMinimumReturnsError {
    fn name(&self) -> &'static str {
        "priority_below_minimum_returns_error"
    }
    fn adapter(&self) -> &'static str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: false,
            min_priority: Some(slashcrux::Priority::High),
        };
        // Low priority is below the High minimum — must be rejected.
        let result = validate_policy(&[], false, Some(slashcrux::Priority::Low), &policy);
        ctx.assert_err(result, "priority Low below minimum High returns error");
        ctx.result()
    }
}

pub struct NoPriorityWhenMinimumRequiredReturnsError;
impl ConformanceTest for NoPriorityWhenMinimumRequiredReturnsError {
    fn name(&self) -> &'static str {
        "no_priority_when_minimum_required_returns_error"
    }
    fn adapter(&self) -> &'static str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: false,
            min_priority: Some(slashcrux::Priority::Medium),
        };
        // No priority supplied when one is required — must be rejected.
        let result = validate_policy(&[], false, None, &policy);
        ctx.assert_err(
            result,
            "no priority when minimum Medium required returns error",
        );
        ctx.result()
    }
}

pub struct PriorityMeetingMinimumSucceeds;
impl ConformanceTest for PriorityMeetingMinimumSucceeds {
    fn name(&self) -> &'static str {
        "priority_meeting_minimum_succeeds"
    }
    fn adapter(&self) -> &'static str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy {
            allow_bind_mounts: false,
            allow_privileged: false,
            min_priority: Some(slashcrux::Priority::Medium),
        };
        // Critical > Medium — should be accepted.
        let result = validate_policy(&[], false, Some(slashcrux::Priority::Critical), &policy);
        ctx.assert_ok(result, "priority Critical meets minimum Medium");
        ctx.result()
    }
}

pub struct PolicyOverrideLoosensBindMountDeny;
impl ConformanceTest for PolicyOverrideLoosensBindMountDeny {
    fn name(&self) -> &'static str {
        "policy_override_loosens_bind_mount_deny"
    }
    fn adapter(&self) -> &'static str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let base = ContainerPolicy::default(); // allow_bind_mounts=false
        let override_ = PolicyOverride {
            allow_bind_mounts: Some(true),
            ..Default::default()
        };
        let effective = base.with_overrides(&override_);
        ctx.assert_true(
            effective.allow_bind_mounts,
            "PolicyOverride(allow_bind_mounts=true) overrides default deny",
        );
        let mounts = vec![a_bind_mount()];
        let result = validate_policy(&mounts, false, None, &effective);
        ctx.assert_ok(
            result,
            "bind mount accepted after PolicyOverride loosens policy",
        );
        ctx.result()
    }
}

pub struct BothViolationsRejectedOnFirstMatch;
impl ConformanceTest for BothViolationsRejectedOnFirstMatch {
    fn name(&self) -> &'static str {
        "both_violations_rejected_on_first_match"
    }
    fn adapter(&self) -> &'static str {
        "policy"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let policy = ContainerPolicy::default(); // denies both
        let mounts = vec![a_bind_mount()];
        // Both bind mount and privileged are requested — validate_policy returns
        // an error (bind mount is checked first in the implementation).
        let result = validate_policy(&mounts, true, None, &policy);
        ctx.assert_err(
            result,
            "bind-mount + privileged together under deny policy returns error",
        );
        ctx.result()
    }
}

/// Return all policy conformance tests.
#[must_use]
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(RunWithBindMountWhenDeniedReturnsError),
        Box::new(RunPrivilegedWhenDeniedReturnsError),
        Box::new(RunWithBindMountWhenAllowedSucceeds),
        Box::new(RunPrivilegedWhenAllowedSucceeds),
        Box::new(DefaultPolicyDeniesBothCapabilities),
        Box::new(PriorityBelowMinimumReturnsError),
        Box::new(NoPriorityWhenMinimumRequiredReturnsError),
        Box::new(PriorityMeetingMinimumSucceeds),
        Box::new(PolicyOverrideLoosensBindMountDeny),
        Box::new(BothViolationsRejectedOnFirstMatch),
    ]
}
