//! EFI boot wiring for the macOS `vz` adapter.
//!
//! `VZEFIBootLoader` has no field for a kernel command line, so `cargo xtask build-vm-image`
//! bakes it into `/boot/grub/grub.cfg` on the ESP. The adapter only needs to point VZ at the
//! variable store and the ESP disk image produced by that tool.
//!
//! See `docs/designs/2026-10-22-vz-efi-boot-design.md`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_foundation::NSURL;
use objc2_virtualization::{
    VZEFIBootLoader, VZEFIVariableStore, VZEFIVariableStoreInitializationOptions,
};

/// EFI boot assets produced by `cargo xtask build-vm-image`.
#[derive(Debug, Clone)]
pub struct EfiBootAssets {
    /// Path of the FAT32 ESP image containing GRUB and the guest kernel/initramfs.
    pub esp_image: PathBuf,
    /// Path of the NVRAM variable store used by the EFI firmware.
    pub variable_store: PathBuf,
}

impl EfiBootAssets {
    /// Default paths under `~/.minibox/vm`.
    #[must_use]
    pub fn default_paths(vm_dir: &Path) -> Self {
        Self {
            esp_image: vm_dir.join("efi").join("boot.img"),
            variable_store: vm_dir.join("efi").join("NVRAM").join("efi_vars.fd"),
        }
    }

    /// Ensure the variable store exists, creating one (allowing overwrite of a stale,
    /// zero-byte file left by a prior failed attempt) if necessary.
    ///
    /// A pre-existing, fully-initialized store is reopened as-is via `initWithURL:` so
    /// boot-time NVRAM state (e.g. the EFI boot order) persists across VM restarts.
    pub fn ensure_variable_store(&self) -> Result<Retained<VZEFIVariableStore>> {
        if let Some(parent) = self.variable_store.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("creating EFI variable store directory {}", parent.display())
            })?;
        }
        let url = unsafe { url_for_path(&self.variable_store) }
            .context("EFI variable store path to NSURL")?;
        let is_valid_existing_store = self
            .variable_store
            .metadata()
            .map(|meta| meta.len() > 0)
            .unwrap_or(false);
        if is_valid_existing_store {
            let store =
                unsafe { VZEFIVariableStore::initWithURL(VZEFIVariableStore::alloc(), &url) };
            return Ok(store);
        }
        let store = unsafe {
            VZEFIVariableStore::initCreatingVariableStoreAtURL_options_error(
                VZEFIVariableStore::alloc(),
                &url,
                VZEFIVariableStoreInitializationOptions::AllowOverwrite,
            )
        }
        .map_err(|e| anyhow::anyhow!("create EFI variable store: {e:?}"))?;
        Ok(store)
    }

    /// Build the EFI boot loader backed by `store`.
    ///
    /// # Safety
    ///
    /// All VZ object constructors used here are standard Objective-C init methods.
    pub unsafe fn build_boot_loader(
        &self,
        store: &VZEFIVariableStore,
    ) -> Result<Retained<VZEFIBootLoader>> {
        // SAFETY: VZEFIBootLoader::new() follows the standard ObjC +new convention.
        let boot_loader = unsafe { VZEFIBootLoader::new() };
        // SAFETY: setVariableStore: is a standard setter; store is a valid VZEFIVariableStore.
        unsafe { boot_loader.setVariableStore(Some(store)) };
        Ok(boot_loader)
    }
}

/// Convert a filesystem path to an `NSURL`.
///
/// # Safety
///
/// `NSURL::fileURLWithPath:` is a standard initializer.
pub unsafe fn url_for_path(path: &Path) -> Result<Retained<NSURL>> {
    let ns_string = objc2_foundation::NSString::from_str(
        path.to_str()
            .context("path contains non-UTF-8 characters")?,
    );
    let url = NSURL::fileURLWithPath(&ns_string);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::vm_boot;

    #[test]
    fn default_paths_use_the_image_directory() {
        let dir = PathBuf::from("/tmp/vm");
        let assets = EfiBootAssets::default_paths(&dir);
        assert_eq!(assets.esp_image, dir.join("efi").join("boot.img"));
        assert_eq!(
            assets.variable_store,
            dir.join("efi").join("NVRAM").join("efi_vars.fd")
        );
    }

    #[test]
    fn esp_layout_matches_the_boot_contract() {
        assert_eq!(vm_boot::ESP_IMAGE, "boot.img");
        assert_eq!(vm_boot::ESP_BOOT_BINARY, "EFI/BOOT/BOOTAA64.EFI");
        assert_eq!(vm_boot::ESP_GRUB_CFG, "boot/grub/grub.cfg");
        assert_eq!(vm_boot::ESP_KERNEL, "boot/vmlinuz-virt");
        assert_eq!(vm_boot::ESP_INITRAMFS, "boot/initramfs-virt");
    }
}
