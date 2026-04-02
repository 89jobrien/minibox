//! Lease service: protect images from GC during in-flight operations.

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;
use uuid::Uuid;

/// A lease protecting one or more image refs from garbage collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub id:         String,
    pub created_at: SystemTime,
    pub expire_at:  SystemTime,
    /// Image `"name:tag"` strings protected by this lease.
    pub image_refs: HashSet<String>,
}

/// Port: lease lifecycle management.
#[async_trait]
pub trait ImageLeaseService: Send + Sync {
    /// Protect `image_ref` from GC for `ttl`. Returns the new lease ID.
    async fn acquire(&self, image_ref: &str, ttl: Duration) -> Result<String>;
    /// Release a lease early (image can now be GC'd if not otherwise protected).
    async fn release(&self, lease_id: &str) -> Result<()>;
    /// Extend a lease's expiry by an additional `ttl`.
    async fn extend(&self, lease_id: &str, ttl: Duration) -> Result<()>;
    /// All leases (including expired).
    async fn list(&self) -> Result<Vec<LeaseRecord>>;
    /// Only non-expired leases.
    async fn list_active(&self) -> Result<Vec<LeaseRecord>>;
    /// Returns true if any active lease covers `image_ref`.
    async fn is_leased(&self, image_ref: &str) -> Result<bool>;
}

/// Disk-backed lease service. Persists to a single JSON file.
pub struct DiskLeaseService {
    leases: Arc<RwLock<HashMap<String, LeaseRecord>>>,
    path:   PathBuf,
}

impl DiskLeaseService {
    pub async fn new(path: PathBuf) -> Result<Self> {
        let leases = if path.exists() {
            let bytes = tokio::fs::read(&path).await
                .with_context(|| format!("lease: read {}", path.display()))?;
            serde_json::from_slice(&bytes)
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        Ok(Self {
            leases: Arc::new(RwLock::new(leases)),
            path,
        })
    }

    async fn persist(&self) -> Result<()> {
        let leases = self.leases.read().await;
        let bytes = serde_json::to_vec_pretty(&*leases)?;
        let tmp = self.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }
}

#[async_trait]
impl ImageLeaseService for DiskLeaseService {
    async fn acquire(&self, image_ref: &str, ttl: Duration) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = SystemTime::now();
        let record = LeaseRecord {
            id: id.clone(),
            created_at: now,
            expire_at: now + ttl,
            image_refs: std::iter::once(image_ref.to_string()).collect(),
        };
        self.leases.write().await.insert(id.clone(), record);
        self.persist().await?;
        Ok(id)
    }

    async fn release(&self, lease_id: &str) -> Result<()> {
        self.leases.write().await.remove(lease_id);
        self.persist().await
    }

    async fn extend(&self, lease_id: &str, ttl: Duration) -> Result<()> {
        let mut leases = self.leases.write().await;
        if let Some(l) = leases.get_mut(lease_id) {
            l.expire_at = SystemTime::now() + ttl;
            drop(leases);
            self.persist().await?;
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<LeaseRecord>> {
        Ok(self.leases.read().await.values().cloned().collect())
    }

    async fn list_active(&self) -> Result<Vec<LeaseRecord>> {
        let now = SystemTime::now();
        Ok(self.leases.read().await.values()
            .filter(|l| l.expire_at > now)
            .cloned()
            .collect())
    }

    async fn is_leased(&self, image_ref: &str) -> Result<bool> {
        let now = SystemTime::now();
        Ok(self.leases.read().await.values().any(|l| {
            l.expire_at > now && l.image_refs.contains(image_ref)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_acquire_and_release() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = DiskLeaseService::new(tmp.path().join("leases.json")).await.unwrap();

        let lease_id = svc.acquire("alpine:latest", Duration::from_secs(3600)).await.unwrap();
        let leases = svc.list().await.unwrap();
        assert_eq!(leases.len(), 1);
        assert!(leases[0].image_refs.contains("alpine:latest"));

        svc.release(&lease_id).await.unwrap();
        let leases = svc.list().await.unwrap();
        assert!(leases.is_empty());
    }

    #[tokio::test]
    async fn test_expired_lease_not_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = DiskLeaseService::new(tmp.path().join("leases.json")).await.unwrap();

        // Acquire with 0-second TTL (immediately expired)
        let _id = svc.acquire("old:image", Duration::from_secs(0)).await.unwrap();
        let active = svc.list_active().await.unwrap();
        assert!(active.is_empty());
    }
}
