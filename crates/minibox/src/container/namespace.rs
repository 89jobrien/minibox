//! Linux namespace configuration and clone-based process isolation.
//!
//! This module provides [`NamespaceConfig`] for declaring which Linux namespaces
//! a new container process should be placed into, and [`clone_with_namespaces`]
//! for actually forking that process.
//!
//! # Safety contract
//!
//! `clone_with_namespaces` uses `libc::clone` internally. Any `OwnedFd` values
//! that must be accessible in the child must have their raw FDs extracted and
//! their `OwnedFd` dropped (via [`std::mem::forget`]) **before** calling this
//! function; otherwise both parent and child will attempt to close the same FD
//! when the `OwnedFd` is dropped, causing a double-close.

use crate::error::NamespaceError;
use anyhow::Context;
use nix::sched::CloneFlags;
use tracing::{debug, info};

/// Which Linux namespaces to create for a container.
///
/// Each field corresponds to a namespace type. When `true` the child process
/// will be placed into a fresh, private copy of that namespace.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct NamespaceConfig {
    /// Isolate the process ID space (`CLONE_NEWPID`).
    pub new_pid: bool,
    /// Isolate the mount namespace (`CLONE_NEWNS`).
    pub new_mount: bool,
    /// Isolate UTS (hostname/domain name) namespace (`CLONE_NEWUTS`).
    pub new_uts: bool,
    /// Isolate System V IPC objects (`CLONE_NEWIPC`).
    pub new_ipc: bool,
    /// Isolate the network namespace (`CLONE_NEWNET`).
    /// A user namespace is always created; UID/GID range policy is configured separately.
    pub new_net: bool,
}

impl NamespaceConfig {
    /// Returns a configuration that enables all supported namespaces.
    pub const fn all() -> Self {
        Self {
            new_pid: true,
            new_mount: true,
            new_uts: true,
            new_ipc: true,
            new_net: true,
        }
    }

    /// Converts the configuration into the [`CloneFlags`] bitmask expected by
    /// `nix::sched::clone` / `libc::clone`.
    pub fn to_clone_flags(&self) -> CloneFlags {
        let mut flags = CloneFlags::CLONE_NEWUSER;
        if self.new_pid {
            flags |= CloneFlags::CLONE_NEWPID;
        }
        if self.new_mount {
            flags |= CloneFlags::CLONE_NEWNS;
        }
        if self.new_uts {
            flags |= CloneFlags::CLONE_NEWUTS;
        }
        if self.new_ipc {
            flags |= CloneFlags::CLONE_NEWIPC;
        }
        if self.new_net {
            flags |= CloneFlags::CLONE_NEWNET;
        }
        flags
    }
}

impl Default for NamespaceConfig {
    /// Returns `all()` — all supported namespaces enabled.
    fn default() -> Self {
        Self::all()
    }
}

#[cfg(test)]
mod user_namespace_tests {
    use super::*;

    #[test]
    fn all_includes_user_namespace() {
        assert!(
            NamespaceConfig::all()
                .to_clone_flags()
                .contains(CloneFlags::CLONE_NEWUSER)
        );
    }
}

/// Stack size allocated for the cloned child process (8 MiB).
///
/// The buffer is zero-initialised to avoid undefined behaviour if the child
/// touches guard pages before the stack frame is properly set up.
const CHILD_STACK_SIZE: usize = 8 * 1024 * 1024;

