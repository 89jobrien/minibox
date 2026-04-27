//! Tests for MemorySearcher — hybrid keyword + vector search with RRF fusion.

use minibox_memory::adapters::in_memory::InMemoryStore;
use minibox_memory::domain::{Embedder, MemoryError, MemoryStore, Record, wings};
use minibox_memory::search::MemorySearcher;
use std::sync::Arc;

/// Test embedder: produces 2D vectors based on content keywords.
struct TestEmbedder;

#[async_trait::async_trait]
impl Embedder for TestEmbedder {
    async fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Ok(inputs
            .iter()
            .map(|text| {
                let lower = text.to_ascii_lowercase();
                if lower.contains("deploy") {
                    vec![1.0, 0.0]
                } else if lower.contains("error") {
                    vec![0.0, 1.0]
                } else {
                    vec![0.5, 0.5]
                }
            })
            .collect())
    }

    fn dimension(&self) -> usize {
        2
    }
}

async fn setup_store_with_records() -> (Arc<InMemoryStore>, MemorySearcher<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let embedder = Arc::new(TestEmbedder);
    let searcher = MemorySearcher::new(store.clone(), embedder);

    store
        .insert(
            &Record::new("r1", wings::DEPLOY, "nginx", "deployed nginx v1.2", "test"),
            Some(&[1.0, 0.0]),
        )
        .await
        .unwrap();
    store
        .insert(
            &Record::new("r2", wings::ERROR, "oom", "OOM error killed worker", "test"),
            Some(&[0.0, 1.0]),
        )
        .await
        .unwrap();
    store
        .insert(
            &Record::new(
                "r3",
                wings::DEPLOY,
                "redis",
                "deployed redis with error fallback",
                "test",
            ),
            Some(&[0.7, 0.3]),
        )
        .await
        .unwrap();

    (store, searcher)
}

#[tokio::test]
async fn keyword_search_finds_matching_content() {
    let (_store, searcher) = setup_store_with_records().await;

    let results = searcher
        .keyword_search("nginx", None, None, 5)
        .await
        .unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].record.id, "r1");
}

#[tokio::test]
async fn keyword_search_returns_empty_for_no_match() {
    let (_store, searcher) = setup_store_with_records().await;

    let results = searcher
        .keyword_search("nonexistent", None, None, 5)
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn keyword_search_filters_by_wing() {
    let (_store, searcher) = setup_store_with_records().await;

    let results = searcher
        .keyword_search("deployed", Some(wings::DEPLOY), None, 5)
        .await
        .unwrap();
    assert!(results.iter().all(|r| r.record.wing == wings::DEPLOY));
}

#[tokio::test]
async fn hybrid_search_returns_fused_results() {
    let (_store, searcher) = setup_store_with_records().await;

    let results = searcher
        .hybrid_search("deploy", None, None, 5)
        .await
        .unwrap();
    assert!(!results.is_empty());
    // First result should be most relevant to "deploy"
    assert_eq!(results[0].record.wing, wings::DEPLOY);
}

#[tokio::test]
async fn hybrid_search_respects_limit() {
    let (_store, searcher) = setup_store_with_records().await;

    let results = searcher
        .hybrid_search("deployed", None, None, 1)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
}
