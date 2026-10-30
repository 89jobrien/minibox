//! EFI boot wiring for the macOS `vz` adapter.
//!
//! `VZEFIBootLoader` has no field for a kernel command line, so `cargo xtask build-vm-image`
//! bakes it into `/boot/grub/grub.cfg` on the ESP. The adapter only needs to point VZ at the
//! variable store and the ESP disk image produced by that tool.
//!
//! See `docs/designs/2026-10-22-vz-efi-boot-design.md`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_foundation::NSURL;
use objc2_virtualization::{
    VZEFIBootLoader, VZEFIVariableStore, VZEFIVariableStoreInitializationOptions,
};
use sha2::{Digest, Sha256};

/// Filename of the manifest `cargo xtask build-vm-image` writes next to the ESP image.
/// Kept in sync with `xtask::vm_image::MANIFEST_FILE` by convention, not by a shared
/// dependency — macbox does not depend on xtask.
const MANIFEST_FILE_NAME: &str = "manifest.json";

/// The remedy printed in every boot-asset preflight failure.
const BUILD_VM_IMAGE_REMEDY: &str = "run `cargo xtask build-vm-image`";

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

    /// Path of the manifest `cargo xtask build-vm-image` writes next to the ESP image.
    fn manifest_path(&self) -> PathBuf {
        self.esp_image
            .parent()
            .map(|parent| parent.join(MANIFEST_FILE_NAME))
            .unwrap_or_else(|| PathBuf::from(MANIFEST_FILE_NAME))
    }

    /// Verify the EFI boot image exists and matches the hash recorded in its build
    /// manifest.
    ///
    /// Without this, a missing or stale ESP surfaces only as an opaque
    /// `VZErrorDomain code=1` once `VZDiskImageStorageDeviceAttachment` tries to open
    /// it. Fails loud instead, naming the missing/mismatched path and the exact
    /// command that fixes it.
    pub fn check_boot_assets(&self) -> Result<()> {
        if !self.esp_image.exists() {
            bail!(
                "EFI boot image not found at {} ({BUILD_VM_IMAGE_REMEDY})",
                self.esp_image.display()
            );
        }

        let manifest_path = self.manifest_path();
        let manifest_bytes = std::fs::read(&manifest_path).with_context(|| {
            format!(
                "EFI boot manifest not found at {} ({BUILD_VM_IMAGE_REMEDY})",
                manifest_path.display()
            )
        })?;
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .with_context(|| format!("parsing {}", manifest_path.display()))?;
        let expected_sha256 = manifest
            .get("esp_sha256")
            .and_then(serde_json::Value::as_str)
            .with_context(|| format!("{} missing esp_sha256 field", manifest_path.display()))?;

        let esp_bytes = std::fs::read(&self.esp_image)
            .with_context(|| format!("reading {}", self.esp_image.display()))?;
        let mut hasher = Sha256::new();
        hasher.update(&esp_bytes);
        let actual_sha256 = format!("{:x}", hasher.finalize());

        if actual_sha256 != expected_sha256 {
            bail!(
                "EFI boot image at {} does not match its manifest hash (expected \
                 {expected_sha256}, got {actual_sha256}); {BUILD_VM_IMAGE_REMEDY} --force",
                self.esp_image.display()
            );
        }

        Ok(())
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

/// Validate `MINIBOX_VZ_BOOT` against the boot modes this adapter actually implements.
///
/// `vz` only boots through `VZEFIBootLoader` as of the Phase 1 EFI migration
/// (see `docs/designs/2026-10-22-vz-efi-boot-design.md`); `VZLinuxBootLoader` was
/// proven to fail deterministically with `VZErrorDomain code=1` on this OS build
/// during that design's Phase 0 gate, so there is no kernel-direct-boot path to
/// select. An unset variable is treated as the default (`efi`); any other value,
/// including `kernel`, is a hard error rather than a silent fallback.
pub fn validate_boot_mode_env() -> Result<()> {
    match std::env::var("MINIBOX_VZ_BOOT") {
        Ok(value) if value == "efi" => Ok(()),
        Ok(other) => bail!(
            "MINIBOX_VZ_BOOT={other:?} is not supported; only \"efi\" is implemented. \
             Kernel-direct boot (VZLinuxBootLoader) was removed in the Phase 1 EFI \
             migration after Phase 0 proved it fails with VZErrorDomain code=1 on this \
             OS build — see docs/designs/2026-10-22-vz-efi-boot-design.md."
        ),
        Err(std::env::VarError::NotPresent) => Ok(()),
        Err(std::env::VarError::NotUnicode(_)) => bail!("MINIBOX_VZ_BOOT is not valid UTF-8"),
    }
}

/// Remediation trailer appended to a VZ start failure whose message names
/// `VZErrorDomain code=1`, listing the three known causes in order of likelihood
/// per `docs/designs/2026-10-22-vz-efi-boot-design.md`'s preflight/diagnostics section.
pub fn code1_remediation_trailer() -> &'static str {
    "the three known causes of VZErrorDomain code=1, in order of likelihood: \
     (1) missing entitlement — run `codesign -d --entitlements :- <miniboxd binary>`; \
     if empty, run `just sign-vz` and restart the daemon; \
     (2) missing or stale EFI boot assets — run `cargo xtask build-vm-image`; \
     (3) an OS-level Virtualization.framework regression — reproduce while running \
     `log stream --predicate 'subsystem == \"com.apple.Virtualization\"'` to distinguish \
     it from the other two."
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
    use minibox_core::vm_boot;

    /// Serializes tests that mutate `MINIBOX_VZ_BOOT`; `std::env::set_var`/`remove_var`
    /// are process-global and unsafe under edition 2024.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    #[test]
    fn check_boot_assets_fails_when_esp_image_is_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let assets = EfiBootAssets::default_paths(dir.path());
        let err = assets
            .check_boot_assets()
            .expect_err("missing ESP should fail");
        let message = err.to_string();
        assert!(message.contains("EFI boot image not found"));
        assert!(message.contains("cargo xtask build-vm-image"));
    }

    #[test]
    fn check_boot_assets_fails_when_manifest_is_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let assets = EfiBootAssets::default_paths(dir.path());
        let efi_dir = assets.esp_image.parent().expect("esp has a parent dir");
        std::fs::create_dir_all(efi_dir).expect("mkdir efi dir");
        std::fs::write(&assets.esp_image, b"not a real esp").expect("write esp");

        let err = assets
            .check_boot_assets()
            .expect_err("missing manifest should fail");
        assert!(err.to_string().contains("EFI boot manifest not found"));
    }

    #[test]
    fn check_boot_assets_fails_on_hash_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let assets = EfiBootAssets::default_paths(dir.path());
        let efi_dir = assets
            .esp_image
            .parent()
            .expect("esp has a parent dir")
            .to_path_buf();
        std::fs::create_dir_all(&efi_dir).expect("mkdir efi dir");
        std::fs::write(&assets.esp_image, b"real esp bytes").expect("write esp");
        std::fs::write(
            efi_dir.join(MANIFEST_FILE_NAME),
            r#"{"esp_sha256": "deadbeef"}"#,
        )
        .expect("write manifest");

        let err = assets
            .check_boot_assets()
            .expect_err("hash mismatch should fail");
        assert!(err.to_string().contains("does not match its manifest hash"));
    }

    #[test]
    fn check_boot_assets_passes_when_hash_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let assets = EfiBootAssets::default_paths(dir.path());
        let efi_dir = assets
            .esp_image
            .parent()
            .expect("esp has a parent dir")
            .to_path_buf();
        std::fs::create_dir_all(&efi_dir).expect("mkdir efi dir");
        let esp_bytes = b"real esp bytes";
        std::fs::write(&assets.esp_image, esp_bytes).expect("write esp");
        let mut hasher = Sha256::new();
        hasher.update(esp_bytes);
        let sha256 = format!("{:x}", hasher.finalize());
        std::fs::write(
            efi_dir.join(MANIFEST_FILE_NAME),
            format!(r#"{{"esp_sha256": "{sha256}"}}"#),
        )
        .expect("write manifest");

        assets.check_boot_assets().expect("hash should match");
    }

    #[test]
    fn validate_boot_mode_env_accepts_efi_and_unset() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK; no other thread reads/writes this var
        // while the guard is held.
        unsafe { std::env::remove_var("MINIBOX_VZ_BOOT") };
        assert!(validate_boot_mode_env().is_ok());
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("MINIBOX_VZ_BOOT", "efi") };
        assert!(validate_boot_mode_env().is_ok());
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::remove_var("MINIBOX_VZ_BOOT") };
    }

    #[test]
    fn validate_boot_mode_env_rejects_kernel() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("MINIBOX_VZ_BOOT", "kernel") };
        let err = validate_boot_mode_env().expect_err("kernel mode should be rejected");
        assert!(err.to_string().contains("not supported"));
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::remove_var("MINIBOX_VZ_BOOT") };
    }

    #[test]
    fn code1_remediation_trailer_names_all_three_causes() {
        let trailer = code1_remediation_trailer();
        assert!(trailer.contains("sign-vz"));
        assert!(trailer.contains("build-vm-image"));
        assert!(trailer.contains("log stream"));
    }
}
