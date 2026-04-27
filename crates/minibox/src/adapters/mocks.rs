//! Mock adapters for testing.
//!
//! This module provides in-process mock implementations of all four domain
//! traits ([`ImageRegistry`], [`FilesystemProvider`], [`ResourceLimiter`],
//! [`ContainerRuntime`]), allowing business logic to be tested without real
//! infrastructure dependencies — no network, no cgroups, no Linux syscalls.
//!
//! Each mock tracks call counts and can be configured to fail on demand via
//! builder methods. All state is shared behind an `Arc<Mutex<…>>` so mocks
//! can be cloned and observed from the test after being injected.
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::adapters::mocks::{MockRegistry, MockFilesystem, MockLimiter, MockRuntime};
//! use crate::domain::*;
//! use std::sync::Arc;
//!
//! #[tokio::test]
//! async fn test_container_creation() {
//!     let deps = HandlerDependencies {
//!         registry: Arc::new(MockRegistry::new()),
//!         filesystem: Arc::new(MockFilesystem::new()),
//!         resource_limiter: Arc::new(MockLimiter::new()),
//!         runtime: Arc::new(MockRuntime::new()),
//!     };
//!
//!     // Test your business logic with zero infrastructure!
//! }
//! ```

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::adapt;
use minibox_core::domain::{
    ContainerRuntime, ContainerSpawnConfig, ImageMetadata, ImageRegistry, LayerInfo, NetworkConfig,
    NetworkProvider, NetworkStats, ResourceConfig, ResourceLimiter, RootfsLayout,
    RuntimeCapabilities, SpawnResult,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// MockRegistry
// ---------------------------------------------------------------------------

/// Mock implementation of [`ImageRegistry`] for testing.
///
/// Simulates an image registry without making network requests. Configure
/// pre-cached images and failure behaviour via builder methods before use.
///
/// All state is stored in a shared `Arc<Mutex<…>>` so the mock can be cloned
/// and observed from the test after injection.
#[derive(Debug, Clone)]
pub struct MockRegistry {
    state: Arc<Mutex<MockRegistryState>>,
}

#[derive(Debug)]
struct MockRegistryState {
    /// Images that are already "cached" locally (checked by `has_image`).
    cached_images: Vec<(String, String)>, // (name, tag)
    /// Whether `pull_image` calls should succeed (`true`) or fail (`false`).
    pull_should_succeed: bool,
    /// Running count of `pull_image` invocations.
    pull_count: usize,
    /// Whether `get_image_layers` should return an empty list.
    return_empty_layers: bool,
}

impl MockRegistry {
    /// Create a new mock registry with no cached images and pull success enabled.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockRegistryState {
                cached_images: Vec::new(),
                pull_should_succeed: true,
                pull_count: 0,
                return_empty_layers: false,
            })),
        }
    }

    /// Configure the registry to report `name:tag` as already cached locally.
    ///
    /// May be called multiple times to seed multiple images.
    pub fn with_cached_image(self, name: &str, tag: &str) -> Self {
        self.state
            .lock()
            .unwrap()
            .cached_images
            .push((name.to_string(), tag.to_string()));
        self
    }

    /// Configure all subsequent `pull_image` calls to return an error.
    pub fn with_pull_failure(self) -> Self {
        self.state.lock().unwrap().pull_should_succeed = false;
        self
    }

    /// Configure `get_image_layers` to return an empty layer list.
    ///
    /// Used to exercise the `EmptyImage` error path in `run_inner`.
    pub fn with_empty_layers(self) -> Self {
        self.state.lock().unwrap().return_empty_layers = true;
        self
    }

    /// Return the number of times `pull_image` has been called.
    pub fn pull_count(&self) -> usize {
        self.state.lock().unwrap().pull_count
    }

    /// Synchronous variant of `has_image` — bypasses async machinery.
    ///
    /// Useful in benchmarks and synchronous test helpers where an async
    /// executor is not available.
    pub fn has_image_sync(&self, image: &str, tag: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .cached_images
            .iter()
            .any(|(n, t)| n == image && t == tag)
    }
}

