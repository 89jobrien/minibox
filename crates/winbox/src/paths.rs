//! Windows-specific default paths for miniboxd.

use std::path::PathBuf;

/// Base data directory for images and containers.
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("C:\\minibox"))
        .join("minibox")
}

/// Runtime directory for Named Pipe and PID files.
pub fn run_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("C:\\Temp"))
        .join("minibox")
}

/// Named pipe path for the daemon.
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
