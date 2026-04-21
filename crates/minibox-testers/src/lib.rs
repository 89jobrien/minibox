//! Test infrastructure for minibox — mocks, fixtures, conformance types.
//!
//! This crate is a `[dev-dependency]` only. It must never be compiled into
//! production binaries. All modules are public so downstream test files can
//! import them directly.

pub mod backend;
pub mod fixtures;
pub mod helpers;
pub mod mocks;
pub mod report;