#[async_trait]
impl ImageRegistry for MockRegistry {
    /// Return `true` if the image was seeded via [`with_cached_image`] or pulled successfully.
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .cached_images
            .iter()
            .any(|(n, t)| n == name && t == tag)
    }

    /// Simulate an image pull.
    ///
    /// Increments the pull counter. On success adds the image to the local
    /// cache and returns [`ImageMetadata`] with two fixed mock layers. On
    /// failure (configured via [`with_pull_failure`]) returns an error
    /// without modifying the cache.
    async fn pull_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
    ) -> Result<ImageMetadata> {
        let name = image_ref.cache_name();
        let tag = image_ref.tag.clone();
        let mut state = self.state.lock().unwrap();
        state.pull_count += 1;

        if !state.pull_should_succeed {
            anyhow::bail!("mock pull failure");
        }

        // Simulate a successful pull by adding the image to the local cache.
        state.cached_images.push((name.clone(), tag.clone()));

        Ok(ImageMetadata {
            name,
            tag,
            layers: vec![
                LayerInfo {
                    digest: "sha256:mock-layer-1".to_string(),
                    size: 1024,
                },
                LayerInfo {
                    digest: "sha256:mock-layer-2".to_string(),
                    size: 2048,
                },
            ],
        })
    }

    /// Return two fixed mock layer paths regardless of the image name or tag.
    ///
    /// Returns an empty vec if configured via [`with_empty_layers`], which
    /// triggers the `EmptyImage` error path in `run_inner`.
    fn get_image_layers(&self, _name: &str, _tag: &str) -> Result<Vec<PathBuf>> {
        let state = self.state.lock().unwrap();
        if state.return_empty_layers {
            return Ok(vec![]);
        }
        Ok(vec![
            PathBuf::from("/mock/layer1"),
            PathBuf::from("/mock/layer2"),
        ])
    }
}

// ---------------------------------------------------------------------------
// MockFilesystem
// ---------------------------------------------------------------------------

/// Mock implementation of [`FilesystemProvider`] for testing.
///
/// Simulates filesystem operations without any actual mounts or syscalls.
/// Tracks `setup_rootfs` and `cleanup` call counts and can be configured to
/// fail on demand.
#[derive(Debug, Clone)]
pub struct MockFilesystem {
    state: Arc<Mutex<MockFilesystemState>>,
}

#[derive(Debug)]
struct MockFilesystemState {
    /// Whether `setup_rootfs` should succeed.
    setup_should_succeed: bool,
    /// Whether `pivot_root` should succeed.
    pivot_should_succeed: bool,
    /// Whether `cleanup` should succeed.
    cleanup_should_succeed: bool,
    /// Running count of `setup_rootfs` invocations.
    setup_count: usize,
    /// Running count of `cleanup` invocations.
    cleanup_count: usize,
}

impl MockFilesystem {
    /// Create a new mock filesystem with all operations succeeding by default.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockFilesystemState {
                setup_should_succeed: true,
                pivot_should_succeed: true,
                cleanup_should_succeed: true,
                setup_count: 0,
                cleanup_count: 0,
            })),
        }
    }

    /// Configure `setup_rootfs` to return an error on the next call.
    pub fn with_setup_failure(self) -> Self {
        self.state.lock().unwrap().setup_should_succeed = false;
        self
    }

    /// Return the number of times `setup_rootfs` has been called.
    pub fn setup_count(&self) -> usize {
        self.state.lock().unwrap().setup_count
    }

    /// Return the number of times `cleanup` has been called.
    pub fn cleanup_count(&self) -> usize {
        self.state.lock().unwrap().cleanup_count
    }
}

impl minibox_core::domain::RootfsSetup for MockFilesystem {
    /// Simulate rootfs setup by returning `container_dir/merged`.
    ///
    /// Increments the setup counter. Returns an error if configured via
    /// [`with_setup_failure`].
    fn setup_rootfs(&self, _layers: &[PathBuf], container_dir: &Path) -> Result<RootfsLayout> {
        let mut state = self.state.lock().unwrap();
        state.setup_count += 1;

        if !state.setup_should_succeed {
            anyhow::bail!("mock filesystem setup failure");
        }

        Ok(RootfsLayout {
            merged_dir: container_dir.join("merged"),
            rootfs_metadata: None,
            source_image_ref: None,
        })
    }

