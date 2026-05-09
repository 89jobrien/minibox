//! Linux namespace exec adapter.
//!
//! Joins a running container's namespaces via `/proc/{pid}/ns/*` + `setns(2)`,
//! then forks a child process that executes the requested command inside the
//! container's isolation boundary.

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use base64::Engine as _;
use minibox_core::as_any;
use minibox_core::domain::{ContainerId, DynExecRuntime, ExecHandle, ExecRuntime, ExecSpec};
use minibox_core::protocol::{DaemonResponse, OutputStreamKind};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

/// Send a [`DaemonResponse`] from a blocking context using the current Tokio handle.
/// Logs a warning if the receiver has been dropped.
fn send_blocking(tx: &mpsc::Sender<DaemonResponse>, msg: DaemonResponse) {
    let rt = tokio::runtime::Handle::current();
    if rt.block_on(tx.send(msg)).is_err() {
        warn!("exec: client disconnected before response could be sent");
    }
}

use crate::container_state::StateHandle;

/// Full exec configuration including async channels for stdin/resize relay.
///
/// Lives in `minibox` (infrastructure adapter layer), not `minibox-core` (domain).
/// Channels are infrastructure concerns — the pure domain spec is [`ExecSpec`].
#[derive(Debug)]
pub struct ExecConfig {
    pub spec: ExecSpec,
    /// Stdin bytes channel (handler → exec adapter). `None` = no stdin relay.
    pub stdin_tx: Option<mpsc::Sender<Vec<u8>>>,
    /// PTY resize events (handler → exec adapter). `None` = no resize relay.
    pub resize_rx: Option<mpsc::Receiver<(u16, u16)>>,
}

pub struct NativeExecRuntime {
    state: StateHandle,
}

impl NativeExecRuntime {
    pub fn new(state: StateHandle) -> Self {
        Self { state }
    }
}

as_any!(NativeExecRuntime);

#[async_trait]
impl ExecRuntime for NativeExecRuntime {
    async fn run_in_container(
        &self,
        container_id: &ContainerId,
        spec: ExecSpec,
        tx: mpsc::Sender<DaemonResponse>,
    ) -> Result<ExecHandle> {
        validate_exec_spec(&spec)?;
        let id = container_id.as_str().to_string();
        let pid = self
            .state
            .get_container_pid(&id)
            .await
            .with_context(|| format!("container {id} not found or not running"))?;

        let exec_id = Uuid::new_v4().simple().to_string();

        let cmd_for_log = spec.cmd.clone();
        // Construct an ExecConfig (infra layer) from the pure ExecSpec.
        // Channel fields start as None at the trait boundary; Task 4 will wire
        // stdin_tx/resize_rx by calling the adapter directly when channels are needed.
        let config = ExecConfig {
            spec,
            stdin_tx: None,
            resize_rx: None,
        };
        let exec_id_for_task = exec_id.clone();
        let exec_id_for_warn = exec_id.clone();

        info!(
            container_id = %id,
            exec_id = %exec_id,
            cmd = ?cmd_for_log,
            "exec: process started"
        );

        let handle = tokio::task::spawn_blocking(move || {
            run_exec_blocking(pid, &exec_id_for_task, config, tx);
        });
        tokio::spawn(async move {
            if let Err(e) = handle.await {
                warn!(exec_id = %exec_id_for_warn, error = ?e, "exec: blocking task panicked");
            }
        });

        Ok(ExecHandle { id: exec_id })
    }
}

/// Dispatch to PTY or pipe execution based on `config.spec.tty`.
fn run_exec_blocking(
    container_pid: u32,
    exec_id: &str,
    config: ExecConfig,
    tx: mpsc::Sender<DaemonResponse>,
) {
    if config.spec.tty {
        run_pty_exec(container_pid, exec_id, config, tx);
    } else {
        run_pipe_exec_command(container_pid, exec_id, config, tx);
    }
}

/// Build an `nsenter` [`std::process::Command`] that joins the namespaces of
/// `container_pid` and runs `cmd` with `env`.
///
/// Uses `nsenter(1)` (kernel >= 3.8, util-linux) so the caller never invokes
/// `libc::fork()` while Tokio threads are live.  `std::process::Command` uses
/// `posix_spawn(3)` internally, which is safe in a multi-threaded process.
///
/// Namespace flags: `--mount`, `--pid`, `--net`, `--uts`, `--ipc`.
pub(crate) fn build_nsenter_command(
    container_pid: u32,
    cmd: &[String],
    env: &[String],
    working_dir: Option<&Path>,
) -> std::process::Command {
    let mut c = std::process::Command::new("nsenter");
    c.args([
        "--target",
        &container_pid.to_string(),
        "--mount",
        "--pid",
        "--net",
        "--uts",
        "--ipc",
    ]);
    if let Some(working_dir) = working_dir {
        c.arg("--wd").arg(working_dir);
    }
    c.arg("--");
    if let Some((prog, rest)) = cmd.split_first() {
        c.arg(prog);
        c.args(rest);
    }
    // Clear inherited environment; apply only the container's env vars.
    c.env_clear();
    for kv in env {
        if let Some((k, v)) = kv.split_once('=') {
            c.env(k, v);
        }
    }
    c
}

