//! Mount immutability enforcement via a seccomp BPF filter.
//!
//! Sysbox-style runtimes intercept `mount(2)` and enforce a one-way ratchet:
//! a mount created read-only at container init can never be remounted
//! read-write, while a read-write mount may always be tightened to
//! read-only. See `docs/ideas/sysbox-security.md` for the reference model
//! this mirrors, and issue #449 for the acceptance criteria.
//!
//! # Design
//!
//! A classic (non-eBPF) seccomp filter cannot consult kernel mount-table
//! state (it only ever sees the raw syscall arguments), so it cannot know
//! whether a specific mount target was originally read-only. Rather than
//! adding a `SECCOMP_RET_USER_NOTIF` supervisor (a much larger change — see
//! "Deferred" below), this filter enforces a conservative approximation of
//! the one-way ratchet using only the `mount(2)` argument bits:
//!
//! - Any `mount(2)` call with `MS_REMOUNT` set **and without `MS_RDONLY`
//!   set** is denied (`EPERM`). This blocks `mount -o remount,rw` on any
//!   mount, matching "read-only mounts can never be remounted read-write."
//! - Any `mount(2)` call with `MS_REMOUNT` and `MS_RDONLY` both set is
//!   allowed, matching "read-write mounts may be remounted read-only."
//! - Any `mount(2)` call **without** `MS_REMOUNT` (i.e. a fresh mount) is
//!   unaffected — "new mounts after init are unrestricted."
//!
//! This is intentionally coarser than sysbox's per-mount policy: it treats
//! *every* mount as if it might be read-only-protected, so a mount that was
//! created read-write and legitimately wants `remount,rw` again (a no-op)
//! is also denied. That tradeoff is the surgical scope chosen for this
//! change — see the module doc on [`install_mount_immutability_filter`].
//!
//! # Deferred
//!
//! A precise per-mount policy (matching sysbox's table exactly, including
//! "read-write mounts may be remounted read-write") would require either:
//! - `SECCOMP_RET_USER_NOTIF` plus a supervisor process that resolves the
//!   target path from `/proc/[pid]/mountinfo` and consults a per-container
//!   mount policy, or
//! - An LSM (e.g. a small BPF LSM program attached to `security_sb_mount`)
//!   with access to the `super_block` and path.
//!
//! Both are substantially larger changes (new supervisor process or BPF
//! LSM program, path resolution, IPC) and are out of scope here. This
//! module implements the surgical, fully-testable BPF-argument-only
//! variant and documents the gap explicitly.

use crate::error::ProcessError;
use tracing::debug;

/// `MS_REMOUNT` from `<sys/mount.h>` (also `nix::mount::MsFlags::MS_REMOUNT`).
const MS_REMOUNT: u32 = 32;
/// `MS_RDONLY` from `<sys/mount.h>` (also `nix::mount::MsFlags::MS_RDONLY`).
const MS_RDONLY: u32 = 1;

// ---------------------------------------------------------------------------
// seccomp_data field offsets (struct seccomp_data, linux/seccomp.h)
//
// struct seccomp_data {
//     int nr;                    // offset 0,  4 bytes
//     __u32 arch;                // offset 4,  4 bytes
//     __u64 instruction_pointer; // offset 8,  8 bytes
//     __u64 args[6];             // offset 16, 8 bytes each
// };
//
// mount(2) is `mount(source, target, filesystemtype, mountflags, data)`, so
// `mountflags` is args[3]. All flag values fit in 32 bits, so on the
// little-endian targets minibox supports (x86_64, aarch64) the low 32 bits
// of args[3] (offset 16 + 3*8 = 40) carry the full flag value.
// ---------------------------------------------------------------------------

const OFFSET_NR: u32 = 0;
const OFFSET_ARCH: u32 = 4;
const OFFSET_ARGS3_LOW: u32 = 16 + (3 * 8);

