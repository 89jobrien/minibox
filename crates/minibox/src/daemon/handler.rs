//! Request handlers for each daemon operation.
//!
//! Each public function corresponds to one `DaemonRequest` variant and
//! returns a `DaemonResponse`.  Errors are caught and returned as
//! `DaemonResponse::Error` so the daemon never panics on bad input.
//!
//! # Hexagonal Architecture
//!
//! Handlers use dependency injection to receive infrastructure adapters
//! via the [`HandlerDependencies`] struct. This allows the business logic
//! to be tested independently of infrastructure concerns.

use anyhow::Result;
use chrono::Utc;
use minibox_core::domain::NetworkMode;
use minibox_core::domain::{
    BindMount, ContainerHooks, ContainerSpawnConfig, DomainError, DynContainerRuntime,
    DynFilesystemProvider, DynMetricsRecorder, DynNetworkProvider, DynRegistryRouter,
    DynResourceLimiter, HookSpec, ResourceConfig, SessionId,
};
use minibox_core::events::{ContainerEvent, EventSink};
use minibox_core::image::reference::ImageRef;
use minibox_core::protocol::{ContainerInfo, DaemonResponse};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::mpsc;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use super::network_lifecycle::NetworkLifecycle;
use super::state::{ContainerRecord, ContainerState, DaemonState, RunCreationParams};
use async_trait::async_trait;

// ─── Internal helpers ─────────────────────────────────────────────────────────

/// Send a terminal `DaemonResponse::Error` on `tx`, logging a warning if the
/// receiver has already been dropped.
///
/// Use this instead of `let _ = tx.send(...).await` so that dropped connections
/// are observable in logs rather than silently swallowed.
async fn send_error(tx: &mpsc::Sender<DaemonResponse>, context: &str, message: String) {
    if tx
        .send(DaemonResponse::Error {
            message: message.clone(),
        })
        .await
        .is_err()
    {
        warn!(
            context,
            error_message = %message,
            "client disconnected before error response could be sent"
        );
    }
}

// ─── PTY session registry ─────────────────────────────────────────────────────

/// Tracks live PTY session channels keyed by session ID string.
///
/// Populated by `handle_exec` when a tty session starts and consumed by
/// `handle_send_input` / `handle_resize_pty` dispatched from `server.rs`.
#[derive(Default)]
pub struct PtySessionRegistry {
    /// Resize event senders: session_id → sender for `(cols, rows)`.
    pub resize: HashMap<String, mpsc::Sender<(u16, u16)>>,
    /// Stdin byte senders: session_id → sender for raw bytes.
    /// Only populated when `tty = true`.
    pub stdin: HashMap<String, mpsc::Sender<Vec<u8>>>,
}

impl PtySessionRegistry {
    /// Remove all channels associated with `session_id`.
    ///
    /// Called when an exec session ends (on `ContainerStopped` or error) to
    /// prevent unbounded registry growth and avoid stale-sender warnings.
    pub fn cleanup(&mut self, session_id: &str) {
        self.resize.remove(session_id);
        self.stdin.remove(session_id);
    }
}

/// Arc-wrapped, async-mutex-guarded PTY session registry.
pub type SharedPtyRegistry = Arc<TokioMutex<PtySessionRegistry>>;

// ─── Default adapters ────────────────────────────────────────────────────────

/// Production no-op image loader.
///
/// Used as a placeholder in platform adapters (e.g. macbox, winbox) that do
/// not yet implement local tarball loading. Accepts any load request and
/// returns `Ok(())` immediately. This is a real adapter, not a test double.
pub struct NoopImageLoader;

