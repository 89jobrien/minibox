//! Domain layer: Pure business logic and trait definitions (ports).
//!
//! This module defines the contracts (traits) that infrastructure adapters
//! must implement. Following hexagonal architecture principles, the domain
//! layer has **zero dependencies** on infrastructure details.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │              Composition Root                   │
//! │                  (main.rs)                      │
//! │   Wires domain to adapters, injects deps        │
//! └────────────────┬────────────────────────────────┘
//!                  │
//!     ┌────────────┴────────────┐
//!     │                         │
//! ┌───▼─────────────┐    ┌──────▼──────────────┐
//! │   Domain Layer  │    │ Infrastructure      │
//! │   (this file)   │    │    Adapters         │
//! │  ┌──────────┐   │    │  ┌──────────────┐   │
//! │  │Business  │   │    │  │ DockerHub    │   │
//! │  │Logic     │───┼────┼─►│ Registry     │   │
//! │  └──────────┘   │    │  └──────────────┘   │
//! │                 │    │  ┌──────────────┐   │
//! │  ┌──────────┐   │    │  │ Overlay      │   │
//! │  │Traits    │◄──┼────┼──│ Filesystem   │   │
//! │  │(Ports)   │   │    │  └──────────────┘   │
//! │  └──────────┘   │    │  ┌──────────────┐   │
//! │  ┌──────────┐   │    │  │ Cgroup V2    │   │
//! │  │Domain    │   │    │  │ Limiter      │   │
//! │  │Types     │   │    │  └──────────────┘   │
//! │  └──────────┘   │    └─────────────────────┘
//! └─────────────────┘
//! ```
//!
//! Dependencies point inward: adapters → domain
//!
//! # Traits (Ports)
//!
//! - [`ImageRegistry`]: Abstraction for pulling container images
//! - [`FilesystemProvider`]: Abstraction for container filesystem operations
//! - [`ResourceLimiter`]: Abstraction for resource isolation and limits
//! - [`ContainerRuntime`]: Abstraction for spawning container processes
//!
//! # Benefits
//!
//! - **Testability**: Easy to create mock implementations for unit tests
//! - **Flexibility**: Swap implementations (e.g., Docker Hub → ghcr.io)
//! - **Maintainability**: Clear separation of concerns
//! - **Future-proofing**: Add new backends without changing business logic

// Core domain traits
pub mod execution_manifest;
pub mod execution_policy;
mod extensions;
mod networking;

// Re-exports for public API
pub use execution_manifest::*;
pub use execution_policy::*;
pub use extensions::*;
pub use networking::*;

// Re-export slashcrux vocabulary types for agentic workflow metadata.
pub use slashcrux::{ExecutionContext, Priority, StepState, Urgency};

mod checkpoint;
mod error;
mod exec;
mod filesystem;
mod ids;
mod image;
mod legacy;
mod pty;
mod runtime;
mod state;

pub use checkpoint::*;
pub use error::*;
pub use exec::*;
pub use filesystem::*;
pub use ids::*;
pub use image::*;
pub use legacy::*;
pub use pty::*;
pub use runtime::*;
pub use state::*;
