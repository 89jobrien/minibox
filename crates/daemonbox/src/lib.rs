//! Shared daemon application layer.
//!
//! Contains the request handlers, in-memory state, and Unix socket server
//! used by both `miniboxd` (Linux) and `macboxd` (macOS).

pub mod handler;
pub mod server;
pub mod state;
