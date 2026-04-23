//! ZoektServiceAdapter — manages zoekt-indexserver + zoekt-webserver on a remote VPS via SSH.

use anyhow::{Context, Result};
use tracing::info;

use crate::deploy::ssh_run;

/// Configuration for the remote Zoekt service.
#[derive(Debug, Clone)]
pub struct ZoektServiceConfig {
    /// Tailscale SSH host alias.
    pub ssh_host: String,
    /// Port for zoekt-webserver (default 6070).
    pub port: u16,
    /// Remote path where binaries and index live.
    pub remote_base: String,
}

impl Default for ZoektServiceConfig {
    fn default() -> Self {
        Self {
            ssh_host: "minibox".into(),
            port: 6070,
            remote_base: "/opt/zoekt".into(),
        }
    }
}

pub struct ZoektServiceAdapter {
    pub config: ZoektServiceConfig,
}

impl ZoektServiceAdapter {
    pub fn new(config: ZoektServiceConfig) -> Self {
        Self { config }
    }

    /// Download, verify, and rsync Zoekt binaries to the VPS.
    /// Must be called once before `start()`.
    pub async fn provision(&self) -> Result<()> {
        let tmp = tempfile::tempdir().context("tempdir")?;
        crate::download::download_release(
            crate::release::ZoektPlatform::LinuxAmd64,
            tmp.path(),
        )
        .await?;
        crate::deploy::deploy_binaries(
            &self.config.ssh_host,
            tmp.path(),
            &format!("{}/bin", self.config.remote_base),
        )
        .await?;
        // Create index dir on VPS
        ssh_run(
            &self.config.ssh_host,
            &format!("mkdir -p {}/index", self.config.remote_base),
        )
        .await?;
        info!(host = %self.config.ssh_host, "zoektbox: provision complete");
        Ok(())
    }

    fn index_dir(&self) -> String {
        format!("{}/index", self.config.remote_base)
    }

    fn bin(&self, name: &str) -> String {
        format!("{}/bin/{name}", self.config.remote_base)
    }

    pub async fn start(&self) -> Result<()> {
        let index = self.index_dir();
        let webserver = self.bin("zoekt-webserver");
        let indexserver = self.bin("zoekt-indexserver");

        // Start indexserver (daemonised via nohup)
        ssh_run(
            &self.config.ssh_host,
            &format!(
                "nohup {indexserver} -index {index} </dev/null >/opt/zoekt/indexserver.log 2>&1 &"
            ),
        )
        .await
        .context("start indexserver")?;

        // Start webserver bound to Tailscale IP only (not 0.0.0.0)
        let ts_ip = self.tailscale_ip().await?;
        ssh_run(
            &self.config.ssh_host,
            &format!(
                "nohup {webserver} -index {index} -listen {ts_ip}:{} </dev/null >/opt/zoekt/webserver.log 2>&1 &",
                self.config.port
            ),
        )
        .await
        .context("start webserver")?;

        info!(host = %self.config.ssh_host, port = self.config.port, "zoektbox: started");
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        ssh_run(
            &self.config.ssh_host,
            "pkill -f zoekt-webserver; pkill -f zoekt-indexserver; true",
        )
        .await
        .context("stop zoekt")?;
        info!(host = %self.config.ssh_host, "zoektbox: stopped");
        Ok(())
    }

    pub async fn status(&self) -> Result<bool> {
        let url = format!(
            "http://{}:{}/healthz",
            self.config.ssh_host, self.config.port
        );
        match reqwest::get(&url).await {
            Ok(r) => Ok(r.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    pub async fn reindex(&self, repo: Option<&str>) -> Result<()> {
        let index = self.index_dir();
        let git_index = self.bin("zoekt-git-index");
        let cmd = match repo {
            Some(r) => format!("{git_index} -index {index} {index}/{r}.git"),
            None => format!(
                "for d in {index}/*.git; do {git_index} -index {index} \"$d\"; done"
            ),
        };
        ssh_run(&self.config.ssh_host, &cmd)
            .await
            .context("reindex")?;
        Ok(())
    }

    async fn tailscale_ip(&self) -> Result<String> {
        let out = ssh_run(&self.config.ssh_host, "tailscale ip -4")
            .await
            .context("get tailscale IP")?;
        Ok(out.trim().to_string())
    }
}
