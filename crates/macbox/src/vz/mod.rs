//! macOS Virtualization.framework integration — vsock protocol bridging.
//!
//! This module provides building blocks for miniboxd on macOS via VZ (native
//! hypervisor on Apple silicon and Intel). Integrates with vsock for IPC between
//! the host daemon and the in-VM agent.
//!
//! # Modules
//!
//! - [`proxy`] — VzProxy for JSON-over-vsock request/response handling

pub mod proxy;

pub use proxy::VzProxy;
