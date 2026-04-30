//! miniboxd — cross-platform container runtime daemon.
//!
//! Listens on a Unix socket (Linux/macOS) or Named Pipe (Windows) and serves
//! JSON-over-newline requests from `minibox` CLI clients.
//!
//! # Adapter suites
//!
//! The daemon supports multiple adapter suites selected via the
//! `MINIBOX_ADAPTER` environment variable (default: `smolvm`, fallback: `krun`):
//!
//! - **smolvm** (default): SmolVM lightweight Linux VMs. Cross-platform. Falls back to krun
//!   automatically when the `smolvm` binary is absent and `MINIBOX_ADAPTER` is unset.
//! - **krun** (fallback): libkrun micro-VM (KVM on Linux, HVF on macOS). Cross-platform.
//! - **native** (Linux only): Linux namespaces, overlay FS, cgroups v2. Requires root.
//! - **gke** (Linux only): proot (ptrace), copy FS, no-op limiter. Unprivileged.
//! - **colima**: Colima/Lima VM via limactl + nerdctl. Cross-platform.
//! - **vz** (macOS only, feature-gated, opt-in): VZ.framework micro-VM.
//!
//! # Startup sequence
//! 1. Detect VZ adapter — if so, use GCD dispatch_main pattern (macOS only).
//! 2. Otherwise: build tokio runtime, run `run_daemon()`.
//! 3. `run_daemon()`: tracing → adapter selection → privilege check →
//!    path resolution → directory creation → state load → dependency injection →
//!    socket bind → signal handler → accept loop.

// ── Windows ───────────────────────────────────────────────────────────────
#[cfg(target_os = "windows")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    winbox::start().await
}

// ── Unix (Linux + macOS) ──────────────────────────────────────────────────
#[cfg(unix)]
fn main() {
    use miniboxd::adapter_registry;

    // Peek at the adapter before building the tokio runtime.
    // VZ needs GCD dispatch_main on the real main thread.
    let _adapter_name = std::env::var("MINIBOX_ADAPTER")
        .unwrap_or_else(|_| adapter_registry::DEFAULT_ADAPTER_SUITE.to_string());

    #[cfg(all(target_os = "macos", feature = "vz"))]
    if _adapter_name == "vz" {
        vz_main();
        // vz_main never returns (dispatch_main is divergent)
    }

    // All non-VZ adapters: standard tokio runtime.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    if let Err(e) = rt.block_on(run_daemon()) {
        eprintln!("miniboxd: fatal: {e:#}");
        std::process::exit(1);
    }
}

/// VZ adapter entry point — keeps the main thread for GCD dispatch_main.
///
/// VZ.framework (Tahoe/macOS 26+) asserts that VZVirtualMachineConfiguration
/// and VZVirtualMachine are constructed on the GCD main queue. `#[tokio::main]`
/// parks the main thread inside the tokio scheduler, so dispatch_sync to
/// the main queue from any worker thread deadlocks.
///
/// Fix: build the tokio runtime on a background thread, keep the main thread
/// free, then call `dispatch_main()` to spin the GCD main-queue runloop.
#[cfg(all(target_os = "macos", feature = "vz"))]
fn vz_main() -> ! {
    #[link(name = "System", kind = "dylib")]
    unsafe extern "C" {
        static _dispatch_main_q: std::ffi::c_void;
        fn dispatch_async_f(
            queue: *const std::ffi::c_void,
            context: *mut std::ffi::c_void,
            work: unsafe extern "C" fn(*mut std::ffi::c_void),
        );
        fn dispatch_main() -> !;
    }

    // SAFETY: ctx is a valid Box<i32> allocated below; called exactly once by GCD.
    unsafe extern "C" fn exit_trampoline(ctx: *mut std::ffi::c_void) {
        // SAFETY: ctx was created via Box::into_raw(Box::new(code)) below.
        let code = unsafe { *Box::from_raw(ctx as *mut i32) };
        std::process::exit(code);
    }

    // VZ adapter delegates to macbox::start() which handles the VZ-specific
    // dispatch_sync_f pattern for VM creation on the GCD main queue.
    std::thread::Builder::new()
        .name("tokio-main".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            let code = match rt.block_on(macbox::start()) {
                Ok(()) => 0i32,
                Err(e) => {
                    eprintln!("miniboxd: fatal: {e:#}");
                    1
                }
            };
            // SAFETY: _dispatch_main_q is a valid dispatch queue; ctx is a
            // heap-allocated i32 owned by exit_trampoline.
            unsafe {
                let ctx = Box::into_raw(Box::new(code)) as *mut std::ffi::c_void;
                dispatch_async_f(&_dispatch_main_q, ctx, exit_trampoline);
            }
        })
        .expect("failed to spawn tokio-main thread");

    // SAFETY: dispatch_main() is the documented entry point for GCD-based CLI
    // processes on macOS. It does not return.
    unsafe { dispatch_main() }
}

