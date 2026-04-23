//! Host capability probing for test infrastructure and diagnostics.
//!
//! Probes the current host for features needed by minibox: cgroups v2,
//! overlay filesystem, kernel version, systemd status. Pure reads, no
//! mutations. Infallible — missing data yields false/empty.
//!
//! Used by:
//! - Integration and e2e tests to skip tests gracefully
//! - `just doctor` to report host readiness
//! - Future `minibox doctor` CLI subcommand

use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// Startup diagnostic paths
// ---------------------------------------------------------------------------

/// Resolved runtime paths logged at daemon startup for observability.
///
/// All three fields reflect the same env-var resolution order used by
/// `miniboxd` and `minibox-client`:
///
/// 1. `MINIBOX_SOCKET_PATH` / `MINIBOX_DATA_DIR` / `MINIBOX_CGROUP_ROOT` (full path)
/// 2. `MINIBOX_RUN_DIR` / `MINIBOX_DATA_DIR` directory hints
/// 3. Platform default
#[derive(Debug, Clone)]
pub struct DiagnosticPaths {
    /// Resolved Unix socket path (`miniboxd.sock`).
    pub socket_path: PathBuf,
    /// Resolved data directory for images and container state.
    pub data_dir: PathBuf,
    /// Resolved cgroup v2 root (typically `/sys/fs/cgroup`).
    pub cgroup_root: PathBuf,
}

impl DiagnosticPaths {
    /// Return a structured, human-readable log string with `key = value` pairs.
    ///
    /// Intended for `tracing::info!` at daemon startup. Keys are `socket_path`,
    /// `data_dir`, and `cgroup_root`.
    pub fn to_log_string(&self) -> String {
        format!(
            "socket_path = {} | data_dir = {} | cgroup_root = {}",
            self.socket_path.display(),
            self.data_dir.display(),
            self.cgroup_root.display(),
        )
    }
}

/// Resolve the runtime paths miniboxd will use, applying the same env-var
/// precedence order as the daemon and client library.
///
/// Pure — reads env vars, does not mutate the filesystem.
///
/// Intended to be called once at daemon startup:
///
/// ```rust,ignore
/// let paths = minibox_core::preflight::resolve_diagnostic_paths();
/// tracing::info!(
///     socket_path = %paths.socket_path.display(),
///     data_dir    = %paths.data_dir.display(),
///     cgroup_root = %paths.cgroup_root.display(),
///     "daemon: resolved runtime paths"
/// );
/// ```
pub fn resolve_diagnostic_paths() -> DiagnosticPaths {
    DiagnosticPaths {
        socket_path: resolve_socket_path(),
        data_dir: resolve_data_dir(),
        cgroup_root: resolve_cgroup_root(),
    }
}

/// Resolve daemon socket path.
///
/// Precedence: `MINIBOX_SOCKET_PATH` > `$MINIBOX_RUN_DIR/miniboxd.sock` > platform default.
fn resolve_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("MINIBOX_SOCKET_PATH") {
        return PathBuf::from(p);
    }
    if let Ok(dir) = std::env::var("MINIBOX_RUN_DIR") {
        return PathBuf::from(dir).join("miniboxd.sock");
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/tmp/minibox/miniboxd.sock")
    }
    #[cfg(not(target_os = "macos"))]
    {
        PathBuf::from("/run/minibox/miniboxd.sock")
    }
}

/// Resolve data directory.
///
/// Precedence: `MINIBOX_DATA_DIR` > platform default.
fn resolve_data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("MINIBOX_DATA_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".local/share/minibox")
    }
    #[cfg(not(target_os = "macos"))]
    {
        PathBuf::from("/var/lib/minibox")
    }
}

/// Resolve cgroup v2 root.
///
/// Precedence: `MINIBOX_CGROUP_ROOT` > `/sys/fs/cgroup`.
fn resolve_cgroup_root() -> PathBuf {
    if let Ok(r) = std::env::var("MINIBOX_CGROUP_ROOT") {
        return PathBuf::from(r);
    }
    PathBuf::from("/sys/fs/cgroup")
}

