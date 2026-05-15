//! Shared xtask utilities.

use std::{env, path::PathBuf};

/// Returns the Cargo target directory, respecting `CARGO_TARGET_DIR` if set.
///
/// Falls back to `<workspace-root>/target` when the env var is absent.
pub fn cargo_target_dir() -> PathBuf {
    env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"))
}
