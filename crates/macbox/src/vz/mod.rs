//! Thin Rust bindings for Apple's Virtualization.framework.

pub mod bindings;
pub mod proxy;
pub mod vm;
pub mod vsock;

pub use proxy::VzProxy;