// ── Imports (unix only) ───────────────────────────────────────────────────

#[cfg(unix)]
use anyhow::{Context, Result};
#[cfg(unix)]
use macbox::krun::{
    filesystem::KrunFilesystem, limiter::KrunLimiter, registry::KrunRegistry, runtime::KrunRuntime,
};
#[cfg(unix)]
use minibox::adapters::NoopNetwork;
#[cfg(unix)]
use minibox::adapters::{SmolVmFilesystem, SmolVmLimiter, SmolVmRegistry, SmolVmRuntime};
#[cfg(unix)]
use minibox::daemon::handler::{ContainerPolicy, HandlerDependencies, PtySessionRegistry};
#[cfg(unix)]
use minibox::daemon::state::DaemonState;
#[cfg(unix)]
use minibox_core::adapters::HostnameRegistryRouter;
#[cfg(unix)]
use minibox_core::events::BroadcastEventBroker;
#[cfg(unix)]
use minibox_core::image::ImageStore;
#[cfg(unix)]
use minibox_core::image::gc::{ImageGarbageCollector, ImageGc};
#[cfg(unix)]
use minibox_core::image::lease::DiskLeaseService;
#[cfg(unix)]
use miniboxd::adapter_registry::{self, AdapterSuite};
#[cfg(unix)]
use miniboxd::listener::UnixServerListener;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use tokio::net::UnixListener;
#[cfg(unix)]
use tokio::sync::Mutex as TokioMutex;
#[cfg(unix)]
use tracing::{info, warn};

// Linux-only imports
#[cfg(target_os = "linux")]
use minibox::adapters::network::BridgeNetwork;
#[cfg(target_os = "linux")]
use minibox::adapters::{
    CgroupV2Limiter, DockerHubRegistry, GhcrRegistry, LinuxNamespaceRuntime, NativeImageLoader,
    OverlayFilesystem,
};
#[cfg(target_os = "linux")]
use minibox::adapters::{CopyFilesystem, NoopLimiter, ProotRuntime};
#[cfg(target_os = "linux")]
use minibox_core::image::registry::RegistryClient;
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(all(target_os = "linux", feature = "tailnet"))]
use tailbox::{TailnetConfig, TailnetNetwork};

// ── Path resolution ───────────────────────────────────────────────────────

/// Resolve data, run, and socket paths for the daemon.
///
/// On macOS, uses `macbox::paths` defaults (~/Library/Application Support,
/// /tmp/minibox). On Linux, uses UID-aware defaults (/var/lib/minibox for
/// root, ~/.minibox/cache for non-root). Environment variables override both.
#[cfg(unix)]
struct DaemonPaths {
    data_dir: PathBuf,
    run_dir: PathBuf,
    socket_path: PathBuf,
    images_dir: PathBuf,
    containers_dir: PathBuf,
    run_containers_dir: PathBuf,
}

#[cfg(unix)]
fn resolve_paths() -> DaemonPaths {
    let data_dir = std::env::var("MINIBOX_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| resolve_default_data_dir());

    let run_dir = std::env::var("MINIBOX_RUN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| resolve_default_run_dir());

    let socket_path = std::env::var("MINIBOX_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| run_dir.join("miniboxd.sock"));

    let images_dir = data_dir.join("images");
    let containers_dir = data_dir.join("containers");
    let run_containers_dir = run_dir.join("containers");

    DaemonPaths {
        data_dir,
        run_dir,
        socket_path,
        images_dir,
        containers_dir,
        run_containers_dir,
    }
}

#[cfg(unix)]
fn resolve_default_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        macbox::paths::data_dir()
    }
    #[cfg(target_os = "linux")]
    {
        let uid = nix::unistd::getuid().as_raw();
        resolve_data_dir_for_uid(uid)
    }
}

#[cfg(unix)]
fn resolve_default_run_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        macbox::paths::run_dir()
    }
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/run/minibox")
    }
}

/// Resolve the image/container data directory based on effective UID (Linux).
///
/// Resolution order:
/// 1. `MINIBOX_DATA_DIR` env var (explicit override) — handled by caller
/// 2. `~/.minibox/cache/` if uid is non-root
/// 3. `/var/lib/minibox/` if uid is root
#[cfg(unix)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn resolve_data_dir_for_uid(uid: u32) -> PathBuf {
    if let Ok(explicit) = std::env::var("MINIBOX_DATA_DIR") {
        return PathBuf::from(explicit);
    }
    if uid == 0 {
        PathBuf::from("/var/lib/minibox")
    } else {
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".minibox/cache"))
            .unwrap_or_else(|_| PathBuf::from("/var/lib/minibox"))
    }
}

// ── Unified daemon entry point ────────────────────────────────────────────

