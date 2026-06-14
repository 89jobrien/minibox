//! Container run handlers and supporting infrastructure.
// Handler signatures require >5 parameters by design (DI pattern). See rustqual.toml.
#![allow(clippy::too_many_arguments)]
// TODO(#326): persist execution manifest before container spawn
// TODO(#366): extract shared run preparation path from handle_run

use anyhow::{Context as _, Result};
use chrono::Utc;
use minibox_core::domain::{
    BindMount, ContainerHooks, ContainerSpawnConfig, DomainError, DynContainerRuntime, HookSpec,
    NetworkMode, ResourceConfig,
};
use minibox_core::events::{ContainerEvent, EventSink};
use minibox_core::image::reference::ImageRef;
use minibox_core::protocol::{ContainerInfo, DaemonResponse};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::daemon::state::{ContainerRecord, ContainerState, DaemonState, RunCreationParams};

use super::super::network_lifecycle::NetworkLifecycle;
use super::{HandlerDependencies, PolicyOverride, send_error};

// ─── RunParams: parameter bundle for the run pipeline ────────────────────────

/// Groups the user-supplied container configuration that flows through the
/// entire `handle_run` → `prepare_run` → `run_inner` call chain.
///
/// This eliminates 11+ individual parameters from every function signature
/// in the run pipeline without changing observable behaviour.
pub struct RunParams {
    pub image: String,
    pub tag: Option<String>,
    pub command: Vec<String>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_weight: Option<u64>,
    pub ephemeral: bool,
    pub network: Option<NetworkMode>,
    pub mounts: Vec<BindMount>,
    pub privileged: bool,
    pub env: Vec<String>,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub cgroup_parent: Option<String>,
    pub priority: Option<slashcrux::Priority>,
    pub policy_override: Option<PolicyOverride>,
}

// ─── Container ID Generation ─────────────────────────────────────────────────

/// Generate a 16-char hex container ID from a UUID v4.
///
/// 16 hex chars = 64 bits. Birthday-paradox collision after ~4 billion containers —
/// callers must still check for collisions against the existing container state.
pub fn generate_container_id() -> String {
    Uuid::new_v4()
        .to_string()
        .replace('-', "")
        .chars()
        .take(16)
        .collect()
}

// ─── Run ─────────────────────────────────────────────────────────────────────

