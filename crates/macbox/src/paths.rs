//! macOS-specific default paths for miniboxd.
//!
//! These functions return compile-time defaults. In production, `macbox::start()`
//! checks the `MINIBOX_DATA_DIR`, `MINIBOX_RUN_DIR`, and `MINIBOX_SOCKET_PATH`
//! environment variables first and only falls back to these defaults when those
//! variables are unset.

use std::path::PathBuf;

/// Base data directory for images and containers.
///
/// Returns `~/Library/Application Support/minibox` via `dirs::data_dir()`,
/// falling back to `/tmp/minibox` if the platform data directory is
/// unavailable (e.g., in a sandboxed or CI environment).
///
/// Overridden at runtime by `MINIBOX_DATA_DIR`.
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("minibox")
}

/// Runtime directory for the Unix socket and ephemeral container state.
///
/// Returns the fixed path `/tmp/minibox`. On macOS there is no `/run`
/// directory, so `/tmp` is used instead of the Linux convention.
///
/// Overridden at runtime by `MINIBOX_RUN_DIR`.
pub fn run_dir() -> PathBuf {
    PathBuf::from("/tmp/minibox")
}

/// Unix socket path for the daemon.
///
/// Returns `<run_dir>/miniboxd.sock`. The CLI connects to this path to send
/// commands to the daemon.
///
/// Overridden at runtime by `MINIBOX_SOCKET_PATH`.
pub fn socket_path() -> PathBuf {
    run_dir().join("miniboxd.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_dir_is_tmp_minibox() {
        assert_eq!(run_dir(), PathBuf::from("/tmp/minibox"));
    }

    #[test]
    fn socket_under_run_dir() {
        assert!(socket_path().starts_with(run_dir()));
    }

    #[test]
    fn socket_filename() {
        assert_eq!(socket_path().file_name().unwrap(), "miniboxd.sock");
    }

    #[test]
    fn data_dir_ends_minibox() {
        assert!(data_dir().to_string_lossy().contains("minibox"));
    }
}
