//! Linux namespace exec adapter.
//!
//! Joins a running container's namespaces via `/proc/{pid}/ns/*` + `setns(2)`,
//! then forks a child process that executes the requested command inside the
//! container's isolation boundary.

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine as _;
use minibox_core::as_any;
use minibox_core::domain::{ContainerId, DynExecRuntime, ExecHandle, ExecRuntime, ExecSpec};
use minibox_core::protocol::{DaemonResponse, OutputStreamKind};
use std::os::fd::AsRawFd;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::daemonbox_state::StateHandle;

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
        let id = container_id.as_str().to_string();
        let pid = self
            .state
            .get_container_pid(&id)
            .await
            .with_context(|| format!("container {id} not found or not running"))?;

        let exec_id = Uuid::new_v4().simple().to_string()[..16].to_string();

        let cmd_for_log = spec.cmd.clone();
        // Construct an ExecConfig (infra layer) from the pure ExecSpec.
        // Channel fields start as None at the trait boundary; Task 4 will wire
        // stdin_tx/resize_rx by calling the adapter directly when channels are needed.
        let config = ExecConfig {
            spec,
            stdin_tx: None,
            resize_rx: None,
        };
        let exec_id_clone = exec_id.clone();

        tokio::task::spawn_blocking(move || {
            run_exec_blocking(pid, &exec_id_clone, config, tx);
        });

        info!(
            container_id = %id,
            exec_id = %exec_id,
            cmd = ?cmd_for_log,
            "exec: process started"
        );

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
        "--",
    ]);
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
    let send_error = |msg: String| {
        let rt = tokio::runtime::Handle::current();
        let _ = rt.block_on(tx.send(DaemonResponse::Error { message: msg }));
    };

    let mut child = match build_nsenter_command(container_pid, &config.spec.cmd, &config.spec.env)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            send_error(format!("exec: nsenter spawn failed: {e}"));
            return;
        }
    };

    // Stream stdout and stderr pipes to the channel.
    if let Some(stdout) = child.stdout.take() {
        use std::os::fd::IntoRawFd;
        stream_fd_to_channel(stdout.into_raw_fd(), OutputStreamKind::Stdout, &tx);
    }
    if let Some(stderr) = child.stderr.take() {
        use std::os::fd::IntoRawFd;
        stream_fd_to_channel(stderr.into_raw_fd(), OutputStreamKind::Stderr, &tx);
    }

    let exit_code = match child.wait() {
        Ok(status) => status.code().unwrap_or(-1),
        Err(e) => {
            warn!(exec_id = %exec_id, error = %e, "exec: wait failed");
            -1
        }
    };

    let rt = tokio::runtime::Handle::current();
    let _ = rt.block_on(tx.send(DaemonResponse::ContainerStopped { exit_code }));
    info!(exec_id = %exec_id, exit_code = exit_code, "exec: process exited");
}

