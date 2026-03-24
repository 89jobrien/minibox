//! Shared daemon application layer.
//!
//! Contains the request handlers, in-memory state, and Unix socket server
//! used by both `miniboxd` (Linux/Windows) and `macbox` (macOS).

pub mod handler;
pub mod network_lifecycle;
pub mod server;
pub mod state;