#[cfg(unix)]
async fn run_daemon() -> Result<()> {
    // ── Tracing ──────────────────────────────────────────────────────────
    #[cfg(feature = "otel")]
    let _otel_guard = {
        let otlp_endpoint = std::env::var("MINIBOX_OTLP_ENDPOINT").ok();
        minibox::daemon::telemetry::traces::init_tracing(otlp_endpoint.as_deref())
    };
    #[cfg(not(feature = "otel"))]
    minibox_core::init_tracing();

    info!("miniboxd starting");

    // ── Adapter suite ────────────────────────────────────────────────────
    let suite = adapter_registry::adapter_from_env().map_err(|e| anyhow::anyhow!("{e}"))?;
    let available = adapter_registry::available_adapter_names();
    info!(
        selected_adapter = %suite,
        available_adapters = ?available,
        "adapter suite selected"
    );

    // ── Privilege check (native only) ────────────────────────────────────
    #[cfg(target_os = "linux")]
    if suite == AdapterSuite::Native && !nix::unistd::getuid().is_root() {
        anyhow::bail!("miniboxd must run as root (native adapter suite)");
    }

    // ── Cgroup self-migration (native only) ──────────────────────────────
    #[cfg(target_os = "linux")]
    if suite == AdapterSuite::Native {
        migrate_to_supervisor_cgroup();
    }

    // ── Resolve paths (configurable via env vars) ───────────────────────
    let paths = resolve_paths();

    // ── Startup diagnostics ──────────────────────────────────────────────
    #[cfg(target_os = "linux")]
    {
        let cgroup_root = std::env::var("MINIBOX_CGROUP_ROOT")
            .unwrap_or_else(|_| "/sys/fs/cgroup/minibox.slice/miniboxd.service".to_string());
        minibox::daemon::server::log_startup_info(
            &paths.socket_path.display().to_string(),
            &paths.data_dir.display().to_string(),
            &cgroup_root,
        );
    }

    // ── Directories ──────────────────────────────────────────────────────
    {
        use std::os::unix::fs::DirBuilderExt;
        for dir in &[
            paths.images_dir.as_path(),
            paths.containers_dir.as_path(),
            paths.run_dir.as_path(),
            paths.run_containers_dir.as_path(),
        ] {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .with_context(|| format!("creating directory {}", dir.display()))?;
        }
    }

    // ── Shared state ─────────────────────────────────────────────────────
    let image_store = ImageStore::new(&paths.images_dir).context("creating image store")?;
    let state = Arc::new(DaemonState::new(image_store, &paths.data_dir));
    state.load_from_disk().await;
    info!("state loaded from disk");

    // ── Image GC ─────────────────────────────────────────────────────────
    let leases_path = paths.data_dir.join("leases.json");
    let lease_service = Arc::new(
        DiskLeaseService::new(leases_path)
            .await
            .context("creating lease service")?,
    );
    let image_gc: Arc<dyn ImageGarbageCollector> =
        Arc::new(ImageGc::new(Arc::clone(&state.image_store), lease_service));

    // ── Event broker ─────────────────────────────────────────────────────
    let event_broker = Arc::new(BroadcastEventBroker::new());

    // ── Metrics ──────────────────────────────────────────────────────────
    #[cfg(feature = "metrics")]
    let metrics_recorder = {
        let metrics_addr: std::net::SocketAddr = std::env::var("MINIBOX_METRICS_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:9090".to_string())
            .parse()
            .context("parsing MINIBOX_METRICS_ADDR")?;
        let recorder = Arc::new(minibox::daemon::telemetry::PrometheusMetricsRecorder::new());
        let (_addr, _handle) =
            minibox::daemon::telemetry::server::run_metrics_server(metrics_addr, recorder.clone())
                .await
                .context("starting metrics server")?;
        info!(addr = %_addr, "metrics server listening");
        recorder as Arc<dyn minibox_core::domain::MetricsRecorder>
    };
    #[cfg(not(feature = "metrics"))]
    let metrics_recorder: Arc<dyn minibox_core::domain::MetricsRecorder> =
        Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new());

    // ── Dependency Injection ─────────────────────────────────────────────
    let require_root_auth = suite == AdapterSuite::Native;

    let deps = build_handler_deps(
        suite,
        Arc::clone(&state),
        &paths,
        metrics_recorder.clone(),
        Arc::clone(&event_broker),
        Arc::clone(&image_gc),
    )
    .await?;

    info!("dependency injection configured");

    // ── Socket ───────────────────────────────────────────────────────────
    let sock_path = &paths.socket_path;
    if sock_path.exists() {
        warn!("removing stale socket at {}", sock_path.display());
        std::fs::remove_file(sock_path)
            .with_context(|| format!("removing stale socket {}", sock_path.display()))?;
    }

    let raw_listener = UnixListener::bind(sock_path)
        .with_context(|| format!("binding Unix socket at {}", sock_path.display()))?;

    // SECURITY: Restrict socket permissions; allow overrides for group access.
    {
        use std::os::unix::fs::PermissionsExt;
        let mut mode = 0o600;
        if let Ok(mode_str) = std::env::var("MINIBOX_SOCKET_MODE") {
            let mode_str = mode_str.trim();
            let mode_str = mode_str.strip_prefix("0o").unwrap_or(mode_str);
            match u32::from_str_radix(mode_str, 8) {
                Ok(parsed) => mode = parsed,
                Err(err) => warn!("invalid MINIBOX_SOCKET_MODE={mode_str}: {err}"),
            }
        }

        if let Ok(group_name) = std::env::var("MINIBOX_SOCKET_GROUP") {
            let group_name = group_name.trim();
            if !group_name.is_empty() {
                match nix::unistd::Group::from_name(group_name)
                    .with_context(|| format!("looking up group {group_name}"))?
                {
                    Some(group) => {
                        nix::unistd::chown(sock_path, None, Some(group.gid))
                            .with_context(|| format!("setting socket group to {group_name}"))?;
                        info!("socket group set to {group_name}");
                    }
                    None => warn!("MINIBOX_SOCKET_GROUP={group_name} not found"),
                }
            }
        }

        let metadata = std::fs::metadata(sock_path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(mode);
        std::fs::set_permissions(sock_path, permissions)
            .with_context(|| format!("setting socket permissions to {mode:04o}"))?;
        info!("socket permissions set to {mode:04o}");
    }

    info!("listening on {}", sock_path.display());

    // ── Signal handling ──────────────────────────────────────────────────
    use tokio::signal::unix::{SignalKind, signal};
    let mut sigterm = signal(SignalKind::terminate()).context("SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("SIGINT handler")?;
    let shutdown = async move {
        tokio::select! {
            _ = sigterm.recv() => { info!("received SIGTERM, shutting down"); }
            _ = sigint.recv()  => { info!("received SIGINT, shutting down");  }
        }
    };

    let listener = UnixServerListener(raw_listener);

    // ── Accept loop via run_server ──────────────────────────────────────
    minibox::daemon::server::run_server(listener, state, deps, require_root_auth, shutdown).await?;

    // ── Cleanup ──────────────────────────────────────────────────────────
    if sock_path.exists() {
        let _ = std::fs::remove_file(sock_path);
    }
    info!("miniboxd stopped");
    Ok(())
}

// ── Handler dependency builders (per adapter suite) ───────────────────────

#[cfg(unix)]
async fn build_handler_deps(
    suite: AdapterSuite,
    state: Arc<DaemonState>,
    paths: &DaemonPaths,
    metrics_recorder: Arc<dyn minibox_core::domain::MetricsRecorder>,
    event_broker: Arc<BroadcastEventBroker>,
    image_gc: Arc<dyn ImageGarbageCollector>,
) -> Result<Arc<HandlerDependencies>> {
    match suite {
        #[cfg(target_os = "linux")]
        AdapterSuite::Native => {
            let native_network = resolve_native_network().await?;
            build_native_handler_dependencies(
                Arc::clone(&state),
                &paths.data_dir,
                paths.containers_dir.clone(),
                paths.run_containers_dir.clone(),
                metrics_recorder,
                event_broker,
                image_gc,
                native_network,
            )
        }
        #[cfg(target_os = "linux")]
        AdapterSuite::Gke => build_gke_handler_dependencies(
            Arc::clone(&state),
            paths.containers_dir.clone(),
            paths.run_containers_dir.clone(),
            metrics_recorder,
            event_broker,
            image_gc,
        ),
        AdapterSuite::Colima => build_colima_handler_dependencies(
            Arc::clone(&state),
            paths.data_dir.clone(),
            paths.containers_dir.clone(),
            paths.run_containers_dir.clone(),
            image_gc,
        ),
        AdapterSuite::SmolVm => build_smolvm_handler_dependencies(
            Arc::clone(&state),
            paths.containers_dir.clone(),
            paths.run_containers_dir.clone(),
            metrics_recorder,
            event_broker,
            image_gc,
        ),
        AdapterSuite::Krun => build_krun_handler_dependencies(
            Arc::clone(&state),
            paths.containers_dir.clone(),
            paths.run_containers_dir.clone(),
            image_gc,
        ),
        // On macOS, native/gke are not available — the adapter_registry already
        // rejects them, but we need exhaustive match arms.
        #[cfg(not(target_os = "linux"))]
        AdapterSuite::Native | AdapterSuite::Gke => {
            anyhow::bail!("{suite} adapter requires Linux");
        }
    }
}

// ── Native adapter (Linux only) ──────────────────────────────────────────

#[cfg(target_os = "linux")]
async fn resolve_native_network() -> Result<Arc<dyn minibox_core::domain::NetworkProvider>> {
    let mode = std::env::var("MINIBOX_NETWORK_MODE").unwrap_or_else(|_| "none".to_string());
    info!(network_mode = %mode, "network provider selected");
    match mode.as_str() {
        "bridge" => Ok(Arc::new(
            BridgeNetwork::new().context("BridgeNetwork init failed")?,
        )),
        "host" => Ok(Arc::new(minibox::adapters::network::HostNetwork::new())),
        #[cfg(feature = "tailnet")]
        "tailnet" => {
            let tailnet_cfg = TailnetConfig {
                auth_key: std::env::var("TAILSCALE_AUTH_KEY").ok(),
                key_secret_name: std::env::var("MINIBOX_TAILNET_SECRET_NAME")
                    .unwrap_or_else(|_| "tailscale-auth-key".to_string()),
            };
            Ok(Arc::new(
                TailnetNetwork::new(tailnet_cfg)
                    .await
                    .context("TailnetNetwork init failed")?,
            ))
        }
        _ => Ok(Arc::new(NoopNetwork::new())),
    }
}

#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
fn build_native_handler_dependencies(
    state: Arc<DaemonState>,
    data_dir: &Path,
    containers_dir: PathBuf,
    run_containers_dir: PathBuf,
    metrics_recorder: Arc<dyn minibox_core::domain::MetricsRecorder>,
    event_broker: Arc<BroadcastEventBroker>,
    image_gc: Arc<dyn ImageGarbageCollector>,
    native_network: Arc<dyn minibox_core::domain::NetworkProvider>,
) -> Result<Arc<HandlerDependencies>> {
    let registry_router = Arc::new(HostnameRegistryRouter::new(
        Arc::new(
            DockerHubRegistry::new(Arc::clone(&state.image_store))
                .context("creating Docker Hub registry adapter")?,
        ),
        [(
            "ghcr.io",
            Arc::new(
                GhcrRegistry::new(Arc::clone(&state.image_store))
                    .context("creating GHCR registry adapter")?,
            ) as minibox_core::domain::DynImageRegistry,
        )],
    ));
    let commit_adapter = minibox::adapters::commit::overlay_commit_adapter(
        Arc::clone(&state.image_store),
        Arc::clone(&state) as minibox::daemonbox_state::StateHandle,
    );
    let image_builder = minibox::adapters::builder::minibox_image_builder(
        Arc::clone(&state.image_store),
        data_dir.to_path_buf(),
    );
    let image_pusher = minibox::adapters::push::oci_push_adapter(
        RegistryClient::new().context("creating OCI push registry client")?,
        Arc::clone(&state.image_store),
    );

    Ok(Arc::new(HandlerDependencies {
        image: minibox::daemon::handler::ImageDeps {
            registry_router,
            image_loader: Arc::new(NativeImageLoader::new(Arc::clone(&state.image_store))),
            image_gc,
            image_store: Arc::clone(&state.image_store),
        },
        lifecycle: minibox::daemon::handler::LifecycleDeps {
            filesystem: Arc::new(OverlayFilesystem::new()),
            resource_limiter: Arc::new(CgroupV2Limiter::new()),
            runtime: Arc::new(LinuxNamespaceRuntime::new()),
            network_provider: native_network,
            containers_base: containers_dir,
            run_containers_base: run_containers_dir,
        },
        exec: minibox::daemon::handler::ExecDeps {
            exec_runtime: Some(minibox::adapters::exec::native_exec_runtime(
                Arc::clone(&state) as minibox::daemonbox_state::StateHandle,
            )),
            pty_sessions: Arc::new(TokioMutex::new(PtySessionRegistry::default())),
        },
        build: minibox::daemon::handler::BuildDeps {
            image_pusher: Some(image_pusher),
            commit_adapter: Some(commit_adapter),
            image_builder: Some(image_builder),
        },
        events: minibox::daemon::handler::EventDeps {
            event_sink: Arc::clone(&event_broker) as Arc<dyn minibox_core::events::EventSink>,
            event_source: Arc::clone(&event_broker) as Arc<dyn minibox_core::events::EventSource>,
            metrics: metrics_recorder,
        },
        policy: ContainerPolicy::default(),
        checkpoint: Arc::new(minibox_core::domain::NoopVmCheckpoint),
    }))
}

// ── GKE adapter (Linux only) ─────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn build_gke_handler_dependencies(
    state: Arc<DaemonState>,
    containers_dir: PathBuf,
    run_containers_dir: PathBuf,
    metrics_recorder: Arc<dyn minibox_core::domain::MetricsRecorder>,
    event_broker: Arc<BroadcastEventBroker>,
    image_gc: Arc<dyn ImageGarbageCollector>,
) -> Result<Arc<HandlerDependencies>> {
    let registry_router = Arc::new(HostnameRegistryRouter::new(
        Arc::new(
            DockerHubRegistry::new(Arc::clone(&state.image_store))
                .context("creating Docker Hub registry adapter")?,
        ),
        [(
            "ghcr.io",
            Arc::new(
                GhcrRegistry::new(Arc::clone(&state.image_store))
                    .context("creating GHCR registry adapter")?,
            ) as minibox_core::domain::DynImageRegistry,
        )],
    ));
    let proot_runtime =
        ProotRuntime::from_env().context("initialising proot runtime for GKE adapter")?;
    Ok(Arc::new(HandlerDependencies {
        image: minibox::daemon::handler::ImageDeps {
            registry_router,
            image_loader: Arc::new(NativeImageLoader::new(Arc::clone(&state.image_store))),
            image_gc,
            image_store: Arc::clone(&state.image_store),
        },
        lifecycle: minibox::daemon::handler::LifecycleDeps {
            filesystem: Arc::new(CopyFilesystem::new()),
            resource_limiter: Arc::new(NoopLimiter::new()),
            runtime: Arc::new(proot_runtime),
            network_provider: Arc::new(NoopNetwork::new()),
            containers_base: containers_dir,
            run_containers_base: run_containers_dir,
        },
        exec: minibox::daemon::handler::ExecDeps {
            exec_runtime: None,
            pty_sessions: Arc::new(TokioMutex::new(PtySessionRegistry::default())),
        },
        build: minibox::daemon::handler::BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: minibox::daemon::handler::EventDeps {
            event_sink: Arc::clone(&event_broker) as Arc<dyn minibox_core::events::EventSink>,
            event_source: Arc::clone(&event_broker) as Arc<dyn minibox_core::events::EventSource>,
            metrics: metrics_recorder,
        },
        policy: ContainerPolicy::default(),
        checkpoint: Arc::new(minibox_core::domain::NoopVmCheckpoint),
    }))
}

