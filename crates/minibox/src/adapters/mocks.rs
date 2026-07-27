//! Mock adapters for testing — re-exported from `minibox-core`.
//!
//! All mock implementations live in `minibox_core::adapters::mocks` and are
//! re-exported here so that code inside the `minibox` crate can use the
//! canonical path `crate::adapters::mocks::MockXxx` without any duplication.
//!
//! `minibox-core` already gates this module on `any(test, feature =
//! "test-utils")`. Since the `minibox` crate unconditionally enables the
//! `test-utils` feature for `minibox-core` (see `Cargo.toml`), the re-exports
//! are always available.
pub use minibox_core::adapters::mocks::*;
