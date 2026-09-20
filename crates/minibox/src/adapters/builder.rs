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
    DynContainerRuntime, DynFilesystemProvider, DynImageBuilder, DynProgressSink,
    DynRegistryRouter, ImageBuilder, ImageMetadata, ProgressSink,
};
use minibox_core::image::ImageStore;
use minibox_core::image::reference::ImageRef;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::adapters::commit::commit_upper_dir_to_image;
use crate::image::dockerfile::{Instruction, ShellOrExec, parse};

/// Native image builder that executes supported Dockerfile instructions.
pub struct MiniboxImageBuilder {
    image_store: Arc<ImageStore>,
    data_dir: PathBuf,
    filesystem: DynFilesystemProvider,
    runtime: DynContainerRuntime,
    registry_router: DynRegistryRouter,
}

impl MiniboxImageBuilder {
    /// Creates a builder from the image store and runtime adapter dependencies.
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
        Instruction::Run(ShellOrExec::Exec(args)) => format!("RUN {args:?}"),
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

fn expand_variables(input: &str, variables: &BTreeMap<String, String>) -> String {
    let mut expanded = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '$' {
            expanded.push(ch);
            continue;
        }

        if chars.peek() == Some(&'{') {
            chars.next();
            let mut name = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == '}' {
                    closed = true;
                    break;
                }
                name.push(next);
            }
            if closed {
                expanded.push_str(variables.get(&name).map_or("", String::as_str));
            } else {
                expanded.push_str("${");
                expanded.push_str(&name);
            }
            continue;
        }

        let mut name = String::new();
        while chars
            .peek()
            .is_some_and(|next| next.is_ascii_alphanumeric() || *next == '_')
        {
            if let Some(next) = chars.next() {
                name.push(next);
            }
        }
        if name.is_empty() {
            expanded.push('$');
        } else {
            expanded.push_str(variables.get(&name).map_or("", String::as_str));
        }
    }

    expanded
}

fn set_env(env: &mut Vec<String>, key: &str, value: &str) {
    let prefix = format!("{key}=");
    let entry = format!("{prefix}{value}");
    if let Some(existing) = env.iter_mut().find(|item| item.starts_with(&prefix)) {
        *existing = entry;
    } else {
        env.push(entry);
    }
}

fn resolve_workdir(current: &Path, requested: &Path) -> Result<PathBuf> {
    let mut resolved = if requested.is_absolute() {
        PathBuf::from("/")
    } else {
        current.to_path_buf()
    };

    for component in requested.components() {
        match component {
            Component::RootDir => resolved = PathBuf::from("/"),
            Component::CurDir => {}
            Component::ParentDir => {
                if resolved != Path::new("/") {
                    resolved.pop();
                }
            }
            Component::Normal(part) => resolved.push(part),
            Component::Prefix(_) => bail!("WORKDIR uses an unsupported path prefix"),
        }
    }

    Ok(resolved)
}

/// Context for a single RUN step inside `build_image`.
struct RunStepContext<'a> {
    step_num: u32,
    run_step: u32,
    builds_dir: &'a std::path::Path,
    build_id: &'a str,
    env_state: &'a [String],
    workdir: &'a Path,
    layer_stack: &'a [PathBuf],
    instr_label: String,
    shell_or_exec: &'a ShellOrExec,
}

