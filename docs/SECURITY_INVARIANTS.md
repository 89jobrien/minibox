# Security Invariants

This document maps each critical security invariant to the code path that enforces it and
the regression test that pins the behaviour. If a test listed here starts failing, a
security-critical invariant has been broken.

Reference commits: `8ea4f73` (tar extraction safety), `2fc7036` (symlink rewrite + setuid strip).

Last updated: 2026-05-06

---

## 1. Zip Slip / Path Traversal Prevention

**Invariant:** Tar entries whose paths contain `..` components must be rejected before any
filesystem write occurs. This prevents a malicious OCI layer from writing files outside the
container rootfs (e.g. `../../../etc/cron.d/evil`).

**Code path:**
- `crates/minibox/src/image/layer.rs` — `validate_tar_entry_path()` called for every entry
  before `entry.unpack_in()`.
- Rejects any path containing a `Component::ParentDir` (`..`) component.

**Fixed in:** commit `8ea4f73`

**Regression tests:**
| Test name | File |
|-----------|------|
| `regression_zip_slip_dotdot_prefix_is_rejected` | `crates/minibox/tests/security_regression.rs` |
| `regression_zip_slip_dotdot_in_middle_is_rejected` | `crates/minibox/tests/security_regression.rs` |

---

## 2. Device Node Extraction Rejection

**Invariant:** Tar entries of type `Block` or `Char` (device nodes) must be rejected outright.
Extracting them allows a container image to ship files that grant raw access to host hardware
(disks, serial ports, `/dev/mem`, etc.).

**Code path:**
- `crates/minibox/src/image/layer.rs` — entry type check before `unpack_in`; `Block` and
  `Char` entries return `ImageError::DeviceNodeRejected`.

**Fixed in:** commit `8ea4f73`

**Regression tests:**
| Test name | File |
|-----------|------|
| `regression_block_device_node_is_rejected` | `crates/minibox/tests/security_regression.rs` |
| `regression_char_device_node_is_rejected` | `crates/minibox/tests/security_regression.rs` |

---

## 3. Absolute Symlink Host Leakage Prevention

**Invariant:** Absolute symlink targets (e.g. `/etc/shadow`) must be rewritten to
container-relative paths so they resolve correctly after `pivot_root` without pointing into
the host filesystem. Targets that still contain `..` after relativisation (e.g.
`/../../etc/shadow` → `../../etc/shadow`) must be rejected entirely.

**Code path:**
- `crates/minibox/src/image/layer.rs` — `relative_path(entry_dir, abs_target)` rewrites the
  target; `has_parent_dir_component()` on the rewritten target gates rejection.
- Safe busybox applet symlinks (e.g. `bin/echo -> /bin/busybox`) are rewritten to relative
  form (`busybox`) and accepted.

**Fixed in:** commit `2fc7036`

**Regression tests:**
| Test name | File |
|-----------|------|
| `regression_absolute_symlink_with_traversal_is_rejected` | `crates/minibox/tests/security_regression.rs` |
| `regression_busybox_applet_symlink_is_rewritten_not_rejected` | `crates/minibox/tests/security_regression.rs` |

---

## 4. Setuid / Setgid Bit Stripping

**Invariant:** Regular files extracted from OCI layers must have the setuid (04000), setgid
(02000), and sticky (01000) bits stripped before writing to disk. Setuid binaries in a
container image could allow privilege escalation to root once the container process runs.

**Code path:**
- `crates/minibox/src/image/layer.rs` — `entry.header_mut().set_mode(mode & 0o777)` applied
  before `entry.unpack_in()` for `Regular` and `Link` entries.

**Fixed in:** commit `2fc7036`

**Regression test:**
| Test name | File |
|-----------|------|
| `regression_setuid_bits_stripped_on_extraction` | `crates/minibox/tests/security_regression.rs` |

---

## 5. FD-Leak Prevention in Child Init

**Invariant:** The container child process must close all file descriptors above stderr (FD 2)
before calling `execve`. Leaked FDs from the daemon (sockets, pipes, log handles) would be
visible inside the container, potentially leaking secrets or enabling container breakout via
daemon socket access.

**Code path:**
- `crates/minibox/src/container/process.rs` — `close_extra_fds()` called inside `child_init`
  before `execve`.
- Fast path: `SYS_close_range(3, u32::MAX, 0)` (Linux 5.9+, kernel `close_range(2)` syscall).
- Fallback: iterate `/proc/self/fd`, collect FD numbers into `Vec`, close each with `fd > 2`.

**Regression test:**
| Test name | File |
|-----------|------|
| `regression_close_extra_fds_uses_close_range_syscall` | `crates/minibox/tests/security_regression.rs` |

---

## 6. Environment Isolation — execve Not execvp

