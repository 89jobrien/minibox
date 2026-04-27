//! Tests for the InfraMemory 3-layer stack.

use minibox_memory::adapters::in_memory::InMemoryStore;
use minibox_memory::domain::{MemoryStore, Record, wings};
use minibox_memory::layers::InfraMemory;
use std::sync::Arc;

fn make_memory(store: InMemoryStore) -> InfraMemory<InMemoryStore> {
    InfraMemory::new(Arc::new(store))
}

#[tokio::test]
async fn file_record_inserts_and_returns_id() {
    let store = InMemoryStore::new();
    let mem = make_memory(store);

    let id = mem
        .file_record(wings::DEPLOY, "nginx", "deployed v1.2.3", "miniboxd")
        .await
        .unwrap();

    assert!(!id.is_empty());
    assert!(mem.store().exists(&id).await.unwrap());
}

#[tokio::test]
async fn l1_recent_ops_includes_filed_records() {
    let store = InMemoryStore::new();
    let arc_store = Arc::new(store);
    let mem = InfraMemory::new(arc_store.clone());

    arc_store
        .insert(
            &Record::new("r1", wings::DEPLOY, "nginx", "deployed v1", "test"),
            None,
        )
        .await
        .unwrap();

    let summary = mem.recent_ops(None).await.unwrap();
    assert!(summary.contains("deployed v1"));
}

#[tokio::test]
async fn l1_recent_ops_filters_by_wing() {
    let store = InMemoryStore::new();
    let arc_store = Arc::new(store);
    let mem = InfraMemory::new(arc_store.clone());

    arc_store
        .insert(
            &Record::new("r1", wings::DEPLOY, "nginx", "deploy event", "test"),
            None,
        )
        .await
        .unwrap();
    arc_store
        .insert(
            &Record::new("r2", wings::ERROR, "oom", "error event", "test"),
            None,
        )
        .await
        .unwrap();

    let summary = mem.recent_ops(Some(wings::ERROR)).await.unwrap();
    assert!(summary.contains("error event"));
    assert!(!summary.contains("deploy event"));
}

#[tokio::test]
async fn status_reports_record_count_and_taxonomy() {
    let store = InMemoryStore::new();
    let arc_store = Arc::new(store);
    let mem = InfraMemory::new(arc_store.clone());

    arc_store
        .insert(
            &Record::new("r1", wings::HEALTH, "cpu", "cpu ok", "test"),
            None,
        )
        .await
        .unwrap();

    let status = mem.status().await.unwrap();
    assert_eq!(status["total_records"], 1);
}
