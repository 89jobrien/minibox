//! macOS-specific default paths for miniboxd.

use std::path::PathBuf;

/// Base data directory for images and containers.
///
/// Uses `~/Library/Application Support/minibox` via `dirs::data_dir()`,
/// falling back to `/tmp/minibox` if that is unavailable.
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("minibox")
}

/// Runtime directory for sockets and PID files.
pub fn run_dir() -> PathBuf {
    PathBuf::from("/tmp/minibox")
}

/// Unix socket path for the daemon.
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