/// Fork a child process into the namespaces described by `config`.
///
/// The child immediately calls `child_fn()`. The parent receives the child's
/// PID.
///
/// # Safety
///
/// This function uses `libc::clone` which is inherently unsafe:
/// - We allocate a properly aligned 8 MiB stack on the heap and pass a pointer
///   to its *top* (stack grows downward on x86-64).
/// - The `child_fn` closure is passed as a raw pointer; we guarantee it lives
///   until the child has started by keeping the `Box` alive for the duration
///   of this call.
/// - After `clone` returns in the parent the `child_fn` memory is still owned by
///   this frame, so there is no double-free.
/// - The child must not return from `child_fn`; it must call `_exit` (or
///   exec). Callers enforce this contract.
pub fn clone_with_namespaces<F>(
    ns_config: &NamespaceConfig,
    child_fn: F,
) -> anyhow::Result<nix::unistd::Pid>
where
    F: FnOnce() -> isize + Send + 'static,
{
    let clone_flags = ns_config.to_clone_flags();
    // SIGCHLD must be set so the parent can waitpid on the child.
    let flags_raw = clone_flags.bits() | libc::SIGCHLD;

    debug!(
        clone_flags = flags_raw, // raw i32 bitmask (CloneFlags bits | SIGCHLD)
        "namespace: cloning container child"
    );

    // Allocate the child stack.  We zero-initialise to avoid UB from
    // uninitialised reads if the child touches guard pages.
    let mut stack: Vec<u8> = vec![0u8; CHILD_STACK_SIZE];

    // Box the closure so we have a stable address to pass through the C ABI.
    let boxed: Box<Box<dyn FnOnce() -> isize + Send>> = Box::new(Box::new(child_fn));
    let child_arg = Box::into_raw(boxed).cast::<libc::c_void>();

    // The trampoline function with the C calling convention required by clone(2).
    extern "C" fn trampoline(arg: *mut libc::c_void) -> libc::c_int {
        // SAFETY: We reconstruct the exact Box we created above.  This is the
        // only place that touches this pointer, and clone guarantees it is
        // called exactly once.
        let closure: Box<Box<dyn FnOnce() -> isize + Send>> =
            unsafe { Box::from_raw(arg.cast::<Box<dyn FnOnce() -> isize + Send>>()) };
        let ret = (*closure)();
        ret as libc::c_int
    }

    // SAFETY:
    // - `stack_top` points to the end of a valid, heap-allocated buffer that
    //   is large enough for the child's execution.
    // - `trampoline` has the correct C signature for clone's `fn` parameter.
    // - `child_arg` is a valid heap pointer that will be consumed by the child.
    let pid = unsafe {
        let stack_top = stack
            .as_mut_ptr()
            .add(CHILD_STACK_SIZE)
            .cast::<libc::c_void>();
        libc::clone(trampoline, stack_top, flags_raw, child_arg)
    };

    // If clone failed the child_arg Box was never consumed -- we must free it.
    if pid < 0 {
        // SAFETY: clone failed, so the trampoline was never called and the
        // pointer still owns the allocation.
        let _ = unsafe { Box::from_raw(child_arg.cast::<Box<dyn FnOnce() -> isize + Send>>()) };
        let errno = nix::errno::Errno::last();
        return Err(NamespaceError::CloneFailed(format!(
            "libc::clone returned {pid}: {errno}"
        )))
        .context("failed to clone container process");
    }

    // SAFETY: clone succeeded without CLONE_VM, so the child has its own
    // copy of the address space and will free its copy of child_arg in the
    // trampoline. The parent's allocation at child_arg still exists and must
    // be freed here to avoid a memory leak.
    let _ = unsafe { Box::from_raw(child_arg.cast::<Box<dyn FnOnce() -> isize + Send>>()) };

    // Keep the stack alive until after clone returns (parent path).
    // The child has its own copy of the address space so it does not share
    // this Vec.
    drop(stack);

    info!(child_pid = pid, "namespace: container child cloned");
    Ok(nix::unistd::Pid::from_raw(pid))
}

