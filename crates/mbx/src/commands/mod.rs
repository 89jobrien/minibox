//! CLI command modules.
//!
//! Each module implements a single subcommand using the [`minibox_core::client`] library
//! to communicate with the daemon. The [`DaemonClient`] abstraction handles socket
//! connection and protocol formatting.

pub mod diagnose;
pub mod events;
pub mod exec;
pub mod load;
pub mod logs;
pub mod pause;
pub mod prune;
pub mod ps;
pub mod pull;
pub mod resume;
pub mod rm;
pub mod rmi;
pub mod run;
pub mod sandbox;
pub mod snapshot;
pub mod stop;
pub mod update;
pub mod upgrade;

/// Shared async test helpers for command unit tests.
///
/// Each command module imports these via `use super::test_helpers::*` inside
/// its `#[cfg(test)] mod tests` block — avoiding the `serve_once` copy-paste
/// that was previously duplicated across every module.
#[cfg(test)]
pub mod test_helpers {
    use minibox_core::protocol::DaemonResponse;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    /// Bind a Unix socket at `socket_path`, accept one connection, read one
    /// newline-delimited request, write back `response` as a JSON line, then
    /// close.  Intended to be spawned as a background task before the test
    /// calls `execute(...)`.
    pub async fn serve_once(socket_path: &Path, response: DaemonResponse) {
        let listener = UnixListener::bind(socket_path).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let mut resp = serde_json::to_string(&response).unwrap();
        resp.push('\n');
        write_half.write_all(resp.as_bytes()).await.unwrap();
        write_half.flush().await.unwrap();
    }

    /// Create a temp dir, bind `serve_once` as a background task, sleep 10 ms
    /// so the listener is ready, and return `(TempDir, socket_path)`.
    ///
    /// The `TempDir` must be kept alive for the duration of the test.
    pub async fn setup(response: DaemonResponse) -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let sp = socket_path.clone();
        tokio::spawn(async move { serve_once(&sp, response).await });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        (tmp, socket_path)
    }

    /// Bind a Unix socket at `socket_path`, accept one connection, read one
    /// newline-delimited request, write back all `responses` as JSON lines in
    /// sequence, then close.
    pub async fn serve_multi(socket_path: &Path, responses: Vec<DaemonResponse>) {
        let listener = UnixListener::bind(socket_path).unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        for response in responses {
            let mut resp = serde_json::to_string(&response).unwrap();
            resp.push('\n');
            write_half.write_all(resp.as_bytes()).await.unwrap();
        }
        write_half.flush().await.unwrap();
    }

    /// Create a temp dir, bind `serve_multi` as a background task, sleep 10 ms
    /// so the listener is ready, and return `(TempDir, socket_path)`.
    ///
    /// The `TempDir` must be kept alive for the duration of the test.
    pub async fn setup_multi(responses: Vec<DaemonResponse>) -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let sp = socket_path.clone();
        tokio::spawn(async move { serve_multi(&sp, responses).await });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        (tmp, socket_path)
    }
}
