---
source_sha: f9aa9a79348227ca3c517fe7ea86b3bc6eb16829
status: proposed
sources:
  - crates/minibox/src/container/process.rs
  - crates/minibox/src/container/namespace.rs
  - crates/minibox/src/container/mount_seccomp.rs
  - crates/minibox/src/adapters/runtime.rs
  - crates/minibox/src/daemon/handler/run/preparation.rs
  - crates/minibox/tests/native_adapter_isolation_tests.rs
  - crates/minibox/tests/security_regression.rs
  - docs/designs/2026-09-21-isolation-profile-contract-design.md
  - docs/designs/2026-09-21-isolation-admission-lifecycle-design.md
generated: 2026-09-21
---

# Design: Native `standard-v1` Enforcement

## Goal

Implement one complete Linux-native `standard-v1` runtime that applies all nine controls, stages a
blocked subject, and reports kernel observations before workload release.

## Ownership

- **Owner:** `minibox`
- **Aggregate adapter:** proposed `crates/minibox/src/adapters/isolation.rs`
- **Kernel modules:** `container/process.rs`, `container/namespace.rs`, and seccomp profile modules
- **Dependencies:** profile contract and admission lifecycle

## Enforcement Order

1. Create PID, mount, UTS, IPC, network, and user namespaces required by the plan.
2. Parent writes UID/GID maps and records their canonical digests.
3. Create cgroup and apply pids, memory, CPU, and I/O limits.
4. Create rootfs and bind mounts through descriptor-relative, race-resistant path resolution.
5. Configure the requested non-host network mode.
6. Pivot root and apply filesystem restrictions.
7. Resolve `cap_last_cap`, set exact effective/permitted sets, clear inheritable/ambient sets, and
   drop every unselected bounding capability.
8. Set `no_new_privileges`.
9. Install the pinned `standard-v1` seccomp program and mount-immutability ratchet.
10. Read back every control from kernel state.
11. Send evidence plus process identity to the parent and block on a private release latch.
12. Parent validates; only then may the child close excess FDs and call `execve`.

## Identity

`MappedRoot` requires container UID/GID zero mapped to nonzero host IDs. `NonRoot` requires a
nonzero effective UID. Native `IsolatedEquivalent` is not accepted because the local kernel path
can provide direct user-namespace evidence.

Evidence includes:

- Effective UID/GID.
- Host UID/GID.
- Canonical `/proc/<pid>/uid_map` and `gid_map` digests.
- User namespace inode.
- Process PID and `/proc/<pid>/stat` start time.

## Linux Capabilities

The resolved profile provides exact effective, permitted, inheritable, ambient, and bounding sets.
The adapter rejects additions unavailable in the daemon's permitted/bounding ceiling or above
`cap_last_cap`.

The Docker baseline includes `CAP_MKNOD`, but `standard-v1` independently blocks device-node
creation through syscall and filesystem policy. Capability evidence comes from `/proc/<pid>/status`
and `prctl` readback before release.

Negative tests prove dropped capabilities cannot be regained through:

- Ambient capabilities.
- File capabilities.
- Setid binaries.
- Descendant processes.
- Nested user namespaces.

## Syscall Policy

Two proposed checked-in artifacts are normative:

- `crates/minibox/src/container/seccomp_profiles/standard-v1.json`
- `crates/minibox/src/container/seccomp_profiles/compatibility-v1.json`

The standard policy is default-deny. It allows operation classes `BasicIo`, `Filesystem`,
`Process`, `Network`, and `Ipc`; it denies `ClockAdmin`, `NamespaceAdmin`, `MountAdmin`,
`KernelAdmin`, `ModuleAdmin`, and `Trace` after staged initialization.

The artifact expands those classes to architecture-specific syscall numbers. Canonical artifact
JSON is hashed into `SyscallFilterPolicy.program_digest`; BPF installation and evidence must match
that digest. Native qualification is impossible until the artifact and golden digest exist.

The compatibility policy is default-allow with the existing mount-immutability ratchet. It remains
operator-authorized and does not qualify standard execution.

## Filesystem And Mounts

- `HostMountPolicy::Deny` rejects every bind mount.
- Allowlisted mounts resolve from already-open directory descriptors; string canonicalization alone
  is insufficient because of symlink races.
- Prefix matching is component-aware.
- `ReadOnlyOnly` rejects writable requests.
- `ReadWrite` authorizes writable access only under an explicit prefix.
- Device nodes and setid bits remain denied regardless of host-mount access.
- Mount namespace identity and canonical mountinfo digest are recorded before release.

## Network

`standard-v1` rejects host network mode. Supported modes must produce a separate network boundary
or a profile-defined no-network state. Evidence records requested mode, actual mode, namespace
inode, and configuration digest.

## Resources

- `pids_max` defaults to 1024.
- Every requested memory, CPU, and I/O value must be present in cgroup readback.
- Missing controller support is pre-mutation rejection, not degradation.
- Cgroup path and controller-file digest are bound to evidence.

## Filesystem Evidence

Evidence records:

- Rootfs read-only state.
- Device-node denial policy.
- Setid-bit denial policy.
- Mount table digest.
- Rootfs/layer digest binding.

## Staged Subject

The child remains blocked before `execve`. The parent opens a pidfd where available and records PID,
start time, namespace inodes, and evidence digest. Release verifies the same subject and consumes the
validated typestate; it cannot create a new `SpawnResult` for another PID.

## Rollback

The native aggregate adapter tracks rootfs, cgroup, network, child, and hook allocations. Stage
failure either proves complete rollback or returns `CleanupRequired` with a durable handle and exact
allocated controls. Cleanup is idempotent and evidence-producing.

## Verification

- Linux root integration tests inspect capability sets, UID/GID maps, namespace inodes,
  `NoNewPrivs`, seccomp mode, mountinfo, networking, and cgroup files.
- Release-latch tests prove workload code cannot run before evidence validation.
- Failure injection covers every step and rollback edge.
- Property tests compare resolved policy values to observations.
- Musl and supported architecture builds compile the same pinned syscall artifacts.

## Qualification Gate

The native adapter advertises `standard-v1` only after all nine controls pass real-kernel tests.
Compilation, mocks, compatibility execution, or partial evidence never qualifies it.

## Out Of Scope

- VM guest agents and hypervisor evidence.
- Protocol migration.
- Signed or remotely attested evidence.
