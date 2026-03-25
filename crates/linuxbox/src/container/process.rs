//! Container init process: clone, setup, pivot_root, exec.
//!
//! [`spawn_container_process`] forks a child process with the requested Linux
//! namespaces, sets up cgroups and the overlay rootfs, then `exec`s the user
//! command. The parent receives the child's PID.

use crate::container::filesystem::pivot_root_to;
use crate::container::namespace::{NamespaceConfig, clone_with_namespaces};
use crate::error::ProcessError;
use anyhow::Context;
use minibox_core::domain::{HookSpec, SpawnResult};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::execvp;
use std::ffi::CString;
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// All information required to launch a containerised process.
#[derive(Debug, Clone)]
pub struct ContainerConfig {
    /// Path to the overlay merged directory (the container's rootfs).
    pub rootfs: PathBuf,
    /// Executable to run (first element of argv).
    pub command: String,
    /// Arguments (not including the command itself).
    pub args: Vec<String>,
    /// Environment variables in `KEY=VALUE` form.
    pub env: Vec<String>,
    /// Namespace flags to apply.
    pub namespace_config: NamespaceConfig,
    /// The container's cgroup path (used by child to add itself).
    pub cgroup_path: PathBuf,
    /// Hostname to set inside the UTS namespace.
    pub hostname: String,
    /// When `true`, container stdout+stderr are captured via a pipe.
    pub capture_output: bool,
    /// Host-side commands to run before the container process is cloned.
    pub pre_exec_hooks: Vec<HookSpec>,
}

/// Spawn the container init process.
///
/// 1. Clones a child with the requested namespaces.
/// 2. Child: adds itself to the cgroup, sets hostname, pivots root, closes
///    stray file descriptors, then `exec`s the user command.
/// 3. Parent: returns the child PID.
///
/// Returns a [`SpawnResult`] containing the child PID and, when
/// `config.capture_output` is true, the read end of a pipe connected to
/// the container's stdout+stderr.
#[cfg(target_os = "linux")]
pub fn spawn_container_process(config: ContainerConfig) -> anyhow::Result<SpawnResult> {
    use nix::fcntl::OFlag;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    info!(command = %config.command, rootfs = ?config.rootfs, "container: spawning process");

    // Run pre-exec hooks on the host before cloning.
    run_hooks(&config.pre_exec_hooks, &config.rootfs, None)
        .with_context(|| "pre-exec hooks failed")?;

    // Create output pipe before cloning so both parent and child inherit it.
    let (read_fd_raw, write_fd_raw): (RawFd, RawFd) = if config.capture_output {
        let (r, w) = nix::unistd::pipe2(OFlag::O_CLOEXEC).context("creating output pipe")?;
        // Extract raw FDs before moving into the closure (OwnedFd is not Clone).
        let r_raw = r.as_raw_fd();
        let w_raw = w.as_raw_fd();
        // SAFETY: After clone(2) both parent and child share the underlying
        // fd-table entries. Dropping an OwnedFd in the parent would close the
        // fd for both processes. We therefore forget the OwnedFds here and
        // take full manual control of the fd lifetimes: the child closes both
        // ends explicitly, and the parent closes the write end after clone
        // returns and wraps the read end in a new OwnedFd.
        std::mem::forget(r);
        std::mem::forget(w);
        (r_raw, w_raw)
    } else {
        (-1, -1)
    };

    let capture_output = config.capture_output;
    let ns_config = config.namespace_config.clone();
    let pid = clone_with_namespaces(&ns_config, move || {
        // ----------------------------------------------------------------
        // Everything here runs in the child process.
        // We must not return; we must either exec or call _exit.
        // ----------------------------------------------------------------

        // Redirect stdout and stderr to the write end of the pipe.
        if capture_output && write_fd_raw >= 0 {
            // SAFETY: write_fd_raw and read_fd_raw are valid open file
            // descriptors inherited across clone(2). Their OwnedFds were
            // forgotten before the clone call so no other owner will close
            // them. dup2 and close are async-signal-safe syscalls.
            unsafe {
                libc::dup2(write_fd_raw, libc::STDOUT_FILENO);
                libc::dup2(write_fd_raw, libc::STDERR_FILENO);
                // Close the original write_fd slot (now dup'd into fds 1 and 2).
                // O_CLOEXEC on the original would close it at exec anyway,
                // but we close explicitly to release the slot now.
                libc::close(write_fd_raw);
                // Close the read end — the child must not hold it open or the
                // parent's read end will never see EOF.
                libc::close(read_fd_raw);
            }
        }

        if let Err(e) = child_init(config) {
            error!(error = %e, "container: child init failed");
            unsafe { libc::_exit(127) };
        }
        // exec replaces the process image, so we never reach here.
        unsafe { libc::_exit(1) };
    })
    .with_context(|| "failed to spawn container process")?;

    // Parent: close the write end so the read end gets EOF when the child exits.
    if capture_output && write_fd_raw >= 0 {
        unsafe { libc::close(write_fd_raw) };
    }

    let pid_raw = pid.as_raw() as u32;
    info!(pid = pid_raw, "container: process started");

    let output_reader = if capture_output && read_fd_raw >= 0 {
        // SAFETY: we created this fd above and haven't closed/moved it in parent.
        Some(unsafe { OwnedFd::from_raw_fd(read_fd_raw) })
    } else {
        None
    };

    Ok(SpawnResult {
        pid: pid_raw,
        output_reader,
    })
}

