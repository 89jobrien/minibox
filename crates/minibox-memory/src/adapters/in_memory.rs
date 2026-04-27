//! In-memory adapter for MemoryStore — test double.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::domain::{MemoryError, MemoryStore, Record};

/// In-memory store for unit testing. Not for production use.
pub struct InMemoryStore {
    records: Arc<RwLock<HashMap<String, Record>>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl MemoryStore for InMemoryStore {
    async fn insert(
        &self,
        record: &Record,
        _embedding: Option<&[f32]>,
    ) -> Result<bool, MemoryError> {
        let mut map = self.records.write().await;
        if map.contains_key(&record.id) {
            return Ok(false);
        }
        map.insert(record.id.clone(), record.clone());
        Ok(true)
    }

    async fn exists(&self, id: &str) -> Result<bool, MemoryError> {
        Ok(self.records.read().await.contains_key(id))
    }

    async fn get(&self, id: &str) -> Result<Option<Record>, MemoryError> {
        Ok(self.records.read().await.get(id).cloned())
    }

    async fn delete(&self, id: &str) -> Result<bool, MemoryError> {
        Ok(self.records.write().await.remove(id).is_some())
    }

    async fn fetch(
        &self,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Record>, MemoryError> {
        let map = self.records.read().await;
        let mut results: Vec<Record> = map
            .values()
            .filter(|r| wing.is_none() || wing == Some(r.wing.as_str()))
            .filter(|r| room.is_none() || room == Some(r.room.as_str()))
            .cloned()
            .collect();
        results.sort_by(|a, b| b.recorded_at.cmp(&a.recorded_at));
        results.truncate(limit);
        Ok(results)
    }

    async fn count(&self) -> Result<usize, MemoryError> {
        Ok(self.records.read().await.len())
    }

    async fn taxonomy(&self) -> Result<(Vec<(String, usize)>, Vec<(String, usize)>), MemoryError> {
        let map = self.records.read().await;
        let mut wing_counts: HashMap<String, usize> = HashMap::new();
        let mut room_counts: HashMap<String, usize> = HashMap::new();
        for r in map.values() {
            *wing_counts.entry(r.wing.clone()).or_default() += 1;
            *room_counts.entry(r.room.clone()).or_default() += 1;
        }
        let mut wings: Vec<_> = wing_counts.into_iter().collect();
        let mut rooms: Vec<_> = room_counts.into_iter().collect();
        wings.sort_by(|a, b| a.0.cmp(&b.0));
        rooms.sort_by(|a, b| a.0.cmp(&b.0));
        Ok((wings, rooms))
    }
}