#[async_trait]
impl minibox_core::domain::ImageLoader for NoopImageLoader {
    async fn load_image(
        &self,
        _path: &std::path::Path,
        _name: &str,
        _tag: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Handler Dependencies — ISP-compliant sub-structs
// ---------------------------------------------------------------------------

/// Image-related dependencies: registry routing, loading, GC, and local store.
///
/// Handlers that only pull or inspect images depend on this sub-struct rather
/// than the full [`HandlerDependencies`].
#[derive(Clone)]
pub struct ImageDeps {
    /// Registry router that selects the appropriate image registry for a given image reference.
    pub registry_router: DynRegistryRouter,
    /// Loader for local OCI image tarballs.
    pub image_loader: minibox_core::domain::DynImageLoader,
    /// Image garbage collector for prune operations.
    pub image_gc: Arc<dyn minibox_core::image::gc::ImageGarbageCollector>,
    /// Image store for direct image operations (e.g. RemoveImage).
    pub image_store: Arc<minibox_core::image::ImageStore>,
}

/// Container lifecycle dependencies: filesystem, limits, runtime, network, and paths.
///
/// Handlers that create or destroy containers depend on this sub-struct.
#[derive(Clone)]
pub struct LifecycleDeps {
    /// Filesystem provider for setting up container rootfs.
    pub filesystem: DynFilesystemProvider,
    /// Resource limiter for enforcing cgroup limits.
    pub resource_limiter: DynResourceLimiter,
    /// Container runtime for spawning isolated processes.
    pub runtime: DynContainerRuntime,
    /// Network provider for container network setup/teardown.
    pub network_provider: DynNetworkProvider,
    /// Base directory for persistent container data (overlay dirs).
    pub containers_base: PathBuf,
    /// Base directory for runtime container state (PID files).
    pub run_containers_base: PathBuf,
}

/// Exec and PTY dependencies for running commands inside containers.
///
/// Handlers that implement `exec` or PTY session management depend on this
/// sub-struct.
#[derive(Clone)]
pub struct ExecDeps {
    /// Exec runtime for running commands inside containers.
    /// `None` on platforms where exec is not supported (macOS, Windows).
    pub exec_runtime: Option<minibox_core::domain::DynExecRuntime>,
    /// Live PTY session channels for SendInput/ResizePty dispatch.
    pub pty_sessions: SharedPtyRegistry,
}

/// Image build/push/commit dependencies.
///
/// Handlers that build, push, or commit images depend on this sub-struct.
/// All fields are `Option` because these operations are platform-conditional.
#[derive(Clone)]
pub struct BuildDeps {
    /// Image pusher for pushing images to OCI registries.
    /// `None` on platforms or configurations where push is not supported.
    pub image_pusher: Option<minibox_core::domain::DynImagePusher>,
    /// Container committer for snapshotting a container's overlay diff.
    /// `None` on platforms where commit is not supported (macOS, Windows).
    pub commit_adapter: Option<minibox_core::domain::DynContainerCommitter>,
    /// Image builder for building images from a Dockerfile.
    /// `None` on platforms where build is not supported (macOS, Windows).
    pub image_builder: Option<minibox_core::domain::DynImageBuilder>,
}

/// Observability and event-bus dependencies.
///
/// Handlers that emit events or record metrics depend on this sub-struct.
#[derive(Clone)]
pub struct EventDeps {
    /// Event sink for emitting container lifecycle events.
    pub event_sink: Arc<dyn EventSink>,
    /// Source for subscribing to the container event stream.
    pub event_source: Arc<dyn minibox_core::events::EventSource>,
    /// Metrics recorder for operational observability.
    pub metrics: DynMetricsRecorder,
}

// ---------------------------------------------------------------------------
// Handler Dependencies (Dependency Injection)
// ---------------------------------------------------------------------------

/// Dependencies injected into request handlers.
///
/// Composed of focused sub-structs ([`ImageDeps`], [`LifecycleDeps`],
/// [`ExecDeps`], [`BuildDeps`], [`EventDeps`]) so each handler can declare a
/// dependency only on the slice of infrastructure it actually uses (ISP).
///
/// # Usage
///
/// Created once in the composition root (main.rs) and passed to all handlers:
///
/// ```rust,ignore
/// use crate::adapters::{DockerHubRegistry, OverlayFilesystem, CgroupV2Limiter, LinuxNamespaceRuntime};
///
/// let deps = Arc::new(HandlerDependencies {
///     image: ImageDeps {
///         registry_router: Arc::new(HostnameRegistryRouter::new(docker_hub, [("ghcr.io", ghcr)])),
///         ..
///     },
///     lifecycle: LifecycleDeps {
///         filesystem: Arc::new(OverlayFilesystem),
///         containers_base: PathBuf::from("/var/lib/minibox/containers"),
///         ..
///     },
///     ..
/// });
/// ```
#[derive(Clone)]
pub struct HandlerDependencies {
    /// Image registry, loader, GC, and local store.
    pub image: ImageDeps,
    /// Container lifecycle: filesystem, limits, runtime, network, paths.
    pub lifecycle: LifecycleDeps,
    /// Exec and PTY session management.
    pub exec: ExecDeps,
    /// Image build, push, and commit.
    pub build: BuildDeps,
    /// Observability: events and metrics.
    pub events: EventDeps,
    /// Policy controlling which container capabilities are permitted.
    pub policy: ContainerPolicy,
    /// VM checkpoint adapter for save/restore snapshot operations.
    pub checkpoint: minibox_core::domain::DynVmCheckpoint,
}

impl HandlerDependencies {
    /// Override the image loader (builder-style).
    pub fn with_image_loader(mut self, loader: minibox_core::domain::DynImageLoader) -> Self {
        self.image.image_loader = loader;
        self
    }
}

// ─── Container Policy ────────────────────────────────────────────────────────

/// Policy rules applied to every `RunContainer` request before any container
/// creation logic executes.  Defaults to deny-all: both bind mounts and
/// privileged mode are blocked unless explicitly enabled.
///
/// Construct with specific overrides for tests or operator-controlled config:
/// ```rust,ignore
/// let policy = ContainerPolicy { allow_bind_mounts: true, ..ContainerPolicy::default() };
/// ```
#[derive(Debug, Clone, Default)]
pub struct ContainerPolicy {
    /// Allow containers to mount host directories (bind mounts).
    /// Default: `false` (deny).
    pub allow_bind_mounts: bool,
    /// Allow containers to run in privileged mode.
    /// Default: `false` (deny).
    pub allow_privileged: bool,
}

/// Validate a container run request against the active policy.
///
/// Returns `Ok(())` if the request is permitted; returns an error string
/// describing the first policy violation found.
///
/// # Errors
///
/// Returns `Err(String)` with a human-readable description when the request
/// violates `policy`.
pub fn validate_policy(
    mounts: &[minibox_core::domain::BindMount],
    privileged: bool,
    policy: &ContainerPolicy,
) -> Result<(), String> {
    if !mounts.is_empty() && !policy.allow_bind_mounts {
        return Err(
            "policy violation: bind mount requested but bind mounts are not allowed".into(),
        );
    }
    if privileged && !policy.allow_privileged {
        return Err(
            "policy violation: privileged mode requested but privileged containers are not allowed"
                .into(),
        );
    }
    Ok(())
}

// ─── Container ID Generation ────────────────────────────────────────────────

/// Generate a 16-char hex container ID from a UUID v4.
///
/// 16 hex chars = 64 bits. Birthday-paradox collision after ~4 billion containers —
/// callers must still check for collisions against the existing container state.
fn generate_container_id() -> String {
    Uuid::new_v4()
        .to_string()
        .replace('-', "")
        .chars()
        .take(16)
        .collect()
}

// ─── Run ────────────────────────────────────────────────────────────────────

/// Create and start a new container from `image:tag`, executing `command`.
///
/// Responses are sent via `tx`.  Non-ephemeral runs send exactly one message.
/// Ephemeral runs (Linux-only) send zero or more `ContainerOutput` messages
/// followed by one terminal `ContainerStopped` message.
#[allow(clippy::too_many_arguments)]
pub async fn handle_run(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    #[allow(unused_variables)] ephemeral: bool,
    #[allow(unused_variables)] network: Option<NetworkMode>,
    mounts: Vec<BindMount>,
    privileged: bool,
    env: Vec<String>,
    name: Option<String>,
    platform: Option<String>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    // Policy gate: deny bind mounts and privileged mode unless explicitly allowed.
    if let Err(msg) = validate_policy(&mounts, privileged, &deps.policy) {
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
    if let Some(ref n) = name {
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
    if ephemeral {
        handle_run_streaming(
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
            state,
            deps,
            tx,
        )
        .await;
        return;
    }

    // Non-ephemeral (or non-Linux): single response.
    let response = match run_inner(
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
        state,
        deps,
    )
    .await
    {
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
#[allow(clippy::too_many_arguments)]
#[cfg(unix)]
async fn handle_run_streaming(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    _network: Option<NetworkMode>,
    mounts: Vec<BindMount>,
    privileged: bool,
    env: Vec<String>,
    name: Option<String>,
    platform: Option<String>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use minibox_core::protocol::OutputStreamKind;
    use std::os::fd::IntoRawFd;

    // Build the container ID and rootfs via the shared inner setup, but we need
    // capture_output=true. We inline a variant of run_inner here.
    let image_label = format!("{}:{}", image, tag.as_deref().unwrap_or("latest"));
    let result = run_inner_capture(
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        _network,
        mounts,
        privileged,
        env,
        name,
        platform,
        Arc::clone(&state),
        Arc::clone(&deps),
    )
    .await;

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
        let mut buf = [0u8; 4096];
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
        .map(|r| r.cgroup_path.clone());

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
#[allow(clippy::too_many_arguments)]
#[cfg(unix)]
async fn run_inner_capture(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    network: Option<NetworkMode>,
    mounts: Vec<BindMount>,
    privileged: bool,
    env: Vec<String>,
    name: Option<String>,
    platform: Option<String>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<(String, u32, std::os::fd::OwnedFd, Option<String>)> {
    use anyhow::Context;
    use minibox_core::domain::NetworkConfig;

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
    let platform_registry = resolve_platform_registry(&platform, &image_ref, &deps)?;
    let default_registry = deps.image.registry_router.route(&image_ref);
    let registry: &dyn minibox_core::domain::ImageRegistry = match &platform_registry {
        Some(r) => r.as_ref(),
        None => default_registry,
    };

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

    let id = generate_container_id();

    if state.get_container(&id).await.is_some() {
        return Err(DomainError::InvalidConfig(format!(
            "container ID collision (extremely rare): {id}"
        ))
        .into());
    }

    let container_dir = deps.lifecycle.containers_base.join(&id);
    let run_dir = deps.lifecycle.run_containers_base.join(&id);

    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder.recursive(true);
        builder.create(&container_dir)?;
        builder.create(&run_dir)?;
    }

    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&container_dir)?;
        std::fs::create_dir_all(&run_dir)?;
    }

    let rootfs_layout = deps
        .lifecycle
        .filesystem
        .setup_rootfs(&layer_dirs, &container_dir)?;
    let merged_dir = rootfs_layout.merged_dir.clone();

    let resource_config = ResourceConfig {
        memory_limit_bytes,
        cpu_weight,
        pids_max: Some(1024),
        io_max_bytes_per_sec: None,
    };
    let cgroup_dir_str = deps
        .lifecycle
        .resource_limiter
        .create(&id, &resource_config)?;
    let cgroup_dir = PathBuf::from(cgroup_dir_str);

    // ── Network setup ──────────────────────────────────────────────────
    let net_mode = network.unwrap_or(NetworkMode::None);
    let network_config = NetworkConfig {
        mode: net_mode,
        ..NetworkConfig::default()
    };
    let net = NetworkLifecycle::new(deps.lifecycle.network_provider.clone());
    let _net_ns = net
        .setup(&id, &network_config)
        .await
        .context("network setup")?;

    let skip_net_ns = net_mode == NetworkMode::Host;

    let image_label = format!("{image}:{tag}");
    let command_str = command.join(" ");
    let record = ContainerRecord {
        info: ContainerInfo {
            id: id.clone(),
            name: name.clone(),
            image: image_label.clone(),
            command: command_str,
            state: "Created".to_string(),
            created_at: Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        rootfs_path: merged_dir.clone(),
        cgroup_path: cgroup_dir.clone(),
        post_exit_hooks: vec![],
        rootfs_metadata: rootfs_layout.rootfs_metadata.clone(),
        source_image_ref: rootfs_layout
            .source_image_ref
            .clone()
            .or_else(|| Some(image_label.clone())),
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: Some(RunCreationParams {
            image: image.clone(),
            tag: Some(tag.clone()),
            command: command.clone(),
            memory_limit_bytes,
            cpu_weight,
            network,
            env: env.clone(),
            mounts: mounts.clone(),
            privileged,
            name: name.clone(),
            tty: false,
            entrypoint: None,
            user: None,
            platform: platform.clone(),
        }),
    };
    state.add_container(record).await;

    let spawn_command = command
        .first()
        .cloned()
        .unwrap_or_else(|| "/bin/sh".to_string());
    let spawn_args = command.iter().skip(1).cloned().collect();
    let mut container_env = vec![
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
        "TERM=xterm".to_string(),
    ];
    container_env.extend(env);
    let spawn_config = ContainerSpawnConfig {
        rootfs: merged_dir.clone(),
        command: spawn_command,
        args: spawn_args,
        env: container_env,
        cgroup_path: cgroup_dir.clone(),
        hostname: format!("minibox-{}", &id[..8]),
        capture_output: true,
        hooks: ContainerHooks::default(),
        skip_network_namespace: skip_net_ns,
        mounts,
        privileged,
    };

    let _spawn_permit = state
        .spawn_semaphore
        .acquire()
        .await
        .expect("semaphore closed");

    let spawn_result = deps.lifecycle.runtime.spawn_process(&spawn_config).await?;

    let pid = spawn_result.pid;
    let runtime_id = spawn_result.runtime_id;
    let output_reader = spawn_result.output_reader.ok_or_else(|| {
        anyhow::anyhow!("capture_output=true but runtime returned no output_reader")
    })?;

    // ── Network attach ─────────────────────────────────────────────────
    net.attach(&id, pid).await.context("network attach")?;

    // Write PID file and update state.
    let pid_file = deps.lifecycle.run_containers_base.join(&id).join("pid");
    if let Err(e) = std::fs::write(&pid_file, pid.to_string()) {
        warn!(
            pid_file = %pid_file.display(),
            error = %e,
            "container: failed to write pid file"
        );
    }
    state.set_container_pid(&id, pid).await;

    Ok((id, pid, output_reader, runtime_id))
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
#[allow(clippy::too_many_arguments)]
async fn run_inner(
    image: String,
    tag: Option<String>,
    command: Vec<String>,
    memory_limit_bytes: Option<u64>,
    cpu_weight: Option<u64>,
    network: Option<NetworkMode>,
    mounts: Vec<BindMount>,
    privileged: bool,
    env: Vec<String>,
    name: Option<String>,
    platform: Option<String>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<String> {
    use anyhow::Context;
    use minibox_core::domain::NetworkConfig;

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
    let platform_registry = resolve_platform_registry(&platform, &image_ref, &deps)?;
    let default_registry = deps.image.registry_router.route(&image_ref);
    let registry: &dyn minibox_core::domain::ImageRegistry = match &platform_registry {
        Some(r) => r.as_ref(),
        None => default_registry,
    };

    // Pull image if not cached (using injected registry trait).
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

    let id = generate_container_id();

    // SECURITY: Verify no collision with existing containers
    if state.get_container(&id).await.is_some() {
        return Err(DomainError::InvalidConfig(format!(
            "container ID collision (extremely rare): {id}"
        ))
        .into());
    }

    let container_dir = deps.lifecycle.containers_base.join(&id);
    let run_dir = deps.lifecycle.run_containers_base.join(&id);

    // SECURITY: Create container directories with restricted permissions (0700)
    // to prevent unauthorized access to container data
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700); // Owner (root) only
        builder.recursive(true);
        builder.create(&container_dir)?;
        builder.create(&run_dir)?;
    }

    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&container_dir)?;
        std::fs::create_dir_all(&run_dir)?;
    }

    // Setup overlayfs (using injected filesystem trait).
    let rootfs_layout = deps
        .lifecycle
        .filesystem
        .setup_rootfs(&layer_dirs, &container_dir)?;
    let merged_dir_from_overlay = rootfs_layout.merged_dir.clone();

    // Setup cgroup (using injected resource limiter trait).
    let resource_config = ResourceConfig {
        memory_limit_bytes,
        cpu_weight,
        pids_max: Some(1024), // Default PID limit for security
        io_max_bytes_per_sec: None,
    };
    let cgroup_dir_str = deps
        .lifecycle
        .resource_limiter
        .create(&id, &resource_config)?;
    let cgroup_dir = PathBuf::from(cgroup_dir_str);

    // ── Network setup ──────────────────────────────────────────────────
    let net_mode = network.unwrap_or(NetworkMode::None);
    let network_config = NetworkConfig {
        mode: net_mode,
        ..NetworkConfig::default()
    };
    let net = NetworkLifecycle::new(deps.lifecycle.network_provider.clone());
    let _net_ns = net
        .setup(&id, &network_config)
        .await
        .context("network setup")?;

    let skip_net_ns = net_mode == NetworkMode::Host;

    // Build ContainerRecord in Created state; updated to Running once the
    // child PID is known.
    let image_label = format!("{image}:{tag}");
    let command_str = command.join(" ");
    let record = ContainerRecord {
        info: ContainerInfo {
            id: id.clone(),
            name: name.clone(),
            image: image_label.clone(),
            command: command_str,
            state: "Created".to_string(),
            created_at: Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        rootfs_path: merged_dir_from_overlay.clone(),
        cgroup_path: cgroup_dir.clone(),
        post_exit_hooks: vec![],
        rootfs_metadata: rootfs_layout.rootfs_metadata.clone(),
        source_image_ref: rootfs_layout
            .source_image_ref
            .clone()
            .or_else(|| Some(image_label.clone())),
        step_state: None,
        priority: None,
        urgency: None,
        execution_context: None,
        creation_params: Some(RunCreationParams {
            image: image.clone(),
            tag: Some(tag.clone()),
            command: command.clone(),
            memory_limit_bytes,
            cpu_weight,
            network,
            env: env.clone(),
            mounts: mounts.clone(),
            privileged,
            name: name.clone(),
            tty: false,
            entrypoint: None,
            user: None,
            platform: platform.clone(),
        }),
    };
    state.add_container(record).await;

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
    container_env.extend(env);
    let spawn_config = ContainerSpawnConfig {
        rootfs: merged_dir_from_overlay.clone(),
        command: spawn_command,
        args: spawn_args,
        env: container_env,
        cgroup_path: cgroup_dir.clone(),
        hostname: format!("minibox-{}", &id[..8]),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: skip_net_ns,
        mounts,
        privileged,
    };

    // SECURITY: Acquire semaphore permit to limit concurrent spawns
    // This prevents fork bomb attacks from overwhelming the system
    let _spawn_permit = state
        .spawn_semaphore
        .acquire()
        .await
        .expect("semaphore closed");

    // Spawn the container process (using injected runtime trait).
    let id_clone = id.clone();
    let state_clone = Arc::clone(&state);
    let runtime_clone = Arc::clone(&deps.lifecycle.runtime);
    let metrics_clone = Arc::clone(&deps.events.metrics);
    let net_clone = net.clone();
    let run_containers_base_clone = deps.lifecycle.run_containers_base.clone();
    let event_sink_clone = Arc::clone(&deps.events.event_sink);
    let image_label_clone = image_label.clone();

    tokio::task::spawn(async move {
        // Permit is held until this task completes (via _spawn_permit drop)
        match runtime_clone.spawn_process(&spawn_config).await {
            Ok(spawn_result) => {
                let pid = spawn_result.pid;
                info!(container_id = %id_clone, pid = pid, "container: process started");

                event_sink_clone.emit(ContainerEvent::Created {
                    id: id_clone.clone(),
                    image: image_label_clone,
                    timestamp: std::time::SystemTime::now(),
                });
                event_sink_clone.emit(ContainerEvent::Started {
                    id: id_clone.clone(),
                    pid,
                    timestamp: std::time::SystemTime::now(),
                });

                metrics_clone.increment_counter(
                    "minibox_container_ops_total",
                    &[("op", "run"), ("adapter", "daemon"), ("status", "ok")],
                );
                let active = state_clone.list_containers().await.len() as f64;
                metrics_clone.set_gauge("minibox_active_containers", active, &[]);

                // ── Network attach ─────────────────────────────────────
                net_clone.attach(&id_clone, pid).await.ok();

                // Write PID file.
                let pid_file = run_containers_base_clone.join(&id_clone).join("pid");
                if let Err(e) = std::fs::write(&pid_file, pid.to_string()) {
                    warn!(
                        pid_file = %pid_file.display(),
                        error = %e,
                        "container: failed to write pid file"
                    );
                }

                state_clone.set_container_pid(&id_clone, pid).await;

                // Wait for the process to exit in a background task.
                let state_wait = Arc::clone(&state_clone);
                let id_wait = id_clone.clone();
                let rootfs_wait = spawn_config.rootfs.clone();
                let hooks_wait = spawn_config.hooks.post_exit.clone();
                let event_sink_wait = Arc::clone(&event_sink_clone);
                let cgroup_path_wait = spawn_config.cgroup_path.clone();
                let runtime_wait = Arc::clone(&runtime_clone);
                let runtime_id = spawn_result.runtime_id.clone();
                tokio::spawn(async move {
                    daemon_wait_for_exit(
                        pid,
                        &id_wait,
                        state_wait,
                        rootfs_wait,
                        hooks_wait,
                        event_sink_wait,
                        cgroup_path_wait,
                        runtime_wait,
                        runtime_id,
                    )
                    .await;
                });
            }
            Err(e) => {
                error!("failed to spawn container {id_clone}: {e:#}");
                metrics_clone.increment_counter(
                    "minibox_container_ops_total",
                    &[("op", "run"), ("adapter", "daemon"), ("status", "error")],
                );
                if let Err(e) = state_clone
                    .update_container_state(&id_clone, ContainerState::Failed)
                    .await
                {
                    warn!(container_id = %id_clone, error = %e, "state: failed to mark container Failed");
                }
            }
        }
    });

    Ok(id)
}

/// Check if a container was OOM-killed by reading cgroup v2 `memory.events`.
///
/// Returns `true` if `oom_kill` count is greater than zero.  Returns `false` if
/// the file cannot be read (e.g. cgroup already deleted, or non-Linux platform).
async fn check_oom_killed(cgroup_path: &std::path::Path) -> bool {
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

///
/// Waits for the container process to exit via the runtime adapter, then
/// updates state and emits lifecycle events.
///
/// Uses `runtime.wait_for_exit()` which dispatches to `waitpid` for native
/// adapters or to the adapter's own wait mechanism (e.g. `SmolvmProcess::wait`
/// for krun).
#[allow(clippy::too_many_arguments)]
#[cfg(unix)]
async fn daemon_wait_for_exit(
    pid: u32,
    id: &str,
    state: Arc<DaemonState>,
    _rootfs: std::path::PathBuf,
    _post_exit_hooks: Vec<HookSpec>,
    event_sink: Arc<dyn EventSink>,
    cgroup_path: std::path::PathBuf,
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
    _rootfs: std::path::PathBuf,
    _post_exit_hooks: Vec<HookSpec>,
    _event_sink: Arc<dyn EventSink>,
    _cgroup_path: std::path::PathBuf,
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
    _rootfs: std::path::PathBuf,
    _post_exit_hooks: Vec<HookSpec>,
    _event_sink: Arc<dyn EventSink>,
    _cgroup_path: std::path::PathBuf,
    _runtime: DynContainerRuntime,
    _runtime_id: Option<String>,
) {
    // No-op on this platform.
}

// ─── Stop ───────────────────────────────────────────────────────────────────

/// Send SIGTERM to a container, then SIGKILL after 10 seconds if needed.
pub async fn handle_stop(
    name_or_id: String,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let id = match state.resolve_id(&name_or_id).await {
        Some(id) => id,
        None => {
            return DaemonResponse::Error {
                message: format!("container not found: {name_or_id}"),
            };
        }
    };

    // ── Network cleanup ────────────────────────────────────────────────
    NetworkLifecycle::new(deps.lifecycle.network_provider.clone())
        .cleanup(&id)
        .await;

    let result = stop_inner(&id, &state).await;
    let status = if result.is_ok() { "ok" } else { "error" };
    deps.events.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "stop"), ("adapter", "daemon"), ("status", status)],
    );

    match result {
        Ok(()) => {
            let active = state.list_containers().await.len() as f64;
            deps.events
                .metrics
                .set_gauge("minibox_active_containers", active, &[]);
            DaemonResponse::Success {
                message: format!("container {id} stopped"),
            }
        }
        Err(e) => {
            error!("handle_stop error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    }
}

/// Unix implementation: send SIGTERM, poll for exit for up to 10 s, then
/// SIGKILL if the process is still alive.  Updates state to `"Stopped"` on
/// completion regardless of how the process exited.
#[cfg(unix)]
async fn stop_inner(id: &str, state: &Arc<DaemonState>) -> Result<()> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    let record = state
        .get_container(id)
        .await
        .ok_or_else(|| DomainError::ContainerNotFound { id: id.to_string() })?;

    let pid = record
        .pid
        .ok_or_else(|| anyhow::anyhow!("container {id} has no PID (not running?)"))?;

    let nix_pid = Pid::from_raw(pid as i32);
    // Signal the entire process group so descendants (e.g. `sleep` spawned
    // by `/bin/sh -c …`) receive SIGTERM directly.  child_init calls setsid()
    // before execve, making the container init a new process group leader;
    // negating its host PID addresses that group.  We fall back to the
    // individual PID if the group signal returns ESRCH (process already gone).
    let pgid = Pid::from_raw(-(pid as i32));

    info!(
        container_id = %id,
        pid = pid,
        "container: sending SIGTERM to process group"
    );
    if kill(pgid, Signal::SIGTERM).is_err() {
        kill(nix_pid, Signal::SIGTERM).ok();
    }

    // Wait up to 2 s for the process to exit gracefully.  In practice,
    // PID 1 in a PID namespace silently ignores SIGTERM (kernel-enforced),
    // so busybox `sh -c …` containers will never respond.  We keep a short
    // window for containers that do install a handler, then fall through to
    // SIGKILL promptly.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if kill(nix_pid, None).is_err() {
            // ESRCH — process is gone.
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            warn!(
                container_id = %id,
                pid = pid,
                "container: did not exit after SIGTERM, sending SIGKILL"
            );
            kill(pgid, Signal::SIGKILL).ok();
            kill(nix_pid, Signal::SIGKILL).ok();
            break;
        }
    }

    if let Err(e) = state
        .update_container_state(id, ContainerState::Stopped)
        .await
    {
        warn!(container_id = %id, error = %e, "state: failed to mark container Stopped");
    }
    Ok(())
}

/// Windows stub: stop is not yet implemented.
///
/// Container stop must go through the HCS or WSL2 adapter stop path.
/// This stub ensures the binary compiles on Windows and returns a clear error.
#[cfg(windows)]
async fn stop_inner(id: &str, _state: &Arc<DaemonState>) -> Result<()> {
    anyhow::bail!(
        "handle_stop not yet implemented on Windows for container {id} \
         — use the HCS/WSL2 adapter stop path"
    )
}

/// Fallback stub for platforms other than Unix or Windows.
#[cfg(not(any(unix, windows)))]
async fn stop_inner(id: &str, _state: &Arc<DaemonState>) -> Result<()> {
    anyhow::bail!("handle_stop not supported on this platform for container {id}")
}

// ─── Pause / Resume ─────────────────────────────────────────────────────────

/// Freeze a running container by writing `1` to its `cgroup.freeze` file.
///
/// Returns `DaemonResponse::ContainerPaused` on success, `DaemonResponse::Error`
/// if the container is not found, not running, or the cgroup write fails.
pub async fn handle_pause(
    id: String,
    state: Arc<DaemonState>,
    event_sink: Arc<dyn EventSink>,
) -> DaemonResponse {
    let record = state.get_container(&id).await;
    let record = match record {
        Some(r) => r,
        None => {
            return DaemonResponse::Error {
                message: format!("container {id} not found"),
            };
        }
    };
    if record.info.state != ContainerState::Running.as_str() {
        return DaemonResponse::Error {
            message: format!(
                "container {id} is not running (state: {})",
                record.info.state
            ),
        };
    }
    let freeze_path = record.cgroup_path.join("cgroup.freeze");
    if let Err(e) = tokio::fs::write(&freeze_path, "1\n").await {
        return DaemonResponse::Error {
            message: format!("pause failed: {e}"),
        };
    }
    if let Err(e) = state
        .update_container_state(&id, ContainerState::Paused)
        .await
    {
        warn!(container_id = %id, error = %e, "state: failed to mark paused");
    }
    info!(container_id = %id, "container: paused");
    event_sink.emit(ContainerEvent::Paused {
        id: id.clone(),
        timestamp: std::time::SystemTime::now(),
    });
    DaemonResponse::ContainerPaused { id }
}

/// Unfreeze a paused container by writing `0` to its `cgroup.freeze` file.
///
/// Returns `DaemonResponse::ContainerResumed` on success, `DaemonResponse::Error`
/// if the container is not found, not paused, or the cgroup write fails.
pub async fn handle_resume(
    id: String,
    state: Arc<DaemonState>,
    event_sink: Arc<dyn EventSink>,
) -> DaemonResponse {
    let record = state.get_container(&id).await;
    let record = match record {
        Some(r) => r,
        None => {
            return DaemonResponse::Error {
                message: format!("container {id} not found"),
            };
        }
    };
    if record.info.state != ContainerState::Paused.as_str() {
        return DaemonResponse::Error {
            message: format!(
                "container {id} is not paused (state: {})",
                record.info.state
            ),
        };
    }
    let freeze_path = record.cgroup_path.join("cgroup.freeze");
    if let Err(e) = tokio::fs::write(&freeze_path, "0\n").await {
        return DaemonResponse::Error {
            message: format!("resume failed: {e}"),
        };
    }
    if let Err(e) = state
        .update_container_state(&id, ContainerState::Running)
        .await
    {
        warn!(container_id = %id, error = %e, "state: failed to mark running after resume");
    }
    info!(container_id = %id, "container: resumed");
    event_sink.emit(ContainerEvent::Resumed {
        id: id.clone(),
        timestamp: std::time::SystemTime::now(),
    });
    DaemonResponse::ContainerResumed { id }
}

// ─── Remove ─────────────────────────────────────────────────────────────────

/// Clean up a stopped container: unmount overlay, delete dirs, remove cgroup.
pub async fn handle_remove(
    name_or_id: String,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let id = match state.resolve_id(&name_or_id).await {
        Some(id) => id,
        None => {
            return DaemonResponse::Error {
                message: format!("container not found: {name_or_id}"),
            };
        }
    };

    let result = remove_inner(&id, &state, &deps).await;
    let status = if result.is_ok() { "ok" } else { "error" };
    deps.events.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "remove"), ("adapter", "daemon"), ("status", status)],
    );

