//! CLI command modules.
//!
//! Each module implements a single subcommand.  Most commands send one
//! [`DaemonRequest`] over the Unix socket and read a single [`DaemonResponse`]
//! back via the shared [`send_request`] helper.  The `run` module is the
//! exception: it opens its own streaming connection to receive a sequence of
//! `ContainerOutput` / `ContainerStopped` messages until the container exits.

pub mod ps;
pub mod pull;
pub mod rm;
pub mod run;
pub mod stop;

use anyhow::{Context, Result};
use linuxbox::protocol::{DaemonRequest, DaemonResponse};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;
use tracing::debug;

/// Resolve the daemon socket path (or Named Pipe name on Windows).
///
/// Checks platform-specific env vars first, then falls back to a
/// platform-appropriate default.
///
/// | Platform | Env var               | Default                          |
/// |----------|-----------------------|----------------------------------|
/// | Linux    | `MINIBOX_SOCKET_PATH` | `/run/minibox/miniboxd.sock`     |
/// | macOS    | `MINIBOX_SOCKET_PATH` | `/tmp/minibox/miniboxd.sock`     |
/// | Windows  | `MINIBOX_PIPE_NAME`   | `\\.\pipe\miniboxd`              |
fn socket_path() -> String {
    #[cfg(target_os = "linux")]
    {
        std::env::var("MINIBOX_SOCKET_PATH")
            .unwrap_or_else(|_| "/run/minibox/miniboxd.sock".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("MINIBOX_SOCKET_PATH")
            .unwrap_or_else(|_| "/tmp/minibox/miniboxd.sock".to_string())
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("MINIBOX_PIPE_NAME").unwrap_or_else(|_| r"\\.\pipe\miniboxd".to_string())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        std::env::var("MINIBOX_SOCKET_PATH")
            .unwrap_or_else(|_| "/run/minibox/miniboxd.sock".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Env var tests mutate process-global state.  Serialise them with a mutex
    // so parallel test threads cannot observe each other's writes.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn socket_path_defaults_when_env_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: single-threaded via ENV_LOCK; no other thread reads this var
        unsafe { std::env::remove_var("MINIBOX_SOCKET_PATH") };
        #[cfg(not(target_os = "windows"))]
        unsafe {
            std::env::remove_var("MINIBOX_PIPE_NAME")
        };
        let path = socket_path();
        // Default varies by platform; verify it contains "minibox" and ends with
        // a known suffix (sock on Unix, pipe prefix on Windows).
        #[cfg(target_os = "linux")]
        assert_eq!(path, "/run/minibox/miniboxd.sock");
        #[cfg(target_os = "macos")]
        assert_eq!(path, "/tmp/minibox/miniboxd.sock");
        #[cfg(target_os = "windows")]
        assert!(path.starts_with(r"\\.\pipe\"));
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        assert!(path.contains("minibox"));
    }

    #[test]
    fn socket_path_returns_env_var_when_set() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: single-threaded via ENV_LOCK; no other thread reads this var
        #[cfg(not(target_os = "windows"))]
        {
            unsafe { std::env::set_var("MINIBOX_SOCKET_PATH", "/tmp/test-minibox.sock") };
            let result = socket_path();
            unsafe { std::env::remove_var("MINIBOX_SOCKET_PATH") };
            assert_eq!(result, "/tmp/test-minibox.sock");
        }
        #[cfg(target_os = "windows")]
        {
            unsafe { std::env::set_var("MINIBOX_PIPE_NAME", r"\\.\pipe\miniboxd-test") };
            let result = socket_path();
            unsafe { std::env::remove_var("MINIBOX_PIPE_NAME") };
            assert_eq!(result, r"\\.\pipe\miniboxd-test");
        }
    }
}

/// Open a connection to the daemon, send one request, and return the response.
///
/// The protocol is a single JSON line → single JSON line.
pub async fn send_request(request: &DaemonRequest) -> Result<DaemonResponse> {
    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .await
        .with_context(|| format!("connecting to daemon at {path}"))?;

    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);

    // Serialise request as a single JSON line.
    let mut payload = serde_json::to_string(request).context("serialising request")?;
    payload.push('\n');

    debug!("sending: {}", payload.trim());

    writer
        .write_all(payload.as_bytes())
        .await
        .context("writing request")?;
    writer.flush().await.context("flushing request")?;

    // Read one response line.
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .context("reading response")?;

    debug!("received: {}", line.trim());

    let response: DaemonResponse = serde_json::from_str(line.trim()).context("parsing response")?;
    Ok(response)
}
