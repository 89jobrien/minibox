//! Domain layer: types, errors, and port traits for infrastructure memory.
//!
//! Zero infrastructure dependencies. Adapters implement these traits.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Wing/room taxonomy
// ---------------------------------------------------------------------------

/// Known wings for infrastructure memory.
pub mod wings {
    pub const DEPLOY: &str = "deploy";
    pub const CONFIG: &str = "config";
    pub const ERROR: &str = "error";
    pub const HEALTH: &str = "health";
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// A single infrastructure memory record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub wing: String,
    pub room: String,
    pub content: String,
    pub source: Option<String>,
    pub recorded_by: String,
    pub recorded_at: String,
}

impl Record {
    pub fn new(
        id: impl Into<String>,
        wing: impl Into<String>,
        room: impl Into<String>,
        content: impl Into<String>,
        recorded_by: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            wing: wing.into(),
            room: room.into(),
            content: content.into(),
            source: None,
            recorded_by: recorded_by.into(),
            recorded_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_recorded_at(mut self, at: impl Into<String>) -> Self {
        self.recorded_at = at.into();
        self
    }
}

/// A semantic search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub record: Record,
    pub similarity: f32,
}

/// A keyword search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordSearchResult {
    pub record: Record,
    pub score: f32,
}

/// A hybrid keyword + vector search result fused via reciprocal-rank fusion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchResult {
    pub record: Record,
    pub score: f32,
    pub semantic_similarity: Option<f32>,
    pub keyword_score: Option<f32>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("store error: {0}")]
    Store(String),
    #[error("embedding error: {0}")]
    Embed(String),
    #[error("search error: {0}")]
    Search(String),
    #[error("record not found: {0}")]
    NotFound(String),
}

// ---------------------------------------------------------------------------
// Port: MemoryStore
// ---------------------------------------------------------------------------

#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Insert a record with an optional embedding vector.
    async fn insert(&self, record: &Record, embedding: Option<&[f32]>)
    -> Result<bool, MemoryError>;

    /// Check if a record ID exists.
    async fn exists(&self, id: &str) -> Result<bool, MemoryError>;

    /// Fetch a record by ID.
    async fn get(&self, id: &str) -> Result<Option<Record>, MemoryError>;

    /// Delete a record by ID. Returns true if it existed.
    async fn delete(&self, id: &str) -> Result<bool, MemoryError>;

    /// Fetch records filtered by wing/room, ordered by recorded_at desc.
    async fn fetch(
        &self,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Record>, MemoryError>;

    /// Total record count.
    async fn count(&self) -> Result<usize, MemoryError>;

    /// Wing and room counts: (wing -> count, room -> count).
    async fn taxonomy(&self) -> Result<(Vec<(String, usize)>, Vec<(String, usize)>), MemoryError>;
}

// ---------------------------------------------------------------------------
// Port: Embedder
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Embedder: Send + Sync {
    /// Embed a batch of text inputs, returning one Vec<f32> per input.
    async fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, MemoryError>;

    /// Embed a single text input.
    async fn embed_one(&self, text: &str) -> Result<Vec<f32>, MemoryError> {
        let mut results = self.embed(&[text]).await?;
        results
            .pop()
            .ok_or_else(|| MemoryError::Embed("empty embedding result".into()))
    }

    /// Vector dimension produced by this embedder.
    fn dimension(&self) -> usize;
}