/// Create and start a new container from `image:tag`, executing `command`.
///
/// Responses are sent via `tx`.  Non-ephemeral runs send exactly one message.
/// Ephemeral runs (Linux-only) send zero or more `ContainerOutput` messages
/// followed by one terminal `ContainerStopped` message.
pub async fn handle_run(
    params: RunParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    // Nesting depth gate: refuse to create containers if we've hit the limit.
    let nesting = crate::nesting::NestingContext::from_env();
    if let Err(e) = nesting.check_depth() {
        let msg = format!("handle_run: {e}");
        warn!(message = %msg, "handle_run: nesting depth exceeded");
        if tx
            .send(DaemonResponse::Error { message: msg })
            .await
            .is_err()
        {
            warn!("handle_run: client disconnected before depth error could be sent");
        }
        return;
    }

    // Policy gate: deny bind mounts and privileged mode unless explicitly allowed.
    if let Err(msg) = super::validate_policy(
        &params.mounts,
        params.privileged,
        params.priority,
        &deps.policy,
    ) {
        warn!(message = %msg, "handle_run: policy violation");
        if tx
            .send(DaemonResponse::Error { message: msg })
            .await
            .is_err()
        {
            warn!("handle_run: client disconnected before policy error could be sent");
        }
        return;
    }

    // Reject duplicate names eagerly before doing any work.
    // Two-guard pattern: Option check then async check (cannot be written as
    // a single `if let ... && await` in stable Rust).
    #[allow(clippy::collapsible_if)]
    if let Some(ref n) = params.name {
        if state.name_in_use(n).await {
            send_error(
                &tx,
                "handle_run",
                format!("container name {n:?} is already in use"),
            )
            .await;
            return;
        }
    }

    #[cfg(unix)]
    if params.ephemeral {
        handle_run_streaming(params, state, deps, tx).await;
        return;
    }

    // Non-ephemeral (or non-Linux): single response.
    let response = match run_inner(params, state, deps).await {
        Ok(id) => DaemonResponse::ContainerCreated { id },
        Err(e) => {
            error!("handle_run error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    };
    if tx.send(response).await.is_err() {
        warn!("handle_run: client disconnected before response could be sent");
    }
}

/// Streaming ephemeral run: sends `ContainerOutput` chunks then `ContainerStopped`.
///
/// The container stdout+stderr are forwarded via the channel until EOF, then
/// the exit code is reported.
#[cfg(unix)]
pub(super) async fn handle_run_streaming(
    params: RunParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use minibox_core::protocol::OutputStreamKind;
    use std::os::fd::IntoRawFd;

    // Build the container ID and rootfs via the shared inner setup, but we need
    // capture_output=true. We inline a variant of run_inner here.
    let image_label = format!(
        "{}:{}",
        params.image,
        params.tag.as_deref().unwrap_or("latest")
    );
    let result = run_inner_capture(params, Arc::clone(&state), Arc::clone(&deps)).await;

    let (container_id, pid, output_reader, runtime_id) = match result {
        Ok(triple) => triple,
        Err(e) => {
            error!("handle_run_streaming setup error: {e:#}");
            send_error(&tx, "handle_run", format!("{e:#}")).await;
            return;
        }
    };

    // Emit the container ID first so the CLI (and tests) can capture it
    // without waiting for the container to exit.  The protocol spec requires
    // ContainerCreated as the first streaming message (see protocol.rs §Ephemeral).
    debug!(pid = pid, "streaming: sending ContainerCreated");
    deps.events.event_sink.emit(ContainerEvent::Created {
        id: container_id.clone(),
        image: image_label,
        timestamp: std::time::SystemTime::now(),
    });
    deps.events.event_sink.emit(ContainerEvent::Started {
        id: container_id.clone(),
        pid,
        timestamp: std::time::SystemTime::now(),
    });
    let _ = tx
        .send(DaemonResponse::ContainerCreated {
            id: container_id.clone(),
        })
        .await;
    debug!(
        pid = pid,
        "streaming: ContainerCreated sent, spawning drain"
    );

    // Spawn blocking task to drain the pipe and forward chunks.
    let tx_clone = tx.clone();
    // SAFETY: OwnedFd is not Send on all platforms, so we transfer ownership via raw fd.
    // The OwnedFd is consumed by into_raw_fd() (no drop), and from_raw_fd() inside the
    // closure takes sole ownership. No other code touches reader_raw after this point.
    let reader_raw = output_reader.into_raw_fd();
    let stdout_log_path = deps
        .lifecycle
        .containers_base
        .join(&container_id)
        .join("stdout.log");
    let drain_handle = tokio::task::spawn_blocking(move || {
        use std::io::{Read, Write};
        use std::os::fd::FromRawFd;

        // SAFETY: we own this fd from the pipe created in spawn_container_process.
        let mut file = unsafe { std::fs::File::from_raw_fd(reader_raw) };
        // Best-effort log file: open for append (create if missing).
        let mut log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stdout_log_path)
            .map_err(|e| {
                warn!(
                    path = %stdout_log_path.display(),
                    error = %e,
                    "streaming: failed to open stdout.log for writing"
                );
            })
            .ok();
        const READ_BUFFER_SIZE: usize = 4096;
        let mut buf = [0u8; READ_BUFFER_SIZE];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break, // EOF — child exited and closed its write end.
                Ok(n) => {
                    // Best-effort write to log file.
                    if let Some(ref mut lf) = log_file
                        && let Err(e) = lf.write_all(&buf[..n])
                    {
                        warn!(
                            path = %stdout_log_path.display(),
                            error = %e,
                            "streaming: stdout.log write error"
                        );
                    }
                    use base64::Engine;
                    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    let _ = tx_clone.blocking_send(DaemonResponse::ContainerOutput {
                        stream: OutputStreamKind::Stdout,
                        data: encoded,
                    });
                }
                Err(e) => {
                    warn!(pid = pid, error = %e, "pipe drain: read error");
                    break;
                }
            }
        }
    });

    // Wait for the child process to exit via the runtime adapter.
    // Native adapters use waitpid; krun/smolvm delegates to SmolvmProcess::wait().
    debug!(pid = pid, "streaming: waiting for child exit");
    let runtime = Arc::clone(&deps.lifecycle.runtime);
    let exit_code = runtime
        .wait_for_exit(runtime_id.as_deref(), pid)
        .await
        .unwrap_or(-1);
    debug!(pid = pid, exit_code = exit_code, "streaming: child exited");

    // Wait for drain to finish before sending ContainerStopped
    // so all output is flushed before the terminal message.
    debug!(pid = pid, "streaming: waiting for drain");
    if let Err(e) = drain_handle.await {
        warn!(pid = pid, "pipe drain task panicked: {:?}", e);
    }
    debug!(pid = pid, "streaming: drain complete");

    // ── Network cleanup (ephemeral) ────────────────────────────────────
    NetworkLifecycle::new(deps.lifecycle.network_provider.clone())
        .cleanup(&container_id)
        .await;
    debug!(pid = pid, "streaming: network cleanup done");

    // Grab cgroup path before removing state, for OOM detection.
    let cgroup_path_opt = state
        .get_container(&container_id)
        .await
        .map(|r| r.cgroup_path);

    // Auto-remove ephemeral container state.
    state.remove_container(&container_id).await;
    debug!(pid = pid, "streaming: container removed");

    // Emit Stopped or OomKilled lifecycle event.
    let oom = if let Some(cgroup_path) = &cgroup_path_opt {
        check_oom_killed(cgroup_path).await
    } else {
        false
    };
    if oom {
        deps.events.event_sink.emit(ContainerEvent::OomKilled {
            id: container_id.clone(),
            timestamp: std::time::SystemTime::now(),
        });
    } else {
        deps.events.event_sink.emit(ContainerEvent::Stopped {
            id: container_id.clone(),
            exit_code,
            timestamp: std::time::SystemTime::now(),
        });
    }

    let _ = tx
        .send(DaemonResponse::ContainerStopped { exit_code })
        .await;
    debug!(pid = pid, "streaming: ContainerStopped sent");
}

