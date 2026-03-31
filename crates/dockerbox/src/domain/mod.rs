pub mod error;

use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::mpsc;

pub use error::RuntimeError;

#[derive(Debug, Clone)]
pub struct CreateConfig {
    pub image: String,
    pub name: Option<String>,
    pub cmd: Vec<String>,
    pub env: Vec<String>,
    pub binds: Vec<String>,
    pub ports: Vec<PortBinding>,
    pub privileged: bool,
    pub cgroup_ns: CgroupNs,
    pub network_mode: Option<String>,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub enum CgroupNs {
    Host,
    Private,
}

#[derive(Debug, Clone)]
pub struct PortBinding {
    pub container_port: u16,
    pub protocol: String,
    pub host_ip: Option<String>,
    pub host_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct ContainerDetails {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub exit_code: Option<i64>,
    pub created: String,
    pub config: CreateConfig,
}

#[derive(Debug, Clone)]
pub struct ContainerSummary {
    pub id: String,
    pub names: Vec<String>,
    pub image: String,
    pub status: String,
    pub state: String,
}

#[derive(Debug)]
pub struct LogChunk {
    pub stream: u8,
    pub data: bytes::Bytes,
}

#[derive(Debug)]
pub struct PullProgress {
    pub status: String,
    pub id: Option<String>,
    pub progress: Option<String>,
}

#[async_trait]
pub trait ContainerRuntime: Send + Sync {
    async fn ping(&self) -> Result<(), RuntimeError>;
    async fn pull_image(
        &self,
        image: &str,
        tag: &str,
        tx: mpsc::Sender<PullProgress>,
    ) -> Result<(), RuntimeError>;
    async fn image_exists(&self, image: &str) -> Result<bool, RuntimeError>;
    async fn create_container(&self, config: CreateConfig) -> Result<String, RuntimeError>;
    async fn start_container(&self, id: &str) -> Result<(), RuntimeError>;
    async fn inspect_container(&self, id: &str) -> Result<ContainerDetails, RuntimeError>;
    async fn list_containers(&self, all: bool) -> Result<Vec<ContainerSummary>, RuntimeError>;
    async fn stream_logs(
        &self,
        id: &str,
        follow: bool,
        tx: mpsc::Sender<LogChunk>,
    ) -> Result<(), RuntimeError>;
    async fn stop_container(&self, id: &str, timeout_secs: u32) -> Result<(), RuntimeError>;
    async fn wait_container(&self, id: &str) -> Result<i64, RuntimeError>;
    async fn remove_container(&self, id: &str) -> Result<(), RuntimeError>;
}
