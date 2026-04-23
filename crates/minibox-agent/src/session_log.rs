//! Session logger — appends JSONL records to `~/.minibox/sessions/`.
//!
//! Each [`SessionRecord`] is a single line of JSON (newline-terminated). The
//! logger creates the directory on first write.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error from the session logger.
#[derive(Debug, Error)]
pub enum SessionLogError {
    /// Could not create the log directory.
    #[error("failed to create session log directory: {0}")]
    DirCreate(String),
    /// Could not write a record.
    #[error("failed to write session record: {0}")]
    Write(String),
    /// Record serialization failed.
    #[error("failed to serialize session record: {0}")]
    Serialize(String),
}

/// A single log record written to the JSONL file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    /// ISO-8601 timestamp of the record.
    pub timestamp: String,
    /// Session identifier.
    pub session_id: String,
    /// Free-form event kind, e.g. `"turn_start"`, `"tool_use"`, `"turn_end"`.
    pub event: String,
    /// Optional structured payload.
    pub payload: Option<serde_json::Value>,
}

impl SessionRecord {
    /// Create a record with the current UTC timestamp.
    pub fn now(
        session_id: impl Into<String>,
        event: impl Into<String>,
        payload: Option<serde_json::Value>,
    ) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            session_id: session_id.into(),
            event: event.into(),
            payload,
        }
    }
}

/// Appends [`SessionRecord`]s as JSONL to a per-session file.
pub struct SessionLogger {
    log_path: PathBuf,
}

impl SessionLogger {
    /// Build with the default path: `~/.minibox/sessions/<session_id>.jsonl`.
    pub fn new(session_id: &str) -> Result<Self, SessionLogError> {
        let base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".minibox")
            .join("sessions");
        Self::with_dir(&base, session_id)
    }

    /// Build with an explicit directory (useful in tests).
    pub fn with_dir(dir: &Path, session_id: &str) -> Result<Self, SessionLogError> {
        fs::create_dir_all(dir).map_err(|e| SessionLogError::DirCreate(e.to_string()))?;
        let log_path = dir.join(format!("{session_id}.jsonl"));
        Ok(Self { log_path })
    }

    /// Append a single [`SessionRecord`] as a JSON line.
    pub fn append(&self, record: &SessionRecord) -> Result<(), SessionLogError> {
        let line =
            serde_json::to_string(record).map_err(|e| SessionLogError::Serialize(e.to_string()))?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .map_err(|e| SessionLogError::Write(e.to_string()))?;

        writeln!(file, "{line}").map_err(|e| SessionLogError::Write(e.to_string()))
    }

    /// Return the path where records are written (useful in tests).
    pub fn path(&self) -> &Path {
        &self.log_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_logger_writes_valid_jsonl() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let logger = SessionLogger::with_dir(dir.path(), "test-session-001").expect("logger");

        let r1 = SessionRecord::now("test-session-001", "turn_start", None);
        let r2 = SessionRecord::now(
            "test-session-001",
            "tool_use",
            Some(serde_json::json!({"tool": "bash"})),
        );
        logger.append(&r1).expect("append r1");
        logger.append(&r2).expect("append r2");

        let content = std::fs::read_to_string(logger.path()).expect("read log");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "should have 2 JSONL lines");

        for line in &lines {
            serde_json::from_str::<serde_json::Value>(line).expect("each line must be valid JSON");
        }
    }

    #[test]
    fn session_record_roundtrips_through_json() {
        let record = SessionRecord::now(
            "sess-abc",
            "turn_end",
            Some(serde_json::json!({"tokens": 42})),
        );
        let json = serde_json::to_string(&record).expect("serialize");
        let back: SessionRecord = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.session_id, record.session_id);
        assert_eq!(back.event, record.event);
    }

    #[test]
    fn session_logger_creates_directory() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let subdir = dir.path().join("nested").join("dir");
        // Directory does not exist yet
        assert!(!subdir.exists());

        let logger = SessionLogger::with_dir(&subdir, "s1").expect("logger");
        logger
            .append(&SessionRecord::now("s1", "test", None))
            .expect("append");

        assert!(subdir.exists());
        assert!(logger.path().exists());
    }
}