// ── Colima adapter (cross-platform) ──────────────────────────────────────

#[cfg(unix)]
fn build_colima_handler_dependencies(
    state: Arc<DaemonState>,
    data_dir: PathBuf,
    containers_dir: PathBuf,
    run_containers_dir: PathBuf,
    image_gc: Arc<dyn ImageGarbageCollector>,
) -> Result<Arc<HandlerDependencies>> {
    use minibox::adapters::{LimaExecutor, LimaSpawner};

    let executor: LimaExecutor = Arc::new(move |args: &[&str]| {
        let output = std::process::Command::new("colima")
            .arg("ssh")
            .arg("--")
            .args(args)
            .output()
            .map_err(|e| anyhow::anyhow!("colima ssh exec failed: {e}"))?;
        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "limactl command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    });

    let spawner: LimaSpawner = Arc::new(move |args: &[&str]| {
        std::process::Command::new("colima")
            .arg("ssh")
            .arg("--")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|e| anyhow::anyhow!("colima ssh spawn failed: {e}"))
    });

    macbox::build_colima_handler_dependencies(
        state,
        data_dir,
        containers_dir,
        run_containers_dir,
        image_gc,
        executor,
        spawner,
    )
}

// ── SmolVM adapter (cross-platform) ──────────────────────────────────────

#[cfg(unix)]
fn build_smolvm_handler_dependencies(
    state: Arc<DaemonState>,
    containers_dir: PathBuf,
    run_containers_dir: PathBuf,
    metrics_recorder: Arc<dyn minibox_core::domain::MetricsRecorder>,
    event_broker: Arc<BroadcastEventBroker>,
    image_gc: Arc<dyn ImageGarbageCollector>,
) -> Result<Arc<HandlerDependencies>> {
    let registry_router = Arc::new(HostnameRegistryRouter::new(
        Arc::new(SmolVmRegistry::new()) as minibox_core::domain::DynImageRegistry,
        std::iter::empty::<(&str, minibox_core::domain::DynImageRegistry)>(),
    ));
    Ok(Arc::new(HandlerDependencies {
        image: minibox::daemon::handler::ImageDeps {
            registry_router,
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc,
            image_store: Arc::clone(&state.image_store),
        },
        lifecycle: minibox::daemon::handler::LifecycleDeps {
            filesystem: Arc::new(SmolVmFilesystem::new()),
            resource_limiter: Arc::new(SmolVmLimiter::new()),
            runtime: Arc::new(SmolVmRuntime::new()),
            network_provider: Arc::new(NoopNetwork::new()),
            containers_base: containers_dir,
            run_containers_base: run_containers_dir,
        },
        exec: minibox::daemon::handler::ExecDeps {
            exec_runtime: None,
            pty_sessions: Arc::new(TokioMutex::new(PtySessionRegistry::default())),
        },
        build: minibox::daemon::handler::BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: minibox::daemon::handler::EventDeps {
            event_sink: Arc::clone(&event_broker) as Arc<dyn minibox_core::events::EventSink>,
            event_source: Arc::clone(&event_broker) as Arc<dyn minibox_core::events::EventSource>,
            metrics: metrics_recorder,
        },
        policy: ContainerPolicy::default(),
        checkpoint: Arc::new(minibox_core::domain::NoopVmCheckpoint),
    }))
}

// ── Krun adapter (cross-platform) ────────────────────────────────────────

#[cfg(unix)]
fn build_krun_handler_dependencies(
    state: Arc<DaemonState>,
    containers_dir: PathBuf,
    run_containers_dir: PathBuf,
    image_gc: Arc<dyn ImageGarbageCollector>,
) -> Result<Arc<HandlerDependencies>> {
    let registry = Arc::new(
        KrunRegistry::new(Arc::clone(&state.image_store))
            .context("creating krun registry adapter")?,
    );
    let registry_port: minibox_core::domain::DynImageRegistry = registry;

    Ok(Arc::new(HandlerDependencies {
        image: minibox::daemon::handler::ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                registry_port,
                std::iter::empty::<(&str, minibox_core::domain::DynImageRegistry)>(),
            )),
            image_loader: Arc::new(minibox::daemon::handler::NoopImageLoader),
            image_gc,
            image_store: Arc::clone(&state.image_store),
        },
        lifecycle: minibox::daemon::handler::LifecycleDeps {
            filesystem: Arc::new(KrunFilesystem::new()),
            resource_limiter: Arc::new(KrunLimiter::new()),
            runtime: Arc::new(KrunRuntime::new()),
            network_provider: Arc::new(NoopNetwork::new()),
            containers_base: containers_dir,
            run_containers_base: run_containers_dir,
        },
        exec: minibox::daemon::handler::ExecDeps {
            exec_runtime: None,
            pty_sessions: Arc::new(TokioMutex::new(PtySessionRegistry::default())),
        },
        build: minibox::daemon::handler::BuildDeps {
            image_pusher: None,
            commit_adapter: None,
            image_builder: None,
        },
        events: minibox::daemon::handler::EventDeps {
            event_sink: Arc::new(minibox_core::events::NoopEventSink),
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: ContainerPolicy::default(),
        checkpoint: Arc::new(minibox_core::domain::NoopVmCheckpoint),
    }))
}

