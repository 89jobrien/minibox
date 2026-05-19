//! Conformance tests for the [`ContainerCommitter`] trait contract.
//!
//! All tests use `MockContainerCommitter` — no real container operations occur.

use minibox::testing::mocks::commit::MockContainerCommitter;
use minibox_core::domain::{CommitConfig, ContainerCommitter, ContainerId};

use crate::harness::{ConformanceTest, TestCategory, TestContext, TestResult};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn container_id() -> ContainerId {
    ContainerId::new("ccmt001abc".to_string()).expect("valid container id")
}

fn commit_config() -> CommitConfig {
    CommitConfig {
        author: None,
        message: None,
        env_overrides: vec![],
        cmd_override: None,
    }
}

// ---------------------------------------------------------------------------
// Test structs
// ---------------------------------------------------------------------------

/// commit succeeds and returns ImageMetadata with correct name/tag.
pub struct CommitReturnsImageMetadata;
impl ConformanceTest for CommitReturnsImageMetadata {
    fn name(&self) -> &str {
        "commit_returns_image_metadata"
    }
    fn adapter(&self) -> &str {
        "container_committer"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockContainerCommitter::new();
        let result = rt().block_on(mock.commit(&container_id(), "myimage:v1", &commit_config()));
        if let Some(meta) = ctx.assert_ok(result, "commit should succeed") {
            ctx.assert_eq("myimage".to_string(), meta.name, "image name");
            ctx.assert_eq("v1".to_string(), meta.tag, "image tag");
        }
        ctx.result()
    }
}

/// commit increments the call count.
pub struct CommitIncrementsCount;
impl ConformanceTest for CommitIncrementsCount {
    fn name(&self) -> &str {
        "commit_increments_count"
    }
    fn adapter(&self) -> &str {
        "container_committer"
    }
    fn category(&self) -> TestCategory {
        TestCategory::Unit
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockContainerCommitter::new();
        rt().block_on(mock.commit(&container_id(), "img:tag", &commit_config()))
            .expect("commit");
        ctx.assert_eq(1, mock.call_count(), "call_count after one commit");
        ctx.result()
    }
}

/// commit defaults to "latest" tag when no colon is present in target ref.
pub struct CommitDefaultsToLatestTag;
impl ConformanceTest for CommitDefaultsToLatestTag {
    fn name(&self) -> &str {
        "commit_defaults_to_latest_tag"
    }
    fn adapter(&self) -> &str {
        "container_committer"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockContainerCommitter::new();
        let result = rt().block_on(mock.commit(&container_id(), "myimage", &commit_config()));
        if let Some(meta) = ctx.assert_ok(result, "commit with no tag should succeed") {
            ctx.assert_eq(
                "latest".to_string(),
                meta.tag,
                "default tag should be latest",
            );
        }
        ctx.result()
    }
}

/// commit returns Err when configured to fail.
pub struct CommitFailureReturnsErr;
impl ConformanceTest for CommitFailureReturnsErr {
    fn name(&self) -> &str {
        "commit_failure_returns_err"
    }
    fn adapter(&self) -> &str {
        "container_committer"
    }
    fn category(&self) -> TestCategory {
        TestCategory::EdgeCase
    }
    fn run_sync(&self, ctx: &mut TestContext) -> TestResult {
        let mock = MockContainerCommitter::new().with_failure();
        let result = rt().block_on(mock.commit(&container_id(), "img:tag", &commit_config()));
        ctx.assert_err(result, "commit with failure configured must return Err");
        ctx.result()
    }
}

/// Return all container_committer conformance tests.
pub fn all() -> Vec<Box<dyn ConformanceTest>> {
    vec![
        Box::new(CommitReturnsImageMetadata),
        Box::new(CommitIncrementsCount),
        Box::new(CommitDefaultsToLatestTag),
        Box::new(CommitFailureReturnsErr),
    ]
}
