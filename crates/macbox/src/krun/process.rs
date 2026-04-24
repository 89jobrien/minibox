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
                Ok(-(sig as i32))
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
