//! Turso/SQLite adapter for MemoryStore.
//!
//! Uses Turso's in-process SQLite engine with:
//! - `vector32()` BLOB column for embedding storage
//! - Standard SQL for keyword search (LIKE-based)
//! - `vector_distance_cos()` for semantic search (when embeddings present)

use std::path::Path;

use crate::domain::{MemoryError, MemoryStore, Record};

/// Production MemoryStore backed by Turso (embedded SQLite + vector support).
pub struct TursoStore {
    db: turso::Database,
}

impl TursoStore {
    /// Open a file-backed store, creating the DB and tables if needed.
    pub async fn open(path: &Path) -> Result<Self, MemoryError> {
        let db = turso::Builder::new_local(&path.to_string_lossy())
            .build()
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?;
        let store = Self { db };
        store.migrate().await?;
        Ok(store)
    }

    /// In-memory store for testing.
    pub async fn memory() -> Result<Self, MemoryError> {
        let db = turso::Builder::new_local(":memory:")
            .build()
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?;
        let store = Self { db };
        store.migrate().await?;
        Ok(store)
    }

    fn conn(&self) -> Result<turso::Connection, MemoryError> {
        self.db
            .connect()
            .map_err(|e| MemoryError::Store(e.to_string()))
    }

    async fn migrate(&self) -> Result<(), MemoryError> {
        let conn = self.conn()?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS records (
                id TEXT PRIMARY KEY,
                wing TEXT NOT NULL,
                room TEXT NOT NULL,
                content TEXT NOT NULL,
                source TEXT,
                recorded_by TEXT NOT NULL,
                recorded_at TEXT NOT NULL,
                embedding BLOB
            )",
            (),
        )
        .await
        .map_err(|e| MemoryError::Store(e.to_string()))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl MemoryStore for TursoStore {
    async fn insert(
        &self,
        record: &Record,
        embedding: Option<&[f32]>,
    ) -> Result<bool, MemoryError> {
        let conn = self.conn()?;

        let emb_json: Option<String> = embedding.map(to_vector32_json);

        let result = conn
            .execute(
                "INSERT OR IGNORE INTO records (id, wing, room, content, source, recorded_by, recorded_at, embedding)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CASE WHEN ?8 IS NOT NULL THEN vector32(?8) ELSE NULL END)",
                turso::params![
                    record.id.as_str(),
                    record.wing.as_str(),
                    record.room.as_str(),
                    record.content.as_str(),
                    record.source.as_deref().unwrap_or(""),
                    record.recorded_by.as_str(),
                    record.recorded_at.as_str(),
                    emb_json.as_deref(),
                ],
            )
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?;

        Ok(result > 0)
    }

    async fn exists(&self, id: &str) -> Result<bool, MemoryError> {
        let conn = self.conn()?;
        let mut rows = conn
            .query("SELECT 1 FROM records WHERE id = ?1", turso::params![id])
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?;

        Ok(rows
            .next()
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?
            .is_some())
    }

    async fn get(&self, id: &str) -> Result<Option<Record>, MemoryError> {
        let conn = self.conn()?;
        let mut rows = conn
            .query(
                "SELECT id, wing, room, content, source, recorded_by, recorded_at
                 FROM records WHERE id = ?1",
                turso::params![id],
            )
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_record(&row)?)),
            None => Ok(None),
        }
    }

    async fn delete(&self, id: &str) -> Result<bool, MemoryError> {
        let conn = self.conn()?;
        let affected = conn
            .execute("DELETE FROM records WHERE id = ?1", turso::params![id])
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?;
        Ok(affected > 0)
    }

    async fn fetch(
        &self,
        wing: Option<&str>,
        room: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Record>, MemoryError> {
        let conn = self.conn()?;

        let (sql, params) = build_fetch_sql(wing, room, limit);

        let mut rows = conn
            .query(&sql, params)
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?
        {
            results.push(row_to_record(&row)?);
        }
        Ok(results)
    }

    async fn count(&self) -> Result<usize, MemoryError> {
        let conn = self.conn()?;
        let mut rows = conn
            .query("SELECT COUNT(*) FROM records", ())
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?;

        let row = rows
            .next()
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?
            .ok_or_else(|| MemoryError::Store("no count result".into()))?;

        let count = match row
            .get_value(0)
            .map_err(|e| MemoryError::Store(e.to_string()))?
        {
            turso::Value::Integer(n) => n as usize,
            _ => 0,
        };
        Ok(count)
    }

    async fn taxonomy(&self) -> Result<(Vec<(String, usize)>, Vec<(String, usize)>), MemoryError> {
        let conn = self.conn()?;

        let mut wing_rows = conn
            .query(
                "SELECT wing, COUNT(*) as cnt FROM records GROUP BY wing ORDER BY wing",
                (),
            )
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?;

        let mut wings = Vec::new();
        while let Some(row) = wing_rows
            .next()
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?
        {
            let wing = val_str(&row, 0);
            let count = val_usize(&row, 1);
            wings.push((wing, count));
        }

        let mut room_rows = conn
            .query(
                "SELECT room, COUNT(*) as cnt FROM records GROUP BY room ORDER BY room",
                (),
            )
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?;

        let mut rooms = Vec::new();
        while let Some(row) = room_rows
            .next()
            .await
            .map_err(|e| MemoryError::Store(e.to_string()))?
        {
            let room = val_str(&row, 0);
            let count = val_usize(&row, 1);
            rooms.push((room, count));
        }

        Ok((wings, rooms))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn row_to_record(row: &turso::Row) -> Result<Record, MemoryError> {
    let source_raw = val_str(row, 4);
    Ok(Record {
        id: val_str(row, 0),
        wing: val_str(row, 1),
        room: val_str(row, 2),
        content: val_str(row, 3),
        source: if source_raw.is_empty() {
            None
        } else {
            Some(source_raw)
        },
        recorded_by: val_str(row, 5),
        recorded_at: val_str(row, 6),
    })
}

fn val_str(row: &turso::Row, idx: usize) -> String {
    match row.get_value(idx) {
        Ok(turso::Value::Text(s)) => s,
        Ok(turso::Value::Null) | Err(_) => String::new(),
        Ok(v) => format!("{v:?}"),
    }
}

fn val_usize(row: &turso::Row, idx: usize) -> usize {
    match row.get_value(idx) {
        Ok(turso::Value::Integer(n)) => n as usize,
        _ => 0,
    }
}

fn to_vector32_json(v: &[f32]) -> String {
    let nums: Vec<String> = v.iter().map(|f| f.to_string()).collect();
    format!("[{}]", nums.join(","))
}

fn build_fetch_sql(
    wing: Option<&str>,
    room: Option<&str>,
    limit: usize,
) -> (String, Vec<turso::Value>) {
    let mut conditions = Vec::new();
    let mut params: Vec<turso::Value> = Vec::new();
    let mut idx = 1;

    if let Some(w) = wing {
        conditions.push(format!("wing = ?{idx}"));
        params.push(turso::Value::Text(w.to_string()));
        idx += 1;
    }
    if let Some(r) = room {
        conditions.push(format!("room = ?{idx}"));
        params.push(turso::Value::Text(r.to_string()));
        idx += 1;
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    params.push(turso::Value::Integer(limit as i64));

    let sql = format!(
        "SELECT id, wing, room, content, source, recorded_by, recorded_at
         FROM records {where_clause}
         ORDER BY recorded_at DESC
         LIMIT ?{idx}"
    );

    (sql, params)
}