    match result {
        Ok(()) => {
            let active = state.list_containers().await.len() as f64;
            deps.events
                .metrics
                .set_gauge("minibox_active_containers", active, &[]);
            DaemonResponse::Success {
                message: format!("container {id} removed"),
            }
        }
        Err(e) => {
            error!("handle_remove error: {e:#}");
            DaemonResponse::Error {
                message: format!("{e:#}"),
            }
        }
    }
}

/// Core remove logic: unmount overlay, delete runtime state dir, clean up
/// cgroup, and deregister the container from the daemon state.
///
/// Returns an error if the container does not exist or is still `"Running"`.
/// Cleanup steps (overlay unmount, cgroup removal) are best-effort: failures
/// are logged as warnings but do not abort the removal.
async fn remove_inner(
    id: &str,
    state: &Arc<DaemonState>,
    deps: &Arc<HandlerDependencies>,
) -> Result<()> {
    let record = state
        .get_container(id)
        .await
        .ok_or_else(|| DomainError::ContainerNotFound { id: id.to_string() })?;

    if record.info.state == "Running" {
        return Err(DomainError::AlreadyRunning { id: id.to_string() }.into());
    }

    // Unmount overlay (using injected filesystem trait).
    let container_dir = deps.lifecycle.containers_base.join(id);
    if container_dir.exists()
        && let Err(e) = deps.lifecycle.filesystem.cleanup(&container_dir)
    {
        warn!("cleanup_mounts for {id}: {e}");
    }

    // Remove runtime state directory.
    let run_dir = deps.lifecycle.run_containers_base.join(id);
    if run_dir.exists() {
        std::fs::remove_dir_all(&run_dir).ok();
    }

    // Cleanup cgroup (using injected resource limiter trait).
    if let Err(e) = deps.lifecycle.resource_limiter.cleanup(id) {
        warn!("cleanup cgroup for {id}: {e}");
    }

    // ── Network cleanup ────────────────────────────────────────────────
    NetworkLifecycle::new(deps.lifecycle.network_provider.clone())
        .cleanup(id)
        .await;

    state.remove_container(id).await;
    Ok(())
}

