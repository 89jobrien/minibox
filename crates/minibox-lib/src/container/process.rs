//! Container init process: clone, setup, pivot_root, exec.
//!
//! [`spawn_container_process`] forks a child process with the requested Linux
//! namespaces, sets up cgroups and the overlay rootfs, then `exec`s the user
//! command. The parent receives the child's PID.

use crate::container::filesystem::pivot_root_to;
use crate::container::namespace::{NamespaceConfig, clone_with_namespaces};
use crate::error::ProcessError;
use anyhow::Context;
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::execvp;
use std::ffi::CString;
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use tracing::{debug, error, info};

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
}

/// Spawn the container init process.
///
/// 1. Clones a child with the requested namespaces.
/// 2. Child: adds itself to the cgroup, sets hostname, pivots root, closes
///    stray file descriptors, then `exec`s the user command.
/// 3. Parent: returns the child PID.
///
/// Returns the child PID on success.
pub fn spawn_container_process(config: ContainerConfig) -> anyhow::Result<u32> {
    info!(command = %config.command, rootfs = ?config.rootfs, "container: spawning process");

    let ns_config = config.namespace_config.clone();
    let pid = clone_with_namespaces(&ns_config, move || {
        // ----------------------------------------------------------------
        // Everything here runs in the child process.
        // We must not return; we must either exec or call _exit.
        // ----------------------------------------------------------------
        if let Err(e) = child_init(config) {
            error!("container init failed: {:#}", e);
            unsafe { libc::_exit(127) };
        }
        // exec replaces the process image, so we never reach here.
        unsafe { libc::_exit(1) };
    })
    .with_context(|| "failed to spawn container process")?;

    let pid_raw = pid.as_raw() as u32;
    info!(pid = pid_raw, "container: process started");
    Ok(pid_raw)
}

/// Logic executed inside the cloned child process.
fn child_init(config: ContainerConfig) -> anyhow::Result<()> {
    // 1. Set hostname (requires UTS namespace).
    debug!("setting hostname to {:?}", config.hostname);
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

    debug!("execvp {:?} {:?}", cmd, argv);

    execvp(&cmd, &argv).map_err(|source| ProcessError::ExecFailed {
        cmd: config.command.clone(),
        source,
    })?;

    // execvp never returns on success.
    unreachable!()
}

/// Write `0` to `{cgroup_path}/cgroup.procs` to add the calling process.
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

/// Close file descriptors > 2.
///
/// We read `/proc/self/fd` to enumerate open FDs so we don't blindly iterate
/// up to some large limit. Failures are silently ignored -- we are about to
/// exec anyway.
///
/// Entries are collected into a Vec before any `close()` calls to avoid
/// closing the directory iterator's own FD mid-iteration.
fn close_extra_fds() {
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
            "closed extra file descriptors before exec"
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
    debug!("waiting for PID {}", pid);

    match waitpid(nix_pid, None).map_err(|source| ProcessError::WaitFailed { pid, source })? {
        WaitStatus::Exited(_, code) => {
            info!("PID {} exited with code {}", pid, code);
            Ok(code)
        }
        WaitStatus::Signaled(_, sig, _) => {
            info!("PID {} killed by signal {:?}", pid, sig);
            Ok(-(sig as i32))
        }
        other => {
            debug!("unexpected wait status for PID {}: {:?}", pid, other);
            Ok(-1)
        }
    }
}