/// Execute a single `RUN` instruction: set up rootfs, spawn the process,
/// stream output, wait for exit, clean up, and commit the resulting layer.
///
/// Returns the updated layer stack on success.
// qual:allow(iosp) reason: "build step orchestration — rootfs, spawn, stream, commit"
async fn execute_run_step(
    builder: &MiniboxImageBuilder,
    ctx: RunStepContext<'_>,
    progress_tx: &dyn ProgressSink<BuildProgress>,
    total: u32,
    config_tag: &str,
) -> Result<(Vec<PathBuf>, ImageMetadata)> {
    let container_dir = ctx.builds_dir.join(format!("run-{}", ctx.run_step));
    tokio::fs::create_dir_all(&container_dir)
        .await
        .context("create run step dir")?;

    let (mut command, mut args) = match ctx.shell_or_exec {
        ShellOrExec::Shell(s) => ("/bin/sh".to_string(), vec!["-c".to_string(), s.clone()]),
        ShellOrExec::Exec(argv) => {
            if argv.is_empty() {
                bail!("RUN exec form has empty argv at step {}", ctx.step_num);
            }
            (argv[0].clone(), argv[1..].to_vec())
        }
    };

    // Mount the current layer stack as the container rootfs.
    let filesystem = Arc::clone(&builder.filesystem);
    let layer_stack_clone = ctx.layer_stack.to_vec();
    let container_dir_clone = container_dir.clone();
    let layout = tokio::task::spawn_blocking(move || {
        filesystem.setup_rootfs(&layer_stack_clone, &container_dir_clone)
    })
    .await
    .context("spawn_blocking setup_rootfs")?
    .context("setup_rootfs for RUN step")?;

    let relative_workdir = ctx
        .workdir
        .strip_prefix("/")
        .context("WORKDIR must resolve to an absolute container path")?;
    let host_workdir = layout.merged_dir.join(relative_workdir);
    tokio::fs::create_dir_all(&host_workdir)
        .await
        .with_context(|| format!("create WORKDIR {}", ctx.workdir.display()))?;

    if ctx.workdir != Path::new("/") {
        let original_command = std::mem::replace(&mut command, "/bin/sh".to_string());
        let original_args = std::mem::take(&mut args);
        args = vec![
            "-c".to_string(),
            "mkdir -p -- \"$1\" && cd -- \"$1\" && shift && exec \"$@\"".to_string(),
            "minibox-build-workdir".to_string(),
            ctx.workdir.display().to_string(),
            original_command,
        ];
        args.extend(original_args);
    }

    let cgroup_path = ctx.builds_dir.join(format!("cgroup-{}", ctx.run_step));

    let spawn_config = ContainerSpawnConfig {
        rootfs: layout.merged_dir.clone(),
        command,
        args,
        env: ctx.env_state.to_vec(),
        hostname: format!("minibox-build-{}", ctx.build_id),
        cgroup_path: cgroup_path.into(),
        capture_output: true,
        hooks: ContainerHooks::default(),
        skip_network_namespace: true,
        mounts: vec![],
        privileged: false,
        image_ref: None,
    };

    let spawn_result = builder
        .runtime
        .spawn_process(&spawn_config)
        .await
        .with_context(|| format!("spawn RUN container at step {}", ctx.step_num))?;

    // Stream captured output as build progress messages.
    #[cfg(unix)]
    if let Some(reader_fd) = spawn_result.output_reader {
        stream_run_output(reader_fd, progress_tx, ctx.step_num, total).await;
    }

    let exit_code = builder
        .runtime
        .wait_for_exit(spawn_result.runtime_id.as_deref(), spawn_result.pid)
        .await
        .with_context(|| format!("wait_for_exit at step {}", ctx.step_num))?;

    // Unmount before checking exit -- always clean up.
    let filesystem_cleanup = Arc::clone(&builder.filesystem);
    let container_dir_for_cleanup = container_dir.clone();
    if let Err(e) =
        tokio::task::spawn_blocking(move || filesystem_cleanup.cleanup(&container_dir_for_cleanup))
            .await
            .context("spawn_blocking cleanup")?
    {
        warn!(
            step = ctx.step_num,
            error = %e,
            "build: rootfs cleanup failed after RUN step"
        );
    }

    if exit_code != 0 {
        bail!(
            "RUN step {} exited with code {exit_code}: {}",
            ctx.step_num,
            ctx.instr_label
        );
    }

    // Commit the upperdir as a new layer and extend the stack.
    let image_store = Arc::clone(&builder.image_store);
    let step_tag = format!("{config_tag}:build-step-{}", ctx.run_step);
    let step_tag_for_lookup = step_tag.clone();
    let upper_dir = layout.rootfs_metadata.as_ref().map_or_else(
        || layout.merged_dir.clone(),
        |m| m.overlay_upper_dir().clone(),
    );
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
    .with_context(|| format!("commit RUN step {}", ctx.step_num))?;

    // The new layer's extracted directory becomes the top of the stack.
    let (step_name, step_tag_part) = split_image_tag(&step_tag_for_lookup);
    let new_layers = builder
        .image_store
        .get_image_layers(&step_name, &step_tag_part)
        .context("get_image_layers after RUN commit")?;
    let mut updated_stack = ctx.layer_stack.to_vec();
    let prev_len = updated_stack.len();
    for layer in new_layers.into_iter().skip(prev_len) {
        updated_stack.push(layer);
    }

    info!(
        step = ctx.step_num,
        layers = step_meta.layers.len(),
        "build: RUN step committed"
    );

    Ok((updated_stack, step_meta))
}

/// Stream output from a RUN step's captured pipe to the progress channel.
#[cfg(unix)]
async fn stream_run_output(
    reader_fd: std::os::fd::OwnedFd,
    progress_tx: &dyn ProgressSink<BuildProgress>,
    step_num: u32,
    total: u32,
) {
    use std::io::{BufRead, BufReader};
    use std::os::fd::FromRawFd;

    // SAFETY: spawn_process returned this fd to us as the read
    // end of a pipe. We take ownership via OwnedFd -> File.
    let file =
        unsafe { std::fs::File::from_raw_fd(std::os::fd::IntoRawFd::into_raw_fd(reader_fd)) };
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
            warn!("build: client disconnected during RUN output at step {step_num}");
            break;
        }
    }
}

