use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

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

#[derive(Clone, Default)]
pub struct StateStore {
    pub networks: Arc<RwLock<HashMap<String, NetworkRecord>>>,
    pub volumes: Arc<RwLock<HashMap<String, VolumeRecord>>>,
}
