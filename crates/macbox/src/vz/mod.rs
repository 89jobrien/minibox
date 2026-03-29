//! Thin Rust bindings for Apple's Virtualization.framework.

pub mod adapter;
pub mod agent_init;
pub mod bindings;
pub mod proxy;
pub mod vm;
pub mod vsock;

pub use adapter::{VzFilesystem, VzLimiter, VzRegistry, VzRuntime};
pub use proxy::VzProxy;