/// Execute a command inside a container using stdout/stderr pipes (tty=false).
fn run_pipe_exec(
    container_pid: u32,
    exec_id: &str,
    config: ExecConfig,
    tx: mpsc::Sender<DaemonResponse>,
) {
    let send_error = |msg: String| {
        let rt = tokio::runtime::Handle::current();
        let _ = rt.block_on(tx.send(DaemonResponse::Error { message: msg }));
    };

    // Open all namespace fds before forking.
    let ns_base = format!("/proc/{container_pid}/ns");
    let ns_names = ["mnt", "pid", "net", "uts", "ipc"];
    let ns_fds: Vec<std::fs::File> = ns_names
        .iter()
        .filter_map(|ns| {
            let path = format!("{ns_base}/{ns}");
            std::fs::File::open(&path)
                .map_err(|e| warn!(ns = %ns, error = %e, "exec: failed to open ns fd"))
                .ok()
        })
        .collect();

    if ns_fds.len() != ns_names.len() {
        send_error(format!(
            "exec: could not open all namespace fds for pid {container_pid}"
        ));
        return;
    }

    // Create stdout/stderr pipes.
    let mut stdout_pipe = [0i32; 2];
    let mut stderr_pipe = [0i32; 2];
    // SAFETY: pipe(2) is safe to call; we check the return value.
    let ok = unsafe {
        libc::pipe(stdout_pipe.as_mut_ptr()) == 0 && libc::pipe(stderr_pipe.as_mut_ptr()) == 0
    };
    if !ok {
        send_error("exec: pipe creation failed".to_string());
        return;
    }
    let [stdout_r, stdout_w] = stdout_pipe;
    let [stderr_r, stderr_w] = stderr_pipe;

    // SAFETY: We are about to fork. All SAFETY comments below describe which
    // process owns each fd and what invariants are upheld after fork.
    let child_pid = unsafe { libc::fork() };
    match child_pid {
        -1 => {
            send_error("exec: fork failed".to_string());
        }
        0 => {
            // ── Child process ────────────────────────────────────────────
            // Join all container namespaces.
            for f in &ns_fds {
                // SAFETY: f is a valid open File; setns joins its namespace.
                unsafe { libc::setns(f.as_raw_fd(), 0) };
            }

            // Redirect stdout/stderr to write ends of pipes.
            // SAFETY: dup2 duplicates fds into slots 1/2; valid because the
            // pipe fds were created above and are open in this process.
            unsafe {
                libc::dup2(stdout_w, 1);
                libc::dup2(stderr_w, 2);
                libc::close(stdout_r);
                libc::close(stdout_w);
                libc::close(stderr_r);
                libc::close(stderr_w);
            }

            let cmd_cstr = match std::ffi::CString::new(config.spec.cmd[0].clone()) {
                Ok(c) => c,
                Err(_) => unsafe { libc::_exit(127) },
            };
            let args: Vec<std::ffi::CString> = config
                .spec
                .cmd
                .iter()
                .filter_map(|s| std::ffi::CString::new(s.as_str()).ok())
                .collect();
            let envp: Vec<std::ffi::CString> = config
                .spec
                .env
                .iter()
                .filter_map(|s| std::ffi::CString::new(s.as_str()).ok())
                .collect();

            let args_ptrs: Vec<*const libc::c_char> = args
                .iter()
                .map(|s| s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            let envp_ptrs: Vec<*const libc::c_char> = envp
                .iter()
                .map(|s| s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();

            // SAFETY: cmd_cstr, args, envp are valid null-terminated C strings;
            // execve replaces this process image.
            unsafe {
                libc::execve(cmd_cstr.as_ptr(), args_ptrs.as_ptr(), envp_ptrs.as_ptr());
                libc::_exit(127);
            }
        }
        child => {
            // ── Parent process ───────────────────────────────────────────
            // Close write ends so we get EOF when child exits.
            // SAFETY: We own these fds; child has dup'd them into slots 1/2.
            unsafe {
                libc::close(stdout_w);
                libc::close(stderr_w);
            }

            stream_fd_to_channel(stdout_r, OutputStreamKind::Stdout, &tx);
            stream_fd_to_channel(stderr_r, OutputStreamKind::Stderr, &tx);

            let mut status: libc::c_int = 0;
            // SAFETY: child is a valid child PID returned by fork.
            unsafe { libc::waitpid(child, &mut status, 0) };
            let exit_code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                -1
            };

            let rt = tokio::runtime::Handle::current();
            let _ = rt.block_on(tx.send(DaemonResponse::ContainerStopped { exit_code }));

            info!(exec_id = %exec_id, exit_code = exit_code, "exec: process exited");
        }
    }
}

/// Execute a command inside a container using a PTY (tty=true).
///
/// Allocates a master/slave PTY pair via `openpty(3)`, forks, joins the container
/// namespaces in the child, creates a new session with `setsid(2)`, acquires the
/// slave as the controlling terminal with `TIOCSCTTY`, then `dup2`s the slave into
/// stdin/stdout/stderr before `execve`. The parent streams master output to `tx`
/// and relays `TIOCSWINSZ` resize events from `config.resize_rx`.
fn run_pty_exec(
    container_pid: u32,
    exec_id: &str,
    config: ExecConfig,
    tx: mpsc::Sender<DaemonResponse>,
) {
    use std::os::fd::IntoRawFd as _;

    let send_err = |msg: String| {
        let rt = tokio::runtime::Handle::current();
        let _ = rt.block_on(tx.send(DaemonResponse::Error { message: msg }));
    };

    // Open namespace fds.
    let ns_base = format!("/proc/{container_pid}/ns");
    let ns_names = ["mnt", "pid", "net", "uts", "ipc"];
    let ns_fds: Vec<std::fs::File> = ns_names
        .iter()
        .filter_map(|ns| {
            std::fs::File::open(format!("{ns_base}/{ns}"))
                .map_err(|e| warn!(ns = %ns, error = %e, "exec: failed to open ns fd"))
                .ok()
        })
        .collect();
    if ns_fds.len() != ns_names.len() {
        send_err(format!(
            "exec: could not open all ns fds for pid {container_pid}"
        ));
        return;
    }

    // Allocate PTY via nix safe bindings over openpty(3).
    // nix::pty is available under the "term" feature in nix 0.29.
    let pty = match nix::pty::openpty(None, None) {
        Ok(p) => p,
        Err(e) => {
            send_err(format!("exec: openpty failed: {e}"));
            return;
        }
    };
    // Convert to raw fds before the fork so we have plain ints to work with
    // across the fork boundary without risking OwnedFd double-close.
    // SAFETY: into_raw_fd() takes ownership of the fd as a raw int. We are
    // responsible for closing both fds explicitly from this point forward.
    let master_fd = pty.master.into_raw_fd();
    let slave_fd = pty.slave.into_raw_fd();

    // SAFETY: about to fork; all fds are managed explicitly below.
    let child_pid = unsafe { libc::fork() };
    match child_pid {
        -1 => {
            send_err("exec: pty fork failed".to_string());
            // SAFETY: fork failed so only this process exists; close both fds.
            unsafe {
                libc::close(master_fd);
                libc::close(slave_fd);
            }
        }
        0 => {
            // ── Child ──────────────────────────────────────────────────────
            for f in &ns_fds {
                // SAFETY: setns joins namespace; f is a valid open fd.
                unsafe { libc::setns(f.as_raw_fd(), 0) };
            }
            // SAFETY: setsid creates a new session; safe in child after fork.
            unsafe { libc::setsid() };
            // SAFETY: TIOCSCTTY acquires slave as controlling terminal in the
            // new session created by setsid above.
            unsafe { libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0i32) };
            // SAFETY: dup2 duplicates slave fd into stdin/stdout/stderr slots.
            // slave_fd > 2 guard prevents closing a just-dup'd slot.
            unsafe {
                libc::dup2(slave_fd, 0);
                libc::dup2(slave_fd, 1);
                libc::dup2(slave_fd, 2);
                if slave_fd > 2 {
                    libc::close(slave_fd);
                }
                libc::close(master_fd);
            }
            let cmd_cstr = match std::ffi::CString::new(config.spec.cmd[0].clone()) {
                Ok(c) => c,
                Err(_) => unsafe { libc::_exit(127) },
            };
            let args: Vec<std::ffi::CString> = config
                .spec
                .cmd
                .iter()
                .filter_map(|s| std::ffi::CString::new(s.as_str()).ok())
                .collect();
            let envp: Vec<std::ffi::CString> = config
                .spec
                .env
                .iter()
                .filter_map(|s| std::ffi::CString::new(s.as_str()).ok())
                .collect();
            let args_ptrs: Vec<*const libc::c_char> = args
                .iter()
                .map(|s| s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            let envp_ptrs: Vec<*const libc::c_char> = envp
                .iter()
                .map(|s| s.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();
            // SAFETY: execve replaces this process image; all pointers are valid
            // null-terminated C strings alive for the duration of this call.
            unsafe {
                libc::execve(cmd_cstr.as_ptr(), args_ptrs.as_ptr(), envp_ptrs.as_ptr());
                libc::_exit(127);
            }
        }
        child => {
            // ── Parent ─────────────────────────────────────────────────────
            // SAFETY: slave is now owned by child; parent closes its copy.
            unsafe { libc::close(slave_fd) };

            // Resize relay thread — forwards TIOCSWINSZ to the PTY master.
            if let Some(mut resize_rx) = config.resize_rx {
                let mfd = master_fd;
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("resize relay runtime");
                    rt.block_on(async {
                        while let Some((cols, rows)) = resize_rx.recv().await {
                            let ws = libc::winsize {
                                ws_col: cols,
                                ws_row: rows,
                                ws_xpixel: 0,
                                ws_ypixel: 0,
                            };
                            // SAFETY: TIOCSWINSZ ioctl on master_fd updates PTY window
                            // size. mfd is valid until master_fd is closed after waitpid.
                            unsafe { libc::ioctl(mfd, libc::TIOCSWINSZ as _, &ws) };
                        }
                    });
                });
            }

            // Stream master → ContainerOutput (PTY merges stdout+stderr into master).
            stream_fd_to_channel(master_fd, OutputStreamKind::Stdout, &tx);

            let mut status: libc::c_int = 0;
            // SAFETY: child is a valid PID returned by fork above.
            unsafe { libc::waitpid(child, &mut status, 0) };
            let exit_code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                -1
            };
            let rt = tokio::runtime::Handle::current();
            let _ = rt.block_on(tx.send(DaemonResponse::ContainerStopped { exit_code }));
            info!(exec_id = %exec_id, exit_code, "exec: pty process exited");
        }
    }
}

fn stream_fd_to_channel(fd: i32, stream: OutputStreamKind, tx: &mpsc::Sender<DaemonResponse>) {
    use std::io::Read;
    use std::os::fd::FromRawFd;
    // SAFETY: fd is the read end of a pipe created above; parent owns it.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut buf = [0u8; 4096];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let data = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                let rt = tokio::runtime::Handle::current();
                let _ = rt.block_on(tx.send(DaemonResponse::ContainerOutput {
                    stream: stream.clone(),
                    data,
                }));
            }
            Err(_) => break,
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

    #[cfg(target_os = "linux")]
    #[test]
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
