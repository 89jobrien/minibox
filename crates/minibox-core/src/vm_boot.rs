//! Guest VM boot contract shared by the macOS VZ adapter and `cargo xtask build-vm-image`.
//!
//! The `vz` adapter boots the guest through `VZEFIBootLoader`, so the kernel command line
//! and the virtiofs share tags are not passed through framework APIs the way the old
//! `VZLinuxBootLoader` path did. They live in the guest's `grub.cfg` and in the ESP layout
//! instead, which means the image builder and the runtime have to agree on them exactly.
//! Defining them once here keeps the two from drifting.
//!
//! See `docs/designs/2026-10-22-vz-efi-boot-design.md`.

/// virtiofs tag of the guest root filesystem share.
pub const ROOTFS_TAG: &str = "minibox-rootfs";

/// virtiofs tag of the read-only image cache share.
pub const IMAGES_TAG: &str = "minibox-images";

/// virtiofs tag of the writable container-state share.
pub const CONTAINERS_TAG: &str = "minibox-containers";

/// Modules the guest loads before it mounts the virtiofs root.
///
/// The Alpine initramfs already carries `virtiofs` and `fuse`; the vsock transport the
/// agent needs is not part of it. `cargo xtask build-vm-image` appends those modules to the
/// initramfs, and the initramfs init loads whatever this list names.
pub const GUEST_MODULES: &[&str] = &[
    "vsock",
    "vmw_vsock_virtio_transport_common",
    "vmw_vsock_virtio_transport",
];

/// Kernel command line rendered into `grub.cfg`.
///
/// `root=` is a virtiofs tag rather than a block device, so the guest mounts the host
/// directory the adapter shares as [`ROOTFS_TAG`].
#[must_use]
pub fn kernel_cmdline() -> String {
    format!(
        "console=hvc0 root={ROOTFS_TAG} rootfstype=virtiofs rw quiet modules={}",
        GUEST_MODULES.join(",")
    )
}

/// Path of the EFI system partition image inside the VM asset directory.
pub const ESP_IMAGE: &str = "boot.img";

/// Path of the EFI removable-media boot binary inside the ESP.
pub const ESP_BOOT_BINARY: &str = "EFI/BOOT/BOOTAA64.EFI";

/// Path of the GRUB configuration inside the ESP.
pub const ESP_GRUB_CFG: &str = "boot/grub/grub.cfg";

/// Path of the guest kernel inside the ESP.
pub const ESP_KERNEL: &str = "boot/vmlinuz-virt";

/// Path of the guest initramfs inside the ESP.
pub const ESP_INITRAMFS: &str = "boot/initramfs-virt";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_cmdline_boots_from_the_virtiofs_root() {
        let cmdline = kernel_cmdline();
        assert!(cmdline.contains("console=hvc0"), "{cmdline}");
        assert!(cmdline.contains(&format!("root={ROOTFS_TAG}")), "{cmdline}");
        assert!(cmdline.contains("rootfstype=virtiofs"), "{cmdline}");
    }

    #[test]
    fn kernel_cmdline_loads_every_guest_module() {
        let cmdline = kernel_cmdline();
        for module in GUEST_MODULES {
            assert!(cmdline.contains(module), "{module} missing from {cmdline}");
        }
    }
}