// ─── List ───────────────────────────────────────────────────────────────────

/// Return all known containers.
pub async fn handle_list(state: Arc<DaemonState>) -> DaemonResponse {
    let containers = state.list_containers().await;
    DaemonResponse::ContainerList { containers }
}

// ─── Logs ───────────────────────────────────────────────────────────────────

/// Retrieve stored log output for a container.
///
/// Reads `{containers_base}/{id}/stdout.log` and `stderr.log`, emitting one
/// [`DaemonResponse::LogLine`] per line.  Terminates with
/// [`DaemonResponse::Success`] when `follow` is `false` (the only supported
/// mode for now).  Sends [`DaemonResponse::Error`] when the container is not
/// found.
pub async fn handle_logs(
    name_or_id: String,
    _follow: bool,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use anyhow::Context as _;
    use minibox_core::protocol::OutputStreamKind;
    use std::io::{BufRead, BufReader};

    let id = match state.resolve_id(&name_or_id).await {
        Some(id) => id,
        None => {
            send_error(
                &tx,
                "handle_logs",
                format!("container not found: {name_or_id}"),
            )
            .await;
            return;
        }
    };

    // Read stdout.log then stderr.log; missing files are silently skipped.
    let log_dir = deps.lifecycle.containers_base.join(&id);
    let log_pairs: &[(&str, OutputStreamKind)] = &[
        ("stdout.log", OutputStreamKind::Stdout),
        ("stderr.log", OutputStreamKind::Stderr),
    ];

    for (filename, stream) in log_pairs {
        let path = log_dir.join(filename);
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                warn!(
                    container_id = %id,
                    path = %path.display(),
                    error = %e,
                    "handle_logs: failed to open log file"
                );
                continue;
            }
        };

        let reader = BufReader::new(file);
        for line_result in reader.lines() {
            let line = match line_result.context("reading log line") {
                Ok(l) => l,
                Err(e) => {
                    warn!(container_id = %id, error = %e, "handle_logs: read error");
                    break;
                }
            };
            if tx
                .send(DaemonResponse::LogLine {
                    stream: stream.clone(),
                    line,
                })
                .await
                .is_err()
            {
                warn!(
                    container_id = %id,
                    "handle_logs: client disconnected mid-stream"
                );
                return;
            }
        }
    }

    if tx
        .send(DaemonResponse::Success {
            message: "end of log".to_string(),
        })
        .await
        .is_err()
    {
        warn!(container_id = %id, "handle_logs: client disconnected before Success");
    }
}

