//! `SmolvmProcess` — lifecycle wrapper for a persistent, named `smolvm` machine.
//!
//! Each instance backs one minibox container, 1:1 with a named smolvm machine
//! (`machine create --name <container_id>` + `machine start`). The container's
//! main command runs via `machine exec --name <id> --stream -- <cmd>` as the
//! tracked child, rather than the ephemeral `machine run` used previously —
//! `machine run` tears the VM down the instant the command exits, leaving
//! nothing addressable for a later `mbx exec` or `mbx logs`. A named,
//! `create`+`start` machine stays up after the main command finishes, so
//! `smolvm machine exec --name <id> -- <cmd>` can still reach it (see
//! [`exec_in_machine`]), and `smolvm machine stop`/`delete` give the daemon an
//! explicit teardown path instead of relying on ephemeral auto-cleanup.

use anyhow::{Context, Result, bail};
use std::path::Path;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

/// Locate the `smolvm` binary on PATH.
fn smolvm_bin() -> Result<std::path::PathBuf> {
    which::which("smolvm").context("smolvm not found on PATH")
}

/// A running `smolvm machine exec --stream` subprocess tracking one named,
/// persistent microVM's main container command.
#[derive(Debug)]
pub struct SmolvmProcess {
    /// smolvm machine name — always the minibox container ID.
    name: String,
    child: Child,
}

impl SmolvmProcess {
    /// Create and start a named persistent machine, then launch `command` inside
    /// it as the tracked child via `smolvm machine exec --stream`.
    ///
    /// Using the `smolvm` binary found on PATH.
    pub async fn spawn(
        name: &str,
        image: &str,
        command: &[String],
        env: &[(String, String)],
    ) -> Result<Self> {
        let bin = smolvm_bin()?;
        Self::spawn_with_bin(&bin, name, image, command, env).await
    }

    /// Spawn using an explicit binary path (useful for testing missing-binary path).
    pub async fn spawn_with_bin(
        bin: &Path,
        name: &str,
        image: &str,
        command: &[String],
        env: &[(String, String)],
    ) -> Result<Self> {
        // 1. Create the named, persistent machine (no workload command attached
        //    here — the main command is launched explicitly via `machine exec`
        //    below so its stdout/exit code are directly observable on this host
        //    process, rather than running as an opaque detached workload).
        let mut create = Command::new(bin);
        create
            .arg("machine")
            .arg("create")
            .arg("--name")
            .arg(name)
            .arg("--image")
            .arg(image)
            .arg("--net");
        for (k, v) in env {
            create.arg("--env").arg(format!("{k}={v}"));
        }
        let status = create
            .status()
            .await
            .with_context(|| format!("failed to run `smolvm machine create` for {name}"))?;
        minibox_signpost::event(c"smolvm.machine.create");
        if !status.success() {
            bail!("smolvm machine create failed for {name}");
        }

        // 2. Start it — boots to a bare agent since no workload command was
        //    attached at create time.
        let status = Command::new(bin)
            .args(["machine", "start", "--name", name])
            .status()
            .await
            .with_context(|| format!("failed to run `smolvm machine start` for {name}"))?;
        minibox_signpost::event(c"smolvm.machine.start");
        if !status.success() {
            bail!("smolvm machine start failed for {name}");
        }

        // 3. Launch the container's actual command as the tracked child via a
        //    streamed exec — this is what `wait()`/`collect_stdout()` observe.
        let mut exec = Command::new(bin);
        exec.arg("machine")
            .arg("exec")
            .arg("--name")
            .arg(name)
            .arg("--stream");
        if !command.is_empty() {
            exec.arg("--");
            exec.args(command);
        }
        exec.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());

        let child = exec
            .spawn()
            .with_context(|| format!("failed to spawn `smolvm machine exec` for {name}"))?;

