//! miniboxd — container runtime daemon.
//!
//! Listens on `/run/minibox/miniboxd.sock` and serves JSON-over-newline
//! requests from `minibox` CLI clients.  Must run as root.
//!
//! # Startup sequence
//! 1. Initialise tracing.
//! 2. Create required directories.
//! 3. Remove stale socket file.
//! 4. Bind `UnixListener`.
//! 5. Accept connections, spawning a tokio task per client.
//! 6. Gracefully shut down on SIGTERM / SIGINT.

mod handler;
mod server;
mod state;

use anyhow::{Context, Result};
use minibox_lib::image::ImageStore;
use state::DaemonState;
use std::path::Path;
use std::sync::Arc;
use tokio::net::UnixListener;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, info, warn};

/// Unix socket path.
const SOCKET_PATH: &str = "/run/minibox/miniboxd.sock";
/// Image store base directory.
const IMAGES_DIR: &str = "/var/lib/minibox/images";
/// Container state base directory.
const CONTAINERS_DIR: &str = "/var/lib/minibox/containers";
/// Daemon runtime state directory.
const RUN_DIR: &str = "/run/minibox";

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

    // ── Privilege check ────────────────────────────────────────────────────
    if !nix::unistd::getuid().is_root() {
        anyhow::bail!("miniboxd must run as root");
    }

    // ── Directories ────────────────────────────────────────────────────────
    for dir in &[IMAGES_DIR, CONTAINERS_DIR, RUN_DIR] {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating directory {}", dir))?;
    }
    // Runtime per-container directory
    std::fs::create_dir_all("/run/minibox/containers")
        .context("creating /run/minibox/containers")?;

    // ── Shared state ───────────────────────────────────────────────────────
    let image_store = ImageStore::new(IMAGES_DIR).context("creating image store")?;
    let state = Arc::new(DaemonState::new(image_store));

    // ── Socket ─────────────────────────────────────────────────────────────
    let sock_path = Path::new(SOCKET_PATH);
    if sock_path.exists() {
        warn!("removing stale socket at {}", SOCKET_PATH);
        std::fs::remove_file(sock_path)
            .with_context(|| format!("removing stale socket {}", SOCKET_PATH))?;
    }

    let listener = UnixListener::bind(sock_path)
        .with_context(|| format!("binding Unix socket at {}", SOCKET_PATH))?;

    // SECURITY: Restrict socket permissions to owner (root) only
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(sock_path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600); // Owner read/write only
        std::fs::set_permissions(sock_path, permissions)
            .context("setting socket permissions to 0600")?;
        info!("socket permissions set to 0600 (root only)");
    }

    info!("listening on {}", SOCKET_PATH);

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
                        tokio::spawn(async move {
                            if let Err(e) = server::handle_connection(stream, state_clone).await {
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
