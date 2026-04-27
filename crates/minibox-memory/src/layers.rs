//! 3-layer infrastructure memory stack.
//!
//! L0: Host profile (static text file)
//! L1: Recent ops summary (top N records grouped by room)
//! L2: Deep hybrid search (delegated to MemorySearcher)

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::domain::{MemoryError, MemoryStore, Record};

const L1_MAX_RECORDS: usize = 15;
const L1_MAX_CHARS: usize = 3200;

/// Full infrastructure memory stack, generic over the store port.
pub struct InfraMemory<S: MemoryStore> {
    store: Arc<S>,
}

impl<S: MemoryStore> InfraMemory<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    /// Access the underlying store (for direct queries).
    pub fn store(&self) -> &S {
        &self.store
    }

    /// File a new infrastructure record, returning its generated ID.
    pub async fn file_record(
        &self,
        wing: &str,
        room: &str,
        content: &str,
        recorded_by: &str,
    ) -> Result<String, MemoryError> {
        let now = chrono::Utc::now().to_rfc3339();
        let digest = md5::compute(format!("{wing}{room}{content}{now}").as_bytes());
        let id = format!("mem_{wing}_{digest:x}");

        let record = Record::new(&id, wing, room, content, recorded_by);
        self.store.insert(&record, None).await?;
        Ok(id)
    }

    /// L1: Recent ops summary -- top N records grouped by room.
    pub async fn recent_ops(&self, wing: Option<&str>) -> Result<String, MemoryError> {
        let records = self.store.fetch(wing, None, 200).await?;

        if records.is_empty() {
            return Ok("## L1 -- No records yet.".to_string());
        }

        let mut by_room: HashMap<String, Vec<&Record>> = HashMap::new();
        for r in records.iter().take(L1_MAX_RECORDS) {
            by_room.entry(r.room.clone()).or_default().push(r);
        }

        let mut lines = vec!["## L1 -- RECENT OPS".to_string()];
        let mut total_len = 0usize;

        let mut rooms: Vec<_> = by_room.iter().collect();
        rooms.sort_by_key(|(r, _)| r.as_str());

        for (room, entries) in rooms {
            let room_line = format!("\n[{}]", room);
            lines.push(room_line.clone());
            total_len += room_line.len();

            for r in entries {
                let snippet: String = r
                    .content
                    .trim()
                    .replace('\n', " ")
                    .chars()
                    .take(200)
                    .collect();
                let snippet = if r.content.len() > 200 {
                    format!("{snippet}...")
                } else {
                    snippet
                };

                let source_tag = r
                    .source
                    .as_deref()
                    .and_then(|s| Path::new(s).file_name())
                    .map(|n| format!("  ({})", n.to_string_lossy()))
                    .unwrap_or_default();

                let entry = format!("  - {snippet}{source_tag}");

                if total_len + entry.len() > L1_MAX_CHARS {
                    lines.push("  ... (more in L2 search)".to_string());
                    return Ok(lines.join("\n"));
                }
                total_len += entry.len();
                lines.push(entry);
            }
        }

        Ok(lines.join("\n"))
    }

    /// Status of the memory store.
    pub async fn status(&self) -> Result<serde_json::Value, MemoryError> {
        let count = self.store.count().await?;
        let (wings, rooms) = self.store.taxonomy().await?;

        Ok(serde_json::json!({
            "total_records": count,
            "wings": wings.iter().map(|(w, c)| serde_json::json!({"wing": w, "count": c})).collect::<Vec<_>>(),
            "rooms": rooms.iter().map(|(r, c)| serde_json::json!({"room": r, "count": c})).collect::<Vec<_>>(),
        }))
    }
}