#[async_trait]
impl ImageBuilder for MiniboxImageBuilder {
    // qual:allow(iosp) reason: "build orchestration — parse Dockerfile, execute steps, commit"
    async fn build_image(
        &self,
        context: &BuildContext,
        config: &BuildConfig,
        progress_tx: DynProgressSink<BuildProgress>,
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
        let build_arg_overrides: BTreeMap<String, String> =
            config.build_args.iter().cloned().collect();
        let mut variables: BTreeMap<String, String> = BTreeMap::new();
        let mut workdir = PathBuf::from("/");
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
                        let image_ref = ImageRef::parse(&base_image)
                            .with_context(|| format!("invalid FROM image ref: {base_image}"))?;
                        let cache_name = image_ref.cache_name();
                        let cache_tag = image_ref.tag.clone();

                        // Pull the base image if not already in the local store.
                        if !self.image_store.has_image(&cache_name, &cache_tag) {
                            info!(image = %base_image, "build: pulling base image");
                            let registry = self.registry_router.route(&image_ref);
                            registry
                                .pull_image(&image_ref)
                                .await
                                .with_context(|| format!("pull base image {base_image}"))?;
                        }

                        layer_stack = self
                            .image_store
                            .get_image_layers(&cache_name, &cache_tag)
                            .with_context(|| {
                                format!("get layer dirs for base image {base_image}")
                            })?;
                    }
                }

                Instruction::Run(shell_or_exec) => {
                    run_step += 1;
                    let run_env: Vec<String> = variables
                        .iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect();
                    let ctx = RunStepContext {
                        step_num,
                        run_step,
                        builds_dir: &builds_dir,
                        build_id: &build_id,
                        env_state: &run_env,
                        workdir: &workdir,
                        layer_stack: &layer_stack,
                        instr_label: instr_display(instr),
                        shell_or_exec,
                    };
                    let (new_stack, _step_meta) =
                        execute_run_step(self, ctx, &*progress_tx, total, &config.tag).await?;
                    layer_stack = new_stack;
                }

                Instruction::Env(pairs) => {
                    for (k, v) in pairs {
                        let value = expand_variables(v, &variables);
                        set_env(&mut env_state, k, &value);
                        variables.insert(k.clone(), value);
                    }
                }

                Instruction::Arg { name, default } => {
                    let value = build_arg_overrides.get(name).cloned().or_else(|| {
                        default
                            .as_deref()
                            .map(|value| expand_variables(value, &variables))
                    });
                    if let Some(value) = value {
                        variables.insert(name.clone(), value);
                    }
                }

                Instruction::Workdir(path) => {
                    let expanded = expand_variables(&path.to_string_lossy(), &variables);
                    workdir = resolve_workdir(&workdir, Path::new(&expanded))?;
                }

                Instruction::Cmd(ShellOrExec::Exec(args)) => {
                    cmd_override = Some(args.clone());
                }
                Instruction::Cmd(ShellOrExec::Shell(s)) => {
                    cmd_override = Some(vec!["/bin/sh".to_string(), "-c".to_string(), s.clone()]);
                }

                // COPY, ADD, ENTRYPOINT, EXPOSE, LABEL, USER
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

