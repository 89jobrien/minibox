//! Adapter registry — re-export of the shared table in `minibox-core`.
//!
//! The authoritative definition now lives in
//! `minibox-core::adapter_registry` so that `miniboxd` (which selects an
//! adapter at startup) and `mbx doctor` (which reports what would be selected)
//! cannot drift apart. This module preserves the original
//! `miniboxd::adapter_registry::*` paths so existing call sites are unchanged.
//!
//! Do not add logic here — add it to `minibox-core` and re-export it.

pub use minibox_core::adapter_registry::*;
