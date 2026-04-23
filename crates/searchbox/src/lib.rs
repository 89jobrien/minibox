pub mod adapters;
pub mod config;
pub mod domain;
pub mod mcp;

pub use domain::{
    IndexError, IndexSource, RepoInfo, SearchError, SearchProvider, SearchQuery, SearchResult,
    ServiceError, ServiceManager, ServiceStatus, SourceType, SyncStats,
};