// ── Cgroup self-migration (Linux only) ────────────────────────────────────

#[cfg(target_os = "linux")]
fn migrate_to_supervisor_cgroup() {
    use std::fs;
    use tracing::{debug, warn};

    let cgroup_entry = match fs::read_to_string("/proc/self/cgroup") {
        Ok(s) => s,
        Err(e) => {
            warn!("could not read /proc/self/cgroup, skipping self-migration: {e}");
            return;
        }
    };

    let cgroup_path = match cgroup_entry.lines().find_map(|l| l.strip_prefix("0::")) {
        Some(p) => p.trim().to_string(),
        None => {
            warn!("no cgroup v2 entry in /proc/self/cgroup, skipping self-migration");
            return;
        }
    };

    if cgroup_path.ends_with("/supervisor") {
        debug!("already in supervisor cgroup, skipping self-migration");
        return;
    }

    let cgroupfs = PathBuf::from("/sys/fs/cgroup");
    let relative = cgroup_path.strip_prefix('/').unwrap_or(&cgroup_path);
    let supervisor_dir = cgroupfs.join(relative).join("supervisor");

    if let Err(e) = fs::create_dir_all(&supervisor_dir) {
        warn!(
            "could not create supervisor cgroup at {}: {e}",
            supervisor_dir.display()
        );
        return;
    }

    let procs_file = supervisor_dir.join("cgroup.procs");
    let pid = std::process::id().to_string();
    if let Err(e) = fs::write(&procs_file, &pid) {
        warn!("could not migrate to supervisor cgroup: {e}");
        return;
    }

    info!(
        "migrated to supervisor cgroup at {}",
        supervisor_dir.display()
    );
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::*;

    #[cfg(unix)]
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    #[cfg(unix)]
    fn resolve_data_dir_non_root_uses_home() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("MINIBOX_DATA_DIR");
            std::env::set_var("HOME", "/home/testuser");
        }
        let dir = resolve_data_dir_for_uid(1000);
        unsafe {
            std::env::remove_var("HOME");
        }
        assert_eq!(dir, PathBuf::from("/home/testuser/.minibox/cache"));
    }

    #[test]
    #[cfg(unix)]
    fn resolve_data_dir_root_uses_var_lib() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("MINIBOX_DATA_DIR");
        }
        let dir = resolve_data_dir_for_uid(0);
        assert_eq!(dir, PathBuf::from("/var/lib/minibox"));
    }

    #[test]
    #[cfg(unix)]
    fn resolve_data_dir_env_override_takes_precedence() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("MINIBOX_DATA_DIR", "/custom/path");
        }
        let dir_non_root = resolve_data_dir_for_uid(1000);
        let dir_root = resolve_data_dir_for_uid(0);
        unsafe {
            std::env::remove_var("MINIBOX_DATA_DIR");
        }
        assert_eq!(dir_non_root, PathBuf::from("/custom/path"));
        assert_eq!(dir_root, PathBuf::from("/custom/path"));
    }

    #[test]
    #[cfg(unix)]
    fn resolve_paths_uses_env_overrides() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var("MINIBOX_DATA_DIR", "/test/data");
            std::env::set_var("MINIBOX_RUN_DIR", "/test/run");
            std::env::set_var("MINIBOX_SOCKET_PATH", "/test/run/custom.sock");
        }
        let paths = resolve_paths();
        unsafe {
            std::env::remove_var("MINIBOX_DATA_DIR");
            std::env::remove_var("MINIBOX_RUN_DIR");
            std::env::remove_var("MINIBOX_SOCKET_PATH");
        }
        assert_eq!(paths.data_dir, PathBuf::from("/test/data"));
        assert_eq!(paths.run_dir, PathBuf::from("/test/run"));
        assert_eq!(paths.socket_path, PathBuf::from("/test/run/custom.sock"));
        assert_eq!(paths.images_dir, PathBuf::from("/test/data/images"));
        assert_eq!(paths.containers_dir, PathBuf::from("/test/data/containers"));
        assert_eq!(
            paths.run_containers_dir,
            PathBuf::from("/test/run/containers")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_paths_macos_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("MINIBOX_DATA_DIR");
            std::env::remove_var("MINIBOX_RUN_DIR");
            std::env::remove_var("MINIBOX_SOCKET_PATH");
        }
        let paths = resolve_paths();
        // On macOS, defaults come from macbox::paths
        assert_eq!(paths.run_dir, PathBuf::from("/tmp/minibox"));
        assert!(
            paths.data_dir.to_string_lossy().contains("minibox"),
            "macOS data_dir should contain minibox"
        );
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn native_suite_wires_local_push_commit_and_build_adapters() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        let images_dir = data_dir.join("images");
        std::fs::create_dir_all(&images_dir).unwrap();
        let image_store = ImageStore::new(&images_dir).unwrap();
        let state = Arc::new(DaemonState::new(image_store, &data_dir));

        let lease_service = Arc::new(
            DiskLeaseService::new(data_dir.join("leases.json"))
                .await
                .unwrap(),
        );
        let image_gc: Arc<dyn ImageGarbageCollector> =
            Arc::new(ImageGc::new(Arc::clone(&state.image_store), lease_service));
        let event_broker = Arc::new(BroadcastEventBroker::new());
        let metrics_recorder: Arc<dyn minibox_core::domain::MetricsRecorder> =
            Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new());
        let native_network: Arc<dyn minibox_core::domain::NetworkProvider> =
            Arc::new(NoopNetwork::new());

        let deps = build_native_handler_dependencies(
            Arc::clone(&state),
            &data_dir,
            data_dir.join("containers"),
            data_dir.join("run/containers"),
            metrics_recorder,
            event_broker,
            image_gc,
            native_network,
        )
        .unwrap();

        assert!(
            deps.build.image_pusher.is_some(),
            "native suite should wire image push"
        );
        assert!(
            deps.build.commit_adapter.is_some(),
            "native suite should wire container commit"
        );
        assert!(
            deps.build.image_builder.is_some(),
            "native suite should wire image build"
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn krun_suite_wires_krun_adapters() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        let images_dir = data_dir.join("images");
        std::fs::create_dir_all(&images_dir).unwrap();
        let image_store = ImageStore::new(&images_dir).unwrap();
        let state = Arc::new(DaemonState::new(image_store, &data_dir));
        let lease_service = Arc::new(
            DiskLeaseService::new(data_dir.join("leases.json"))
                .await
                .unwrap(),
        );
        let image_gc: Arc<dyn ImageGarbageCollector> =
            Arc::new(ImageGc::new(Arc::clone(&state.image_store), lease_service));

        let deps = build_krun_handler_dependencies(
            state,
            data_dir.join("containers"),
            data_dir.join("run/containers"),
            image_gc,
        )
        .unwrap();

        // Krun has no build adapters
        assert!(deps.build.image_pusher.is_none());
        assert!(deps.build.commit_adapter.is_none());
        assert!(deps.build.image_builder.is_none());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn smolvm_suite_wires_smolvm_adapters() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let data_dir = temp_dir.path().to_path_buf();
        let images_dir = data_dir.join("images");
        std::fs::create_dir_all(&images_dir).unwrap();
        let image_store = ImageStore::new(&images_dir).unwrap();
        let state = Arc::new(DaemonState::new(image_store, &data_dir));
        let lease_service = Arc::new(
            DiskLeaseService::new(data_dir.join("leases.json"))
                .await
                .unwrap(),
        );
        let image_gc: Arc<dyn ImageGarbageCollector> =
            Arc::new(ImageGc::new(Arc::clone(&state.image_store), lease_service));
        let event_broker = Arc::new(BroadcastEventBroker::new());
        let metrics_recorder: Arc<dyn minibox_core::domain::MetricsRecorder> =
            Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new());

        let deps = build_smolvm_handler_dependencies(
            state,
            data_dir.join("containers"),
            data_dir.join("run/containers"),
            metrics_recorder,
            event_broker,
            image_gc,
        )
        .unwrap();

        assert!(deps.build.image_pusher.is_none());
    }
}
