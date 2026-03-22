//! Windows Host Compute Service (HCS) adapter — Phase 2 stub.
//!
//! HCS is the low-level Windows API for running native Windows Containers
//! (process-isolated or Hyper-V-isolated). A full implementation would use the
//! `hcs-rs` crate or direct FFI calls to `computecore.dll` to create and
//! manage container instances.
//!
//! All functions in this module are stubs that immediately return an error.

use anyhow::Result;

/// Start a container using the Windows Host Compute Service.
///
/// **Phase 2 stub** — always returns an error. A real implementation would:
/// 1. Create an HCS compute system from an OCI image layer path.
/// 2. Start the compute system and attach stdio.
/// 3. Return the process ID of the container's init process.
///
/// # Parameters
/// - `_image`: the image name or layer path to use as the container root.
/// - `_command`: the command and arguments to execute inside the container.
///
/// # Returns
/// The PID of the started container process on success.
pub async fn start_container(_image: &str, _command: &[&str]) -> Result<u32> {
    anyhow::bail!("HCS backend not yet implemented (Phase 2)")
}