// ─── PreparedRun: shared setup extracted from run_inner / run_inner_capture ───

/// All state produced by container preparation, before the process is spawned.
#[cfg(unix)]
struct PreparedRun {
    id: String,
    spawn_config: ContainerSpawnConfig,
    image_label: String,
    /// Network lifecycle handle — must stay alive until attach is called.
    net: NetworkLifecycle,
    manifest_path: PathBuf,
    workload_digest: String,
}

/// Construct an `ExecutionManifest` from container run parameters.
#[cfg(unix)]
fn build_execution_manifest(
    id: &str,
    ref_str: &str,
    layer_dirs: &[PathBuf],
    command: &[String],
    env: &[String],
    mounts: &[BindMount],
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    net_mode: NetworkMode,
    privileged: bool,
    platform: &Option<String>,
    name: &Option<String>,
    capture_output: bool,
) -> minibox_core::domain::ExecutionManifest {
    use minibox_core::domain::{
        ExecutionManifest, ExecutionManifestEnvVar, ExecutionManifestImage, ExecutionManifestMount,
        ExecutionManifestRequest, ExecutionManifestResourceLimits, ExecutionManifestRuntime,
        ExecutionManifestSubject,
    };

    // TODO(#436): replace Debug format with explicit Display/as_str
    let net_mode_str = format!("{net_mode:?}").to_lowercase();
    ExecutionManifest {
        schema_version: 1,
        container_id: id.to_string(),
        created_at: Utc::now().to_rfc3339(),
        manifest_path: None,
        workload_digest: None,
        subject: ExecutionManifestSubject {
            image_ref: ref_str.to_string(),
            image: ExecutionManifestImage {
                manifest_digest: None,
                config_digest: None,
                layer_digests: layer_dirs
                    .iter()
                    .filter_map(|p| p.file_name()?.to_str().map(|s| s.replacen('_', ":", 1)))
                    .collect(),
            },
        },
        runtime: ExecutionManifestRuntime {
            command: command.to_vec(),
            env: env
                .iter()
                .filter_map(|e| {
                    let (k, v) = e.split_once('=')?;
                    Some(ExecutionManifestEnvVar::new(k, v))
                })
                .collect(),
            mounts: mounts
                .iter()
                .map(ExecutionManifestMount::from_bind_mount)
                .collect(),
            resource_limits: Some(ExecutionManifestResourceLimits {
                memory_limit_bytes,
                cpu_weight,
            }),
            network_mode: net_mode_str,
            privileged,
            platform: platform.clone(),
        },
        request: ExecutionManifestRequest {
            name: name.clone(),
            ephemeral: capture_output,
        },
    }
}