// ---------------------------------------------------------------------------
// Host capabilities
// ---------------------------------------------------------------------------

/// Host capabilities relevant to minibox operation.
#[derive(Debug, Clone)]
pub struct HostCapabilities {
    /// Running as UID 0.
    pub is_root: bool,
    /// Kernel version as (major, minor, patch).
    pub kernel_version: (u32, u32, u32),
    /// cgroup2 filesystem mounted (typically at /sys/fs/cgroup).
    pub cgroups_v2: bool,
    /// Controllers listed in /sys/fs/cgroup/cgroup.controllers.
    pub cgroup_controllers: Vec<String>,
    /// Can write to cgroup.subtree_control (delegation works).
    pub cgroup_subtree_delegatable: bool,
    /// "overlay" listed in /proc/filesystems.
    pub overlay_fs: bool,
    /// systemctl binary exists and responds.
    pub systemd_available: bool,
    /// Parsed from `systemctl --version` (e.g., 252).
    pub systemd_version: Option<u32>,
    /// minibox.slice is loaded in systemd.
    pub minibox_slice_active: bool,
}

/// Probe the current host for minibox-relevant capabilities.
///
/// This function never fails — it returns false/empty for anything it
/// cannot determine. Safe to call on any platform.
pub fn probe() -> HostCapabilities {
    HostCapabilities {
        is_root: probe_root(),
        kernel_version: probe_kernel_version(),
        cgroups_v2: probe_cgroups_v2(),
        cgroup_controllers: probe_cgroup_controllers(),
        cgroup_subtree_delegatable: probe_subtree_delegatable(),
        overlay_fs: probe_overlay_fs(),
        systemd_available: probe_systemd_available(),
        systemd_version: probe_systemd_version(),
        minibox_slice_active: probe_minibox_slice(),
    }
}

