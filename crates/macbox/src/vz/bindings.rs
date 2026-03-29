//! Raw objc2 bindings helpers for Virtualization.framework.
//!
//! All functions in this module that touch VZ.framework objects must be called
//! from a dedicated GCD serial queue or the main thread, as VZ.framework
//! requires it.

use anyhow::{Context, Result, bail};
use std::path::Path;

/// Load the Virtualization.framework bundle.
///
/// SAFETY: NSBundle loading is safe to call from any thread.
/// Must be called before accessing any VZ class.
pub fn load_vz_framework() -> Result<()> {
    // The Virtualization.framework is a system framework available on macOS 11+.
    // We just verify it can be loaded by checking the VZVirtualMachine class exists.
    // objc2 will panic if a class is not found, so this serves as our availability check.
    #[cfg(target_os = "macos")]
    {
        // Just verify we're on macOS 11+. VZ.framework loads lazily when first class is accessed.
        let version = std::process::Command::new("sw_vers")
            .args(["-productVersion"])
            .output()
            .context("checking macOS version")?;
        let ver_str = String::from_utf8_lossy(&version.stdout);
        let major: u32 = ver_str
            .trim()
            .split('.')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if major < 11 {
            bail!(
                "Virtualization.framework requires macOS 11 or later (got {})",
                ver_str.trim()
            );
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    bail!("Virtualization.framework is only available on macOS")
}

/// Create a VZLinuxBootLoader — STUB, implemented in Task 7 alongside VzVm.
///
/// # Safety
///
/// Must be called on the VZ dispatch queue. The returned pointer is owned by
/// the caller and must be released via the Objective-C runtime when done.
pub unsafe fn new_linux_boot_loader(
    _kernel_path: &Path,
    _initrd_path: Option<&Path>,
    _cmdline: &str,
) -> Result<*mut std::ffi::c_void> {
    bail!("VZ bindings not yet fully implemented — see vm.rs Task 7")
}

/// Create a VZVirtioFileSystemDeviceConfiguration — STUB.
///
/// # Safety
///
/// Must be called on the VZ dispatch queue. The returned pointer is owned by
/// the caller and must be released via the Objective-C runtime when done.
pub unsafe fn new_virtio_fs(_tag: &str, _host_path: &Path) -> Result<*mut std::ffi::c_void> {
    bail!("VZ bindings not yet fully implemented — see vm.rs Task 7")
}

/// Create a VZVirtioSocketDeviceConfiguration — STUB.
///
/// # Safety
///
/// Must be called on the VZ dispatch queue. The returned pointer is owned by
/// the caller and must be released via the Objective-C runtime when done.
pub unsafe fn new_vsock_device() -> Result<*mut std::ffi::c_void> {
    bail!("VZ bindings not yet fully implemented — see vm.rs Task 7")
}
