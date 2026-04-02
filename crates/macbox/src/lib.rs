//! macOS orchestration for miniboxd via the Colima adapter suite.
//!
//! On macOS, Linux container primitives (namespaces, cgroups, overlayfs) are
//! unavailable. This crate bridges that gap by delegating container operations
//! to [Colima](https://github.com/abiosoft/colima) — a lightweight Linux VM
//! that exposes a `nerdctl`-compatible interface on top of `limactl`.
//!
//! `miniboxd` selects this code path when compiled for macOS (see the
//! `#[cfg(target_os = "macos")]` dispatch in `miniboxd/src/main.rs`).
//! The `MINIBOX_ADAPTER=colima` environment variable does **not** need to be
//! set explicitly on macOS — the platform dispatch happens at compile time.
//!
//! # Modules
//!
//! - [`paths`] — macOS-specific default directories and socket path
//! - [`preflight`] — Colima/backend detection via `colima status`
//! - [`vz`] — VZ.framework and vsock integration

pub mod paths;
pub mod preflight;

#[cfg(feature = "vz")]
pub mod vz;

use anyhow::{Context, Result};
use daemonbox::handler::HandlerDependencies;
use daemonbox::state::DaemonState;
use mbx::adapters::{
    ColimaFilesystem, ColimaLimiter, ColimaRegistry, ColimaRuntime, LimaExecutor, LimaSpawner,
    NoopNetwork,
};
use minibox_core::image::ImageStore;
use minibox_core::image::gc::{ImageGarbageCollector, ImageGc};
use minibox_core::image::lease::DiskLeaseService;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::UnixListener;
use tracing::{info, warn};

/// Errors that can be returned by the macOS daemon entry point.
#[derive(thiserror::Error, Debug)]
pub enum MacboxError {
    /// Colima is not installed or not reachable. The user must install and
    /// start Colima before running miniboxd on macOS.
    #[error("no container backend — install Colima (`brew install colima && colima start`)")]
    NoBackendAvailable,
}

/// Newtype wrapper around [`tokio::net::UnixListener`] that implements
/// [`daemonbox::server::ServerListener`] for the macOS daemon.
///
/// On macOS, `SO_PEERCRED` is not available through the `nix` crate, so
/// peer credential checking is skipped (the `accept` implementation returns
/// `None` for `PeerCreds`). This means the UID-based root-auth guard is
/// disabled on macOS — container operations are delegated to the Colima VM
/// anyway, so the attack surface is limited to whoever can reach the socket.
struct MacUnixListener(UnixListener);

impl daemonbox::server::ServerListener for MacUnixListener {
    type Stream = tokio::net::UnixStream;

    async fn accept(&self) -> anyhow::Result<(Self::Stream, Option<daemonbox::server::PeerCreds>)> {
        let (stream, _addr) = self.0.accept().await?;
        // On macOS SO_PEERCRED is not available via nix; skip credential check.
        Ok((stream, None))
    }
}

