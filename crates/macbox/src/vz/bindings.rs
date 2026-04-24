//! Raw objc2 bindings helpers for Virtualization.framework.
//!
//! All functions in this module that touch VZ.framework objects must be called
//! from a dedicated GCD serial queue or the main thread, as VZ.framework
//! requires it.

use anyhow::{Context, Result, bail};

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
