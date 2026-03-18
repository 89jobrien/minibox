# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Minibox is a Docker-like container runtime written in Rust featuring daemon/client architecture, OCI image pulling from Docker Hub, Linux namespace isolation, cgroups v2 resource limits, and overlay filesystem support.

## Build and Development Commands

### Building

```bash
# Build all crates in workspace
cargo build --release

# Build specific crate
cargo build -p minibox-lib
cargo build -p miniboxd
cargo build -p minibox-cli

# Check all crates without building
cargo check --workspace
```

Binaries output to `target/release/miniboxd` and `target/release/minibox`.

### Running the Daemon and CLI

```bash
# Start daemon (requires root)
sudo ./target/release/miniboxd

# CLI commands (daemon must be running)
sudo ./target/release/minibox pull alpine
sudo ./target/release/minibox run alpine -- /bin/echo "Hello"
sudo ./target/release/minibox ps
sudo ./target/release/minibox stop <container_id>
sudo ./target/release/minibox rm <container_id>
```

### Testing

See `TESTING.md` for comprehensive testing strategy and guidelines.

**Quick reference:**

```bash
# On macOS, only minibox-lib tests run — miniboxd and minibox-cli have
# compile_error!() guards. Use: cargo test -p minibox-lib

# Run all tests (requires Linux)
cargo test --workspace

# Run tests for specific crate
cargo test -p minibox-lib

# Run specific test module
cargo test -p minibox-lib protocol::tests

# Run with output
cargo test -- --nocapture

# Task runner (preferred for integration/e2e)
just test-unit          # unit tests, any platform
just test-integration   # cgroup tests, Linux+root
just test-e2e           # daemon+CLI tests, Linux+root
just doctor             # preflight capability check

# Run benchmarks (minibox-lib only)
cargo bench -p minibox-lib          # protocol codec encode/decode
```

**Test Status:**

- Unit + conformance: ~53 lib tests + 12 handler + 10 conformance passing
- Cgroup integration: 16 tests (Linux+root, `just test-integration`)
- E2E daemon+CLI: 14 tests (Linux+root, `just test-e2e`)
- Existing integration: 8 tests (Linux+root)
- Path validation: TODO (security-critical)
- Tar entry validation: TODO (security-critical)
- Specs/plans: `docs/superpowers/specs/`, `docs/superpowers/plans/`

## Architecture Overview

### Workspace Structure

Three crates in cargo workspace:

1. **minibox-lib** (library): Core container primitives, image management, shared protocol types
2. **miniboxd** (binary): Async daemon managing containers via Unix socket API
3. **minibox-cli** (binary): CLI client sending commands to daemon

### Critical Design Patterns

**Hexagonal Architecture**: Domain traits (`ResourceLimiter`, `FilesystemProvider`, `ContainerRuntime`, `ImageRegistry`) in `minibox-lib/src/domain.rs` are implemented by adapters in `minibox-lib/src/adapters/`. Tests use mock adapters (`adapters::mocks`). Integration tests exercise real adapters against live infrastructure.

**Adapter Suites**: `MINIBOX_ADAPTER` env var selects between `native` (Linux namespaces, overlay FS, cgroups v2, requires root) and `gke` (proot, copy FS, no-op limiter, unprivileged). Wired in `miniboxd/src/main.rs`.

**Async/Sync Boundary**: Daemon uses Tokio async for socket I/O (`server.rs`) but spawns blocking tasks for container operations (fork/clone syscalls cannot be async). Container creation in `handler.rs` uses `tokio::task::spawn_blocking`.

**Protocol**: JSON-over-newline on Unix socket (`/run/minibox/miniboxd.sock`). Each message is single JSON object terminated by `\n`. Types defined in `minibox-lib/src/protocol.rs` using serde with `#[serde(tag = "type")]` for tagged enums.

**State Management**: In-memory HashMap in `miniboxd/src/state.rs` tracks containers. Not persisted - daemon restart loses all records. Container state machine: Created → Running → Stopped.

**CLI returns immediately** — `minibox run` prints the container ID as soon as the daemon creates it, *before* the container process has exec'd. Actual execution success/failure is only visible in `journalctl -u miniboxd`. A fast `time minibox run ...` does not confirm the container ran.

**Image Storage**: Layers stored as extracted directories in `/var/lib/minibox/images/{image}/{digest}/`. Overlay filesystem stacks layers (read-only lower dirs) + container-specific upper/work dirs.

### Container Lifecycle Flow

