use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::path::Path;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub repos: Option<Vec<String>>, // None = all
    pub lang: Option<String>,
    pub case_sensitive: bool,
    pub context_lines: u8, // default 2
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            repos: None,
            lang: None,
            case_sensitive: false,
            context_lines: 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub repo: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub snippet: String,
    pub score: f32,
    pub commit: Option<String>, // SHA if from git history ref
}

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub name: String,
    pub source_type: SourceType,
    pub last_indexed: Option<DateTime<Utc>>,
    pub doc_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    Git,
    #[serde(rename = "fs")]
    Filesystem,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Indexing,
}

pub struct SyncStats {
    pub files_synced: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("zoekt unavailable: {0}")]
    Unavailable(String),
    #[error("query failed: {0}")]
    QueryFailed(String),
}

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("sync failed for {repo}: {reason}")]
    SyncFailed { repo: String, reason: String },
    #[error("index command failed: {0}")]
    IndexCmd(String),
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("ssh: {0}")]
    Ssh(String),
    #[error("process: {0}")]
    Process(String),
}

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

#[async_trait]
pub trait SearchProvider: Send + Sync {
    async fn search(&self, query: SearchQuery) -> Result<Vec<SearchResult>, SearchError>;
    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError>;
}

#[async_trait]
pub trait IndexSource: Send + Sync {
    fn name(&self) -> &str;
    fn source_type(&self) -> SourceType;
    async fn sync(&self, dest: &Path) -> Result<SyncStats, IndexError>;
}

#[async_trait]
pub trait ServiceManager: Send + Sync {
    async fn start(&self) -> Result<(), ServiceError>;
    async fn stop(&self) -> Result<(), ServiceError>;
    async fn status(&self) -> Result<ServiceStatus, ServiceError>;
    async fn reindex(&self, repo: Option<&str>) -> Result<(), ServiceError>;
}
