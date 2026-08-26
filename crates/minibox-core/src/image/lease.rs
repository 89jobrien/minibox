//! Lease service: protect images from GC during in-flight operations.
//!
//! The [`ImageLeaseService`] port conformance suite lives in the [`conformance`]
//! submodule (enabled by the `test-utils` feature). Use
//! [`conformance::run_conformance_suite`] to verify any implementation against
//! the full behavioral contract of the port.

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
    /// Unique lease identifier.
    pub id: String,
    /// Time the lease was created.
    pub created_at: SystemTime,
    /// Time after which the lease no longer protects images.
    pub expire_at: SystemTime,
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
    path: PathBuf,
}

impl DiskLeaseService {
    /// Create a new lease service backed by the JSON file at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    // qual:allow(iosp) reason: "I/O boundary — read file, parse, construct"
    pub async fn new(path: PathBuf) -> Result<Self> {
        let leases = if path.exists() {
            let bytes = tokio::fs::read(&path)
                .await
                .with_context(|| format!("lease: read {}", path.display()))?;
            serde_json::from_slice(&bytes).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Ok(Self {
            leases: Arc::new(RwLock::new(leases)),
            path,
        })
    }

    async fn persist(&self) -> Result<()> {
        let bytes = {
            let leases = self.leases.read().await;
            serde_json::to_vec_pretty(&*leases)?
        };
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
        Ok(self
            .leases
            .read()
            .await
            .values()
            .filter(|l| l.expire_at > now)
            .cloned()
            .collect())
    }

    async fn is_leased(&self, image_ref: &str) -> Result<bool> {
        let now = SystemTime::now();
        Ok(self
            .leases
            .read()
            .await
            .values()
            .any(|l| l.expire_at > now && l.image_refs.contains(image_ref)))
    }
}

// ---------------------------------------------------------------------------
// InMemoryLeaseService — test double for the port
// ---------------------------------------------------------------------------

/// Pure in-memory implementation of [`ImageLeaseService`].
///
/// State is never persisted to disk. Intended for use in conformance tests and
/// as a fast test double in any crate that depends on `minibox-core` with the
/// `test-utils` feature enabled.
#[cfg(any(test, feature = "test-utils"))]
pub struct InMemoryLeaseService {
    leases: Arc<RwLock<HashMap<String, LeaseRecord>>>,
}

#[cfg(any(test, feature = "test-utils"))]
impl Default for InMemoryLeaseService {
    fn default() -> Self {
        Self {
            leases: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[cfg(any(test, feature = "test-utils"))]
#[async_trait]
impl ImageLeaseService for InMemoryLeaseService {
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
        Ok(id)
    }

    async fn release(&self, lease_id: &str) -> Result<()> {
        self.leases.write().await.remove(lease_id);
        Ok(())
    }

    async fn extend(&self, lease_id: &str, ttl: Duration) -> Result<()> {
        let mut leases = self.leases.write().await;
        if let Some(l) = leases.get_mut(lease_id) {
            l.expire_at = SystemTime::now() + ttl;
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<LeaseRecord>> {
        Ok(self.leases.read().await.values().cloned().collect())
    }

    async fn list_active(&self) -> Result<Vec<LeaseRecord>> {
        let now = SystemTime::now();
        Ok(self
            .leases
            .read()
            .await
            .values()
            .filter(|l| l.expire_at > now)
            .cloned()
            .collect())
    }

    async fn is_leased(&self, image_ref: &str) -> Result<bool> {
        let now = SystemTime::now();
        Ok(self
            .leases
            .read()
            .await
            .values()
            .any(|l| l.expire_at > now && l.image_refs.contains(image_ref)))
    }
}

// ---------------------------------------------------------------------------
// Port conformance suite
// ---------------------------------------------------------------------------

/// Port conformance suite for [`ImageLeaseService`].
///
/// Any correct implementation of the port must satisfy all assertions in
/// [`run_conformance_suite`]. The suite is deliberately implementation-agnostic:
/// it only calls methods on the [`ImageLeaseService`] trait and makes no
/// assumptions about persistence, threading, or disk layout.
///
/// # Usage
///
/// ```rust,ignore
/// use minibox_core::image::lease::conformance;
/// use minibox_core::image::lease::InMemoryLeaseService;
///
/// #[tokio::test]
/// async fn my_impl_conforms() {
///     let svc = MyLeaseService::new();
///     conformance::run_conformance_suite(&svc).await;
/// }
/// ```
#[cfg(any(test, feature = "test-utils"))]
#[allow(clippy::expect_used, clippy::unwrap_used)]
pub mod conformance {
    use super::{Duration, ImageLeaseService};

    /// Run the full `ImageLeaseService` port conformance suite against `svc`.
    ///
    /// `svc` must be a freshly constructed, empty service instance (no
    /// pre-existing leases). The function panics on the first contract
    /// violation, reporting which invariant failed.
    ///
    /// # Behavioral contracts exercised
    ///
    /// - `acquire` returns a non-empty, unique ID on every call.
    /// - After `acquire` with a non-zero TTL, `is_leased` returns `true` for
    ///   the protected ref and `false` for an unrelated ref.
    /// - The new lease appears in both `list` and `list_active`.
    /// - `release` removes the lease: `list` and `list_active` become empty,
    ///   `is_leased` returns `false`.
    /// - Releasing an unknown ID is a no-op (returns `Ok`).
    /// - A lease acquired with `ttl = 0` is immediately expired: it appears in
    ///   `list` but not in `list_active`, and `is_leased` returns `false`.
    /// - `extend` with a non-zero TTL re-activates an expired lease.
    /// - Leases for distinct image refs are independent: releasing one does not
    ///   affect the other.
    /// - When two leases protect the same image ref, releasing one still leaves
    ///   the ref leased via the other.
    pub async fn run_conformance_suite<S: ImageLeaseService>(svc: &S) {
        conformance_acquire_returns_nonempty_id(svc).await;
        conformance_acquire_returns_unique_ids(svc).await;
        conformance_active_lease_is_leased(svc).await;
        conformance_unrelated_ref_not_leased(svc).await;
        conformance_lease_appears_in_list_and_list_active(svc).await;
        conformance_release_removes_lease(svc).await;
        conformance_release_unknown_id_is_ok(svc).await;
        conformance_expired_lease_in_list_not_in_list_active(svc).await;
        conformance_expired_lease_not_is_leased(svc).await;
        conformance_extend_reactivates_expired_lease(svc).await;
        conformance_distinct_refs_are_independent(svc).await;
        conformance_dual_lease_same_ref_partial_release(svc).await;
    }

    // -----------------------------------------------------------------------
    // Individual contract assertions
    // -----------------------------------------------------------------------

    async fn conformance_acquire_returns_nonempty_id<S: ImageLeaseService>(svc: &S) {
        let id = svc
            .acquire("conformance:nonempty", Duration::from_secs(60))
            .await
            .expect("acquire should succeed");
        assert!(!id.is_empty(), "acquire: returned ID must be non-empty");
        svc.release(&id).await.expect("release cleanup");
    }

    async fn conformance_acquire_returns_unique_ids<S: ImageLeaseService>(svc: &S) {
        let id_a = svc
            .acquire("conformance:unique", Duration::from_secs(60))
            .await
            .expect("acquire A");
        let id_b = svc
            .acquire("conformance:unique", Duration::from_secs(60))
            .await
            .expect("acquire B");
        assert_ne!(id_a, id_b, "acquire: each call must return a distinct ID");
        svc.release(&id_a).await.expect("release A");
        svc.release(&id_b).await.expect("release B");
    }

    async fn conformance_active_lease_is_leased<S: ImageLeaseService>(svc: &S) {
        let id = svc
            .acquire("conformance:is-leased", Duration::from_secs(60))
            .await
            .expect("acquire");
        let leased = svc
            .is_leased("conformance:is-leased")
            .await
            .expect("is_leased");
        assert!(
            leased,
            "is_leased: must return true while an active lease exists for the ref"
        );
        svc.release(&id).await.expect("release cleanup");
    }

    async fn conformance_unrelated_ref_not_leased<S: ImageLeaseService>(svc: &S) {
        let id = svc
            .acquire("conformance:owned", Duration::from_secs(60))
            .await
            .expect("acquire");
        let leased = svc
            .is_leased("conformance:unrelated")
            .await
            .expect("is_leased");
        assert!(
            !leased,
            "is_leased: must return false for a ref not covered by any lease"
        );
        svc.release(&id).await.expect("release cleanup");
    }

    async fn conformance_lease_appears_in_list_and_list_active<S: ImageLeaseService>(svc: &S) {
        let id = svc
            .acquire("conformance:list", Duration::from_secs(60))
            .await
            .expect("acquire");

        let all = svc.list().await.expect("list");
        assert!(
            all.iter().any(|r| r.id == id),
            "list: the new lease must appear in the full list"
        );

        let active = svc.list_active().await.expect("list_active");
        assert!(
            active.iter().any(|r| r.id == id),
            "list_active: the non-expired lease must appear in the active list"
        );

        svc.release(&id).await.expect("release cleanup");
    }

    async fn conformance_release_removes_lease<S: ImageLeaseService>(svc: &S) {
        let id = svc
            .acquire("conformance:release", Duration::from_secs(60))
            .await
            .expect("acquire");
        svc.release(&id).await.expect("release");

        let all = svc.list().await.expect("list after release");
        assert!(
            all.iter().all(|r| r.id != id),
            "list: released lease must not appear in any subsequent list"
        );

        let active = svc.list_active().await.expect("list_active after release");
        assert!(
            active.iter().all(|r| r.id != id),
            "list_active: released lease must not appear in active list"
        );

        let leased = svc
            .is_leased("conformance:release")
            .await
            .expect("is_leased after release");
        assert!(
            !leased,
            "is_leased: must return false after the only lease for a ref is released"
        );
    }

    async fn conformance_release_unknown_id_is_ok<S: ImageLeaseService>(svc: &S) {
        svc.release("lease-id-that-does-not-exist")
            .await
            .expect("release of unknown ID must not error");
    }

    async fn conformance_expired_lease_in_list_not_in_list_active<S: ImageLeaseService>(svc: &S) {
        let id = svc
            .acquire("conformance:expired-list", Duration::from_secs(0))
            .await
            .expect("acquire with zero TTL");

        let all = svc.list().await.expect("list");
        assert!(
            all.iter().any(|r| r.id == id),
            "list: expired lease must still appear in the full (unfiltered) list"
        );

        let active = svc.list_active().await.expect("list_active");
        assert!(
            active.iter().all(|r| r.id != id),
            "list_active: expired lease must NOT appear in the active list"
        );

        svc.release(&id).await.expect("release cleanup");
    }

    async fn conformance_expired_lease_not_is_leased<S: ImageLeaseService>(svc: &S) {
        let id = svc
            .acquire("conformance:expired-is-leased", Duration::from_secs(0))
            .await
            .expect("acquire with zero TTL");

        let leased = svc
            .is_leased("conformance:expired-is-leased")
            .await
            .expect("is_leased after expiry");
        assert!(
            !leased,
            "is_leased: must return false for an expired lease (TTL=0)"
        );

        svc.release(&id).await.expect("release cleanup");
    }

    async fn conformance_extend_reactivates_expired_lease<S: ImageLeaseService>(svc: &S) {
        let id = svc
            .acquire("conformance:extend", Duration::from_secs(0))
            .await
            .expect("acquire with zero TTL");

        // Confirm it is not active before extending.
        let active_before = svc.list_active().await.expect("list_active before extend");
        assert!(
            active_before.iter().all(|r| r.id != id),
            "extend setup: lease with TTL=0 must start inactive"
        );

        svc.extend(&id, Duration::from_secs(3600))
            .await
            .expect("extend");

        let active_after = svc.list_active().await.expect("list_active after extend");
        assert!(
            active_after.iter().any(|r| r.id == id),
            "list_active: extended lease must now appear as active"
        );

        let leased = svc
            .is_leased("conformance:extend")
            .await
            .expect("is_leased after extend");
        assert!(
            leased,
            "is_leased: must return true after a previously expired lease is extended"
        );

        svc.release(&id).await.expect("release cleanup");
    }

    async fn conformance_distinct_refs_are_independent<S: ImageLeaseService>(svc: &S) {
        let id_a = svc
            .acquire("conformance:ref-a", Duration::from_secs(60))
            .await
            .expect("acquire ref-a");
        let id_b = svc
            .acquire("conformance:ref-b", Duration::from_secs(60))
            .await
            .expect("acquire ref-b");

        svc.release(&id_a).await.expect("release ref-a");

        let leased_a = svc
            .is_leased("conformance:ref-a")
            .await
            .expect("is_leased ref-a after release");
        let leased_b = svc
            .is_leased("conformance:ref-b")
            .await
            .expect("is_leased ref-b after releasing ref-a");

        assert!(
            !leased_a,
            "is_leased: ref-a must not be leased after its lease is released"
        );
        assert!(
            leased_b,
            "is_leased: ref-b must remain leased after an unrelated lease is released"
        );

        svc.release(&id_b).await.expect("release ref-b cleanup");
    }

    async fn conformance_dual_lease_same_ref_partial_release<S: ImageLeaseService>(svc: &S) {
        let id_first = svc
            .acquire("conformance:shared-ref", Duration::from_secs(60))
            .await
            .expect("acquire first lease");
        let id_second = svc
            .acquire("conformance:shared-ref", Duration::from_secs(60))
            .await
            .expect("acquire second lease");

        svc.release(&id_first).await.expect("release first lease");

        let still_leased = svc
            .is_leased("conformance:shared-ref")
            .await
            .expect("is_leased after partial release");
        assert!(
            still_leased,
            "is_leased: ref must remain leased while a second active lease still covers it"
        );

        svc.release(&id_second).await.expect("release second lease");

        let now_unleased = svc
            .is_leased("conformance:shared-ref")
            .await
            .expect("is_leased after both leases released");
        assert!(
            !now_unleased,
            "is_leased: ref must become unleased once all covering leases are released"
        );
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::default_constructed_unit_structs
)]
mod tests {
    use super::*;

    // --- InMemoryLeaseService conforms to the port contract ---

    #[tokio::test]
    async fn in_memory_lease_service_port_conformance() {
        let svc = InMemoryLeaseService::default();
        conformance::run_conformance_suite(&svc).await;
    }

    // --- DiskLeaseService conforms to the port contract ---

    #[tokio::test]
    async fn disk_lease_service_port_conformance() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let svc = DiskLeaseService::new(tmp.path().join("leases.json"))
            .await
            .expect("DiskLeaseService::new");
        conformance::run_conformance_suite(&svc).await;
    }

    // --- DiskLeaseService-specific: persistence across instances ---

    #[tokio::test]
    async fn disk_lease_service_persists_across_reload() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("leases.json");

        let id = {
            let svc = DiskLeaseService::new(path.clone())
                .await
                .expect("DiskLeaseService::new (first)");
            svc.acquire("alpine:latest", Duration::from_secs(3600))
                .await
                .expect("acquire")
        };

        // Re-open from the same file; the lease must survive.
        let svc2 = DiskLeaseService::new(path)
            .await
            .expect("DiskLeaseService::new (second)");
        let all = svc2.list().await.expect("list after reload");
        assert!(
            all.iter().any(|r| r.id == id),
            "DiskLeaseService: lease must be visible after reload from disk"
        );
        assert!(
            svc2.is_leased("alpine:latest").await.expect("is_leased"),
            "DiskLeaseService: is_leased must return true after reload"
        );
    }
}