**Invariant:** The container child process must use `execve` (explicit `envp` parameter)
rather than `execvp` (inherits the calling process's environment). `execvp` would expose the
daemon's entire environment — including API keys, secrets, and host configuration — inside
every container.

**Code path:**
- `crates/minibox/src/container/process.rs` — `child_init` calls
  `nix::unistd::execve(&cmd, &argv, &envp)` where `envp` is built exclusively from
  `config.env` (caller-supplied container environment variables).
- The host environment (`std::env::vars()`) is never consulted.

**Regression tests:**
| Test name | File |
|-----------|------|
| `regression_child_init_uses_execve_not_execvp` | `crates/minibox/tests/security_regression.rs` |
| `regression_envp_built_from_config_env_only` | `crates/minibox/tests/security_regression.rs` |

---

## 7. SO_PEERCRED Unix Socket Authentication

**Invariant:** The daemon's Unix socket must only accept connections from UID 0 (root) when
`require_root_auth` is enabled. Non-root connections must be rejected before any request
processing occurs.

**Code path:**
- `crates/minibox/src/daemon/server.rs` — `is_authorized(creds, require_root_auth)` is the
  single authorisation predicate.
- `PeerCreds` is populated from `SO_PEERCRED` on the accepted socket.
- Socket file is created with mode `0600` (owner-only read/write).
- Client UID and PID are logged on every connection for audit.

**Behaviour table:**

| `require_root_auth` | `creds`        | Result  |
|---------------------|----------------|---------|
| `false`             | any / `None`   | allowed |
| `true`              | `None`         | allowed (warning logged) |
| `true`              | `Some(uid = 0)`| allowed |
| `true`              | `Some(uid > 0)`| denied  |

**Regression tests:**
| Test name | File |
|-----------|------|
| `root_uid_accepted_when_root_required` | `crates/minibox/tests/daemon_security_regression.rs` |
| `non_root_uid_rejected_when_root_required` | `crates/minibox/tests/daemon_security_regression.rs` |
| `uid_1_rejected_when_root_required` | `crates/minibox/tests/daemon_security_regression.rs` |
| `any_uid_accepted_when_root_not_required` | `crates/minibox/tests/daemon_security_regression.rs` |
| `missing_creds_accepted_when_root_not_required` | `crates/minibox/tests/daemon_security_regression.rs` |
| `missing_creds_still_allowed_through_when_root_required` | `crates/minibox/tests/daemon_security_regression.rs` |
| `max_uid_rejected_when_root_required` | `crates/minibox/tests/daemon_security_regression.rs` |

---

## 8. Tar Root Entry Skip

**Invariant:** The tar archive root marker entries `"."` and `"./"` must be silently skipped
rather than passed to path validation. Without this skip, `validate_tar_entry_path` produces
a false-positive path-escape error because `Path::join("./")` normalises away the `CurDir`
component.

**Code path:**
- `crates/minibox/src/image/layer.rs` — explicit check for `"."` and `"./"` before
  `validate_tar_entry_path`.

**Regression test:**
| Test name | File |
|-----------|------|
| `regression_root_dot_entries_are_silently_skipped` | `crates/minibox/tests/security_regression.rs` |

---

## 9. FIFO / Named Pipe Non-Crash Guarantee

**Invariant:** Tar entries of type `Fifo` (named pipe) must not cause a panic or corrupt
daemon state. FIFOs are not explicitly rejected like device nodes, but extraction must
complete without crashing.

**Code path:**
- `crates/minibox/src/image/layer.rs` — FIFO entries fall through to `unpack_in`; behaviour
  is platform-dependent but must not panic.

**Regression test:**
| Test name | File |
|-----------|------|
| `regression_fifo_entry_does_not_crash` | `crates/minibox/tests/security_regression.rs` |

---

## Request Size Limit

**Invariant:** Daemon request reads are bounded to prevent memory exhaustion from malicious
clients sending oversized JSON.

**Code path:**
- `crates/minibox/src/daemon/server.rs` — `MAX_REQUEST_SIZE = 1_048_576` (1 MB) enforced
  before deserialisation.

*(No dedicated regression test — enforced by the constant and the `read_line` size check in
`serve_connection`.)*

---

## Image Pull Resource Limits

**Invariant:** Image pull operations enforce per-resource limits to prevent DoS via
oversized manifest or layer downloads.

**Code path:**
- `crates/minibox/src/image/registry.rs`:
  - Max manifest size: 10 MB
  - Max layer size: 1 GB per layer
  - Total image size limit: 5 GB

*(No dedicated regression test — enforced by constants checked during streaming reads.)*

---

## Newtype Wrappers for Validated Paths (Consideration)

The acceptance criteria for issue #157 mention considering newtype wrappers for validated
paths. The current implementation uses free functions (`validate_tar_entry_path`,
`validate_layer_path`) rather than newtypes.

A `ValidatedPath` newtype would make it impossible to call `entry.unpack_in()` with an
unvalidated path at the type level. This is a worthwhile future hardening step but is not
required for the current invariant regression suite.

If implemented, the newtype should live in `crates/minibox/src/image/layer.rs` (for tar
paths) and `crates/minibox/src/container/filesystem.rs` (for overlay paths), with
`TryFrom<&Path>` as the validated constructor.
