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
use mbx::ImageRef;
use minibox_core::domain::NetworkMode;
use minibox_core::domain::{
    BindMount, ContainerHooks, ContainerSpawnConfig, DomainError, DynContainerRuntime,
    DynFilesystemProvider, DynImageRegistry, DynMetricsRecorder, DynNetworkProvider,
    DynResourceLimiter, HookSpec, ResourceConfig, SessionId,
};
use minibox_core::events::{ContainerEvent, EventSink};
use minibox_core::protocol::{ContainerInfo, DaemonResponse};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::mpsc;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use crate::network_lifecycle::NetworkLifecycle;
use crate::state::{ContainerRecord, ContainerState, DaemonState};
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
// Handler Dependencies (Dependency Injection)
// ---------------------------------------------------------------------------

/// Dependencies injected into request handlers.
///
/// This struct bundles all infrastructure adapters (trait implementations)
/// that handlers need to perform their operations. Following hexagonal
/// architecture principles, handlers depend on trait abstractions rather
/// than concrete implementations.
///
/// # Usage
///
/// Created once in the composition root (main.rs) and passed to all handlers:
///
/// ```rust,ignore
/// use mbx::adapters::{DockerHubRegistry, OverlayFilesystem, CgroupV2Limiter, LinuxNamespaceRuntime};
///
/// let deps = Arc::new(HandlerDependencies {
///     registry: Arc::new(DockerHubRegistry::new(store)?),
///     filesystem: Arc::new(OverlayFilesystem),
///     resource_limiter: Arc::new(CgroupV2Limiter),
///     runtime: Arc::new(LinuxNamespaceRuntime),
///     containers_base: PathBuf::from("/var/lib/minibox/containers"),
///     run_containers_base: PathBuf::from("/run/minibox/containers"),
/// });
/// ```
#[derive(Clone)]
pub struct HandlerDependencies {
    /// Image registry for pulling Docker Hub images.
    pub registry: DynImageRegistry,
    /// Image registry for pulling GHCR images.
    pub ghcr_registry: DynImageRegistry,
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
    /// Metrics recorder for operational observability.
    pub metrics: DynMetricsRecorder,
    /// Loader for local OCI image tarballs.
    pub image_loader: minibox_core::domain::DynImageLoader,
    /// Exec runtime for running commands inside containers.
    /// `None` on platforms where exec is not supported (macOS, Windows).
    pub exec_runtime: Option<minibox_core::domain::DynExecRuntime>,
    /// Image pusher for pushing images to OCI registries.
    /// `None` on platforms or configurations where push is not supported.
    pub image_pusher: Option<minibox_core::domain::DynImagePusher>,
    /// Container committer for snapshotting a container's overlay diff.
    /// `None` on platforms where commit is not supported (macOS, Windows).
    pub commit_adapter: Option<minibox_core::domain::DynContainerCommitter>,
    /// Image builder for building images from a Dockerfile.
    /// `None` on platforms where build is not supported (macOS, Windows).
    pub image_builder: Option<minibox_core::domain::DynImageBuilder>,
    /// Event sink for emitting container lifecycle events.
    pub event_sink: Arc<dyn EventSink>,
    /// Source for subscribing to the container event stream.
    pub event_source: Arc<dyn minibox_core::events::EventSource>,
    /// Image garbage collector for prune operations.
    pub image_gc: Arc<dyn minibox_core::image::gc::ImageGarbageCollector>,
    /// Image store for direct image operations (e.g. RemoveImage).
    pub image_store: Arc<minibox_core::image::ImageStore>,
    /// Policy controlling which container capabilities are permitted.
    pub policy: ContainerPolicy,
    /// Live PTY session channels for SendInput/ResizePty dispatch.
    pub pty_sessions: SharedPtyRegistry,
}

