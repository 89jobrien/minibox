//! miniboxd — container runtime daemon.
//!
//! Listens on a Unix socket (Linux/macOS) or Named Pipe (Windows) and serves
//! JSON-over-newline requests from `minibox` CLI clients.
//!
//! Platform dispatch:
//! - Linux  → native namespaces, overlay FS, cgroups v2 via this file
//! - macOS  → delegates to `macbox::start()`
//! - Windows → delegates to `winbox::start()`
//!
//! # Adapter suites (Linux)
//!
//! The daemon supports multiple adapter suites selected via the
//! `MINIBOX_ADAPTER` environment variable:
//!
//! - **native** (default): Linux namespaces, overlay FS, cgroups v2.
//!   Requires root.
//! - **gke**: proot (ptrace), copy-based FS, no-op limiter.
//!   Runs unprivileged in standard GKE pods.
//! - **colima**: delegates to Colima/Lima VM via limactl + nerdctl.
//!   No local root required; requires Colima running on the host.
//!
//! # Startup sequence (Linux)
//! 1. Initialise tracing.
//! 2. Select adapter suite from `MINIBOX_ADAPTER`.
//! 3. Resolve directory paths (configurable via env vars).
//! 4. Create required directories.
//! 5. Remove stale socket file.
//! 6. Bind `UnixListener`.
//! 7. Accept connections via `daemonbox::server::run_server`.
//! 8. Gracefully shut down on SIGTERM / SIGINT.

// ── macOS ─────────────────────────────────────────────────────────────────
#[cfg(target_os = "macos")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    macbox::start().await
}

// ── Windows ───────────────────────────────────────────────────────────────
#[cfg(target_os = "windows")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    winbox::start().await
}

// ── Linux ─────────────────────────────────────────────────────────────────
#[cfg(target_os = "linux")]
use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use daemonbox::handler::HandlerDependencies;
#[cfg(target_os = "linux")]
use daemonbox::state::DaemonState;
#[cfg(target_os = "linux")]
use linuxbox::adapters::{
    CgroupV2Limiter, DockerHubRegistry, GhcrRegistry, LinuxNamespaceRuntime, OverlayFilesystem,
};
#[cfg(target_os = "linux")]
use linuxbox::adapters::{ColimaFilesystem, ColimaLimiter, ColimaRegistry, ColimaRuntime};
#[cfg(target_os = "linux")]
use linuxbox::adapters::{CopyFilesystem, NoopLimiter, NoopNetwork, ProotRuntime};
#[cfg(target_os = "linux")]
use minibox_core::image::ImageStore;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use tokio::net::UnixListener;
#[cfg(target_os = "linux")]
use tokio::signal::unix::{SignalKind, signal};
#[cfg(target_os = "linux")]
use tracing::{error, info, warn};

// ── Default paths ─────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
const DEFAULT_RUN_DIR: &str = "/run/minibox";

// ── Adapter suite selection ───────────────────────────────────────────────

/// Which set of adapters to use for container operations.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterSuite {
    /// Linux-native: namespaces, overlay FS, cgroups v2. Requires root.
    Native,
    /// GKE unprivileged: proot, copy FS, no-op limiter. No root needed.
    Gke,
    /// macOS via Colima/Lima: delegates to limactl, nerdctl, chroot in VM.
    Colima,
}

#[cfg(target_os = "linux")]
impl AdapterSuite {
    /// Parse the `MINIBOX_ADAPTER` environment variable into an [`AdapterSuite`].
    ///
    /// Accepted values: `"native"` (default when the variable is absent),
    /// `"gke"`, `"colima"`.  Returns an error for any other value.
    fn from_env() -> Result<Self> {
        match std::env::var("MINIBOX_ADAPTER").as_deref() {
            Ok("gke") => Ok(Self::Gke),
            Ok("colima") => Ok(Self::Colima),
            Ok("native") | Err(_) => Ok(Self::Native),
            Ok(other) => anyhow::bail!(
                "unknown MINIBOX_ADAPTER value {:?} (expected \"native\", \"gke\", or \"colima\")",
                other
            ),
        }
    }
}

/// Resolve the image/container data directory based on effective UID.
///
/// Resolution order:
/// 1. `MINIBOX_DATA_DIR` env var (explicit override)
/// 2. `~/.mbx/cache/` if uid is non-root
/// 3. `/var/lib/minibox/` if uid is root
#[cfg(target_os = "linux")]
fn resolve_data_dir_for_uid(uid: u32) -> std::path::PathBuf {
    if let Ok(explicit) = std::env::var("MINIBOX_DATA_DIR") {
        return std::path::PathBuf::from(explicit);
    }
    if uid == 0 {
        std::path::PathBuf::from("/var/lib/minibox")
    } else {
        std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(".mbx/cache"))
            .unwrap_or_else(|_| std::path::PathBuf::from("/var/lib/minibox"))
    }
}