1. CLI sends `RunContainer` request to daemon
2. Daemon checks image cache, pulls from Docker Hub if missing (anonymous auth)
3. Creates overlay mount: `lowerdir=layer1:layer2:...`, `upperdir=container_rw`, `workdir=container_work`
4. Forks child with `clone(CLONE_NEWPID|CLONE_NEWNS|CLONE_NEWUTS|CLONE_NEWIPC|CLONE_NEWNET)` via nix crate
5. Child process (in `minibox-lib/src/container/process.rs`):
   - Creates cgroup at `/sys/fs/cgroup/minibox/{id}/`
   - Writes PID to `cgroup.procs`
   - Sets memory.max and cpu.weight if limits specified
   - Mounts proc, sys, tmpfs in new mount namespace
   - Calls `pivot_root()` to switch to container rootfs
   - Closes inherited FDs
   - Executes user command via `execvp()`
6. Parent tracks PID, spawns background reaper task to detect exit
7. On exit, reaper updates state to Stopped

### Key Modules

**minibox-lib/src/**:

- `preflight.rs`: Host capability probing (cgroups v2, overlay, systemd, kernel version). Used by `just doctor` and test `require_capability!` macro.
- `domain.rs`: Trait definitions (ports) for hexagonal architecture

**minibox-lib/src/container/**:

- `namespace.rs`: Linux namespace setup using nix crate wrappers
- `cgroups.rs`: cgroups v2 manipulation (memory, CPU weight)
- `filesystem.rs`: overlay mount, pivot_root, path validation
- `process.rs`: Container init process, fork/clone, exec

**minibox-lib/src/image/**:

- `registry.rs`: Docker Hub v2 API client (token auth, manifest/blob fetch)
- `manifest.rs`: OCI manifest parsing
- `layer.rs`: Tar extraction with security validation

**miniboxd/src/**:

- `server.rs`: Unix socket listener with SO_PEERCRED auth
- `handler.rs`: Request routing (run, ps, stop, rm, pull)
- `state.rs`: In-memory container tracking

## Security Considerations

**Critical vulnerabilities were fixed in commits `8ea4f73` and `2fc7036`**. When modifying code, maintain these protections:

### Path Validation

Always validate paths in overlay filesystem and tar extraction:

- Use `validate_layer_path()` in `filesystem.rs` to canonicalize and check for `..` components
- Reject symlinks to absolute paths or parent directories
- Use `std::fs::canonicalize()` and verify result stays within base directory

### Tar Extraction Safety

In `layer.rs`, manual entry validation prevents Zip Slip attacks:

- Reject paths with `..` components
- Reject absolute symlinks
- Reject device nodes, named pipes, character/block devices
- Strip setuid/setgid bits from extracted files

### Unix Socket Authentication

`server.rs` uses `SO_PEERCRED` to authenticate clients:

- Only UID 0 (root) can connect
- Socket permissions set to `0600` (owner-only)
- Client UID/PID logged for audit trail

### Resource Limits

Image pulls enforce limits to prevent DoS:

- Max manifest size: 10MB (`registry.rs`)
- Max layer size: 1GB per layer
- Total image size limit: 5GB

## Directory Structure

### Runtime Paths

- `/run/minibox/miniboxd.sock`: Daemon Unix socket
- `/run/minibox/containers/{id}/`: Runtime state (PID files)

### Persistent Storage

- `/var/lib/minibox/images/`: Image layers (extracted tar contents) + manifests
- `/var/lib/minibox/containers/{id}/`: Per-container overlay dirs (merged, upper, work)

### Cgroups

- `/sys/fs/cgroup/minibox.slice/miniboxd.service/{id}/`: Per-container cgroup (systemd-managed)
- `/sys/fs/cgroup/minibox.slice/miniboxd.service/supervisor/`: Daemon's own leaf cgroup

## System Requirements

**Linux-specific**: Code uses Linux kernel syscalls via nix crate. Cannot compile on macOS/Windows.

**Required kernel features**:

- Kernel 4.0+ (5.0+ recommended for cgroups v2)
- cgroups v2 unified hierarchy: `/sys/fs/cgroup/` must be mounted as cgroup2
- Namespace support: `CONFIG_USER_NS`, `CONFIG_PID_NS`, `CONFIG_NET_NS`, `CONFIG_UTS_NS`, `CONFIG_IPC_NS`
- Overlay filesystem: `CONFIG_OVERLAY_FS=y`

**Root required**: Daemon must run as root to create namespaces, mount filesystems, and manipulate cgroups.

## Current Limitations

Understanding these helps prioritize feature development:

- **No networking setup**: Containers get isolated network namespace but no bridge/veth configuration
- **No user namespace remapping**: Runs as root inside containers (no rootless support)
- **No persistent state**: Daemon restart loses all container records
- **No TTY support**: stdout/stderr not piped back to CLI
- **No exec command**: Cannot run commands in existing containers
- **No logs capture**: Container output not stored
- **No Dockerfile support**: Image-only workflow
- **Adapter wiring incomplete**: `docker_desktop` and `wsl` adapters exist in `minibox-lib/src/adapters/` but are not wired into `miniboxd`. `MINIBOX_ADAPTER` accepts `native`, `gke`, or `colima`; `docker_desktop` and `wsl` are library-only.

## Debugging

### Container init gotchas (relevant when modifying `filesystem.rs` or `process.rs`)

- **`pivot_root` requires `MS_PRIVATE` first** — after `CLONE_NEWNS` the child inherits shared mount propagation from the parent; `pivot_root` fails EINVAL unless you call `mount("", "/", MS_REC|MS_PRIVATE)` inside the child before the bind-mount.
- **`close_extra_fds` must collect before closing** — iterating `/proc/self/fd` and calling `close()` inside the loop closes the `ReadDir`'s own FD, causing a panic. Collect all FD numbers into a `Vec` first, then close.
- **Absolute symlink rewrite in `layer.rs`** — `strip_prefix("/")` gives a path relative to the container root, not the symlink's directory. Use `relative_path(entry_dir, abs_target)` (defined in `layer.rs`) to get the correct relative target; otherwise busybox applet symlinks resolve to non-existent paths (e.g. `/bin/bin/busybox`).
- **Tar root entries** — `"."` and `"./"` entries in OCI layers must be skipped before path validation; `Path::join("./")` normalizes the CurDir component away, causing a false path-escape error.

### Cgroup v2 gotchas (relevant when modifying `cgroups.rs`)

- `io.max` requires `MAJOR:MINOR` of a real block device — Colima VM uses virtio (`vda` = 253:0), not sda (8:0). Use `find_first_block_device()` (reads `/sys/block/*/dev`) rather than hardcoding.
- PID 0 is silently accepted by kernel 6.8 but is never valid — validate explicitly before writing to `cgroup.procs`.
- A cgroup cannot have both processes AND children (cgroup v2 "no internal process" rule). Tests run inside a dedicated `minibox-test-slice/runner-leaf` cgroup via `scripts/run-cgroup-tests.sh`.

### Check kernel features

```bash
# Verify cgroups v2
mount | grep cgroup2

