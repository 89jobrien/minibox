//! WSL2 backend stub — Phase 2.

use anyhow::Result;

/// Start a container via WSL2 (Phase 2 stub).
pub async fn start_container(_image: &str, _command: &[&str]) -> Result<u32> {
    anyhow::bail!("WSL2 backend not yet implemented (Phase 2)")
}
