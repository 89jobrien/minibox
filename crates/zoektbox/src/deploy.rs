use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

/// Rsync binaries from `src_dir` to `dest_host:dest_path` via SSH.
/// `ssh_host` is a Tailscale alias (e.g. "minibox").
pub async fn deploy_binaries(ssh_host: &str, src_dir: &Path, dest_path: &str) -> Result<()> {
    info!(host = ssh_host, src = %src_dir.display(), dest = dest_path, "zoektbox: deploying binaries");

    let status = tokio::process::Command::new("rsync")
        .args([
            "-avz",
            "--chmod=755",
            &format!("{}/", src_dir.to_string_lossy()),
            &format!("{ssh_host}:{dest_path}/"),
        ])
        .status()
        .await
        .context("rsync")?;

    if !status.success() {
        anyhow::bail!("rsync exited with {status}");
    }
    info!(host = ssh_host, "zoektbox: deploy complete");
    Ok(())
}

/// Run a command on the remote host via SSH, returning stdout as a String.
pub async fn ssh_run(ssh_host: &str, cmd: &str) -> Result<String> {
    let out = tokio::process::Command::new("ssh")
        .args([ssh_host, cmd])
        .output()
        .await
        .with_context(|| format!("ssh {ssh_host} {cmd}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("ssh command failed: {stderr}");
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
