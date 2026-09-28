//! Minibox image builder — executes a Dockerfile instruction-by-instruction.
//!
//! A native stage graph tracks inherited rootfs layers and OCI metadata.
//! `RUN` captures writable overlay diffs, while secure `COPY`/local `ADD`
//! operations create filesystem layers without following context symlinks.
//! The final stage is assembled with its complete ordered descriptor stack.

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use minibox_core::as_any;
use minibox_core::domain::{
    BuildConfig, BuildContext, BuildProgress, ContainerHooks, ContainerSpawnConfig,
    DynContainerRuntime, DynFilesystemProvider, DynImageBuilder, DynProgressSink,
    DynRegistryRouter, ImageBuilder, ImageMetadata, ProgressSink,
};
use minibox_core::image::ImageStore;
use minibox_core::image::dockerfile::{AddSource, FromSource, StageReference};
use minibox_core::image::reference::ImageRef;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::adapters::commit::{
    LayerInput, commit_layer_stack_to_image, materialize_layer_directories, store_snapshot_layer,
};
use crate::image::dockerfile::{Instruction, ShellOrExec, parse};

mod copy;
mod dockerignore;

use copy::{CopyRequest, copy_sources};
use dockerignore::DockerIgnore;

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

#[derive(Debug, Clone)]
struct StageState {
    alias: Option<String>,
    layers: Vec<LayerInput>,
    config: Value,
    env: Vec<String>,
    variables: BTreeMap<String, String>,
    workdir: PathBuf,
}

impl StageState {
    fn scratch(alias: Option<String>) -> Self {
        Self {
            alias,
            layers: Vec::new(),
            config: json!({
                "architecture": match std::env::consts::ARCH {
                    "aarch64" => "arm64",
                    "x86_64" => "amd64",
                    other => other,
                },
                "os": "linux",
                "config": {}
            }),
            env: Vec::new(),
            variables: BTreeMap::new(),
            workdir: PathBuf::from("/"),
        }
    }

    fn layer_paths(&self) -> Vec<PathBuf> {
        self.layers
            .iter()
            .map(|layer| layer.directory.clone())
            .collect()
    }
}

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
        Instruction::Volume(paths) => format!("VOLUME {} paths", paths.len()),
        Instruction::Comment(_) => "# comment".to_string(),
    }
}

/// Parse `"image:tag"` or `"image"` (defaults to `"latest"`).
#[cfg(test)]
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

fn image_config_mut(config: &mut Value) -> Result<&mut Map<String, Value>> {
    let root = config
        .as_object_mut()
        .context("OCI image config root must be an object")?;
    let value = root
        .entry("config")
        .or_insert_with(|| Value::Object(Map::new()));
    value
        .as_object_mut()
        .context("OCI image config.config must be an object")
}

fn image_config(config: &Value) -> Option<&Map<String, Value>> {
    config.get("config").and_then(Value::as_object)
}

fn string_array_field(config: &Value, field: &str) -> Vec<String> {
    image_config(config)
        .and_then(|object| object.get(field))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn string_field(config: &Value, field: &str) -> Option<String> {
    image_config(config)
        .and_then(|object| object.get(field))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn command_value(command: &ShellOrExec) -> Vec<String> {
    match command {
        ShellOrExec::Exec(arguments) => arguments.clone(),
        ShellOrExec::Shell(command) => {
            vec!["/bin/sh".to_string(), "-c".to_string(), command.clone()]
        }
    }
}

fn find_copy_stage<'a>(
    stages: &'a [StageState],
    reference: &StageReference,
) -> Result<&'a StageState> {
    match reference {
        StageReference::Index(index) => stages
            .get(*index)
            .with_context(|| format!("COPY --from references missing stage index {index}")),
        StageReference::Alias(alias) => stages
            .iter()
            .find(|stage| {
                stage
                    .alias
                    .as_deref()
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(alias))
            })
            .with_context(|| format!("COPY --from references missing stage alias {alias:?}")),
    }
}

fn validate_stage_graph(instructions: &[Instruction]) -> Result<()> {
    let mut aliases = Vec::<String>::new();
    let mut current_alias = None::<String>;
    let mut stage_count = 0usize;
    for instruction in instructions {
        match instruction {
            Instruction::From { alias, source, .. } => {
                if let Some(completed_alias) = current_alias.take() {
                    aliases.push(completed_alias);
                }
                if let FromSource::Stage(StageReference::Index(index)) = source
                    && *index >= stage_count
                {
                    bail!("FROM references missing stage index {index}");
                }
                if let Some(alias) = alias {
                    if aliases
                        .iter()
                        .any(|existing| existing.eq_ignore_ascii_case(alias))
                    {
                        bail!("duplicate build stage alias: {alias}");
                    }
                    current_alias = Some(alias.clone());
                }
                stage_count += 1;
            }
            Instruction::Copy {
                from: Some(from), ..
            } => match from {
                StageReference::Index(index) if *index + 1 >= stage_count => {
                    bail!("COPY --from references missing stage index {index}")
                }
                StageReference::Alias(alias)
                    if !aliases
                        .iter()
                        .any(|existing| existing.eq_ignore_ascii_case(alias)) =>
                {
                    bail!("COPY --from references missing stage alias {alias:?}")
                }
                _ => {}
            },
            Instruction::Add { srcs, .. }
                if srcs
                    .iter()
                    .any(|source| matches!(source, AddSource::Url(_))) =>
            {
                bail!("remote URL ADD is unsupported by the native builder")
            }
            _ => {}
        }
    }
    if stage_count == 0 {
        bail!("Dockerfile has no build stages");
    }
    Ok(())
}