# Check namespace support
ls /proc/self/ns/

# Verify overlay module
lsmod | grep overlay
```

### Inspect container state

```bash
# View cgroup limits (path depends on systemd config, check MINIBOX_CGROUP_ROOT)
cat /sys/fs/cgroup/minibox.slice/miniboxd.service/{container_id}/memory.max
cat /sys/fs/cgroup/minibox.slice/miniboxd.service/{container_id}/cpu.weight

# Check overlay mount
mount | grep minibox

# View container process
ps aux | grep {container_pid}
```

### Daemon logs

Daemon uses tracing crate. Set `RUST_LOG` environment variable:

```bash
RUST_LOG=debug sudo ./target/release/miniboxd
```

## Adding New Features

When extending minibox:

1. **Protocol changes**: Update `protocol.rs` types first, then implement in `handler.rs`
2. **Container primitives**: Add to `minibox-lib/src/container/`, use nix crate for syscalls
3. **Image operations**: Extend `minibox-lib/src/image/` modules
4. **State persistence**: Consider replacing HashMap in `state.rs` with serialized storage
5. **Networking**: Implement in new `network.rs` module, add bridge/veth setup in container init

## Environment Variables

Override runtime paths (useful for testing and non-standard deployments):

- `MINIBOX_DATA_DIR` — image/container storage (default: `/var/lib/minibox`)
- `MINIBOX_RUN_DIR` — socket/runtime dir (default: `/run/minibox`)
- `MINIBOX_SOCKET_PATH` — Unix socket path
- `MINIBOX_CGROUP_ROOT` — cgroup root for containers (default: `/sys/fs/cgroup/minibox.slice/miniboxd.service`)
- `MINIBOX_ADAPTER` — adapter suite: `native` (default) or `gke`

## Skills Available

Global minibox skills available across all projects:

- `minibox:build-test`: Build automation and testing workflows
- `minibox:runtime`: Container debugging and daemon operations
- `minibox:setup`: Environment configuration and kernel feature verification
- `minibox:architecture`: Codebase navigation and component details

Invoke with `/` prefix, e.g., `/minibox:setup` or `/minibox:runtime`.