/// Start the macOS daemon using the Colima adapter suite.
///
/// Called from `miniboxd`'s macOS `main()`. Performs the following steps:
///
/// 1. Initialises `tracing_subscriber` from `RUST_LOG`.
/// 2. Resolves runtime directories from environment variables
///    (`MINIBOX_DATA_DIR`, `MINIBOX_RUN_DIR`, `MINIBOX_SOCKET_PATH`) with
///    macOS-specific defaults from [`paths`] as fallbacks.
/// 3. Creates missing directories for images, containers, and the socket.
/// 4. Loads persisted container state from disk.
/// 5. Wires up the full Colima adapter suite:
///    [`ColimaRegistry`], [`ColimaFilesystem`], [`ColimaLimiter`],
///    [`ColimaRuntime`].
/// 6. Removes any stale socket file, binds a new Unix socket, and runs the
///    [`daemonbox::server::run_server`] accept loop with root-auth disabled.
/// 7. Cleans up the socket file on graceful shutdown (Ctrl-C).
pub async fn start() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("miniboxd (macOS) starting");

    // ── Paths ────────────────────────────────────────────────────────────
    let data_dir = std::env::var("MINIBOX_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| paths::data_dir());
    let run_dir = std::env::var("MINIBOX_RUN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| paths::run_dir());
    let socket_path = std::env::var("MINIBOX_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| paths::socket_path());

    let images_dir = data_dir.join("images");
    let containers_dir = data_dir.join("containers");
    let run_containers_dir = run_dir.join("containers");

    for dir in &[&images_dir, &containers_dir, &run_dir, &run_containers_dir] {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating directory {}", dir.display()))?;
    }

    // ── Shared state ─────────────────────────────────────────────────────
    let image_store = ImageStore::new(&images_dir).context("creating image store")?;
    let state = Arc::new(DaemonState::new(image_store, &data_dir));
    state.load_from_disk().await;
    info!("state loaded from disk");

    // ── Image GC ─────────────────────────────────────────────────────────
    let lease_service = Arc::new(
        DiskLeaseService::new(data_dir.join("leases.json"))
            .await
            .context("creating lease service")?,
    );
    let image_gc: Arc<dyn ImageGarbageCollector> =
        Arc::new(ImageGc::new(Arc::clone(&state.image_store), lease_service));

    // ── VZ branch ────────────────────────────────────────────────────────
    #[cfg(feature = "vz")]
    if std::env::var("MINIBOX_ADAPTER").as_deref() == Ok("vz") {
        return start_vz(
            socket_path,
            images_dir,
            containers_dir,
            run_containers_dir,
            state,
        )
        .await;
    }

    // ── Dependency Injection — Colima adapter suite ──────────────────────

    // Shared executor closure — runs fire-and-forget commands inside the Lima VM.
    // Uses `colima ssh --` rather than `limactl shell colima` because Colima manages
    // its own Lima instance directory and limactl may not find it via LIMA_HOME.
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

    // Spawner closure — starts a long-lived process with piped stdout.
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

    let colima_image_loader = Arc::new(ColimaRegistry::new().with_executor(executor.clone()));
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(ColimaRegistry::new().with_executor(executor.clone())),
        // Colima's nerdctl handles any registry (ghcr.io included) via the same adapter.
        ghcr_registry: Arc::new(ColimaRegistry::new().with_executor(executor.clone())),
        filesystem: Arc::new(ColimaFilesystem::new()),
        resource_limiter: Arc::new(ColimaLimiter::new().with_executor(executor.clone())),
        runtime: Arc::new(
            ColimaRuntime::new()
                .with_executor(executor)
                .with_spawner(spawner),
        ),
        network_provider: Arc::new(NoopNetwork::new()),
        containers_base: containers_dir,
        run_containers_base: run_containers_dir,
        metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        image_loader: colima_image_loader as minibox_core::domain::DynImageLoader,
        exec_runtime: None,
        image_pusher: None,
        commit_adapter: None,
        image_builder: None,
        event_sink: Arc::new(minibox_core::events::NoopEventSink),
        event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
        image_gc: Arc::clone(&image_gc),
        image_store: Arc::clone(&state.image_store),
    });

    // ── Socket ───────────────────────────────────────────────────────────
    if socket_path.exists() {
        warn!("removing stale socket at {}", socket_path.display());
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
    }

    let raw_listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("binding Unix socket at {}", socket_path.display()))?;

    // SECURITY: Restrict socket to owner-only (0o600). macOS does not require
    // root auth for Colima (operations run in the VM), but the socket should
    // still not be accessible to other local users.
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&socket_path, permissions)
            .with_context(|| format!("setting socket permissions on {}", socket_path.display()))?;
        info!("socket permissions set to 0600");
    }

    info!("listening on {}", socket_path.display());

    // ── Signal handling ──────────────────────────────────────────────────
    let shutdown = async {
        tokio::signal::ctrl_c().await.ok();
        info!("received Ctrl-C, shutting down");
    };

    // macOS: no root auth for Colima (operations run in the VM).
    daemonbox::server::run_server(
        MacUnixListener(raw_listener),
        state,
        deps,
        false, // require_root_auth
        shutdown,
    )
    .await?;

    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }
    info!("miniboxd (macOS) stopped");
    Ok(())
}