    /// Simulate filesystem cleanup.
    ///
    /// Increments the cleanup counter. Returns an error if the mock is
    /// configured to fail cleanup.
    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.cleanup_count += 1;

        if !state.cleanup_should_succeed {
            anyhow::bail!("mock cleanup failure");
        }
        Ok(())
    }
}

impl minibox_core::domain::ChildInit for MockFilesystem {
    /// Simulate `pivot_root` — succeeds unless configured to fail.
    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        let state = self.state.lock().unwrap();
        if !state.pivot_should_succeed {
            anyhow::bail!("mock pivot_root failure");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MockLimiter
// ---------------------------------------------------------------------------

/// Mock implementation of [`ResourceLimiter`] for testing.
///
/// Simulates cgroup operations without any kernel interaction. Returns a fake
/// cgroup path on success and tracks call counts.
#[derive(Debug, Clone)]
pub struct MockLimiter {
    state: Arc<Mutex<MockLimiterState>>,
}

#[derive(Debug)]
struct MockLimiterState {
    /// Whether `create` should succeed.
    create_should_succeed: bool,
    /// Whether `add_process` should succeed.
    add_process_should_succeed: bool,
    /// Whether `cleanup` should succeed.
    cleanup_should_succeed: bool,
    /// Running count of `create` invocations.
    create_count: usize,
    /// Running count of `cleanup` invocations.
    cleanup_count: usize,
    /// Container IDs for which a cgroup was successfully created.
    created_cgroups: Vec<String>,
}

impl MockLimiter {
    /// Create a new mock resource limiter with all operations succeeding by default.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockLimiterState {
                create_should_succeed: true,
                add_process_should_succeed: true,
                cleanup_should_succeed: true,
                create_count: 0,
                cleanup_count: 0,
                created_cgroups: Vec::new(),
            })),
        }
    }

    /// Configure `create` to return an error.
    pub fn with_create_failure(self) -> Self {
        self.state.lock().unwrap().create_should_succeed = false;
        self
    }

    /// Configure `cleanup` to return an error.
    ///
    /// Used to exercise the best-effort cgroup cleanup path in `remove_inner`.
    pub fn with_cleanup_failure(self) -> Self {
        self.state.lock().unwrap().cleanup_should_succeed = false;
        self
    }

    /// Return the number of times `create` has been called.
    pub fn create_count(&self) -> usize {
        self.state.lock().unwrap().create_count
    }

    /// Return the number of times `cleanup` has been called.
    pub fn cleanup_count(&self) -> usize {
        self.state.lock().unwrap().cleanup_count
    }
}

impl ResourceLimiter for MockLimiter {
    /// Simulate cgroup creation and return a fake cgroup path.
    ///
    /// Increments the create counter and records the container ID. Returns
    /// `/mock/cgroup/<container_id>` on success.
    fn create(&self, container_id: &str, _config: &ResourceConfig) -> Result<String> {
        let mut state = self.state.lock().unwrap();
        state.create_count += 1;

        if !state.create_should_succeed {
            anyhow::bail!("mock resource limiter create failure");
        }

        state.created_cgroups.push(container_id.to_string());
        Ok(format!("/mock/cgroup/{container_id}"))
    }

    /// Simulate adding a process to a cgroup — succeeds unless configured to fail.
    fn add_process(&self, _container_id: &str, _pid: u32) -> Result<()> {
        let state = self.state.lock().unwrap();
        if !state.add_process_should_succeed {
            anyhow::bail!("mock add_process failure");
        }
        Ok(())
    }

