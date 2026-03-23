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
//! - [`Wsl2Runtime`]: Windows Subsystem for Linux implementation
//! - [`Wsl2Filesystem`]: WSL2-based filesystem provider
//! - [`Wsl2Limiter`]: WSL2-based resource limiter
//!
//! **Cross-Platform (macOS via Virtualization.framework — Phase 2):**
//! - [`VfRuntime`]: Apple Virtualization Framework runtime stub
//! - [`VfFilesystem`]: VF filesystem provider stub
//! - [`VfLimiter`]: VF resource limiter stub
//! - [`VfRegistry`]: VF image registry stub
//!
//! **Cross-Platform (Windows via HCS — Phase 2):**
//! - [`HcsRuntime`]: Windows Host Compute Service runtime stub
//! - [`HcsFilesystem`]: HCS filesystem provider stub
//! - [`HcsLimiter`]: HCS resource limiter stub
//! - [`HcsRegistry`]: HCS image registry stub
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
//! use linuxbox::adapters::DockerHubRegistry;
//! use linuxbox::domain::DynImageRegistry;
//! use linuxbox::image::ImageStore;
//! use std::sync::Arc;
//!
//! let store = Arc::new(ImageStore::new("/var/lib/minibox/images")?);
//! let registry: DynImageRegistry = Arc::new(
//!     DockerHubRegistry::new(store.clone())?
//! );
//! ```

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
mod hcs;
mod vf;
mod wsl2;

// Network adapters (always available — no platform restrictions)
pub mod network;

// Test doubles (always available for testing)
pub mod mocks;

// Shared test fixtures (test mode only)
#[cfg(test)]
pub mod test_fixtures;

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
pub use colima::{
    ColimaFilesystem, ColimaLimiter, ColimaRegistry, ColimaRuntime, LimaExecutor, LimaSpawner,
};
pub use docker_desktop::{DockerDesktopFilesystem, DockerDesktopLimiter, DockerDesktopRuntime};
pub use hcs::{HcsFilesystem, HcsLimiter, HcsRegistry, HcsRuntime};
pub use network::{HostNetwork, NoopNetwork};
pub use vf::{VfFilesystem, VfLimiter, VfRegistry, VfRuntime};
pub use wsl2::{Wsl2Filesystem, Wsl2Limiter, Wsl2Runtime};
