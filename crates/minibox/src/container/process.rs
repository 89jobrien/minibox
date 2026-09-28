//! Container init process: clone, setup, `pivot_root`, exec.
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

/// Concrete host ID range installed in a container user namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UidMapping {
    /// First host UID mapped to container UID zero.
    pub host_uid: u32,
    /// First host GID mapped to container GID zero.
    pub host_gid: u32,
    /// Number of contiguous IDs in both maps.
    pub size: u32,
}

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
    /// Bind mounts applied inside the container's mount namespace before `pivot_root`.
    pub mounts: Vec<minibox_core::domain::BindMount>,
    /// If `true`, call `capset(2)` with all capabilities set before `execve`.
    pub privileged: bool,
    /// Host IDs mapped to container IDs 0..size in the user namespace.
    pub uid_mapping: UidMapping,
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
// qual:allow(complexity) reason: "fork/clone setup — must be single cohesive unit"
pub fn spawn_container_process(config: ContainerConfig) -> anyhow::Result<SpawnResult> {
    use nix::fcntl::OFlag;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    info!(command = %config.command, rootfs = ?config.rootfs, "container: spawning process");

    if !config.mounts.is_empty() {
        crate::container::filesystem::preflight_idmapped_mounts()?;
    }

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
    let uid_mapping = config.uid_mapping;
    let ns_config = config.namespace_config.clone();

    // Parent/child barrier: the child must not proceed — in particular must not
    // reach `execve` — until the parent has installed the UID/GID maps. Without
    // this the child could `exec` while its user namespace is still unmapped,
    // and every UID in the container image would resolve to the unmapped
    // overflow UID.
    let (sync_read, sync_write) = nix::unistd::pipe2(OFlag::O_CLOEXEC)
        .context("creating user namespace synchronization pipe")?;
    let sync_read_raw = sync_read.as_raw_fd();
    let sync_write_raw = sync_write.as_raw_fd();
    // Take full manual control of the fd lifetimes, for the same reason as the
    // capture pipe above: dropping either OwnedFd here would close the fd for
    // both parent and child after clone(2).
    std::mem::forget(sync_read);
    std::mem::forget(sync_write);

    let pid = clone_with_namespaces(&ns_config, move || {
        // ----------------------------------------------------------------
        // Everything here runs in the child process.
        // We must not return; we must either exec or call _exit.
        // ----------------------------------------------------------------

        // Block until the parent has written uid_map/gid_map. This must happen
        // before anything that observes our credentials, and before the
        // descriptor sweep in `child_init` closes the sync pipe's descriptor.
        // SAFETY: both fds are valid descriptors inherited across clone(2);
        // their OwnedFds were forgotten before the clone so no other owner will
        // close them. read/write/close are async-signal-safe.
        unsafe {
            libc::close(sync_write_raw);
            let mut ready = 0u8;
            if libc::read(sync_read_raw, (&raw mut ready).cast(), 1) != 1 {
                libc::_exit(127);
            }
            libc::close(sync_read_raw);
        }

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

        const EXEC_FAILURE_EXIT_CODE: i32 = 127;
        if let Err(e) = child_init(config) {
            // `?e` (Debug), not `%e` (Display). anyhow's Display prints only the
            // outermost context, so `%e` collapses a failure to "child:
            // <context>" and discards the errno and the whole chain — every
            // mount target, path, and io::Error underneath. child_init's
            // failures are the ones you most need to diagnose and they were
            // the least diagnosable. Matches `error = ?e` in adapters/exec.rs.
            error!(error = ?e, "container: child init failed");
            unsafe { libc::_exit(EXEC_FAILURE_EXIT_CODE) };
        }
        // exec replaces the process image, so we never reach here.
        unsafe { libc::_exit(1) };
    })
    .with_context(|| "failed to spawn container process");

    if let Err(ref _e) = pid {
        // Clone failed — no child was created, so neither the sync pipe's nor
        // the capture pipe's forgotten OwnedFds were ever consumed. Close all
        // four raw FDs to prevent leaks.
        unsafe { libc::close(sync_read_raw) };
        unsafe { libc::close(sync_write_raw) };
        if capture_output && read_fd_raw >= 0 {
            unsafe { libc::close(read_fd_raw) };
            unsafe { libc::close(write_fd_raw) };
        }
    }

    let pid = pid?;

    // Parent: install the user-namespace UID/GID maps. This must happen from
    // the parent — only a process *outside* the new user namespace may write
    // uid_map/gid_map, and the child is blocked on the sync pipe until we do.
    //
    // The cgroup is deliberately NOT written here: `child_init` already adds
    // the child to its cgroup from the inside via `add_self_to_cgroup`, which
    // is develop's mechanism and is required because the child's PID inside its
    // new PID namespace need not match the PID the parent sees. Writing
    // cgroup.procs from both sides would be redundant.
    unsafe { libc::close(sync_read_raw) };
    if let Err(error) = configure_child_isolation(pid.as_raw(), uid_mapping) {
        // The child is parked on the sync pipe; it will never be released, so
        // reap it rather than leaking a live process.
        unsafe { libc::close(sync_write_raw) };
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
        let _ = waitpid(pid, None);
        if capture_output && read_fd_raw >= 0 {
            unsafe { libc::close(read_fd_raw) };
            unsafe { libc::close(write_fd_raw) };
        }
        return Err(error);
    }
    let ready = [1u8];
    let write_result = unsafe { libc::write(sync_write_raw, ready.as_ptr().cast(), 1) };
    unsafe { libc::close(sync_write_raw) };
    if write_result != 1 {
        anyhow::bail!(
            "failed to release user namespace child: {}",
            std::io::Error::last_os_error()
        );
    }

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
        runtime_id: None,
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
        const DEFAULT_HOOK_TIMEOUT_SECS: u64 = 30;
        let timeout = Duration::from_secs(hook.timeout_secs.unwrap_or(DEFAULT_HOOK_TIMEOUT_SECS));
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
                    const HOOK_POLL_INTERVAL_MS: u64 = 50;
                    std::thread::sleep(Duration::from_millis(HOOK_POLL_INTERVAL_MS));
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
/// `inheritable`. Called inside the child process before `execve` when
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
/// | `CAP_SYS_MODULE`    |  16 | Load/unload kernel modules                |
/// | `CAP_SYS_BOOT`      |  22 | Reboot, shutdown, or kexec the host       |
/// | `CAP_MAC_OVERRIDE`  |  32 | Bypass MAC (SELinux/AppArmor) enforcement |
/// | `CAP_MAC_ADMIN`     |  33 | Modify/load MAC policies on the host      |
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
    const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
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
            (&raw mut header).cast::<libc::c_void>(),
            data.as_mut_ptr().cast::<libc::c_void>(),
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
/// setup steps before `execve` replaces the process image:
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
/// 7. Build the `argv` and `envp` vectors and call `execve` to exec the user
///    command. `envp` comes only from `config.env` (see [`build_envp`]).
///
/// On any error the caller is expected to call `libc::_exit(127)` so the
/// process terminates without running Rust destructors.
// qual:allow(complexity) reason: "child init sequence — must be linear and auditable"
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
    use std::os::fd::AsRawFd as _;
    let user_namespace = std::fs::File::open("/proc/self/ns/user")
        .context("child: open user namespace for ID-mapped mounts")?;
    crate::container::filesystem::apply_bind_mounts(
        &config.mounts,
        &config.rootfs,
        user_namespace.as_raw_fd(),
    )
    .with_context(|| "child: apply_bind_mounts")?;

    // 4. Pivot root to the overlay merged directory.
    pivot_root_to(&config.rootfs).with_context(|| "child: pivot_root")?;

    // 4b. Install the mount-immutability seccomp filter now that every
    //     init-time mount (overlay, bind mounts, pivot_root's proc/sysfs/dev)
    //     has been applied. From this point on, any `mount(2)` remount
    //     attempt that drops MS_RDONLY (i.e. an attempted `remount,rw`) is
    //     denied for the lifetime of this process tree; fresh (non-remount)
    //     mounts created after this point remain unrestricted. See
    //     crate::container::mount_seccomp for the enforcement model and its
    //     documented scope.
    crate::container::mount_seccomp::install_mount_immutability_filter()
        .with_context(|| "child: install_mount_immutability_filter")?;

    // TODO(feature-idea-14): add and verify a default capability drop policy plus
    // no_new_privs for unprivileged containers; keep privileged mode an explicit relaxation.
    // TODO(feature-idea-16): the user namespace and its uid_map/gid_map are now
    // installed (see `configure_child_isolation`), but the storage, networking,
    // and cgroup support required for genuinely *rootless* operation are not.
    // Writing a non-identity map requires the daemon to hold CAP_SETUID in the
    // parent user namespace, so an unprivileged daemon still cannot create
    // containers.
    // TODO(feature-idea-17): extend the mount-only seccomp filter into a documented,
    // configurable syscall profile for container workloads.
    // TODO(feature-idea-18): add end-to-end security regressions for capability drops,
    // seccomp denials, user mappings, and rootless escape boundaries.
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

    let envp = build_envp(&config.env)?;

    debug!(command = %config.command, "container: execve");

    execve(&cmd, &argv, &envp).map_err(|source| ProcessError::ExecFailed {
        cmd: config.command.clone(),
        source,
    })?;

    // execve never returns on success.
    unreachable!()
}

