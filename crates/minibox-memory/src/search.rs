//! Hybrid search over infrastructure memory.
//!
//! Combines keyword (LIKE) and vector (cosine similarity) search branches,
//! fused via reciprocal-rank fusion (RRF).

use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::{
    Embedder, HybridSearchResult, KeywordSearchResult, MemoryError, MemoryStore, Record,
};

const RRF_K: f32 = 60.0;

/// Searcher combining keyword + vector search with RRF fusion.
/// Generic over MemoryStore + Embedder ports.
pub struct MemorySearcher<S: MemoryStore> {
    store: Arc<S>,
    embedder: Arc<dyn Embedder>,
}

impl<S: MemoryStore> MemorySearcher<S> {
    pub fn new(store: Arc<S>, embedder: Arc<dyn Embedder>) -> Self {
        Self { store, embedder }
    }

    /// Keyword search: case-insensitive substring match on content.
    pub async fn keyword_search(
        &self,
        query: &str,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> Result<Vec<KeywordSearchResult>, MemoryError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let records = self.store.fetch(wing, room, 500).await?;
        let query_lower = query.to_ascii_lowercase();

        let mut results: Vec<KeywordSearchResult> = records
            .into_iter()
            .filter_map(|record| {
                let haystack = record.content.to_ascii_lowercase();
                if haystack.contains(&query_lower) {
                    let score = keyword_score(&record, &query_lower);
                    Some(KeywordSearchResult { record, score })
                } else {
                    None
                }
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    /// Vector search: cosine similarity against embedded query.
    /// Note: InMemoryStore doesn't store embeddings, so this works by
    /// re-embedding all records and comparing. Production TursoStore
    /// would use vector_distance_cos in SQL.
    pub async fn vector_search(
        &self,
        query: &str,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::domain::SearchResult>, MemoryError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let query_vec = self.embedder.embed_one(query).await?;
        let records = self.store.fetch(wing, room, 500).await?;

        let texts: Vec<&str> = records.iter().map(|r| r.content.as_str()).collect();
        let embeddings = if texts.is_empty() {
            Vec::new()
        } else {
            self.embedder.embed(&texts).await?
        };

        let mut results: Vec<crate::domain::SearchResult> = records
            .into_iter()
            .zip(embeddings.into_iter())
            .map(|(record, emb)| {
                let similarity = cosine_similarity(&query_vec, &emb);
                crate::domain::SearchResult { record, similarity }
            })
            .collect();

        results.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    /// Hybrid search: keyword + vector fused with RRF.
    pub async fn hybrid_search(
        &self,
        query: &str,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> Result<Vec<HybridSearchResult>, MemoryError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let candidate_limit = limit.saturating_mul(4).max(12);

        let (keyword_results, vector_results) = tokio::join!(
            self.keyword_search(query, wing, room, candidate_limit),
            self.vector_search(query, wing, room, candidate_limit),
        );

        let keyword_hits = keyword_results.unwrap_or_default();
        let vector_hits = vector_results.unwrap_or_default();

        Ok(fuse_rrf(keyword_hits, vector_hits, limit))
    }
}

fn keyword_score(record: &Record, query_lower: &str) -> f32 {
    let haystack =
        format!("{}\n{}\n{}", record.content, record.wing, record.room,).to_ascii_lowercase();

    let mut score = 0.0f32;
    if haystack.contains(query_lower) {
        score += 2.0;
    }

    let tokens: Vec<&str> = query_lower.split_whitespace().collect();
    if !tokens.is_empty() {
        let hits = tokens.iter().filter(|t| haystack.contains(**t)).count() as f32;
        score += hits / tokens.len() as f32;
    }

    score
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    (dot / (mag_a * mag_b)).clamp(0.0, 1.0)
}

fn fuse_rrf(
    keyword_hits: Vec<KeywordSearchResult>,
    vector_hits: Vec<crate::domain::SearchResult>,
    limit: usize,
) -> Vec<HybridSearchResult> {
    struct Acc {
        record: Record,
        rrf_score: f32,
        semantic_similarity: Option<f32>,
        keyword_score: Option<f32>,
    }

    let mut fused: HashMap<String, Acc> = HashMap::new();

    for (rank, hit) in vector_hits.into_iter().enumerate() {
        let id = hit.record.id.clone();
        let entry = fused.entry(id).or_insert_with(|| Acc {
            record: hit.record,
            rrf_score: 0.0,
            semantic_similarity: None,
            keyword_score: None,
        });
        entry.rrf_score += 1.0 / (RRF_K + rank as f32 + 1.0);
        entry.semantic_similarity = Some(hit.similarity);
    }

    for (rank, hit) in keyword_hits.into_iter().enumerate() {
        let id = hit.record.id.clone();
        let entry = fused.entry(id).or_insert_with(|| Acc {
            record: hit.record,
            rrf_score: 0.0,
            semantic_similarity: None,
            keyword_score: None,
        });
        entry.rrf_score += 1.0 / (RRF_K + rank as f32 + 1.0);
        entry.keyword_score = Some(hit.score);
    }

    let mut results: Vec<HybridSearchResult> = fused
        .into_values()
        .map(|acc| HybridSearchResult {
            record: acc.record,
            score: acc.rrf_score,
            semantic_similarity: acc.semantic_similarity,
            keyword_score: acc.keyword_score,
        })
        .collect();

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical_vectors() {
        assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_similarity_zero_vector() {
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
    }
}
