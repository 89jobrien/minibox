//! macbox — macOS orchestration for miniboxd.
//!
//! Provides:
//! - `start()`: entry point called from `miniboxd` on macOS
//! - `paths`: macOS-specific default paths
//! - `preflight`: Colima/backend detection

pub mod paths;
pub mod preflight;

use anyhow::{Context, Result};
use daemonbox::handler::HandlerDependencies;
use daemonbox::state::DaemonState;
use minibox_lib::adapters::{ColimaFilesystem, ColimaLimiter, ColimaRegistry, ColimaRuntime};
use minibox_lib::image::ImageStore;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::UnixListener;
use tracing::{info, warn};

#[derive(thiserror::Error, Debug)]
pub enum MacboxError {
    #[error("no container backend — install Colima (`brew install colima && colima start`)")]
    NoBackendAvailable,
}

/// Unix socket listener for macOS.
struct MacUnixListener(UnixListener);

impl daemonbox::server::ServerListener for MacUnixListener {
    type Stream = tokio::net::UnixStream;

    async fn accept(&self) -> anyhow::Result<(Self::Stream, Option<daemonbox::server::PeerCreds>)> {
        let (stream, _addr) = self.0.accept().await?;
        // On macOS SO_PEERCRED is not available via nix; skip credential check.
        Ok((stream, None))
    }
}

/// Start the macOS daemon.
///
/// Called from `miniboxd`'s macOS `main()`. Sets up the Colima adapter suite,
/// binds a Unix socket, and runs the accept loop.
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

    // ── Dependency Injection — Colima adapter suite ──────────────────────
    let deps = Arc::new(HandlerDependencies {
        registry: Arc::new(ColimaRegistry::new()),
        filesystem: Arc::new(ColimaFilesystem::new()),
        resource_limiter: Arc::new(ColimaLimiter::new()),
        runtime: Arc::new(ColimaRuntime::new()),
        containers_base: containers_dir,
        run_containers_base: run_containers_dir,
    });

    // ── Socket ───────────────────────────────────────────────────────────
    if socket_path.exists() {
        warn!("removing stale socket at {}", socket_path.display());
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("removing stale socket {}", socket_path.display()))?;
    }

    let raw_listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("binding Unix socket at {}", socket_path.display()))?;

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