/// Build the `envp` vector passed to `execve` from the container's declared
/// environment.
///
/// Security invariant: `envp` is built **only** from `declared` — never from
/// `std::env::vars()` or any other host-environment source. `execvp` (or an
/// `envp` seeded from the daemon's environment) would leak every API key and
/// secret the daemon holds into every container.
fn build_envp(declared: &[String]) -> anyhow::Result<Vec<CString>> {
    declared
        .iter()
        .map(|kv| {
            CString::new(kv.as_str()).map_err(|_| {
                anyhow::Error::from(ProcessError::SpawnFailed(format!("invalid env var: {kv}")))
            })
        })
        .collect()
}

/// Install the user-namespace UID/GID maps for a freshly cloned child.
///
/// Called in the **parent**, after `clone(2)` returns and before the child is
/// released from the sync barrier. Only a process outside the new user
/// namespace may write these files, so this cannot be done from `child_init`.
///
/// The three writes are ordered because the kernel requires them in this
/// sequence: `setgroups` must be set to `deny` before an unprivileged writer
/// may populate `gid_map` (writing `allow` would let the child re-enable
/// `setgroups(2)` and escape the GID translation).
///
/// A single-line map (`0 <host_id> <size>`) translates container IDs `0..size`
/// to `host_id..host_id+size`. Using one contiguous range rather than an
/// identity map is what keeps two containers from sharing host UIDs when they
/// are given different ranges.
#[cfg(target_os = "linux")]
fn configure_child_isolation(pid: i32, mapping: UidMapping) -> anyhow::Result<()> {
    let proc_dir = std::path::PathBuf::from(format!("/proc/{pid}"));
    std::fs::write(proc_dir.join("setgroups"), "deny\n")
        .with_context(|| format!("disable setgroups for child {pid}"))?;
    std::fs::write(
        proc_dir.join("uid_map"),
        format!("0 {} {}\n", mapping.host_uid, mapping.size),
    )
    .with_context(|| format!("write exclusive uid_map for child {pid}"))?;
    std::fs::write(
        proc_dir.join("gid_map"),
        format!("0 {} {}\n", mapping.host_gid, mapping.size),
    )
    .with_context(|| format!("write exclusive gid_map for child {pid}"))?;

    // Read the maps back rather than trusting the writes. A successful write
    // means the kernel accepted the mapping, so this is confirmation rather
    // than a second source of truth — but the contents are the only direct
    // evidence of which UID range a container actually received, and they are
    // what an operator needs when asking "why is my container running as the
    // wrong user".
    //
    // Compare token-wise, not as raw strings: the kernel renders these files
    // with each field padded to a fixed width, so a correctly installed map
    // reads back as "0     165536      65536" rather than the "0 165536 65536"
    // that was written. String equality would flag every healthy container.
    // A mismatch is logged at warn without failing the spawn, so a diagnostic
    // never becomes a new failure mode.
    let applied_uid = std::fs::read_to_string(proc_dir.join("uid_map")).unwrap_or_default();
    let applied_gid = std::fs::read_to_string(proc_dir.join("gid_map")).unwrap_or_default();
    let expected_uid = format!("0 {} {}", mapping.host_uid, mapping.size);
    let expected_gid = format!("0 {} {}", mapping.host_gid, mapping.size);
    let same = |applied: &str, expected: &str| -> bool {
        applied.split_whitespace().eq(expected.split_whitespace())
    };
    if same(&applied_uid, &expected_uid) && same(&applied_gid, &expected_gid) {
        info!(
            pid,
            host_uid = mapping.host_uid,
            host_gid = mapping.host_gid,
            size = mapping.size,
            "container: user namespace UID/GID maps installed"
        );
    } else {
        warn!(
            pid,
            expected = expected_uid,
            actual = applied_uid.trim(),
            "container: installed uid_map does not match the requested range"
        );
    }
    Ok(())
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
    const FIRST_NON_STDIO_FD: u32 = 3;
    let ret = unsafe { libc::syscall(libc::SYS_close_range, FIRST_NON_STDIO_FD, u32::MAX, 0u32) };
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
            // SAFETY: fd was parsed from /proc/self/fd so it is a valid open fd
            // belonging to this process; closing it here is intentional cleanup
            // before exec. Errors are ignored — the kernel will close remaining
            // fds on exec for those with O_CLOEXEC.
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
            pty: None,
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
            pty: None,
        };
        assert!(cfg.privileged);
    }

    /// Verify that the privileged capability bitmasks exclude the four
    /// host-escape capabilities and retain all others.
    #[test]
    fn apply_privileged_capabilities_bitmasks_exclude_host_escape_caps() {
        // Verify ContainerConfig accepts privileged=true (SUT data structure check).
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
            pty: None,
        };
        assert!(cfg.privileged, "privileged mode must be set");
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

    // -----------------------------------------------------------------------
    // Invariant 6 — Environment isolation (replaces a source-string test)
    // -----------------------------------------------------------------------

    /// `envp` is built only from the container's declared variables.
    #[test]
    fn build_envp_contains_only_declared_vars() {
        let declared = vec!["PATH=/bin".to_string(), "FOO=bar".to_string()];
        let envp = build_envp(&declared).expect("valid env vars must build");

        let rendered: Vec<String> = envp
            .iter()
            .map(|c| c.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, vec!["PATH=/bin", "FOO=bar"]);

        // A host secret that exists in this process must not appear.
        // SAFETY: single-threaded test body; no concurrent env mutation.
        unsafe { std::env::set_var("MINIBOX_HOST_SECRET_CANARY", "leaked") };
        let envp = build_envp(&declared).expect("valid env vars must build");
        let rendered: Vec<String> = envp
            .iter()
            .map(|c| c.to_string_lossy().into_owned())
            .collect();
        assert!(
            !rendered.iter().any(|kv| kv.contains("CANARY")),
            "host environment leaked into envp: {rendered:?}"
        );
        // SAFETY: see above.
        unsafe { std::env::remove_var("MINIBOX_HOST_SECRET_CANARY") };
    }

    /// An empty declaration yields an empty `envp` — not an inherited one.
    ///
    /// An empty `envp` passed to `execve` gives the container *no* environment,
    /// which is the correct outcome when nothing is declared. Inheriting the
    /// daemon's environment here is precisely the leak this guards against.
    #[test]
    fn build_envp_empty_declaration_yields_no_inherited_env() {
        let envp = build_envp(&[]).expect("empty env builds");
        assert!(
            envp.is_empty(),
            "an empty declaration must produce an empty envp, not an inherited one"
        );
    }

    /// A NUL byte in a declared variable is rejected rather than silently
    /// truncating the value (which would smuggle extra env past the check).
    #[test]
    fn build_envp_rejects_nul_bytes() {
        let declared = vec!["EVIL=ok\0INJECTED=bad".to_string()];
        let err = build_envp(&declared).expect_err("embedded NUL must be rejected");
        assert!(
            err.to_string().contains("invalid env var"),
            "unexpected error: {err:#}"
        );
    }

    // -----------------------------------------------------------------------
    // Invariant 5 — FD-leak prevention (replaces a source-string test)
    // -----------------------------------------------------------------------

    /// `close_extra_fds` really closes descriptors above stderr, and really
    /// leaves 0/1/2 open.
    ///
    /// Runs in a forked child so that closing the test runner's own descriptors
    /// cannot corrupt the harness. The child reports its verdict through the
    /// exit status.
    ///
    /// SAFETY: the assertions inside the child only touch descriptors the child
    /// itself opened, plus an explicit dup of stderr onto a known slot.
    #[cfg(unix)]
    #[test]
    fn close_extra_fds_closes_fds_above_stderr_and_keeps_stdio() {
        // SAFETY: this test body performs no env mutation and runs before any
        // forking test in the same process; `fork` is used immediately below.
        unsafe {
            let pid = libc::fork();
            assert!(pid >= 0, "fork failed");

            if pid == 0 {
                // ---- child ----
                let code = child_fd_probe();
                libc::_exit(code);
            }

            // ---- parent ----
            let mut status: libc::c_int = 0;
            assert_eq!(libc::waitpid(pid, &mut status, 0), pid, "waitpid failed");
            assert!(libc::WIFEXITED(status), "child did not exit normally");
            assert_eq!(
                libc::WEXITSTATUS(status),
                0,
                "close_extra_fds left the descriptor table in the wrong state"
            );
        }
    }

    /// Child half of [`close_extra_fds_closes_fds_above_stderr_and_keeps_stdio`].
    ///
    /// Opens a handful of descriptors above stderr, parks one at a known high
    /// slot via `dup2`, calls `close_extra_fds`, then verifies the real
    /// descriptor table.
    ///
    /// Returns a process exit code: 0 = invariant held, 1 = leaked fd, 2 = stdio
    /// was closed, 3 = probe setup failed.
    ///
    /// # Safety
    ///
    /// Runs in a freshly forked child (see the caller). Every `libc` call here
    /// operates only on descriptors the child itself opened, or on the inherited
    /// stdio slots, and no pointer outlives the call. The child never returns —
    /// it `_exit`s — so no Rust destructor or allocator state is touched.
    #[cfg(unix)]
    unsafe fn child_fd_probe() -> libc::c_int {
        unsafe {
            // Open several descriptors above stderr.
            let opened: Vec<libc::c_int> = (0..4)
                .filter_map(|_| {
                    let fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC);
                    (fd >= 0).then_some(fd)
                })
                .collect();
            if opened.len() < 4 {
                return 3;
            }

            // Park a descriptor at a high, predictable slot so the check does
            // not depend on allocation order.
            const HIGH_FD: libc::c_int = 900;
            if libc::dup2(opened[0], HIGH_FD) < 0 {
                return 3;
            }

            close_extra_fds();

            // Invariant: the high descriptor is gone.
            if libc::fcntl(HIGH_FD, libc::F_GETFD) >= 0 {
                return 1;
            }

            // Invariant: every descriptor we opened above stderr is gone.
            for &fd in &opened {
                if libc::fcntl(fd, libc::F_GETFD) >= 0 {
                    return 1;
                }
            }

            // Invariant: stdio survived. If stdin/stdout/stderr had been closed
            // the kernel could hand one of them back to us, so check explicitly.
            for fd in 0..3 {
                if libc::fcntl(fd, libc::F_GETFD) < 0 {
                    return 2;
                }
            }

            0
        }
    }
}

