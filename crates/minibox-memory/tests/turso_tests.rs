//! Tests for the Turso/SQLite MemoryStore adapter.

use minibox_memory::adapters::turso::TursoStore;
use minibox_memory::domain::{MemoryStore, Record, wings};

#[tokio::test]
async fn turso_insert_and_get_roundtrip() {
    let store = TursoStore::memory().await.unwrap();
    let record = Record::new("t1", wings::DEPLOY, "nginx", "deployed v2.0", "miniboxd");

    assert!(store.insert(&record, None).await.unwrap());

    let fetched = store.get("t1").await.unwrap().expect("should exist");
    assert_eq!(fetched.id, "t1");
    assert_eq!(fetched.wing, wings::DEPLOY);
    assert_eq!(fetched.content, "deployed v2.0");
}

#[tokio::test]
async fn turso_insert_duplicate_returns_false() {
    let store = TursoStore::memory().await.unwrap();
    let record = Record::new("t1", wings::DEPLOY, "nginx", "deployed", "test");

    assert!(store.insert(&record, None).await.unwrap());
    assert!(!store.insert(&record, None).await.unwrap());
}

#[tokio::test]
async fn turso_delete_removes_record() {
    let store = TursoStore::memory().await.unwrap();
    let record = Record::new("t1", wings::ERROR, "oom", "OOM killed", "test");
    store.insert(&record, None).await.unwrap();

    assert!(store.delete("t1").await.unwrap());
    assert!(!store.exists("t1").await.unwrap());
}

#[tokio::test]
async fn turso_fetch_filters_by_wing() {
    let store = TursoStore::memory().await.unwrap();
    store
        .insert(
            &Record::new("t1", wings::DEPLOY, "nginx", "deploy", "test"),
            None,
        )
        .await
        .unwrap();
    store
        .insert(
            &Record::new("t2", wings::ERROR, "oom", "error", "test"),
            None,
        )
        .await
        .unwrap();

    let results = store.fetch(Some(wings::DEPLOY), None, 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].wing, wings::DEPLOY);
}

#[tokio::test]
async fn turso_count_and_taxonomy() {
    let store = TursoStore::memory().await.unwrap();
    store
        .insert(
            &Record::new("t1", wings::DEPLOY, "nginx", "d1", "test"),
            None,
        )
        .await
        .unwrap();
    store
        .insert(
            &Record::new("t2", wings::DEPLOY, "redis", "d2", "test"),
            None,
        )
        .await
        .unwrap();
    store
        .insert(&Record::new("t3", wings::HEALTH, "cpu", "h1", "test"), None)
        .await
        .unwrap();

    assert_eq!(store.count().await.unwrap(), 3);

    let (wing_counts, room_counts) = store.taxonomy().await.unwrap();
    assert_eq!(wing_counts.len(), 2);
    assert_eq!(room_counts.len(), 3);
}

#[tokio::test]
async fn turso_insert_with_embedding() {
    let store = TursoStore::memory().await.unwrap();
    let record = Record::new("t1", wings::DEPLOY, "nginx", "deployed", "test");
    let embedding = vec![0.1, 0.2, 0.3];

    assert!(store.insert(&record, Some(&embedding)).await.unwrap());

    let fetched = store.get("t1").await.unwrap().expect("should exist");
    assert_eq!(fetched.id, "t1");
}
