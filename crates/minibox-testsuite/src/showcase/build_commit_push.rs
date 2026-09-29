//! Showcase scenario: build / commit / push (Experimental feature set).
//!
//! `mbx build`, `mbx commit`, and `mbx push` have no CLI subcommand — these
//! capabilities are domain-trait-only (`ImageBuilder`, `ContainerCommitter`,
//! `ImagePusher`), reachable only via the daemon handler or a direct library
//! call. This scenario is therefore the one place in the showcase suite that
//! legitimately calls into `minibox`/`minibox-core` adapters and domain
//! traits directly rather than shelling out to the `mbx` binary, mirroring
//! the pattern already used by `crates/minibox/tests/conformance_build.rs`,
//! `conformance_commit.rs`, and `conformance_push.rs`.
//!
//! Gating:
//! - build is gated on `BackendCapability::BuildFromContext`
//! - commit is gated on `BackendCapability::Commit`
//! - push is gated on `BackendCapability::PushToRegistry`, and its live-registry
//!   tier is further gated on the `CONFORMANCE_PUSH_REGISTRY` env var, skipping
//!   cleanly when unset — exactly as `conformance_push.rs` does.
//!
//! `krun` declares none of these capabilities and is excluded per the backend
//! support matrix documented on `BackendCapability`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use minibox::adapters::{MiniboxImageBuilder, OciPushAdapter, commit_upper_dir_to_image};
use minibox::testing::fixtures::{BuildContextFixture, WritableUpperDirFixture};
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::{
    AsAny, BackendCapability, BuildConfig, BuildContext, CommitConfig, ContainerCommitter,
    ContainerId, DynContainerCommitter, DynFilesystemProvider, DynImageBuilder, DynImagePusher,
    DynImageRegistry, DynRegistryRouter, ImageMetadata, RegistryCredentials,
};
use minibox_core::image::ImageStore;
use minibox_core::image::reference::ImageRef;
use minibox_core::image::registry::RegistryClient;
use tokio::sync::mpsc;

use super::reporter::Reporter;
use super::{Scenario, ScenarioCtx};

/// Env var opting in to the live-registry push tier, matching the convention
/// established by `crates/minibox/tests/conformance_push.rs`.
const PUSH_REGISTRY_ENV: &str = "CONFORMANCE_PUSH_REGISTRY";

/// Demonstrates the Experimental build/commit/push feature set.
pub struct BuildCommitPush;

impl Scenario for BuildCommitPush {
    fn name(&self) -> &'static str {
        "build_commit_push"
    }

    fn required_capability(&self) -> Option<BackendCapability> {
        Some(BackendCapability::BuildFromContext)
    }

    fn run(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        r.section("build / commit / push (experimental)");

        // Domain-trait calls are async; the showcase harness runs scenarios
        // from sync context (mirroring `ConformanceTest::run_sync`), so we
        // drive a dedicated Tokio runtime here rather than requiring an
        // async fn in the `Scenario` trait.
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| anyhow::anyhow!("failed to start scenario runtime: {e}"))?;

        rt.block_on(async { self.run_async(ctx, r).await })
    }
}

impl BuildCommitPush {
    async fn run_async(&self, ctx: &ScenarioCtx, r: &dyn Reporter) -> anyhow::Result<()> {
        // This scenario calls domain adapters directly (see module docs) rather
        // than driving the `mbx` CLI through `ctx.cli()`, so it manages its own
        // scratch directory instead of reaching into `ScenarioCtx`.
        let tmp = tempfile::TempDir::new()
            .map_err(|e| anyhow::anyhow!("create scenario temp dir: {e}"))?;
        let data_dir = tmp.path().join("build-commit-push");
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| anyhow::anyhow!("create scenario data dir: {e}"))?;
        let image_store = Arc::new(
            ImageStore::new(data_dir.join("images"))
                .map_err(|e| anyhow::anyhow!("ImageStore::new: {e}"))?,
        );