/// Build a `ContainerRecord` in `"Created"` state for a new container.
#[cfg(unix)]
fn build_container_record(
    id: &str,
    name: &Option<String>,
    image_label: &str,
    command: &[String],
    merged_dir: &minibox_core::path::InternalPath,
    cgroup_dir: &std::path::Path,
    rootfs_layout: &minibox_core::domain::RootfsLayout,
    image: &str,
    tag: &str,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    network: Option<NetworkMode>,
    env: &[String],
    mounts: &[BindMount],
    privileged: bool,
    platform: &Option<String>,
    cgroup_parent: &Option<String>,
) -> ContainerRecord {
    let command_str = command.join(" ");
    ContainerRecord {
        info: ContainerInfo {
            id: id.to_string(),
            name: name.clone(),
            image: image_label.to_string(),
            command: command_str,
            state: "Created".to_string(),
            created_at: Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        rootfs_path: merged_dir.clone().into_inner(),
        cgroup_path: cgroup_dir.to_path_buf(),
        post_exit_hooks: vec![],
        rootfs_metadata: rootfs_layout.rootfs_metadata.clone(),
        source_image_ref: rootfs_layout
            .source_image_ref
            .clone()
            .or_else(|| Some(image_label.to_string())),
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: Some(RunCreationParams {
            image: image.to_string(),
            tag: Some(tag.to_string()),
            command: command.to_vec(),
            memory_limit_bytes,
            cpu_weight,
            network,
            env: env.to_vec(),
            mounts: mounts.to_vec(),
            privileged,
            name: name.clone(),
            tty: false,
            entrypoint: None,
            user: None,
            platform: platform.clone(),
            cgroup_parent: cgroup_parent.clone(),
        }),
        manifest_path: None,
        workload_digest: None,
    }
}

/// Shared container preparation: image pull, overlay setup, cgroup creation,
/// network setup, container record registration, spawn config construction,
/// and execution manifest persistence.
///
/// The `capture_output` flag is the only behavioural difference between the
/// streaming (`run_inner_capture`) and fire-and-forget (`run_inner`) paths.
#[cfg(unix)]
async fn prepare_run(
    params: RunParams,
    capture_output: bool,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<PreparedRun> {
    let RunParams {
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        network,
        mounts,
        privileged,
        env,
        name,
        platform,
        cgroup_parent,
        ephemeral: _,
        priority: _,
        policy_override: _,
    } = params;

    // Build full ref string from image + optional tag, then parse into ImageRef.
    let ref_str = match &tag {
        Some(t) => format!("{image}:{t}"),
        None => image.clone(),
    };
    let image_ref = ImageRef::parse(&ref_str)
        .with_context(|| format!("invalid image reference {ref_str:?}"))
        .map_err(|e| DomainError::InvalidConfig(e.to_string()))?;
    let tag = image_ref.tag.clone();
    let full_image = image_ref.cache_name();

    // Resolve platform-overridden registry if requested, otherwise route by hostname.
    let platform_registry = super::image::resolve_platform_registry(&platform, &image_ref, &deps)?;
    let default_registry = deps.image.registry_router.route(&image_ref);
    let registry: &dyn minibox_core::domain::ImageRegistry = match &platform_registry {
        Some(r) => r.as_ref(),
        None => default_registry,
    };

    // Pull image if not cached.
    if !registry.has_image(&full_image, &tag).await {
        info!("image {full_image}:{tag} not cached, pulling…");
        registry
            .pull_image(&image_ref)
            .await
            .map_err(|e| DomainError::ImagePullFailed {
                image: full_image.clone(),
                tag: tag.clone(),
                source: e,
            })?;
    }

    let layer_dirs = registry.get_image_layers(&full_image, &tag)?;
    if layer_dirs.is_empty() {
        return Err(DomainError::EmptyImage {
            name: full_image.clone(),
            tag: tag.clone(),
        }
        .into());
    }

    let net_mode = network.unwrap_or(NetworkMode::None);

    // ── Execution policy gate ───────────────────────────────────────
    // Evaluate BEFORE creating any resources (overlay, cgroup, network).
    // The manifest depends only on request parameters and cached layer
    // digests, so no cleanup is needed on denial.
    let id = generate_container_id();

    // SECURITY: Verify no collision with existing containers.
    if state.get_container(&id).await.is_some() {
        return Err(DomainError::InvalidConfig(format!(
            "container ID collision (extremely rare): {id}"
        ))
        .into());
    }

    let mut manifest = build_execution_manifest(
        &id,
        &ref_str,
        &layer_dirs,
        &command,
        &env,
        &mounts,
        memory_limit_bytes,
        cpu_weight,
        net_mode,
        privileged,
        &platform,
        &name,
        capture_output,
    );
    manifest
        .seal()
        .context("failed to compute execution manifest digest")?;

    if let Some(ref policy) = deps.execution_policy {
        use minibox_core::domain::PolicyDecision;
        match policy.evaluate(&manifest) {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny(reason) => {
                return Err(anyhow::anyhow!("execution policy denied: {reason}"));
            }
        }
    }

    let container_dir = deps.lifecycle.containers_base.join(&id);
    let run_dir = deps.lifecycle.run_containers_base.join(&id);

    // SECURITY: Create container directories with restricted permissions (0700).
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        const OWNER_RWX_PERMS: u32 = 0o700;
        builder.mode(OWNER_RWX_PERMS);
        builder.recursive(true);
        builder.create(&container_dir)?;
        builder.create(&run_dir)?;
    }

    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&container_dir)?;
        std::fs::create_dir_all(&run_dir)?;
    }

    // Setup overlayfs.
    let rootfs_layout = deps
        .lifecycle
        .filesystem
        .setup_rootfs(&layer_dirs, &container_dir)?;
    let merged_dir = rootfs_layout.merged_dir.clone();

    // Setup cgroup.
    const DEFAULT_PIDS_MAX: u64 = 1024;
    let resource_config = ResourceConfig {
        memory_limit_bytes,
        cpu_weight,
        pids_max: Some(DEFAULT_PIDS_MAX),
        io_max_bytes_per_sec: None,
    };
    let cgroup_dir_str = {
        #[cfg(target_os = "linux")]
        if let Some(ref parent) = cgroup_parent {
            // Validate cgroup_parent is under /sys/fs/cgroup/ to prevent arbitrary
            // directory creation elsewhere on the filesystem.
            crate::container::cgroups::validate_cgroup_parent(parent)?;
            let mgr = crate::container::cgroups::CgroupManager::with_root(
                &id,
                crate::container::cgroups::CgroupConfig {
                    memory_limit_bytes: resource_config.memory_limit_bytes,
                    cpu_weight: resource_config.cpu_weight,
                    pids_max: resource_config.pids_max,
                    io_max_bytes_per_sec: resource_config.io_max_bytes_per_sec,
                },
                std::path::PathBuf::from(parent),
            );
            mgr.create()?;
            mgr.cgroup_path().display().to_string()
        } else {
            deps.lifecycle
                .resource_limiter
                .create(&id, &resource_config)?
        }

        #[cfg(not(target_os = "linux"))]
        {
            if cgroup_parent.is_some() {
                anyhow::bail!("--cgroup-parent is only supported on Linux");
            }
            deps.lifecycle
                .resource_limiter
                .create(&id, &resource_config)?
        }
    };
    let cgroup_dir = PathBuf::from(cgroup_dir_str);

    // ── Cgroup delegation for nested containers ────────────────────────
    // Only privileged containers get subtree delegation — non-privileged
    // containers use the flat cgroup model (no child cgroup creation).
    #[cfg(target_os = "linux")]
    if privileged {
        let delegation = crate::container::cgroups::DelegationPaths {
            subtree: cgroup_dir.clone(),
            init_leaf: cgroup_dir.join("init"),
        };
        if let Err(e) = crate::container::cgroups::delegate_subtree(&delegation) {
            debug!(
                container_id = %id,
                error = %e,
                "cgroup delegation skipped (non-fatal)"
            );
        }
    }

    // ── Network setup ──────────────────────────────────────────────────
    let network_config = minibox_core::domain::NetworkConfig {
        mode: net_mode,
        ..minibox_core::domain::NetworkConfig::default()
    };
    let net = NetworkLifecycle::new(deps.lifecycle.network_provider.clone());
    let _net_ns = net
        .setup(&id, &network_config)
        .await
        .context("network setup")?;

    let skip_net_ns = net_mode == NetworkMode::Host;

    // Build ContainerRecord in Created state.
    let image_label = format!("{image}:{tag}");
    let record = build_container_record(
        &id,
        &name,
        &image_label,
        &command,
        &merged_dir,
        &cgroup_dir,
        &rootfs_layout,
        &image,
        &tag,
        memory_limit_bytes,
        cpu_weight,
        network,
        &env,
        &mounts,
        privileged,
        &platform,
        &cgroup_parent,
    );
    // Build the ContainerSpawnConfig for the runtime.
    let spawn_command = command
        .first()
        .cloned()
        .unwrap_or_else(|| "/bin/sh".to_string());
    let spawn_args = command.iter().skip(1).cloned().collect();
    let mut container_env = vec![
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
        "TERM=xterm".to_string(),
    ];
    container_env.extend(env.clone());

    // Inject nesting depth for minibox-in-minibox support.
    let nesting = crate::nesting::NestingContext::from_env();
    container_env.extend(nesting.child_env_vars());
    let spawn_config = ContainerSpawnConfig {
        rootfs: merged_dir.clone(),
        command: spawn_command,
        args: spawn_args,
        env: container_env,
        cgroup_path: cgroup_dir.clone().into(),
        hostname: format!("minibox-{}", &id[..8]),
        capture_output,
        hooks: ContainerHooks::default(),
        skip_network_namespace: skip_net_ns,
        mounts: mounts.clone(),
        privileged,
        image_ref: Some(image_label.clone()),
    };

    // ── Persist execution manifest ─────────────────────────────────────
    let manifest_path = container_dir.join("execution-manifest.json");
    let manifest_json =
        serde_json::to_string_pretty(&manifest).context("serialise execution manifest")?;
    std::fs::write(&manifest_path, &manifest_json)
        .with_context(|| format!("write execution manifest to {}", manifest_path.display()))?;
    manifest.manifest_path = Some(manifest_path.clone());

    let workload_digest = manifest.workload_digest.clone().unwrap_or_default();

    // Register the container only after all fallible ops (overlay, cgroup,
    // network, manifest seal, policy gate) have succeeded.  This prevents
    // phantom records when any preparation step fails.
    state.add_container(record).await;

    Ok(PreparedRun {
        id,
        spawn_config,
        image_label,
        net,
        manifest_path,
        workload_digest,
    })
}

