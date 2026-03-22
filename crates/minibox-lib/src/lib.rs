//! # minibox-lib
//!
//! Core library for the Minibox container runtime. This crate is the shared
//! foundation used by both the daemon (`miniboxd`) and the CLI (`minibox`).
//!
//! ## Module overview
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`domain`] | Hexagonal-architecture ports: trait definitions for [`domain::ImageRegistry`], [`domain::FilesystemProvider`], [`domain::ResourceLimiter`], and [`domain::ContainerRuntime`]. Zero infrastructure dependencies. |
//! | [`adapters`] | Concrete implementations of the domain traits (Docker Hub registry, overlay FS, cgroups v2, Colima/macOS, etc.). Wired together at startup via the `MINIBOX_ADAPTER` environment variable. |
//! | [`container`] | Linux-only container primitives: namespace setup (`clone(2)`), cgroups v2 manipulation, overlay filesystem mounting, `pivot_root`, and the container init process. Gated on `target_os = "linux"`. |
//! | [`image`] | OCI image handling: parsing `image:tag` references, Docker Hub v2 registry client (anonymous token auth), manifest parsing, and tar layer extraction with path-traversal protection. |
//! | [`protocol`] | Newline-delimited JSON types for the Unix socket protocol between the daemon and CLI. Includes framing helpers and the streaming ephemeral run protocol. |
//! | [`preflight`] | Host capability probing (cgroups v2, overlay FS, kernel version, systemd). Used by `just doctor` and the `require_capability!` test macro. |
//! | [`error`] | Top-level [`MiniboxError`] type and shared error utilities. |
//!
//! ## Architecture
//!
//! The crate follows a hexagonal (ports-and-adapters) architecture. The
//! [`domain`] module defines the contracts; the [`adapters`] module provides
//! the implementations. Tests use mock adapters from [`adapters::mocks`] to
//! exercise business logic without touching real infrastructure.
//!
//! ## Feature flags
//!
//! No Cargo feature flags are defined. Linux-only modules are gated with
//! `#[cfg(target_os = "linux")]`; the rest of the crate compiles on macOS
//! and Windows for IDE / CI purposes.

pub mod adapters;
#[cfg(target_os = "linux")]
pub mod container;
pub mod domain;
pub mod error;
pub mod image;
pub mod preflight;
pub mod protocol;

pub use error::MiniboxError;
pub use minibox_macros::{adapt, as_any, default_new};

/// Convenience re-export of the [`anyhow::Result`] type used throughout this crate.
pub type Result<T> = anyhow::Result<T>;