/// Format a human-readable capability report suitable for `just doctor` output.
///
/// Each capability is prefixed with `PASS`, `WARN`, or `FAIL` depending on
/// whether it meets the requirement for running minibox containers.
pub fn format_report(caps: &HostCapabilities) -> String {
    let mut lines = Vec::new();
    lines.push("Minibox Host Capabilities".to_string());
    lines.push("=".repeat(40));

    let (maj, min, patch) = caps.kernel_version;
    lines.push(format!(
        "{} Kernel: {}.{}.{}",
        if maj >= 5 { "PASS" } else { "WARN" },
        maj,
        min,
        patch
    ));
    lines.push(format!(
        "{} Root: {}",
        if caps.is_root { "PASS" } else { "FAIL" },
        caps.is_root
    ));
    lines.push(format!(
        "{} cgroups v2: {}",
        if caps.cgroups_v2 { "PASS" } else { "FAIL" },
        caps.cgroups_v2
    ));
    lines.push(format!(
        "     Controllers: [{}]",
        caps.cgroup_controllers.join(", ")
    ));
    lines.push(format!(
        "{} Subtree delegation: {}",
        if caps.cgroup_subtree_delegatable {
            "PASS"
        } else {
            "WARN"
        },
        caps.cgroup_subtree_delegatable
    ));
    lines.push(format!(
        "{} Overlay FS: {}",
        if caps.overlay_fs { "PASS" } else { "FAIL" },
        caps.overlay_fs
    ));
    lines.push(format!(
        "     systemd: {} (version: {})",
        caps.systemd_available,
        caps.systemd_version
            .map(|v| v.to_string())
            .unwrap_or_else(|| "N/A".to_string())
    ));
    lines.push(format!("     minibox.slice: {}", caps.minibox_slice_active));

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Probe helpers
// ---------------------------------------------------------------------------

fn probe_root() -> bool {
    #[cfg(unix)]
    {
        nix::unistd::geteuid().is_root()
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn probe_kernel_version() -> (u32, u32, u32) {
    let content = match std::fs::read_to_string("/proc/version") {
        Ok(s) => s,
        Err(_) => return (0, 0, 0),
    };
    let version_str = content.split_whitespace().nth(2).unwrap_or("0.0.0");
    parse_kernel_version(version_str)
}

/// Parse a kernel version string like `"6.1.0-18-amd64"` into `(major, minor, patch)`.
fn parse_kernel_version(s: &str) -> (u32, u32, u32) {
    let parts: Vec<&str> = s.split('.').collect();
    let major = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let patch = parts
        .get(2)
        .and_then(|p| p.split('-').next())
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    (major, minor, patch)
}

fn probe_cgroups_v2() -> bool {
    std::fs::read_to_string("/proc/mounts")
        .map(|s| s.contains("cgroup2"))
        .unwrap_or(false)
}

fn probe_cgroup_controllers() -> Vec<String> {
    std::fs::read_to_string("/sys/fs/cgroup/cgroup.controllers")
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

fn probe_subtree_delegatable() -> bool {
    Path::new("/sys/fs/cgroup/cgroup.subtree_control").exists()
}

fn probe_overlay_fs() -> bool {
    std::fs::read_to_string("/proc/filesystems")
        .map(|s| s.contains("overlay"))
        .unwrap_or(false)
}

fn probe_systemd_available() -> bool {
    Command::new("systemctl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn probe_systemd_version() -> Option<u32> {
    let output = Command::new("systemctl").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn probe_minibox_slice() -> bool {
    Command::new("systemctl")
        .args(["is-active", "minibox.slice"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_paths_socket_default_linux() {
        // Default socket filename must be miniboxd.sock, not the old minibox.sock.
        let paths = resolve_diagnostic_paths();
        let socket = paths.socket_path.to_string_lossy();
        assert!(
            socket.ends_with("miniboxd.sock"),
            "socket path must end with miniboxd.sock, got: {socket}"
        );
    }

    #[test]
    fn diagnostic_paths_socket_env_override() {
        // SAFETY: test-only env mutation.
        unsafe {
            std::env::set_var("MINIBOX_SOCKET_PATH", "/tmp/test/custom.sock");
        }
        let paths = resolve_diagnostic_paths();
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
        }
        assert_eq!(paths.socket_path.to_string_lossy(), "/tmp/test/custom.sock");
    }

    #[test]
    fn diagnostic_paths_run_dir_env_override() {
        // MINIBOX_RUN_DIR: socket becomes <run_dir>/miniboxd.sock.
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::set_var("MINIBOX_RUN_DIR", "/tmp/run-test");
        }
        let paths = resolve_diagnostic_paths();
        unsafe {
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        assert_eq!(
            paths.socket_path.to_string_lossy(),
            "/tmp/run-test/miniboxd.sock"
        );
    }

    #[test]
    fn diagnostic_paths_log_output_is_structured() {
        let paths = resolve_diagnostic_paths();
        let output = paths.to_log_string();
        assert!(output.contains("socket_path"));
        assert!(output.contains("data_dir"));
        assert!(output.contains("cgroup_root"));
    }

    #[test]
    fn test_parse_kernel_version() {
        assert_eq!(parse_kernel_version("6.1.0-18-amd64"), (6, 1, 0));
        assert_eq!(parse_kernel_version("5.15.0"), (5, 15, 0));
        assert_eq!(parse_kernel_version("4.19.128"), (4, 19, 128));
        assert_eq!(parse_kernel_version("garbage"), (0, 0, 0));
        assert_eq!(parse_kernel_version(""), (0, 0, 0));
    }

    #[test]
    fn test_probe_does_not_panic() {
        let caps = probe();
        let _ = format!("{caps:?}");
    }

    #[test]
    fn test_format_report_does_not_panic() {
        let caps = probe();
        let report = format_report(&caps);
        assert!(report.contains("Minibox Host Capabilities"));
    }
}
