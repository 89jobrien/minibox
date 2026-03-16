//! Infrastructure adapters implementing domain traits.
//!
//! This module contains concrete implementations of the domain layer traits
//! (ports) defined in [`crate::domain`]. Each adapter wraps existing
//! infrastructure code and provides a clean interface following hexagonal
//! architecture principles.
//!
//! # Architecture
//!
//! ```text
//! Domain Layer (Traits)     →     Infrastructure Adapters
//! ─────────────────────────────────────────────────────────
//! ImageRegistry             →     DockerHubRegistry
//! FilesystemProvider        →     OverlayFilesystem
//! ResourceLimiter           →     CgroupV2Limiter
//! ContainerRuntime          →     LinuxNamespaceRuntime
//! ```
//!
//! # Adapters
//!
//! **Platform-Native (Linux):**
//! - [`DockerHubRegistry`]: Docker Hub implementation of [`ImageRegistry`]
//! - [`OverlayFilesystem`]: Overlay filesystem implementation of [`FilesystemProvider`]
//! - [`CgroupV2Limiter`]: cgroups v2 implementation of [`ResourceLimiter`]
//! - [`LinuxNamespaceRuntime`]: Linux namespaces implementation of [`ContainerRuntime`]
//!
//! **Cross-Platform (Windows via WSL2):**
//! - [`WslRuntime`]: Windows Subsystem for Linux implementation
//! - [`WslFilesystem`]: WSL2-based filesystem provider
//! - [`WslLimiter`]: WSL2-based resource limiter
//!
//! **Cross-Platform (macOS via Docker Desktop):**
//! - [`DockerDesktopRuntime`]: Docker Desktop VM implementation
//! - [`DockerDesktopFilesystem`]: Docker Desktop-based filesystem provider
//! - [`DockerDesktopLimiter`]: Docker Desktop-based resource limiter
//!
//! # Usage
//!
//! Adapters are typically instantiated in the composition root (main.rs) and
//! injected into the business logic layer:
//!
//! ```rust,ignore
//! use minibox_lib::adapters::DockerHubRegistry;
//! use minibox_lib::domain::ImageRegistry;
//! use minibox_lib::image::ImageStore;
//! use std::sync::Arc;
//!
//! let store = Arc::new(ImageStore::new("/var/lib/minibox/images")?);
//! let registry: Arc<dyn ImageRegistry> = Arc::new(
//!     DockerHubRegistry::new(store.clone())?
//! );
//! ```

// Platform-native adapters (Linux only)
mod registry;
#[cfg(target_os = "linux")]
mod filesystem;
#[cfg(target_os = "linux")]
mod limiter;
#[cfg(target_os = "linux")]
mod runtime;

// Cross-platform adapters
mod wsl;
mod docker_desktop;

// Test doubles (always available for testing)
pub mod mocks;

// Linux-native exports (only on Linux)
pub use registry::DockerHubRegistry;
#[cfg(target_os = "linux")]
pub use filesystem::OverlayFilesystem;
#[cfg(target_os = "linux")]
pub use limiter::CgroupV2Limiter;
#[cfg(target_os = "linux")]
pub use runtime::LinuxNamespaceRuntime;

// Cross-platform exports (always available)
pub use wsl::{WslRuntime, WslFilesystem, WslLimiter};
pub use docker_desktop::{DockerDesktopRuntime, DockerDesktopFilesystem, DockerDesktopLimiter};
