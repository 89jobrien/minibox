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

pub mod krun;
pub mod paths;
pub mod preflight;

use anyhow::{Context, Result};
use krun::filesystem::KrunFilesystem;
use krun::limiter::KrunLimiter;
use krun::registry::KrunRegistry;
use krun::runtime::KrunRuntime;
use minibox::adapters::{
    ColimaFilesystem, ColimaLimiter, ColimaRegistry, ColimaRuntime, LimaExecutor, LimaSpawner,
    NoopNetwork,
};
use minibox::daemon::handler::HandlerDependencies;
use minibox::daemon::state::DaemonState;
use minibox_core::adapters::HostnameRegistryRouter;
use minibox_core::domain::{DynImageLoader, DynImageRegistry};
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

#[allow(clippy::too_many_arguments)]
pub fn build_colima_handler_dependencies(
    state: Arc<DaemonState>,
    data_dir: PathBuf,
    containers_dir: PathBuf,
    run_containers_dir: PathBuf,
    image_gc: Arc<dyn ImageGarbageCollector>,
    executor: LimaExecutor,
    spawner: LimaSpawner,
) -> Result<Arc<HandlerDependencies>> {
    let registry = Arc::new(ColimaRegistry::new().with_executor(executor.clone()));
    let registry_port: DynImageRegistry = registry.clone();
    let image_loader: DynImageLoader = registry.clone();
    let commit_adapter = minibox::adapters::commit::overlay_commit_adapter(
        Arc::clone(&state.image_store),
        Arc::clone(&state) as minibox::container_state::StateHandle,
    );
    let registry_router = Arc::new(HostnameRegistryRouter::new(
        registry_port,
        std::iter::empty::<(&str, DynImageRegistry)>(),
    ));
    let filesystem = Arc::new(ColimaFilesystem::new());
    let runtime = Arc::new(
        ColimaRuntime::new()
            .with_executor(executor.clone())
            .with_spawner(spawner),
    );
    let image_builder = minibox::adapters::builder::minibox_image_builder(
        Arc::clone(&state.image_store),
        data_dir.clone(),
        Arc::clone(&filesystem) as minibox_core::domain::DynFilesystemProvider,
        Arc::clone(&runtime) as minibox_core::domain::DynContainerRuntime,
        Arc::clone(&registry_router) as minibox_core::domain::DynRegistryRouter,
    );
    let image_pusher = minibox::adapters::colima_image_pusher(
        Arc::clone(&state.image_store),
        Arc::clone(&image_loader),
        data_dir.join("exports"),
        executor.clone(),
    );

    Ok(Arc::new(HandlerDependencies {
        image: minibox::daemon::handler::ImageDeps {
            // Colima's nerdctl handles any registry (ghcr.io included) via the same adapter,
            // so we use it as the default with no hostname overrides.
            registry_router,
            image_loader,
            image_gc,
            image_store: Arc::clone(&state.image_store),
        },
        lifecycle: minibox::daemon::handler::LifecycleDeps {
            filesystem,
            resource_limiter: Arc::new(ColimaLimiter::new().with_executor(executor.clone())),
            runtime,
            network_provider: Arc::new(NoopNetwork::new()),
            containers_base: containers_dir,
            run_containers_base: run_containers_dir,
        },
        exec: minibox::daemon::handler::ExecDeps {
            exec_runtime: None,
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                minibox::daemon::handler::PtySessionRegistry::default(),
            )),
        },
        build: minibox::daemon::handler::BuildDeps {
            image_pusher: Some(image_pusher),
            commit_adapter: Some(commit_adapter),
            image_builder: Some(image_builder),
        },
        events: minibox::daemon::handler::EventDeps {
            event_sink: Arc::new(minibox_core::events::NoopEventSink),
            event_source: Arc::new(minibox_core::events::BroadcastEventBroker::new()),
            metrics: Arc::new(minibox::daemon::telemetry::NoOpMetricsRecorder::new()),
        },
        policy: minibox::daemon::handler::ContainerPolicy::default(),
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    }))
}