/// Move the current daemon process into a `supervisor` leaf cgroup.
///
/// cgroups v2 enforces the "no internal process" rule: a cgroup that has child
/// cgroups cannot also contain processes directly.  When miniboxd runs as a
/// systemd service under `minibox.slice/miniboxd.service`, it starts in that
/// cgroup.  Creating per-container child cgroups underneath it would violate
/// the rule.
///
/// This function reads `/proc/self/cgroup` to find the current v2 cgroup path,
/// creates a `supervisor/` subdirectory within it, and writes the daemon PID to
/// `cgroup.procs` to migrate it into the leaf.  Container cgroups are created
/// as siblings of `supervisor/`, not children of it.
///
/// Failures are logged as warnings and do not abort startup.
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

// ── UnixServerListener ────────────────────────────────────────────────────

/// Wraps a Tokio [`UnixListener`] and implements [`daemonbox::server::ServerListener`].
///
/// On `accept()`, peer credentials are read via `SO_PEERCRED` (using the `nix`
/// crate's `PeerCredentials` socket option) and returned alongside the stream.
/// The UID and PID are used by `run_server` to enforce root-only access when
/// `require_root_auth` is `true`.
#[cfg(target_os = "linux")]
struct UnixServerListener(UnixListener);

#[cfg(target_os = "linux")]
impl daemonbox::server::ServerListener for UnixServerListener {
    type Stream = tokio::net::UnixStream;

    async fn accept(&self) -> anyhow::Result<(Self::Stream, Option<daemonbox::server::PeerCreds>)> {
        let (stream, _addr) = self.0.accept().await?;
        let creds = {
            use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
            use std::os::unix::io::AsFd;
            getsockopt(&stream.as_fd(), PeerCredentials).ok().map(|c| {
                daemonbox::server::PeerCreds {
                    uid: c.uid(),
                    pid: c.pid(),
                }
            })
        };
        Ok((stream, creds))
    }
}

// ── Linux main ────────────────────────────────────────────────────────────

