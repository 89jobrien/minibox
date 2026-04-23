//! # minibox
//!
//! Ergonomic facade re-exporting the full Minibox container runtime surface.
//!
//! External consumers can depend on this crate alone to get everything:
//!
//! ```toml
//! [dependencies]
//! minibox = "0.19"
//! ```
//!
//! Then:
//!
//! ```rust,ignore
//! use minibox::*; // protocol, domain traits, image types, Linux primitives
//! ```
//!
//! ## Crate organisation
//!
//! | Crate | What it provides |
//! |-------|-----------------|
//! | [`minibox_core`] | Cross-platform: protocol, domain traits, image types, preflight |
//! | [`linuxbox`] | Linux-specific: namespaces, cgroups, overlay FS, adapters |

pub use linuxbox::*;
// Re-export minibox_core items not already covered by linuxbox's re-exports.
// linuxbox already re-exports minibox_core::{domain, image, preflight, protocol, error},
// so we only add the items unique to minibox_core here.
pub use minibox_core::require_capability;