async fn load_external_stage(
    builder: &MiniboxImageBuilder,
    image: &str,
    tag: &str,
    alias: Option<String>,
) -> Result<StageState> {
    let base_image = format!("{image}:{tag}");
    let image_ref = ImageRef::parse(&base_image)
        .with_context(|| format!("invalid FROM image ref: {base_image}"))?;
    let cache_name = image_ref.cache_name();
    let cache_tag = image_ref.tag.clone();
    if !builder.image_store.has_image(&cache_name, &cache_tag) {
        info!(image = %base_image, "build: pulling base image");
        builder
            .registry_router
            .route(&image_ref)
            .pull_image(&image_ref)
            .await
            .with_context(|| format!("pull base image {base_image}"))?;
    }

    let manifest = builder
        .image_store
        .load_manifest_pub(&cache_name, &cache_tag)
        .with_context(|| format!("load manifest for base image {base_image}"))?;
    let directories = builder
        .image_store
        .get_image_layers(&cache_name, &cache_tag)
        .with_context(|| format!("get layer dirs for base image {base_image}"))?;
    if manifest.layers.len() != directories.len() {
        bail!("base image manifest and extracted layer count differ for {base_image}");
    }
    let config = match builder
        .image_store
        .load_config_blob_pub(&cache_name, &cache_tag)
    {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .with_context(|| format!("parse config for base image {base_image}"))?,
        Err(_) => StageState::scratch(None).config,
    };
    let layers = manifest
        .layers
        .into_iter()
        .zip(directories)
        .map(|(_descriptor, directory)| LayerInput::existing(directory))
        .collect();
    let env = string_array_field(&config, "Env");
    let variables = env
        .iter()
        .filter_map(|entry| entry.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    let workdir = string_field(&config, "WorkingDir")
        .filter(|workdir| !workdir.is_empty())
        .map_or_else(|| PathBuf::from("/"), PathBuf::from);
    Ok(StageState {
        alias,
        layers,
        config,
        env,
        variables,
        workdir,
    })
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

fn validate_build_context(context: &BuildContext) -> Result<(PathBuf, PathBuf, PathBuf)> {
    if !context.directory.is_absolute() {
        bail!("build context directory must be absolute");
    }
    if context.dockerfile.as_os_str().is_empty() || context.dockerfile.is_absolute() {
        bail!("Dockerfile path must be a non-empty relative path");
    }
    let mut relative_dockerfile = PathBuf::new();
    for component in context.dockerfile.components() {
        match component {
            Component::Normal(part) => relative_dockerfile.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("Dockerfile path must be relative and parent-free")
            }
        }
    }
    let directory = context
        .directory
        .canonicalize()
        .context("canonicalize build context")?;
    let dockerfile = directory.join(&relative_dockerfile);
    let canonical_dockerfile = dockerfile
        .canonicalize()
        .with_context(|| format!("canonicalize Dockerfile {}", dockerfile.display()))?;
    if !canonical_dockerfile.starts_with(&directory) {
        bail!("Dockerfile path escapes the build context");
    }
    Ok((directory, canonical_dockerfile, relative_dockerfile))
}

struct FilesystemStep {
    image_store: Arc<ImageStore>,
    current_layers: Vec<LayerInput>,
    source_layers: Option<Vec<LayerInput>>,
    context_directory: PathBuf,
    sources: Vec<PathBuf>,
    destination: PathBuf,
    workdir: PathBuf,
    ignore: Option<DockerIgnore>,
    excluded_dockerfile: Option<PathBuf>,
    chown: Option<minibox_core::image::dockerfile::CopyOwnership>,
    chmod: Option<u32>,
    working_root: PathBuf,
    source_root: PathBuf,
    snapshot_name: String,
    snapshot_tag: String,
}

fn execute_filesystem_step(step: FilesystemStep) -> Result<LayerInput> {
    materialize_layer_directories(&step.current_layers, &step.working_root)?;
    let source_root = if let Some(source_layers) = &step.source_layers {
        materialize_layer_directories(source_layers, &step.source_root)?;
        &step.source_root
    } else {
        &step.context_directory
    };
    copy_sources(&CopyRequest {
        source_root,
        sources: &step.sources,
        destination: &step.destination,
        workdir: &step.workdir,
        destination_rootfs: &step.working_root,
        layer_dir: &step.working_root,
        ignore: step.ignore.as_ref(),
        excluded_source: step.excluded_dockerfile.as_deref(),
        chown: step.chown.as_ref(),
        chmod: step.chmod,
    })?;
    Ok(store_snapshot_layer(
        &step.image_store,
        &step.working_root,
        &step.snapshot_name,
        &step.snapshot_tag,
    )?
    .layer)
}

fn create_workdir_snapshot(
    image_store: &ImageStore,
    layers: &[LayerInput],
    workdir: &Path,
    working_root: &Path,
    snapshot_name: &str,
    snapshot_tag: &str,
) -> Result<LayerInput> {
    materialize_layer_directories(layers, working_root)?;
    let relative = workdir
        .strip_prefix("/")
        .context("WORKDIR must resolve to an absolute path")?;
    let target = working_root.join(relative);
    std::fs::create_dir_all(&target)
        .with_context(|| format!("create WORKDIR {}", workdir.display()))?;
    Ok(store_snapshot_layer(image_store, working_root, snapshot_name, snapshot_tag)?.layer)
}

fn is_archive_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    [
        ".tar", ".tar.gz", ".tgz", ".tar.bz2", ".tbz", ".tar.xz", ".txz",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
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
) -> Result<LayerInput> {
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

    let exit_result = builder
        .runtime
        .wait_for_exit(spawn_result.runtime_id.as_deref(), spawn_result.pid)
        .await
        .with_context(|| format!("wait_for_exit at step {}", ctx.step_num));

    let snapshot_result = if matches!(exit_result, Ok(0)) {
        let image_store = Arc::clone(&builder.image_store);
        let merged_dir = layout.merged_dir.to_path_buf();
        let snapshot_name = format!("_minibox_build/{}", ctx.build_id);
        let snapshot_tag = format!("run-{}", ctx.run_step);
        Some(
            match tokio::task::spawn_blocking(move || {
                store_snapshot_layer(&image_store, &merged_dir, &snapshot_name, &snapshot_tag)
            })
            .await
            {
                Ok(result) => result,
                Err(error) => {
                    Err(anyhow::Error::new(error).context("spawn_blocking snapshot RUN rootfs"))
                }
            },
        )
    } else {
        None
    };

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

    let exit_code = exit_result?;
    if exit_code != 0 {
        bail!(
            "RUN step {} exited with code {exit_code}: {}",
            ctx.step_num,
            ctx.instr_label
        );
    }

    let snapshot = snapshot_result
        .context("RUN snapshot missing after successful process")?
        .with_context(|| format!("snapshot RUN step {}", ctx.step_num))?;

    info!(
        step = ctx.step_num,
        path = %snapshot.layer.directory.display(),
        "build: RUN step captured"
    );

    Ok(snapshot.layer)
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
        let context_for_validation = context.clone();
        let (context_directory, dockerfile_path, excluded_dockerfile) =
            tokio::task::spawn_blocking(move || validate_build_context(&context_for_validation))
                .await
                .context("spawn_blocking validate build context")??;
        let dockerfile_content = tokio::fs::read_to_string(&dockerfile_path)
            .await
            .with_context(|| format!("read Dockerfile at {}", dockerfile_path.display()))?;

        let instructions = parse(&dockerfile_content).context("parse Dockerfile")?;
        validate_stage_graph(&instructions).context("plan Dockerfile stage graph")?;

        let steps: Vec<&Instruction> = instructions
            .iter()
            .filter(|i| !matches!(i, Instruction::Comment(_)))
            .collect();
        let total = steps.len() as u32;

        let raw_uuid = Uuid::new_v4().simple().to_string();
        let build_id = raw_uuid[..16].to_string();
        let builds_dir = self.data_dir.join("builds").join(&build_id);

        let build_arg_overrides: BTreeMap<String, String> =
            config.build_args.iter().cloned().collect();
        let mut global_variables = BTreeMap::<String, String>::new();
        let ignore_context = context_directory.clone();
        let ignore = tokio::task::spawn_blocking(move || DockerIgnore::load(&ignore_context))
            .await
            .context("spawn_blocking load .dockerignore")??;
        let mut stages = Vec::<StageState>::new();
        let mut current = None::<StageState>;
        let mut run_step = 0u32;
        let mut copy_step = 0u32;

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
                Instruction::From {
                    image, tag, alias, ..
                } => {
                    if let Some(stage) = current.take() {
                        stages.push(stage);
                    }
                    if let Some(alias) = alias
                        && stages.iter().any(|stage| {
                            stage
                                .alias
                                .as_deref()
                                .is_some_and(|existing| existing.eq_ignore_ascii_case(alias))
                        })
                    {
                        bail!("duplicate build stage alias: {alias}");
                    }
                    let expanded_image = expand_variables(image, &global_variables);
                    let expanded_tag = expand_variables(tag, &global_variables);
                    if expanded_image.is_empty() || expanded_tag.is_empty() {
                        bail!("FROM expansion produced an empty image or tag");
                    }
                    let stage_reference = if expanded_tag == "latest" {
                        expanded_image.parse::<usize>().map_or_else(
                            |_| {
                                stages.iter().position(|stage| {
                                    stage.alias.as_deref().is_some_and(|candidate| {
                                        candidate.eq_ignore_ascii_case(&expanded_image)
                                    })
                                })
                            },
                            Some,
                        )
                    } else {
                        None
                    };
                    let mut stage = if expanded_image == "scratch" && stage_reference.is_none() {
                        StageState::scratch(alias.clone())
                    } else if let Some(index) = stage_reference {
                        let mut inherited = stages
                            .get(index)
                            .with_context(|| {
                                format!("FROM references missing stage index {index}")
                            })?
                            .clone();
                        inherited.alias.clone_from(alias);
                        inherited
                    } else {
                        load_external_stage(self, &expanded_image, &expanded_tag, alias.clone())
                            .await?
                    };
                    for (key, value) in &global_variables {
                        stage.variables.insert(key.clone(), value.clone());
                    }
                    for (key, value) in &build_arg_overrides {
                        stage.variables.insert(key.clone(), value.clone());
                    }
                    current = Some(stage);
                }

                Instruction::Run(shell_or_exec) => {
                    run_step += 1;
                    let stage = current.as_mut().context("RUN appears before FROM")?;
                    if string_field(&stage.config, "User").is_some_and(|user| !user.is_empty()) {
                        bail!(
                            "RUN after USER is unsupported by the current runtime contract; refusing to execute as root"
                        );
                    }
                    let run_env: Vec<String> = stage
                        .variables
                        .iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect();
                    let layer_stack = stage.layer_paths();
                    let ctx = RunStepContext {
                        step_num,
                        run_step,
                        builds_dir: &builds_dir,
                        build_id: &build_id,
                        env_state: &run_env,
                        workdir: &stage.workdir,
                        layer_stack: &layer_stack,
                        instr_label: instr_display(instr),
                        shell_or_exec,
                    };
                    let layer = execute_run_step(self, ctx, &*progress_tx, total).await?;
                    stage.layers = vec![layer];
                }

                Instruction::Env(pairs) => {
                    let stage = current.as_mut().context("ENV appears before FROM")?;
                    for (k, v) in pairs {
                        let value = expand_variables(v, &stage.variables);
                        set_env(&mut stage.env, k, &value);
                        stage.variables.insert(k.clone(), value);
                    }
                    image_config_mut(&mut stage.config)?
                        .insert("Env".to_string(), json!(stage.env));
                }

                Instruction::Arg { name, default } => {
                    let variables = current
                        .as_ref()
                        .map_or(&global_variables, |stage| &stage.variables);
                    let value = build_arg_overrides.get(name).cloned().or_else(|| {
                        default
                            .as_deref()
                            .map(|value| expand_variables(value, variables))
                    });
                    if let Some(value) = value {
                        if let Some(stage) = current.as_mut() {
                            stage.variables.insert(name.clone(), value);
                        } else {
                            global_variables.insert(name.clone(), value);
                        }
                    }
                }

                Instruction::Workdir(path) => {
                    copy_step += 1;
                    let stage = current.as_mut().context("WORKDIR appears before FROM")?;
                    let expanded = expand_variables(&path.to_string_lossy(), &stage.variables);
                    stage.workdir = resolve_workdir(&stage.workdir, Path::new(&expanded))?;
                    image_config_mut(&mut stage.config)?.insert(
                        "WorkingDir".to_string(),
                        json!(stage.workdir.to_string_lossy()),
                    );
                    let image_store = Arc::clone(&self.image_store);
                    let layers = stage.layers.clone();
                    let workdir = stage.workdir.clone();
                    let working_root = builds_dir.join(format!("workdir-{copy_step}-rootfs"));
                    let snapshot_name = format!("_minibox_build/{build_id}");
                    let snapshot_tag = format!("workdir-{copy_step}");
                    let layer = tokio::task::spawn_blocking(move || {
                        create_workdir_snapshot(
                            &image_store,
                            &layers,
                            &workdir,
                            &working_root,
                            &snapshot_name,
                            &snapshot_tag,
                        )
                    })
                    .await
                    .context("spawn_blocking create WORKDIR snapshot")??;
                    stage.layers = vec![layer];
                }

                Instruction::Cmd(command) => {
                    let stage = current.as_mut().context("CMD appears before FROM")?;
                    image_config_mut(&mut stage.config)?
                        .insert("Cmd".to_string(), json!(command_value(command)));
                }
                Instruction::Entrypoint(command) => {
                    let stage = current.as_mut().context("ENTRYPOINT appears before FROM")?;
                    image_config_mut(&mut stage.config)?
                        .insert("Entrypoint".to_string(), json!(command_value(command)));
                }
                Instruction::User { name, group } => {
                    let stage = current.as_mut().context("USER appears before FROM")?;
                    let user = group
                        .as_ref()
                        .map_or_else(|| name.clone(), |group| format!("{name}:{group}"));
                    image_config_mut(&mut stage.config)?.insert("User".to_string(), json!(user));
                }
                Instruction::Label(pairs) => {
                    let stage = current.as_mut().context("LABEL appears before FROM")?;
                    let expanded_pairs = pairs
                        .iter()
                        .map(|(key, value)| {
                            (key.clone(), expand_variables(value, &stage.variables))
                        })
                        .collect::<Vec<_>>();
                    let config = image_config_mut(&mut stage.config)?;
                    let labels = config
                        .entry("Labels")
                        .or_insert_with(|| Value::Object(Map::new()))
                        .as_object_mut()
                        .context("OCI config Labels must be an object")?;
                    for (key, value) in expanded_pairs {
                        labels.insert(key, json!(value));
                    }
                }
                Instruction::Expose { port, proto } => {
                    let stage = current.as_mut().context("EXPOSE appears before FROM")?;
                    let config = image_config_mut(&mut stage.config)?;
                    config
                        .entry("ExposedPorts")
                        .or_insert_with(|| Value::Object(Map::new()))
                        .as_object_mut()
                        .context("OCI config ExposedPorts must be an object")?
                        .insert(format!("{port}/{proto}"), json!({}));
                }
                Instruction::Volume(paths) => {
                    let stage = current.as_mut().context("VOLUME appears before FROM")?;
                    let resolved: Vec<String> = paths
                        .iter()
                        .map(|path| resolve_workdir(&stage.workdir, path))
                        .collect::<Result<Vec<_>>>()?
                        .into_iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect();
                    let config = image_config_mut(&mut stage.config)?;
                    let volumes = config
                        .entry("Volumes")
                        .or_insert_with(|| Value::Object(Map::new()))
                        .as_object_mut()
                        .context("OCI config Volumes must be an object")?;
                    for path in resolved {
                        volumes.insert(path, json!({}));
                    }
                }
                Instruction::Copy {
                    srcs,
                    dest,
                    from,
                    chown,
                    chmod,
                } => {
                    copy_step += 1;
                    let stage = current.as_ref().context("COPY appears before FROM")?;
                    let (source_layers, sources, ignore_rules, excluded) = if let Some(reference) =
                        from
                    {
                        let source_stage = find_copy_stage(&stages, reference)?;
                        (
                            Some(source_stage.layers.clone()),
                            srcs.iter()
                                .map(|path| path.strip_prefix("/").unwrap_or(path).to_path_buf())
                                .collect::<Vec<_>>(),
                            None,
                            None,
                        )
                    } else {
                        (
                            None,
                            srcs.clone(),
                            Some(&ignore),
                            Some(excluded_dockerfile.as_path()),
                        )
                    };
                    let filesystem_step = FilesystemStep {
                        image_store: Arc::clone(&self.image_store),
                        current_layers: stage.layers.clone(),
                        source_layers,
                        context_directory: context_directory.clone(),
                        sources,
                        destination: dest.clone(),
                        workdir: stage.workdir.clone(),
                        ignore: ignore_rules.cloned(),
                        excluded_dockerfile: excluded.map(Path::to_path_buf),
                        chown: chown.clone(),
                        chmod: *chmod,
                        working_root: builds_dir.join(format!("copy-{copy_step}-rootfs")),
                        source_root: builds_dir.join(format!("copy-{copy_step}-source-rootfs")),
                        snapshot_name: format!("_minibox_build/{build_id}"),
                        snapshot_tag: format!("copy-{copy_step}"),
                    };
                    let layer = tokio::task::spawn_blocking(move || {
                        execute_filesystem_step(filesystem_step)
                    })
                    .await
                    .context("spawn_blocking execute COPY")??;
                    current.as_mut().context("COPY stage disappeared")?.layers = vec![layer];
                }
                Instruction::Add {
                    srcs,
                    dest,
                    chown,
                    chmod,
                } => {
                    let mut local_sources = Vec::with_capacity(srcs.len());
                    for source in srcs {
                        match source {
                            AddSource::Local(path) => {
                                if is_archive_path(path) {
                                    bail!(
                                        "local archive ADD is unsupported; use COPY or extract the archive explicitly: {}",
                                        path.display()
                                    );
                                }
                                local_sources.push(path.clone());
                            }
                            AddSource::Url(url) => {
                                bail!("remote URL ADD is unsupported: {url}")
                            }
                        }
                    }
                    copy_step += 1;
                    let stage = current.as_ref().context("ADD appears before FROM")?;
                    let filesystem_step = FilesystemStep {
                        image_store: Arc::clone(&self.image_store),
                        current_layers: stage.layers.clone(),
                        source_layers: None,
                        context_directory: context_directory.clone(),
                        sources: local_sources,
                        destination: dest.clone(),
                        workdir: stage.workdir.clone(),
                        ignore: Some(ignore.clone()),
                        excluded_dockerfile: Some(excluded_dockerfile.clone()),
                        chown: chown.clone(),
                        chmod: *chmod,
                        working_root: builds_dir.join(format!("add-{copy_step}-rootfs")),
                        source_root: builds_dir.join(format!("add-{copy_step}-source-rootfs")),
                        snapshot_name: format!("_minibox_build/{build_id}"),
                        snapshot_tag: format!("add-{copy_step}"),
                    };
                    let layer = tokio::task::spawn_blocking(move || {
                        execute_filesystem_step(filesystem_step)
                    })
                    .await
                    .context("spawn_blocking execute ADD")??;
                    current.as_mut().context("ADD stage disappeared")?.layers = vec![layer];
                }
                Instruction::Comment(_) => {}
            }
        }

        if let Some(stage) = current.take() {
            stages.push(stage);
        }
        let mut final_stage = stages
            .pop()
            .context("Dockerfile produced no build stages")?;
        if final_stage.layers.is_empty() {
            let empty_layer = builds_dir.join("final-empty-layer");
            std::fs::create_dir_all(&empty_layer).context("create empty final layer")?;
            final_stage.layers.push(LayerInput::generated(empty_layer));
        }

        let image_store = Arc::clone(&self.image_store);
        let target_tag = config.tag.clone();
        let target_tag_for_log = target_tag.clone();
        let layers = final_stage.layers;
        let image_config = final_stage.config;
        let meta = tokio::task::spawn_blocking(move || {
            commit_layer_stack_to_image(image_store, &layers, &target_tag, image_config)
        })
        .await
        .context("spawn_blocking build commit")?
        .context("commit build result")?;

        info!(tag = %target_tag_for_log, "build: complete");
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
    use minibox_core::domain::{
        BackendRootfsMetadata, BuildConfig, BuildContext, ChildInit, DynImageRegistry,
        RootfsLayout, RootfsSetup,
    };
    use minibox_core::image::manifest::{Descriptor, OciManifest};
    use minibox_core::progress::TokioProgressSink;
    use std::sync::Mutex;
    use tokio::sync::mpsc;

    #[derive(Debug)]
    struct CleanupValidatingFilesystem {
        image_root: PathBuf,
        setups: Mutex<Vec<Vec<PathBuf>>>,
    }

    impl CleanupValidatingFilesystem {
        fn new(image_root: PathBuf) -> Self {
            Self {
                image_root,
                setups: Mutex::new(Vec::new()),
            }
        }
    }

    minibox_core::as_any!(CleanupValidatingFilesystem);

    impl RootfsSetup for CleanupValidatingFilesystem {
        fn setup_rootfs(&self, layers: &[PathBuf], container_dir: &Path) -> Result<RootfsLayout> {
            for layer in layers {
                if !layer.starts_with(&self.image_root) || !layer.is_dir() {
                    bail!(
                        "test filesystem rejected non-durable layer {}",
                        layer.display()
                    );
                }
            }
            self.setups
                .lock()
                .expect("lock setup records")
                .push(layers.to_vec());
            let merged = container_dir.join("merged");
            let layer_inputs = layers
                .iter()
                .cloned()
                .map(LayerInput::existing)
                .collect::<Vec<_>>();
            materialize_layer_directories(&layer_inputs, &merged)?;
            let upper = container_dir.join("upper");
            std::fs::create_dir_all(&upper)?;
            Ok(RootfsLayout {
                merged_dir: merged.into(),
                rootfs_metadata: Some(BackendRootfsMetadata::Overlay {
                    upper_dir: upper.into(),
                    metadata: std::collections::HashMap::new(),
                }),
                source_image_ref: None,
            })
        }

        fn cleanup(&self, container_dir: &Path) -> Result<()> {
            if container_dir.exists() {
                std::fs::remove_dir_all(container_dir)?;
            }
            Ok(())
        }
    }

    impl ChildInit for CleanupValidatingFilesystem {
        fn pivot_root(&self, _new_root: &Path) -> Result<()> {
            Ok(())
        }
    }

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

        let config: serde_json::Value = serde_json::from_slice(
            &image_store
                .load_config_blob_pub("example/env", "latest")
                .expect("read image config"),
        )
        .expect("parse image config");
        assert_eq!(config["config"]["Env"][0], "BUILD_DATE=2026-09-20");
    }

    async fn build_fixture(
        dockerfile: &str,
        files: &[(&str, &str)],
    ) -> (tempfile::TempDir, Arc<ImageStore>, Result<ImageMetadata>) {
        let temp_dir = tempfile::TempDir::new().expect("create temp dir");
        let image_store =
            Arc::new(ImageStore::new(temp_dir.path().join("images")).expect("create image store"));
        let context_dir = temp_dir.path().join("context");
        std::fs::create_dir(&context_dir).expect("create context");
        std::fs::write(context_dir.join("Dockerfile"), dockerfile).expect("write Dockerfile");
        for (path, contents) in files {
            let target = context_dir.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).expect("create fixture parent");
            }
            std::fs::write(target, contents).expect("write fixture file");
        }

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
        let (progress_tx, _progress_rx) = mpsc::channel(32);
        let result = builder
            .build_image(
                &BuildContext {
                    directory: context_dir,
                    dockerfile: PathBuf::from("Dockerfile"),
                },
                &BuildConfig {
                    tag: "example/native-stage:latest".to_string(),
                    build_args: Vec::new(),
                    no_cache: false,
                },
                TokioProgressSink::shared(progress_tx),
            )
            .await;
        (temp_dir, image_store, result)
    }

    #[tokio::test]
    async fn build_copies_context_and_persists_complete_metadata() {
        let dockerfile = r#"FROM scratch
COPY ["app.txt", "/srv/app.txt"]
ENV MODE=production
WORKDIR /srv
USER 1000:1001
LABEL org.example.kind=test
EXPOSE 8080/udp
VOLUME ["/data"]
ENTRYPOINT ["/bin/app"]
CMD ["serve"]
"#;
        let (_temp, store, result) = build_fixture(dockerfile, &[("app.txt", "hello")]).await;
        result.expect("native metadata build should succeed");

        let layers = store
            .get_image_layers("example/native-stage", "latest")
            .expect("load final layers");
        assert_eq!(
            std::fs::read_to_string(layers[0].join("srv/app.txt")).expect("read copied file"),
            "hello"
        );

        let config: serde_json::Value = serde_json::from_slice(
            &store
                .load_config_blob_pub("example/native-stage", "latest")
                .expect("load config"),
        )
        .expect("parse config");
        assert_eq!(config["config"]["WorkingDir"], "/srv");
        assert_eq!(config["config"]["User"], "1000:1001");
        assert_eq!(config["config"]["Entrypoint"][0], "/bin/app");
        assert_eq!(config["config"]["Cmd"][0], "serve");
        assert_eq!(config["config"]["Labels"]["org.example.kind"], "test");
        assert!(config["config"]["ExposedPorts"]["8080/udp"].is_object());
        assert!(config["config"]["Volumes"]["/data"].is_object());
    }

    #[tokio::test]
    async fn build_stage_copy_uses_alias_and_dockerignore_negation() {
        let dockerfile = "FROM scratch AS build\nCOPY . /source/\nFROM build AS final\nCOPY --from=0 /source/keep.txt /result/keep.txt\n";
        let (_temp, store, result) = build_fixture(
            dockerfile,
            &[
                (".dockerignore", "*.txt\n!keep.txt\n"),
                ("keep.txt", "keep"),
                ("drop.txt", "drop"),
            ],
        )
        .await;
        result.expect("multi-stage copy should succeed");

        let layers = store
            .get_image_layers("example/native-stage", "latest")
            .expect("load final layers");
        assert!(
            layers
                .iter()
                .any(|layer| layer.join("result/keep.txt").is_file())
        );
        assert!(
            !layers
                .iter()
                .any(|layer| layer.join("source/drop.txt").exists())
        );
    }

    #[tokio::test]
    async fn build_rejects_copy_source_traversal() {
        let (_temp, _store, result) =
            build_fixture("FROM scratch\nCOPY ../secret /secret\n", &[]).await;
        let error = result.expect_err("source traversal must fail");
        assert!(error.to_string().contains("traversal"), "error: {error:#}");
    }

    #[test]
    fn repository_dockerfile_parses_and_plans_three_stages() {
        let instructions = parse(include_str!("../../../../Dockerfile"))
            .expect("repository Dockerfile should parse");
        validate_stage_graph(&instructions).expect("repository stage graph should plan");
        let stages = instructions
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::From { .. }))
            .count();
        assert_eq!(stages, 3);
        assert!(matches!(
            instructions
                .iter()
                .rev()
                .find(|instruction| matches!(instruction, Instruction::From { .. })),
            Some(Instruction::From { image, alias: Some(alias), .. })
                if image == "builder" && alias == "test"
        ));
    }

    #[tokio::test]
    async fn build_rejects_missing_copy_stage_before_execution() {
        let (_temp, _store, result) =
            build_fixture("FROM scratch\nCOPY --from=missing /file /file\n", &[]).await;
        let error = result.expect_err("missing stage must fail");
        assert!(
            error.to_string().contains("stage graph"),
            "error: {error:#}"
        );
    }

    #[tokio::test]
    async fn build_flattens_base_with_valid_blob_diff_id_and_oci_config() {
        let temp_dir = tempfile::TempDir::new().expect("create temp dir");
        let image_store =
            Arc::new(ImageStore::new(temp_dir.path().join("images")).expect("create store"));
        let base_layers = image_store
            .layers_dir_pub("library/base", "latest")
            .expect("base layers dir");
        let base_layer = base_layers.join("sha256_base");
        std::fs::create_dir_all(&base_layer).expect("create base layer");
        std::fs::write(base_layer.join("base.txt"), "base").expect("write base file");
        image_store
            .store_config_blob(
                "library/base",
                "latest",
                br#"{"architecture":"amd64","os":"linux","config":{"Env":["BASE=1"],"Entrypoint":["/base"]},"rootfs":{"type":"layers","diff_ids":["sha256:base-diff"]}}"#,
            )
            .expect("store base config");
        image_store
            .store_manifest(
                "library/base",
                "latest",
                &OciManifest {
                    schema_version: 2,
                    media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
                    config: Descriptor {
                        media_type: "application/vnd.oci.image.config.v1+json".to_string(),
                        size: 0,
                        digest: "sha256:config".to_string(),
                        platform: None,
                    },
                    layers: vec![Descriptor {
                        media_type: "application/vnd.oci.image.layer.v1.tar".to_string(),
                        size: 4,
                        digest: "sha256:base".to_string(),
                        platform: None,
                    }],
                },
            )
            .expect("store base manifest");

        let context = temp_dir.path().join("context");
        std::fs::create_dir(&context).expect("create context");
        std::fs::write(
            context.join("Dockerfile"),
            "FROM base:latest\nCOPY app.txt /app.txt\nENV CHILD=1\n",
        )
        .expect("write Dockerfile");
        std::fs::write(context.join("app.txt"), "child").expect("write context file");
        let builder = MiniboxImageBuilder::new(
            Arc::clone(&image_store),
            temp_dir.path().join("data"),
            Arc::new(MockFilesystem::new()),
            Arc::new(MockRuntime::new()),
            Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                std::iter::empty::<(&str, DynImageRegistry)>(),
            )),
        );
        let (progress_tx, _progress_rx) = mpsc::channel(16);
        builder
            .build_image(
                &BuildContext {
                    directory: context,
                    dockerfile: PathBuf::from("Dockerfile"),
                },
                &BuildConfig {
                    tag: "example/inherited:latest".to_string(),
                    build_args: vec![],
                    no_cache: false,
                },
                TokioProgressSink::shared(progress_tx),
            )
            .await
            .expect("build inherited image");

        let manifest = image_store
            .load_manifest_pub("example/inherited", "latest")
            .expect("load result manifest");
        assert_eq!(manifest.layers.len(), 1);
        let layers = image_store
            .get_image_layers("example/inherited", "latest")
            .expect("load result layers");
        assert_eq!(
            std::fs::read_to_string(layers[0].join("base.txt")).expect("base file"),
            "base"
        );
        assert_eq!(
            std::fs::read_to_string(layers[0].join("app.txt")).expect("app file"),
            "child"
        );
        let digest_key = manifest.layers[0].digest.replace(':', "_");
        let blob = std::fs::read(
            image_store
                .layers_dir_pub("example/inherited", "latest")
                .expect("result layers dir")
                .join(format!("{digest_key}.tar")),
        )
        .expect("matching raw layer blob");
        use sha2::{Digest, Sha256};
        assert_eq!(
            manifest.layers[0].digest,
            format!("sha256:{:x}", Sha256::digest(&blob))
        );
        let mut tar_bytes = Vec::new();
        std::io::copy(
            &mut flate2::read::GzDecoder::new(blob.as_slice()),
            &mut tar_bytes,
        )
        .expect("decompress layer blob");
        let expected_diff_id = format!("sha256:{:x}", Sha256::digest(&tar_bytes));
        let config: serde_json::Value = serde_json::from_slice(
            &image_store
                .load_config_blob_pub("example/inherited", "latest")
                .expect("load result config"),
        )
        .expect("parse result config");
        assert_eq!(config["config"]["Entrypoint"][0], "/base");
        assert_eq!(config["config"]["Env"], json!(["BASE=1", "CHILD=1"]));
        assert_eq!(config["rootfs"]["diff_ids"], json!([expected_diff_id]));
    }

    async fn build_with_validating_filesystem(
        dockerfile: &str,
        files: &[(&str, &str)],
    ) -> Result<ImageMetadata> {
        let temp_dir = tempfile::TempDir::new().expect("create temp dir");
        let image_store =
            Arc::new(ImageStore::new(temp_dir.path().join("images")).expect("create store"));
        let context = temp_dir.path().join("context");
        std::fs::create_dir(&context).expect("create context");
        std::fs::write(context.join("Dockerfile"), dockerfile).expect("write Dockerfile");
        for (path, contents) in files {
            std::fs::write(context.join(path), contents).expect("write context fixture");
        }
        let builder = MiniboxImageBuilder::new(
            Arc::clone(&image_store),
            temp_dir.path().join("data"),
            Arc::new(CleanupValidatingFilesystem::new(
                image_store.base_dir.clone(),
            )),
            Arc::new(MockRuntime::new()),
            Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                std::iter::empty::<(&str, DynImageRegistry)>(),
            )),
        );
        let (tx, _rx) = mpsc::channel(32);
        builder
            .build_image(
                &BuildContext {
                    directory: context,
                    dockerfile: PathBuf::from("Dockerfile"),
                },
                &BuildConfig {
                    tag: "durability:test".to_string(),
                    build_args: vec![],
                    no_cache: false,
                },
                TokioProgressSink::shared(tx),
            )
            .await
    }

    #[tokio::test]
    async fn run_then_run_uses_durable_image_store_layer() {
        build_with_validating_filesystem("FROM scratch\nRUN true\nRUN true\n", &[])
            .await
            .expect("RUN layers must survive cleanup and satisfy path validation");
    }

    #[tokio::test]
    async fn copy_then_run_uses_durable_image_store_layer() {
        build_with_validating_filesystem(
            "FROM scratch\nCOPY file /file\nRUN true\n",
            &[("file", "contents")],
        )
        .await
        .expect("COPY layer must be image-store owned before RUN");
    }

    #[tokio::test]
    async fn build_rejects_user_followed_by_run() {
        let (_temp, _store, result) =
            build_fixture("FROM scratch\nUSER 1000\nRUN true\n", &[]).await;
        let error = result.expect_err("USER followed by RUN must not silently execute as root");
        assert!(
            error.to_string().contains("USER") && error.to_string().contains("RUN"),
            "error: {error:#}"
        );
    }

    #[tokio::test]
    async fn workdir_exists_without_later_run() {
        let (_temp, store, result) = build_fixture("FROM scratch\nWORKDIR /app/data\n", &[]).await;
        result.expect("WORKDIR-only build should succeed");
        let layers = store
            .get_image_layers("example/native-stage", "latest")
            .expect("load final layers");
        assert!(layers.iter().any(|layer| layer.join("app/data").is_dir()));
    }

    #[tokio::test]
    async fn build_rejects_parent_dockerfile_path() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let context = temp.path().join("context");
        std::fs::create_dir(&context).expect("create context");
        std::fs::write(temp.path().join("Dockerfile"), "FROM scratch\n")
            .expect("write outside Dockerfile");
        let image_store = Arc::new(ImageStore::new(temp.path().join("images")).expect("store"));
        let builder = MiniboxImageBuilder::new(
            image_store,
            temp.path().join("data"),
            Arc::new(MockFilesystem::new()),
            Arc::new(MockRuntime::new()),
            Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                std::iter::empty::<(&str, DynImageRegistry)>(),
            )),
        );
        let (tx, _rx) = mpsc::channel(8);
        let error = builder
            .build_image(
                &BuildContext {
                    directory: context,
                    dockerfile: PathBuf::from("../Dockerfile"),
                },
                &BuildConfig {
                    tag: "invalid:test".to_string(),
                    build_args: vec![],
                    no_cache: false,
                },
                TokioProgressSink::shared(tx),
            )
            .await
            .expect_err("parent Dockerfile path must be rejected");
        assert!(
            error.to_string().contains("Dockerfile path"),
            "error: {error:#}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn build_rejects_dockerfile_symlink_escape() {
        let temp = tempfile::TempDir::new().expect("create temp dir");
        let context = temp.path().join("context");
        std::fs::create_dir(&context).expect("create context");
        let outside = temp.path().join("outside.Dockerfile");
        std::fs::write(&outside, "FROM scratch\n").expect("write outside Dockerfile");
        std::os::unix::fs::symlink(&outside, context.join("Dockerfile"))
            .expect("symlink Dockerfile");
        let image_store = Arc::new(ImageStore::new(temp.path().join("images")).expect("store"));
        let builder = MiniboxImageBuilder::new(
            image_store,
            temp.path().join("data"),
            Arc::new(MockFilesystem::new()),
            Arc::new(MockRuntime::new()),
            Arc::new(HostnameRegistryRouter::new(
                Arc::new(MockRegistry::new()) as DynImageRegistry,
                std::iter::empty::<(&str, DynImageRegistry)>(),
            )),
        );
        let (tx, _rx) = mpsc::channel(8);
        let error = builder
            .build_image(
                &BuildContext {
                    directory: context,
                    dockerfile: PathBuf::from("Dockerfile"),
                },
                &BuildConfig {
                    tag: "invalid:test".to_string(),
                    build_args: vec![],
                    no_cache: false,
                },
                TokioProgressSink::shared(tx),
            )
            .await
            .expect_err("symlinked Dockerfile escape must be rejected");
        assert!(error.to_string().contains("escapes"), "error: {error:#}");
    }

    #[tokio::test]
    async fn local_archive_add_is_explicitly_rejected() {
        let (_temp, _store, result) = build_fixture(
            "FROM scratch\nADD payload.tar /payload/\n",
            &[("payload.tar", "not a tar")],
        )
        .await;
        let error = result.expect_err("local archive ADD must not silently COPY");
        assert!(
            error.to_string().contains("archive ADD"),
            "error: {error:#}"
        );
    }

    #[tokio::test]
    async fn explicit_dockerfile_and_dockerignore_copy_are_rejected() {
        for source in ["Dockerfile", ".dockerignore"] {
            let dockerfile = format!("FROM scratch\nCOPY {source} /copied\n");
            let files = if source == ".dockerignore" {
                vec![(".dockerignore", "")]
            } else {
                vec![]
            };
            let (_temp, _store, result) = build_fixture(&dockerfile, &files).await;
            let error = result.expect_err("internal build files must be excluded");
            assert!(error.to_string().contains("excluded"), "error: {error:#}");
        }
    }

    #[tokio::test]
    async fn global_arg_expands_from_image() {
        let (_temp, _store, result) =
            build_fixture("ARG BASE=scratch\nFROM ${BASE}\nWORKDIR /expanded\n", &[]).await;
        result.expect("global ARG should expand FROM before stage resolution");
    }

    #[test]
    fn instr_display_from() {
        let instr = Instruction::From {
            image: "alpine".to_string(),
            tag: "3.18".to_string(),
            alias: None,
            source: FromSource::Image,
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
