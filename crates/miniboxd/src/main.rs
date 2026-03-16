//! miniboxd — container runtime daemon.
//!
//! Listens on a Unix socket and serves JSON-over-newline requests from
//! `minibox` CLI clients.
//!
//! # Adapter suites
//!
//! The daemon supports multiple adapter suites selected via the
//! `MINIBOX_ADAPTER` environment variable:
//!
//! - **native** (default): Linux namespaces, overlay FS, cgroups v2.
//!   Requires root.
//! - **gke**: proot (ptrace), copy-based FS, no-op limiter.
//!   Runs unprivileged in standard GKE pods.
//!
//! # Startup sequence
//! 1. Initialise tracing.
//! 2. Select adapter suite from `MINIBOX_ADAPTER`.
//! 3. Resolve directory paths (configurable via env vars).
//! 4. Create required directories.
//! 5. Remove stale socket file.
//! 6. Bind `UnixListener`.
//! 7. Accept connections, spawning a tokio task per client.
//! 8. Gracefully shut down on SIGTERM / SIGINT.

mod handler;
mod server;
mod state;

use anyhow::{Context, Result};
use handler::HandlerDependencies;
use minibox_lib::adapters::{
    CgroupV2Limiter, DockerHubRegistry, LinuxNamespaceRuntime, OverlayFilesystem,
};
use minibox_lib::adapters::{CopyFilesystem, NoopLimiter, ProotRuntime};
use minibox_lib::image::ImageStore;
use state::DaemonState;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// Default paths (native mode)
// ---------------------------------------------------------------------------

/// Default Unix socket path.
const DEFAULT_SOCKET_PATH: &str = "/run/minibox/miniboxd.sock";
/// Default image store base directory.
const DEFAULT_IMAGES_DIR: &str = "/var/lib/minibox/images";
/// Default container state base directory.
const DEFAULT_CONTAINERS_DIR: &str = "/var/lib/minibox/containers";
/// Default daemon runtime state directory.
const DEFAULT_RUN_DIR: &str = "/run/minibox";

// ---------------------------------------------------------------------------
// Adapter suite selection
// ---------------------------------------------------------------------------

/// Which set of adapters to use for container operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterSuite {
    /// Linux-native: namespaces, overlay FS, cgroups v2. Requires root.
    Native,
    /// GKE unprivileged: proot, copy FS, no-op limiter. No root needed.
    Gke,
}

