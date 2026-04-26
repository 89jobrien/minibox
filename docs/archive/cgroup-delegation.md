> **ARCHIVED** — This document is not authoritative. See the current docs in the repo root.

# cgroup v2 Delegation for miniboxd

This document explains how miniboxd is wired into the systemd cgroup hierarchy, why the
"supervisor leaf" pattern is required, and what breaks if you deviate from it.

## Background: cgroup v2 No-Internal-Process Rule

cgroup v2 enforces a hard constraint: **a cgroup that contains at least one process cannot
enable `subtree_control` for its children.** This is the "no internal process" rule.

Consequence: if miniboxd lives in the same cgroup where it tries to create container children,
the kernel rejects all writes to `pids.max`, `memory.max`, `cpu.weight`, `io.max`, etc. for
those children. The error is `Permission denied` or `I/O error` on the `cgroup.subtree_control`
write, not on the resource file directly.

## Why the Naive Approaches Fail

### Attempt 1: Enable controllers on the service cgroup

Writing `+pids +memory +cpu +io` to the service cgroup's `cgroup.subtree_control` while the
daemon process lives there fails. The daemon process IS in that cgroup, so the kernel's
no-internal-process rule blocks the write (or allows the write but marks children as
`domain invalid`, making further delegation impossible).

### Attempt 2: `DelegateSubgroup=yes` in the systemd unit

`DelegateSubgroup` takes a subgroup **name**, not a boolean. Setting it to `yes` creates a
literal subgroup at `.../miniboxd.service/yes`. The daemon is moved into `/yes`. But `/yes`
now contains the daemon process, so writing `+pids` to `/yes/cgroup.subtree_control` fails
with `I/O error` — same root cause, one level deeper.

## The Fix: Supervisor Leaf Pattern

The standard solution (used by containerd, slurm, and described in the systemd cgroup
delegation docs) is to separate the daemon process from the cgroup subtree it manages.

```
/minibox.slice/miniboxd.service/        ← inner node, NO processes
    cgroup.subtree_control              ← +pids +memory +cpu +io
    supervisor/                         ← daemon PID lives here (leaf)
    {container_id}/                     ← container cgroup (leaf)
```

- The service cgroup is a pure **inner node** — no processes, free to enable controllers.
- `supervisor/` is a **leaf** — holds only the daemon PID, never creates children.
- Container cgroups are **siblings** of `supervisor/`, not children of it.

This satisfies the no-internal-process constraint at every level.

## Implementation

### systemd unit (`ops/miniboxd.service`)

```ini
[Service]
DelegateSubgroup=supervisor
Environment=MINIBOX_CGROUP_ROOT=/sys/fs/cgroup/minibox.slice/miniboxd.service
```

`DelegateSubgroup=supervisor` (requires systemd >= 254) tells systemd to place the daemon
process in `.../miniboxd.service/supervisor/` automatically at startup. The service cgroup
itself remains empty.

`MINIBOX_CGROUP_ROOT` points to the service cgroup, not the supervisor subgroup. Container
cgroups are created directly under the service cgroup as siblings of `supervisor/`.

### Runtime fallback (`crates/miniboxd/src/main.rs`)

`migrate_to_supervisor_cgroup()` runs at daemon startup:

1. Reads `/proc/self/cgroup` to find the current cgroup path.
2. Creates a `supervisor/` child cgroup if it does not exist.
3. Writes the daemon's own PID to `supervisor/cgroup.procs`.

This is a no-op if systemd's `DelegateSubgroup` already placed the daemon in `supervisor/`.
It exists as a fallback for environments where `DelegateSubgroup` is unavailable (systemd
< 254) or where the unit file was not reloaded.

**cgroup v1 hosts:** `migrate_to_supervisor_cgroup()` detects whether it is running under
a cgroup v2 unified hierarchy by checking `/proc/self/cgroup`. On a v1 or hybrid mount it
silently returns without migrating. This means resource limits (`pids.max`, `memory.max`,
etc.) have **no effect** on cgroup v1 hosts — miniboxd requires a cgroup v2 unified
hierarchy to enforce container limits.

### Controller enablement (`crates/minibox/src/container/cgroups.rs`)

`enable_subtree_controllers()` writes `+pids +memory +cpu +io` to
`{MINIBOX_CGROUP_ROOT}/cgroup.subtree_control` before any container cgroup is created.
It is idempotent and non-fatal if a controller is unavailable on the host kernel.

## Verification

After deploying, confirm the hierarchy is correct:

```bash
# Daemon should be inside supervisor/, not directly in the service cgroup
sudo systemctl status miniboxd --no-pager
# Look for: CGroup: /minibox.slice/miniboxd.service/supervisor

# Service cgroup should have controllers enabled
sudo cat /sys/fs/cgroup/minibox.slice/miniboxd.service/cgroup.subtree_control
# Expected: cpu io memory pids

# Smoke test: run a container
sudo /usr/local/bin/minibox run alpine -- /bin/true
```

## Key Constraints

| Constraint                                                                           | Why                                                                            |
| ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------ |
| `DelegateSubgroup=supervisor` requires systemd >= 254                                | Older systemd doesn't support the named subgroup directive                     |
| `MINIBOX_CGROUP_ROOT` must point to the **service** cgroup, not `supervisor/`        | Containers are siblings of `supervisor/`, not children of it                   |
| `supervisor/` must never create child cgroups                                        | It is a leaf; adding children would trigger the no-internal-process rule again |
| `enable_subtree_controllers()` must run before the first container cgroup is created | Controllers cannot be enabled after a child cgroup exists with processes       |

## Related Code

| File                                      | Role                                                       |
| ----------------------------------------- | ---------------------------------------------------------- |
| `ops/miniboxd.service`                    | systemd unit — `DelegateSubgroup` + `MINIBOX_CGROUP_ROOT`  |
| `crates/miniboxd/src/main.rs`             | `migrate_to_supervisor_cgroup()` runtime fallback          |
| `crates/minibox/src/container/cgroups.rs` | `enable_subtree_controllers()`, per-container cgroup setup |
| `docs/archive/cgroup-findings.md`         | Full debugging log with timeline and evidence              |