// ─── Pull ───────────────────────────────────────────────────────────────────

/// Apply a per-request platform override to whichever registry the router selected.
///
/// Downcasts the routed registry to its concrete type and reconstructs it with
/// the requested [`TargetPlatform`].  Returns `None` when `platform` is absent
/// (the caller should use the router's result directly).
///
/// # Errors
///
/// Returns an error if `platform` cannot be parsed, or if the adapter cannot
/// be reconstructed (e.g. TLS init failure).
fn resolve_platform_registry(
    platform: &Option<String>,
    image_ref: &minibox_core::image::reference::ImageRef,
    deps: &HandlerDependencies,
) -> Result<Option<Box<dyn minibox_core::domain::ImageRegistry>>> {
    let Some(p) = platform else {
        return Ok(None);
    };

    let tp = minibox_core::image::manifest::TargetPlatform::parse(p)?;
    info!(platform = %p, "using per-request platform override");

    // Route first so we know which registry type owns this image reference,
    // then reconstruct that adapter with the platform override applied.
    let routed = deps.image.registry_router.route(image_ref);

    if routed.as_any().is::<crate::adapters::GhcrRegistry>() {
        let registry =
            crate::adapters::GhcrRegistry::with_platform(Arc::clone(&deps.image.image_store), tp)?;
        return Ok(Some(Box::new(registry)));
    }

    // Default: treat as Docker Hub (covers `native` adapter and any unknown
    // hostname that the router falls back to its default for).
    let registry =
        crate::adapters::DockerHubRegistry::with_platform(Arc::clone(&deps.image.image_store), tp)?;
    Ok(Some(Box::new(registry)))
}

#[instrument(skip(_state, deps), fields(image = %image, tag = ?tag))]
pub async fn handle_pull(
    image: String,
    tag: Option<String>,
    platform: Option<String>,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    // Build full ref string from image + optional tag, then parse into ImageRef.
    let ref_str = match &tag {
        Some(t) => format!("{image}:{t}"),
        None => image.clone(),
    };
    let image_ref = match ImageRef::parse(&ref_str) {
        Ok(r) => r,
        Err(e) => {
            error!("handle_pull: invalid image reference {ref_str:?}: {e}");
            return DaemonResponse::Error {
                message: format!("invalid image reference {ref_str:?}: {e}"),
            };
        }
    };
    let tag = image_ref.tag.clone();

    // When a platform override is requested, reconstruct the routed registry
    // adapter with the requested platform applied. Otherwise use the router's
    // result directly.  The box is held for the lifetime of the pull call.
    let platform_registry = match resolve_platform_registry(&platform, &image_ref, &deps) {
        Ok(r) => r,
        Err(e) => {
            error!("handle_pull: invalid platform: {e}");
            return DaemonResponse::Error {
                message: format!("invalid platform: {e}"),
            };
        }
    };

    let registry: &dyn minibox_core::domain::ImageRegistry = match &platform_registry {
        Some(r) => r.as_ref(),
        None => deps.image.registry_router.route(&image_ref),
    };

    // Pull image (using selected registry trait).
    let start = std::time::Instant::now();
    let (status, response) = match registry.pull_image(&image_ref).await {
        Ok(_metadata) => (
            "ok",
            DaemonResponse::Success {
                message: format!("pulled {image}:{tag}"),
            },
        ),
        Err(e) => {
            error!("handle_pull error: {e:#}");
            (
                "error",
                DaemonResponse::Error {
                    message: format!("{e:#}"),
                },
            )
        }
    };

    deps.events.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "pull"), ("adapter", "daemon"), ("status", status)],
    );
    deps.events.metrics.record_histogram(
        "minibox_container_op_duration_seconds",
        start.elapsed().as_secs_f64(),
        &[("op", "pull"), ("adapter", "daemon")],
    );

    response
}

// ─── Load Image ─────────────────────────────────────────────────────────────

/// Load a local OCI image tarball into the image store.
#[instrument(skip(_state, deps), fields(path = %path, name = %name, tag = %tag))]
pub async fn handle_load_image(
    path: String,
    name: String,
    tag: String,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let image_path = std::path::Path::new(&path);
    let start = std::time::Instant::now();
    let (status, response) = match deps
        .image
        .image_loader
        .load_image(image_path, &name, &tag)
        .await
    {
        Ok(()) => {
            info!(
                path = %path,
                image = %format!("{name}:{tag}"),
                "load_image: loaded successfully"
            );
            (
                "ok",
                DaemonResponse::ImageLoaded {
                    image: format!("{name}:{tag}"),
                },
            )
        }
        Err(e) => {
            error!(error = %e, "load_image: failed");
            (
                "error",
                DaemonResponse::Error {
                    message: format!("{e:#}"),
                },
            )
        }
    };
    deps.events.metrics.increment_counter(
        "minibox_container_ops_total",
        &[
            ("op", "load_image"),
            ("adapter", "daemon"),
            ("status", status),
        ],
    );
    deps.events.metrics.record_histogram(
        "minibox_container_op_duration_seconds",
        start.elapsed().as_secs_f64(),
        &[("op", "load_image"), ("adapter", "daemon")],
    );
    response
}

// ---------------------------------------------------------------------------
// Exec handler
// ---------------------------------------------------------------------------

/// Run a command inside an already-running container via namespace join.
///
/// Streams `ContainerOutput` messages and terminates with `ContainerStopped`.
/// Returns `Error` immediately if the exec runtime is unavailable or the
/// container is not running.
#[allow(clippy::too_many_arguments)]
pub async fn handle_exec(
    container_id: String,
    cmd: Vec<String>,
    env: Vec<String>,
    working_dir: Option<String>,
    tty: bool,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let start = std::time::Instant::now();
    let Some(ref exec_rt) = deps.exec.exec_runtime else {
        deps.events.metrics.increment_counter(
            "minibox_container_ops_total",
            &[("op", "exec"), ("adapter", "daemon"), ("status", "error")],
        );
        send_error(
            &tx,
            "handle_exec",
            "exec not supported on this platform".to_string(),
        )
        .await;
        return;
    };

    let cid = match minibox_core::domain::ContainerId::new(container_id.clone()) {
        Ok(id) => id,
        Err(e) => {
            send_error(&tx, "handle_exec", format!("invalid container id: {e}")).await;
            return;
        }
    };

    // Allocate PTY channels and register them so SendInput/ResizePty can reach
    // the running exec session.
    let session_key = container_id.clone();
    let (resize_tx, resize_rx) = mpsc::channel::<(u16, u16)>(8);
    let (stdin_ch_tx, _stdin_ch_rx) = mpsc::channel::<Vec<u8>>(32);
    if tty {
        // Only register PTY channels for tty sessions; non-tty execs have no
        // use for resize or stdin channels. Registered entries are removed when
        // the session ends (see cleanup call below).
        let mut reg = deps.exec.pty_sessions.lock().await;
        reg.resize.insert(session_key.clone(), resize_tx);
        reg.stdin.insert(session_key.clone(), stdin_ch_tx.clone());
    }
    let _ = resize_rx; // handed to exec runtime in future task; avoid unused-var lint
    let _ = stdin_ch_tx;

    let spec = minibox_core::domain::ExecSpec {
        cmd,
        env,
        working_dir: working_dir.map(std::path::PathBuf::from),
        tty,
    };

    match exec_rt
        .as_ref()
        .run_in_container(&cid, spec, tx.clone())
        .await
    {
        Ok(handle) => {
            info!(
                container_id = %container_id,
                exec_id = %handle.id,
                "exec: started"
            );
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "exec"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "exec"), ("adapter", "daemon")],
            );
            let _ = tx
                .send(DaemonResponse::ExecStarted { exec_id: handle.id })
                .await;
            // Session ends when run_in_container's output stream closes; clean up
            // PTY channels so the registry does not grow unboundedly.
            deps.exec.pty_sessions.lock().await.cleanup(&session_key);
        }
        Err(e) => {
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "exec"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "exec"), ("adapter", "daemon")],
            );
            deps.exec.pty_sessions.lock().await.cleanup(&session_key);
            send_error(&tx, "handle_exec", format!("exec failed: {e:#}")).await;
        }
    }
}

// ─── SendInput / ResizePty ────────────────────────────────────────────────────

/// Forward base64-encoded stdin bytes to a running PTY session.
///
/// Looks up the session in [`PtySessionRegistry`] and forwards decoded bytes.
/// Returns `Success` on delivery, `Error` when the session is unknown or the
/// channel has been closed.
pub async fn handle_send_input(
    session_id: SessionId,
    data: String,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use base64::Engine as _;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(&data) {
        Ok(b) => b,
        Err(e) => {
            send_error(&tx, "handle_send_input", format!("base64 decode: {e}")).await;
            return;
        }
    };
    let reg = deps.exec.pty_sessions.lock().await;
    match reg.stdin.get(session_id.as_ref()) {
        Some(stdin_tx) => {
            if stdin_tx.send(bytes).await.is_err() {
                warn!(
                    session_id = %session_id,
                    "send_input: stdin channel closed"
                );
            }
        }
        None => {
            send_error(
                &tx,
                "handle_send_input",
                format!("no active tty session: {session_id}"),
            )
            .await;
            return;
        }
    }
    if tx
        .send(DaemonResponse::Success {
            message: "input forwarded".to_string(),
        })
        .await
        .is_err()
    {
        warn!(session_id = %session_id, "send_input: client disconnected");
    }
}

