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
use nix::unistd::execve;
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
    /// Bind mounts applied inside the container's mount namespace before pivot_root.
    pub mounts: Vec<minibox_core::domain::BindMount>,
    /// If `true`, call `capset(2)` with all capabilities set before `execvp`.
    pub privileged: bool,
    /// Optional PTY configuration for interactive containers.
    ///
    /// When `Some`, the daemon should attempt to allocate a PTY pair via the
    /// [`PtyAllocator`] port before cloning the container process.  The actual
    /// PTY fork/exec wiring is deferred to Linux-specific adapters.
    pub pty: Option<minibox_core::domain::PtyConfig>,
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

/// Grant the container process a curated privileged capability set.
///
/// Uses `capset(2)` with `LINUX_CAPABILITY_VERSION_3` to set a wide but
/// deliberately bounded set of capabilities in `permitted`, `effective`, and
/// `inheritable`. Called inside the child process before `execvp` when
/// `config.privileged` is true.
///
/// # Excluded capabilities (host-escape tier)
///
/// The following capabilities are **never** granted, even in privileged mode,
/// because they provide direct paths to compromising the host kernel or its
/// security enforcement and have no legitimate container use:
///
/// | Capability        | Bit | Reason                                    |
/// |-------------------|-----|-------------------------------------------|
/// | CAP_SYS_MODULE    |  16 | Load/unload kernel modules                |
/// | CAP_SYS_BOOT      |  22 | Reboot, shutdown, or kexec the host       |
/// | CAP_MAC_OVERRIDE  |  32 | Bypass MAC (SELinux/AppArmor) enforcement |
/// | CAP_MAC_ADMIN     |  33 | Modify/load MAC policies on the host      |
///
/// # Safety
///
/// This function uses `libc::syscall(SYS_capset)`. We are in the child
/// process (single-threaded after clone). The repr(C) structs match the
/// kernel's `linux_capability_version_3` ABI exactly.
#[cfg(target_os = "linux")]
fn apply_privileged_capabilities() -> anyhow::Result<()> {
    // LINUX_CAPABILITY_VERSION_3: supports 64-bit capability sets as two
    // 32-bit words (low bits 0-31, high bits 32-40).
    const LINUX_CAPABILITY_VERSION_3: u32 = 0x20080522;
    // Low caps (0-31) minus CAP_SYS_MODULE (16) and CAP_SYS_BOOT (22).
    const CAP_PRIVILEGED_LOW: u32 = !(1_u32 << 16) & !(1_u32 << 22);
    // High caps (32-40) minus CAP_MAC_OVERRIDE (bit 0) and CAP_MAC_ADMIN (bit 1).
    const CAP_PRIVILEGED_HIGH: u32 = 0x0000_01FF & !(1 << 0) & !(1 << 1);

    #[repr(C)]
    struct CapHeader {
        version: u32,
        pid: i32,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CapData {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }

    // SAFETY: capset(2) is a pure fd-table-independent syscall. We are in a
    // freshly cloned child process. The CapHeader and CapData structs are
    // #[repr(C)] with the exact layout the kernel expects for version 3.
    unsafe {
        let mut header = CapHeader {
            version: LINUX_CAPABILITY_VERSION_3,
            pid: 0, // 0 = calling process
        };
        let full = CapData {
            effective: CAP_PRIVILEGED_LOW,
            permitted: CAP_PRIVILEGED_LOW,
            inheritable: CAP_PRIVILEGED_LOW,
        };
        let full_high = CapData {
            effective: CAP_PRIVILEGED_HIGH,
            permitted: CAP_PRIVILEGED_HIGH,
            inheritable: CAP_PRIVILEGED_HIGH,
        };
        let mut data = [full, full_high];
        let ret = libc::syscall(
            libc::SYS_capset,
            &mut header as *mut CapHeader as *mut libc::c_void,
            data.as_mut_ptr() as *mut libc::c_void,
        );
        if ret != 0 {
            return Err(anyhow::anyhow!(
                "capset failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    debug!("container: privileged capabilities applied");
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
/// 3. Call [`crate::container::filesystem::apply_bind_mounts`] to apply any
///    bind mounts into the overlay rootfs inside the new mount namespace.
/// 4. Call [`pivot_root_to`] to switch the root filesystem to the overlay
///    merged directory.
/// 5. If `config.privileged` is true, call [`apply_full_capabilities`] to
///    grant all Linux capabilities via `capset(2)`.
/// 6. Call [`close_extra_fds`] to release any file descriptors > 2 that
///    leaked from the parent across the clone boundary.
/// 7. Build the `argv` vector and call `execvp` to exec the user command.
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

    // 3. Apply bind mounts into the overlay rootfs before pivot_root.
    //    These mounts live inside this child's new mount namespace (CLONE_NEWNS).
    crate::container::filesystem::apply_bind_mounts(&config.mounts, &config.rootfs)
        .with_context(|| "child: apply_bind_mounts")?;

    // 4. Pivot root to the overlay merged directory.
    pivot_root_to(&config.rootfs).with_context(|| "child: pivot_root")?;

    // 5. Apply privileged capability whitelist if requested.
    #[cfg(target_os = "linux")]
    if config.privileged {
        // Audit log: privileged containers are a security boundary relaxation.
        // CAP_SYS_MODULE, CAP_SYS_BOOT, CAP_MAC_OVERRIDE, CAP_MAC_ADMIN are
        // always withheld to limit host-escape surface.
        warn!(
            hostname = %config.hostname,
            command = %config.command,
            "container: starting in privileged mode — capability whitelist applied"
        );
        apply_privileged_capabilities().with_context(|| "child: apply_privileged_capabilities")?;
    }

    // 6. Become a new session and process group leader so the daemon can
    //    signal the entire container process tree by negating the PGID.
    //    Without setsid(), the child inherits the daemon's process group;
    //    with it, kill(-pgid) from stop_inner reaches every descendant
    //    (including grandchildren like `sleep` spawned by `/bin/sh -c …`)
    //    and bypasses the kernel rule that silently drops SIGTERM delivered
    //    to PID 1 of a PID namespace when no handler is installed.
    // SAFETY: setsid() is always safe to call; it fails only if the caller
    // is already a process group leader, which cannot happen here because
    // clone() always gives the child a new PID.
    let _ = unsafe { libc::setsid() };

    // 7. Close any file descriptors > 2 (stdin/stdout/stderr) that leaked
    //    from the parent. We do this on a best-effort basis.
    close_extra_fds();

    // 7. Build argv and envp for execve.
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

    let mut envp: Vec<CString> = Vec::with_capacity(config.env.len());
    for kv in &config.env {
        envp.push(
            CString::new(kv.as_str())
                .map_err(|_| ProcessError::SpawnFailed(format!("invalid env var: {kv}")))?,
        );
    }

    debug!(command = %config.command, "container: execve");

    execve(&cmd, &argv, &envp).map_err(|source| ProcessError::ExecFailed {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_config_privileged_defaults_false() {
        let cfg = ContainerConfig {
            rootfs: std::path::PathBuf::from("/tmp/test-rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            namespace_config: crate::container::namespace::NamespaceConfig::all(),
            cgroup_path: std::path::PathBuf::from("/sys/fs/cgroup/minibox/test"),
            hostname: "test".to_string(),
            capture_output: false,
            pre_exec_hooks: vec![],
            mounts: vec![],
            privileged: false,
        };
        assert!(!cfg.privileged);
        assert!(cfg.mounts.is_empty());
    }

    #[test]
    fn container_config_privileged_true() {
        let cfg = ContainerConfig {
            rootfs: std::path::PathBuf::from("/tmp/test-rootfs"),
            command: "/bin/sh".to_string(),
            args: vec![],
            env: vec![],
            namespace_config: crate::container::namespace::NamespaceConfig::all(),
            cgroup_path: std::path::PathBuf::from("/sys/fs/cgroup/minibox/test"),
            hostname: "test".to_string(),
            capture_output: false,
            pre_exec_hooks: vec![],
            mounts: vec![],
            privileged: true,
        };
        assert!(cfg.privileged);
    }

    /// Verify that the privileged capability bitmasks exclude the four
    /// host-escape capabilities and retain all others.
    #[test]
    fn privileged_capability_bitmasks_exclude_host_escape_caps() {
        // Reproduce the constants from apply_privileged_capabilities.
        const CAP_PRIVILEGED_LOW: u32 = !(1_u32 << 16) & !(1_u32 << 22);
        const CAP_PRIVILEGED_HIGH: u32 = 0x0000_01FF & !(1 << 0) & !(1 << 1);

        // CAP_SYS_MODULE (16) must be absent from low word.
        assert_eq!(
            CAP_PRIVILEGED_LOW & (1 << 16),
            0,
            "CAP_SYS_MODULE must be excluded"
        );
        // CAP_SYS_BOOT (22) must be absent from low word.
        assert_eq!(
            CAP_PRIVILEGED_LOW & (1 << 22),
            0,
            "CAP_SYS_BOOT must be excluded"
        );
        // CAP_MAC_OVERRIDE (32 → high bit 0) must be absent from high word.
        assert_eq!(
            CAP_PRIVILEGED_HIGH & (1 << 0),
            0,
            "CAP_MAC_OVERRIDE must be excluded"
        );
        // CAP_MAC_ADMIN (33 → high bit 1) must be absent from high word.
        assert_eq!(
            CAP_PRIVILEGED_HIGH & (1 << 1),
            0,
            "CAP_MAC_ADMIN must be excluded"
        );

        // All other low caps should be present (spot-check a few).
        assert_ne!(
            CAP_PRIVILEGED_LOW & (1 << 0),
            0,
            "CAP_CHOWN must be retained"
        );
        assert_ne!(
            CAP_PRIVILEGED_LOW & (1 << 21),
            0,
            "CAP_SYS_ADMIN must be retained"
        );
        assert_ne!(
            CAP_PRIVILEGED_LOW & (1 << 12),
            0,
            "CAP_NET_ADMIN must be retained"
        );

        // All other high caps should be present (spot-check).
        assert_ne!(
            CAP_PRIVILEGED_HIGH & (1 << 2),
            0,
            "CAP_SYSLOG must be retained"
        );
        assert_ne!(
            CAP_PRIVILEGED_HIGH & (1 << 7),
            0,
            "CAP_BPF must be retained"
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