        Ok(Self {
            name: name.to_owned(),
            child,
        })
    }

    /// The smolvm machine name backing this process (== minibox container ID).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Wait for the tracked exec process to exit and return its exit code.
    pub async fn wait(&mut self) -> Result<i32> {
        let status = self
            .child
            .wait()
            .await
            .context("smolvm process wait failed")?;
        use std::os::unix::process::ExitStatusExt;
        if let Some(code) = status.code() {
            Ok(code)
        } else {
            let sig = status.signal().unwrap_or(0);
            tracing::debug!(signal = sig, "smolvm: process killed by signal");
            Ok(-sig)
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
    /// the caller can pass it through the `SpawnResult` `output_reader` channel.
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

    /// Stop the backing microVM via `smolvm machine stop --name <name>`, then
    /// reap the tracked exec-stream child (best-effort) if it's still around.
    ///
    /// Unlike the previous ephemeral `machine run` design, SIGTERM-ing the
    /// tracked child only kills the `machine exec --stream` CLI wrapper, not
    /// the persistent VM it's attached to — the VM must be stopped explicitly.
    pub async fn stop(&mut self) -> Result<()> {
        let bin = smolvm_bin()?;
        let status = Command::new(&bin)
            .args(["machine", "stop", "--name", &self.name])
            .status()
            .await
            .with_context(|| format!("failed to run `smolvm machine stop` for {}", self.name))?;
        if !status.success() {
            tracing::warn!(name = %self.name, "krun: machine stop reported non-zero exit");
        }
        let _ = self.child.kill().await;
        Ok(())
    }

    /// Delete the named machine configuration via `smolvm machine delete -f`.
    ///
    /// Call after [`stop`](Self::stop) to fully release the machine's resources.
    pub async fn delete(&self) -> Result<()> {
        let bin = smolvm_bin()?;
        let status = Command::new(&bin)
            .args(["machine", "delete", "--name", &self.name, "-f"])
            .status()
            .await
            .with_context(|| format!("failed to run `smolvm machine delete` for {}", self.name))?;
        if !status.success() {
            tracing::warn!(name = %self.name, "krun: machine delete reported non-zero exit");
        }
        Ok(())
    }
}

/// Run an ad-hoc command inside an already-running named machine via
/// `smolvm machine exec --name <name> -- <cmd>`, returning the spawned child
/// with piped stdout/stderr for the caller to stream.
///
/// `tty` requests a pseudo-terminal from smolvm itself (`-i -t`); the guest
/// allocates and manages the PTY, so the host side still just reads two
/// regular pipes.
pub fn exec_in_machine(
    name: &str,
    cmd: &[String],
    env: &[(String, String)],
    tty: bool,
) -> Result<Child> {
    let bin = smolvm_bin()?;
    let mut exec = Command::new(&bin);
    exec.arg("machine").arg("exec").arg("--name").arg(name);
    for (k, v) in env {
        exec.arg("--env").arg(format!("{k}={v}"));
    }
    if tty {
        exec.arg("-i").arg("-t");
        exec.stdin(std::process::Stdio::piped());
    } else {
        // Without `-i`, smolvm does not read stdin, but leaving it inherited
        // from the daemon process risks `machine exec` blocking on an fd that
        // never reaches EOF (e.g. a long-lived daemon's own stdin).
        exec.stdin(std::process::Stdio::null());
    }
    exec.arg("--");
    exec.args(cmd);
    exec.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    exec.spawn()
        .with_context(|| format!("failed to spawn `smolvm machine exec` into {name}"))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::default_constructed_unit_structs,
    clippy::redundant_clone,
    clippy::items_after_test_module,
    clippy::doc_markdown,
    clippy::map_unwrap_or
)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_with_nonexistent_binary_returns_err() {
        let result = SmolvmProcess::spawn_with_bin(
            Path::new("/nonexistent/smolvm-binary"),
            "test-machine",
            "alpine",
            &["/bin/true".to_string()],
            &[],
        )
        .await;
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(msg.contains("failed to run"));
    }

    #[test]
    fn exec_in_machine_with_bad_binary_on_path_returns_err() {
        // which::which will fail to find "smolvm" in a PATH that doesn't
        // contain it; simulate the "not found" branch by pointing PATH at an
        // empty temp dir for the duration of this call.
        let tmp = tempfile::tempdir().expect("tempdir");
        let original_path = std::env::var_os("PATH");
        // SAFETY: test-only env mutation, not run concurrently with other
        // PATH-sensitive tests in this crate (no other test here reads PATH).
        unsafe { std::env::set_var("PATH", tmp.path()) };
        let result = exec_in_machine("test-machine", &["echo".to_string()], &[], false);
        // SAFETY: restoring PATH to its pre-test value; same single-threaded
        // test-only mutation guarantee as above.
        if let Some(path) = original_path {
            unsafe { std::env::set_var("PATH", path) };
        } else {
            unsafe { std::env::remove_var("PATH") };
        }
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(msg.contains("smolvm not found on PATH"));
    }
}