/// Newtype wrapper around [`tokio::net::UnixListener`] that implements
/// [`minibox::daemon::server::ServerListener`] for the macOS daemon.
///
/// On macOS, `SO_PEERCRED` is not available through the `nix` crate, so
/// peer credential checking is skipped (the `accept` implementation returns
/// `None` for `PeerCreds`). This means the UID-based root-auth guard is
/// disabled on macOS — container operations are delegated to the Colima VM
/// anyway, so the attack surface is limited to whoever can reach the socket.
/// Extract peer credentials from a connected Unix socket fd.
///
/// On macOS, uses `getpeereid(2)` (pid unavailable, returns 0 sentinel).
/// On Linux, uses `SO_PEERCRED` via `getsockopt`.
#[cfg(target_os = "macos")]
fn get_peer_creds(fd: std::os::unix::io::RawFd) -> Option<minibox::daemon::server::PeerCreds> {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: fd is a valid connected Unix socket fd. getpeereid is safe to
    // call on any connected Unix domain socket.
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } == 0 {
        Some(minibox::daemon::server::PeerCreds { uid, pid: 0 })
    } else {
        tracing::warn!("getpeereid failed: {}", std::io::Error::last_os_error());
        None
    }
}

#[cfg(target_os = "linux")]
fn get_peer_creds(fd: std::os::unix::io::RawFd) -> Option<minibox::daemon::server::PeerCreds> {
    use std::mem;
    let mut cred: libc::ucred = unsafe { mem::zeroed() };
    let mut len = mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: fd is a valid connected Unix socket fd. getsockopt with
    // SO_PEERCRED is safe on any connected Unix domain socket.
    let ret = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if ret == 0 {
        Some(minibox::daemon::server::PeerCreds {
            uid: cred.uid,
            pid: cred.pid,
        })
    } else {
        tracing::warn!("SO_PEERCRED failed: {}", std::io::Error::last_os_error());
        None
    }
}

struct MacUnixListener(UnixListener);

impl minibox::daemon::server::ServerListener for MacUnixListener {
    type Stream = tokio::net::UnixStream;

