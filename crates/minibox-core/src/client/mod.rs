//! Client library for communicating with miniboxd daemon.
//!
//! Provides a high-level async client for sending requests to the minibox daemon
//! over a Unix domain socket.

pub mod error;
pub mod socket;

pub use error::{ClientError, Result};
pub use socket::{DaemonClient, DaemonResponseStream, DaemonWriter};

use std::path::PathBuf;

/// Get the default daemon socket path, respecting environment variable overrides.
///
/// Resolution order (first set wins):
/// 1. `MINIBOX_SOCKET_PATH` — full path to the Unix socket
/// 2. `MINIBOX_RUN_DIR` — directory; socket is `<dir>/miniboxd.sock`
/// 3. Platform default:
///    - macOS: `/tmp/minibox/miniboxd.sock` (no `/run` directory on macOS)
///    - Linux/other: `/run/minibox/miniboxd.sock`
pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("MINIBOX_SOCKET_PATH") {
        return PathBuf::from(p);
    }
    if let Ok(dir) = std::env::var("MINIBOX_RUN_DIR") {
        return PathBuf::from(dir).join("miniboxd.sock");
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/tmp/minibox/miniboxd.sock")
    }
    #[cfg(not(target_os = "macos"))]
    {
        PathBuf::from("/run/minibox/miniboxd.sock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn socket_path_default_filename_is_miniboxd_sock() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        // SAFETY: serialised by ENV_MUTEX; no other thread mutates the
        // process-wide env while the lock is held.
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        let path = default_socket_path();
        assert_eq!(
            path.file_name().expect("path has filename"),
            "miniboxd.sock",
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn socket_path_default_macos_is_tmp_minibox() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        let path = default_socket_path();
        assert_eq!(path, PathBuf::from("/tmp/minibox/miniboxd.sock"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn socket_path_default_linux_is_run_minibox() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        let path = default_socket_path();
        assert_eq!(path, PathBuf::from("/run/minibox/miniboxd.sock"));
    }

    #[test]
    fn socket_path_env_socket_path_overrides_default() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        unsafe {
            std::env::set_var("MINIBOX_SOCKET_PATH", "/custom/path/daemon.sock");
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        let path = default_socket_path();
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
        }
        assert_eq!(path, PathBuf::from("/custom/path/daemon.sock"));
    }

    #[test]
    fn socket_path_env_socket_path_wins_over_run_dir() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        unsafe {
            std::env::set_var("MINIBOX_SOCKET_PATH", "/explicit/miniboxd.sock");
            std::env::set_var("MINIBOX_RUN_DIR", "/some/run/dir");
        }
        let path = default_socket_path();
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        assert_eq!(path, PathBuf::from("/explicit/miniboxd.sock"));
    }

    #[test]
    fn socket_path_run_dir_appends_miniboxd_sock() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::set_var("MINIBOX_RUN_DIR", "/var/run/minibox-test");
        }
        let path = default_socket_path();
        unsafe {
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        assert_eq!(path, PathBuf::from("/var/run/minibox-test/miniboxd.sock"));
    }
}
