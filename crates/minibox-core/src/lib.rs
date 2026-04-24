//! # minibox-core
//!
//! Cross-platform shared types, domain traits, protocol definitions, and
//! image handling for the Minibox container runtime.
//!
//! ## Module overview
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`domain`] | Hexagonal-architecture ports: trait definitions for [`domain::ImageRegistry`], [`domain::FilesystemProvider`], [`domain::ResourceLimiter`], and [`domain::ContainerRuntime`]. Zero infrastructure dependencies. |
//! | [`adapters`] | Cross-platform adapter implementations: Docker Hub registry, mock test doubles, and shared test fixtures. |
//! | [`image`] | OCI image handling: parsing `image:tag` references, Docker Hub v2 registry client (anonymous token auth), manifest parsing, and tar layer extraction with path-traversal protection. |
//! | [`protocol`] | Newline-delimited JSON types for the Unix socket protocol between the daemon and CLI. Includes framing helpers and the streaming ephemeral run protocol. |
//! | [`preflight`] | Host capability probing (cgroups v2, overlay FS, kernel version, systemd). Used by `just doctor` and the `require_capability!` test macro. |
//! | [`error`] | Top-level [`MiniboxError`] type and cross-platform error types. |
//!
//! ## Feature flags
//!
//! - `test-utils`: enables mock adapters (`adapters::mocks`, `adapters::test_fixtures`)
//!   for use in other crates' dev-dependencies without `cfg(test)` restrictions.

pub mod adapters;
pub mod domain;
pub mod error;
pub mod events;
pub mod preflight;
pub mod protocol;
pub mod trace;
pub mod tracing_init;

pub use tracing_init::init_tracing;

// Image handling is provided by the standalone minibox-oci crate.
// Re-export the full module so existing `minibox_core::image::*` paths continue to work.
pub use minibox_oci::image;

pub use error::MiniboxError;
pub use minibox_macros::{adapt, as_any, default_new, require_capability};

/// Convenience re-export of the [`anyhow::Result`] type used throughout this crate.
pub type Result<T> = anyhow::Result<T>;