fn validate_exec_spec(spec: &ExecSpec) -> Result<()> {
    let Some(program) = spec.cmd.first() else {
        return Err(anyhow!("exec command must not be empty"));
    };
    if program.is_empty() {
        return Err(anyhow!("exec command program must not be empty"));
    }
    for part in &spec.cmd {
        if part.contains('\0') {
            return Err(anyhow!("exec command arguments must not contain NUL bytes"));
        }
    }
    for kv in &spec.env {
        let Some((key, value)) = kv.split_once('=') else {
            return Err(anyhow!("exec env entries must use KEY=VALUE format"));
        };
        if key.is_empty() {
            return Err(anyhow!("exec env key must not be empty"));
        }
        if key.contains('\0') || value.contains('\0') {
            return Err(anyhow!("exec env entries must not contain NUL bytes"));
        }
    }
    if let Some(working_dir) = &spec.working_dir {
        let Some(working_dir) = working_dir.to_str() else {
            return Err(anyhow!("exec working directory must be valid UTF-8"));
        };
        if working_dir.is_empty() {
            return Err(anyhow!("exec working directory must not be empty"));
        }
        if working_dir.contains('\0') {
            return Err(anyhow!("exec working directory must not contain NUL bytes"));
        }
    }
    Ok(())
}

/// Execute a command inside a container using `nsenter(1)` + `std::process::Command`
/// (tty=false, pipe mode).
///
/// Replaces the `libc::fork()` path (`run_pipe_exec`) to eliminate POSIX UB
/// when forking inside a multi-threaded Tokio runtime.  `std::process::Command`
/// uses `posix_spawn(3)` which is safe to call from any thread.
fn run_pipe_exec_command(
    container_pid: u32,
    exec_id: &str,
    config: ExecConfig,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let mut child = match build_nsenter_command(
        container_pid,
        &config.spec.cmd,
        &config.spec.env,
        config.spec.working_dir.as_deref(),
    )
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            send_blocking(
                &tx,
                DaemonResponse::Error {
                    message: format!("exec: nsenter spawn failed: {e}"),
                },
            );
            return;
        }
    };

    // Stream stdout and stderr pipes to the channel.
    use std::os::fd::IntoRawFd;
    if let Some(stdout) = child.stdout.take() {
        stream_fd_to_channel(stdout.into_raw_fd(), OutputStreamKind::Stdout, &tx);
    }
    if let Some(stderr) = child.stderr.take() {
        stream_fd_to_channel(stderr.into_raw_fd(), OutputStreamKind::Stderr, &tx);
    }

    let exit_code = match child.wait() {
        Ok(status) => status.code().unwrap_or(-1),
        Err(e) => {
            warn!(exec_id = %exec_id, error = %e, "exec: wait failed");
            -1
        }
    };

    send_blocking(&tx, DaemonResponse::ContainerStopped { exit_code });
    info!(exec_id = %exec_id, exit_code = exit_code, "exec: process exited");
}