/// Variant of `run_inner` that enables output capture for ephemeral containers.
///
/// Sets `capture_output = true` in the spawn config so the runtime creates a
/// pipe between the container process and the daemon.  Returns the container ID,
/// the child PID, and the read end of the output pipe as an [`OwnedFd`].
///
/// The caller is responsible for draining the pipe (to avoid blocking the child
/// on a full pipe buffer) and for calling `wait_for_exit` to reap the process.
///
/// Container state transitions: `"Created"` → `"Running"` (via
/// `set_container_pid`).  The `"Stopped"` transition is handled by the caller
/// (`handle_run_streaming`) after the process exits.
///
/// Compiled on Unix (Linux and macOS). The output pipe uses `OwnedFd`
/// and `waitpid` — both available on any Unix via the `nix` crate.
#[cfg(unix)]
async fn run_inner_capture(
    params: RunParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<(String, u32, std::os::fd::OwnedFd, Option<String>)> {
    let prepared = prepare_run(params, true, Arc::clone(&state), Arc::clone(&deps)).await?;

    state
        .set_manifest_info(
            &prepared.id,
            prepared.manifest_path.clone(),
            prepared.workload_digest.clone(),
        )
        .await;

    // Semaphore closed only if the daemon is shutting down; no recovery possible.
    #[allow(clippy::expect_used)]
    let _spawn_permit = state
        .spawn_semaphore
        .acquire()
        .await
        .expect("semaphore closed");

    let spawn_result = deps
        .lifecycle
        .runtime
        .spawn_process(&prepared.spawn_config)
        .await?;

    let pid = spawn_result.pid;
    let runtime_id = spawn_result.runtime_id;
    let output_reader = spawn_result.output_reader.ok_or_else(|| {
        anyhow::anyhow!("capture_output=true but runtime returned no output_reader")
    })?;

    // ── Network attach ─────────────────────────────────────────────────
    prepared
        .net
        .attach(&prepared.id, pid)
        .await
        .context("network attach")?;

    // Write PID file and update state.
    let pid_file = deps
        .lifecycle
        .run_containers_base
        .join(&prepared.id)
        .join("pid");
    if let Err(e) = std::fs::write(&pid_file, pid.to_string()) {
        warn!(
            pid_file = %pid_file.display(),
            error = %e,
            "container: failed to write pid file"
        );
    }
    state.set_container_pid(&prepared.id, pid).await;

    Ok((prepared.id, pid, output_reader, runtime_id))
}

/// Pull the image if needed, set up the overlay rootfs and cgroup, register the
/// container in `"Created"` state, then spawn the container process.
///
/// Returns the new container ID immediately after the spawn task is dispatched.
/// The container transitions from `"Created"` to `"Running"` asynchronously
/// once the runtime reports the child PID.  A background reaper task
/// (`daemon_wait_for_exit`) drives the final `"Stopped"` transition.
///
/// # Async / sync boundary
///
/// The runtime's `spawn_process` is async (it may perform IPC with an external
/// runtime such as Colima).  The actual fork/clone/exec for the native Linux
/// adapter happens inside `spawn_process` via `tokio::task::spawn_blocking` in
/// the runtime implementation, keeping blocking syscalls off the Tokio worker
/// threads.  The reaper is also dispatched via `spawn_blocking` because
/// `waitpid` is a blocking syscall.
async fn run_inner(
    params: RunParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<String> {
    let prepared = prepare_run(params, false, Arc::clone(&state), Arc::clone(&deps)).await?;

    state
        .set_manifest_info(
            &prepared.id,
            prepared.manifest_path.clone(),
            prepared.workload_digest.clone(),
        )
        .await;

    let id = prepared.id.clone();
    let image_label = prepared.image_label.clone();
    let spawn_config = prepared.spawn_config;

    // SECURITY: Acquire semaphore permit to limit concurrent spawns.
    // Semaphore closed only if the daemon is shutting down; no recovery possible.
    #[allow(clippy::expect_used)]
    let _spawn_permit = state
        .spawn_semaphore
        .acquire()
        .await
        .expect("semaphore closed");

    // Spawn the container process synchronously so failures propagate to the caller.
    let spawn_result = match deps.lifecycle.runtime.spawn_process(&spawn_config).await {
        Ok(r) => r,
        Err(e) => {
            error!("failed to spawn container {id}: {e:#}");
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "run"), ("adapter", "daemon"), ("status", "error")],
            );
            if let Err(ue) = state
                .update_container_state(&id, ContainerState::Failed)
                .await
            {
                warn!(container_id = %id, error = %ue, "state: failed to mark container Failed");
            }
            return Err(e);
        }
    };
    // Release the permit now that the process is running.
    drop(_spawn_permit);

    let pid = spawn_result.pid;
    info!(container_id = %id, pid = pid, "container: process started");

    deps.events.event_sink.emit(ContainerEvent::Created {
        id: id.clone(),
        image: image_label.clone(),
        timestamp: std::time::SystemTime::now(),
    });
    deps.events.event_sink.emit(ContainerEvent::Started {
        id: id.clone(),
        pid,
        timestamp: std::time::SystemTime::now(),
    });

    deps.events.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "run"), ("adapter", "daemon"), ("status", "ok")],
    );
    let active = state.list_containers().await.len() as f64;
    deps.events
        .metrics
        .set_gauge("minibox_active_containers", active, &[]);

    // TODO(#429): propagate net.attach error instead of swallowing with .ok()
    prepared.net.attach(&id, pid).await.ok();

    let pid_file = deps.lifecycle.run_containers_base.join(&id).join("pid");
    if let Err(e) = std::fs::write(&pid_file, pid.to_string()) {
        warn!(
            pid_file = %pid_file.display(),
            error = %e,
            "container: failed to write pid file"
        );
    }

    state.set_container_pid(&id, pid).await;

    // Hand off wait-for-exit to a background task.
    let state_wait = Arc::clone(&state);
    let id_wait = id.clone();
    let event_sink_wait = Arc::clone(&deps.events.event_sink);
    let runtime_wait = Arc::clone(&deps.lifecycle.runtime);
    let runtime_id = spawn_result.runtime_id.clone();
    tokio::spawn(async move {
        daemon_wait_for_exit(
            pid,
            &id_wait,
            state_wait,
            spawn_config.rootfs,
            spawn_config.hooks.post_exit,
            event_sink_wait,
            spawn_config.cgroup_path,
            runtime_wait,
            runtime_id,
        )
        .await;
    });

    Ok(id)
}

