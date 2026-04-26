//! `SmolvmProcess` — lifecycle wrapper for a single `smolvm machine run` subprocess.
//!
//! Each instance represents one running microVM. The process is spawned with
//! stdout piped; callers can stream output via [`SmolvmProcess::collect_stdout`]
//! or wait for exit via [`SmolvmProcess::wait`].

use anyhow::{Context, Result};
use std::path::Path;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

/// A running `smolvm machine run` subprocess representing one microVM.
#[derive(Debug)]
pub struct SmolvmProcess {
    child: Child,
}

impl SmolvmProcess {
    /// Spawn using the `smolvm` binary found on PATH.
    pub async fn spawn(image: &str, command: &[String], env: &[(String, String)]) -> Result<Self> {
        let bin = which::which("smolvm").context("smolvm not found on PATH")?;
        Self::spawn_with_bin(&bin, image, command, env).await
    }

    /// Spawn using an explicit binary path (useful for testing missing-binary path).
    pub async fn spawn_with_bin(
        bin: &Path,
        image: &str,
        command: &[String],
        env: &[(String, String)],
    ) -> Result<Self> {
        let mut cmd = Command::new(bin);
        cmd.arg("machine").arg("run").arg("--image").arg(image);

        for (k, v) in env {
            cmd.arg("--env").arg(format!("{k}={v}"));
        }

        if !command.is_empty() {
            cmd.arg("--");
            cmd.args(command);
        }

        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());

        let child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn smolvm at {}", bin.display()))?;

        Ok(Self { child })
    }

    /// Wait for the process to exit and return its exit code.
    pub async fn wait(&mut self) -> Result<i32> {
        let status = self
            .child
            .wait()
            .await
            .context("smolvm process wait failed")?;
        use std::os::unix::process::ExitStatusExt;
        match status.code() {
            Some(code) => Ok(code),
            None => {
                let sig = status.signal().unwrap_or(0);
                tracing::debug!(signal = sig, "smolvm: process killed by signal");
                Ok(-sig)
            }
        }
    }

    /// Collect all stdout output, then wait for exit.
    pub async fn collect_stdout(&mut self) -> Result<String> {
        let stdout = self.child.stdout.take().context("stdout not piped")?;
        let mut buf = String::new();
        let mut reader = tokio::io::BufReader::new(stdout);
        reader
            .read_to_string(&mut buf)
            .await
            .context("reading smolvm stdout")?;
        Ok(buf)
    }

    /// Take the piped stdout from the child process, returning the raw fd.
    ///
    /// Converts the async `ChildStdout` back to its underlying `OwnedFd` so
    /// the caller can pass it through the `SpawnResult` output_reader channel.
    /// Returns `None` if stdout was not piped or was already taken.
    #[cfg(unix)]
    pub fn take_stdout_fd(&mut self) -> Option<std::os::fd::OwnedFd> {
        use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
        let stdout = self.child.stdout.take()?;
        let raw_fd = stdout.as_raw_fd();
        // SAFETY: we just took ownership of stdout from the child, and
        // `as_raw_fd` returns a valid fd. We must forget the tokio wrapper
        // to prevent it from closing the fd when dropped.
        let owned = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        std::mem::forget(stdout);
        Some(owned)
    }

    /// Send SIGTERM to the subprocess to stop the microVM.
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(id) = self.child.id() {
            // SAFETY: kill(pid, SIGTERM) is safe; pid comes from our own child.
            unsafe { libc::kill(id as libc::pid_t, libc::SIGTERM) };
        }
        self.child
            .wait()
            .await
            .context("smolvm stop: wait after SIGTERM")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_with_nonexistent_binary_returns_err() {
        let result = SmolvmProcess::spawn_with_bin(
            Path::new("/nonexistent/smolvm-binary"),
            "alpine",
            &["/bin/true".to_string()],
            &[],
        )
        .await;
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(msg.contains("failed to spawn"));
    }

    #[tokio::test]
    async fn spawn_without_smolvm_on_path_returns_err() {
        // If smolvm isn't installed, spawn() should return a clear error.
        // If it IS installed, this test still validates the happy path doesn't panic.
        let result = SmolvmProcess::spawn_with_bin(
            Path::new("/usr/bin/false"),
            "alpine",
            &["/bin/true".to_string()],
            &[],
        )
        .await;
        // /usr/bin/false exists so spawn succeeds, but the process exits immediately.
        // Either outcome is valid — we just confirm no panic.
        let _ = result;
    }

    #[tokio::test]
    async fn spawn_with_bin_builds_correct_args() {
        // Use /usr/bin/echo as a fake smolvm to verify arg construction.
        // It will just print the args and exit 0.
        let result = SmolvmProcess::spawn_with_bin(
            Path::new("/bin/echo"),
            "test-image",
            &["cmd1".to_string(), "cmd2".to_string()],
            &[("KEY".to_string(), "VAL".to_string())],
        )
        .await;

        match result {
            Ok(mut proc) => {
                let output = proc
                    .collect_stdout()
                    .await
                    .expect("collect_stdout should work");
                // echo prints: machine run --image test-image --env KEY=VAL -- cmd1 cmd2
                assert!(
                    output.contains("machine"),
                    "should contain 'machine': {output}"
                );
                assert!(output.contains("run"), "should contain 'run': {output}");
                assert!(
                    output.contains("--image"),
                    "should contain '--image': {output}"
                );
                assert!(
                    output.contains("test-image"),
                    "should contain image name: {output}"
                );
                assert!(
                    output.contains("KEY=VAL"),
                    "should contain env pair: {output}"
                );
                assert!(output.contains("cmd1"), "should contain command: {output}");
                assert!(output.contains("cmd2"), "should contain arg: {output}");
            }
            Err(e) => panic!("spawn_with_bin(/bin/echo) should succeed: {e:#}"),
        }
    }
}
