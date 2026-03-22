//! Windows-specific default paths for miniboxd.
//!
//! These functions return compile-time defaults. In a full Phase 2
//! implementation, `winbox::start()` would check environment variables
//! (`MINIBOX_DATA_DIR`, `MINIBOX_RUN_DIR`, `MINIBOX_SOCKET_PATH`) before
//! falling back to these defaults, mirroring the pattern used by `macbox`.

use std::path::PathBuf;

/// Base data directory for images and containers.
///
/// Returns `%APPDATA%\minibox` via `dirs::data_dir()` (typically
/// `C:\Users\<user>\AppData\Roaming\minibox`), falling back to
/// `C:\minibox` if the platform data directory is unavailable.
///
/// Would be overridden at runtime by `MINIBOX_DATA_DIR` in a full implementation.
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("C:\\minibox"))
        .join("minibox")
}

/// Runtime directory for ephemeral state files.
///
/// Returns a subdirectory of the Windows local cache directory
/// (`%LOCALAPPDATA%\minibox`, typically
/// `C:\Users\<user>\AppData\Local\minibox`), falling back to `C:\Temp\minibox`.
/// Unlike the Unix daemon, the Windows daemon uses a Named Pipe (not a socket
/// file), so this directory primarily holds PID files and ephemeral metadata.
///
/// Would be overridden at runtime by `MINIBOX_RUN_DIR` in a full implementation.
pub fn run_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("C:\\Temp"))
        .join("minibox")
}

/// Named Pipe path for the Windows daemon.
///
/// Returns the fixed UNC path `\\.\pipe\miniboxd`. Windows Named Pipes use
/// this prefix by convention. The CLI would connect to this path in place of
/// the Unix socket used on Linux and macOS.
///
/// This path is not currently used — `winbox::start()` is a Phase 1 stub.
pub fn pipe_name() -> String {
    r"\\.\pipe\miniboxd".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_has_prefix() {
        assert!(pipe_name().starts_with(r"\\.\pipe\"));
    }

    #[test]
    fn data_dir_ends_minibox() {
        assert!(data_dir().to_string_lossy().contains("minibox"));
    }
}