/// Re-run a container from its stored `RunCreationParams`.
///
/// Used by `handle_update` to restart containers after an image update.
/// Delegates to `run_inner` with all fields from the stored params.
#[cfg(unix)]
pub(super) async fn run_from_params(
    creation_params: &RunCreationParams,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<String> {
    let params = RunParams {
        image: creation_params.image.clone(),
        tag: creation_params.tag.clone(),
        command: creation_params.command.clone(),
        memory_limit_bytes: creation_params.memory_limit_bytes,
        cpu_weight: creation_params.cpu_weight,
        ephemeral: false,
        network: creation_params.network,
        mounts: creation_params.mounts.clone(),
        privileged: creation_params.privileged,
        env: creation_params.env.clone(),
        name: creation_params.name.clone(),
        platform: creation_params.platform.clone(),
        cgroup_parent: creation_params.cgroup_parent.clone(),
        priority: None,
        policy_override: None,
    };
    run_inner(params, state, deps).await
}

// ─── OOM detection ───────────────────────────────────────────────────────────

/// Check if a container was OOM-killed by reading cgroup v2 `memory.events`.
///
/// Returns `true` if `oom_kill` count is greater than zero.  Returns `false` if
/// the file cannot be read (e.g. cgroup already deleted, or non-Linux platform).
pub async fn check_oom_killed(cgroup_path: &std::path::Path) -> bool {
    let events_path = cgroup_path.join("memory.events");
    if let Ok(content) = tokio::fs::read_to_string(&events_path).await {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("oom_kill ") {
                return rest.trim().parse::<u64>().unwrap_or(0) > 0;
            }
        }
    }
    false
}