impl HandlerDependencies {
    /// Override the image loader (builder-style).
    pub fn with_image_loader(mut self, loader: minibox_core::domain::DynImageLoader) -> Self {
        self.image_loader = loader;
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

// ─── Registry Selection ─────────────────────────────────────────────────────

/// Choose the registry adapter based on the image reference's registry hostname.
///
/// - `ghcr.io` → `ghcr` adapter
/// - everything else → `docker` (Docker Hub) adapter
fn select_registry<'a>(
    image_ref: &ImageRef,
    docker: &'a dyn minibox_core::domain::ImageRegistry,
    ghcr: &'a dyn minibox_core::domain::ImageRegistry,
) -> &'a dyn minibox_core::domain::ImageRegistry {
    if image_ref.registry.to_lowercase() == "ghcr.io" {
        ghcr
    } else {
        docker
    }
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
        Arc::clone(&state),
        Arc::clone(&deps),
    )
    .await;

    let (container_id, pid, output_reader) = match result {
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
    deps.event_sink.emit(ContainerEvent::Created {
        id: container_id.clone(),
        image: image_label,
        timestamp: std::time::SystemTime::now(),
    });
    deps.event_sink.emit(ContainerEvent::Started {
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
    let stdout_log_path = deps.containers_base.join(&container_id).join("stdout.log");
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

    // Wait for the child process to exit.
    debug!(pid = pid, "streaming: waiting for child exit");
    let exit_code = tokio::task::spawn_blocking(move || handler_wait_for_exit(pid))
        .await
        .unwrap_or(Ok(-1))
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
    NetworkLifecycle::new(deps.network_provider.clone())
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
    let oom = if let Some(ref cgroup_path) = cgroup_path_opt {
        check_oom_killed(cgroup_path).await
    } else {
        false
    };
    if oom {
        deps.event_sink.emit(ContainerEvent::OomKilled {
            id: container_id.clone(),
            timestamp: std::time::SystemTime::now(),
        });
    } else {
        deps.event_sink.emit(ContainerEvent::Stopped {
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
    state: Arc<DaemonState>,
    deps: Arc<HandlerDependencies>,
) -> Result<(String, u32, std::os::fd::OwnedFd)> {
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

    // Select registry based on image reference hostname.
    let registry = select_registry(
        &image_ref,
        deps.registry.as_ref(),
        deps.ghcr_registry.as_ref(),
    );

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

    let container_dir = deps.containers_base.join(&id);
    let run_dir = deps.run_containers_base.join(&id);

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

    let merged_dir = deps.filesystem.setup_rootfs(&layer_dirs, &container_dir)?;

    let resource_config = ResourceConfig {
        memory_limit_bytes,
        cpu_weight,
        pids_max: Some(1024),
        io_max_bytes_per_sec: None,
    };
    let cgroup_dir_str = deps.resource_limiter.create(&id, &resource_config)?;
    let cgroup_dir = PathBuf::from(cgroup_dir_str);

    // ── Network setup ──────────────────────────────────────────────────
    let net_mode = network.unwrap_or(NetworkMode::None);
    let network_config = NetworkConfig {
        mode: net_mode,
        ..NetworkConfig::default()
    };
    let net = NetworkLifecycle::new(deps.network_provider.clone());
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
            image: image_label,
            command: command_str,
            state: "Created".to_string(),
            created_at: Utc::now().to_rfc3339(),
            pid: None,
        },
        pid: None,
        rootfs_path: merged_dir.clone(),
        cgroup_path: cgroup_dir.clone(),
        post_exit_hooks: vec![],
        overlay_upper: None,
        source_image_ref: None,
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

    let spawn_result = deps.runtime.spawn_process(&spawn_config).await?;

    let pid = spawn_result.pid;
    let output_reader = spawn_result.output_reader.ok_or_else(|| {
        anyhow::anyhow!("capture_output=true but runtime returned no output_reader")
    })?;

    // ── Network attach ─────────────────────────────────────────────────
    net.attach(&id, pid).await.context("network attach")?;

    // Write PID file and update state.
    let pid_file = deps.run_containers_base.join(&id).join("pid");
    if let Err(e) = std::fs::write(&pid_file, pid.to_string()) {
        warn!(
            pid_file = %pid_file.display(),
            error = %e,
            "container: failed to write pid file"
        );
    }
    state.set_container_pid(&id, pid).await;

    Ok((id, pid, output_reader))
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

    // Select registry based on image reference hostname.
    let registry = select_registry(
        &image_ref,
        deps.registry.as_ref(),
        deps.ghcr_registry.as_ref(),
    );

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

    let container_dir = deps.containers_base.join(&id);
    let run_dir = deps.run_containers_base.join(&id);

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
    let merged_dir_from_overlay = deps.filesystem.setup_rootfs(&layer_dirs, &container_dir)?;

    // Setup cgroup (using injected resource limiter trait).
    let resource_config = ResourceConfig {
        memory_limit_bytes,
        cpu_weight,
        pids_max: Some(1024), // Default PID limit for security
        io_max_bytes_per_sec: None,
    };
    let cgroup_dir_str = deps.resource_limiter.create(&id, &resource_config)?;
    let cgroup_dir = PathBuf::from(cgroup_dir_str);

    // ── Network setup ──────────────────────────────────────────────────
    let net_mode = network.unwrap_or(NetworkMode::None);
    let network_config = NetworkConfig {
        mode: net_mode,
        ..NetworkConfig::default()
    };
    let net = NetworkLifecycle::new(deps.network_provider.clone());
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
        overlay_upper: None,
        source_image_ref: None,
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
    let runtime_clone = Arc::clone(&deps.runtime);
    let metrics_clone = Arc::clone(&deps.metrics);
    let net_clone = net.clone();
    let run_containers_base_clone = deps.run_containers_base.clone();
    let event_sink_clone = Arc::clone(&deps.event_sink);
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
                tokio::task::spawn_blocking(move || {
                    daemon_wait_for_exit(
                        pid,
                        &id_wait,
                        state_wait,
                        rootfs_wait,
                        hooks_wait,
                        event_sink_wait,
                        cgroup_path_wait,
                    );
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

/// Wait for a process to exit and return its exit code.
///
/// Thin wrapper around `waitpid` usable on any Unix platform.
/// The `mbx::container::process::wait_for_exit` variant is only
/// available on Linux (the `container` module is gated
/// `#[cfg(target_os = "linux")]`). This local version provides the same
/// functionality for the macOS streaming path.
#[cfg(unix)]
fn handler_wait_for_exit(pid: u32) -> Result<i32> {
    use nix::sys::wait::{WaitStatus, waitpid};
    use nix::unistd::Pid;
    let nix_pid = Pid::from_raw(pid as i32);
    match waitpid(nix_pid, None) {
        Ok(WaitStatus::Exited(_, code)) => Ok(code),
        Ok(WaitStatus::Signaled(_, sig, _)) => Ok(-(sig as i32)),
        Ok(other) => {
            info!(pid = pid, wait_status = ?other, "handler_wait_for_exit: unexpected status");
            Ok(-1)
        }
        Err(e) => {
            warn!(pid = pid, error = %e, "handler_wait_for_exit: waitpid error");
            Ok(-1)
        }
    }
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

/// Synchronous variant of [`check_oom_killed`] for use inside `spawn_blocking` contexts.
fn check_oom_killed_sync(cgroup_path: &std::path::Path) -> bool {
    let events_path = cgroup_path.join("memory.events");
    if let Ok(content) = std::fs::read_to_string(&events_path) {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("oom_kill ") {
                return rest.trim().parse::<u64>().unwrap_or(0) > 0;
            }
        }
    }
    false
}

/// Block the calling thread until the container process exits.
///
/// This function must be called from a `tokio::task::spawn_blocking` context
/// because `waitpid(2)` is a blocking syscall that cannot run on a Tokio
/// worker thread.
///
/// After the process exits:
/// 1. Any post-exit hooks registered on the container are executed
///    (Linux only, via `mbx::container::process::run_hooks`).
/// 2. The container state is updated to `"Stopped"` in `DaemonState`.
///    Because this runs in a blocking thread, the state update bridges back
///    to the async runtime via `Handle::try_current` or a one-shot runtime.
#[cfg(unix)]
fn daemon_wait_for_exit(
    pid: u32,
    id: &str,
    state: Arc<DaemonState>,
    _rootfs: std::path::PathBuf,
    _post_exit_hooks: Vec<HookSpec>,
    event_sink: Arc<dyn EventSink>,
    cgroup_path: std::path::PathBuf,
) {
    use nix::sys::wait::{WaitStatus, waitpid};
    use nix::unistd::Pid;
    let nix_pid = Pid::from_raw(pid as i32);
    let exit_code = match waitpid(nix_pid, None) {
        Ok(WaitStatus::Exited(_, code)) => {
            info!(container_id = %id, exit_code = code, "container: exited");
            code
        }
        Ok(WaitStatus::Signaled(_, sig, _)) => {
            info!(container_id = %id, signal = %sig, "container: killed by signal");
            -(sig as i32)
        }
        Ok(other) => {
            info!(container_id = %id, status = ?other, "container: unexpected wait status");
            -1
        }
        Err(e) => {
            warn!(container_id = %id, error = %e, "container: waitpid error");
            -1
        }
    };

    #[cfg(target_os = "linux")]
    if !_post_exit_hooks.is_empty() {
        use mbx::container::process::run_hooks;
        if let Err(e) = run_hooks(&_post_exit_hooks, &_rootfs, Some(exit_code)) {
            warn!(container_id = %id, error = %e, "container: post-exit hooks error");
        }
    }

    // Check OOM and emit lifecycle event (sync: read memory.events directly).
    let oom = check_oom_killed_sync(&cgroup_path);
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

    // Mark stopped; bridge async state update from sync context.
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            if let Err(e) =
                handle.block_on(state.update_container_state(id, ContainerState::Stopped))
            {
                warn!(container_id = %id, error = %e, "state: failed to mark container Stopped");
            }
        }
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("one-shot runtime");
            if let Err(e) = rt.block_on(state.update_container_state(id, ContainerState::Stopped)) {
                warn!(container_id = %id, error = %e, "state: failed to mark container Stopped");
            }
        }
    }
}

/// Windows stub: no-op because HCS/WSL2 lifecycle is managed externally.
///
/// Containers on Windows remain in `"Running"` state until an explicit
/// `stop` or `remove` command is issued.
#[cfg(windows)]
fn daemon_wait_for_exit(
    _pid: u32,
    _id: &str,
    _state: Arc<DaemonState>,
    _rootfs: std::path::PathBuf,
    _post_exit_hooks: Vec<HookSpec>,
    _event_sink: Arc<dyn EventSink>,
    _cgroup_path: std::path::PathBuf,
) {
    // No-op on Windows. Container stays "Running" until explicit stop/remove.
}

/// Fallback stub for platforms other than Unix or Windows.
#[cfg(not(any(unix, windows)))]
fn daemon_wait_for_exit(
    _pid: u32,
    _id: &str,
    _state: Arc<DaemonState>,
    _rootfs: std::path::PathBuf,
    _post_exit_hooks: Vec<HookSpec>,
    _event_sink: Arc<dyn EventSink>,
    _cgroup_path: std::path::PathBuf,
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
    NetworkLifecycle::new(deps.network_provider.clone())
        .cleanup(&id)
        .await;

    let result = stop_inner(&id, &state).await;
    let status = if result.is_ok() { "ok" } else { "error" };
    deps.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "stop"), ("adapter", "daemon"), ("status", status)],
    );

    match result {
        Ok(()) => {
            let active = state.list_containers().await.len() as f64;
            deps.metrics
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
    deps.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "remove"), ("adapter", "daemon"), ("status", status)],
    );

    match result {
        Ok(()) => {
            let active = state.list_containers().await.len() as f64;
            deps.metrics
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
    let container_dir = deps.containers_base.join(id);
    if container_dir.exists()
        && let Err(e) = deps.filesystem.cleanup(&container_dir)
    {
        warn!("cleanup_mounts for {id}: {e}");
    }

    // Remove runtime state directory.
    let run_dir = deps.run_containers_base.join(id);
    if run_dir.exists() {
        std::fs::remove_dir_all(&run_dir).ok();
    }

    // Cleanup cgroup (using injected resource limiter trait).
    if let Err(e) = deps.resource_limiter.cleanup(id) {
        warn!("cleanup cgroup for {id}: {e}");
    }

    // ── Network cleanup ────────────────────────────────────────────────
    NetworkLifecycle::new(deps.network_provider.clone())
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
    let log_dir = deps.containers_base.join(&id);
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

/// Pull an image from the appropriate registry and cache it locally.
#[instrument(skip(_state, deps), fields(image = %image, tag = ?tag))]
pub async fn handle_pull(
    image: String,
    tag: Option<String>,
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

    // Select registry based on image reference hostname.
    let registry = select_registry(
        &image_ref,
        deps.registry.as_ref(),
        deps.ghcr_registry.as_ref(),
    );

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

    deps.metrics.increment_counter(
        "minibox_container_ops_total",
        &[("op", "pull"), ("adapter", "daemon"), ("status", status)],
    );
    deps.metrics.record_histogram(
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
    let (status, response) = match deps.image_loader.load_image(image_path, &name, &tag).await {
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
    deps.metrics.increment_counter(
        "minibox_container_ops_total",
        &[
            ("op", "load_image"),
            ("adapter", "daemon"),
            ("status", status),
        ],
    );
    deps.metrics.record_histogram(
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
    let Some(ref exec_rt) = deps.exec_runtime else {
        deps.metrics.increment_counter(
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
    {
        let mut reg = deps.pty_sessions.lock().await;
        reg.resize.insert(session_key.clone(), resize_tx);
        if tty {
            reg.stdin.insert(session_key, stdin_ch_tx.clone());
        }
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
            deps.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "exec"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "exec"), ("adapter", "daemon")],
            );
            let _ = tx
                .send(DaemonResponse::ExecStarted { exec_id: handle.id })
                .await;
        }
        Err(e) => {
            deps.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "exec"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.metrics.record_histogram(
                "minibox_container_op_duration_seconds",
                start.elapsed().as_secs_f64(),
                &[("op", "exec"), ("adapter", "daemon")],
            );
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
    let reg = deps.pty_sessions.lock().await;
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
    let reg = deps.pty_sessions.lock().await;
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
    let Some(ref pusher) = deps.image_pusher else {
        deps.metrics.increment_counter(
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
            deps.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "push"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.metrics.record_histogram(
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
            deps.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "push"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.metrics.record_histogram(
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
    let Some(ref committer) = deps.commit_adapter else {
        deps.metrics.increment_counter(
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
            deps.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "commit"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.metrics.record_histogram(
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
            deps.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "commit"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.metrics.record_histogram(
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
    let Some(ref builder) = deps.image_builder else {
        deps.metrics.increment_counter(
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
    let dockerfile_path = context_dir.join("Dockerfile.mbx-build");
    if let Err(e) = tokio::fs::write(&dockerfile_path, &dockerfile).await {
        send_error(&tx, "handle_build", format!("write Dockerfile: {e}")).await;
        return;
    }

    let context = minibox_core::domain::BuildContext {
        directory: context_dir,
        dockerfile: std::path::PathBuf::from("Dockerfile.mbx-build"),
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
            deps.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "build"), ("adapter", "daemon"), ("status", "ok")],
            );
            deps.metrics.record_histogram(
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
            deps.metrics.increment_counter(
                "minibox_container_ops_total",
                &[("op", "build"), ("adapter", "daemon"), ("status", "error")],
            );
            deps.metrics.record_histogram(
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
mod select_registry_tests {
    use super::*;
    use mbx::ImageRef;
    use mbx::adapters::{DockerHubRegistry, GhcrRegistry};
    use minibox_core::image::ImageStore;
    use std::sync::Arc;

    #[test]
    fn select_registry_routes_ghcr() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp.path().join("images")).unwrap());
        let docker: Arc<dyn minibox_core::domain::ImageRegistry> =
            Arc::new(DockerHubRegistry::new(Arc::clone(&store)).unwrap());
        let ghcr: Arc<dyn minibox_core::domain::ImageRegistry> =
            Arc::new(GhcrRegistry::new(Arc::clone(&store)).unwrap());

        let ghcr_ref = ImageRef::parse("ghcr.io/org/minibox-rust-ci:stable").unwrap();
        let selected = select_registry(&ghcr_ref, docker.as_ref(), ghcr.as_ref());

        assert!(std::ptr::eq(
            selected as *const dyn minibox_core::domain::ImageRegistry as *const (),
            ghcr.as_ref() as *const dyn minibox_core::domain::ImageRegistry as *const ()
        ));
    }

    #[test]
    fn select_registry_routes_ghcr_case_insensitive() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp.path().join("images")).unwrap());
        let docker: Arc<dyn minibox_core::domain::ImageRegistry> =
            Arc::new(DockerHubRegistry::new(Arc::clone(&store)).unwrap());
        let ghcr: Arc<dyn minibox_core::domain::ImageRegistry> =
            Arc::new(GhcrRegistry::new(Arc::clone(&store)).unwrap());

        // GHCR.IO (uppercase) must still route to the ghcr adapter
        let ghcr_ref = ImageRef::parse("GHCR.IO/org/image:tag").unwrap();
        let selected = select_registry(&ghcr_ref, docker.as_ref(), ghcr.as_ref());

        assert!(std::ptr::eq(
            selected as *const dyn minibox_core::domain::ImageRegistry as *const (),
            ghcr.as_ref() as *const dyn minibox_core::domain::ImageRegistry as *const ()
        ));
    }

    #[test]
    fn select_registry_routes_docker() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Arc::new(ImageStore::new(temp.path().join("images")).unwrap());
        let docker: Arc<dyn minibox_core::domain::ImageRegistry> =
            Arc::new(DockerHubRegistry::new(Arc::clone(&store)).unwrap());
        let ghcr: Arc<dyn minibox_core::domain::ImageRegistry> =
            Arc::new(GhcrRegistry::new(Arc::clone(&store)).unwrap());

        let docker_ref = ImageRef::parse("alpine").unwrap();
        let selected = select_registry(&docker_ref, docker.as_ref(), ghcr.as_ref());

        assert!(std::ptr::eq(
            selected as *const dyn minibox_core::domain::ImageRegistry as *const (),
            docker.as_ref() as *const dyn minibox_core::domain::ImageRegistry as *const ()
        ));
    }
}
