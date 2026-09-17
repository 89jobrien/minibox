//! MCP server for controlling minibox through the daemon socket protocol.
//!
//! The Cargo package is published as `minibox-mcp`; the Rust library crate and
//! default binary are named `mcp`.
//!
//! Generic mutating daemon calls require policy authorization:
//!
//! ```no_run
//! use mcp::{client::MiniboxDaemonClient, policy::AgentPolicy};
//! use minibox_core::protocol::DaemonRequest;
//!
//! # async fn stop() -> mcp::error::Result<()> {
//! let client = MiniboxDaemonClient::from_env();
//! let policy = AgentPolicy::from_env();
//! let request = policy.authorize_mutation(
//!     "custom_stop",
//!     DaemonRequest::Stop { id: "example".into() },
//! )?;
//! let (_result, _output_truncated) = client
//!     .call_authorized(request, policy.max_output_bytes)
//!     .await?;
//! # Ok(())
//! # }
//! ```
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
