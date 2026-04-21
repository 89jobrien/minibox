use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::domain::ExecConfig;

#[derive(Debug, Clone)]
pub struct NetworkRecord {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub created: String,
}

#[derive(Debug, Clone)]
pub struct VolumeRecord {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    pub created: String,
}

#[derive(Debug, Clone)]
pub struct ExecRecord {
    pub container_id: String,
    pub config: ExecConfig,
    pub exit_code: Option<i64>,
    pub running: bool,
}

#[derive(Clone, Default)]
pub struct StateStore {
    pub networks: Arc<RwLock<HashMap<String, NetworkRecord>>>,
    pub volumes: Arc<RwLock<HashMap<String, VolumeRecord>>>,
    pub execs: Arc<RwLock<HashMap<String, ExecRecord>>>,
}
