//! Infrastructure memory for minibox.
//!
//! Hybrid vector + keyword search over Turso/SQLite for persistent
//! infrastructure knowledge: deployments, config changes, errors, health.

pub mod adapters;
pub mod domain;
pub mod layers;
pub mod search;

pub use domain::{
    Embedder, HybridSearchResult, KeywordSearchResult, MemoryError, MemoryStore, Record,
    SearchResult, wings,
};
pub use layers::InfraMemory;
pub use search::MemorySearcher;
