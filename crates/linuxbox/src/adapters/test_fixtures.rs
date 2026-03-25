//! Shared test fixtures for adapter and integration tests.
//!
//! This module provides builder utilities and temporary directory fixtures
//! to simplify test setup across the linuxbox test suite.
//!
//! # Usage
//!
//! ```rust,ignore
//! use linuxbox::adapters::test_fixtures::{MockAdapterBuilder, TempContainerFixture};
//!
//! #[tokio::test]
//! async fn test_something() {
//!     let adapters = MockAdapterBuilder::new()
//!         .with_cached_image("alpine", "latest")
//!         .build();
//!
//!     let fixture = TempContainerFixture::new().unwrap();
//!     // use adapters.registry, adapters.filesystem, etc.
//! }
//! ```

use minibox_core::domain::{
    DynContainerRuntime, DynFilesystemProvider, DynImageRegistry, DynResourceLimiter,
};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

use super::mocks::{MockFilesystem, MockLimiter, MockRegistry, MockRuntime};

// ---------------------------------------------------------------------------
// MockAdapterSet
// ---------------------------------------------------------------------------

/// A complete set of mock domain adapters, ready for injection into tests.
pub struct MockAdapterSet {
    pub filesystem: DynFilesystemProvider,
    pub limiter: DynResourceLimiter,
    pub registry: DynImageRegistry,
    pub runtime: DynContainerRuntime,
}

// ---------------------------------------------------------------------------
// MockAdapterBuilder
// ---------------------------------------------------------------------------

/// Builder for constructing [`MockAdapterSet`] with configurable failure modes.
///
/// All failure modes default to `false` (i.e. success). Use the `with_*`
/// methods to inject specific failures before calling [`build`](Self::build).
pub struct MockAdapterBuilder {
    fail_setup: bool,
    fail_create: bool,
    fail_pull: bool,
    fail_spawn: bool,
    cached_images: Vec<(String, String)>,
}

impl MockAdapterBuilder {
    /// Create a new builder with all adapters configured to succeed.
    pub fn new() -> Self {
        Self {
            fail_setup: false,
            fail_create: false,
            fail_pull: false,
            fail_spawn: false,
            cached_images: Vec::new(),
        }
    }

    /// Cause `FilesystemProvider::setup_rootfs` to return an error.
    pub fn with_setup_failure(mut self) -> Self {
        self.fail_setup = true;
        self
    }

    /// Cause `ResourceLimiter::create` to return an error.
    pub fn with_create_failure(mut self) -> Self {
        self.fail_create = true;
        self
    }

    /// Cause `ImageRegistry::pull_image` to return an error.
    pub fn with_pull_failure(mut self) -> Self {
        self.fail_pull = true;
        self
    }

    /// Cause `ContainerRuntime::spawn_process` to return an error.
    pub fn with_spawn_failure(mut self) -> Self {
        self.fail_spawn = true;
        self
    }

    /// Pre-populate the registry cache so `has_image` returns `true`.
    pub fn with_cached_image(mut self, name: &str, tag: &str) -> Self {
        self.cached_images.push((name.to_string(), tag.to_string()));
        self
    }

    /// Construct a [`MockAdapterSet`] with the configured failure modes.
    pub fn build(self) -> MockAdapterSet {
        let mut registry = MockRegistry::new();
        for (name, tag) in &self.cached_images {
            registry = registry.with_cached_image(name, tag);
        }
        if self.fail_pull {
            registry = registry.with_pull_failure();
        }

        let filesystem = if self.fail_setup {
            MockFilesystem::new().with_setup_failure()
        } else {
            MockFilesystem::new()
        };

        let limiter = if self.fail_create {
            MockLimiter::new().with_create_failure()
        } else {
            MockLimiter::new()
        };

        let runtime = if self.fail_spawn {
            MockRuntime::new().with_spawn_failure()
        } else {
            MockRuntime::new()
        };

        MockAdapterSet {
            filesystem: Arc::new(filesystem),
            limiter: Arc::new(limiter),
            registry: Arc::new(registry),
            runtime: Arc::new(runtime),
        }
    }
}

impl Default for MockAdapterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// TempContainerFixture
// ---------------------------------------------------------------------------

/// Temporary directory fixture providing `images/` and `containers/` subdirs.
///
/// The underlying [`TempDir`] is cleaned up when this struct is dropped.
pub struct TempContainerFixture {
    /// The root temporary directory (kept alive for Drop).
    pub dir: TempDir,
    /// Path to the `images/` subdirectory.
    pub images_dir: PathBuf,
    /// Path to the `containers/` subdirectory.
    pub containers_dir: PathBuf,
}

impl TempContainerFixture {
    /// Create a new fixture, creating `images/` and `containers/` inside a
    /// fresh temporary directory.
    pub fn new() -> std::io::Result<Self> {
        let dir = TempDir::new()?;
        let images_dir = dir.path().join("images");
        let containers_dir = dir.path().join("containers");
        std::fs::create_dir(&images_dir)?;
        std::fs::create_dir(&containers_dir)?;
        Ok(Self {
            dir,
            images_dir,
            containers_dir,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::ResourceConfig;
    use std::path::Path;

    #[test]
    fn test_builder_creates_success_adapters() {
        let adapters = MockAdapterBuilder::new().build();
        // filesystem setup_rootfs should succeed
        let result = adapters
            .filesystem
            .setup_rootfs(&[], Path::new("/container"));
        assert!(
            result.is_ok(),
            "default builder: setup_rootfs should succeed"
        );
        // limiter create should succeed
        let result = adapters
            .limiter
            .create("test-id", &ResourceConfig::default());
        assert!(
            result.is_ok(),
            "default builder: limiter create should succeed"
        );
    }

    #[test]
    fn test_builder_injects_setup_failure() {
        let adapters = MockAdapterBuilder::new().with_setup_failure().build();
        let result = adapters
            .filesystem
            .setup_rootfs(&[], Path::new("/container"));
        assert!(
            result.is_err(),
            "with_setup_failure: setup_rootfs should fail"
        );
    }

    #[tokio::test]
    async fn test_builder_injects_pull_failure() {
        let adapters = MockAdapterBuilder::new().with_pull_failure().build();
        let result = adapters.registry.pull_image("alpine", "latest").await;
        assert!(result.is_err(), "with_pull_failure: pull_image should fail");
    }

    #[tokio::test]
    async fn test_builder_with_cached_image() {
        let adapters = MockAdapterBuilder::new()
            .with_cached_image("alpine", "latest")
            .build();
        assert!(
            adapters.registry.has_image("alpine", "latest").await,
            "with_cached_image: has_image should return true"
        );
        assert!(
            !adapters.registry.has_image("ubuntu", "latest").await,
            "with_cached_image: uncached image should return false"
        );
    }

    #[test]
    fn test_temp_fixture_creates_dirs() {
        let fixture = TempContainerFixture::new().expect("fixture creation failed");
        assert!(
            fixture.images_dir.exists(),
            "images_dir should exist after fixture creation"
        );
        assert!(
            fixture.containers_dir.exists(),
            "containers_dir should exist after fixture creation"
        );
        assert!(fixture.images_dir.is_dir());
        assert!(fixture.containers_dir.is_dir());
    }
}
