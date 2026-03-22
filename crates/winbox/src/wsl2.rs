//! WSL2 container backend adapter — Phase 2 stub.
//!
//! WSL2 (Windows Subsystem for Linux 2) provides a full Linux kernel running
//! inside a lightweight Hyper-V VM. A full implementation would invoke
//! `wsl.exe` or the WSL2 interop API to run Linux container images without
//! needing a separate VM like Colima.
//!
//! All functions in this module are stubs that immediately return an error.

use anyhow::Result;

/// Start a container using the WSL2 backend.
///
/// **Phase 2 stub** — always returns an error. A real implementation would:
/// 1. Import or locate the OCI image as a WSL2 distribution.
/// 2. Invoke `wsl.exe --distribution <name> -- <command>` or the WSL2
///    interop API to run the container process.
/// 3. Return the Linux PID of the container's init process as seen from
///    the Windows host.
///
/// # Parameters
/// - `_image`: the image name or WSL2 distribution name to use.
/// - `_command`: the command and arguments to execute inside the container.
///
/// # Returns
/// The PID of the started container process on success.
pub async fn start_container(_image: &str, _command: &[&str]) -> Result<u32> {
    anyhow::bail!("WSL2 backend not yet implemented (Phase 2)")
}
