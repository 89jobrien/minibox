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

/// Get the default daemon socket path, respecting MINIBOX_SOCKET_PATH environment variable.
///
/// Falls back to the platform-appropriate default:
/// - macOS: `/tmp/minibox/miniboxd.sock` (no `/run` directory on macOS)
/// - Linux: `/run/minibox/miniboxd.sock`
pub fn default_socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("MINIBOX_SOCKET_PATH") {
        return PathBuf::from(p);
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

    #[test]
    fn test_default_socket_path() {
        let path = default_socket_path();
        assert!(path.as_os_str().len() > 0);
    }

    #[test]
    fn test_default_socket_path_contains_sock() {
        let path = default_socket_path();
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains("miniboxd.sock"),
            "path should reference miniboxd.sock"
        );
    }
}
