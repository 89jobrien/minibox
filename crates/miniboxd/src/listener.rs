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

/// `AF_VSOCK` listener for the vz adapter's in-VM agent (`miniboxd` itself,
/// started with `MINIBOX_VSOCK_PORT` set). The host connects over
/// `VZVirtioSocketDevice` (see `crates/macbox/src/vz/vsock.rs`); there is no
/// shared filesystem for a Unix socket inside a freshly-booted VZ guest, so
/// vsock is the only transport available until the virtiofs shares are
/// mounted by `/etc/init.d/rcS`.
///
/// `AF_VSOCK` has no `SO_PEERCRED` equivalent, so [`ServerListener::accept`]
/// always returns `None` creds. That is fine: the guest is a single-tenant
/// VM reachable only from the host via Virtualization.framework, which is
/// itself the trust boundary. Callers must pass `require_root_auth = false`
/// to [`minibox::daemon::server::run_server`] for this listener.
#[cfg(target_os = "linux")]
pub struct VsockServerListener {
    fd: tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>,
}

#[cfg(target_os = "linux")]
impl VsockServerListener {
    /// Bind an `AF_VSOCK` listener on `port`, accepting connections from any CID.
    pub fn bind(port: u32) -> Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

        // SAFETY: socket() is called with constant, valid arguments; the
        // return value is checked for the -1 error sentinel before use.
        let raw = unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error()).context("socket(AF_VSOCK)");
        }
        // SAFETY: raw is a just-created, valid, exclusively-owned fd.
        let owned = unsafe { OwnedFd::from_raw_fd(raw) };

        // SAFETY: sockaddr_vm is a plain-data C struct; the zeroed bit
        // pattern is valid and every field is set explicitly below before
        // the struct is read by bind(2).
        let mut addr: libc::sockaddr_vm = unsafe { std::mem::zeroed() };
        addr.svm_family = libc::AF_VSOCK as libc::sa_family_t;
        addr.svm_port = port;
        addr.svm_cid = libc::VMADDR_CID_ANY;

        // SAFETY: owned.as_raw_fd() is a valid AF_VSOCK socket; addr is a
        // fully-initialized sockaddr_vm matching the size passed below.
        let bind_rc = unsafe {
            libc::bind(
                owned.as_raw_fd(),
                (&raw const addr).cast::<libc::sockaddr>(),
                std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
            )
        };
        if bind_rc < 0 {
            return Err(std::io::Error::last_os_error()).context("bind(AF_VSOCK)");
        }

        // SAFETY: owned.as_raw_fd() is the bound socket from above.
        let listen_rc = unsafe { libc::listen(owned.as_raw_fd(), 16) };
        if listen_rc < 0 {
            return Err(std::io::Error::last_os_error()).context("listen(AF_VSOCK)");
        }

        set_nonblocking(owned.as_raw_fd())?;

        Ok(Self {
            fd: tokio::io::unix::AsyncFd::new(owned).context("AsyncFd::new for vsock listener")?,
        })
    }
}

#[cfg(target_os = "linux")]
impl ServerListener for VsockServerListener {
    type Stream = tokio::net::UnixStream;

    async fn accept(&self) -> Result<(Self::Stream, Option<PeerCreds>)> {
        use std::os::fd::{AsRawFd, FromRawFd};
        loop {
            let mut guard = self
                .fd
                .readable()
                .await
                .context("poll vsock listener readable")?;
            // SAFETY: guard.get_ref() is the listening AF_VSOCK socket owned
            // by self.fd, valid for the lifetime of this call.
            let listen_fd = guard.get_ref().as_raw_fd();
            // SAFETY: listen_fd is a valid, bound, listening socket.
            let conn_fd =
                unsafe { libc::accept(listen_fd, std::ptr::null_mut(), std::ptr::null_mut()) };
            if conn_fd < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    guard.clear_ready();
                    continue;
                }
                return Err(err).context("accept(AF_VSOCK)");
            }
            set_nonblocking(conn_fd)?;
            // SAFETY: conn_fd is a freshly-accepted fd this call now owns
            // exclusively; from_raw_fd transfers that ownership to std_stream.
            let std_stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(conn_fd) };
            let stream = tokio::net::UnixStream::from_std(std_stream)
                .context("UnixStream::from_std for vsock connection")?;
            return Ok((stream, None));
        }
    }
}

/// Set `fd` to non-blocking mode via `fcntl`.
#[cfg(target_os = "linux")]
fn set_nonblocking(fd: std::os::fd::RawFd) -> Result<()> {
    // SAFETY: fd is a valid, open socket fd owned by the caller.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error()).context("fcntl(F_GETFL)");
    }
    // SAFETY: fd is valid; flags is the flag set just read from the same fd.
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error()).context("fcntl(F_SETFL, O_NONBLOCK)");
    }
    Ok(())
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