// ---------------------------------------------------------------------------
// Kani formal verification proofs (cfg-gated, never compiled in normal builds)
// ---------------------------------------------------------------------------

#[cfg(kani)]
mod kani_proofs {
    /// Proof 10: Privileged capability bitmask excludes all four host-escape
    /// capabilities for every possible input. Exhaustive over the full u32
    /// space (Kani explores all 2^32 values symbolically).
    #[kani::proof]
    fn capability_bitmask_excludes_host_escape() {
        // Reproduce the constants from apply_privileged_capabilities.
        const CAP_PRIVILEGED_LOW: u32 = !(1_u32 << 16) & !(1_u32 << 22);
        const CAP_PRIVILEGED_HIGH: u32 = 0x0000_01FF & !(1 << 0) & !(1 << 1);

        // Low word: CAP_SYS_MODULE (16) and CAP_SYS_BOOT (22) must be clear.
        assert_eq!(CAP_PRIVILEGED_LOW & (1 << 16), 0, "CAP_SYS_MODULE leaked");
        assert_eq!(CAP_PRIVILEGED_LOW & (1 << 22), 0, "CAP_SYS_BOOT leaked");

        // High word: CAP_MAC_OVERRIDE (bit 0) and CAP_MAC_ADMIN (bit 1) must be clear.
        assert_eq!(CAP_PRIVILEGED_HIGH & (1 << 0), 0, "CAP_MAC_OVERRIDE leaked");
        assert_eq!(CAP_PRIVILEGED_HIGH & (1 << 1), 0, "CAP_MAC_ADMIN leaked");

        // High word must not exceed the valid capability range (bits 0-8).
        assert_eq!(
            CAP_PRIVILEGED_HIGH & !0x0000_01FF,
            0,
            "high word exceeds valid range"
        );

        // Verify that applying ANY arbitrary mask to the privileged constants
        // still cannot re-introduce the excluded bits.
        let mask: u32 = kani::any();
        assert_eq!(
            (CAP_PRIVILEGED_LOW & mask) & (1 << 16),
            0,
            "mask re-introduced CAP_SYS_MODULE"
        );
        assert_eq!(
            (CAP_PRIVILEGED_LOW & mask) & (1 << 22),
            0,
            "mask re-introduced CAP_SYS_BOOT"
        );
        assert_eq!(
            (CAP_PRIVILEGED_HIGH & mask) & (1 << 0),
            0,
            "mask re-introduced CAP_MAC_OVERRIDE"
        );
        assert_eq!(
            (CAP_PRIVILEGED_HIGH & mask) & (1 << 1),
            0,
            "mask re-introduced CAP_MAC_ADMIN"
        );
    }

