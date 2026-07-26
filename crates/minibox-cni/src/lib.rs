//! CNI (Container Network Interface) plugin exec protocol and chain
//! orchestration for minibox's native Linux adapter.
//!
//! This crate is deliberately ignorant of how a network namespace is
//! obtained — callers pass an opaque `netns: &str` target straight through
//! to the `CNI_NETNS` environment variable. On Linux that's a
//! `/proc/<pid>/ns/net` path; nothing in this crate assumes that format,
//! keeping the door open for a future non-Linux (WinCNI/HNS) caller
//! without modification here.

pub mod config;
pub mod error;
pub mod exec;
pub mod provider;
pub mod result;

pub use config::{NetworkConfigList, PluginConfig};
pub use error::CniError;
pub use provider::CniNetworkProvider;
pub use result::{CniDns, CniErrorPayload, CniInterface, CniIpConfig, CniResult};
