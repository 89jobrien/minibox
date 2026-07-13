//! Benchmark fixtures and harness support for the minibox workspace.
//!
//! This crate is a leaf: nothing depends on it, and it is the only place
//! where `test-utils` features of the lib crates are enabled.

pub mod fixtures;

/// Runtime guard for root-required Linux benches.
#[must_use]
pub fn is_root() -> bool {
    #[cfg(unix)]
    {
        nix::unistd::geteuid().is_root()
    }
    #[cfg(not(unix))]
    {
        false
    }
}
