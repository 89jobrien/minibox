//! Cross-platform Unix socket listener with peer credential extraction.
//!
//! Provides [`UnixServerListener`] which implements
//! [`minibox::daemon::server::ServerListener`] on both Linux (via `SO_PEERCRED`)
//! and macOS (via `getpeereid(2)`).
//!
//! Peer credential extraction is delegated to
//! [`minibox::daemon::server::get_peer_creds`] — the shared implementation
//! used by both `miniboxd` and `macbox`.

use anyhow::Result;
use minibox::daemon::server::{PeerCreds, ServerListener};
use tokio::net::UnixListener;

/// Wraps a Tokio [`UnixListener`] and implements [`ServerListener`].
///
/// On `accept()`, peer credentials are extracted via platform-specific
/// mechanisms and returned alongside the stream.
pub struct UnixServerListener(pub UnixListener);

impl ServerListener for UnixServerListener {
    type Stream = tokio::net::UnixStream;

    async fn accept(&self) -> Result<(Self::Stream, Option<PeerCreds>)> {
        let (stream, _addr) = self.0.accept().await?;
        use std::os::unix::io::AsRawFd;
        let creds = minibox::daemon::server::get_peer_creds(stream.as_raw_fd());
        Ok((stream, creds))
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::only_used_in_recursion,
    clippy::collapsible_if,
    clippy::used_underscore_items,
    clippy::doc_markdown
)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream as StdUnixStream;

    /// Verify that accepting a connection returns peer credentials.
    #[tokio::test]
    async fn accept_returns_peer_creds() {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let sock_path = dir.path().join("test.sock");
        let raw = UnixListener::bind(&sock_path).expect("bind unix listener");
        let listener = UnixServerListener(raw);

        // Connect from a std UnixStream (spawned in background).
        let path = sock_path.clone();
        let _client = tokio::task::spawn_blocking(move || {
            StdUnixStream::connect(path).expect("connect to test socket")
        });

        let (stream, creds) = listener.accept().await.expect("accept connection");
        drop(stream);

        // On both Linux and macOS, we should get Some creds for a local connection.
        let creds = creds.expect("peer creds should be available for local socket");
        // UID should match our process — we're connecting to ourselves.
        assert_eq!(
            creds.uid,
            nix::unistd::getuid().as_raw(),
            "peer uid should match current process uid"
        );
    }

    /// Verify the listener type implements ServerListener (compile-time check).
    #[tokio::test]
    async fn implements_server_listener_trait() {
        fn _assert_impl<T: ServerListener>() {}
        _assert_impl::<UnixServerListener>();
    }
}
