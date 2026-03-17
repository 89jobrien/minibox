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
//! **GKE Unprivileged (Linux, no CAP_SYS_ADMIN):**
//! - [`NoopLimiter`]: No-op resource limiter (cgroups unavailable)
//! - [`CopyFilesystem`]: Copy-based layer merging (no overlay FS)
//! - [`ProotRuntime`]: proot (ptrace-based) fake chroot runtime
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
//! **Cross-Platform (macOS via Colima/Lima):**
//! - [`ColimaRegistry`]: Colima VM implementation of [`ImageRegistry`]
//! - [`ColimaRuntime`]: Colima VM implementation
//! - [`ColimaFilesystem`]: Colima-based filesystem provider
//! - [`ColimaLimiter`]: Colima-based resource limiter
//!
//! # Usage
//!
//! Adapters are typically instantiated in the composition root (main.rs) and
//! injected into the business logic layer:
//!
//! ```rust,ignore
//! use minibox_lib::adapters::DockerHubRegistry;
//! use minibox_lib::domain::DynImageRegistry;
//! use minibox_lib::image::ImageStore;
//! use std::sync::Arc;
//!
//! let store = Arc::new(ImageStore::new("/var/lib/minibox/images")?);
//! let registry: DynImageRegistry = Arc::new(
//!     DockerHubRegistry::new(store.clone())?
//! );
//! ```

/// Implement [`crate::domain::AsAny`] for one or more types.
///
/// # Example
/// ```rust,ignore
/// as_any!(DockerHubRegistry);
/// as_any!(WslRuntime, WslFilesystem, WslLimiter);
/// ```
#[macro_export]
macro_rules! as_any {
    ($($t:ty),+ $(,)?) => {
        $(
            impl $crate::domain::AsAny for $t {
                fn as_any(&self) -> &dyn ::std::any::Any {
                    self
                }
            }
        )+
    };
}

/// Implement `Default` by delegating to `Self::new()`.
///
/// Only valid for types whose `new()` takes no arguments.
///
/// # Example
/// ```rust,ignore
/// default_new!(CgroupV2Limiter, OverlayFilesystem);
/// ```
#[macro_export]
macro_rules! default_new {
    ($($t:ty),+ $(,)?) => {
        $(
            impl Default for $t {
                fn default() -> Self {
                    Self::new()
                }
            }
        )+
    };
}

/// Implement both [`crate::domain::AsAny`] and `Default` for one or more types.
///
/// Equivalent to calling [`as_any!`] and [`default_new!`] with the same list.
/// Only valid for types whose `new()` takes no arguments.
///
/// # Example
/// ```rust,ignore
/// adapt!(ColimaRegistry, ColimaFilesystem, ColimaLimiter, ColimaRuntime);
/// ```
#[macro_export]
macro_rules! adapt {
    ($($t:ty),+ $(,)?) => {
        $crate::as_any!($($t),+);
        $crate::default_new!($($t),+);
    };
}

// Platform-native adapters (Linux only)
#[cfg(target_os = "linux")]
mod filesystem;
#[cfg(target_os = "linux")]
mod limiter;
mod registry;
#[cfg(target_os = "linux")]
mod runtime;

// GKE unprivileged adapters (proot-based, no kernel privileges needed)
mod gke;

// Cross-platform adapters
mod colima;
mod docker_desktop;
mod wsl;

// Test doubles (always available for testing)
pub mod mocks;

// Linux-native exports (only on Linux)
#[cfg(target_os = "linux")]
pub use filesystem::OverlayFilesystem;
#[cfg(target_os = "linux")]
pub use limiter::CgroupV2Limiter;
pub use registry::DockerHubRegistry;
#[cfg(target_os = "linux")]
pub use runtime::LinuxNamespaceRuntime;

// GKE unprivileged exports (Linux only at runtime, but compile-check everywhere)
#[cfg(target_os = "linux")]
pub use gke::{CopyFilesystem, NoopLimiter, ProotRuntime};

// Cross-platform exports (always available)
pub use colima::{ColimaFilesystem, ColimaLimiter, ColimaRegistry, ColimaRuntime};
pub use docker_desktop::{DockerDesktopFilesystem, DockerDesktopLimiter, DockerDesktopRuntime};
pub use wsl::{WslFilesystem, WslLimiter, WslRuntime};
