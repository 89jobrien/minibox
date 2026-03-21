//! Mock adapters for testing.
//!
//! This module provides mock implementations of all domain traits, allowing
//! business logic to be tested without real infrastructure dependencies
//! (no Docker Hub, no cgroups, no Linux syscalls).
//!
//! # Usage
//!
//! ```rust,ignore
//! use minibox_lib::adapters::mocks::{MockRegistry, MockFilesystem, MockLimiter, MockRuntime};
//! use minibox_lib::domain::*;
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

use crate::adapt;
use crate::domain::{
    ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ImageMetadata, ImageRegistry,
    LayerInfo, ResourceConfig, ResourceLimiter, RuntimeCapabilities, SpawnResult,
};
use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// MockRegistry
// ---------------------------------------------------------------------------

/// Mock image registry for testing.
///
/// Simulates an image registry without network calls. Configure behavior
/// using builder methods.
#[derive(Debug, Clone)]
pub struct MockRegistry {
    state: Arc<Mutex<MockRegistryState>>,
}

#[derive(Debug)]
struct MockRegistryState {
    /// Images that are "cached" locally.
    cached_images: Vec<(String, String)>, // (name, tag)
    /// Whether pull operations should succeed.
    pull_should_succeed: bool,
    /// Number of times pull_image was called.
    pull_count: usize,
}

impl MockRegistry {
    /// Create a new mock registry with default settings.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockRegistryState {
                cached_images: Vec::new(),
                pull_should_succeed: true,
                pull_count: 0,
            })),
        }
    }

    /// Configure the registry to report an image as cached.
    pub fn with_cached_image(self, name: &str, tag: &str) -> Self {
        self.state
            .lock()
            .unwrap()
            .cached_images
            .push((name.to_string(), tag.to_string()));
        self
    }

    /// Configure pull operations to fail.
    pub fn with_pull_failure(self) -> Self {
        self.state.lock().unwrap().pull_should_succeed = false;
        self
    }

    /// Get the number of times pull_image was called.
    pub fn pull_count(&self) -> usize {
        self.state.lock().unwrap().pull_count
    }

    /// Sync test helper — bypasses async machinery for benchmarks.
    pub fn has_image_sync(&self, image: &str, tag: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .cached_images
            .contains(&(image.to_string(), tag.to_string()))
    }
}

#[async_trait]
impl ImageRegistry for MockRegistry {
    async fn has_image(&self, name: &str, tag: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .cached_images
            .contains(&(name.to_string(), tag.to_string()))
    }