/// Linux daemon entry point.
///
/// Performs the full startup sequence documented in the crate-level `//!` doc:
/// tracing init → adapter selection → privilege check → cgroup self-migration →
/// path resolution → directory creation → state load → dependency injection →
/// socket bind → signal handler setup → accept loop.
///
/// Cleans up the socket file on exit.  Individual container cleanup (overlay
/// unmount, cgroup removal) is not performed on daemon shutdown; a subsequent
/// `minibox rm` or `cargo xtask nuke-test-state` is needed to reclaim resources
/// from containers that were running when the daemon exited.
#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<()> {
    // ── Tracing ──────────────────────────────────────────────────────────
    let otlp_endpoint = std::env::var("MINIBOX_OTLP_ENDPOINT").ok();
    let _otel_guard = daemonbox::telemetry::traces::init_tracing(otlp_endpoint.as_deref());

    info!("miniboxd starting");

    // ── Adapter suite ────────────────────────────────────────────────────
    let suite = AdapterSuite::from_env()?;
    info!("adapter suite: {suite:?}");

    // ── Privilege check (native only) ────────────────────────────────────
    if suite == AdapterSuite::Native && !nix::unistd::getuid().is_root() {
        anyhow::bail!("miniboxd must run as root (native adapter suite)");
    }

    // ── Cgroup self-migration (native only) ──────────────────────────────
    if suite == AdapterSuite::Native {
        migrate_to_supervisor_cgroup();
    }

    // ── Resolve paths (configurable via env vars) ───────────────────────
    let uid = nix::unistd::getuid().as_raw();
    let data_dir = resolve_data_dir_for_uid(uid);
    let run_dir = std::env::var("MINIBOX_RUN_DIR").unwrap_or_else(|_| DEFAULT_RUN_DIR.to_string());

    let images_dir = data_dir.join("images");
    let containers_dir = data_dir.join("containers");
    let run_containers_dir = format!("{run_dir}/containers");
    let socket_path_str =
        std::env::var("MINIBOX_SOCKET_PATH").unwrap_or_else(|_| format!("{run_dir}/miniboxd.sock"));

    // ── Directories ──────────────────────────────────────────────────────
    // Explicit 0700 keeps extracted layer/rootfs contents private on shared
    // hosts (matters for the per-user ~/.mbx/cache path; harmless for the
    // root-owned /var/lib/minibox default).
    {
        use std::os::unix::fs::DirBuilderExt;
        for dir in &[
            images_dir.as_path(),
            containers_dir.as_path(),
            Path::new(&run_dir),
            Path::new(&run_containers_dir),
        ] {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .with_context(|| format!("creating directory {}", dir.display()))?;
        }
    }

    // ── Shared state ─────────────────────────────────────────────────────
    let image_store = ImageStore::new(&images_dir).context("creating image store")?;
    let state = Arc::new(DaemonState::new(image_store, &data_dir));
    state.load_from_disk().await;
    info!("state loaded from disk");

    // ── Metrics ─────────────────────────────────────────────────────────
    let metrics_addr: std::net::SocketAddr = std::env::var("MINIBOX_METRICS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9090".to_string())
        .parse()
        .context("parsing MINIBOX_METRICS_ADDR")?;

    let metrics_recorder = Arc::new(daemonbox::telemetry::PrometheusMetricsRecorder::new());

    let (_metrics_addr, _metrics_handle) =
        daemonbox::telemetry::server::run_metrics_server(metrics_addr, metrics_recorder.clone())
            .await
            .context("starting metrics server")?;
    info!(addr = %_metrics_addr, "metrics server listening");

    // ── Dependency Injection ─────────────────────────────────────────────
    let require_root_auth = suite == AdapterSuite::Native;

    let deps = match suite {
        AdapterSuite::Native => {
            let registry = Arc::new(
                DockerHubRegistry::new(Arc::clone(&state.image_store))
                    .context("creating Docker Hub registry adapter")?,
            );
            let ghcr_registry = Arc::new(
                GhcrRegistry::new(Arc::clone(&state.image_store))
                    .context("creating GHCR registry adapter")?,
            );
            Arc::new(HandlerDependencies {
                registry,
                ghcr_registry,
                filesystem: Arc::new(OverlayFilesystem::new()),
                resource_limiter: Arc::new(CgroupV2Limiter::new()),
                runtime: Arc::new(LinuxNamespaceRuntime::new()),
                network_provider: Arc::new(NoopNetwork::new()),
                containers_base: containers_dir.clone(),
                run_containers_base: PathBuf::from(&run_containers_dir),
                metrics: metrics_recorder.clone(),
            })
        }
        AdapterSuite::Gke => {
            let registry = Arc::new(
                DockerHubRegistry::new(Arc::clone(&state.image_store))
                    .context("creating Docker Hub registry adapter")?,
            );
            let ghcr_registry = Arc::new(
                GhcrRegistry::new(Arc::clone(&state.image_store))
                    .context("creating GHCR registry adapter")?,
            );
            let proot_runtime =
                ProotRuntime::from_env().context("initialising proot runtime for GKE adapter")?;
            Arc::new(HandlerDependencies {
                registry,
                ghcr_registry,
                filesystem: Arc::new(CopyFilesystem::new()),
                resource_limiter: Arc::new(NoopLimiter::new()),
                runtime: Arc::new(proot_runtime),
                network_provider: Arc::new(NoopNetwork::new()),
                containers_base: containers_dir.clone(),
                run_containers_base: PathBuf::from(&run_containers_dir),
                metrics: metrics_recorder.clone(),
            })
        }
        AdapterSuite::Colima => {
            let ghcr_registry = Arc::new(
                GhcrRegistry::new(Arc::clone(&state.image_store))
                    .context("creating GHCR registry adapter")?,
            );
            Arc::new(HandlerDependencies {
                registry: Arc::new(ColimaRegistry::new()),
                ghcr_registry,
                filesystem: Arc::new(ColimaFilesystem::new()),
                resource_limiter: Arc::new(ColimaLimiter::new()),
                runtime: Arc::new(ColimaRuntime::new()),
                network_provider: Arc::new(NoopNetwork::new()),
                containers_base: containers_dir.clone(),
                run_containers_base: PathBuf::from(&run_containers_dir),
                metrics: metrics_recorder.clone(),
            })
        }
    };

    info!("dependency injection configured");

    // ── Socket ───────────────────────────────────────────────────────────
    let sock_path = Path::new(&socket_path_str);
    if sock_path.exists() {
        warn!("removing stale socket at {socket_path_str}");
        std::fs::remove_file(sock_path)
            .with_context(|| format!("removing stale socket {socket_path_str}"))?;
    }

    let raw_listener = UnixListener::bind(sock_path)
        .with_context(|| format!("binding Unix socket at {socket_path_str}"))?;

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

    info!("listening on {socket_path_str}");

    // ── Signal handling ──────────────────────────────────────────────────
    let mut sigterm = signal(SignalKind::terminate()).context("SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("SIGINT handler")?;
    let shutdown = async move {
        tokio::select! {
            _ = sigterm.recv() => { info!("received SIGTERM, shutting down"); }
            _ = sigint.recv()  => { info!("received SIGINT, shutting down");  }
        }
    };

    let listener = UnixServerListener(raw_listener);

    // ── Accept loop via run_server ────────────────────────────────────────
    daemonbox::server::run_server(listener, state, deps, require_root_auth, shutdown).await?;

    // ── Cleanup ──────────────────────────────────────────────────────────
    if sock_path.exists() {
        let _ = std::fs::remove_file(sock_path);
    }
    info!("miniboxd stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::*;
    #[cfg(target_os = "linux")]
    use std::path::PathBuf;

    // SAFETY: env var mutations are serialised with ENV_LOCK
    #[cfg(target_os = "linux")]
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    #[cfg(target_os = "linux")]
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
        assert_eq!(dir, PathBuf::from("/home/testuser/.mbx/cache"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn resolve_data_dir_root_uses_var_lib() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::remove_var("MINIBOX_DATA_DIR");
        }
        let dir = resolve_data_dir_for_uid(0);
        assert_eq!(dir, PathBuf::from("/var/lib/minibox"));
    }

    #[test]
    #[cfg(target_os = "linux")]
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
}
