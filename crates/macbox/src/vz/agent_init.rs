//! Generates Alpine init configuration files installed into the VM rootfs.
//!
//! These are written by `cargo xtask build-vm-image` into `rootfs/etc/`
//! so the VM boots directly into minibox-agent.

use anyhow::{Context, Result};
use std::path::Path;

/// Generate `/etc/inittab` content that boots minibox-agent via busybox init.
pub fn generate_inittab() -> String {
    "::sysinit:/etc/init.d/rcS\n::once:/sbin/minibox-agent\n::ctrlaltdel:/sbin/reboot\n::shutdown:/bin/umount -a -r\n".to_string()
}

/// Generate `/etc/init.d/rcS` content: minimal system prep before agent starts.
pub fn generate_rc_local() -> String {
    r#"#!/bin/sh
# Minimal system initialization for minibox-agent VM
set -e

# Mount virtual filesystems
mount -t proc proc /proc 2>/dev/null || true
mount -t sysfs sys /sys 2>/dev/null || true
mount -t devtmpfs dev /dev 2>/dev/null || true
mount -t tmpfs tmpfs /tmp 2>/dev/null || true

# Mount virtiofs shares
mkdir -p /var/lib/minibox/images /var/lib/minibox/containers
mount -t virtiofs minibox-images /var/lib/minibox/images 2>/dev/null || true
mount -t virtiofs minibox-containers /var/lib/minibox/containers 2>/dev/null || true

# Loopback interface
ip link set lo up 2>/dev/null || true

# Hostname
hostname minibox-vm 2>/dev/null || true
"#
    .to_string()
}

/// Install init files into `rootfs_dir/etc/`.
pub fn install_init_files(rootfs_dir: &Path) -> Result<()> {
    let etc = rootfs_dir.join("etc");
    std::fs::create_dir_all(&etc).context("creating rootfs/etc")?;

    // /etc/inittab
    let inittab = etc.join("inittab");
    std::fs::write(&inittab, generate_inittab())
        .with_context(|| format!("writing {}", inittab.display()))?;

    // /etc/init.d/rcS
    let initd = etc.join("init.d");
    std::fs::create_dir_all(&initd).context("creating rootfs/etc/init.d")?;
    let rcs = initd.join("rcS");
    std::fs::write(&rcs, generate_rc_local())
        .with_context(|| format!("writing {}", rcs.display()))?;

    // Make rcS executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&rcs, perms)
            .with_context(|| format!("chmod rcS {}", rcs.display()))?;
    }

    tracing::info!(init = %rcs.display(), inittab = %inittab.display(), "agent_init: installed init files");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inittab_contains_agent_entry() {
        let content = generate_inittab();
        assert!(
            content.contains("minibox-agent"),
            "inittab must reference minibox-agent"
        );
        assert!(
            content.contains("::sysinit:"),
            "inittab must have sysinit entry"
        );
    }

    #[test]
    fn rc_local_mounts_proc_and_sys() {
        let content = generate_rc_local();
        assert!(content.contains("mount -t proc"), "must mount proc");
        assert!(content.contains("mount -t sysfs"), "must mount sysfs");
        assert!(content.contains("mount -t devtmpfs"), "must mount devtmpfs");
    }

    #[test]
    fn rc_local_mounts_virtiofs_shares() {
        let content = generate_rc_local();
        assert!(content.contains("virtiofs"), "must use virtiofs");
        assert!(
            content.contains("minibox-images"),
            "must mount minibox-images share"
        );
        assert!(
            content.contains("minibox-containers"),
            "must mount minibox-containers share"
        );
    }

    #[test]
    fn install_init_files_creates_correct_structure() {
        let tmp = tempfile::tempdir().context("creating temp dir").unwrap();
        install_init_files(tmp.path()).unwrap();
        assert!(tmp.path().join("etc").join("inittab").exists());
        assert!(tmp.path().join("etc").join("init.d").join("rcS").exists());
    }
}
