# miniboxd

Async container daemon entry point with platform dispatch.

## Architecture

Dispatches to platform-specific implementations via conditional compilation:

- **Linux** — `minibox` crate for full container runtime (handler, server, state)
- **macOS** — `macbox` for smolvm, krun, or Colima containers
- **Windows** — `winbox` stub for future implementation

The main server loop uses `minibox::daemon` for request handling and state
management across all platforms.

## Adapter selection

Set `MINIBOX_ADAPTER` to choose an adapter suite at startup. Unrecognized values
print a structured error listing valid options.

| Value    | Platform     | Notes                                      |
| -------- | ------------ | ------------------------------------------ |
| `native` | Linux        | Full namespace/cgroup v2/overlay isolation |
| `gke`    | Linux        | Unprivileged pods via proot + copy-FS      |
| `smolvm` | macOS/Linux  | Default on macOS; falls back to `krun`     |
| `krun`   | macOS/Linux  | Automatic fallback when smolvm absent      |
| `colima` | macOS/Linux  | Delegates to Colima (limactl + nerdctl)    |
| `vz`     | macOS only   | Feature-gated (`--features vz`); blocked by Apple bug (GH #61) |

Run `mbx doctor` to see which adapter suites are compiled into the current build.

## Configuration

Environment variables:

- `MINIBOX_ADAPTER` — Adapter suite (see table above)
- `MINIBOX_DATA_DIR` — Image/container storage (default: `/var/lib/minibox`)
- `MINIBOX_RUN_DIR` — Socket/runtime dir (default: `/run/minibox`)
- `MINIBOX_SOCKET_PATH` — Unix socket path
- `MINIBOX_CGROUP_ROOT` — Cgroup root for containers
- `MINIBOX_NETWORK_MODE` — Network mode: `none` (default) or `bridge`
- `MINIBOX_OTLP_ENDPOINT` — OTLP trace export endpoint (`otel` feature required)
- `MINIBOX_METRICS_ADDR` — Prometheus metrics bind address (`metrics` feature required)

## Logging

Set `RUST_LOG` to see daemon activity:

```bash
RUST_LOG=info miniboxd
RUST_LOG=debug miniboxd  # verbose
```
