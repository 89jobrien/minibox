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

use std::path::Path;
use std::process::Command;

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
    lines.push(format!("     data dir:    {}", active_data_dir().display()));

    lines.join("\n")
}

/// Resolve the active data directory using the same logic as the daemon.
///
/// Checks `MINIBOX_DATA_DIR` first, then falls back to `/var/lib/minibox`
/// for root or `~/.mbx/cache` for non-root users.
fn active_data_dir() -> std::path::PathBuf {
    if let Ok(explicit) = std::env::var("MINIBOX_DATA_DIR") {
        return std::path::PathBuf::from(explicit);
    }
    #[cfg(unix)]
    let is_root = nix::unistd::geteuid().is_root();
    #[cfg(not(unix))]
    let is_root = false;

    if is_root {
        std::path::PathBuf::from("/var/lib/minibox")
    } else {
        std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(".mbx/cache"))
            .unwrap_or_else(|_| std::path::PathBuf::from("/var/lib/minibox"))
    }
}

// ---------------------------------------------------------------------------
// Probe helpers
// ---------------------------------------------------------------------------

/// Check whether the current process is running as UID 0 (root).
///
/// Returns `false` on non-Unix platforms.
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

/// Read and parse the running kernel version from `/proc/version`.
///
/// Returns `(0, 0, 0)` if the file is unreadable or the format is unexpected.
fn probe_kernel_version() -> (u32, u32, u32) {
    let content = match std::fs::read_to_string("/proc/version") {
        Ok(s) => s,
        Err(_) => return (0, 0, 0),
    };
    // "Linux version 6.1.0-18-amd64 ..."
    let version_str = content.split_whitespace().nth(2).unwrap_or("0.0.0");
    parse_kernel_version(version_str)
}

/// Parse a kernel version string like `"6.1.0-18-amd64"` into `(major, minor, patch)`.
///
/// Any non-numeric suffix after the patch component (e.g. `-18-amd64`) is ignored.
/// Individual components that fail to parse are treated as `0`.
fn parse_kernel_version(s: &str) -> (u32, u32, u32) {
    let parts: Vec<&str> = s.split('.').collect();
    let major = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    // Patch may have suffix like "0-18-amd64"; take only the numeric prefix.
    let patch = parts
        .get(2)
        .and_then(|p| p.split('-').next())
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    (major, minor, patch)
}

/// Check whether a cgroup v2 unified hierarchy is mounted by scanning `/proc/mounts`.
///
/// A `cgroup2` entry in `/proc/mounts` means the unified hierarchy is active.
/// Missing or unreadable file returns `false`.
fn probe_cgroups_v2() -> bool {
    std::fs::read_to_string("/proc/mounts")
        .map(|s| s.contains("cgroup2"))
        .unwrap_or(false)
}

/// Read the list of available cgroup v2 controllers from
/// `/sys/fs/cgroup/cgroup.controllers`.
///
/// Returns an empty `Vec` if the file is absent (cgroup v1 or non-Linux host).
/// Typical controllers are `cpu`, `memory`, `io`, `pids`.
fn probe_cgroup_controllers() -> Vec<String> {
    std::fs::read_to_string("/sys/fs/cgroup/cgroup.controllers")
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

/// Check whether `/sys/fs/cgroup/cgroup.subtree_control` exists.
///
/// Presence of this file indicates that the cgroup v2 root supports subtree
/// controller delegation, which minibox requires to create per-container
/// cgroups and assign controllers to them.
fn probe_subtree_delegatable() -> bool {
    Path::new("/sys/fs/cgroup/cgroup.subtree_control").exists()
}

/// Check whether the `overlay` filesystem type is registered with the kernel
/// by scanning `/proc/filesystems`.
///
/// Returns `false` if overlay is not compiled in or not loaded as a module.
fn probe_overlay_fs() -> bool {
    std::fs::read_to_string("/proc/filesystems")
        .map(|s| s.contains("overlay"))
        .unwrap_or(false)
}

/// Check whether `systemctl --version` succeeds, indicating systemd is running.
///
/// Returns `false` if `systemctl` is not in `$PATH` or exits with a non-zero status.
fn probe_systemd_available() -> bool {
    Command::new("systemctl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse the systemd version number from `systemctl --version`.
///
/// Expects output starting with a line like `"systemd 252 (252.22-1~deb12u1)"`.
/// Returns `None` if the command fails or the version cannot be parsed.
fn probe_systemd_version() -> Option<u32> {
    let output = Command::new("systemctl").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // "systemd 252 (252.22-1~deb12u1)"
    stdout
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Check whether `minibox.slice` is active in systemd.
///
/// Uses `systemctl is-active minibox.slice`; returns `false` if systemd is
/// absent or the slice has not been created.
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