/// Execute a command inside a container using a PTY (tty=true).
///
/// Allocates a master/slave PTY pair via `openpty(3)` in the parent, then spawns
/// `nsenter(1)` (util-linux) with the slave wired as its stdin/stdout/stderr.
/// `std::process::Command` uses `posix_spawn(3)` internally — no `fork(2)` is
/// called in this process, eliminating POSIX UB in a multi-threaded context.
///
/// The parent reads PTY output from the master fd and forwards resize events
/// via `TIOCSWINSZ`.
fn run_pty_exec(
    container_pid: u32,
    exec_id: &str,
    config: ExecConfig,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use std::os::fd::IntoRawFd as _;

    // Allocate PTY in the parent. The slave is passed to nsenter as
    // stdin/stdout/stderr; the master stays with us for streaming and resize.
    let pty = match nix::pty::openpty(None, None) {
        Ok(p) => p,
        Err(e) => {
            send_blocking(
                &tx,
                DaemonResponse::Error {
                    message: format!("exec: openpty failed: {e}"),
                },
            );
            return;
        }
    };
    // SAFETY: into_raw_fd() surrenders OwnedFd ownership to a raw int.
    // Both fds are closed explicitly; no other path touches them after this.
    let master_fd = pty.master.into_raw_fd();
    let slave_fd = pty.slave.into_raw_fd();

    // Helper: dup slave_fd so each Stdio object owns its own fd.
    let dup_slave = || -> std::process::Stdio {
        use std::os::fd::FromRawFd;
        // SAFETY: dup() creates a new fd referencing the same file description.
        // The resulting fd is immediately given to Stdio which closes it on drop.
        let duped = unsafe { libc::dup(slave_fd) };
        if duped < 0 {
            std::process::Stdio::null()
        } else {
            unsafe { std::process::Stdio::from_raw_fd(duped) }
        }
    };

    let mut cmd = build_nsenter_command(
        container_pid,
        &config.spec.cmd,
        &config.spec.env,
        config.spec.working_dir.as_deref(),
    );
    cmd.stdin(dup_slave())
        .stdout(dup_slave())
        .stderr(dup_slave());

    // SAFETY: close the original slave fd after spawning — nsenter's duped
    // copies keep the slave alive in the child; we no longer need it here.
    let mut child = match cmd.spawn() {
        Ok(c) => {
            unsafe { libc::close(slave_fd) };
            c
        }
        Err(e) => {
            unsafe { libc::close(slave_fd) };
            unsafe { libc::close(master_fd) };
            send_blocking(
                &tx,
                DaemonResponse::Error {
                    message: format!("exec: nsenter pty spawn failed: {e}"),
                },
            );
            return;
        }
    };

    // Resize relay thread — forwards TIOCSWINSZ to the PTY master fd.
    // Uses blocking_recv() directly; no Tokio runtime needed on this thread.
    if let Some(mut resize_rx) = config.resize_rx {
        let mfd = master_fd;
        std::thread::spawn(move || {
            while let Some((cols, rows)) = resize_rx.blocking_recv() {
                let ws = libc::winsize {
                    ws_col: cols,
                    ws_row: rows,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                };
                // SAFETY: mfd is valid until master is closed after wait below.
                unsafe { libc::ioctl(mfd, libc::TIOCSWINSZ as _, &ws) };
            }
        });
    }

    // Stream master → ContainerOutput (PTY merges stdout+stderr into master).
    stream_fd_to_channel(master_fd, OutputStreamKind::Stdout, &tx);

    let exit_code = match child.wait() {
        Ok(status) => status.code().unwrap_or(-1),
        Err(e) => {
            warn!(exec_id = %exec_id, error = %e, "exec: pty wait failed");
            -1
        }
    };

    send_blocking(&tx, DaemonResponse::ContainerStopped { exit_code });
    info!(exec_id = %exec_id, exit_code, "exec: pty process exited");
}

fn stream_fd_to_channel(fd: i32, stream: OutputStreamKind, tx: &mpsc::Sender<DaemonResponse>) {
    use std::io::{ErrorKind, Read};
    use std::os::fd::FromRawFd;
    // SAFETY: fd is the read end of a pipe or PTY master created above; caller transfers ownership.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut buf = [0u8; 4096];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                send_blocking(
                    tx,
                    DaemonResponse::ContainerOutput {
                        stream: stream.clone(),
                        data,
                    },
                );
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => {
                warn!(error = %e, "exec: stream read error");
                break;
            }
        }
    }
}

