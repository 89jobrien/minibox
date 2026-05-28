//! Cross-platform filesystem utilities used by container setup.
//!
//! These helpers contain no Linux-specific syscalls (no mount, mknod, etc.)
//! and can be tested on macOS. The `container::filesystem` module re-uses
//! them for overlay fallback and /dev setup.

use anyhow::Context;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Recursive directory copy
// ---------------------------------------------------------------------------

/// Recursively copy directory contents from `src` to `dst`.
///
/// Files are overwritten if they already exist in `dst` (used for
/// multi-layer merge in tmpfs fallback). Symlinks are preserved as
/// symlinks. Errors on individual symlink creation are propagated.
pub fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    for entry in fs::read_dir(src).with_context(|| format!("read_dir {}", src.display()))? {
        let entry = entry?;
        copy_entry(&entry, dst)?;
    }
    Ok(())
}

/// Copy a single directory entry to `dst`, dispatching by file type.
fn copy_entry(entry: &fs::DirEntry, dst: &Path) -> anyhow::Result<()> {
    let src_path = entry.path();
    let dst_path = dst.join(entry.file_name());
    let ft = entry.file_type()?;
    if ft.is_dir() {
        fs::create_dir_all(&dst_path)?;
        copy_dir_recursive(&src_path, &dst_path)?;
    } else if ft.is_symlink() {
        copy_symlink(&src_path, &dst_path)?;
    } else {
        fs::copy(&src_path, &dst_path)?;
    }
    Ok(())
}