    async fn accept(
        &self,
    ) -> anyhow::Result<(Self::Stream, Option<minibox::daemon::server::PeerCreds>)> {
        let (stream, _addr) = self.0.accept().await?;
        use std::os::unix::io::AsRawFd;
        let creds = get_peer_creds(stream.as_raw_fd());
        Ok((stream, creds))
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
///    [`minibox::daemon::server::run_server`] accept loop with root-auth disabled.
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

    // ── krun branch ──────────────────────────────────────────────────────
    if std::env::var("MINIBOX_ADAPTER").as_deref() == Ok("krun") {
        return start_krun(
            socket_path,
            images_dir,
            containers_dir,
            run_containers_dir,
            state,
            image_gc,
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

    let deps = build_colima_handler_dependencies(
        Arc::clone(&state),
        data_dir.clone(),
        containers_dir,
        run_containers_dir,
        Arc::clone(&image_gc),
        executor,
        spawner,
    )?;

    // ── Socket ───────────────────────────────────────────────────────────
    if socket_path.exists() {
        warn!(path = %socket_path.display(), "socket: removing stale socket");
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
    minibox::daemon::server::run_server(
        MacUnixListener(raw_listener),
        state,
        deps,
        false, // require_root_auth
        shutdown,
    )
    .await?;

    if let Err(e) = std::fs::remove_file(&socket_path) {
        warn!(error = %e, path = %socket_path.display(), "socket: cleanup on shutdown failed");
    }
    info!("miniboxd (macOS) stopped");
    Ok(())
}

/// Start the macOS daemon using the krun/smolvm adapter suite.
///
/// Selected when `MINIBOX_ADAPTER=krun` is set. Wires `KrunRegistry`,
/// `KrunFilesystem`, `KrunLimiter`, and `KrunRuntime` into
/// [`HandlerDependencies`] and runs the standard socket server accept loop.
///
/// The krun backend delegates container execution to `smolvm` (a thin
/// wrapper over libkrun) rather than Linux namespaces or Colima.
async fn start_krun(
    socket_path: std::path::PathBuf,
    _images_dir: std::path::PathBuf,
    containers_dir: std::path::PathBuf,
    run_containers_dir: std::path::PathBuf,
    state: Arc<DaemonState>,
    image_gc: Arc<dyn ImageGarbageCollector>,
) -> Result<()> {
    info!("miniboxd (macOS/krun) starting");

    let registry = Arc::new(
        KrunRegistry::new(Arc::clone(&state.image_store))
            .context("krun: creating registry adapter")?,
    );
    let registry_port: DynImageRegistry = registry;

    let deps = Arc::new(HandlerDependencies {
        image: minibox::daemon::handler::ImageDeps {
            registry_router: Arc::new(HostnameRegistryRouter::new(
                registry_port,
                std::iter::empty::<(&str, DynImageRegistry)>(),
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
            pty_sessions: std::sync::Arc::new(tokio::sync::Mutex::new(
                minibox::daemon::handler::PtySessionRegistry::default(),
            )),
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
        policy: minibox::daemon::handler::ContainerPolicy::default(),
        checkpoint: std::sync::Arc::new(minibox_core::domain::NoopVmCheckpoint),
    });

    // ── Socket ───────────────────────────────────────────────────────────
    if socket_path.exists() {
        warn!(path = %socket_path.display(), "socket: removing stale socket");
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
    }

    let raw_listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("krun: binding Unix socket at {}", socket_path.display()))?;

    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| {
                format!(
                    "krun: setting socket permissions on {}",
                    socket_path.display()
                )
            })?;
    }

    info!("krun: listening on {}", socket_path.display());

    let shutdown = async {
        tokio::signal::ctrl_c().await.ok();
        info!("krun: received Ctrl-C, shutting down");
    };

    minibox::daemon::server::run_server(
        MacUnixListener(raw_listener),
        state,
        deps,
        false, // require_root_auth — krun operations run in the VM
        shutdown,
    )
    .await?;

    if let Err(e) = std::fs::remove_file(&socket_path) {
        warn!(error = %e, path = %socket_path.display(), "socket: cleanup on shutdown failed");
    }
    info!("miniboxd (macOS/krun) stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::image::gc::ImageGc;
    use tempfile::TempDir;

    #[tokio::test]
    async fn colima_dependencies_wire_local_commit_build_and_push_adapters() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let containers_dir = data_dir.join("containers");
        let run_containers_dir = tmp.path().join("run").join("containers");
        std::fs::create_dir_all(&containers_dir).unwrap();
        std::fs::create_dir_all(&run_containers_dir).unwrap();

        let image_store = ImageStore::new(data_dir.join("images")).expect("image store");
        let state = Arc::new(DaemonState::new(image_store, &data_dir));
        let lease_service = Arc::new(
            DiskLeaseService::new(data_dir.join("leases.json"))
                .await
                .expect("lease service"),
        );
        let image_gc: Arc<dyn ImageGarbageCollector> =
            Arc::new(ImageGc::new(Arc::clone(&state.image_store), lease_service));

        let executor: LimaExecutor = Arc::new(|_args: &[&str]| Ok(String::new()));
        let spawner: LimaSpawner = Arc::new(|_args: &[&str]| {
            Err(anyhow::anyhow!("spawner should not run in structural test"))
        });

        let deps = build_colima_handler_dependencies(
            Arc::clone(&state),
            data_dir,
            containers_dir,
            run_containers_dir,
            image_gc,
            executor,
            spawner,
        )
        .expect("colima deps");

        assert!(
            deps.build.commit_adapter.is_some(),
            "commit adapter should be wired"
        );
        assert!(
            deps.build.image_builder.is_some(),
            "image builder should be wired"
        );
        assert!(
            deps.build.image_pusher.is_some(),
            "image pusher should be wired"
        );
    }
}