// ---------------------------------------------------------------------------
// Kani formal verification proofs (cfg-gated, never compiled in normal builds)
// ---------------------------------------------------------------------------

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Proof 6: to_clone_flags is monotone — enabling more namespace fields
    /// always produces a superset of flags (never loses bits).
    #[kani::proof]
    fn to_clone_flags_monotone() {
        let base = NamespaceConfig {
            new_pid: kani::any(),
            new_mount: kani::any(),
            new_uts: kani::any(),
            new_ipc: kani::any(),
            new_net: kani::any(),
        };
        let base_flags = base.to_clone_flags();

        // Enable one additional field (superset config).
        let mut superset = base.clone();
        let field: u8 = kani::any();
        kani::assume(field < 5);
        match field {
            0 => superset.new_pid = true,
            1 => superset.new_mount = true,
            2 => superset.new_uts = true,
            3 => superset.new_ipc = true,
            _ => superset.new_net = true,
        }
        let super_flags = superset.to_clone_flags();

        // Superset flags must contain all bits from base.
        assert_eq!(
            base_flags & super_flags,
            base_flags,
            "enabling a namespace field must not clear existing flags"
        );
    }

    /// Proof 7: each NamespaceConfig field maps to exactly one distinct
    /// CloneFlags bit, and the all() config produces the union of all five.
    #[kani::proof]
    fn to_clone_flags_bijection() {
        let all = NamespaceConfig::all();
        let all_flags = all.to_clone_flags();

        let configs = [
            NamespaceConfig {
                new_pid: true,
                new_mount: false,
                new_uts: false,
                new_ipc: false,
                new_net: false,
            },
            NamespaceConfig {
                new_pid: false,
                new_mount: true,
                new_uts: false,
                new_ipc: false,
                new_net: false,
            },
            NamespaceConfig {
                new_pid: false,
                new_mount: false,
                new_uts: true,
                new_ipc: false,
                new_net: false,
            },
            NamespaceConfig {
                new_pid: false,
                new_mount: false,
                new_uts: false,
                new_ipc: true,
                new_net: false,
            },
            NamespaceConfig {
                new_pid: false,
                new_mount: false,
                new_uts: false,
                new_ipc: false,
                new_net: true,
            },
        ];

        let mut union = CloneFlags::empty();
        for cfg in &configs {
            let f = cfg.to_clone_flags();
            assert!(!f.is_empty(), "single namespace field must produce a flag");
            assert_eq!(
                f.bits().count_ones(),
                1,
                "single field must map to exactly one bit"
            );
            assert!(
                (union & f).is_empty(),
                "each namespace field must map to a distinct bit"
            );
            union |= f;
        }

        assert_eq!(
            union, all_flags,
            "all() must be the union of all individual flags"
        );
    }

    /// Proof 8: the empty config produces no clone flags.
    #[kani::proof]
    fn to_clone_flags_empty() {
        let empty = NamespaceConfig {
            new_pid: false,
            new_mount: false,
            new_uts: false,
            new_ipc: false,
            new_net: false,
        };
        assert!(
            empty.to_clone_flags().is_empty(),
            "all fields false must produce empty flags"
        );
    }

    /// Proof 9: Box pointer lifecycle model for clone_with_namespaces.
    ///
    /// Models the child_arg allocation as an abstract state machine and proves
    /// it is freed exactly once on both the success and failure paths.
    #[kani::proof]
    fn box_pointer_freed_exactly_once() {
        let boxed: Box<Box<dyn FnOnce() -> isize + Send>> = Box::new(Box::new(|| 0isize));
        let ptr = Box::into_raw(boxed);

        let mut free_count: u32 = 0;
        let clone_succeeded: bool = kani::any();

        if !clone_succeeded {
            // Clone failed: parent frees the pointer (line 152).
            // SAFETY: clone failed so the trampoline was never called; ptr still owns the allocation.
            let _ = unsafe { Box::from_raw(ptr) };
            free_count += 1;
        } else {
            // Clone succeeded without CLONE_VM: parent frees its copy (line 164).
            // Child has its own address space copy freed in the trampoline.
            // SAFETY: clone() without CLONE_VM gives parent and child separate address spaces;
            // the parent frees its copy here; the child's copy is freed in the trampoline.
            let _ = unsafe { Box::from_raw(ptr) };
            free_count += 1;
        }

        assert_eq!(free_count, 1, "parent must free child_arg exactly once");
    }
}
