//! # mbx
//!
//! Linux-only container primitives for the Minibox container runtime.
//!
//! Cross-platform shared types (domain traits, protocol, image handling, error
//! types, preflight probes) live in [`minibox_core`]. This crate contains only
//! Linux-specific container infrastructure.
//!
//! ## Module overview
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`container`] | Linux-only container primitives: namespace setup (`clone(2)`), cgroups v2 manipulation, overlay filesystem mounting, `pivot_root`, and the container init process. Gated on `target_os = "linux"`. |
//! | [`adapters`] | Concrete Linux/platform adapter implementations of domain traits (overlay FS, cgroups v2, Colima/macOS, GKE, etc.). |

pub mod adapters;
#[cfg(target_os = "linux")]
pub mod container;
pub mod daemonbox_state;

// The `as_any!` and `adapt!` macros from minibox-macros expand to
// `crate::domain::AsAny` at the call site. Re-export the domain module here
// so those macro invocations in mbx source files resolve correctly.
// Also re-export error so container/ modules can still use `crate::error::*`
// through their own `use minibox_core::error::*` — but other callers that
// relied on `mbx::domain` or `mbx::error` continue to compile.
pub use minibox_core::domain;
pub mod error;
pub use minibox_core::image;
pub use minibox_core::preflight;
pub use minibox_core::protocol;
// Convenience re-exports for ImageRef used by daemonbox and miniboxd.
pub use minibox_core::image::reference::{ImageRef, ImageRefError};
pub use minibox_core::require_capability;
