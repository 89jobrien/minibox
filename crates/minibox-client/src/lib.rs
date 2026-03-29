//! Client library for communicating with miniboxd daemon.
//!
//! This crate provides a high-level async client for sending requests to the minibox daemon
//! over a Unix domain socket. It abstracts the protocol handling and connection management.
//!
//! # Examples
//!
//! ```ignore
//! use minibox_client::DaemonClient;
//! use linuxbox::protocol::DaemonRequest;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = DaemonClient::new()?;
//!     let request = DaemonRequest::List;
//!     let mut responses = client.call(request).await?;
//!
//!     while let Some(response) = responses.next().await? {
//!         println!("{:?}", response);
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod error;
pub mod socket;

pub use error::{ClientError, Result};
pub use socket::{DaemonClient, DaemonResponseStream};

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

    /// Serialise env-mutation tests so parallel test threads don't race on
    /// process-wide environment variables.
    ///
    /// Rust 2024 edition requires `unsafe` for `set_var`/`remove_var`.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    // -----------------------------------------------------------------------
    // Default path (no env overrides)
    // -----------------------------------------------------------------------

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
            "default socket filename must be miniboxd.sock"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn socket_path_default_macos_is_tmp_minibox() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        // SAFETY: serialised by ENV_MUTEX.
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        let path = default_socket_path();
        assert_eq!(
            path,
            PathBuf::from("/tmp/minibox/miniboxd.sock"),
            "macOS default socket path must be /tmp/minibox/miniboxd.sock"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn socket_path_default_linux_is_run_minibox() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        // SAFETY: serialised by ENV_MUTEX.
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        let path = default_socket_path();
        assert_eq!(
            path,
            PathBuf::from("/run/minibox/miniboxd.sock"),
            "Linux default socket path must be /run/minibox/miniboxd.sock"
        );
    }

    // -----------------------------------------------------------------------
    // MINIBOX_SOCKET_PATH override
    // -----------------------------------------------------------------------

    #[test]
    fn socket_path_env_socket_path_overrides_default() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        // SAFETY: serialised by ENV_MUTEX.
        unsafe {
            std::env::set_var("MINIBOX_SOCKET_PATH", "/custom/path/daemon.sock");
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        let path = default_socket_path();
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
        }
        assert_eq!(
            path,
            PathBuf::from("/custom/path/daemon.sock"),
            "MINIBOX_SOCKET_PATH should override the default completely"
        );
    }

    #[test]
    fn socket_path_env_socket_path_wins_over_run_dir() {
        // Even when MINIBOX_RUN_DIR is also set, SOCKET_PATH takes precedence.
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        // SAFETY: serialised by ENV_MUTEX.
        unsafe {
            std::env::set_var("MINIBOX_SOCKET_PATH", "/explicit/miniboxd.sock");
            std::env::set_var("MINIBOX_RUN_DIR", "/some/run/dir");
        }
        let path = default_socket_path();
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        assert_eq!(
            path,
            PathBuf::from("/explicit/miniboxd.sock"),
            "MINIBOX_SOCKET_PATH must win over MINIBOX_RUN_DIR"
        );
    }

    // -----------------------------------------------------------------------
    // MINIBOX_RUN_DIR override
    // -----------------------------------------------------------------------

    #[test]
    fn socket_path_run_dir_appends_miniboxd_sock() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        // SAFETY: serialised by ENV_MUTEX.
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::set_var("MINIBOX_RUN_DIR", "/var/run/minibox-test");
        }
        let path = default_socket_path();
        unsafe {
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        assert_eq!(
            path,
            PathBuf::from("/var/run/minibox-test/miniboxd.sock"),
            "MINIBOX_RUN_DIR should set the socket directory"
        );
    }

    #[test]
    fn socket_path_run_dir_filename_remains_miniboxd_sock() {
        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
        // SAFETY: serialised by ENV_MUTEX.
        unsafe {
            std::env::remove_var("MINIBOX_SOCKET_PATH");
            std::env::set_var("MINIBOX_RUN_DIR", "/tmp/custom-run");
        }
        let path = default_socket_path();
        unsafe {
            std::env::remove_var("MINIBOX_RUN_DIR");
        }
        assert_eq!(
            path.file_name().expect("path has filename"),
            "miniboxd.sock",
            "filename must always be miniboxd.sock when using MINIBOX_RUN_DIR"
        );
    }
}