/// Non-Linux stub: always errors because namespace containers require Linux.
#[cfg(not(target_os = "linux"))]
pub fn spawn_container_process(_config: ContainerConfig) -> anyhow::Result<SpawnResult> {
    anyhow::bail!("spawn_container_process is only supported on Linux")
}

/// Run a list of host-side lifecycle hooks.
///
/// Each hook is executed with `CONTAINER_ROOTFS` set. If `exit_code` is
/// provided (post-exit context), `EXIT_CODE` is also set.
///
/// Hooks that exceed their timeout are abandoned with a warning rather than
/// killing the overall operation.
pub fn run_hooks(
    hooks: &[HookSpec],
    rootfs: &std::path::Path,
    exit_code: Option<i32>,
) -> anyhow::Result<()> {
    for hook in hooks {
        let timeout = Duration::from_secs(hook.timeout_secs.unwrap_or(30));
        debug!(command = %hook.command, "running lifecycle hook");

        let mut cmd = std::process::Command::new(&hook.command);
        cmd.args(&hook.args).env("CONTAINER_ROOTFS", rootfs);
        if let Some(code) = exit_code {
            cmd.env("EXIT_CODE", code.to_string());
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("lifecycle hook '{}' failed to start", hook.command))?;

        // Poll for completion up to the timeout.
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        warn!(
                            command = %hook.command,
                            code = ?status.code(),
                            "lifecycle hook exited with non-zero status"
                        );
                    }
                    break;
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        warn!(command = %hook.command, "lifecycle hook timed out, abandoning");
                        let _ = child.kill();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    warn!(command = %hook.command, error = %e, "lifecycle hook wait error");
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Initialise the container environment inside the cloned child process.
///
/// Called immediately after `clone(2)` returns in the child. Performs all
/// setup steps before `execvp` replaces the process image:
///
/// 1. Set the UTS hostname (requires `CLONE_NEWUTS`).
/// 2. Add the child to its cgroup by writing `"0"` to `cgroup.procs` (the
///    kernel interprets PID 0 as the calling process).
/// 3. Call [`pivot_root_to`] to switch the root filesystem to the overlay
///    merged directory.
/// 4. Call [`close_extra_fds`] to release any file descriptors > 2 that
///    leaked from the parent across the clone boundary.
/// 5. Build the `argv` vector and call `execvp` to exec the user command.
///
/// On any error the caller is expected to call `libc::_exit(127)` so the
/// process terminates without running Rust destructors.
fn child_init(config: ContainerConfig) -> anyhow::Result<()> {
    // 1. Set hostname (requires UTS namespace).
    debug!(hostname = %config.hostname, "container: setting hostname");
    nix::unistd::sethostname(&config.hostname).map_err(|e| {
        crate::error::NamespaceError::SetHostnameFailed(format!(
            "sethostname({:?}) failed: {e}",
            config.hostname
        ))
    })?;

    // 2. Add ourselves to the cgroup so resource limits apply.
    //    We write PID 0 which the kernel interprets as "current process"
    //    for cgroup.procs.
    add_self_to_cgroup(&config.cgroup_path).with_context(|| "child: add_self_to_cgroup")?;

    // 3. Pivot root to the overlay merged directory.
    pivot_root_to(&config.rootfs).with_context(|| "child: pivot_root")?;

    // 4. Close any file descriptors > 2 (stdin/stdout/stderr) that leaked
    //    from the parent. We do this on a best-effort basis.
    close_extra_fds();

    // 5. Build argv for execvp.
    let cmd = CString::new(config.command.clone()).map_err(|_| {
        ProcessError::SpawnFailed(format!("invalid command string: {}", config.command))
    })?;

    let mut argv: Vec<CString> = Vec::with_capacity(config.args.len() + 1);
    argv.push(cmd.clone());
    for arg in &config.args {
        argv.push(
            CString::new(arg.as_str())
                .map_err(|_| ProcessError::SpawnFailed(format!("invalid argument: {arg}")))?,
        );
    }

    debug!(command = %config.command, "container: execvp");

    execvp(&cmd, &argv).map_err(|source| ProcessError::ExecFailed {
        cmd: config.command.clone(),
        source,
    })?;

    // execvp never returns on success.
    unreachable!()
}

/// Add the calling process to the cgroup at `cgroup_path`.
///
/// Writes `"0\n"` to `cgroup.procs`; the kernel interprets PID 0 as the
/// calling process. This is the correct mechanism to use from inside the
/// child after `clone(2)`, because the child's PID inside its new PID
/// namespace may differ from the PID visible to the parent.
fn add_self_to_cgroup(cgroup_path: &std::path::Path) -> anyhow::Result<()> {
    let procs_file = cgroup_path.join("cgroup.procs");
    std::fs::write(&procs_file, "0\n").map_err(|source| {
        crate::error::CgroupError::AddProcessFailed {
            pid: 0,
            path: procs_file.display().to_string(),
            source,
        }
    })?;
    Ok(())
}

/// Close all file descriptors with index >= 3 (i.e., everything except
/// stdin, stdout, and stderr).
///
/// Uses a two-tier strategy inspired by QEMU's `qemu_close_all_open_fd`:
///
/// 1. **`close_range(3, MAX, 0)` syscall** (kernel 5.9+) — single syscall,
///    no allocation, no `/proc` dependency.
/// 2. **`/proc/self/fd` scan** — fallback for older kernels. Entries are
///    collected into a `Vec` **before** any `close()` calls to avoid closing
///    the `ReadDir` iterator's own FD mid-iteration.
///
/// Failures from individual `close()` calls are silently ignored — the process
/// is about to `exec` and any remaining FDs will be closed by the kernel
/// (for those with `O_CLOEXEC`) or will be safe to leave open temporarily.
fn close_extra_fds() {
    // Fast path: close_range(3, u32::MAX, 0) — available since Linux 5.9.
    // SAFETY: close_range is a pure fd-table operation with no memory side
    // effects; the worst outcome is ENOSYS on older kernels.
    let ret = unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) };
    if ret == 0 {
        debug!("container: closed extra file descriptors via close_range");
        return;
    }

    // Fallback: enumerate /proc/self/fd.
    if let Ok(entries) = std::fs::read_dir("/proc/self/fd") {
        let fds: Vec<RawFd> = entries
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|n| n.parse::<RawFd>().ok())
            .filter(|&fd| fd > 2)
            .collect();
        let count = fds.len();
        for fd in fds {
            unsafe { libc::close(fd) };
        }
        debug!(
            fds_closed = count,
            "container: closed extra file descriptors via /proc/self/fd scan"
        );
    }
}

// ---------------------------------------------------------------------------
// wait_for_exit
// ---------------------------------------------------------------------------

/// Wait for a container process to exit and return its exit code.
///
/// This is a blocking call -- use it from a dedicated thread or a
/// `tokio::task::spawn_blocking` context.
pub fn wait_for_exit(pid: u32) -> anyhow::Result<i32> {
    let nix_pid = nix::unistd::Pid::from_raw(pid as i32);
    debug!(pid = pid, "container: waiting for process exit");

    match waitpid(nix_pid, None).map_err(|source| ProcessError::WaitFailed { pid, source })? {
        WaitStatus::Exited(_, code) => {
            info!(pid = pid, exit_code = code, "container: process exited");
            Ok(code)
        }
        WaitStatus::Signaled(_, sig, _) => {
            info!(pid = pid, signal = ?sig, "container: process killed by signal");
            Ok(-(sig as i32))
        }
        other => {
            debug!(pid = pid, wait_status = ?other, "container: unexpected wait status");
            Ok(-1)
        }
    }
}
