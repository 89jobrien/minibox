//! 1Password helpers for secret-injected task commands.

use anyhow::{Context, Result};
use std::process::Command;

pub fn op_read(op_ref: &str) -> Result<String> {
    let output = Command::new("op")
        .args(["read", "--account", "my.1password.com", op_ref])
        .output()
        .context("failed to run op read")?;
    if !output.status.success() {
        anyhow::bail!(
            "op read failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)
        .context("op read output not UTF-8")?
        .trim()
        .to_string())
}
