//! Shared xtask utilities.
//!
// TODO: consolidate all path resolution (workspace root, target dir, binary
// discovery) into tested helpers here. 5 path/binary bugs traced to hard-coded
// env assumptions (CARGO_MANIFEST_DIR, CARGO_TARGET_DIR) that differ between
// local dev, CI, and VM. See mistakes.md "xtask: path/binary resolution errors".

use std::{env, path::PathBuf};

/// Returns the Cargo target directory, respecting `CARGO_TARGET_DIR` if set.
///
/// Falls back to `<workspace-root>/target` when the env var is absent.
pub fn cargo_target_dir() -> PathBuf {
    env::var("CARGO_TARGET_DIR").map_or_else(|_| PathBuf::from("target"), PathBuf::from)
}
