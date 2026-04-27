//! Tests for MemoryStore port — exercised against InMemoryStore adapter.

use minibox_memory::adapters::in_memory::InMemoryStore;
use minibox_memory::domain::{MemoryStore, Record, wings};

#[tokio::test]
async fn insert_and_get_roundtrip() {
    let store = InMemoryStore::new();
    let record = Record::new("r1", wings::DEPLOY, "nginx", "deployed v1.2", "miniboxd");

    let inserted = store.insert(&record, None).await.unwrap();
    assert!(inserted);

    let fetched = store.get("r1").await.unwrap().expect("record should exist");
    assert_eq!(fetched.id, "r1");
    assert_eq!(fetched.wing, wings::DEPLOY);
    assert_eq!(fetched.room, "nginx");
    assert_eq!(fetched.content, "deployed v1.2");
}

#[tokio::test]
async fn insert_duplicate_returns_false() {
    let store = InMemoryStore::new();
    let record = Record::new("r1", wings::DEPLOY, "nginx", "deployed v1.2", "miniboxd");

    assert!(store.insert(&record, None).await.unwrap());
    assert!(!store.insert(&record, None).await.unwrap());
}

#[tokio::test]
async fn exists_returns_false_for_missing() {
    let store = InMemoryStore::new();
    assert!(!store.exists("missing").await.unwrap());
}

#[tokio::test]
async fn delete_removes_record() {
    let store = InMemoryStore::new();
    let record = Record::new("r1", wings::ERROR, "oom", "OOM killed", "miniboxd");
    store.insert(&record, None).await.unwrap();

    assert!(store.delete("r1").await.unwrap());
    assert!(!store.exists("r1").await.unwrap());
}

#[tokio::test]
async fn delete_missing_returns_false() {
    let store = InMemoryStore::new();
    assert!(!store.delete("missing").await.unwrap());
}

#[tokio::test]
async fn fetch_filters_by_wing() {
    let store = InMemoryStore::new();
    store
        .insert(
            &Record::new("r1", wings::DEPLOY, "nginx", "deploy", "test"),
            None,
        )
        .await
        .unwrap();
    store
        .insert(
            &Record::new("r2", wings::ERROR, "oom", "error", "test"),
            None,
        )
        .await
        .unwrap();

    let results = store.fetch(Some(wings::DEPLOY), None, 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].wing, wings::DEPLOY);
}

#[tokio::test]
async fn fetch_filters_by_room() {
    let store = InMemoryStore::new();
    store
        .insert(
            &Record::new("r1", wings::HEALTH, "cpu", "cpu ok", "test"),
            None,
        )
        .await
        .unwrap();
    store
        .insert(
            &Record::new("r2", wings::HEALTH, "mem", "mem ok", "test"),
            None,
        )
        .await
        .unwrap();

    let results = store
        .fetch(Some(wings::HEALTH), Some("cpu"), 10)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].room, "cpu");
}

#[tokio::test]
async fn count_reflects_inserts_and_deletes() {
    let store = InMemoryStore::new();
    assert_eq!(store.count().await.unwrap(), 0);

    store
        .insert(
            &Record::new("r1", wings::CONFIG, "dns", "changed", "test"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(store.count().await.unwrap(), 1);

    store.delete("r1").await.unwrap();
    assert_eq!(store.count().await.unwrap(), 0);
}

#[tokio::test]
async fn taxonomy_returns_wing_and_room_counts() {
    let store = InMemoryStore::new();
    store
        .insert(
            &Record::new("r1", wings::DEPLOY, "nginx", "d1", "test"),
            None,
        )
        .await
        .unwrap();
    store
        .insert(
            &Record::new("r2", wings::DEPLOY, "redis", "d2", "test"),
            None,
        )
        .await
        .unwrap();
    store
        .insert(&Record::new("r3", wings::ERROR, "oom", "e1", "test"), None)
        .await
        .unwrap();

    let (wings_list, rooms_list) = store.taxonomy().await.unwrap();

    assert_eq!(wings_list.len(), 2);
    let deploy_count = wings_list
        .iter()
        .find(|(w, _)| w == wings::DEPLOY)
        .map(|(_, c)| *c)
        .unwrap();
    assert_eq!(deploy_count, 2);

    assert_eq!(rooms_list.len(), 3);
}