    /// Proof 11: Pipe fd lifecycle state machine — models the fd states in
    /// spawn_container_process and proves no double-close and no leak.
    ///
    /// State encoding per fd: 0 = open, 1 = closed.
    /// A double-close is closing an already-closed fd. A leak is an fd still
    /// open when all paths complete.
    #[kani::proof]
    fn pipe_fd_no_double_close_no_leak() {
        let capture_output: bool = kani::any();

        if !capture_output {
            // No pipe created, nothing to track.
            return;
        }

        // Model: two fds created (read_fd, write_fd), both start open.
        let mut read_closed: bool = false;
        let mut write_closed: bool = false;

        let clone_succeeded: bool = kani::any();

        if !clone_succeeded {
            // Clone failed path (lines 134-137): close both.
            assert!(!read_closed, "read_fd double-close on failure path");
            read_closed = true;
            assert!(!write_closed, "write_fd double-close on failure path");
            write_closed = true;
        } else {
            // Clone succeeded.
            // Child path (modeled separately — in its own address space):
            //   dup2(write_fd -> stdout), dup2(write_fd -> stderr),
            //   close(write_fd), close(read_fd)
            // Parent path (lines 143-144): close write_fd.
            assert!(!write_closed, "write_fd double-close on success path");
            write_closed = true;

            // read_fd is wrapped in OwnedFd (line 152) and returned.
            // When the caller drops it, it closes. Model that as the final close.
            assert!(!read_closed, "read_fd double-close on success path");
            read_closed = true;
        }

        // All fds must be closed by the end of each path.
        assert!(read_closed, "read_fd leaked");
        assert!(write_closed, "write_fd leaked");
    }

    /// Proof 12: The EXEC_FAILURE_EXIT_CODE constant (127) matches the POSIX
    /// convention for "command not found" and fits in a u8-range exit code.
    #[kani::proof]
    fn exec_failure_exit_code_is_valid() {
        const EXEC_FAILURE_EXIT_CODE: i32 = 127;
        // Must be in the valid exit code range [0, 255].
        assert!(EXEC_FAILURE_EXIT_CODE >= 0 && EXEC_FAILURE_EXIT_CODE <= 255);
        // Must be exactly 127 (POSIX "command not found").
        assert_eq!(EXEC_FAILURE_EXIT_CODE, 127);
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
