//! Generates Alpine init configuration files installed into the VM rootfs.
//!
//! These are written by `cargo xtask build-vm-image` into `rootfs/etc/`
//! so the VM boots directly into minibox-agent.

use anyhow::{Context, Result};
use minibox_core::vm_boot::{KERNEL_RELEASE, VSOCK_AGENT_PORT};
use std::path::Path;

/// Generate `/etc/inittab` content that boots minibox-agent via busybox init.
pub fn generate_inittab() -> String {
    // MINIBOX_ALLOW_PRIVILEGED/MINIBOX_ALLOW_BIND_MOUNTS: the vz isolation
    // tests boot privileged containers inside this VM specifically to assert
    // namespace isolation. The policy gate's default-deny protects the host
    // daemon from an untrusted local client; it doesn't apply here, since
    // the only client that can reach this agent is the macOS host over
    // vsock (see VsockServerListener's doc comment in
    // crates/miniboxd/src/listener.rs).
    format!(
        "::sysinit:/etc/init.d/rcS\n::once:/bin/sh -c \"MINIBOX_VSOCK_PORT={VSOCK_AGENT_PORT} MINIBOX_ALLOW_PRIVILEGED=1 MINIBOX_ALLOW_BIND_MOUNTS=1 exec /sbin/minibox-agent\"\n::ctrlaltdel:/sbin/reboot\n::shutdown:/bin/umount -a -r\n"
    )
}

/// Generate `/etc/init.d/rcS` content: minimal system prep before agent starts.
pub fn generate_rc_local() -> String {
    format!(
        r"#!/bin/sh
# Minimal system initialization for minibox-agent VM
set -e

# The virtiofs root is shared read-write from the host (see build_virtiofs()
# in vm.rs, called with read_only=false for ROOTFS_TAG) and the kernel
# cmdline passes `rw` (see minibox_core::vm_boot::kernel_cmdline), but the
# initramfs that hands off to this script still mounts the virtiofs root
# read-only before switch_root. Force it rw before anything below tries to
# create a directory on it.
mount -o remount,rw / 2>/dev/null || true

# Mount virtual filesystems
mount -t proc proc /proc 2>/dev/null || true
mount -t sysfs sys /sys 2>/dev/null || true
mount -t devtmpfs dev /dev 2>/dev/null || true
mount -t tmpfs tmpfs /tmp 2>/dev/null || true

# virtiofs (FUSE-backed) does not support AF_UNIX bind(2) on its shared
# directories -- miniboxd's own control socket at /run/minibox/miniboxd.sock
# must live on a real Linux filesystem, so give it a tmpfs.
mkdir -p /run
mount -t tmpfs tmpfs /run 2>/dev/null || true
mkdir -p /run/minibox

# The `modules=` kernel cmdline param asks the initramfs to modprobe these
# before switch_root, but that has been observed not to actually load them.
# Load explicitly, in dependency order, so AF_VSOCK is available to the agent.
insmod /lib/modules/{KERNEL_RELEASE}/kernel/net/vmw_vsock/vsock.ko 2>/dev/null || true
insmod /lib/modules/{KERNEL_RELEASE}/kernel/net/vmw_vsock/vmw_vsock_virtio_transport_common.ko 2>/dev/null || true
insmod /lib/modules/{KERNEL_RELEASE}/kernel/net/vmw_vsock/vmw_vsock_virtio_transport.ko 2>/dev/null || true

# virtio_net is the driver for the guest NIC itself (VZVirtioNetworkDeviceConfiguration);
# without it eth0 never exists at all. It depends on net_failover, which depends on
# failover -- load those first or insmod fails on an unresolved symbol (silently, since
# errors are discarded below) and eth0 never appears. udhcpc separately opens an
# AF_PACKET socket for raw DHCP frames. Same modules= gap as above.
insmod /lib/modules/{KERNEL_RELEASE}/kernel/net/core/failover.ko 2>/dev/null || true
insmod /lib/modules/{KERNEL_RELEASE}/kernel/drivers/net/net_failover.ko 2>/dev/null || true
insmod /lib/modules/{KERNEL_RELEASE}/kernel/drivers/net/virtio_net.ko 2>/dev/null || true
insmod /lib/modules/{KERNEL_RELEASE}/kernel/net/packet/af_packet.ko 2>/dev/null || true

# overlay rootfs mounts need this; same modules= gap as above.
insmod /lib/modules/{KERNEL_RELEASE}/kernel/fs/overlayfs/overlay.ko 2>/dev/null || true

# Mount virtiofs shares
mkdir -p /var/lib/minibox/images /var/lib/minibox/containers
mount -t virtiofs minibox-images /var/lib/minibox/images 2>/dev/null || true
mount -t virtiofs minibox-containers /var/lib/minibox/containers 2>/dev/null || true

# Loopback interface
ip link set lo up 2>/dev/null || true

# Guest network device (VZVirtioNetworkDeviceConfiguration + NAT attachment, see
# vm.rs). udhcpc + its default script (both pre-provisioned in this rootfs) write
# /etc/resolv.conf from the DHCP-offered nameserver.
ip link set eth0 up 2>/dev/null || true

# eth0 can still be negotiating (or the host's VZNATNetworkDeviceAttachment
# DHCP server not yet ready) the instant this script runs, which makes a
# single udhcpc -n attempt fail with no lease and never retries, leaving the
# guest with no route/DNS for the rest of boot. Retry briefly instead of
# treating link-not-ready as permanent failure.
for _ in 1 2 3 4 5 6 7 8 9 10; do
    udhcpc -i eth0 -n -q 2>/dev/null && break
    sleep 1
done

echo '-- net diag --'
ip addr show eth0 2>&1
ip route 2>&1
cat /etc/resolv.conf 2>&1
echo '-- end net diag --'

# Hostname
hostname minibox-vm 2>/dev/null || true
"
    )
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
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::default_constructed_unit_structs
)]
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