/// Copy a symlink, replacing any existing file at `dst`.
fn copy_symlink(src: &Path, dst: &Path) -> anyhow::Result<()> {
    let target = fs::read_link(src)?;
    if dst.symlink_metadata().is_ok() {
        fs::remove_file(dst).ok();
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, dst)
        .with_context(|| format!("symlink {} -> {}", dst.display(), target.display()))?;
    #[cfg(not(unix))]
    {
        let _ = target;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Device node / symlink definitions (data only, no syscalls)
// ---------------------------------------------------------------------------

/// A device node to create via mknod inside the container's /dev.
#[derive(Debug, Clone)]
pub struct DeviceNode {
    pub name: &'static str,
    pub major: u32,
    pub minor: u32,
    pub mode: u32,
}

/// A symlink to create inside the container's /dev.
#[derive(Debug, Clone)]
pub struct DevSymlink {
    pub name: &'static str,
    pub target: &'static str,
}

/// Standard device nodes matching runc/libcontainer defaults.
pub fn default_device_nodes() -> Vec<DeviceNode> {
    vec![
        DeviceNode {
            name: "null",
            major: 1,
            minor: 3,
            mode: 0o666,
        },
        DeviceNode {
            name: "zero",
            major: 1,
            minor: 5,
            mode: 0o666,
        },
        DeviceNode {
            name: "full",
            major: 1,
            minor: 7,
            mode: 0o666,
        },
        DeviceNode {
            name: "random",
            major: 1,
            minor: 8,
            mode: 0o666,
        },
        DeviceNode {
            name: "urandom",
            major: 1,
            minor: 9,
            mode: 0o444,
        },
        DeviceNode {
            name: "tty",
            major: 5,
            minor: 0,
            mode: 0o666,
        },
        DeviceNode {
            name: "console",
            major: 5,
            minor: 1,
            mode: 0o600,
        },
    ]
}

/// Standard /dev symlinks.
pub fn default_dev_symlinks() -> Vec<DevSymlink> {
    vec![
        DevSymlink {
            name: "fd",
            target: "/proc/self/fd",
        },
        DevSymlink {
            name: "stdin",
            target: "/proc/self/fd/0",
        },
        DevSymlink {
            name: "stdout",
            target: "/proc/self/fd/1",
        },
        DevSymlink {
            name: "stderr",
            target: "/proc/self/fd/2",
        },
        DevSymlink {
            name: "ptmx",
            target: "pts/ptmx",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── copy_dir_recursive ───────────────────────────────────────────────

    #[test]
    fn copy_files_and_dirs() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        fs::write(src.path().join("file.txt"), "hello").unwrap();
        fs::create_dir_all(src.path().join("sub")).unwrap();
        fs::write(src.path().join("sub/nested.txt"), "world").unwrap();

        copy_dir_recursive(src.path(), dst.path()).unwrap();

        assert_eq!(
            fs::read_to_string(dst.path().join("file.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(dst.path().join("sub/nested.txt")).unwrap(),
            "world"
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_preserves_symlinks() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        fs::write(src.path().join("target.txt"), "data").unwrap();
        std::os::unix::fs::symlink("target.txt", src.path().join("link.txt")).unwrap();

        copy_dir_recursive(src.path(), dst.path()).unwrap();

        let link_target = fs::read_link(dst.path().join("link.txt")).unwrap();
        assert_eq!(link_target.to_str().unwrap(), "target.txt");
    }

    #[test]
    fn later_layer_overwrites_earlier() {
        let layer1 = TempDir::new().unwrap();
        let layer2 = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        fs::write(layer1.path().join("config"), "v1").unwrap();
        fs::write(layer1.path().join("base"), "unchanged").unwrap();
        fs::write(layer2.path().join("config"), "v2").unwrap();

        copy_dir_recursive(layer1.path(), dst.path()).unwrap();
        copy_dir_recursive(layer2.path(), dst.path()).unwrap();

        assert_eq!(
            fs::read_to_string(dst.path().join("config")).unwrap(),
            "v2",
            "later layer should overwrite"
        );
        assert_eq!(
            fs::read_to_string(dst.path().join("base")).unwrap(),
            "unchanged"
        );
    }

    #[test]
    fn empty_source_is_noop() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        copy_dir_recursive(src.path(), dst.path()).unwrap();
        let entries: Vec<_> = fs::read_dir(dst.path()).unwrap().collect();
        assert!(entries.is_empty());
    }

    #[test]
    fn nonexistent_source_returns_error() {
        let dst = TempDir::new().unwrap();
        let result = copy_dir_recursive(Path::new("/nonexistent/path"), dst.path());
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn copy_overwrites_existing_symlink() {
        let layer1 = TempDir::new().unwrap();
        let layer2 = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        // Layer 1 has a symlink pointing to "old_target"
        std::os::unix::fs::symlink("old_target", layer1.path().join("link")).unwrap();
        // Layer 2 has the same symlink pointing to "new_target"
        std::os::unix::fs::symlink("new_target", layer2.path().join("link")).unwrap();

        copy_dir_recursive(layer1.path(), dst.path()).unwrap();
        copy_dir_recursive(layer2.path(), dst.path()).unwrap();

        let target = fs::read_link(dst.path().join("link")).unwrap();
        assert_eq!(target.to_str().unwrap(), "new_target");
    }

    // ── device node / symlink data ───────────────────────────────────────

    #[test]
    fn device_nodes_complete() {
        let nodes = default_device_nodes();
        let names: Vec<&str> = nodes.iter().map(|n| n.name).collect();
        assert!(names.contains(&"null"), "missing /dev/null");
        assert!(names.contains(&"zero"), "missing /dev/zero");
        assert!(names.contains(&"full"), "missing /dev/full");
        assert!(names.contains(&"random"), "missing /dev/random");
        assert!(names.contains(&"urandom"), "missing /dev/urandom");
        assert!(names.contains(&"tty"), "missing /dev/tty");
        assert!(names.contains(&"console"), "missing /dev/console");
    }

    #[test]
    fn dev_symlinks_complete() {
        let links = default_dev_symlinks();
        let names: Vec<&str> = links.iter().map(|l| l.name).collect();
        assert!(names.contains(&"fd"), "missing /dev/fd");
        assert!(names.contains(&"stdin"), "missing /dev/stdin");
        assert!(names.contains(&"stdout"), "missing /dev/stdout");
        assert!(names.contains(&"stderr"), "missing /dev/stderr");
        assert!(names.contains(&"ptmx"), "missing /dev/ptmx");
    }

    #[test]
    fn device_node_majmin_matches_linux_standard() {
        let nodes = default_device_nodes();
        let null_node = nodes.iter().find(|n| n.name == "null").unwrap();
        assert_eq!(null_node.major, 1);
        assert_eq!(null_node.minor, 3);
        let zero_node = nodes.iter().find(|n| n.name == "zero").unwrap();
        assert_eq!(zero_node.major, 1);
        assert_eq!(zero_node.minor, 5);
        let tty_node = nodes.iter().find(|n| n.name == "tty").unwrap();
        assert_eq!(tty_node.major, 5);
        assert_eq!(tty_node.minor, 0);
        let console_node = nodes.iter().find(|n| n.name == "console").unwrap();
        assert_eq!(console_node.major, 5);
        assert_eq!(console_node.minor, 1);
    }

    #[test]
    fn device_node_modes_are_valid() {
        for node in default_device_nodes() {
            assert!(
                node.mode <= 0o777,
                "{} has invalid mode: {:#o}",
                node.name,
                node.mode
            );
        }
    }

    #[test]
    fn dev_symlinks_no_absolute_targets_except_proc() {
        for link in default_dev_symlinks() {
            if link.target.starts_with('/') {
                assert!(
                    link.target.starts_with("/proc/"),
                    "/dev/{} points to absolute path outside /proc: {}",
                    link.name,
                    link.target
                );
            }
        }
    }
}