    /// Simulate cgroup cleanup.
    ///
    /// Increments the cleanup counter. Returns an error if the mock is
    /// configured to fail cleanup.
    fn cleanup(&self, _container_id: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.cleanup_count += 1;

        if !state.cleanup_should_succeed {
            anyhow::bail!("mock cleanup failure");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MockRuntime
// ---------------------------------------------------------------------------

/// Mock implementation of [`ContainerRuntime`] for testing.
///
/// Simulates container process spawning without any syscalls. Returns
/// monotonically increasing fake PIDs starting from 10000.
#[derive(Debug, Clone)]
pub struct MockRuntime {
    state: Arc<Mutex<MockRuntimeState>>,
}

#[derive(Debug)]
struct MockRuntimeState {
    /// Whether `spawn_process` calls should succeed.
    spawn_should_succeed: bool,
    /// The PID to hand out on the next successful spawn; incremented after each use.
    next_pid: u32,
    /// Running count of `spawn_process` invocations (both sync and async).
    spawn_count: usize,
}

impl MockRuntime {
    /// Create a new mock runtime with spawn succeeding and PIDs starting at 10000.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockRuntimeState {
                spawn_should_succeed: true,
                next_pid: 10000,
                spawn_count: 0,
            })),
        }
    }

    /// Configure all subsequent `spawn_process` calls to return an error.
    pub fn with_spawn_failure(self) -> Self {
        self.state.lock().unwrap().spawn_should_succeed = false;
        self
    }

    /// Return the total number of spawn attempts (successful and failed).
    pub fn spawn_count(&self) -> usize {
        self.state.lock().unwrap().spawn_count
    }

    /// Synchronous variant of `spawn_process` — bypasses async machinery.
    ///
    /// Useful in benchmarks and synchronous test helpers where an async
    /// executor is not available. Shares state with the async variant.
    pub fn spawn_process_sync(&self, _cfg: &ContainerSpawnConfig) -> Result<SpawnResult> {
        let mut state = self.state.lock().unwrap();
        state.spawn_count += 1;
        if !state.spawn_should_succeed {
            anyhow::bail!("mock spawn failure");
        }
        let pid = state.next_pid;
        state.next_pid += 1;
        Ok(SpawnResult {
            runtime_id: None,
            pid,
            output_reader: None,
        })
    }
}

#[async_trait]
impl ContainerRuntime for MockRuntime {
    /// Return minimal capabilities — the mock does not support any Linux-specific features.
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: false,
            supports_overlay_fs: false,
            supports_network_isolation: false,
            max_containers: None,
        }
    }

    /// Simulate spawning a container process and return a fake PID.
    ///
    /// Increments the spawn counter and the internal PID counter on success.
    /// The `output_reader` field is always `None`.
    async fn spawn_process(&self, _config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        let mut state = self.state.lock().unwrap();
        state.spawn_count += 1;

        if !state.spawn_should_succeed {
            anyhow::bail!("mock spawn failure");
        }

        let pid = state.next_pid;
        state.next_pid += 1;
        Ok(SpawnResult {
            runtime_id: None,
            pid,
            output_reader: None,
        })
    }
}

// ---------------------------------------------------------------------------
// MockNetwork
// ---------------------------------------------------------------------------

/// Mock implementation of [`NetworkProvider`] for testing.
///
/// Simulates network setup and cleanup without any real syscalls or namespace
/// operations. Returns a fixed fake netns path on `setup` and tracks call
/// counts for `setup` and `cleanup`.
#[derive(Debug, Clone)]
pub struct MockNetwork {
    state: Arc<Mutex<MockNetworkState>>,
}

#[derive(Debug)]
struct MockNetworkState {
    /// Whether `setup` should succeed.
    setup_should_succeed: bool,
    /// Whether `cleanup` should succeed.
    cleanup_should_succeed: bool,
    /// Running count of `setup` invocations.
    setup_count: usize,
    /// Running count of `cleanup` invocations.
    cleanup_count: usize,
}