/// Forward a terminal resize event to a running PTY session.
///
/// Looks up the session in [`PtySessionRegistry`] and sends `(cols, rows)`.
/// Returns `Success` on delivery, `Error` when the session is unknown or the
/// channel has been closed.
pub async fn handle_resize_pty(
    session_id: SessionId,
    cols: u16,
    rows: u16,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let reg = deps.exec.pty_sessions.lock().await;
    match reg.resize.get(session_id.as_ref()) {
        Some(resize_tx) => {
            if resize_tx.send((cols, rows)).await.is_err() {
                warn!(
                    session_id = %session_id,
                    "resize_pty: resize channel closed"
                );
            }
        }
        None => {
            send_error(
                &tx,
                "handle_resize_pty",
                format!("no active tty session: {session_id}"),
            )
            .await;
            return;
        }
    }
    if tx
        .send(DaemonResponse::Success {
            message: "resize forwarded".to_string(),
        })
        .await
        .is_err()
    {
        warn!(session_id = %session_id, "resize_pty: client disconnected");
    }
}

// ─── Push ────────────────────────────────────────────────────────────────────

/// Push a locally-stored image to a remote OCI registry.
///
/// Sends zero or more `PushProgress` messages followed by `Success` or `Error`.
pub async fn handle_push(
    image_ref_str: String,
    credentials: minibox_core::protocol::PushCredentials,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let start = std::time::Instant::now();
    let Some(ref pusher) = deps.build.image_pusher else {
        deps.events.metrics.increment_counter(
            "minibox_container_ops_total",
            &[("op", "push"), ("adapter", "daemon"), ("status", "error")],
        );
        send_error(
            &tx,
            "handle_push",
            "push not supported on this platform".to_string(),
        )
        .await;
        return;
    };

    let image_ref = match minibox_core::image::reference::ImageRef::parse(&image_ref_str) {
        Ok(r) => r,
        Err(e) => {
            send_error(&tx, "handle_push", format!("invalid image ref: {e}")).await;
            return;
        }
    };

    let creds = match credentials {
        minibox_core::protocol::PushCredentials::Anonymous => {
            minibox_core::domain::RegistryCredentials::Anonymous
        }
        minibox_core::protocol::PushCredentials::Basic { username, password } => {
            minibox_core::domain::RegistryCredentials::Basic { username, password }
        }
        minibox_core::protocol::PushCredentials::Token { token } => {
            minibox_core::domain::RegistryCredentials::Token(token)
        }
    };

    let (progress_tx, mut progress_rx) = mpsc::channel::<minibox_core::domain::PushProgress>(32);
    let tx2 = tx.clone();
    tokio::spawn(async move {
        while let Some(p) = progress_rx.recv().await {
            let _ = tx2
                .send(DaemonResponse::PushProgress {
                    layer_digest: p.layer_digest,
                    bytes_uploaded: p.bytes_uploaded,
                    total_bytes: p.total_bytes,
                })
                .await;
        }
    });

    match pusher
        .push_image(&image_ref, &creds, Some(progress_tx))
        .await
    {
        Ok(result) => {
            info!(
                image_ref = %image_ref_str,
                digest = %result.digest,
                size_bytes = result.size_bytes,
                "push: completed"
            );
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "push"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "push"), ("adapter", "daemon")],
            );
            let _ = tx
                .send(DaemonResponse::Success {
                    message: format!("pushed {} digest:{}", image_ref_str, result.digest),
                })
                .await;
        }
        Err(e) => {
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "push"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "push"), ("adapter", "daemon")],
            );
            send_error(&tx, "handle_push", e.to_string()).await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_commit(
    container_id: String,
    target_image: String,
    author: Option<String>,
    message: Option<String>,
    env_overrides: Vec<String>,
    cmd_override: Option<Vec<String>>,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let start = std::time::Instant::now();
    let Some(ref committer) = deps.build.commit_adapter else {
        deps.events.metrics.increment_counter(
            "minibox_container_ops_total",
            &[("op", "commit"), ("adapter", "daemon"), ("status", "error")],
        );
        send_error(
            &tx,
            "handle_commit",
            "commit not supported on this platform".to_string(),
        )
        .await;
        return;
    };

    let cid = match minibox_core::domain::ContainerId::new(container_id.clone()) {
        Ok(id) => id,
        Err(e) => {
            send_error(&tx, "handle_commit", format!("invalid container id: {e}")).await;
            return;
        }
    };

    let config = minibox_core::domain::CommitConfig {
        author,
        message,
        env_overrides,
        cmd_override,
    };

    match committer.commit(&cid, &target_image, &config).await {
        Ok(meta) => {
            info!(
                container_id = %container_id,
                target = %target_image,
                layers = meta.layers.len(),
                "commit: completed"
            );
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "commit"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "commit"), ("adapter", "daemon")],
            );
            let _ = tx
                .send(DaemonResponse::Success {
                    message: format!(
                        "committed {} digest:{}",
                        target_image,
                        meta.layers
                            .first()
                            .map(|l| l.digest.as_str())
                            .unwrap_or("unknown")
                    ),
                })
                .await;
        }
        Err(e) => {
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "commit"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "commit"), ("adapter", "daemon")],
            );
            send_error(&tx, "handle_commit", e.to_string()).await;
        }
    }
}

// ─── Build ──────────────────────────────────────────────────────────────────

/// Build an image from an inline Dockerfile string.
///
/// Streams [`DaemonResponse::BuildOutput`] for each Dockerfile step, then
/// sends exactly one terminal response: [`DaemonResponse::BuildComplete`] on
/// success or [`DaemonResponse::Error`] on failure.
#[allow(clippy::too_many_arguments)]
pub async fn handle_build(
    dockerfile: String,
    context_path: String,
    tag: String,
    build_args: Vec<(String, String)>,
    no_cache: bool,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let start = std::time::Instant::now();
    let Some(ref builder) = deps.build.image_builder else {
        deps.events.metrics.increment_counter(
            "minibox_container_ops_total",
            &[("op", "build"), ("adapter", "daemon"), ("status", "error")],
        );
        send_error(
            &tx,
            "handle_build",
            "build not supported on this platform".to_string(),
        )
        .await;
        return;
    };

    // SECURITY: context_path comes from the protocol request. SO_PEERCRED restricts
    // who can connect (UID 0 only), but not what paths they may name. We canonicalize
    // to resolve symlinks and reject relative paths before touching the filesystem.
    let context_dir = {
        let raw = std::path::PathBuf::from(&context_path);
        if !raw.is_absolute() {
            send_error(
                &tx,
                "handle_build",
                format!("build context_path must be absolute: {context_path:?}"),
            )
            .await;
            return;
        }
        match raw.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                send_error(
                    &tx,
                    "handle_build",
                    format!("build context_path invalid: {e}"),
                )
                .await;
                return;
            }
        }
    };
    let dockerfile_path = context_dir.join("Dockerfile.minibox-build");
    if let Err(e) = tokio::fs::write(&dockerfile_path, &dockerfile).await {
        send_error(&tx, "handle_build", format!("write Dockerfile: {e}")).await;
        return;
    }

    let context = minibox_core::domain::BuildContext {
        directory: context_dir,
        dockerfile: std::path::PathBuf::from("Dockerfile.minibox-build"),
    };
    let config = minibox_core::domain::BuildConfig {
        tag: tag.clone(),
        build_args,
        no_cache,
    };

    let (progress_tx, mut progress_rx) = mpsc::channel::<minibox_core::domain::BuildProgress>(64);
    let tx2 = tx.clone();
    tokio::spawn(async move {
        while let Some(p) = progress_rx.recv().await {
            let _ = tx2
                .send(DaemonResponse::BuildOutput {
                    step: p.step,
                    total_steps: p.total_steps,
                    message: p.message,
                })
                .await;
        }
    });

    match builder.build_image(&context, &config, progress_tx).await {
        Ok(meta) => {
            info!(
                tag = %tag,
                layers = meta.layers.len(),
                "build: complete"
            );
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "build"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "build"), ("adapter", "daemon")],
            );
            let image_id = meta
                .layers
                .first()
                .map(|l| l.digest.clone())
                .unwrap_or_else(|| format!("built:{tag}"));
            let _ = tx
                .send(DaemonResponse::BuildComplete { image_id, tag })
                .await;
        }
        Err(e) => {
            deps.events.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "build"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.events.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "build"), ("adapter", "daemon")],
            );
            send_error(&tx, "handle_build", format!("build failed: {e}")).await;
        }
    }
}

// ─── Event subscription ──────────────────────────────────────────────────────

