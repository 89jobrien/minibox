//! Conformance tests for the [`ContainerCommitter`] trait contract.
//!
//! All tests use `MockContainerCommitter` — no real container operations occur.

use minibox::testing::mocks::commit::MockContainerCommitter;
use minibox_core::domain::{CommitConfig, ContainerCommitter, ContainerId};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().expect("build Tokio runtime")
}

fn container_id() -> ContainerId {
    ContainerId::new("ccmt001abc".to_string()).expect("valid container id")
}

const fn commit_config() -> CommitConfig {
    CommitConfig {
        author: None,
        message: None,
        env_overrides: vec![],
        cmd_override: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

crate::conformance_test! {
    name: "commit_returns_image_metadata",
    adapter: "container_committer",
    capability: Commit,
    category: Unit,
    |ctx| {
        let mock = MockContainerCommitter::new();
        let result = rt().block_on(mock.commit(&container_id(), "myimage:v1", &commit_config()));
        if let Some(meta) = ctx.assert_ok(result, "commit should succeed") {
            ctx.assert_eq("myimage".to_string(), meta.name, "image name");
            ctx.assert_eq("v1".to_string(), meta.tag, "image tag");
        }
        ctx.result()
    }
}

crate::conformance_test! {
    name: "commit_increments_count",
    adapter: "container_committer",
    capability: Commit,
    category: Unit,
    |ctx| {
        let mock = MockContainerCommitter::new();
        rt().block_on(mock.commit(&container_id(), "img:tag", &commit_config()))
            .expect("commit");
        ctx.assert_eq(1, mock.call_count(), "call_count after one commit");
        ctx.result()
    }
}

crate::conformance_test! {
    name: "commit_defaults_to_latest_tag",
    adapter: "container_committer",
    capability: Commit,
    category: EdgeCase,
    |ctx| {
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

crate::conformance_test! {
    name: "commit_failure_returns_err",
    adapter: "container_committer",
    capability: Commit,
    category: EdgeCase,
    |ctx| {
        let mock = MockContainerCommitter::new().with_failure();
        let result = rt().block_on(mock.commit(&container_id(), "img:tag", &commit_config()));
        ctx.assert_err(result, "commit with failure configured must return Err");
        ctx.result()
    }
}
