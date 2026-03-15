//! # minibox-lib
//!
//! Core library for the Minibox container runtime. Provides container lifecycle
//! management, image pulling, OCI manifest parsing, cgroups v2 resource limits,
//! Linux namespace isolation, overlay filesystem setup, and the daemon/CLI
//! communication protocol.

pub mod container;
pub mod error;
pub mod image;
pub mod protocol;

pub use error::MiniboxError;

/// Convenience re-export of the anyhow [`Result`] type used throughout this crate.
pub type Result<T> = anyhow::Result<T>;