impl AdapterSuite {
    /// Read `MINIBOX_ADAPTER` env var. Defaults to `Native`.
    fn from_env() -> Result<Self> {
        match std::env::var("MINIBOX_ADAPTER").as_deref() {
            Ok("gke") => Ok(Self::Gke),
            Ok("native") | Err(_) => Ok(Self::Native),
            Ok(other) => anyhow::bail!(
                "unknown MINIBOX_ADAPTER value {:?} (expected \"native\" or \"gke\")",
                other
            ),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // ── Tracing ────────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("miniboxd=info".parse().unwrap()),
        )
        .init();

    info!("miniboxd starting");

    // ── Adapter suite ──────────────────────────────────────────────────────
    let suite = AdapterSuite::from_env()?;
    info!("adapter suite: {:?}", suite);

    // ── Privilege check (native only) ──────────────────────────────────────
    if suite == AdapterSuite::Native && !nix::unistd::getuid().is_root() {
        anyhow::bail!("miniboxd must run as root (native adapter suite)");
    }

    // ── Resolve paths (configurable via env vars for GKE) ─────────────────
    let data_dir = std::env::var("MINIBOX_DATA_DIR")
        .unwrap_or_else(|_| "/var/lib/minibox".to_string());
    let run_dir = std::env::var("MINIBOX_RUN_DIR")
        .unwrap_or_else(|_| DEFAULT_RUN_DIR.to_string());

    let images_dir = format!("{}/images", data_dir);
    let containers_dir = format!("{}/containers", data_dir);
    let run_containers_dir = format!("{}/containers", run_dir);
    let socket_path_str = std::env::var("MINIBOX_SOCKET_PATH")
        .unwrap_or_else(|_| format!("{}/miniboxd.sock", run_dir));

    // ── Directories ────────────────────────────────────────────────────────
    for dir in &[&images_dir, &containers_dir, &run_dir, &run_containers_dir] {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating directory {}", dir))?;
    }

    // ── Shared state ───────────────────────────────────────────────────────
    let image_store = ImageStore::new(&images_dir).context("creating image store")?;
    let state = Arc::new(DaemonState::new(image_store, Path::new(&data_dir)));
    state.load_from_disk().await;
    info!("state loaded from disk");

    // ── Dependency Injection (Composition Root) ───────────────────────────
    // Create concrete adapters implementing domain traits.
    // This is the only place that knows about specific implementations.

    let require_root_auth = suite == AdapterSuite::Native;

    let deps = match suite {
        AdapterSuite::Native => {
            let registry = Arc::new(
                DockerHubRegistry::new(Arc::clone(&state.image_store))
                    .context("creating Docker Hub registry adapter")?,
            );
            Arc::new(HandlerDependencies {
                registry,
                filesystem: Arc::new(OverlayFilesystem::new()),
                resource_limiter: Arc::new(CgroupV2Limiter::new()),
                runtime: Arc::new(LinuxNamespaceRuntime::new()),
                containers_base: PathBuf::from(&containers_dir),
                run_containers_base: PathBuf::from(&run_containers_dir),
            })
        }
        AdapterSuite::Gke => {
            let registry = Arc::new(
                DockerHubRegistry::new(Arc::clone(&state.image_store))
                    .context("creating Docker Hub registry adapter")?,
            );
            let proot_runtime = ProotRuntime::from_env()
                .context("initialising proot runtime for GKE adapter")?;
            Arc::new(HandlerDependencies {
                registry,
                filesystem: Arc::new(CopyFilesystem::new()),
                resource_limiter: Arc::new(NoopLimiter::new()),
                runtime: Arc::new(proot_runtime),
                containers_base: PathBuf::from(&containers_dir),
                run_containers_base: PathBuf::from(&run_containers_dir),
            })
        }
    };

    info!("dependency injection configured");

    // ── Socket ─────────────────────────────────────────────────────────────
    let sock_path = Path::new(&socket_path_str);
    if sock_path.exists() {
        warn!("removing stale socket at {}", socket_path_str);
        std::fs::remove_file(sock_path)
            .with_context(|| format!("removing stale socket {}", socket_path_str))?;
    }

    let listener = UnixListener::bind(sock_path)
        .with_context(|| format!("binding Unix socket at {}", socket_path_str))?;

    // SECURITY: Restrict socket permissions to owner only
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(sock_path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600); // Owner read/write only
        std::fs::set_permissions(sock_path, permissions)
            .context("setting socket permissions to 0600")?;
        info!("socket permissions set to 0600");
    }

    info!("listening on {}", socket_path_str);

    // ── Signal handling ────────────────────────────────────────────────────
    let mut sigterm = signal(SignalKind::terminate()).context("SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("SIGINT handler")?;

    // ── Accept loop ────────────────────────────────────────────────────────
    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _addr)) => {
                        info!("accepted new client connection");
                        let state_clone = Arc::clone(&state);
                        let deps_clone = Arc::clone(&deps);
                        tokio::spawn(async move {
                            if let Err(e) = server::handle_connection(stream, state_clone, deps_clone, require_root_auth).await {
                                // Log but do not crash the daemon on a per-connection error.
                                error!("connection handler error: {:#}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("accept error: {}", e);
                    }
                }
            }

            _ = sigterm.recv() => {
                info!("received SIGTERM, shutting down");
                break;
            }

            _ = sigint.recv() => {
                info!("received SIGINT, shutting down");
                break;
            }
        }
    }

    // ── Cleanup ────────────────────────────────────────────────────────────
    if sock_path.exists() {
        let _ = std::fs::remove_file(sock_path);
    }
    info!("miniboxd stopped");
    Ok(())
}
