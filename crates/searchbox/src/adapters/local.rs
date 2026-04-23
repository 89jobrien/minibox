use async_trait::async_trait;
use std::path::Path;

use crate::domain::{
    IndexError, IndexSource, RepoInfo, SearchError, SearchProvider, SearchQuery, SearchResult,
    SourceType, SyncStats,
};

use super::zoekt::ZoektAdapter;

/// Local Mac-side Zoekt sidecar. Indexes paths that must not leave the Mac.
/// Acts as both IndexSource (manages local zoekt process) and SearchProvider (queries it).
pub struct LocalZoektSource {
    pub name: String,
    pub local_path: String,
    zoekt: ZoektAdapter,
}

impl LocalZoektSource {
    pub fn new(name: impl Into<String>, local_path: impl Into<String>, port: u16) -> Self {
        Self {
            name: name.into(),
            local_path: local_path.into(),
            zoekt: ZoektAdapter::new(format!("http://localhost:{port}")),
        }
    }
}

#[async_trait]
impl IndexSource for LocalZoektSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> SourceType {
        SourceType::Local
    }

    async fn sync(&self, _dest: &Path) -> Result<SyncStats, IndexError> {
        let path = shellexpand::tilde(&self.local_path).into_owned();
        let status = tokio::process::Command::new("zoekt-git-index")
            .args(["-index", &format!("/tmp/zoekt-local-{}", self.name), &path])
            .status()
            .await
            .map_err(|e| IndexError::IndexCmd(e.to_string()))?;

        if !status.success() {
            return Err(IndexError::IndexCmd(format!(
                "zoekt-git-index exited {status}"
            )));
        }
        Ok(SyncStats { files_synced: 1 })
    }
}

#[async_trait]
impl SearchProvider for LocalZoektSource {
    async fn search(&self, query: SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        self.zoekt.search(query).await
    }

    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError> {
        self.zoekt.list_repos().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_type_is_local() {
        let src = LocalZoektSource::new("myrepo", "~/dev/myrepo", 6071);
        assert_eq!(src.source_type(), SourceType::Local);
        assert_eq!(src.name(), "myrepo");
    }
}
