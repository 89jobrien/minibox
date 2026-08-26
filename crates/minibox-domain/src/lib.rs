#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::doc_markdown,
        clippy::missing_const_for_fn,
        clippy::panic,
        clippy::redundant_clone,
        clippy::single_match_else,
        clippy::unwrap_used,
        clippy::uninlined_format_args
    )
)]
//! Pure domain values and ports for the minibox container runtime.
//!
//! This crate is the innermost dependency ring. It contains no network,
//! filesystem, process, socket, tracing-subscriber, or async-runtime adapters.

use std::any::Any;

pub mod capability;
pub mod checkpoint;
pub mod error;
pub mod events;
pub mod exec;
pub mod execution_manifest;
pub mod execution_policy;
pub mod extensions;
pub mod filesystem;
pub mod ids;
pub mod image;
pub mod image_reference;
pub mod metrics;
pub mod networking;
pub mod path;
pub mod progress;
pub mod pty;
pub mod runtime;
pub mod state;
pub mod workflow;

pub use capability::*;
pub use checkpoint::*;
pub use error::*;
pub use events::*;
pub use exec::*;
pub use execution_manifest::*;
pub use execution_policy::*;
pub use extensions::*;
pub use filesystem::*;
pub use ids::*;
pub use image::*;
pub use image_reference::{ImageRef, ImageRefError};
pub use metrics::*;
pub use networking::*;
pub use path::*;
pub use progress::*;
pub use pty::*;
pub use runtime::*;
pub use slashcrux::{ExecutionContext, Priority, StepState, Urgency};
pub use state::*;
pub use workflow::*;

/// Enables test and composition code to downcast domain port implementations.
pub trait AsAny: Send + Sync {
    /// Return this value as [`Any`].
    fn as_any(&self) -> &dyn Any;
}
