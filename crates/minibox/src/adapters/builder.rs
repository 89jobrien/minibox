//! Minibox image builder — executes a Dockerfile instruction-by-instruction.
//!
//! Each `RUN` step spawns an ephemeral container via the injected
//! `ContainerRuntime` and `FilesystemProvider`, then commits the writable
//! overlay diff as a new image layer. `FROM` triggers a pull if the base
//! image is not already in the local store. `ENV`/`CMD` metadata is
//! accumulated and written into the final image config.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    BuildConfig, BuildContext, BuildProgress, CommitConfig, ContainerHooks, ContainerSpawnConfig,
    DynContainerRuntime, DynFilesystemProvider, DynImageBuilder, DynRegistryRouter, ImageBuilder,
    ImageMetadata,
};
use minibox_core::image::ImageStore;
use minibox_core::image::reference::ImageRef;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::adapters::commit::commit_upper_dir_to_image;
use crate::image::dockerfile::{Instruction, ShellOrExec, parse};

pub struct MiniboxImageBuilder {
    image_store: Arc<ImageStore>,
    data_dir: PathBuf,
    filesystem: DynFilesystemProvider,
    runtime: DynContainerRuntime,
    registry_router: DynRegistryRouter,
}

impl MiniboxImageBuilder {
    pub fn new(
        image_store: Arc<ImageStore>,
        data_dir: PathBuf,
        filesystem: DynFilesystemProvider,
        runtime: DynContainerRuntime,
        registry_router: DynRegistryRouter,
    ) -> Self {
        Self {
            image_store,
            data_dir,
            filesystem,
            runtime,
            registry_router,
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

/// Parse `"image:tag"` or `"image"` (defaults to `"latest"`).
fn split_image_tag(s: &str) -> (String, String) {
    if let Some((img, tag)) = s.rsplit_once(':') {
        (img.to_string(), tag.to_string())
    } else {
        (s.to_string(), "latest".to_string())
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

        let raw_uuid = Uuid::new_v4().simple().to_string();
        let build_id = raw_uuid[..16].to_string();
        let builds_dir = self.data_dir.join("builds").join(&build_id);

        // layer_stack: ordered list of extracted layer dirs (bottom → top).
        // Populated after FROM, extended after each RUN step.
        let mut layer_stack: Vec<PathBuf> = vec![];
        let mut base_image = String::new();
        let mut env_state: Vec<String> = vec![];
        let mut cmd_override: Option<Vec<String>> = None;
        let mut run_step = 0u32;

        for (step_idx, instr) in steps.iter().enumerate() {
            let step_num = step_idx as u32 + 1;
            let msg = format!("Step {step_num}/{total}: {}", instr_display(instr));
            if progress_tx
                .send(BuildProgress {
                    step: step_num,
                    total_steps: total,
                    message: msg,
                })
                .await
                .is_err()
            {
                warn!("build: client disconnected before step {step_num} progress could be sent");
            }

            info!(step = step_num, "build: step");

            match instr {
                Instruction::From { image, tag, .. } => {
                    base_image = format!("{image}:{tag}");

                    // `FROM scratch` means an empty base — no layers to pull or mount.
                    if image == "scratch" {
                        layer_stack = vec![];
                    } else {
                        // Pull the base image if not already in the local store.
                        let (img_name, img_tag) = split_image_tag(&base_image);
                        if !self.image_store.has_image(&img_name, &img_tag) {
                            info!(image = %base_image, "build: pulling base image");
                            let image_ref = ImageRef::parse(&base_image)
                                .with_context(|| format!("invalid FROM image ref: {base_image}"))?;
                            let registry = self.registry_router.route(&image_ref);
                            registry
                                .pull_image(&image_ref)
                                .await
                                .with_context(|| format!("pull base image {base_image}"))?;
                        }

                        layer_stack = self
                            .image_store
                            .get_image_layers(&img_name, &img_tag)
                            .with_context(|| {
                                format!("get layer dirs for base image {base_image}")
                            })?;
                    }
                }

                Instruction::Run(shell_or_exec) => {
                    run_step += 1;
                    let container_dir = builds_dir.join(format!("run-{run_step}"));
                    tokio::fs::create_dir_all(&container_dir)
                        .await
                        .context("create run step dir")?;

                    let (command, args) = match shell_or_exec {
                        ShellOrExec::Shell(s) => {
                            ("/bin/sh".to_string(), vec!["-c".to_string(), s.clone()])
                        }
                        ShellOrExec::Exec(argv) => {
                            if argv.is_empty() {
                                bail!("RUN exec form has empty argv at step {step_num}");
                            }
                            (argv[0].clone(), argv[1..].to_vec())
                        }
                    };

                    // Mount the current layer stack as the container rootfs.
                    let filesystem = Arc::clone(&self.filesystem);
                    let layer_stack_clone = layer_stack.clone();
                    let container_dir_clone = container_dir.clone();
                    let layout = tokio::task::spawn_blocking(move || {
                        filesystem.setup_rootfs(&layer_stack_clone, &container_dir_clone)
                    })
                    .await
                    .context("spawn_blocking setup_rootfs")?
                    .context("setup_rootfs for RUN step")?;

                    // Build the cgroup path — reuse builds_dir so it stays
                    // within the daemon's allowed cgroup subtree.
                    let cgroup_path = builds_dir.join(format!("cgroup-{run_step}"));

                    let spawn_config = ContainerSpawnConfig {
                        rootfs: layout.merged_dir.clone(),
                        command,
                        args,
                        env: env_state.clone(),
                        hostname: format!("minibox-build-{build_id}"),
                        cgroup_path,
                        capture_output: true,
                        hooks: ContainerHooks::default(),
                        skip_network_namespace: true,
                        mounts: vec![],
                        privileged: false,
                    };

                    let spawn_result = self
                        .runtime
                        .spawn_process(&spawn_config)
                        .await
                        .with_context(|| format!("spawn RUN container at step {step_num}"))?;

                    // Stream captured output as build progress messages.
                    #[cfg(unix)]
                    if let Some(reader_fd) = spawn_result.output_reader {
                        use std::io::{BufRead, BufReader};
                        use std::os::fd::FromRawFd;

                        // SAFETY: spawn_process returned this fd to us as the read
                        // end of a pipe. We take ownership via OwnedFd → File.
                        let file = unsafe {
                            std::fs::File::from_raw_fd(std::os::fd::IntoRawFd::into_raw_fd(
                                reader_fd,
                            ))
                        };
                        let reader = BufReader::new(file);
                        for line in reader.lines() {
                            let line = line.unwrap_or_default();
                            if progress_tx
                                .send(BuildProgress {
                                    step: step_num,
                                    total_steps: total,
                                    message: line,
                                })
                                .await
                                .is_err()
                            {
                                warn!(
                                    "build: client disconnected during RUN output at step \
                                     {step_num}"
                                );
                                break;
                            }
                        }
                    }

                    let exit_code = self
                        .runtime
                        .wait_for_exit(spawn_result.runtime_id.as_deref(), spawn_result.pid)
                        .await
                        .with_context(|| format!("wait_for_exit at step {step_num}"))?;

                    // Unmount before checking exit — always clean up.
                    let filesystem_cleanup = Arc::clone(&self.filesystem);
                    let container_dir_for_cleanup = container_dir.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        filesystem_cleanup.cleanup(&container_dir_for_cleanup)
                    })
                    .await
                    .context("spawn_blocking cleanup")?
                    {
                        warn!(
                            step = step_num,
                            error = %e,
                            "build: rootfs cleanup failed after RUN step"
                        );
                    }

                    if exit_code != 0 {
                        bail!(
                            "RUN step {step_num} exited with code {exit_code}: {}",
                            instr_display(instr)
                        );
                    }

                    // Commit the upperdir as a new layer and extend the stack.
                    let image_store = Arc::clone(&self.image_store);
                    let step_tag = format!("{}:build-step-{run_step}", config.tag);
                    let step_tag_for_lookup = step_tag.clone();
                    let upper_dir = layout
                        .rootfs_metadata
                        .as_ref()
                        .map(|m| m.overlay_upper_dir().clone())
                        .unwrap_or_else(|| layout.merged_dir.clone());
                    let step_meta = tokio::task::spawn_blocking(move || {
                        commit_upper_dir_to_image(
                            image_store,
                            &upper_dir,
                            &step_tag,
                            &CommitConfig {
                                author: None,
                                message: None,
                                env_overrides: vec![],
                                cmd_override: None,
                            },
                        )
                    })
                    .await
                    .context("spawn_blocking commit RUN step")?
                    .with_context(|| format!("commit RUN step {step_num}"))?;

                    // The new layer's extracted directory becomes the top of the stack.
                    let (step_name, step_tag_part) = split_image_tag(&step_tag_for_lookup);
                    let new_layers = self
                        .image_store
                        .get_image_layers(&step_name, &step_tag_part)
                        .context("get_image_layers after RUN commit")?;
                    // Extend with layers added by this step (typically the last one).
                    let prev_len = layer_stack.len();
                    for layer in new_layers.into_iter().skip(prev_len) {
                        layer_stack.push(layer);
                    }

                    info!(
                        step = step_num,
                        layers = step_meta.layers.len(),
                        "build: RUN step committed"
                    );
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

                // COPY, ADD, WORKDIR, ENTRYPOINT, ARG, EXPOSE, LABEL, USER
                // are not yet implemented — treat as no-ops so the build
                // continues. A warning is emitted so users know.
                other => {
                    warn!(
                        instruction = %instr_display(other),
                        "build: instruction not yet implemented, skipping"
                    );
                }
            }
        }

        // Commit the final layer stack with accumulated ENV/CMD metadata.
        let upper_dir = builds_dir.join("final-upper");
        tokio::fs::create_dir_all(&upper_dir)
            .await
            .context("create final upper dir")?;

        let commit_config = CommitConfig {
            author: None,
            message: Some(format!("built from {base_image}")),
            env_overrides: env_state,
            cmd_override,
        };

        let image_store = Arc::clone(&self.image_store);
        let target_tag = config.tag.clone();
        let meta = tokio::task::spawn_blocking(move || {
            commit_upper_dir_to_image(image_store, &upper_dir, &target_tag, &commit_config)
        })
        .await
        .context("spawn_blocking build commit")?
        .context("commit build result")?;

        info!(tag = %config.tag, "build: complete");
        Ok(meta)
    }
}

pub fn minibox_image_builder(
    image_store: Arc<ImageStore>,
    data_dir: PathBuf,
    filesystem: DynFilesystemProvider,
    runtime: DynContainerRuntime,
    registry_router: DynRegistryRouter,
) -> DynImageBuilder {
    Arc::new(MiniboxImageBuilder::new(
        image_store,
        data_dir,
        filesystem,
        runtime,
        registry_router,
    ))
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

    #[test]
    fn split_image_tag_with_tag() {
        let (img, tag) = split_image_tag("alpine:3.21");
        assert_eq!(img, "alpine");
        assert_eq!(tag, "3.21");
    }

    #[test]
    fn split_image_tag_no_tag_defaults_latest() {
        let (img, tag) = split_image_tag("alpine");
        assert_eq!(img, "alpine");
        assert_eq!(tag, "latest");
    }
}