// ─── daemon_wait_for_exit ────────────────────────────────────────────────────

///
/// Waits for the container process to exit via the runtime adapter, then
/// updates state and emits lifecycle events.
///
/// Uses `runtime.wait_for_exit()` which dispatches to `waitpid` for native
/// adapters or to the adapter's own wait mechanism (e.g. `SmolvmProcess::wait`
/// for krun).
#[cfg(unix)]
async fn daemon_wait_for_exit(
    pid: u32,
    id: &str,
    state: Arc<DaemonState>,
    _rootfs: minibox_core::path::InternalPath,
    _post_exit_hooks: Vec<HookSpec>,
    event_sink: Arc<dyn EventSink>,
    cgroup_path: minibox_core::path::InternalPath,
    runtime: DynContainerRuntime,
    runtime_id: Option<String>,
) {
    let exit_code = runtime
        .wait_for_exit(runtime_id.as_deref(), pid)
        .await
        .unwrap_or_else(|e| {
            warn!(container_id = %id, error = %e, "container: wait_for_exit error");
            -1
        });
    info!(container_id = %id, exit_code = exit_code, "container: exited");

    #[cfg(target_os = "linux")]
    if !_post_exit_hooks.is_empty() {
        use crate::container::process::run_hooks;
        if let Err(e) = run_hooks(&_post_exit_hooks, &_rootfs, Some(exit_code)) {
            warn!(container_id = %id, error = %e, "container: post-exit hooks error");
        }
    }

    // Check OOM and emit lifecycle event.
    let oom = check_oom_killed(&cgroup_path).await;
    if oom {
        event_sink.emit(ContainerEvent::OomKilled {
            id: id.to_string(),
            timestamp: std::time::SystemTime::now(),
        });
    } else {
        event_sink.emit(ContainerEvent::Stopped {
            id: id.to_string(),
            exit_code,
            timestamp: std::time::SystemTime::now(),
        });
    }

    if let Err(e) = state
        .update_container_state(id, ContainerState::Stopped)
        .await
    {
        warn!(container_id = %id, error = %e, "state: failed to mark container Stopped");
    }
}