/// Constructs a dynamic native image-builder adapter.
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
    use crate::testing::mocks::{MockFilesystem, MockRegistry, MockRuntime};
    use minibox_core::adapters::HostnameRegistryRouter;
    use minibox_core::domain::{BuildConfig, BuildContext, DynImageRegistry};
    use minibox_core::image::manifest::{Descriptor, OciManifest};
    use minibox_core::progress::TokioProgressSink;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn build_uses_normalized_cache_key_for_bare_base_image() {
        let temp_dir = tempfile::TempDir::new().expect("create temp dir");
        let image_store =
            Arc::new(ImageStore::new(temp_dir.path().join("images")).expect("create image store"));
        image_store
            .store_manifest(
                "library/alpine",
                "3.21",
                &OciManifest {
                    schema_version: 2,
                    media_type: String::new(),
                    config: Descriptor {
                        media_type: String::new(),
                        size: 0,
                        digest: "sha256:config".to_string(),
                        platform: None,
                    },
                    layers: Vec::new(),
                },
            )
            .expect("seed normalized base image");

        let context_dir = temp_dir.path().join("context");
        std::fs::create_dir(&context_dir).expect("create context");
        std::fs::write(context_dir.join("Dockerfile"), "FROM alpine:3.21\n")
            .expect("write Dockerfile");

        let registry_router = Arc::new(HostnameRegistryRouter::new(
            Arc::new(MockRegistry::new()) as DynImageRegistry,
            std::iter::empty::<(&str, DynImageRegistry)>(),
        ));
        let builder = MiniboxImageBuilder::new(
            Arc::clone(&image_store),
            temp_dir.path().join("data"),
            Arc::new(MockFilesystem::new()),
            Arc::new(MockRuntime::new()),
            registry_router,
        );
        let (progress_tx, _progress_rx) = mpsc::channel(8);

        let result = builder
            .build_image(
                &BuildContext {
                    directory: context_dir,
                    dockerfile: PathBuf::from("Dockerfile"),
                },
                &BuildConfig {
                    tag: "example:test".to_string(),
                    build_args: Vec::new(),
                    no_cache: false,
                },
                TokioProgressSink::shared(progress_tx),
            )
            .await;

        assert!(
            result.is_ok(),
            "build should use cached base image: {result:?}"
        );
    }

    #[tokio::test]
    async fn build_creates_workdir_before_run_step() {
        let temp_dir = tempfile::TempDir::new().expect("create temp dir");
        let image_store =
            Arc::new(ImageStore::new(temp_dir.path().join("images")).expect("create image store"));
        let context_dir = temp_dir.path().join("context");
        std::fs::create_dir(&context_dir).expect("create context");
        std::fs::write(
            context_dir.join("Dockerfile"),
            "FROM scratch\nWORKDIR /app\nRUN true\n",
        )
        .expect("write Dockerfile");

        let registry_router = Arc::new(HostnameRegistryRouter::new(
            Arc::new(MockRegistry::new()) as DynImageRegistry,
            std::iter::empty::<(&str, DynImageRegistry)>(),
        ));
        let builder = MiniboxImageBuilder::new(
            Arc::clone(&image_store),
            temp_dir.path().join("data"),
            Arc::new(MockFilesystem::new()),
            Arc::new(MockRuntime::new()),
            registry_router,
        );
        let (progress_tx, _progress_rx) = mpsc::channel(8);

        let result = builder
            .build_image(
                &BuildContext {
                    directory: context_dir,
                    dockerfile: PathBuf::from("Dockerfile"),
                },
                &BuildConfig {
                    tag: "example/workdir:latest".to_string(),
                    build_args: Vec::new(),
                    no_cache: false,
                },
                TokioProgressSink::shared(progress_tx),
            )
            .await;

        assert!(result.is_ok(), "WORKDIR build should succeed: {result:?}");
        let builds_dir = temp_dir.path().join("data/builds");
        let build_dir = std::fs::read_dir(builds_dir)
            .expect("read builds dir")
            .next()
            .expect("build directory entry")
            .expect("read build directory entry")
            .path();
        assert!(build_dir.join("run-1/merged/app").is_dir());
    }

    #[tokio::test]
    async fn build_expands_arg_override_in_env() {
        let temp_dir = tempfile::TempDir::new().expect("create temp dir");
        let image_store =
            Arc::new(ImageStore::new(temp_dir.path().join("images")).expect("create image store"));
        let context_dir = temp_dir.path().join("context");
        std::fs::create_dir(&context_dir).expect("create context");
        std::fs::write(
            context_dir.join("Dockerfile"),
            "FROM scratch\nARG BUILD_DATE=unknown\nENV BUILD_DATE=${BUILD_DATE}\n",
        )
        .expect("write Dockerfile");

        let registry_router = Arc::new(HostnameRegistryRouter::new(
            Arc::new(MockRegistry::new()) as DynImageRegistry,
            std::iter::empty::<(&str, DynImageRegistry)>(),
        ));
        let builder = MiniboxImageBuilder::new(
            Arc::clone(&image_store),
            temp_dir.path().join("data"),
            Arc::new(MockFilesystem::new()),
            Arc::new(MockRuntime::new()),
            registry_router,
        );
        let (progress_tx, _progress_rx) = mpsc::channel(8);

        builder
            .build_image(
                &BuildContext {
                    directory: context_dir,
                    dockerfile: PathBuf::from("Dockerfile"),
                },
                &BuildConfig {
                    tag: "example/env:latest".to_string(),
                    build_args: vec![("BUILD_DATE".to_string(), "2026-09-20".to_string())],
                    no_cache: false,
                },
                TokioProgressSink::shared(progress_tx),
            )
            .await
            .expect("build image");

        let config = std::fs::read_to_string(
            image_store
                .layers_dir_pub("example/env", "latest")
                .expect("resolve layers dir")
                .join("config.json"),
        )
        .expect("read image config");
        let config: serde_json::Value = serde_json::from_str(&config).expect("parse image config");
        assert_eq!(config["config"]["Env"][0], "BUILD_DATE=2026-09-20");
    }

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
