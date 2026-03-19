//! winbox — Windows orchestration for miniboxd.
//!
//! Provides:
//! - `start()`: entry point called from `miniboxd` on Windows
//! - `paths`: Windows-specific default paths
//! - `preflight`: HCS/WSL2 backend detection

pub mod hcs;
pub mod paths;
pub mod preflight;
pub mod wsl2;

use anyhow::Result;
use tracing::info;

#[derive(thiserror::Error, Debug)]
pub enum WinboxError {
    #[error("no backend — enable Windows Containers or install WSL2")]
    NoBackendAvailable,
}

/// Start the Windows daemon.
///
/// Called from `miniboxd`'s Windows `main()`. Phase 1 stub — full
/// Named Pipe server implementation is planned for Phase 2.
pub async fn start() -> Result<()> {
    info!("miniboxd (Windows) starting");
    // Phase 1 stub — full implementation in Phase 2
    anyhow::bail!("Windows server loop not yet implemented (Phase 1 stub)")
}
