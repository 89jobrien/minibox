use anyhow::Context;
use async_trait::async_trait;
use std::path::Path;
use tracing::info;

use crate::domain::{IndexError, IndexSource, SourceType, SyncStats};

/// Mirrors a remote git repo into `dest` (bare clone) for zoekt-git-index.
pub struct GitRepoSource {
    pub name: String,
    pub url: String,
    /// SSH host to run `zoekt-git-index` on after mirroring.
    pub ssh_host: String,
    pub remote_index_dir: String,
}

#[async_trait]
impl IndexSource for GitRepoSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> SourceType {
        SourceType::Git
    }

    async fn sync(&self, dest: &Path) -> Result<SyncStats, IndexError> {
        let bare = dest.join(format!("{}.git", self.name));
        let map_err = |e: anyhow::Error| IndexError::SyncFailed {
            repo: self.name.clone(),
            reason: e.to_string(),
        };

        if bare.exists() {
            info!(repo = %self.name, "GitRepoSource: updating mirror");
            tokio::process::Command::new("git")
                .args(["-C", bare.to_str().unwrap_or_default(), "remote", "update"])
                .status()
                .await
                .context("git remote update")
                .map_err(map_err)?;
        } else {
            info!(repo = %self.name, url = %self.url, "GitRepoSource: cloning mirror");
            tokio::process::Command::new("git")
                .args([
                    "clone",
                    "--mirror",
                    &self.url,
                    bare.to_str().unwrap_or_default(),
                ])
                .status()
                .await
                .context("git clone --mirror")
                .map_err(map_err)?;
        }

        // Rsync bare repo to VPS
        let status = tokio::process::Command::new("rsync")
            .args([
                "-avz",
                &format!("{}/", bare.display()),
                &format!(
                    "{}:{}/{}.git/",
                    self.ssh_host, self.remote_index_dir, self.name
                ),
            ])
            .status()
            .await
            .context("rsync to VPS")
            .map_err(map_err)?;

        if !status.success() {
            return Err(IndexError::SyncFailed {
                repo: self.name.clone(),
                reason: format!("rsync exited {status}"),
            });
        }

        // Trigger zoekt-git-index on VPS
        let index_cmd = format!(
            "{dir}/bin/zoekt-git-index -index {dir} {dir}/{name}.git",
            dir = self.remote_index_dir,
            name = self.name,
        );
        zoektbox::deploy::ssh_run(&self.ssh_host, &index_cmd)
            .await
            .context("zoekt-git-index")
            .map_err(|e| IndexError::IndexCmd(e.to_string()))?;

        info!(repo = %self.name, "GitRepoSource: sync complete");
        Ok(SyncStats { files_synced: 1 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_type_is_git() {
        let src = GitRepoSource {
            name: "testrepo".into(),
            url: "git@github.com:example/repo.git".into(),
            ssh_host: "minibox".into(),
            remote_index_dir: "/opt/zoekt".into(),
        };
        assert_eq!(src.source_type(), SourceType::Git);
        assert_eq!(src.name(), "testrepo");
    }
}
