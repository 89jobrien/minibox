# Socket Paths

This document defines the canonical socket paths for all minibox components. It resolves the
inconsistency between `/run/minibox/minibox.sock` (seen in some older docs) and
`/run/minibox/miniboxd.sock` (the actual runtime path).

---

## Canonical Daemon Socket

The miniboxd Unix socket is named **`miniboxd.sock`** in all code paths:

| Source | Path |
|---|---|
| `miniboxd/src/main.rs` (default) | `/run/minibox/miniboxd.sock` |
| `minibox-client/src/lib.rs` (Linux default) | `/run/minibox/miniboxd.sock` |
| `minibox-client/src/lib.rs` (macOS default) | `/tmp/minibox/miniboxd.sock` |
| `dockerbox/src/main.rs` (`MINIBOX_SOCKET` default) | `/run/minibox/miniboxd.sock` |

The name `minibox.sock` does not appear in any code path. Any documentation referencing
`/run/minibox/minibox.sock` is incorrect.

---

## dockerbox Socket

`dockerboxd` exposes a Docker-compatible API shim on a separate socket:

| Component | Default Path | Override Env Var |
|---|---|---|
| dockerboxd listen socket | `/run/dockerbox/dockerbox.sock` | `DOCKERBOX_SOCKET` |
| dockerboxd upstream (miniboxd) | `/run/minibox/miniboxd.sock` | `MINIBOX_SOCKET` |

Note: the `MINIBOX_SOCKET` variable used by dockerbox is distinct from `MINIBOX_SOCKET_PATH`
used by the minibox CLI and client library. Both point to the same miniboxd socket but are
read by different components.

---

## Environment Variable Overrides

### miniboxd (daemon)

| Env Var | Effect | Default |
|---|---|---|
| `MINIBOX_SOCKET_PATH` | Full path to the daemon socket | `$MINIBOX_RUN_DIR/miniboxd.sock` |
| `MINIBOX_RUN_DIR` | Directory for the socket and runtime files | `/run/minibox` |
| `MINIBOX_SOCKET_MODE` | Octal permission bits for the daemon socket | `0600` |
| `MINIBOX_SOCKET_GROUP` | Group name to `chown` the daemon socket to | (none) |

Resolution order in miniboxd: `MINIBOX_SOCKET_PATH` → `$MINIBOX_RUN_DIR/miniboxd.sock` →
`/run/minibox/miniboxd.sock`.

### minibox-client / minibox-cli

| Env Var | Effect | Default |
|---|---|---|
| `MINIBOX_SOCKET_PATH` | Full path to the daemon socket | see below |
| `MINIBOX_RUN_DIR` | Directory; socket is `<dir>/miniboxd.sock` | `/run/minibox` |

Resolution order in `minibox-client::default_socket_path()`:
1. `MINIBOX_SOCKET_PATH` (full path)
2. `$MINIBOX_RUN_DIR/miniboxd.sock`
3. Platform default: `/tmp/minibox/miniboxd.sock` (macOS) or `/run/minibox/miniboxd.sock`
   (Linux)

### dockerbox

| Env Var | Effect | Default |
|---|---|---|
| `DOCKERBOX_SOCKET` | Path to the dockerbox listen socket | `/run/dockerbox/dockerbox.sock` |
| `MINIBOX_SOCKET` | Path to the upstream miniboxd socket | `/run/minibox/miniboxd.sock` |
| `DOCKERBOX_SOCKET_MODE` | Octal permission bits for the dockerbox socket | `0660` |
| `DOCKERBOX_SOCKET_GROUP` | Group name to `chown` the dockerbox socket to | (none) |

---

## Security Model

### miniboxd socket

- Default permissions: `0600` (owner-only)
- SO_PEERCRED enforced: only UID 0 connections are accepted by the native and GKE adapter
  suites (`require_root_auth = true`). The colima suite does not enforce this gate.
- Group access: set `MINIBOX_SOCKET_GROUP` to allow group members to connect; set
  `MINIBOX_SOCKET_MODE=0660` to enable group read/write.

### dockerbox socket

- Default permissions: `0660` (root-owned, group-accessible) — matches Docker daemon convention
- Group access: set `DOCKERBOX_SOCKET_GROUP=docker` so members of the `docker` group can
  connect without sudo
- The upstream miniboxd gate (SO_PEERCRED UID 0) still applies to all operations that reach
  miniboxd, regardless of dockerbox socket permissions

---

## Summary

| Socket | Default Path | Owner | Permissions |
|---|---|---|---|
| miniboxd | `/run/minibox/miniboxd.sock` | root | `0600` |
| dockerboxd | `/run/dockerbox/dockerbox.sock` | root | `0660` |
