use async_trait::async_trait;
use glob::glob;
use std::path::{Path, PathBuf};
use tracing::info;

use crate::domain::{IndexError, IndexSource, SourceType, SyncStats};

/// Rsyncs a glob-expanded local path to the VPS, then runs zoekt-index.
pub struct FilesystemSource {
    pub name: String,
    /// Glob pattern, e.g. "~/dev/*/docs"
    pub glob_pattern: String,
    pub ssh_host: String,
    pub remote_index_dir: String,
    pub remote_base: String,
}

#[async_trait]
impl IndexSource for FilesystemSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> SourceType {
        SourceType::Filesystem
    }

    async fn sync(&self, _dest: &Path) -> Result<SyncStats, IndexError> {
        let pattern = shellexpand::tilde(&self.glob_pattern).into_owned();
        let paths: Vec<PathBuf> = glob(&pattern)
            .map_err(|e| IndexError::SyncFailed {
                repo: self.name.clone(),
                reason: format!("glob error: {e}"),
            })?
            .filter_map(|e| e.ok())
            .collect();

        if paths.is_empty() {
            return Err(IndexError::SyncFailed {
                repo: self.name.clone(),
                reason: format!("glob `{pattern}` matched no paths"),
            });
        }

        let remote_path = format!("{}/{}", self.remote_base, self.name);
        zoektbox::deploy::ssh_run(&self.ssh_host, &format!("mkdir -p {remote_path}"))
            .await
            .map_err(|e| IndexError::SyncFailed {
                repo: self.name.clone(),
                reason: e.to_string(),
            })?;

        let mut total = 0u64;
        for local_path in &paths {
            info!(
                src = %local_path.display(),
                dest = %remote_path,
                "FilesystemSource: rsyncing"
            );
            let status = tokio::process::Command::new("rsync")
                .args([
                    "-avz",
                    "--delete",
                    &format!("{}/", local_path.display()),
                    &format!("{}:{remote_path}/", self.ssh_host),
                ])
                .status()
                .await
                .map_err(|e| IndexError::SyncFailed {
                    repo: self.name.clone(),
                    reason: e.to_string(),
                })?;

            if !status.success() {
                return Err(IndexError::SyncFailed {
                    repo: self.name.clone(),
                    reason: format!("rsync exited {status}"),
                });
            }
            total += 1;
        }

        // Trigger zoekt-index on VPS
        let index_cmd = format!(
            "{base}/bin/zoekt-index -index {idx} {remote_path}",
            base = self.remote_base,
            idx = self.remote_index_dir,
        );
        zoektbox::deploy::ssh_run(&self.ssh_host, &index_cmd)
            .await
            .map_err(|e| IndexError::IndexCmd(e.to_string()))?;

        Ok(SyncStats {
            files_synced: total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_type_is_filesystem() {
        let src = FilesystemSource {
            name: "docs".into(),
            glob_pattern: "~/dev/*/docs".into(),
            ssh_host: "minibox".into(),
            remote_index_dir: "/opt/zoekt/index".into(),
            remote_base: "/opt/zoekt".into(),
        };
        assert_eq!(src.source_type(), SourceType::Filesystem);
        assert_eq!(src.name(), "docs");
    }
}
