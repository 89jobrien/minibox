#![allow(dead_code)]

use anyhow::{Context, Result};
use std::process::Command;

/// Fetch a secret from 1Password by its `op://` reference.
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

/// Build a [`Command`] that runs `program` with `args` through `dotenvx run`,
/// injecting the decrypted env file secrets.
///
/// `env_file_path` is the resolved path to the encrypted env file (e.g. from
/// `XConfig::dotenv.resolved_env_file()`).
///
/// Reads the 1Password `op://` reference for the dotenvx private key from the
/// `DOTENV_KEY_OP_REF` environment variable.
pub fn dotenvx_command(program: &str, args: &[&str], env_file_path: &str) -> Result<Command> {
    let op_ref = std::env::var("DOTENV_KEY_OP_REF").context(
        "DOTENV_KEY_OP_REF env var not set — set it to the op:// ref for the dotenvx private key",
    )?;
    let key = op_read(&op_ref)?;
    let env_file = format!("--env-file={env_file_path}");

    let mut cmd = Command::new("dotenvx");
    cmd.args(["run", &env_file, "--", program]);
    cmd.args(args);
    cmd.env("DOTENV_PRIVATE_KEY", &key);
    Ok(cmd)
}