impl MockNetwork {
    /// Create a new mock network with all operations succeeding by default.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockNetworkState {
                setup_should_succeed: true,
                cleanup_should_succeed: true,
                setup_count: 0,
                cleanup_count: 0,
            })),
        }
    }

    /// Configure `setup` to return an error.
    pub fn with_setup_failure(self) -> Self {
        self.state.lock().unwrap().setup_should_succeed = false;
        self
    }

    /// Configure `cleanup` to return an error.
    pub fn with_cleanup_failure(self) -> Self {
        self.state.lock().unwrap().cleanup_should_succeed = false;
        self
    }

    /// Return the number of times `setup` has been called.
    pub fn setup_count(&self) -> usize {
        self.state.lock().unwrap().setup_count
    }

    /// Return the number of times `cleanup` has been called.
    pub fn cleanup_count(&self) -> usize {
        self.state.lock().unwrap().cleanup_count
    }
}

#[async_trait]
impl NetworkProvider for MockNetwork {
    /// Simulate network namespace setup and return a fixed fake netns path.
    ///
    /// Increments the setup counter. Returns an error if configured via
    /// [`with_setup_failure`].
    async fn setup(&self, _container_id: &str, _config: &NetworkConfig) -> Result<String> {
        let mut state = self.state.lock().unwrap();
        state.setup_count += 1;

        if !state.setup_should_succeed {
            anyhow::bail!("mock network setup failure");
        }

        Ok("/mock/netns".to_string())
    }

    /// Simulate attaching a container to its network namespace — always succeeds.
    async fn attach(&self, _container_id: &str, _pid: u32) -> Result<()> {
        Ok(())
    }

    /// Simulate network cleanup.
    ///
    /// Increments the cleanup counter. Returns an error if configured via
    /// [`with_cleanup_failure`].
    async fn cleanup(&self, _container_id: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.cleanup_count += 1;
        if !state.cleanup_should_succeed {
            anyhow::bail!("mock network cleanup failure");
        }
        Ok(())
    }

    /// Return default (all-zero) network statistics.
    async fn stats(&self, _container_id: &str) -> Result<NetworkStats> {
        Ok(NetworkStats::default())
    }
}

// Register the five primary mock types with the adapt! macro so they satisfy
// the AsAny + Default bounds required by the daemon's adapter registry.
adapt!(
    MockRegistry,
    MockFilesystem,
    MockLimiter,
    MockRuntime,
    MockNetwork
);

// ---------------------------------------------------------------------------
// FailableFilesystemMock
// ---------------------------------------------------------------------------

/// Filesystem mock with runtime-toggleable failure injection via atomics.
///
/// Unlike [`MockFilesystem`] whose failure modes are fixed at construction
/// time via builder methods, this mock lets tests flip failures on and off
/// between individual calls using atomic stores. This is useful for testing
/// error-recovery paths such as cleanup-after-setup-failure.
///
/// Uses `SeqCst` ordering throughout to avoid races in parallel test scenarios.
pub struct FailableFilesystemMock {
    /// Whether the next `setup_rootfs` call should return an error.
    should_fail_setup: AtomicBool,
    /// Whether the next `cleanup` call should return an error.
    should_fail_cleanup: AtomicBool,
    /// Running count of `setup_rootfs` invocations.
    setup_count: AtomicUsize,
    /// Running count of `cleanup` invocations.
    cleanup_count: AtomicUsize,
}

impl FailableFilesystemMock {
    /// Create a new mock with both operations succeeding by default.
    pub fn new() -> Self {
        Self {
            should_fail_setup: AtomicBool::new(false),
            should_fail_cleanup: AtomicBool::new(false),
            setup_count: AtomicUsize::new(0),
            cleanup_count: AtomicUsize::new(0),
        }
    }

    /// Toggle whether `setup_rootfs` returns an error on the next call.
    ///
    /// Pass `true` to inject a failure; `false` to restore success.
    pub fn set_fail_setup(&self, fail: bool) {
        self.should_fail_setup.store(fail, Ordering::SeqCst);
    }

    /// Toggle whether `cleanup` returns an error on the next call.
    ///
    /// Pass `true` to inject a failure; `false` to restore success.
    pub fn set_fail_cleanup(&self, fail: bool) {
        self.should_fail_cleanup.store(fail, Ordering::SeqCst);
    }

    /// Return the number of times `setup_rootfs` has been called.
    pub fn setup_count(&self) -> usize {
        self.setup_count.load(Ordering::SeqCst)
    }

