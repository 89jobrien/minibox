# miniboxd

Async container daemon entry point with platform dispatch.

## Architecture

Dispatches to platform-specific implementations via conditional compilation:
- **Linux** — `mbx` + `daemonbox` for full container runtime
- **macOS** — `macbox` for Colima or Virtualization.framework containers
- **Windows** — `winbox` stub for future implementation

The main server loop uses `daemonbox` for request handling and state management across all platforms.

## Configuration

Environment variables:
- `MINIBOX_DATA_DIR` — Image/container storage (default: `~/.mbx/cache` or `/var/lib/minibox`)
- `MINIBOX_RUN_DIR` — Socket/runtime dir (default: `/run/minibox`)
- `MINIBOX_SOCKET_PATH` — Unix socket path
- `MINIBOX_CGROUP_ROOT` — Cgroup root for containers
- `MINIBOX_ADAPTER` — Adapter suite: `native`, `gke`, `colima`, `vz` (macOS, requires `--features vz`)

## Logging

Set `RUST_LOG` to see daemon activity:

```bash
RUST_LOG=info miniboxd
RUST_LOG=debug miniboxd  # verbose
```
