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

    lines.join("\n")
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

// ---------------------------------------------------------------------------
// Workspace-aware helpers
// ---------------------------------------------------------------------------

/// Result from probing whether a command can run from a workspace directory.
#[derive(Debug, Clone)]
pub struct CommandProbeResult {
    /// The workspace root that was used as the working directory.
    pub workspace_root: std::path::PathBuf,
    /// The command that was executed.
    pub command: String,
    /// Whether the command was found on `$PATH` and exited successfully.
    pub success: bool,
    /// Human-readable explanation of what happened.
    pub diagnostic: String,
}

/// Walk upward from `start` to find the nearest directory containing a
/// `Cargo.toml` with a `[workspace]` table.
///
/// Returns `None` if the filesystem root is reached without finding one.
pub fn workspace_root(start: &Path) -> Option<std::path::PathBuf> {
    let mut dir = start.to_path_buf();
    // Canonicalize if possible so we get an absolute path.
    if let Ok(c) = dir.canonicalize() {
        dir = c;
    }
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.exists()
            && let Ok(content) = std::fs::read_to_string(&candidate)
            && content.contains("[workspace]")
        {
            return Some(dir);
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => return None,
        }
    }
}

/// Check whether a crate named `crate_name` exists anywhere within `workspace_root`.
///
/// Walks `workspace_root/crates/` (if present) and the root itself, looking for
/// a `Cargo.toml` that contains `name = "<crate_name>"`.
///
/// Returns `false` if `workspace_root` is not a directory or no match is found.
pub fn workspace_crate_exists(workspace_root: &Path, crate_name: &str) -> bool {
    // Search root Cargo.toml first (single-crate workspace).
    if crate_name_matches(&workspace_root.join("Cargo.toml"), crate_name) {
        return true;
    }
    // Search crates/ subdirectory.
    let crates_dir = workspace_root.join("crates");
    if let Ok(entries) = std::fs::read_dir(&crates_dir) {
        for entry in entries.flatten() {
            let toml = entry.path().join("Cargo.toml");
            if crate_name_matches(&toml, crate_name) {
                return true;
            }
        }
    }
    false
}

/// Probe whether `cmd` (with `args`) runs successfully when invoked with
/// `workspace_root` as the working directory.
///
/// The returned `CommandProbeResult` always contains a human-readable
/// `diagnostic` string that includes the resolved `workspace_root` path,
/// the command attempted, and whether the failure was a missing binary or a
/// non-zero exit.
pub fn command_runnable_from_workspace(
    workspace_root: &Path,
    cmd: &str,
    args: &[&str],
) -> CommandProbeResult {
    let root = workspace_root.to_path_buf();
    let command_str = if args.is_empty() {
        cmd.to_string()
    } else {
        format!("{} {}", cmd, args.join(" "))
    };

    let result = Command::new(cmd)
        .args(args)
        .current_dir(&root)
        .output();

    match result {
        Ok(output) if output.status.success() => CommandProbeResult {
            workspace_root: root.clone(),
            command: command_str.clone(),
            success: true,
            diagnostic: format!(
                "OK: `{}` succeeded (workspace: {})",
                command_str,
                root.display()
            ),
        },
        Ok(output) => {
            let code = output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string());
            CommandProbeResult {
                workspace_root: root.clone(),
                command: command_str.clone(),
                success: false,
                diagnostic: format!(
                    "FAIL: `{}` exited with code {} (workspace: {})",
                    command_str,
                    code,
                    root.display()
                ),
            }
        }
        Err(e) => CommandProbeResult {
            workspace_root: root.clone(),
            command: command_str.clone(),
            success: false,
            diagnostic: format!(
                "FAIL: `{}` could not be launched — {} (workspace: {})",
                command_str,
                e,
                root.display()
            ),
        },
    }
}

/// Return `true` when the `Cargo.toml` at `path` contains `name = "crate_name"`.
fn crate_name_matches(path: &Path, crate_name: &str) -> bool {
    if !path.exists() {
        return false;
    }
    std::fs::read_to_string(path)
        .map(|s| s.contains(&format!("name = \"{crate_name}\"")))
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

    // ----- workspace_root -----

    #[test]
    fn workspace_root_finds_minibox_workspace() {
        // Start from this file's crate directory — workspace root is two levels up.
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = workspace_root(&crate_dir)
            .expect("should find workspace root from crate dir");
        // The workspace Cargo.toml must contain [workspace].
        let content = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(content.contains("[workspace]"), "root Cargo.toml must have [workspace]");
    }

    #[test]
    fn workspace_root_returns_none_for_tmp() {
        // /tmp has no Cargo.toml — should return None.
        let result = workspace_root(std::path::Path::new("/tmp"));
        assert!(result.is_none(), "expected None for /tmp, got {:?}", result);
    }

    // ----- workspace_crate_exists -----

    #[test]
    fn workspace_crate_exists_finds_minibox_core() {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = workspace_root(&crate_dir).expect("workspace root");
        assert!(
            workspace_crate_exists(&root, "minibox-core"),
            "minibox-core should exist in workspace"
        );
    }

    #[test]
    fn workspace_crate_exists_returns_false_for_unknown() {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = workspace_root(&crate_dir).expect("workspace root");
        assert!(
            !workspace_crate_exists(&root, "this-crate-does-not-exist-xyz"),
            "unknown crate should not be found"
        );
    }

    // ----- command_runnable_from_workspace -----

    #[test]
    fn command_runnable_detects_cargo_version() {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = workspace_root(&crate_dir).expect("workspace root");
        let result = command_runnable_from_workspace(&root, "cargo", &["--version"]);
        assert!(result.success, "cargo --version should succeed");
        assert!(
            result.diagnostic.contains(root.to_str().unwrap()),
            "diagnostic must include workspace root path"
        );
    }

    #[test]
    fn command_runnable_failure_includes_workspace_root() {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = workspace_root(&crate_dir).expect("workspace root");
        let result = command_runnable_from_workspace(
            &root,
            "this-binary-does-not-exist-xyz",
            &[],
        );
        assert!(!result.success, "missing binary should not succeed");
        assert!(
            result.diagnostic.contains(root.to_str().unwrap()),
            "failure diagnostic must include workspace root: {}",
            result.diagnostic
        );
    }
}
