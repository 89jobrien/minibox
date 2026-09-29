//! Compatibility facade for minibox VM adapter suites.
//!
//! This crate preserves stable import paths for lightweight VM backends:
//!
//! - [`smolvm`] re-exports the implementation from `minibox::adapters`
//! - [`krun`] re-exports the implementation from `macbox::krun`
//! - [`preflight`] -- smolvm binary detection and version checking

pub mod krun;
pub mod preflight;
pub mod smolvm;
