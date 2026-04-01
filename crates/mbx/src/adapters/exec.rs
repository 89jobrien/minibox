//! Linux namespace exec adapter.
//!
//! Joins a running container's namespaces via `/proc/{pid}/ns/*` + `setns(2)`,
//! then forks a child process that executes the requested command inside the
//! container's isolation boundary.

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine as _;
use minibox_core::as_any;
use minibox_core::domain::{
    AsAny, ContainerId, DynExecRuntime, ExecConfig, ExecHandle, ExecRuntime,
};
use minibox_core::protocol::{DaemonResponse, OutputStreamKind};
use std::os::fd::AsRawFd;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::daemonbox_state::StateHandle;

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
        config: &ExecConfig,
        tx: mpsc::Sender<DaemonResponse>,
    ) -> Result<ExecHandle> {
        let id = container_id.as_str().to_string();
        let pid = self
            .state
            .get_container_pid(&id)
            .await
            .with_context(|| format!("container {id} not found or not running"))?;

        let exec_id = Uuid::new_v4().simple().to_string()[..16].to_string();
        let config = config.clone();
        let exec_id_clone = exec_id.clone();

        tokio::task::spawn_blocking(move || {
            run_exec_blocking(pid, &exec_id_clone, &config, tx);
        });

        info!(
            container_id = %id,
            exec_id = %exec_id,
            cmd = ?config.cmd,
            "exec: process started"
        );

        Ok(ExecHandle { id: exec_id })
    }
}

fn run_exec_blocking(
    container_pid: u32,
    exec_id: &str,
    config: &ExecConfig,
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
            send_error(format!("exec: fork failed"));
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

            let cmd_cstr = match std::ffi::CString::new(config.cmd[0].clone()) {
                Ok(c) => c,
                Err(_) => unsafe { libc::_exit(127) },
            };
            let args: Vec<std::ffi::CString> = config
                .cmd
                .iter()
                .filter_map(|s| std::ffi::CString::new(s.as_str()).ok())
                .collect();
            let envp: Vec<std::ffi::CString> = config
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

    #[test]
    fn exec_config_fields() {
        let cfg = ExecConfig {
            cmd: vec!["echo".to_string(), "hello".to_string()],
            env: vec!["HOME=/root".to_string()],
            working_dir: None,
            tty: false,
        };
        assert_eq!(cfg.cmd[0], "echo");
        assert_eq!(cfg.cmd[1], "hello");
        assert_eq!(cfg.env[0], "HOME=/root");
    }
}
