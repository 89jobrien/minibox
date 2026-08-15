//! MCP server for controlling minibox through the daemon socket protocol.
//!
//! The Cargo package is published as `minibox-mcp`; the Rust library crate and
//! default binary are named `mcp`.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::doc_markdown,
        clippy::unwrap_in_result
    )
)]

pub mod client;
pub mod error;
pub mod policy;
pub mod server;
pub mod tools;
pub mod types;

pub use server::MiniboxMcpServer;