/// Windows stub: no-op because HCS/WSL2 lifecycle is managed externally.
///
/// Containers on Windows remain in `"Running"` state until an explicit
/// `stop` or `remove` command is issued.
#[cfg(windows)]
async fn daemon_wait_for_exit(
    _pid: u32,
    _id: &str,
    _state: Arc<DaemonState>,
    _rootfs: minibox_core::path::InternalPath,
    _post_exit_hooks: Vec<HookSpec>,
    _event_sink: Arc<dyn EventSink>,
    _cgroup_path: minibox_core::path::InternalPath,
    _runtime: DynContainerRuntime,
    _runtime_id: Option<String>,
) {
    // No-op on Windows. Container stays "Running" until explicit stop/remove.
}

/// Fallback stub for platforms other than Unix or Windows.
#[cfg(not(any(unix, windows)))]
async fn daemon_wait_for_exit(
    _pid: u32,
    _id: &str,
    _state: Arc<DaemonState>,
    _rootfs: minibox_core::path::InternalPath,
    _post_exit_hooks: Vec<HookSpec>,
    _event_sink: Arc<dyn EventSink>,
    _cgroup_path: minibox_core::path::InternalPath,
    _runtime: DynContainerRuntime,
    _runtime_id: Option<String>,
) {
    // No-op on this platform.
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod run_inner_tests {
    use super::generate_container_id;

    #[test]
    fn run_inner_capture_signature_accepts_mounts_and_privileged() {
        // Compile-time check: the BindMount type is accessible in this crate.
        use minibox_core::domain::BindMount;
        let _: Vec<BindMount> = vec![];
        let _: bool = false;
    }

    // ── generate_container_id properties ─────────────────────────────────────

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig {
            failure_persistence: None,
            cases: 256,
            ..proptest::prelude::ProptestConfig::default()
        })]

        /// Generated IDs must always be exactly 16 characters long.
        #[test]
        fn generated_id_is_always_16_chars(_dummy in proptest::prelude::Just(())) {
            let id = generate_container_id();
            prop_assert_eq!(
                id.len(),
                16,
                "expected 16-char id, got {:?} (len={})",
                id,
                id.len()
            );
        }

        /// Generated IDs must contain only lowercase hex characters (0-9, a-f).
        #[test]
        fn generated_id_is_lowercase_hex(_dummy in proptest::prelude::Just(())) {
            let id = generate_container_id();
            prop_assert!(
                id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "id contains non-lowercase-hex chars: {:?}",
                id
            );
        }
    }

    /// Two consecutive calls must produce distinct IDs (birthday-paradox:
    /// collision probability per pair is ~2^-64, negligible in testing).
    #[test]
    fn generated_ids_are_distinct_across_calls() {
        let ids: std::collections::HashSet<String> =
            (0..256).map(|_| generate_container_id()).collect();
        assert_eq!(
            ids.len(),
            256,
            "collision detected among 256 generated container IDs"
        );
    }
}