    async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata> {
        let mut state = self.state.lock().unwrap();
        state.pull_count += 1;

        if !state.pull_should_succeed {
            anyhow::bail!("mock pull failure");
        }

        // Simulate successful pull by adding to cached images
        state
            .cached_images
            .push((name.to_string(), tag.to_string()));

        Ok(ImageMetadata {
            name: name.to_string(),
            tag: tag.to_string(),
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

    fn get_image_layers(&self, _name: &str, _tag: &str) -> Result<Vec<PathBuf>> {
        Ok(vec![
            PathBuf::from("/mock/layer1"),
            PathBuf::from("/mock/layer2"),
        ])
    }
}

// ---------------------------------------------------------------------------
// MockFilesystem
// ---------------------------------------------------------------------------

/// Mock filesystem provider for testing.
///
/// Simulates filesystem operations without actual mounts or syscalls.
#[derive(Debug, Clone)]
pub struct MockFilesystem {
    state: Arc<Mutex<MockFilesystemState>>,
}

#[derive(Debug)]
struct MockFilesystemState {
    setup_should_succeed: bool,
    pivot_should_succeed: bool,
    cleanup_should_succeed: bool,
    setup_count: usize,
    cleanup_count: usize,
}

impl MockFilesystem {
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

    pub fn with_setup_failure(self) -> Self {
        self.state.lock().unwrap().setup_should_succeed = false;
        self
    }

    pub fn setup_count(&self) -> usize {
        self.state.lock().unwrap().setup_count
    }

    pub fn cleanup_count(&self) -> usize {
        self.state.lock().unwrap().cleanup_count
    }
}

impl FilesystemProvider for MockFilesystem {
    fn setup_rootfs(&self, _layers: &[PathBuf], container_dir: &Path) -> Result<PathBuf> {
        let mut state = self.state.lock().unwrap();
        state.setup_count += 1;

        if !state.setup_should_succeed {
            anyhow::bail!("mock filesystem setup failure");
        }

        Ok(container_dir.join("merged"))
    }

    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        let state = self.state.lock().unwrap();
        if !state.pivot_should_succeed {
            anyhow::bail!("mock pivot_root failure");
        }
        Ok(())
    }

    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.cleanup_count += 1;

        if !state.cleanup_should_succeed {
            anyhow::bail!("mock cleanup failure");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MockLimiter
// ---------------------------------------------------------------------------

/// Mock resource limiter for testing.
///
/// Simulates cgroup operations without actual kernel interaction.
#[derive(Debug, Clone)]
pub struct MockLimiter {
    state: Arc<Mutex<MockLimiterState>>,
}

#[derive(Debug)]
struct MockLimiterState {
    create_should_succeed: bool,
    add_process_should_succeed: bool,
    cleanup_should_succeed: bool,
    create_count: usize,
    cleanup_count: usize,
    created_cgroups: Vec<String>, // container IDs
}

impl MockLimiter {
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

    pub fn with_create_failure(self) -> Self {
        self.state.lock().unwrap().create_should_succeed = false;
        self
    }

    pub fn create_count(&self) -> usize {
        self.state.lock().unwrap().create_count
    }

    pub fn cleanup_count(&self) -> usize {
        self.state.lock().unwrap().cleanup_count
    }
}

impl ResourceLimiter for MockLimiter {
    fn create(&self, container_id: &str, _config: &ResourceConfig) -> Result<String> {
        let mut state = self.state.lock().unwrap();
        state.create_count += 1;

        if !state.create_should_succeed {
            anyhow::bail!("mock resource limiter create failure");
        }

        state.created_cgroups.push(container_id.to_string());
        Ok(format!("/mock/cgroup/{container_id}"))
    }

    fn add_process(&self, _container_id: &str, _pid: u32) -> Result<()> {
        let state = self.state.lock().unwrap();
        if !state.add_process_should_succeed {
            anyhow::bail!("mock add_process failure");
        }
        Ok(())
    }

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

/// Mock container runtime for testing.
///
/// Simulates container process spawning without actual syscalls.
#[derive(Debug, Clone)]
pub struct MockRuntime {
    state: Arc<Mutex<MockRuntimeState>>,
}

#[derive(Debug)]
struct MockRuntimeState {
    spawn_should_succeed: bool,
    next_pid: u32,
    spawn_count: usize,
}

impl MockRuntime {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockRuntimeState {
                spawn_should_succeed: true,
                next_pid: 10000,
                spawn_count: 0,
            })),
        }
    }

    pub fn with_spawn_failure(self) -> Self {
        self.state.lock().unwrap().spawn_should_succeed = false;
        self
    }

    pub fn spawn_count(&self) -> usize {
        self.state.lock().unwrap().spawn_count
    }

    /// Sync test helper — bypasses async machinery for benchmarks.
    pub fn spawn_process_sync(&self, _cfg: &ContainerSpawnConfig) -> Result<SpawnResult> {
        let mut state = self.state.lock().unwrap();
        state.spawn_count += 1;
        if !state.spawn_should_succeed {
            anyhow::bail!("mock spawn failure");
        }
        let pid = state.next_pid;
        state.next_pid += 1;
        Ok(SpawnResult {
            pid,
            output_reader: None,
        })
    }
}

#[async_trait]
impl ContainerRuntime for MockRuntime {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: false,
            supports_overlay_fs: false,
            supports_network_isolation: false,
            max_containers: None,
        }
    }

    async fn spawn_process(&self, _config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        let mut state = self.state.lock().unwrap();
        state.spawn_count += 1;

        if !state.spawn_should_succeed {
            anyhow::bail!("mock spawn failure");
        }

        let pid = state.next_pid;
        state.next_pid += 1;
        Ok(SpawnResult {
            pid,
            output_reader: None,
        })
    }
}