    /// Return the number of times `cleanup` has been called.
    pub fn cleanup_count(&self) -> usize {
        self.cleanup_count.load(Ordering::SeqCst)
    }
}

impl minibox_core::domain::RootfsSetup for FailableFilesystemMock {
    /// Simulate rootfs setup, honouring the current failure toggle.
    fn setup_rootfs(&self, _layers: &[PathBuf], container_dir: &Path) -> Result<RootfsLayout> {
        self.setup_count.fetch_add(1, Ordering::SeqCst);
        if self.should_fail_setup.load(Ordering::SeqCst) {
            anyhow::bail!("injected setup failure");
        }
        Ok(RootfsLayout {
            merged_dir: container_dir.join("merged"),
            rootfs_metadata: None,
            source_image_ref: None,
        })
    }

    /// Simulate filesystem cleanup, honouring the current failure toggle.
    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        self.cleanup_count.fetch_add(1, Ordering::SeqCst);
        if self.should_fail_cleanup.load(Ordering::SeqCst) {
            anyhow::bail!("injected cleanup failure");
        }
        Ok(())
    }
}

impl minibox_core::domain::ChildInit for FailableFilesystemMock {
    /// Always succeeds — `pivot_root` failure injection is not supported by this mock.
    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        Ok(())
    }
}

