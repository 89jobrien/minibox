//! Tailscale-rs network adapter for minibox containers.
//!
//! Implements [`minibox_core::domain::NetworkProvider`] via [`TailnetNetwork`].
//!
//! Two modes:
//! - **Gateway** — one shared `tailscale::Device` for the daemon; containers reach
//!   tailnet peers via proxy connections through that device.
//! - **PerContainer** — each container gets its own `tailscale::Device` and tailnet IP.
//!
//! The entire crate is intended to be compiled only when `miniboxd` is built with
//! `--features tailnet`.
//!
//! # Platform support
//!
//! Linux and macOS ARM64 only, matching `tailscale-rs` v0.2 platform support.

pub mod adapter;
pub mod auth;
pub mod config;
pub mod experiment;

pub use adapter::TailnetNetwork;
pub use config::TailnetConfig;
