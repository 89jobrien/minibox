//! CNI (Container Network Interface) plugin exec protocol and chain
//! orchestration for minibox's native Linux adapter.
//!
//! This crate is deliberately ignorant of how a network namespace is
//! obtained — callers pass an opaque `netns: &str` target straight through
//! to the `CNI_NETNS` environment variable. On Linux that's a
//! `/proc/<pid>/ns/net` path; nothing in this crate assumes that format,
//! keeping the door open for a future non-Linux (WinCNI/HNS) caller
//! without modification here.
//!
//! The crate ships two installable binaries:
//!
//! - `minibox-cni-bridge` — the CNI plugin for the bridge network mode,
//!   installed on `CNI_PATH` and referenced by `"type": "minibox-bridge"`
//!   in a `.conflist`.
//! - `minibox-cni-verify` — the rollout preflight, which checks that the
//!   `.conflist`, the installed plugin binaries, and the spec version each
//!   binary advertises all agree.
//!
//! The spec version lives in exactly one place, [`version::CNI_SPEC_VERSION`].
//! The `VERSION` reply, the `ADD`/`DEL`/`CHECK` request validation, the
//! `.conflist` validation on the chain side, and the rollout preflight all
//! read it rather than repeating the literal.
//!
//! # Layout
//!
//! [`version`], [`plugin`], and [`rollout`] are the domain layer: pure
//! functions with no filesystem, process, or kernel access. [`adapters`]
//! holds the implementations of the ports those modules declare —
//! [`plugin::PluginEnvironment`], [`plugin::NamespaceAttachment`], and
//! [`rollout::PluginProbe`] — so the protocol logic can be tested
//! in-memory, with no root, no network namespace, and no CNI plugin on
//! disk.

pub mod adapters;
pub mod config;
pub mod error;
pub mod exec;
pub mod plugin;
pub mod provider;
pub mod result;
pub mod rollout;
pub mod version;

pub use config::{NetworkConfigList, PluginConfig};
pub use error::CniError;
pub use plugin::{
    AttachmentRequest, CniCommand, NamespaceAttachment, PluginEnvironment, PluginRequest,
};
pub use provider::CniNetworkProvider;
pub use result::{CniDns, CniErrorPayload, CniInterface, CniIpConfig, CniResult};
pub use rollout::{CniPreflightReport, CniRollout, PluginProbe};
pub use version::{CNI_SPEC_VERSION, VersionInfo, validate_spec_version, version_info};
