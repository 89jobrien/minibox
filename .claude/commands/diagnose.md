---
name: diagnose
description: >
  Use when a container fails to start, crashes unexpectedly, or produces unexpected output.
  Gathers daemon logs, cgroup state, overlay mounts, and runtime files, then diagnoses the
  failure cause.
disable-model-invocation: false
---

# diagnose — Container Failure Diagnosis

Systematic diagnosis for container failures. Works on Linux with a running or recently-crashed
miniboxd instance.

## Step 1 — Run the diagnosis script

```bash
# Diagnose most recent failure
nu scripts/diagnose.nu

# Focus on a specific container
nu scripts/diagnose.nu --container <container_id>

# Fetch more log lines
nu scripts/diagnose.nu --lines 500
```

The script collects: journalctl daemon logs, overlay mount state, cgroup hierarchy, and
runtime state files.

## Step 2 — Interpret common failure patterns

| Symptom | Likely cause | Investigation |
|---|---|---|
| `pivot_root: EINVAL` | Missing `MS_PRIVATE` remount before pivot | Check `filesystem.rs` — `mount("", "/", MS_REC\|MS_PRIVATE)` must run in child |
| `execvp: ENOENT` | Command not found in container rootfs | Verify image layers extracted; check `MINIBOX_DATA_DIR` |
| `cgroup: permission denied` | Not running as root, or cgroup path wrong | `MINIBOX_CGROUP_ROOT` env var; check `/sys/fs/cgroup/minibox.slice/` exists |
| `overlay: invalid argument` | Layers on different filesystems | upper/work must be on same FS as lower |
| Container exits immediately | Missing entrypoint or bad command | Check `config.env` and command field in `RunContainer` request |
| `CLONE_NEWUSER: operation not permitted` | Kernel config missing `CONFIG_USER_NS` | Check `/proc/sys/kernel/unprivileged_userns_clone` |

## Step 3 — Manual log inspection

```bash
# Daemon logs (requires root or journal access)
journalctl -u miniboxd -n 200 --no-pager

# Check overlay mounts
mount | grep minibox

# Inspect cgroup state for container
ls /sys/fs/cgroup/minibox.slice/miniboxd.service/<container_id>/
cat /sys/fs/cgroup/minibox.slice/miniboxd.service/<container_id>/memory.events

# Check runtime state file
ls /run/minibox/containers/<container_id>/
```

## Step 4 — Enable verbose daemon logging

```bash
RUST_LOG=debug sudo ./target/release/miniboxd
```

Reproduce the failure with debug logging active. Look for:
- `"container: process started"` — fork succeeded
- `"pivot_root: complete"` — rootfs switch succeeded
- `"container: exec"` — exec succeeded
- Any `error!` or `warn!` events with `container_id` field

## Step 5 — Cleanup stuck state

If the container is stuck and the daemon can't recover:

```bash
# Kill orphan processes
sudo cargo xtask nuke-test-state

# Unmount stuck overlays manually
mount | grep minibox | awk '{print $3}' | xargs -r sudo umount -l

# Remove stale cgroup
sudo rmdir /sys/fs/cgroup/minibox.slice/miniboxd.service/<container_id>/
```

## Key Rules

- **Check cgroup delegation first** — most Linux container failures are cgroup permission issues
- **`pivot_root` failures are always mount namespace ordering** — see container init gotchas in CLAUDE.md
- **Use `RUST_LOG=debug`** — structured tracing fields (`container_id`, `pid`, `rootfs`) make
  the failure location obvious
- **`nuke-test-state` is safe to run** — it only affects minibox-managed state, not system state