        self.run_build(ctx, r, Arc::clone(&image_store), data_dir.join("build"))
            .await;
        self.run_commit(ctx, r, Arc::clone(&image_store)).await?;
        self.run_push(ctx, r, Arc::clone(&image_store)).await?;

        Ok(())
    }

    /// Build an image from a minimal `FROM scratch` Dockerfile context.
    async fn run_build(
        &self,
        ctx: &ScenarioCtx,
        r: &dyn Reporter,
        image_store: Arc<ImageStore>,
        data_dir: PathBuf,
    ) {
        r.step("build: image from BuildContextFixture");

        if !ctx.supports(BackendCapability::BuildFromContext) {
            r.skip("backend does not support BackendCapability::BuildFromContext");
            return;
        }

        let outcome = self.build_once(image_store, data_dir).await;

        match outcome {
            Ok(meta) => r.success(&format!(
                "built image '{}' ({} layer(s))",
                meta.name,
                meta.layers.len()
            )),
            Err(e) => r.failure(&format!("build failed: {e}")),
        }
    }

    async fn build_once(
        &self,
        image_store: Arc<ImageStore>,
        data_dir: PathBuf,
    ) -> anyhow::Result<ImageMetadata> {
        let ctx_fixture = BuildContextFixture::new()
            .map_err(|e| anyhow::anyhow!("BuildContextFixture::new: {e}"))?;

        let filesystem = Arc::new(minibox::testing::mocks::MockFilesystem::new());
        let runtime = Arc::new(minibox::testing::mocks::MockRuntime::new());
        let registry_router = Arc::new(HostnameRegistryRouter::new(
            Arc::new(minibox::testing::mocks::MockRegistry::new()) as DynImageRegistry,
            std::iter::empty::<(&str, DynImageRegistry)>(),
        ));

        let builder: DynImageBuilder = Arc::new(MiniboxImageBuilder::new(
            Arc::clone(&image_store),
            data_dir,
            filesystem as DynFilesystemProvider,
            runtime as minibox_core::domain::DynContainerRuntime,
            registry_router as DynRegistryRouter,
        ));

        let build_context = BuildContext {
            directory: ctx_fixture.context_dir.clone(),
            dockerfile: PathBuf::from("Dockerfile"),
        };
        let build_config = BuildConfig {
            tag: "showcase/build-commit-push:latest".to_string(),
            build_args: vec![],
            no_cache: false,
        };

        let (tx, _rx) = mpsc::channel(64);
        builder
            .build_image(
                &build_context,
                &build_config,
                crate::progress::tokio_progress_sink(tx),
            )
            .await
            .map_err(|e| anyhow::anyhow!("build_image: {e}"))
    }

    /// Commit a synthetic writable-upperdir snapshot into a new image.
    async fn run_commit(
        &self,
        ctx: &ScenarioCtx,
        r: &dyn Reporter,
        image_store: Arc<ImageStore>,
    ) -> anyhow::Result<()> {
        r.step("commit: WritableUpperDirFixture -> ImageMetadata");

        if !ctx.supports(BackendCapability::Commit) {
            r.skip("backend does not support BackendCapability::Commit");
            return Ok(());
        }

        let upper = WritableUpperDirFixture::new()
            .map_err(|e| anyhow::anyhow!("WritableUpperDirFixture::new: {e}"))?;

        let committer: DynContainerCommitter = Arc::new(ScenarioCommitAdapter {
            image_store: Arc::clone(&image_store),
            upper_dir: upper.upper_dir.clone(),
        });

        let cid = ContainerId::new("showcasebuildcommit1".to_string())
            .map_err(|e| anyhow::anyhow!("ContainerId::new: {e}"))?;
        let commit_config = CommitConfig {
            author: Some("showcase".to_string()),
            message: Some("showcase build_commit_push scenario".to_string()),
            env_overrides: vec![],
            cmd_override: None,
        };

        match committer
            .commit(&cid, "showcase/committed-image:v1", &commit_config)
            .await
        {
            Ok(meta) => r.success(&format!(
                "committed '{}:{}' ({} layer(s))",
                meta.name,
                meta.tag,
                meta.layers.len()
            )),
            Err(e) => r.failure(&format!("commit failed: {e}")),
        }

        Ok(())
    }

    /// Push tier: wiring is always exercised; the live-registry round trip
    /// only runs when `CONFORMANCE_PUSH_REGISTRY` is set, matching
    /// `conformance_push.rs` tier 2 semantics exactly.
    async fn run_push(
        &self,
        ctx: &ScenarioCtx,
        r: &dyn Reporter,
        image_store: Arc<ImageStore>,
    ) -> anyhow::Result<()> {
        r.step("push: OciPushAdapter wiring + optional live registry round trip");

        if !ctx.supports(BackendCapability::PushToRegistry) {
            r.skip("backend does not support BackendCapability::PushToRegistry");
            return Ok(());
        }

        let Some(registry_host) = std::env::var(PUSH_REGISTRY_ENV).ok() else {
            r.skip(&format!(
                "{PUSH_REGISTRY_ENV} not set — skipping live-registry push (set it to e.g. \
                 localhost:5000 with a registry running to exercise this tier)"
            ));
            return Ok(());
        };

        let upper = WritableUpperDirFixture::new()
            .map_err(|e| anyhow::anyhow!("WritableUpperDirFixture::new: {e}"))?;
        let target_name = "showcase/push-test";
        let target_tag = "latest";
        let target_ref = format!("{target_name}:{target_tag}");
        let commit_config = CommitConfig {
            author: None,
            message: Some("showcase push test".to_string()),
            env_overrides: vec![],
            cmd_override: None,
        };
        commit_upper_dir_to_image(
            Arc::clone(&image_store),
            &upper.upper_dir,
            &target_ref,
            &commit_config,
        )
        .map_err(|e| anyhow::anyhow!("seed commit for push: {e}"))?;

        let push_ref_str = format!("{registry_host}/{target_name}:{target_tag}");
        let image_ref = ImageRef::parse(&push_ref_str)
            .map_err(|e| anyhow::anyhow!("parse push ref '{push_ref_str}': {e}"))?;

        let client =
            RegistryClient::new().map_err(|e| anyhow::anyhow!("RegistryClient::new: {e}"))?;
        let pusher: DynImagePusher =
            Arc::new(OciPushAdapter::new(client, Arc::clone(&image_store)));

        match pusher
            .push_image(&image_ref, &RegistryCredentials::Anonymous, None)
            .await
        {
            Ok(result) => r.success(&format!(
                "pushed '{push_ref_str}' -> digest {} ({} bytes)",
                result.digest, result.size_bytes
            )),
            Err(e) => r.failure(&format!("push to {registry_host} failed: {e}")),
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Inline commit adapter (mirrors `ConformanceCommitAdapter` in
// `crates/minibox/tests/conformance_commit.rs`) — wraps
// `commit_upper_dir_to_image` directly so the scenario is self-contained and
// does not require a live `StateHandle`.
// ---------------------------------------------------------------------------

struct ScenarioCommitAdapter {
    image_store: Arc<ImageStore>,
    upper_dir: PathBuf,
}

impl AsAny for ScenarioCommitAdapter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait]
impl ContainerCommitter for ScenarioCommitAdapter {
    async fn commit(
        &self,
        _container_id: &ContainerId,
        target_ref: &str,
        config: &CommitConfig,
    ) -> anyhow::Result<ImageMetadata> {
        let image_store = Arc::clone(&self.image_store);
        let upper_dir = self.upper_dir.clone();
        let target_ref = target_ref.to_string();
        let config = config.clone();
        tokio::task::spawn_blocking(move || {
            commit_upper_dir_to_image(image_store, &upper_dir, &target_ref, &config)
        })
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking join: {e}"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_capability_are_stable() {
        let scenario = BuildCommitPush;
        assert_eq!(scenario.name(), "build_commit_push");
        assert_eq!(
            scenario.required_capability(),
            Some(BackendCapability::BuildFromContext)
        );
    }
}