// Register FailableFilesystemMock separately — it only implements
// FilesystemProvider, not the full four-trait set.
adapt!(FailableFilesystemMock);

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::{ContainerHooks, RootfsSetup};

    #[test]
    fn mock_registry_has_image_sync_cached() {
        let reg = MockRegistry::new().with_cached_image("alpine", "latest");
        assert!(reg.has_image_sync("alpine", "latest"));
        assert!(!reg.has_image_sync("alpine", "missing"));
    }

    #[test]
    fn mock_runtime_spawn_process_sync_increments_count() {
        use minibox_core::domain::{ContainerHooks, ContainerSpawnConfig};
        let runtime = MockRuntime::new();
        let cfg = ContainerSpawnConfig {
            rootfs: std::path::PathBuf::from("/mock/rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            hostname: "mock".to_string(),
            cgroup_path: std::path::PathBuf::from("/mock/cgroup"),
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: false,
            mounts: vec![],
            privileged: false,
        };
        let result = runtime.spawn_process_sync(&cfg).unwrap();
        assert_eq!(result.pid, 10000);
        assert_eq!(runtime.spawn_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_registry_cached_image() {
        let registry = MockRegistry::new().with_cached_image("library/alpine", "latest");

        assert!(registry.has_image("library/alpine", "latest").await);
        assert!(!registry.has_image("library/ubuntu", "latest").await);
    }

    #[tokio::test]
    async fn test_mock_registry_pull_success() {
        let registry = MockRegistry::new();

        assert_eq!(registry.pull_count(), 0);
        let image_ref = crate::image::reference::ImageRef::parse("alpine").unwrap();
        let result = registry.pull_image(&image_ref).await;
        assert!(result.is_ok());
        assert_eq!(registry.pull_count(), 1);

        // After pull, image should be cached
        assert!(registry.has_image("library/alpine", "latest").await);
    }

    #[tokio::test]
    async fn test_mock_registry_pull_failure() {
        let registry = MockRegistry::new().with_pull_failure();

        let image_ref = crate::image::reference::ImageRef::parse("alpine").unwrap();
        let result = registry.pull_image(&image_ref).await;
        assert!(result.is_err());
        assert_eq!(registry.pull_count(), 1);
    }

    #[test]
    fn test_mock_filesystem_setup() {
        let fs = MockFilesystem::new();

        assert_eq!(fs.setup_count(), 0);
        let result = fs.setup_rootfs(&[PathBuf::from("/layer1")], Path::new("/container"));
        assert!(result.is_ok());
        assert_eq!(fs.setup_count(), 1);
    }

    #[test]
    fn test_mock_limiter_create() {
        let limiter = MockLimiter::new();

        assert_eq!(limiter.create_count(), 0);
        let result = limiter.create("container123", &ResourceConfig::default());
        assert!(result.is_ok());
        assert_eq!(limiter.create_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_runtime_spawn() {
        let runtime = MockRuntime::new();

        assert_eq!(runtime.spawn_count(), 0);

        let config = ContainerSpawnConfig {
            rootfs: PathBuf::from("/mock/rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            hostname: "mock-host".to_string(),
            cgroup_path: PathBuf::from("/mock/cgroup"),
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: false,
            mounts: vec![],
            privileged: false,
        };

        let result = runtime.spawn_process(&config).await;
        assert!(result.is_ok());
        assert_eq!(runtime.spawn_count(), 1);

        // Second spawn should give different PID
        let result2 = runtime.spawn_process(&config).await.unwrap();
        assert_eq!(result2.pid, 10001);
    }

    #[test]
    fn test_failable_mock_toggles_setup_failure() {
        let mock = FailableFilesystemMock::new();

        // Default: success
        assert!(mock.setup_rootfs(&[], Path::new("/test")).is_ok());
        assert_eq!(mock.setup_count(), 1);

        // Toggle on
        mock.set_fail_setup(true);
        assert!(mock.setup_rootfs(&[], Path::new("/test")).is_err());
        assert_eq!(mock.setup_count(), 2);

        // Toggle off
        mock.set_fail_setup(false);
        assert!(mock.setup_rootfs(&[], Path::new("/test")).is_ok());
        assert_eq!(mock.setup_count(), 3);
    }

    #[test]
    fn test_failable_mock_toggles_cleanup_failure() {
        let mock = FailableFilesystemMock::new();

        assert!(mock.cleanup(Path::new("/test")).is_ok());
        mock.set_fail_cleanup(true);
        assert!(mock.cleanup(Path::new("/test")).is_err());
        assert_eq!(mock.cleanup_count(), 2);
    }

    #[tokio::test]
    async fn test_mock_network_setup() {
        let net = MockNetwork::new();
        assert_eq!(net.setup_count(), 0);
        let result = net.setup("container-1", &NetworkConfig::default()).await;
        assert!(result.is_ok());
        assert_eq!(net.setup_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_network_cleanup() {
        let net = MockNetwork::new();
        let result = net.cleanup("container-1").await;
        assert!(result.is_ok());
        assert_eq!(net.cleanup_count(), 1);
    }
}

#[cfg(test)]
mod macro_contract_tests {
    use super::*;
    use minibox_core::domain::{
        ContainerRuntime, FilesystemProvider, ImageRegistry, ResourceLimiter,
    };
    use std::sync::Arc;

    #[test]
    fn mock_registry_downcasts_to_concrete() {
        let arc: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());
        let result = arc.as_ref().as_any().downcast_ref::<MockRegistry>();
        assert!(
            result.is_some(),
            "MockRegistry must downcast to itself via as_any()"
        );
    }

    #[test]
    fn wrong_type_downcast_returns_none() {
        let arc: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());
        // Downcasting to a completely different concrete type must return None, not panic
        let result = arc.as_ref().as_any().downcast_ref::<MockFilesystem>();
        assert!(result.is_none(), "wrong-type downcast must return None");
    }

    #[test]
    fn default_matches_new() {
        // default_new! implements Default by delegating to ::new()
        // If this compiles and runs, the implementation is correct
        let _via_default = MockRegistry::default();
        let _via_new = MockRegistry::new();
    }

    #[test]
    fn all_mock_types_downcast_correctly() {
        let fs: Arc<dyn FilesystemProvider> = Arc::new(MockFilesystem::new());
        assert!(
            fs.as_ref()
                .as_any()
                .downcast_ref::<MockFilesystem>()
                .is_some()
        );

        let limiter: Arc<dyn ResourceLimiter> = Arc::new(MockLimiter::new());
        assert!(
            limiter
                .as_ref()
                .as_any()
                .downcast_ref::<MockLimiter>()
                .is_some()
        );

        let runtime: Arc<dyn ContainerRuntime> = Arc::new(MockRuntime::new());
        assert!(
            runtime
                .as_ref()
                .as_any()
                .downcast_ref::<MockRuntime>()
                .is_some()
        );
    }
}