/// Construct a [`DynExecRuntime`] backed by Linux namespace setns.
pub fn native_exec_runtime(state: StateHandle) -> DynExecRuntime {
    Arc::new(NativeExecRuntime::new(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    // minibox-26: pipe exec must use nsenter (Command) not libc::fork
    #[test]
    fn build_nsenter_command_constructs_correct_args() {
        let cmd = build_nsenter_command(
            1234,
            &["/bin/echo".to_string(), "hello".to_string()],
            &["HOME=/root".to_string()],
            None,
        );
        let prog = cmd.get_program().to_string_lossy().into_owned();
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        assert_eq!(prog, "nsenter");
        assert!(args.contains(&"--target".to_string()), "must pass --target");
        assert!(
            args.contains(&"1234".to_string()),
            "must pass container pid"
        );
        assert!(
            args.iter().any(|a| a == "/bin/echo"),
            "must include command"
        );
    }

    /// Regression guard for issue #292: exec must not call fork() inside a Tokio
    /// runtime.  The safe path is `nsenter(1)` via `std::process::Command` (which
    /// uses `posix_spawn(3)`) dispatched from `spawn_blocking`.
    ///
    /// This test asserts that `build_nsenter_command` produces a Command whose
    /// program is "nsenter" (not a direct binary that would require fork) and that
    /// env isolation is applied (env_clear + only specified vars).
    #[test]
    fn build_nsenter_command_env_clear_and_var_injection() {
        let cmd = build_nsenter_command(
            9999,
            &["id".to_string()],
            &["PATH=/usr/bin:/bin".to_string(), "HOME=/root".to_string()],
            None,
        );
        assert_eq!(
            cmd.get_program().to_string_lossy(),
            "nsenter",
            "must use nsenter, not a direct fork path"
        );
        let env_vars: Vec<(String, String)> = cmd
            .get_envs()
            .filter_map(|(k, v)| {
                v.map(|val| {
                    (
                        k.to_string_lossy().into_owned(),
                        val.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect();
        // env_clear() leaves an empty explicit set; only the vars we injected appear.
        assert!(
            env_vars
                .iter()
                .any(|(k, v)| k == "PATH" && v == "/usr/bin:/bin"),
            "PATH must be injected"
        );
        assert!(
            env_vars.iter().any(|(k, v)| k == "HOME" && v == "/root"),
            "HOME must be injected"
        );
    }

    #[test]
    fn exec_config_fields() {
        let cfg = ExecConfig {
            spec: ExecSpec {
                cmd: vec!["echo".to_string(), "hello".to_string()],
                env: vec!["HOME=/root".to_string()],
                working_dir: None,
                tty: false,
            },
            stdin_tx: None,
            resize_rx: None,
        };
        assert_eq!(cfg.spec.cmd[0], "echo");
        assert_eq!(cfg.spec.cmd[1], "hello");
        assert_eq!(cfg.spec.env[0], "HOME=/root");
        assert!(cfg.stdin_tx.is_none());
        assert!(cfg.resize_rx.is_none());
    }

    #[test]
    fn validate_exec_spec_rejects_empty_command() {
        let spec = ExecSpec {
            cmd: vec![],
            env: vec![],
            working_dir: None,
            tty: false,
        };
        assert!(validate_exec_spec(&spec).is_err());
    }

    #[test]
    fn validate_exec_spec_rejects_invalid_env() {
        let spec = ExecSpec {
            cmd: vec!["id".to_string()],
            env: vec!["NO_SEPARATOR".to_string()],
            working_dir: None,
            tty: false,
        };
        assert!(validate_exec_spec(&spec).is_err());
    }

    #[test]
    fn build_nsenter_command_applies_working_dir_inside_namespace() {
        let cmd = build_nsenter_command(
            1234,
            &["pwd".to_string()],
            &[],
            Some(Path::new("/workspace")),
        );
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let wd_pos = args.iter().position(|a| a == "--wd").expect("--wd missing");
        let sep_pos = args.iter().position(|a| a == "--").expect("-- missing");
        assert_eq!(args.get(wd_pos + 1).map(String::as_str), Some("/workspace"));
        assert!(
            wd_pos < sep_pos,
            "--wd must be passed to nsenter before the command separator"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore] // requires nsenter privileges; run in privileged CI only
    fn pty_exec_echo_roundtrip() {
        use tokio::sync::mpsc;

        let rt = tokio::runtime::Runtime::new().unwrap();
        let (tx, mut rx) = mpsc::channel::<DaemonResponse>(32);
        let (_resize_tx, resize_rx) = tokio::sync::mpsc::channel::<(u16, u16)>(8);
        let config = ExecConfig {
            spec: ExecSpec {
                cmd: vec!["/bin/echo".to_string(), "pty-ok".to_string()],
                env: vec![],
                working_dir: None,
                tty: true,
            },
            stdin_tx: None,
            resize_rx: Some(resize_rx),
        };
        // Use the test process's own PID so nsenter --target joins our existing
        // namespaces (effectively a no-op). This is a connectivity smoke test,
        // not a namespace isolation test.
        let our_pid = std::process::id();

        std::thread::spawn(move || {
            rt.block_on(async {
                tokio::task::spawn_blocking(move || {
                    run_exec_blocking(our_pid, "test-pty-1", config, tx);
                })
                .await
                .unwrap();
            });
        });

        let responses: Vec<DaemonResponse> = {
            let rt2 = tokio::runtime::Runtime::new().unwrap();
            rt2.block_on(async {
                let mut out = vec![];
                loop {
                    match tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await {
                        Ok(Some(r)) => {
                            let done = matches!(r, DaemonResponse::ContainerStopped { .. });
                            out.push(r);
                            if done {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                out
            })
        };

        let has_output = responses.iter().any(|r| {
            if let DaemonResponse::ContainerOutput { data, .. } = r {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .unwrap();
                String::from_utf8_lossy(&bytes).contains("pty-ok")
            } else {
                false
            }
        });
        assert!(has_output, "expected pty-ok in output; got: {responses:?}");
        assert!(
            responses
                .iter()
                .any(|r| matches!(r, DaemonResponse::ContainerStopped { exit_code: 0 })),
            "expected ContainerStopped(0)"
        );
    }
}
