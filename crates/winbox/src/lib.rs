//! Windows orchestration for miniboxd — Phase 1 stub.
//!
//! This crate is a placeholder for the planned Windows daemon implementation.
//! On Windows, `miniboxd` would delegate container operations to either the
//! Windows Host Compute Service (HCS) or WSL2, depending on what is available.
//!
//! **Current state**: `start()` unconditionally returns an error. No container
//! operations are implemented. The [`hcs`] and [`wsl2`] modules contain stub
//! functions that also bail immediately.
//!
//! **Phase 2 work required**:
//! - Implement a Named Pipe server in place of the Unix socket (see [`paths::pipe_name`]).
//! - Wire up HCS adapter in [`hcs`] for native Windows Containers.
//! - Wire up WSL2 adapter in [`wsl2`] for Linux containers via WSL2.
//! - Implement `SO_PEERCRED`-equivalent auth (token or ACL on the Named Pipe).
//!
//! # Modules
//!
//! - [`hcs`] — Windows Host Compute Service stub
//! - [`wsl2`] — WSL2 backend stub
//! - [`paths`] — Windows-specific default directories and Named Pipe path
//! - [`preflight`] — HCS/WSL2 backend detection

pub mod hcs;
pub mod paths;
pub mod preflight;
pub mod wsl2;

use anyhow::Result;
use tracing::info;

/// Errors that can be returned by the Windows daemon entry point.
#[derive(thiserror::Error, Debug)]
pub enum WinboxError {
    /// No supported container backend was found. The user must enable Windows
    /// Containers (HCS) via Windows Features or install WSL2 before running
    /// miniboxd on Windows.
    #[error("no backend — enable Windows Containers or install WSL2")]
    NoBackendAvailable,
}

/// Start the Windows daemon.
///
/// Called from `miniboxd`'s Windows `main()`. This is a **Phase 1 stub** that
/// logs a startup message and immediately returns an error.
///
/// Phase 2 work needed before this can function:
/// - Create a Named Pipe listener at [`paths::pipe_name`].
/// - Select the appropriate adapter (HCS or WSL2) based on [`preflight`] output.
/// - Run the `daemonbox::server::run_server` accept loop over the Named Pipe.
pub async fn start() -> Result<()> {
    info!("miniboxd (Windows) starting");
    // Phase 1 stub — full implementation in Phase 2
    anyhow::bail!("Windows server loop not yet implemented (Phase 1 stub)")
}
