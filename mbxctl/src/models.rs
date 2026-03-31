use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJobRequest {
    pub image: String,
    pub tag: Option<String>,
    pub command: Vec<String>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_weight: Option<u64>,
    pub stream_output: Option<bool>,
    pub timeout_seconds: Option<u64>,
    #[serde(default)]
    pub env: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJobResponse {
    pub job_id: String,
    pub container_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatus {
    pub job_id: String,
    pub container_id: Option<String>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

#[allow(dead_code)] // reserved for future log retrieval API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub stream: String,
    pub data: String,
    pub timestamp: String,
}
