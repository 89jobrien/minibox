//! Per-image pull locking to prevent redundant concurrent pulls.
//!
//! [`LockedImageRegistry`] wraps any [`ImageRegistry`] implementation and
//! serializes concurrent `pull_image` calls for the same image reference.
//! When two callers request the same `name:tag` simultaneously, the second
//! waits until the first pull completes rather than starting a duplicate
//! network transfer.
//!
//! All other trait methods (`has_image`, `get_image_layers`) delegate to the
//! inner registry without locking — they are read-only and safe to call
//! concurrently.
//!
//! # Example
//!
//! ```rust,ignore
//! use minibox_core::adapters::LockedImageRegistry;
//! use std::sync::Arc;
//!
//! let inner = Arc::new(DockerHubRegistry::new(image_store)?);
//! let locked = Arc::new(LockedImageRegistry::new(inner));
//! // `locked` implements `ImageRegistry` and can be used anywhere the inner
//! // registry was used.
//! ```

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::Mutex;

use crate::domain::{AsAny, ImageMetadata, ImageRegistry};
use crate::image::reference::ImageRef;

/// An [`ImageRegistry`] adapter that serializes concurrent pulls for the same
/// image reference.
///
/// Internally a [`DashMap`] holds one `Arc<Mutex<()>>` per image key
/// (`"name:tag"`). When `pull_image` is called the adapter acquires the
/// per-image mutex before delegating to the inner registry, so a second
/// concurrent pull for the same image will wait rather than racing.
///
/// The lock is *per image*, so pulls of different images proceed in parallel.
pub struct LockedImageRegistry {
    inner: Arc<dyn ImageRegistry>,
    /// Per-image key (`"registry/namespace/name:tag"`) → serialisation lock.
    locks: DashMap<String, Arc<Mutex<()>>>,
}

impl LockedImageRegistry {
    /// Wrap `inner` with per-image pull serialisation.
    #[must_use]
    pub fn new(inner: Arc<dyn ImageRegistry>) -> Self {
        Self {
            inner,
            locks: DashMap::new(),
        }
    }

    /// Derive a stable lock key from an image reference.
    ///
    /// The key includes the registry, namespace, name, and tag so that
    /// `ubuntu:22.04` and `ubuntu:24.04` use distinct locks.
    fn lock_key(image_ref: &ImageRef) -> String {
        format!(
            "{}/{}/{}:{}",
            image_ref.registry, image_ref.namespace, image_ref.name, image_ref.tag
        )
    }
}

impl AsAny for LockedImageRegistry {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait]
impl ImageRegistry for LockedImageRegistry {
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        self.inner.has_image(name, tag).await
    }

    /// Pull an image, serialising concurrent requests for the same image key.
    ///
    /// If a pull for `image_ref` is already in progress this call blocks until
    /// that pull finishes, then runs its own pull. Because `has_image` is
    /// checked by the caller before `pull_image` in most code paths, the
    /// second waiter will typically find the image already cached; the inner
    /// pull will be a fast no-op in that case.
    async fn pull_image(&self, image_ref: &ImageRef) -> Result<ImageMetadata> {
        let key = Self::lock_key(image_ref);
        // `or_insert_with` is atomic for `DashMap` — only one lock per key.
        let lock = self
            .locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        self.inner.pull_image(image_ref).await
    }

    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<std::path::PathBuf>> {
        self.inner.get_image_layers(name, tag)
    }
}

