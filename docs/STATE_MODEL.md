# State Persistence Model

This document clarifies the actual persistence behaviour of `DaemonState` as implemented in
`crates/daemonbox/src/state.rs`. It supersedes the stale note in CLAUDE.md that said "daemon
restart loses all records."

## Summary

Container records **are persisted to disk** after every mutation. The daemon reloads them on
startup. The earlier "in-memory only" description was accurate for an older version of the code
and is now incorrect.

---

## What Is Persisted

`DaemonState` serialises a `HashMap<String, ContainerRecord>` to JSON after every write
operation. Each `ContainerRecord` contains:

- Container metadata (`ContainerInfo`): id, name, image, command, state string, creation time,
  PID snapshot
- `pid` — host-namespace PID at time of last write (may be stale after restart)
- `rootfs_path` — path to the merged overlay directory used as the container rootfs
- `cgroup_path` — path to the container's cgroup directory
- `post_exit_hooks` — host-side commands to run after the container process exits
- `rootfs_metadata` — writable-layer overlay metadata (None for GKE/VZ adapters)
- `source_image_ref` — image reference used to create the container (e.g. `"alpine:latest"`)

### What Is NOT Persisted

- **IP allocations** (`allocated_ips`): the bridge-network IP map is in-memory only and is
  not written to disk. Bridge-assigned IPs are lost on daemon restart.
- **PTY sessions**: in-memory only; not serialised.
- **Event broker subscriptions**: in-memory only.
- **Image layer contents**: managed separately by `ImageStore`, not by `DaemonState`.

---

## Write Trigger Points

`save_to_disk()` is called (as an async best-effort operation) after every state mutation:

| Method                   | Trigger                                          |
| ------------------------ | ------------------------------------------------ |
| `add_container`          | New container registered                         |
| `remove_container`       | Container removed (`minibox rm`)                 |
| `update_container_state` | State transition (Running→Stopped, Paused, etc.) |
| `set_container_pid`      | PID recorded after fork                          |

Writes are **not batched**. Each mutation causes one synchronous JSON serialisation and one
filesystem write+rename cycle.

---

## Storage Location

The state file is named `state.json` and lives in the data directory:

| Context                   | Path                           |
| ------------------------- | ------------------------------ |
| Root daemon (default)     | `/var/lib/minibox/state.json`  |
| Non-root daemon (default) | `~/.minibox/cache/state.json`  |
| Explicit override         | `$MINIBOX_DATA_DIR/state.json` |

---

## What Survives a Daemon Restart

`DaemonState::load_from_disk()` is called during startup (see `miniboxd/src/main.rs`).

**Survives restart:**

- All container records (id, name, image, command, rootfs/cgroup paths, source image ref)
- `Stopped` container records — visible in `minibox ps` after restart

**Does not survive restart:**

- Running/Created/Paused container state: all such containers are marked `Stopped` on load,
  because their processes are gone. The `pid` field is cleared to `None`.
- Bridge-network IP allocations (`allocated_ips`)

---

## Crash-Consistency Guarantees

Writes use an atomic rename pattern:

1. JSON is written to `state.json.tmp` (a sibling temp file)
2. File permissions are set to `0o600` (owner-only)
3. `rename(state.json.tmp, state.json)` is called

On POSIX filesystems, `rename(2)` is atomic with respect to readers: a reader either sees the
old complete file or the new complete file, never a partial write.

**Gaps and limitations:**

- **No fsync**: `save_to_disk` does not call `fsync` before rename. On a crash (kernel panic,
  power loss) the rename may be lost even if the write appeared to succeed. The previous
  `state.json` survives but will not reflect mutations since the last successful write.
- **Not transactional**: if the daemon crashes mid-operation (e.g. between creating an overlay
  mount and recording the container), the filesystem may hold resources that have no matching
  state file entry. Use `cargo xtask nuke-test-state` to clean orphaned overlays and cgroups.
- **No WAL or journal**: there is no write-ahead log. The state file is a point-in-time
  snapshot, not an append-only log.

---

## StateRepository Port (Hexagonal Architecture)

`state.rs` also defines a `StateRepository` trait and a `JsonFileRepository` adapter. This is
the hexagonal port for persistence injection in tests. The `DaemonState::with_repository`
constructor accepts an `Arc<dyn StateRepository>`, but note that the current implementation
does not yet route internal `save_to_disk`/`load_from_disk` calls through this port — those
still use the direct `state_file` path. Full extraction through the port is a planned follow-on
refactor.

---

## Known Gaps

- No fsync before rename — crash may lose the last write
- `allocated_ips` (bridge network) is not persisted
- `DaemonState::with_repository` accepts a `StateRepository` port but does not yet use it for
  internal saves — the port is wired for future extraction
- Orphaned overlay mounts and cgroups are not cleaned up on restart
