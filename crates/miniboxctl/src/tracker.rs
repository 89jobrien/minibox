use crate::models::JobStatus;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

/// A log event broadcast to SSE subscribers.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum LogEvent {
    /// A chunk of container output.
    #[serde(rename = "output")]
    Output {
        stream: String,
        data: String,
        timestamp: String,
    },
    /// The container has stopped.
    #[serde(rename = "completed")]
    Completed { exit_code: Option<i32> },
}

/// In-memory job state tracker.
///
/// Stores [`JobStatus`] records keyed by job ID.  All mutations go through
/// async RwLock so the tracker can be shared across axum handlers.
///
/// Each job also has an optional broadcast channel for streaming log events
/// to SSE subscribers.
#[derive(Clone)]
pub struct JobTracker {
    jobs: Arc<RwLock<HashMap<String, JobStatus>>>,
    /// Broadcast senders for per-job log streams.  Created when a job starts;
    /// subscribers call [`subscribe`] to get a receiver.
    log_channels: Arc<RwLock<HashMap<String, broadcast::Sender<LogEvent>>>>,
}

/// Capacity of the per-job broadcast channel.  Late subscribers may miss
/// messages older than this window.
const LOG_CHANNEL_CAPACITY: usize = 256;

impl JobTracker {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            log_channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Insert (or overwrite) a job record.
    pub async fn insert(&self, status: JobStatus) {
        self.jobs
            .write()
            .await
            .insert(status.job_id.clone(), status);
    }

    /// Look up a job by ID.
    pub async fn get(&self, job_id: &str) -> Option<JobStatus> {
        self.jobs.read().await.get(job_id).cloned()
    }

    /// Update the status string and optional exit code of an existing job.
    ///
    /// Sets `completed_at` when status transitions to `"completed"` or
    /// `"failed"`.
    pub async fn update_status(&self, job_id: &str, status: &str, exit_code: Option<i32>) {
        if let Some(job) = self.jobs.write().await.get_mut(job_id) {
            job.status = status.to_string();
            job.exit_code = exit_code;
            if status == "completed" || status == "failed" || status == "timeout" {
                job.completed_at = Some(chrono::Utc::now().to_rfc3339());
            }
        }
    }

    /// Create a broadcast channel for the given job and return the sender.
    ///
    /// The sender is stored internally so that [`subscribe`] can hand out
    /// receivers to SSE clients.
    pub async fn create_log_channel(&self, job_id: &str) -> broadcast::Sender<LogEvent> {
        let (tx, _) = broadcast::channel(LOG_CHANNEL_CAPACITY);
        self.log_channels
            .write()
            .await
            .insert(job_id.to_string(), tx.clone());
        tx
    }

    /// Subscribe to the log stream for a job.  Returns `None` if the job has
    /// no active channel (either it was never created or the job already
    /// finished and the channel was cleaned up).
    pub async fn subscribe(&self, job_id: &str) -> Option<broadcast::Receiver<LogEvent>> {
        self.log_channels
            .read()
            .await
            .get(job_id)
            .map(|tx| tx.subscribe())
    }

    /// Remove the log channel for a finished job.
    pub async fn remove_log_channel(&self, job_id: &str) {
        self.log_channels.write().await.remove(job_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_status(job_id: &str) -> JobStatus {
        JobStatus {
            job_id: job_id.to_string(),
            container_id: Some("ctr-1".to_string()),
            status: "running".to_string(),
            exit_code: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            completed_at: None,
        }
    }

    #[tokio::test]
    async fn insert_and_get() {
        let tracker = JobTracker::new();
        let status = sample_status("j1");
        tracker.insert(status.clone()).await;
        let got = tracker.get("j1").await.expect("should exist");
        assert_eq!(got.job_id, "j1");
        assert_eq!(got.status, "running");
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let tracker = JobTracker::new();
        assert!(tracker.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn update_status_sets_completed_at() {
        let tracker = JobTracker::new();
        tracker.insert(sample_status("j2")).await;

        tracker.update_status("j2", "completed", Some(0)).await;

        let got = tracker.get("j2").await.expect("should exist");
        assert_eq!(got.status, "completed");
        assert_eq!(got.exit_code, Some(0));
        assert!(got.completed_at.is_some());
    }

    #[tokio::test]
    async fn update_status_on_missing_is_noop() {
        let tracker = JobTracker::new();
        // Should not panic
        tracker.update_status("ghost", "completed", Some(0)).await;
    }

    #[tokio::test]
    async fn log_channel_subscribe_receives_events() {
        let tracker = JobTracker::new();
        let tx = tracker.create_log_channel("j3").await;
        let mut rx = tracker.subscribe("j3").await.expect("channel exists");

        let event = LogEvent::Output {
            stream: "stdout".to_string(),
            data: "aGVsbG8=".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };
        tx.send(event).expect("send should succeed");

        let received = rx.recv().await.expect("should receive event");
        match received {
            LogEvent::Output { stream, data, .. } => {
                assert_eq!(stream, "stdout");
                assert_eq!(data, "aGVsbG8=");
            }
            _ => panic!("expected Output event"),
        }
    }

    #[tokio::test]
    async fn subscribe_missing_returns_none() {
        let tracker = JobTracker::new();
        assert!(tracker.subscribe("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn remove_log_channel_cleans_up() {
        let tracker = JobTracker::new();
        let _tx = tracker.create_log_channel("j4").await;
        assert!(tracker.subscribe("j4").await.is_some());
        tracker.remove_log_channel("j4").await;
        assert!(tracker.subscribe("j4").await.is_none());
    }
}