/// Start the macOS daemon using the VZ.framework adapter suite.
///
/// Selected when `MINIBOX_ADAPTER=vz` is set and the `vz` feature is compiled
/// in. Boots a [`vz::vm::VzVm`], waits for the in-VM agent to accept
/// connections over vsock, wires up the four Vz* adapters into
/// [`HandlerDependencies`], and then runs the standard socket server accept
/// loop.
#[cfg(feature = "vz")]
async fn start_vz(
    socket_path: std::path::PathBuf,
    images_dir: std::path::PathBuf,
    containers_dir: std::path::PathBuf,
    run_containers_dir: std::path::PathBuf,
    state: Arc<daemonbox::state::DaemonState>,
) -> Result<()> {
    use vz::vm::{VzVm, VzVmConfig, default_vm_dir};
    use vz::{VzFilesystem, VzLimiter, VzRegistry, VzRuntime};

    let vm_dir = default_vm_dir()
        .ok_or_else(|| anyhow::anyhow!("vz: cannot determine home directory for VM image path"))?;

    info!(vm_dir = %vm_dir.display(), "vz: booting Linux VM");

    let config = VzVmConfig {
        vm_dir,
        images_dir,
        containers_dir: containers_dir.clone(),
        memory_bytes: 1 * 1024 * 1024 * 1024, // 1 GiB
        cpu_count: 2,
    };

    // VZ.framework (Tahoe/macOS 26+) requires VZVirtualMachineConfiguration and
    // VZVirtualMachine to be constructed on the GCD main queue. The start
    // completion handler is also dispatched back to the main queue, so we must
    // NOT block the main queue while waiting for it to fire.
    //
    // Two-phase boot:
    //   1. dispatch_sync_f to main queue: build config, create VM, call
    //      startWithCompletionHandler — returns immediately with a shared signal.
    //   2. Poll the signal from the tokio worker thread (main queue stays free).
    //
    // No new deps — _dispatch_main_q / dispatch_sync_f are in libSystem.
    #[link(name = "System", kind = "dylib")]
    unsafe extern "C" {
        static _dispatch_main_q: std::ffi::c_void;
        fn dispatch_sync_f(
            queue: *const std::ffi::c_void,
            context: *mut std::ffi::c_void,
            work: unsafe extern "C" fn(*mut std::ffi::c_void),
        );
    }

    // Phase 1: prepare on main queue via dispatch_sync_f.
    type PrepareResult = anyhow::Result<(VzVm, Arc<std::sync::Mutex<Option<Result<(), String>>>>)>;
    struct PrepareCtx {
        config: Option<VzVmConfig>,
        result: Option<PrepareResult>,
    }
    unsafe extern "C" fn prepare_trampoline(ctx: *mut std::ffi::c_void) {
        // SAFETY: ctx is a valid &mut PrepareCtx allocated on the stack in the
        // spawn_blocking closure below; dispatch_sync_f guarantees it outlives
        // this call and that this function runs exactly once.
        let c = unsafe { &mut *(ctx as *mut PrepareCtx) };
        let config = c.config.take().expect("PrepareCtx config missing");
        c.result = Some(VzVm::prepare_on_main_queue(config));
    }

    let (vm, start_signal) = tokio::task::spawn_blocking(move || {
        let mut ctx = PrepareCtx {
            config: Some(config),
            result: None,
        };
        // SAFETY: _dispatch_main_q is the live GCD main queue (kept running by
        // dispatch_main() in miniboxd's main()); prepare_trampoline writes to ctx
        // before dispatch_sync_f returns; ctx is stack-allocated and outlives the call.
        unsafe {
            dispatch_sync_f(
                &_dispatch_main_q,
                &mut ctx as *mut PrepareCtx as *mut std::ffi::c_void,
                prepare_trampoline,
            );
        }
        ctx.result.expect("prepare_trampoline did not set result")
    })
    .await
    .context("spawn_blocking prepare_on_main_queue")??;

    // Phase 2: poll from worker thread — main queue stays free for VZ callbacks.
    let vm = tokio::task::spawn_blocking(move || VzVm::wait_for_running(vm, start_signal))
        .await
        .context("spawn_blocking wait_for_running")??;

    info!(
        port = vz::vsock::AGENT_PORT,
        "vz: VM booted, waiting for agent"
    );

    let vm_arc = Arc::new(vm);

    // Wait for the in-VM agent to start accepting vsock connections.
    vz::vsock::connect_to_agent(&vm_arc, 60)
        .await
        .context("vz: agent did not come up within 60 attempts")?;
    info!("vz: agent ready");

    // Build a minimal GC for VZ — leases file lives next to images_dir.
    let vz_image_store_ref = Arc::clone(&state.image_store);
    let vz_lease_path = images_dir
        .parent()
        .unwrap_or(&images_dir)
        .join("leases.json");
    let vz_leases = Arc::new(
        DiskLeaseService::new(vz_lease_path)
            .await
            .context("vz: creating lease service")?,
    );
    let vz_image_gc: Arc<dyn ImageGarbageCollector> =
        Arc::new(ImageGc::new(Arc::clone(&vz_image_store_ref), vz_leases));
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(VzRegistry::new(Arc::clone(&vm_arc))),
        ghcr_registry: Arc::new(VzRegistry::new(Arc::clone(&vm_arc))),
        filesystem: Arc::new(VzFilesystem::new(Arc::clone(&vm_arc))),
        resource_limiter: Arc::new(VzLimiter::new(Arc::clone(&vm_arc))),
        runtime: Arc::new(VzRuntime::new(Arc::clone(&vm_arc))),
        network_provider: Arc::new(mbx::adapters::NoopNetwork::new()),
        containers_base: containers_dir,
        run_containers_base: run_containers_dir,
        metrics: Arc::new(daemonbox::telemetry::NoOpMetricsRecorder::new()),
        image_loader: Arc::new(VzRegistry::new(Arc::clone(&vm_arc))),
        exec_runtime: None,
        image_pusher: None,
        commit_adapter: None,
        image_builder: None,
        event_sink: Arc::new(minibox_core::events::NoopEventSink),
        event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
        image_gc: vz_image_gc,
        image_store: vz_image_store_ref,
    });

    // ── Socket ───────────────────────────────────────────────────────────
    if socket_path.exists() {
        warn!("vz: removing stale socket at {}", socket_path.display());
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
    }

    let raw_listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("vz: binding Unix socket at {}", socket_path.display()))?;

    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| {
                format!(
                    "vz: setting socket permissions on {}",
                    socket_path.display()
                )
            })?;
    }

    info!("vz: listening on {}", socket_path.display());

    let vm_for_shutdown = Arc::clone(&vm_arc);
    let shutdown = async move {
        tokio::signal::ctrl_c().await.ok();
        info!("vz: received Ctrl-C, shutting down VM");
        vm_for_shutdown.stop();
    };

    daemonbox::server::run_server(
        MacUnixListener(raw_listener),
        state,
        deps,
        false, // require_root_auth — VZ operations run in the VM
        shutdown,
    )
    .await?;

    if socket_path.exists() {
        let _ = std::fs::remove_file(&socket_path);
    }
    info!("miniboxd (macOS/vz) stopped");
    Ok(())
}

#[cfg(all(test, feature = "vz"))]
mod vz_start_tests {
    #[test]
    fn vz_adapter_env_detection() {
        // Structural check — env comparison logic compiles and returns bool.
        let is_vz = std::env::var("MINIBOX_ADAPTER").as_deref() == Ok("vz");
        let _ = is_vz;
    }
}
