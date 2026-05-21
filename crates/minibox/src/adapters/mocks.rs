//! Mock adapters for testing.
//!
//! This module re-exports mock implementations from [`minibox_core::adapters::mocks`],
//! providing a single canonical source for all test doubles. Each mock tracks call
//! counts and can be configured to fail on demand via builder methods.
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

pub use minibox_core::adapters::mocks::{
    FailableFilesystemMock, MockFilesystem, MockLimiter, MockNetwork, MockRegistry, MockRuntime,
};

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::{
        ContainerHooks, ContainerRuntime, ContainerSpawnConfig, ImageRegistry, NetworkConfig,
        NetworkProvider, ResourceConfig, ResourceLimiter, RootfsSetup,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn mock_registry_has_image_sync_cached() {
        let reg = MockRegistry::new().with_cached_image("alpine", "latest");
        assert!(reg.has_image_sync("alpine", "latest"));
        assert!(!reg.has_image_sync("alpine", "missing"));
    }

    #[test]
    fn mock_runtime_spawn_process_sync_increments_count() {
        let runtime = MockRuntime::new();
        let cfg = ContainerSpawnConfig {
            rootfs: PathBuf::from("/mock/rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            hostname: "mock".to_string(),
            cgroup_path: PathBuf::from("/mock/cgroup"),
            capture_output: false,
            hooks: ContainerHooks::default(),
            skip_network_namespace: false,
            mounts: vec![],
            privileged: false,
            image_ref: None,
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
            image_ref: None,
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
        let result = arc.as_ref().as_any().downcast_ref::<MockFilesystem>();
        assert!(result.is_none(), "wrong-type downcast must return None");
    }

    #[test]
    fn default_matches_new() {
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
