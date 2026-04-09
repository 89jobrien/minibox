//! Minibox image builder — executes a Dockerfile instruction-by-instruction.
//!
//! RUN steps are no-ops in this MVP builder — the focus is on parsing and
//! layer tracking for ENV/CMD/WORKDIR metadata. A full implementation would
//! spawn ephemeral containers per RUN step and commit each layer.

use anyhow::{Context, Result};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    BuildConfig, BuildContext, BuildProgress, CommitConfig, ContainerId, DynContainerCommitter,
    DynImageBuilder, ImageBuilder, ImageMetadata,
};
use minibox_core::image::ImageStore;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;
use uuid::Uuid;

use crate::image::dockerfile::{Instruction, ShellOrExec, parse};

pub struct MiniboxImageBuilder {
    #[allow(dead_code)]
    image_store: Arc<ImageStore>,
    committer: DynContainerCommitter,
    data_dir: PathBuf,
}

impl MiniboxImageBuilder {
    pub fn new(
        image_store: Arc<ImageStore>,
        committer: DynContainerCommitter,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            image_store,
            committer,
            data_dir,
        }
    }
}

as_any!(MiniboxImageBuilder);

fn instr_display(instr: &Instruction) -> String {
    match instr {
        Instruction::From { image, tag, .. } => format!("FROM {image}:{tag}"),
        Instruction::Run(ShellOrExec::Shell(s)) => format!("RUN {s}"),
        Instruction::Run(ShellOrExec::Exec(args)) => format!("RUN {:?}", args),
        Instruction::Copy { dest, .. } => format!("COPY -> {}", dest.display()),
        Instruction::Add { dest, .. } => format!("ADD -> {}", dest.display()),
        Instruction::Env(pairs) => format!("ENV {} pairs", pairs.len()),
        Instruction::Workdir(p) => format!("WORKDIR {}", p.display()),
        Instruction::Cmd(_) => "CMD".to_string(),
        Instruction::Entrypoint(_) => "ENTRYPOINT".to_string(),
        Instruction::Arg { name, .. } => format!("ARG {name}"),
        Instruction::Expose { port, proto } => format!("EXPOSE {port}/{proto}"),
        Instruction::Label(_) => "LABEL".to_string(),
        Instruction::User { name, .. } => format!("USER {name}"),
        Instruction::Comment(_) => "# comment".to_string(),
    }
}

#[async_trait]
impl ImageBuilder for MiniboxImageBuilder {
    async fn build_image(
        &self,
        context: &BuildContext,
        config: &BuildConfig,
        progress_tx: mpsc::Sender<BuildProgress>,
    ) -> Result<ImageMetadata> {
        let dockerfile_path = context.directory.join(&context.dockerfile);
        let dockerfile_content = tokio::fs::read_to_string(&dockerfile_path)
            .await
            .with_context(|| format!("read Dockerfile at {}", dockerfile_path.display()))?;

        let instructions = parse(&dockerfile_content).context("parse Dockerfile")?;

        let steps: Vec<&Instruction> = instructions
            .iter()
            .filter(|i| !matches!(i, Instruction::Comment(_)))
            .collect();
        let total = steps.len() as u32;

        let mut base_image = String::new();
        let mut env_state: Vec<String> = vec![];
        let mut cmd_override: Option<Vec<String>> = None;

        // Use a synthetic build ID for commit operations.
        let raw_uuid = Uuid::new_v4().simple().to_string();
        let build_id = raw_uuid[..16].to_string();

        for (step_idx, instr) in steps.iter().enumerate() {
            let step_num = step_idx as u32 + 1;
            let msg = format!("Step {step_num}/{total}: {}", instr_display(instr));
            let _ = progress_tx
                .send(BuildProgress {
                    step: step_num,
                    total_steps: total,
                    message: msg,
                })
                .await;

            info!(step = step_num, "build: step");

            match instr {
                Instruction::From { image, tag, .. } => {
                    base_image = format!("{image}:{tag}");
                }
                Instruction::Env(pairs) => {
                    for (k, v) in pairs {
                        env_state.push(format!("{k}={v}"));
                    }
                }
                Instruction::Cmd(ShellOrExec::Exec(args)) => {
                    cmd_override = Some(args.clone());
                }
                Instruction::Cmd(ShellOrExec::Shell(s)) => {
                    cmd_override = Some(vec!["/bin/sh".to_string(), "-c".to_string(), s.clone()]);
                }
                // RUN, COPY, ADD, WORKDIR, ENTRYPOINT, etc. are no-ops in this
                // simplified MVP builder. A full implementation would spawn
                // ephemeral containers per RUN step and commit each layer.
                _ => {}
            }
        }

        // Create a placeholder upper dir so the committer has something to tar.
        let upper_dir = self.data_dir.join("builds").join(&build_id).join("upper");
        tokio::fs::create_dir_all(&upper_dir)
            .await
            .context("create build upper dir")?;

        let cid = ContainerId::new(build_id.clone())
            .with_context(|| format!("invalid build container id: {build_id}"))?;

        let commit_config = CommitConfig {
            author: None,
            message: Some(format!("built from {base_image}")),
            env_overrides: env_state,
            cmd_override,
        };

        let meta = self
            .committer
            .commit(&cid, &config.tag, &commit_config)
            .await
            .context("commit build result")?;

        info!(tag = %config.tag, "build: complete");
        Ok(meta)
    }
}

pub fn minibox_image_builder(
    image_store: Arc<ImageStore>,
    committer: DynContainerCommitter,
    data_dir: PathBuf,
) -> DynImageBuilder {
    Arc::new(MiniboxImageBuilder::new(image_store, committer, data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instr_display_from() {
        let instr = Instruction::From {
            image: "alpine".to_string(),
            tag: "3.18".to_string(),
            alias: None,
        };
        assert!(instr_display(&instr).contains("alpine"));
    }

    #[test]
    fn instr_display_run_shell() {
        let instr = Instruction::Run(ShellOrExec::Shell("echo hi".to_string()));
        assert!(instr_display(&instr).starts_with("RUN"));
    }
}