adapt!(MockRegistry, MockFilesystem, MockLimiter, MockRuntime);

// ---------------------------------------------------------------------------
// FailableFilesystemMock
// ---------------------------------------------------------------------------

/// Filesystem mock with runtime-controllable failure injection via atomics.
///
/// Unlike `MockFilesystem` (whose failure mode is fixed at construction),
/// this mock lets tests toggle failures between calls.
pub struct FailableFilesystemMock {
    should_fail_setup: AtomicBool,
    should_fail_cleanup: AtomicBool,
    setup_count: AtomicUsize,
    cleanup_count: AtomicUsize,
}

impl FailableFilesystemMock {
    pub fn new() -> Self {
        Self {
            should_fail_setup: AtomicBool::new(false),
            should_fail_cleanup: AtomicBool::new(false),
            setup_count: AtomicUsize::new(0),
            cleanup_count: AtomicUsize::new(0),
        }
    }

    /// Toggle whether `setup_rootfs` returns an error.
    pub fn set_fail_setup(&self, fail: bool) {
        self.should_fail_setup.store(fail, Ordering::SeqCst);
    }

    /// Toggle whether `cleanup` returns an error.
    pub fn set_fail_cleanup(&self, fail: bool) {
        self.should_fail_cleanup.store(fail, Ordering::SeqCst);
    }

    pub fn setup_count(&self) -> usize {
        self.setup_count.load(Ordering::SeqCst)
    }

    pub fn cleanup_count(&self) -> usize {
        self.cleanup_count.load(Ordering::SeqCst)
    }
}

impl FilesystemProvider for FailableFilesystemMock {
    fn setup_rootfs(&self, _layers: &[PathBuf], container_dir: &Path) -> Result<PathBuf> {
        self.setup_count.fetch_add(1, Ordering::SeqCst);
        if self.should_fail_setup.load(Ordering::SeqCst) {
            anyhow::bail!("injected setup failure");
        }
        Ok(container_dir.join("merged"))
    }

    fn pivot_root(&self, _new_root: &Path) -> Result<()> {
        Ok(())
    }

    fn cleanup(&self, _container_dir: &Path) -> Result<()> {
        self.cleanup_count.fetch_add(1, Ordering::SeqCst);
        if self.should_fail_cleanup.load(Ordering::SeqCst) {
            anyhow::bail!("injected cleanup failure");
        }
        Ok(())
    }
}

adapt!(FailableFilesystemMock);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ContainerHooks;

    #[test]
    fn mock_registry_has_image_sync_cached() {
        let reg = MockRegistry::new().with_cached_image("alpine", "latest");
        assert!(reg.has_image_sync("alpine", "latest"));
        assert!(!reg.has_image_sync("alpine", "missing"));
    }

    #[test]
    fn mock_runtime_spawn_process_sync_increments_count() {
        use crate::domain::{ContainerHooks, ContainerSpawnConfig};
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
        let result = registry.pull_image("library/alpine", "latest").await;
        assert!(result.is_ok());
        assert_eq!(registry.pull_count(), 1);

        // After pull, image should be cached
        assert!(registry.has_image("library/alpine", "latest").await);
    }

    #[tokio::test]
    async fn test_mock_registry_pull_failure() {
        let registry = MockRegistry::new().with_pull_failure();

        let result = registry.pull_image("library/alpine", "latest").await;
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
}

#[cfg(test)]
mod macro_contract_tests {
    use super::*;
    use crate::domain::{ContainerRuntime, FilesystemProvider, ImageRegistry, ResourceLimiter};
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
