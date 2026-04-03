//! Image garbage collection.

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;

use super::ImageStore;
use super::lease::ImageLeaseService;

/// Summary of a prune operation.
#[derive(Debug, Default)]
pub struct PruneReport {
    /// Image refs that were (or would be) removed.
    pub removed: Vec<String>,
    /// Bytes freed (or that would be freed in dry-run mode).
    pub freed_bytes: u64,
    /// True if this was a dry run (no files actually deleted).
    pub dry_run: bool,
}

/// Port: remove unused images.
#[async_trait]
pub trait ImageGarbageCollector: Send + Sync {
    /// Remove images not referenced by active containers or valid leases.
    ///
    /// `in_use` is a slice of `"name:tag"` strings for images currently used
    /// by running or paused containers.
    async fn prune(&self, dry_run: bool, in_use: &[String]) -> Result<PruneReport>;
}

/// Adapter: GC implementation using `ImageStore` + `ImageLeaseService`.
pub struct ImageGc {
    store: Arc<ImageStore>,
    leases: Arc<dyn ImageLeaseService>,
}

impl ImageGc {
    pub fn new(store: Arc<ImageStore>, leases: Arc<dyn ImageLeaseService>) -> Self {
        Self { store, leases }
    }
}

#[async_trait]
impl ImageGarbageCollector for ImageGc {
    async fn prune(&self, dry_run: bool, in_use: &[String]) -> Result<PruneReport> {
        let all = self.store.list_all_images().await?;
        let in_use_set: HashSet<&str> = in_use.iter().map(|s| s.as_str()).collect();

        let mut report = PruneReport {
            dry_run,
            ..Default::default()
        };

        for image_ref in &all {
            // Skip images in use by running/paused containers
            if in_use_set.contains(image_ref.as_str()) {
                continue;
            }
            // Skip images protected by an active lease
            if self.leases.is_leased(image_ref).await? {
                continue;
            }

            // Parse "name:tag"
            let (name, tag) = match image_ref.rsplit_once(':') {
                Some(pair) => pair,
                None => continue,
            };

            let size = self.store.image_size_bytes(name, tag).await.unwrap_or(0);
            report.freed_bytes += size;
            report.removed.push(image_ref.clone());

            if !dry_run && let Err(e) = self.store.delete_image(name, tag).await {
                tracing::warn!(image = %image_ref, error = %e, "gc: failed to delete image");
            }
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::lease::DiskLeaseService;
    use std::sync::Arc;

    async fn make_gc(tmp: &tempfile::TempDir) -> ImageGc {
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());
        let leases = Arc::new(
            DiskLeaseService::new(tmp.path().join("leases.json"))
                .await
                .unwrap(),
        );
        ImageGc::new(store, leases)
    }

    #[tokio::test]
    async fn test_prune_empty_store_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let gc = make_gc(&tmp).await;
        let report = gc.prune(false, &[]).await.unwrap();
        assert_eq!(report.removed.len(), 0);
        assert_eq!(report.freed_bytes, 0);
    }

    #[tokio::test]
    async fn test_prune_removes_unreferenced_image() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());

        // Seed a fake image
        let img_dir = tmp.path().join("images").join("alpine").join("latest");
        tokio::fs::create_dir_all(&img_dir).await.unwrap();
        tokio::fs::write(img_dir.join("manifest.json"), b"{}")
            .await
            .unwrap();

        let leases = Arc::new(
            DiskLeaseService::new(tmp.path().join("leases.json"))
                .await
                .unwrap(),
        );
        let gc = ImageGc::new(Arc::clone(&store), leases);

        // in_use: empty (no containers using alpine:latest)
        let report = gc.prune(false, &[]).await.unwrap();
        assert!(
            !report.removed.is_empty(),
            "Expected at least one image removed"
        );
        // The image_ref format from list_all_images is "alpine:latest"
        assert!(
            report
                .removed
                .iter()
                .any(|r| r.contains("alpine") && r.contains("latest"))
        );
    }

    #[tokio::test]
    async fn test_prune_dry_run_does_not_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(ImageStore::new(tmp.path().join("images")).unwrap());

        let img_dir = tmp.path().join("images").join("alpine").join("latest");
        tokio::fs::create_dir_all(&img_dir).await.unwrap();
        tokio::fs::write(img_dir.join("manifest.json"), b"{}")
            .await
            .unwrap();

        let leases = Arc::new(
            DiskLeaseService::new(tmp.path().join("leases.json"))
                .await
                .unwrap(),
        );
        let gc = ImageGc::new(Arc::clone(&store), leases);
        let report = gc.prune(true, &[]).await.unwrap();

        assert!(report.dry_run);
        assert!(!report.removed.is_empty());
        // Directory must still exist
        assert!(
            img_dir.exists(),
            "dry-run must not delete the image directory"
        );
    }
}
