# minibox

A Docker-like container runtime written in Rust. Features a daemon/client architecture with OCI image pulling from Docker Hub, Linux namespace isolation, cgroups v2 resource limits, and overlay filesystem support.

Built as a foundation for extending into orchestration, networking, and custom runtimes.

## Architecture

```
┌──────────────────┐         Unix Socket          ┌──────────────────────┐
│   minibox (CLI)  │  ──── JSON/newline ────────▶  │   miniboxd (daemon)  │
│                  │  ◀──── JSON/newline ────────  │                      │
│  clap-based CLI  │                               │  tokio async server  │
│  run/ps/stop/    │                               │  container lifecycle │
│  rm/pull         │                               │  image management    │
└──────────────────┘                               └──────────┬───────────┘
                                                              │
                                                   ┌──────────▼───────────┐
                                                   │   minibox-lib        │
                                                   │                      │
                                                   │  • namespaces        │
                                                   │  • cgroups v2        │
                                                   │  • overlay fs        │
                                                   │  • pivot_root        │
                                                   │  • OCI registry      │
                                                   │  • image store       │
                                                   │  • protocol types    │
                                                   └──────────────────────┘
```

## Crate Structure

| Crate | Binary | Description |
|-------|--------|-------------|
| `minibox-lib` | — | Core library: container primitives, image management, protocol |
| `miniboxd` | `miniboxd` | Daemon: manages containers, serves Unix socket API |
| `minibox-cli` | `minibox` | CLI client: sends commands to daemon |

## Features

- **Container isolation** via Linux namespaces (PID, Mount, UTS, IPC, Network)
- **Resource limits** via cgroups v2 (memory, CPU weight)
- **OCI image pulling** from Docker Hub (anonymous auth, manifest list resolution for multi-arch)
- **Overlay filesystem** for layered container rootfs
- **pivot_root** for proper root filesystem switching
- **Daemon/client architecture** over Unix domain socket (JSON-over-newline protocol)
- **Graceful shutdown** with SIGTERM/SIGINT handling

## Prerequisites

- **Linux kernel 4.0+** with namespace and cgroup support
- **cgroups v2** unified hierarchy enabled
- **Overlay filesystem** kernel module (`CONFIG_OVERLAY_FS=y`)
- **Root access** (daemon creates namespaces, mounts, and cgroups)
- **Rust 1.75+**

### Required kernel features

```
CONFIG_USER_NS=y
CONFIG_PID_NS=y
CONFIG_NET_NS=y
CONFIG_UTS_NS=y
CONFIG_IPC_NS=y
CONFIG_CGROUPS=y
CONFIG_OVERLAY_FS=y
```

## Building

```bash
cargo build --release
```

Binaries are output to `target/release/miniboxd` and `target/release/minibox`.

## Usage

### Start the daemon

```bash
sudo ./target/release/miniboxd
```

The daemon listens on `/run/minibox/miniboxd.sock` and stores images in `/var/lib/minibox/images/` and container state in `/var/lib/minibox/containers/`.

### Pull an image

```bash
sudo ./target/release/minibox pull alpine
```

### Run a container

```bash
# Run a command in an alpine container
sudo ./target/release/minibox run alpine -- /bin/echo "Hello from minibox!"

# Run with resource limits
sudo ./target/release/minibox run alpine --memory 536870912 --cpu-weight 500 -- /bin/sh

# Run a specific tag
sudo ./target/release/minibox run ubuntu -t 22.04 -- /bin/bash
```

### List containers

```bash
sudo ./target/release/minibox ps
```

Output:
```
CONTAINER ID    IMAGE               COMMAND              STATE       CREATED                    PID
------------------------------------------------------------------------------------------------------
a1b2c3d4e5f6    alpine:latest       /bin/echo Hello…     Stopped     2026-03-09T10:30:00+00:00  -
```

### Stop a container

```bash
sudo ./target/release/minibox stop a1b2c3d4e5f6
```

### Remove a container

```bash
sudo ./target/release/minibox rm a1b2c3d4e5f6
```

## Protocol

The CLI and daemon communicate over a Unix domain socket using newline-delimited JSON. Each message is a tagged enum:

### Requests (CLI → Daemon)

```json
{"type":"Run","image":"alpine","tag":"latest","command":["/bin/sh"],"memory_limit_bytes":null,"cpu_weight":null}
{"type":"Stop","id":"a1b2c3d4e5f6"}
{"type":"Remove","id":"a1b2c3d4e5f6"}
{"type":"List"}
{"type":"Pull","image":"alpine","tag":"latest"}
```

### Responses (Daemon → CLI)

```json
{"type":"ContainerCreated","id":"a1b2c3d4e5f6"}
{"type":"Success","message":"container a1b2c3d4e5f6 stopped"}
{"type":"ContainerList","containers":[...]}
{"type":"Error","message":"container not found"}
```

## Directory Layout

| Path | Purpose |
|------|---------|
| `/run/minibox/miniboxd.sock` | Daemon Unix socket |
| `/run/minibox/containers/{id}/` | Runtime state (PID files) |
| `/var/lib/minibox/images/` | Pulled image layers + manifests |
| `/var/lib/minibox/containers/{id}/` | Per-container rootfs (overlay: merged, upper, work) |
| `/sys/fs/cgroup/minibox/{id}/` | Per-container cgroup directory |

## Container Lifecycle

1. CLI sends `Run` request to daemon
2. Daemon checks image cache, pulls from Docker Hub if needed
3. Daemon creates overlay rootfs from stacked image layers
4. Daemon clones child process with `CLONE_NEWPID | CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWNET`
5. Child: creates cgroup → sets hostname → pivot_root → closes leaked FDs → exec user command
6. Parent: tracks PID, spawns background reaper task
7. On exit: reaper updates state to Stopped

## Limitations (v0.1)

- **No networking**: containers get an isolated network namespace but no bridge/veth setup
- **No user namespace remapping**: runs as root
- **No Dockerfile/build support**: image-only
- **No persistent container state**: daemon restart loses all container records
- **No interactive TTY**: stdout/stderr not piped back to CLI
- **Linux only**: relies on Linux-specific syscalls

## Extending

This is designed as a foundation. Natural next steps:

- **Networking**: bridge + veth pair setup, port mapping
- **User namespaces**: rootless container support
- **Build**: Dockerfile parsing and layer building
- **Persistent state**: serialize container records to disk
- **TTY support**: pipe container stdout/stderr back through the socket
- **Exec**: `minibox exec <id> <cmd>` to run commands in existing containers
- **Logs**: capture and serve container output

## License

MIT
