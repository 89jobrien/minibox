//! # minibox
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::doc_markdown,
        clippy::unwrap_in_result,
        clippy::uninlined_format_args,
        clippy::redundant_clone,
        clippy::redundant_closure,
        clippy::redundant_closure_for_method_calls,
        clippy::single_char_pattern,
        clippy::collapsible_if,
        clippy::match_same_arms,
        clippy::only_used_in_recursion,
        clippy::used_underscore_binding,
        clippy::map_unwrap_or,
        clippy::manual_assert,
        clippy::as_ptr_cast_mut,
        clippy::ptr_as_ptr,
        clippy::must_use_candidate,
        clippy::used_underscore_items,
        clippy::missing_const_for_fn,
        clippy::manual_string_new,
        clippy::semicolon_if_nothing_returned,
        clippy::redundant_field_names,
        clippy::unreadable_literal,
        clippy::ref_as_ptr,
        clippy::default_constructed_unit_structs,
        clippy::allow_attributes_without_reason,
        clippy::needless_raw_string_hashes,
        clippy::manual_is_variant_and,
        clippy::ignore_without_reason,
        clippy::default_trait_access,
        clippy::cast_lossless,
        clippy::if_not_else,
        clippy::print_literal,
    )
)]
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
pub mod container_state;
pub mod daemon;
pub mod fs_util;
pub mod nesting;

// The `as_any!` and `adapt!` macros from minibox-macros expand to
// `crate::domain::AsAny` at the call site. Re-export the domain module here
// so those macro invocations in minibox source files resolve correctly.
// Also re-export error so container/ modules can still use `crate::error::*`
// through their own `use minibox_core::error::*` — but other callers that
// relied on `minibox::domain` or `minibox::error` continue to compile.
pub use minibox_core::domain;
pub mod error;
pub use minibox_core::image;
pub use minibox_core::preflight;
pub use minibox_core::protocol;
// Convenience re-exports for ImageRef used by daemon and miniboxd.
pub use minibox_core::image::reference::{ImageRef, ImageRefError};
pub use minibox_core::require_capability;

#[cfg(feature = "test-utils")]
pub mod testing;