// ---------------------------------------------------------------------------
// Tests
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
    use crate::adapters::mocks::MockRegistry;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn test_ref(name: &str, tag: &str) -> ImageRef {
        ImageRef {
            registry: "docker.io".to_string(),
            namespace: "library".to_string(),
            name: name.to_string(),
            tag: tag.to_string(),
        }
    }

    #[tokio::test]
    async fn delegates_has_image() {
        let inner = Arc::new(MockRegistry::new().with_cached_image("ubuntu", "22.04"));
        let locked = LockedImageRegistry::new(inner);

        assert!(locked.has_image("ubuntu", "22.04").await);
        assert!(!locked.has_image("ubuntu", "24.04").await);
    }

    #[tokio::test]
    async fn delegates_pull_image_success() {
        let inner = Arc::new(MockRegistry::new());
        let locked = LockedImageRegistry::new(inner);

        let result = locked.pull_image(&test_ref("alpine", "latest")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn delegates_pull_image_failure() {
        let inner = Arc::new(MockRegistry::new().with_pull_failure());
        let locked = LockedImageRegistry::new(inner);

        let result = locked.pull_image(&test_ref("alpine", "latest")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn different_images_do_not_share_locks() {
        // Two different image refs should acquire independent locks and can
        // therefore run in parallel without one blocking the other.
        let inner = Arc::new(MockRegistry::new());
        let locked = Arc::new(LockedImageRegistry::new(inner));

        let l1 = Arc::clone(&locked);
        let l2 = Arc::clone(&locked);

        let h1 = tokio::spawn(async move { l1.pull_image(&test_ref("ubuntu", "22.04")).await });
        let h2 = tokio::spawn(async move { l2.pull_image(&test_ref("alpine", "latest")).await });

        let (r1, r2) = tokio::join!(h1, h2);
        assert!(r1.expect("task 1 panicked").is_ok());
        assert!(r2.expect("task 2 panicked").is_ok());
    }

    /// Verify that concurrent pulls for the **same** image serialize rather
    /// than running in parallel.
    ///
    /// The test injects a slow inner registry (via a `Mutex`-gated counter)
    /// to detect whether two pulls overlap. If the outer lock is absent, both
    /// tasks enter the inner mock simultaneously and the total elapsed time
    /// drops below the sum of the two individual delays; with the lock in
    /// place the second pull waits for the first to finish.
    #[tokio::test]
    async fn concurrent_pulls_for_same_image_serialize() {
        use std::sync::Mutex as StdMutex;

        // Track the maximum observed concurrent depth.
        let concurrent_depth: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
        let max_depth: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));

        // A custom registry that records concurrency and introduces a small
        // artificial delay to make races detectable.
        #[allow(dead_code)]
        struct SlowRegistry {
            concurrent: Arc<AtomicUsize>,
            max: Arc<AtomicUsize>,
            // gate forces both tasks to attempt entry before either finishes
            gate: Arc<StdMutex<()>>,
        }

        impl AsAny for SlowRegistry {
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }

        #[async_trait]
        impl ImageRegistry for SlowRegistry {
            async fn has_image(&self, _: &str, _: &str) -> bool {
                false
            }

            async fn pull_image(&self, _: &ImageRef) -> Result<ImageMetadata> {
                let depth = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                self.max.fetch_max(depth, Ordering::SeqCst);

                // Hold for a short time so both tasks have a chance to be
                // concurrently in-flight if locking is absent.
                tokio::time::sleep(Duration::from_millis(20)).await;

                self.concurrent.fetch_sub(1, Ordering::SeqCst);
                Ok(ImageMetadata {
                    name: "test".to_string(),
                    tag: "latest".to_string(),
                    layers: vec![],
                })
            }

            fn get_image_layers(&self, _: &str, _: &str) -> Result<Vec<std::path::PathBuf>> {
                Ok(vec![])
            }
        }

        let slow_inner = Arc::new(SlowRegistry {
            concurrent: Arc::clone(&concurrent_depth),
            max: Arc::clone(&max_depth),
            gate: Arc::new(StdMutex::new(())),
        });

        let locked = Arc::new(LockedImageRegistry::new(slow_inner));

        let l1 = Arc::clone(&locked);
        let l2 = Arc::clone(&locked);
        let image = test_ref("ubuntu", "22.04");

        let h1 = tokio::spawn(async move { l1.pull_image(&image).await });
        // Give the first task a tiny head start so it acquires the lock first.
        tokio::time::sleep(Duration::from_millis(2)).await;
        let image2 = test_ref("ubuntu", "22.04");
        let h2 = tokio::spawn(async move { l2.pull_image(&image2).await });

        let (r1, r2) = tokio::join!(h1, h2);
        assert!(r1.expect("task 1 panicked").is_ok());
        assert!(r2.expect("task 2 panicked").is_ok());

        // If LockedImageRegistry serializes correctly, the two pulls never
        // overlapped, so the maximum observed concurrency is 1.
        assert_eq!(
            max_depth.load(Ordering::SeqCst),
            1,
            "concurrent pulls for the same image should be serialized (max depth > 1 means they overlapped)"
        );
    }
}
