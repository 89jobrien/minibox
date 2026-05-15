# State Persistence Model

How minibox tracks container state across daemon restarts.

Last updated: 2026-05-14

---

## Overview

`DaemonState` (defined in `crates/minibox/src/daemon/state.rs`) is the single
shared data structure for all container metadata. It is held behind
`Arc<RwLock<...>>` (verified against source 2026-05-06) so many readers
proceed concurrently while writes are exclusive.

State is persisted to a JSON file after **every mutation** (add, remove, state
transition, PID assignment). The file is written atomically via rename so
readers never see a partial write.

## What Survives a Restart

| Data                                                                  | Survives? | Notes                                                  |
| --------------------------------------------------------------------- | --------- | ------------------------------------------------------ |
| Container records (ID, image, command, creation time)                 | Yes       | Loaded from `state.json` on startup                    |
| Container state (Created, Running, Paused, Stopped, Failed, Orphaned) | Yes       | Adjusted on load — see below                           |
| Host PID                                                              | No        | PIDs are not valid across restarts                     |
| Overlay mount                                                         | No        | Mount namespace is gone after daemon exit              |
| Cgroup tree                                                           | No        | Cgroup dirs may persist on disk but are not reattached |
| Allocated bridge IPs                                                  | No        | `allocated_ips` map is in-memory only                  |

## State File Location

- Default: `$MINIBOX_DATA_DIR/state.json`
    - Linux root: `/var/lib/minibox/state.json`
    - Linux non-root: `~/.minibox/cache/state.json`
    - macOS: `~/Library/Application Support/minibox/state.json`
      (falls back to `/tmp/minibox/state.json` in sandboxed/CI environments)
- Override: set `MINIBOX_DATA_DIR` explicitly (takes precedence on all platforms)

## Atomic Write Protocol

1. Serialise container map to pretty-printed JSON.
2. Write to `state.json.tmp` (sibling file).
3. Set permissions to `0o600` (owner-only, POSIX only).
4. `rename(state.json.tmp, state.json)` — atomic on POSIX filesystems.

Failures at any step are logged as warnings but do not crash the daemon.
State writes are best-effort.

## Startup Reconciliation

On daemon start, `load_from_disk()` followed by `reconcile_on_startup()`
adjusts stale records:

| Previous state        | Action on reload  | Rationale                                         |
| --------------------- | ----------------- | ------------------------------------------------- |
| `Created`             | Set to `Stopped`  | Process was never forked or fork did not complete |
| `Paused`              | Set to `Stopped`  | Cgroup freeze is lost after daemon exit           |
| `Running` (PID alive) | Left as `Running` | Process survived daemon restart                   |
| `Running` (PID dead)  | Set to `Orphaned` | Process exited while daemon was down              |
| `Stopped`             | Unchanged         | Already terminal                                  |
| `Failed`              | Unchanged         | Already terminal                                  |
| `Orphaned`            | Unchanged         | Already terminal                                  |

The PID liveness check is performed by the `ProcessChecker` port (default
adapter uses `kill(pid, 0)`). Tests inject doubles that always return
alive or dead.

## Container State Machine

```
Created ──► Running ──► Stopped
              │   │
              │   └──► Failed
              │
              ├──► Paused ──► Running  (resume)
              │           └──► Stopped
              │
              └── (daemon restart, PID dead) ──► Orphaned
```

Valid transitions are enforced by `update_container_state()`. Invalid
transitions return an error.

## Persistence Port

The `StateRepository` trait abstracts persistence:

```rust
pub trait StateRepository: Send + Sync + 'static {
    fn load_containers(&self) -> Result<HashMap<String, ContainerRecord>>;
    fn save_containers(&self, containers: &HashMap<String, ContainerRecord>) -> Result<()>;
}
```

- **Production adapter**: `JsonFileRepository` — atomic JSON file.
- **Test adapter**: in-memory doubles or `TempDir`-backed `DaemonState`.

`DaemonState::with_repository()` accepts an `Arc<dyn StateRepository>` for
dependency injection. This is the preferred constructor for tests. The production
daemon uses `DaemonState::new()`, which operates directly on the raw `state_file`
path without a repository; both paths coexist by design (not an in-progress
migration).

## What Is NOT Persisted

- **Running processes**: PIDs are recorded but processes are not reattached.
  A container marked `Running` after reload may have its process still alive
  (checked by `reconcile_on_startup`) but the daemon does not re-enter its
  reaper loop. The container will appear as `Orphaned` if the PID dies later.
- **Network state**: Bridge IP allocations, veth pairs, and iptables rules
  are ephemeral. Containers lose network connectivity after a daemon restart.
- **Mount state**: Overlay mounts are gone. The layer directories on disk
  remain, but the merged view is not recreated.

## Security

- State file is restricted to `0o600` (owner-read/write only).
- Contains PIDs and rootfs paths — should not be world-readable.
- `SO_PEERCRED` auth on the daemon socket prevents unauthorized state queries.