/// `AUDIT_ARCH_X86_64` (`EM_X86_64 | __AUDIT_ARCH_64BIT | __AUDIT_ARCH_LE`).
#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH_CURRENT: u32 = 0xC000_003E;
/// `AUDIT_ARCH_AARCH64` (`EM_AARCH64 | __AUDIT_ARCH_64BIT | __AUDIT_ARCH_LE`).
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH_CURRENT: u32 = 0xC000_00B7;

// BPF classic opcode components (linux/filter.h / linux/bpf_common.h).
const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_K: u16 = 0x00;
const BPF_RET: u16 = 0x06;
const BPF_ALU: u16 = 0x04;
const BPF_AND: u16 = 0x50;

const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;

const fn stmt(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

const fn jump(code: u16, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter { code, jt, jf, k }
}

/// Build the BPF program enforcing the mount-immutability ratchet described
/// in the module docs.
///
/// Pure data construction — no syscalls — so this is testable on any
/// platform, but the module itself only compiles on Linux (the whole
/// `container` module tree is `#[cfg(target_os = "linux")]`-gated in
/// `lib.rs`).
fn build_mount_immutability_program() -> Vec<libc::sock_filter> {
    vec![
        // 0: load syscall arch
        stmt(BPF_LD | BPF_W | BPF_ABS, OFFSET_ARCH),
        // 1: if arch != current, jump to ALLOW (index 11); we only reason
        //    about the architecture we were compiled for.
        jump(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH_CURRENT, 0, 9),
        // 2: load syscall number
        stmt(BPF_LD | BPF_W | BPF_ABS, OFFSET_NR),
        // 3: if nr != SYS_mount, jump to ALLOW (index 11)
        jump(BPF_JMP | BPF_JEQ | BPF_K, libc::SYS_mount as u32, 0, 7),
        // 4: load low word of mountflags (args[3])
        stmt(BPF_LD | BPF_W | BPF_ABS, OFFSET_ARGS3_LOW),
        // 5: A &= MS_REMOUNT
        stmt(BPF_ALU | BPF_AND | BPF_K, MS_REMOUNT),
        // 6: if A == 0 (not a remount), jump to ALLOW (index 11)
        jump(BPF_JMP | BPF_JEQ | BPF_K, 0, 4, 0),
        // 7: reload mountflags (previous AND clobbered A)
        stmt(BPF_LD | BPF_W | BPF_ABS, OFFSET_ARGS3_LOW),
        // 8: A &= MS_RDONLY
        stmt(BPF_ALU | BPF_AND | BPF_K, MS_RDONLY),
        // 9: if A == 0 (RDONLY absent -> remount,rw attempt), fall through to DENY;
        //    otherwise (RDONLY present -> remount,ro) jump to ALLOW (index 11)
        jump(BPF_JMP | BPF_JEQ | BPF_K, 0, 0, 1),
        // 10: DENY
        stmt(
            BPF_RET | BPF_K,
            SECCOMP_RET_ERRNO | (libc::EPERM as u32 & 0xffff),
        ),
        // 11: ALLOW
        stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
    ]
}

/// Install the mount-immutability seccomp filter in the current process.
///
/// Must be called inside the container child process, after all init-time
/// mounts (overlay, bind mounts, `pivot_root`'s `proc`/`sysfs`/`dev`) have
/// been applied and before `execve`. Once installed, the filter is
/// inherited across `fork`/`clone`/`execve` by the entire container process
/// tree (seccomp filters are never removable, only ever narrowed further),
/// so no workload process — including ones started after this point — can
/// widen a read-only mount back to read-write.
///
/// # Errors
///
/// Returns an error if `PR_SET_NO_NEW_PRIVS` or the filter installation
/// itself fails. Both are simple `prctl(2)` calls that only fail on
/// misconfigured kernels (e.g. seccomp disabled at build time) or an
/// already-more-restrictive filter being present.
pub fn install_mount_immutability_filter() -> anyhow::Result<()> {
    // no_new_privs is required by the kernel before an unprivileged process
    // may install a seccomp filter (CVE-avoidance: prevents setuid binaries
    // from being run under a filter the invoker doesn't consent to).
    //
    // SAFETY: prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) has no aliasing/pointer
    // arguments here (the trailing three args are ignored by the kernel for
    // this option) and cannot cause undefined behaviour; it only affects the
    // calling process's `no_new_privs` flag.
    let rc = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        return Err(ProcessError::SeccompInstallFailed(format!(
            "prctl(PR_SET_NO_NEW_PRIVS) failed: {err}"
        ))
        .into());
    }

    let program = build_mount_immutability_program();
    let fprog = libc::sock_fprog {
        // Program length is a fixed, small compile-time constant (12
        // instructions), always representable in u16.
        len: program.len() as u16,
        filter: program.as_ptr().cast_mut(),
    };

    // SAFETY: `fprog.filter` points at `program`, which is alive for the
    // duration of this call (it is not dropped until after `prctl`
    // returns). `PR_SET_SECCOMP` with `SECCOMP_MODE_FILTER` reads the
    // `sock_fprog` once during the call and does not retain the pointer
    // afterward, so no dangling-pointer invariant needs to outlive this
    // scope. The trailing two `prctl` varargs are unused for this option.
    let rc = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER,
            std::ptr::addr_of!(fprog),
            0,
            0,
        )
    };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        return Err(ProcessError::SeccompInstallFailed(format!(
            "prctl(PR_SET_SECCOMP) failed: {err}"
        ))
        .into());
    }

    debug!("container: mount immutability seccomp filter installed");
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn program_has_expected_instruction_count() {
        let program = build_mount_immutability_program();
        assert_eq!(program.len(), 12);
    }

    #[test]
    fn program_ends_with_allow() {
        let program = build_mount_immutability_program();
        let last = program.last().expect("program is non-empty");
        assert_eq!(last.code, BPF_RET | BPF_K);
        assert_eq!(last.k, SECCOMP_RET_ALLOW);
    }

    #[test]
    fn program_denies_remount_without_rdonly() {
        let program = build_mount_immutability_program();
        // Instruction 10 (0-indexed) is the DENY RET.
        let deny = program[10];
        assert_eq!(deny.code, BPF_RET | BPF_K);
        assert_eq!(deny.k, SECCOMP_RET_ERRNO | (libc::EPERM as u32 & 0xffff));
    }

    #[test]
    fn remount_rdonly_check_masks_match_kernel_flags() {
        // Pin the hardcoded flag values against nix's MsFlags so a future
        // nix upgrade that changes these constants (it won't -- they're
        // kernel ABI -- but as a regression guard) is caught here rather
        // than silently producing a no-op or overly broad filter.
        assert_eq!(
            u64::from(MS_REMOUNT),
            nix::mount::MsFlags::MS_REMOUNT.bits()
        );
        assert_eq!(u64::from(MS_RDONLY), nix::mount::MsFlags::MS_RDONLY.bits());
    }

    /// The jump targets in `build_mount_immutability_program` are computed
    /// by hand in comments; this test walks the program with a tiny
    /// interpreter to catch any drift between the comments and the actual
    /// `jt`/`jf` values, for every arch/nr/flags combination that matters.
    #[test]
    fn interpreter_matches_documented_policy() {
        fn run(nr: i64, arch: u32, flags: u32) -> u32 {
            let program = build_mount_immutability_program();
            let data_for_offset = |offset: u32| -> u32 {
                match offset {
                    OFFSET_NR => nr as u32,
                    OFFSET_ARCH => arch,
                    OFFSET_ARGS3_LOW => flags,
                    other => panic!("unexpected offset {other} read by test interpreter"),
                }
            };

            let mut pc: usize = 0;
            let mut acc: u32 = 0;
            loop {
                let instr = program[pc];
                if instr.code == (BPF_LD | BPF_W | BPF_ABS) {
                    acc = data_for_offset(instr.k);
                    pc += 1;
                } else if instr.code == (BPF_ALU | BPF_AND | BPF_K) {
                    acc &= instr.k;
                    pc += 1;
                } else if instr.code == (BPF_JMP | BPF_JEQ | BPF_K) {
                    pc += 1 + if acc == instr.k {
                        instr.jt as usize
                    } else {
                        instr.jf as usize
                    };
                } else if instr.code == (BPF_RET | BPF_K) {
                    return instr.k;
                } else {
                    panic!("unhandled instruction {:?}", instr.code);
                }
            }
        }

        let arch = AUDIT_ARCH_CURRENT;
        let sys_mount = libc::SYS_mount as i64;

        // remount,rw (MS_REMOUNT set, MS_RDONLY absent) -> denied.
        assert_eq!(
            run(sys_mount, arch, MS_REMOUNT),
            SECCOMP_RET_ERRNO | (libc::EPERM as u32 & 0xffff)
        );

        // remount,ro (MS_REMOUNT | MS_RDONLY) -> allowed.
        assert_eq!(
            run(sys_mount, arch, MS_REMOUNT | MS_RDONLY),
            SECCOMP_RET_ALLOW
        );

        // fresh mount (no MS_REMOUNT) -> allowed regardless of other flags.
        assert_eq!(run(sys_mount, arch, MS_RDONLY), SECCOMP_RET_ALLOW);
        assert_eq!(run(sys_mount, arch, 0), SECCOMP_RET_ALLOW);

        // non-mount syscall -> always allowed.
        assert_eq!(run(sys_mount + 1, arch, MS_REMOUNT), SECCOMP_RET_ALLOW);

        // wrong arch -> allowed (conservative: we only police our own arch).
        assert_eq!(
            run(sys_mount, arch.wrapping_add(1), MS_REMOUNT),
            SECCOMP_RET_ALLOW
        );
    }

    /// Real kernel enforcement test (Linux + root only). Forks a throwaway
    /// child process that:
    /// 1. Mounts a tmpfs and remounts it read-only (simulating an
    ///    init-time read-only mount).
    /// 2. Installs the mount-immutability seccomp filter.
    /// 3. Attempts `remount,rw` on it -- must fail (EPERM) per acceptance
    ///    criterion "mount -o remount,rw /readonly-mount fails".
    /// 4. Attempts `remount,ro` on it again -- must succeed per acceptance
    ///    criterion "mount -o remount,ro /readwrite-mount succeeds"
    ///    (remounting to the same, more restrictive state is always
    ///    allowed).
    /// 5. Mounts a second, fresh tmpfs -- must succeed per acceptance
    ///    criterion "new mounts after init are unrestricted".
    ///
    /// The child never returns to shared Rust state after `fork()` --
    /// it only calls libc mount/prctl functions and exits via
    /// `libc::_exit`, mirroring the fork-then-exit-only pattern used by
    /// the real container spawn path in `process.rs` to avoid
    /// fork-in-multithreaded-process hazards.
    #[test]
    fn kernel_enforces_remount_rw_denial_after_filter_install() {
        // SAFETY: geteuid() is a pure read of the process credential with no
        // side effects; always safe to call.
        if unsafe { libc::geteuid() } != 0 {
            eprintln!("skipping: requires root");
            return;
        }

        let dir = tempfile::TempDir::new().expect("tempdir");
        let ro_target = dir.path().join("ro");
        let fresh_target = dir.path().join("fresh");
        std::fs::create_dir_all(&ro_target).expect("mkdir ro_target");
        std::fs::create_dir_all(&fresh_target).expect("mkdir fresh_target");

        let ro_target_c =
            std::ffi::CString::new(ro_target.as_os_str().as_encoded_bytes()).expect("cstring");
        let fresh_target_c =
            std::ffi::CString::new(fresh_target.as_os_str().as_encoded_bytes()).expect("cstring");
        let tmpfs_c = std::ffi::CString::new("tmpfs").expect("cstring");

        // SAFETY: fork() is unsafe because the child shares the parent's
        // address space until exec/exit. The child below performs only
        // async-signal-safe-adjacent libc calls (mount, prctl) and always
        // terminates via `libc::_exit`, never returning to shared Rust
        // runtime state (allocator locks, tokio, etc.), which is the same
        // discipline the real container child_init path follows.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");

        if pid == 0 {
            // Child: never return normally.
            // SAFETY: all calls below are libc functions taking C strings
            // (`CString`s owned above, alive for the child's entire short
            // lifetime) or null pointers where the kernel permits omitting
            // an argument (`mount`'s source/fstype/data for a remount, and
            // `waitpid`-style status args are not used here). The child
            // never touches Rust runtime state shared with the parent
            // (allocator, tokio, etc.) and always terminates via
            // `libc::_exit`, so no destructors or shared locks are at risk.
            unsafe {
                // 1. Mount tmpfs read-write, then remount read-only (this is
                //    the "initial read-only mount" the ratchet protects).
                let rc = libc::mount(
                    tmpfs_c.as_ptr(),
                    ro_target_c.as_ptr(),
                    tmpfs_c.as_ptr(),
                    0,
                    std::ptr::null(),
                );
                if rc != 0 {
                    libc::_exit(10);
                }
                let rc = libc::mount(
                    std::ptr::null(),
                    ro_target_c.as_ptr(),
                    std::ptr::null(),
                    (libc::MS_REMOUNT | libc::MS_RDONLY) as libc::c_ulong,
                    std::ptr::null(),
                );
                if rc != 0 {
                    libc::_exit(11);
                }

                // 2. Install the filter.
                if install_mount_immutability_filter().is_err() {
                    libc::_exit(12);
                }

                // 3. remount,rw must now fail with EPERM.
                let rc = libc::mount(
                    std::ptr::null(),
                    ro_target_c.as_ptr(),
                    std::ptr::null(),
                    libc::MS_REMOUNT as libc::c_ulong,
                    std::ptr::null(),
                );
                if rc == 0 {
                    libc::_exit(20); // BUG: remount,rw succeeded
                }
                if std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
                    libc::_exit(21); // wrong errno
                }

                // 4. remount,ro must still succeed (tightening is safe).
                let rc = libc::mount(
                    std::ptr::null(),
                    ro_target_c.as_ptr(),
                    std::ptr::null(),
                    (libc::MS_REMOUNT | libc::MS_RDONLY) as libc::c_ulong,
                    std::ptr::null(),
                );
                if rc != 0 {
                    libc::_exit(22); // remount,ro unexpectedly denied
                }

                // 5. A fresh (non-remount) mount must be unrestricted.
                let rc = libc::mount(
                    tmpfs_c.as_ptr(),
                    fresh_target_c.as_ptr(),
                    tmpfs_c.as_ptr(),
                    0,
                    std::ptr::null(),
                );
                if rc != 0 {
                    libc::_exit(23); // fresh mount unexpectedly denied
                }

                libc::_exit(0);
            }
        }

        let mut status: libc::c_int = 0;
        // SAFETY: `pid` was just returned by fork() above and has not been
        // waited on yet; `&mut status` is a valid, uniquely-owned local.
        let rc = unsafe { libc::waitpid(pid, &mut status, 0) };
        assert_eq!(rc, pid, "waitpid failed");
        assert!(libc::WIFEXITED(status), "child did not exit normally");
        let code = libc::WEXITSTATUS(status);
        assert_eq!(
            code, 0,
            "child exited with code {code} (see test source for meaning)"
        );

        // Best-effort cleanup: the child's mount namespace was NOT
        // unshared (fork() alone shares it with the parent/host), so the
        // tmpfs mounts above are visible here and must be torn down.
        // SAFETY: `ro_target_c`/`fresh_target_c` are valid, live `CString`s
        // owned by this function; `umount2` return values are intentionally
        // ignored (best-effort cleanup, mirroring `cleanup_bind_mounts`
        // elsewhere in this crate).
        unsafe {
            libc::umount2(ro_target_c.as_ptr(), libc::MNT_DETACH);
            libc::umount2(fresh_target_c.as_ptr(), libc::MNT_DETACH);
        }
    }
}
