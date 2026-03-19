//! Windows Host Compute Service (HCS) stub — Phase 2.

use anyhow::Result;

/// Start a container via HCS (Phase 2 stub).
pub async fn start_container(_image: &str, _command: &[&str]) -> Result<u32> {
    anyhow::bail!("HCS backend not yet implemented (Phase 2)")
}