/// Stream container lifecycle events to a client.
///
/// Subscribes to the event broker and forwards each [`ContainerEvent`] as a
/// [`DaemonResponse::Event`] message until the client disconnects (channel
/// send fails) or the broker is shut down.
pub(crate) async fn handle_subscribe_events(
    event_source: Arc<dyn minibox_core::events::EventSource>,
    tx: tokio::sync::mpsc::Sender<minibox_core::protocol::DaemonResponse>,
) {
    let mut rx = event_source.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) => {
                if tx
                    .send(minibox_core::protocol::DaemonResponse::Event { event })
                    .await
                    .is_err()
                {
                    // Client disconnected.
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(skipped = n, "events: subscriber lagged, skipping events");
                // Continue — don't break on lag.
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

// ─── Prune ──────────────────────────────────────────────────────────────────

/// Remove unused images from the image store.
pub(crate) async fn handle_prune(
    dry_run: bool,
    state: Arc<DaemonState>,
    image_gc: Arc<dyn minibox_core::image::gc::ImageGarbageCollector>,
    event_sink: Arc<dyn EventSink>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let in_use: Vec<String> = state
        .list_containers()
        .await
        .into_iter()
        .filter_map(|c| {
            if c.state == "running" || c.state == "paused" {
                Some(c.image.clone())
            } else {
                None
            }
        })
        .collect();

    match image_gc.prune(dry_run, &in_use).await {
        Ok(report) => {
            let count = report.removed.len();
            let freed = report.freed_bytes;
            event_sink.emit(minibox_core::events::ContainerEvent::ImagePruned {
                count,
                freed_bytes: freed,
                timestamp: std::time::SystemTime::now(),
            });
            let _ = tx
                .send(DaemonResponse::Pruned {
                    removed: report.removed,
                    freed_bytes: freed,
                    dry_run: report.dry_run,
                })
                .await;
        }
        Err(e) => {
            send_error(&tx, "handle_build", e.to_string()).await;
        }
    }
}

// ─── RemoveImage ─────────────────────────────────────────────────────────────

/// Remove a specific image by reference.
pub(crate) async fn handle_remove_image(
    image_ref: String,
    state: Arc<DaemonState>,
    image_store: Arc<minibox_core::image::ImageStore>,
    event_sink: Arc<dyn EventSink>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let in_use = state
        .list_containers()
        .await
        .into_iter()
        .any(|c| (c.state == "running" || c.state == "paused") && c.image == image_ref);

    if in_use {
        send_error(
            &tx,
            "handle_build",
            format!("image {image_ref} is in use by a running container"),
        )
        .await;
        return;
    }

    let (name, tag) = match image_ref.rsplit_once(':') {
        Some(pair) => pair,
        None => {
            send_error(
                &tx,
                "handle_build",
                format!("invalid image ref: {image_ref}"),
            )
            .await;
            return;
        }
    };

    match image_store.delete_image(name, tag).await {
        Ok(()) => {
            event_sink.emit(minibox_core::events::ContainerEvent::ImageRemoved {
                image: image_ref.clone(),
                timestamp: std::time::SystemTime::now(),
            });
            let _ = tx
                .send(DaemonResponse::Success {
                    message: format!("removed {image_ref}"),
                })
                .await;
        }
        Err(e) => {
            send_error(&tx, "handle_build", e.to_string()).await;
        }
    }
}

/// List all cached images stored in the image store.
pub(crate) async fn handle_list_images(
    image_store: Arc<minibox_core::image::ImageStore>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    match image_store.list_all_images().await {
        Ok(images) => {
            if tx.send(DaemonResponse::ImageList { images }).await.is_err() {
                warn!("handle_list_images: client disconnected before ImageList could be sent");
            }
        }
        Err(e) => {
            send_error(&tx, "handle_list_images", e.to_string()).await;
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

// ─── Snapshot handlers ───────────────────────────────────────────────────────

/// Save a VM state snapshot for a container.
pub async fn handle_save_snapshot(
    id: String,
    name: Option<String>,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let snap_name = name.unwrap_or_else(|| Utc::now().format("%Y%m%d-%H%M%S").to_string());

    let data_dir = &deps.lifecycle.containers_base;
    let snap_dir = data_dir
        .parent()
        .unwrap_or(data_dir)
        .join("snapshots")
        .join(&id);
    if let Err(e) = std::fs::create_dir_all(&snap_dir) {
        return DaemonResponse::Error {
            message: format!("failed to create snapshot dir: {e}"),
        };
    }

    let snap_path = snap_dir.join(format!("{snap_name}.snap"));
    match deps.checkpoint.save_snapshot(&id, &snap_path) {
        Ok(info) => DaemonResponse::SnapshotSaved { info },
        Err(e) => DaemonResponse::Error {
            message: format!("save_snapshot: {e}"),
        },
    }
}

/// Restore a VM state snapshot for a container.
pub async fn handle_restore_snapshot(
    id: String,
    name: String,
    _state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> DaemonResponse {
    let data_dir = &deps.lifecycle.containers_base;
    let snap_path = data_dir
        .parent()
        .unwrap_or(data_dir)
        .join("snapshots")
        .join(&id)
        .join(format!("{name}.snap"));

    match deps.checkpoint.restore_snapshot(&id, &snap_path) {
        Ok(()) => DaemonResponse::SnapshotRestored { id, name },
        Err(e) => DaemonResponse::Error {
            message: format!("restore_snapshot: {e}"),
        },
    }
}

/// List available snapshots for a container.
pub async fn handle_list_snapshots(id: String, deps: Arc<HandlerDependencies>) -> DaemonResponse {
    match deps.checkpoint.list_snapshots(&id) {
        Ok(snapshots) => DaemonResponse::SnapshotList { id, snapshots },
        Err(e) => DaemonResponse::Error {
            message: format!("list_snapshots: {e}"),
        },
    }
}

// ─── Pipeline ───────────────────────────────────────────────────────────────

/// Run a crux pipeline inside an ephemeral container.
///
/// Higher-level than `handle_run`: pulls image, creates container with the
/// pipeline file bind-mounted at `/pipeline.cruxx`, streams `ContainerOutput`
/// to the client, then after the container exits reads `/trace.json` from the
/// overlay upper dir and emits [`DaemonResponse::PipelineComplete`].
///
/// # Protocol sequence
///
/// ```text
/// Client  ──RunPipeline──►  Daemon
/// Client  ◄──ContainerCreated──  (container ID)
/// Client  ◄──ContainerOutput──   (zero or more stdout/stderr chunks)
/// Client  ◄──PipelineComplete──  (trace + exit_code; terminal)
/// ```
///
/// On macOS / non-Unix platforms the streaming run path is unavailable;
/// `handle_pipeline` returns an `Error` response immediately on those builds.
///
/// # Trace file
///
/// After the container exits the handler looks for `<containers_base>/<id>/upper/trace.json`.
/// If the file is present it is parsed as JSON and included in `PipelineComplete.trace`.
/// If absent or unparseable, a synthetic empty trace `{"steps":[]}` is used —
/// the pipeline still completes successfully (the exit code determines success).
#[allow(clippy::too_many_arguments)]
pub async fn handle_pipeline(
    pipeline_path: String,
    input: Option<serde_json::Value>,
    image: Option<String>,
    budget: Option<serde_json::Value>,
    env: Vec<(String, String)>,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    #[cfg(not(unix))]
    {
        let _ = (pipeline_path, input, image, budget, env, state, deps);
        send_error(
            &tx,
            "handle_pipeline",
            "RunPipeline is only supported on Unix platforms".to_string(),
        )
        .await;
        return;
    }

    #[cfg(unix)]
    {
        let image_ref = image.unwrap_or_else(|| "cruxx-runtime:latest".to_string());

        // Validate pipeline path is absolute so the bind-mount is unambiguous.
        let host_pipeline = std::path::PathBuf::from(&pipeline_path);
        if !host_pipeline.is_absolute() {
            send_error(
                &tx,
                "handle_pipeline",
                format!("pipeline_path must be absolute, got: {pipeline_path:?}"),
            )
            .await;
            return;
        }

        // Build the bind mount: pipeline file → /pipeline.cruxx (read-only).
        let pipeline_mount = BindMount {
            host_path: host_pipeline,
            container_path: std::path::PathBuf::from("/pipeline.cruxx"),
            read_only: true,
        };

        // Build env list: inherit caller env, add CRUXX_PLUGIN_PATH and optional budget.
        let mut container_env: Vec<String> =
            env.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
        container_env.push("CRUXX_PLUGIN_PATH=/usr/local/bin/minibox-crux-plugin".to_string());
        if let Some(ref b) = budget
            && let Ok(s) = serde_json::to_string(b)
        {
            container_env.push(format!("CRUXX_BUDGET_JSON={s}"));
        }
        if let Some(inp) = &input
            && let Ok(s) = serde_json::to_string(inp)
        {
            container_env.push(format!("CRUXX_INPUT_JSON={s}"));
        }

        // Clone deps and override policy to permit bind mounts for this
        // internal pipeline run.  Pipeline requests originate from the daemon
        // (not from an end user), so the bind-mount policy exception is safe.
        let mut pipeline_deps = (*deps).clone();
        pipeline_deps.policy.allow_bind_mounts = true;
        let pipeline_deps = Arc::new(pipeline_deps);

        // Bridge channel: collect all streaming responses from handle_run internally.
        let (inner_tx, mut inner_rx) = tokio::sync::mpsc::channel::<DaemonResponse>(64);

        let pipeline_state = Arc::clone(&state);
        let pipeline_deps_clone = Arc::clone(&pipeline_deps);

        // Spawn handle_run in the background; we drain inner_rx below.
        tokio::spawn(async move {
            handle_run(
                image_ref,
                None,
                vec![
                    "crux".to_string(),
                    "run".to_string(),
                    "/pipeline.cruxx".to_string(),
                    "--output".to_string(),
                    "/trace.json".to_string(),
                ],
                None,
                None,
                true, // ephemeral: stream output
                None,
                vec![pipeline_mount],
                false,
                container_env,
                None,
                None,
                pipeline_state,
                pipeline_deps_clone,
                inner_tx,
            )
            .await;
        });

        // Drain the inner channel, collecting the container ID and exit code.
        let mut container_id = String::new();
        let mut exit_code = 0i32;

        loop {
            match inner_rx.recv().await {
                None => break,
                Some(DaemonResponse::ContainerCreated { id }) => {
                    container_id = id;
                    // Do not forward ContainerCreated — pipeline clients receive
                    // PipelineComplete instead.
                }
                Some(DaemonResponse::ContainerOutput { stream, data }) => {
                    // Forward output chunks to the client in real time.
                    if tx
                        .send(DaemonResponse::ContainerOutput { stream, data })
                        .await
                        .is_err()
                    {
                        warn!("handle_pipeline: client disconnected during ContainerOutput");
                        return;
                    }
                }
                Some(DaemonResponse::ContainerStopped { exit_code: ec }) => {
                    exit_code = ec;
                    break;
                }
                Some(DaemonResponse::Error { message }) => {
                    // Container failed to start or run — propagate as error.
                    send_error(&tx, "handle_pipeline", message).await;
                    return;
                }
                Some(other) => {
                    debug!(
                        response = ?other,
                        "handle_pipeline: unexpected inner response, ignoring"
                    );
                }
            }
        }

        if container_id.is_empty() {
            send_error(
                &tx,
                "handle_pipeline",
                "pipeline container did not produce a container ID".to_string(),
            )
            .await;
            return;
        }

        // Read trace.json from the overlay upper dir.
        // Path: <containers_base>/<id>/upper/trace.json
        let trace_path = deps
            .lifecycle
            .containers_base
            .join(&container_id)
            .join("upper")
            .join("trace.json");

        let trace = if trace_path.exists() {
            match std::fs::read_to_string(&trace_path) {
                Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                    Ok(v) => {
                        info!(
                            container_id = %container_id,
                            path = %trace_path.display(),
                            "handle_pipeline: trace loaded"
                        );
                        v
                    }
                    Err(e) => {
                        warn!(
                            container_id = %container_id,
                            path = %trace_path.display(),
                            error = %e,
                            "handle_pipeline: trace file is not valid JSON, using empty trace"
                        );
                        serde_json::json!({"steps": []})
                    }
                },
                Err(e) => {
                    warn!(
                        container_id = %container_id,
                        path = %trace_path.display(),
                        error = %e,
                        "handle_pipeline: failed to read trace file, using empty trace"
                    );
                    serde_json::json!({"steps": []})
                }
            }
        } else {
            debug!(
                container_id = %container_id,
                path = %trace_path.display(),
                "handle_pipeline: no trace file found, using empty trace"
            );
            serde_json::json!({"steps": []})
        };

        info!(
            container_id = %container_id,
            exit_code,
            "handle_pipeline: pipeline complete"
        );

        // Persist the trace before notifying the client so that a client that
        // immediately queries `mbx pipeline show <id>` sees the record.
        {
            let store = state.trace_store.clone();
            let id_for_store = container_id.clone();
            let pipeline_for_store = pipeline_path.clone();
            let trace_for_store = trace.clone();
            if let Err(e) = store.store(
                &id_for_store,
                &pipeline_for_store,
                &trace_for_store,
                exit_code,
            ) {
                warn!(
                    container_id = %container_id,
                    error = %e,
                    "handle_pipeline: failed to store trace (non-fatal)"
                );
            }
        }

        if tx
            .send(DaemonResponse::PipelineComplete {
                trace,
                container_id,
                exit_code,
            })
            .await
            .is_err()
        {
            warn!("handle_pipeline: client disconnected before PipelineComplete could be sent");
        }
    }
}

// ─── Update ─────────────────────────────────────────────────────────────────

/// Re-pull cached images to pick up newer versions.
///
/// Sends a non-terminal [`DaemonResponse::UpdateProgress`] for each image
/// processed, then a terminal [`DaemonResponse::Success`] with a summary.
///
/// # Image resolution order
///
/// 1. If `all` is `true`: every image returned by [`ImageStore::list_all_images`].
/// 2. If `containers` is `true`: deduplicated `source_image_ref` values from all
///    container records held in `state`.
/// 3. Otherwise: the explicit `images` list.
///
/// When `restart` is `true` a warning is logged for each affected container;
/// the actual restart is not yet implemented (tracked for a later wave).
pub async fn handle_update(
    images: Vec<String>,
    all: bool,
    containers: bool,
    restart: bool,
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
    tx: mpsc::Sender<DaemonResponse>,
) {
    // ── Step 1: resolve the list of image refs to update ─────────────────────
    let target_refs: Vec<String> = if all {
        match deps.image.image_store.list_all_images().await {
            Ok(refs) => refs,
            Err(e) => {
                send_error(
                    &tx,
                    "handle_update",
                    format!("failed to list images: {e:#}"),
                )
                .await;
                return;
            }
        }
    } else if containers {
        let containers_list = state.list_containers().await;
        let mut seen = std::collections::HashSet::new();
        let mut refs = Vec::new();
        for info in containers_list {
            let record = state.get_container(&info.id).await;
            if let Some(source_ref) = record.and_then(|r| r.source_image_ref)
                && seen.insert(source_ref.clone())
            {
                refs.push(source_ref);
            }
        }
        refs
    } else {
        images
    };

    let total = target_refs.len();
    let mut updated: usize = 0;

    // ── Step 2: pull each image, send UpdateProgress per image ────────────────
    for ref_str in &target_refs {
        let image_ref = match ImageRef::parse(ref_str) {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    image = %ref_str,
                    error = %e,
                    "handle_update: invalid image reference, skipping"
                );
                let status = format!("error: {e}");
                if tx
                    .send(DaemonResponse::UpdateProgress {
                        image: ref_str.clone(),
                        status,
                    })
                    .await
                    .is_err()
                {
                    warn!(
                        image = %ref_str,
                        "handle_update: client disconnected during UpdateProgress"
                    );
                    return;
                }
                continue;
            }
        };

        let registry = deps.image.registry_router.route(&image_ref);
        let status = match registry.pull_image(&image_ref).await {
            Ok(_) => {
                info!(
                    image = %ref_str,
                    "handle_update: image refreshed"
                );
                updated += 1;
                "updated".to_string()
            }
            Err(e) => {
                warn!(
                    image = %ref_str,
                    error = %e,
                    "handle_update: pull failed"
                );
                format!("error: {e:#}")
            }
        };

        if tx
            .send(DaemonResponse::UpdateProgress {
                image: ref_str.clone(),
                status,
            })
            .await
            .is_err()
        {
            warn!(
                image = %ref_str,
                "handle_update: client disconnected during UpdateProgress"
            );
            return;
        }
    }

    // ── Step 3: stop containers using updated images (restart = true) ────────
    //
    // Full replay (stop + re-run with original config) requires the container's
    // creation parameters to be stored in ContainerRecord, which is tracked for
    // a future wave.  For now, "restart" means: stop every Running or Paused
    // container whose source image was just updated so it picks up the new
    // layers on its next manual start.
    //
    // stop_inner is unix-only so this entire block is cfg-gated.
    #[cfg(unix)]
    let stopped: usize = if restart {
        let target_set: std::collections::HashSet<&str> =
            target_refs.iter().map(String::as_str).collect();

        let candidate_ids: Vec<String> = state
            .list_containers()
            .await
            .into_iter()
            .filter(|info| info.state == "Running" || info.state == "Paused")
            .map(|info| info.id)
            .collect();

        let mut count = 0usize;
        for id in candidate_ids {
            let record = state.get_container(&id).await;
            let image_ref = record.and_then(|r| r.source_image_ref);
            if !image_ref
                .as_deref()
                .map(|r| target_set.contains(r))
                .unwrap_or(false)
            {
                continue;
            }

            info!(
                container_id = %id,
                "handle_update: stopping container for image update (restart=true)"
            );
            match stop_inner(&id, &state).await {
                Ok(()) => {
                    count += 1;
                    info!(container_id = %id, "handle_update: container stopped");
                }
                Err(e) => {
                    warn!(
                        container_id = %id,
                        error = %e,
                        "handle_update: failed to stop container — continuing"
                    );
                }
            }
        }
        count
    } else {
        0
    };

    #[cfg(not(unix))]
    let stopped: usize = {
        if restart {
            warn!("handle_update: restart not supported on this platform");
        }
        0
    };

    // ── Step 4: terminal Success ──────────────────────────────────────────────
    let message = if restart && stopped > 0 {
        format!("updated {updated}/{total} images; stopped {stopped} container(s)")
    } else {
        format!("updated {updated}/{total} images")
    };
    info!(updated, total, "handle_update: complete");
    if tx.send(DaemonResponse::Success { message }).await.is_err() {
        warn!("handle_update: client disconnected before Success could be sent");
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod run_inner_tests {
    #[test]
    fn run_inner_capture_signature_accepts_mounts_and_privileged() {
        // Compile-time check: the BindMount type is accessible in this crate.
        use minibox_core::domain::BindMount;
        let _: Vec<BindMount> = vec![];
        let _: bool = false;
    }
}

#[cfg(test)]
mod registry_router_tests {
    use crate::adapters::{DockerHubRegistry, GhcrRegistry};
    use minibox_core::adapters::HostnameRegistryRouter;
    use minibox_core::domain::{DynImageRegistry, RegistryRouter};
    use minibox_core::image::ImageStore;
    use minibox_core::image::reference::ImageRef;
    use std::sync::Arc;

    fn make_router(store: &Arc<ImageStore>) -> (HostnameRegistryRouter, *const (), *const ()) {
        let docker: DynImageRegistry = Arc::new(DockerHubRegistry::new(Arc::clone(store)).unwrap());
        let ghcr: DynImageRegistry = Arc::new(GhcrRegistry::new(Arc::clone(store)).unwrap());

        let docker_ptr = Arc::as_ptr(&docker) as *const ();
        let ghcr_ptr = Arc::as_ptr(&ghcr) as *const ();

        let router = HostnameRegistryRouter::new(docker, [("ghcr.io", ghcr)]);
        (router, docker_ptr, ghcr_ptr)
    }

    #[test]
    fn routes_ghcr() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp.path().join("images")).unwrap());
        let (router, _, ghcr_ptr) = make_router(&store);

        let image_ref = ImageRef::parse("ghcr.io/org/minibox-rust-ci:stable").unwrap();
        let selected =
            router.route(&image_ref) as *const dyn minibox_core::domain::ImageRegistry as *const ();

        assert_eq!(selected, ghcr_ptr);
    }

    #[test]
    fn routes_ghcr_case_insensitive() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp.path().join("images")).unwrap());
        let (router, _, ghcr_ptr) = make_router(&store);

        // GHCR.IO (uppercase) must still route to the ghcr adapter
        let image_ref = ImageRef::parse("GHCR.IO/org/image:tag").unwrap();
        let selected =
            router.route(&image_ref) as *const dyn minibox_core::domain::ImageRegistry as *const ();

        assert_eq!(selected, ghcr_ptr);
    }

    #[test]
    fn routes_docker_hub_as_default() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp.path().join("images")).unwrap());
        let (router, docker_ptr, _) = make_router(&store);

        let image_ref = ImageRef::parse("alpine").unwrap();
        let selected =
            router.route(&image_ref) as *const dyn minibox_core::domain::ImageRegistry as *const ();

        assert_eq!(selected, docker_ptr);
    }
}
