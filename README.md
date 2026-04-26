# minibox

[![CI](https://github.com/89jobrien/minibox/actions/workflows/ci.yml/badge.svg)](https://github.com/89jobrien/minibox/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/89jobrien/minibox/branch/main/graph/badge.svg)](https://codecov.io/gh/89jobrien/minibox)
[![dependency status](https://deps.rs/repo/github/89jobrien/minibox/status.svg)](https://deps.rs/repo/github/89jobrien/minibox)

A container runtime written in Rust with daemon/client architecture, OCI image
pulling, Linux namespace isolation, cgroups v2 resource limits, and overlay
filesystem support. Hexagonal architecture makes adapter suites swappable at
startup.

**Status:** Development (`v0.21.0`)

---

## What Works Today (Linux)

The `native` adapter suite on Linux is the production path. It provides:

- **Container lifecycle** -- pull, run, stop, rm, ps, pause/resume
- **Container exec** -- `setns`-based exec with `-it` PTY support (native adapter only)
- **OCI image pull** -- Docker Hub v2 + ghcr.io, anonymous auth, parallel layers
- **Image management** -- `prune` / `rmi` with lease-based GC
- **Bind mounts + privileged mode** -- `-v`/`--mount`, `--privileged`
- **Log capture** -- `minibox logs <id>` for stored stdout/stderr
- **Container events** -- `minibox events` streams lifecycle events
- **Bridge networking** (experimental) -- veth pairs, NAT via iptables DNAT

---

## Quick Start

Requires Linux, root, kernel 4.0+ (5.0+ recommended), cgroups v2, overlay FS.

```bash
# Build
cargo build --release

# Start daemon
sudo ./target/release/miniboxd

# Pull, run, list, stop, remove
sudo ./target/release/mbx pull alpine
sudo ./target/release/mbx run alpine -- /bin/echo "Hello from minibox!"
sudo ./target/release/mbx ps
sudo ./target/release/mbx stop <id>
sudo ./target/release/mbx rm <id>
```

---

## Platform Support

| Platform | Status | Adapter | Notes |
| --- | --- | --- | --- |
| Linux x86_64 | **Production** | `native` | Full namespace/cgroup v2/overlay isolation |
| Linux aarch64 | **Production** | `native` | Same as x86_64 |
| Linux (GKE) | **Production** | `gke` | Unprivileged pods via proot + copy-FS |
| macOS (Apple Silicon) | Experimental | `colima`, `krun` (WIP) | VZ blocked by Apple bug ([GH #61](https://github.com/89jobrien/minibox/issues/61)) |
| macOS (Intel) | Experimental | `colima` | exec/logs limited |
| Windows | Planned | `winbox` stub | `winbox::start()` returns error; no runtime yet |

See [`docs/FEATURE_MATRIX.md`](docs/FEATURE_MATRIX.md) for the full per-platform
capability breakdown.

`miniboxd` selects the platform crate at compile time (`cfg` gates) and the
adapter suite at startup via `MINIBOX_ADAPTER`. Unrecognized values cause the
daemon to exit before binding the socket.

---

## Security Model

| Area | Protection |
| --- | --- |
| Socket auth | `SO_PEERCRED` -- UID 0 only, socket mode `0600` |
| Path traversal | `canonicalize()` + `..` rejection in overlay FS and tar extraction |
| Tar extraction | Rejects `..`, absolute symlinks, device nodes; strips setuid/setgid |
| DoS limits | 1 MB request, 10 MB manifest, 1 GB/layer, 5 GB total image |
| Mount flags | `MS_NOSUID`, `MS_NODEV`, `MS_NOEXEC` on proc/sys/tmpfs |
| PID limit | 1024 per container (default) |

**Not yet implemented:** capability dropping, seccomp filters, user namespace
remapping, rootless support. See `CLAUDE.md` ("Security Considerations") for
the full threat model.

---

## Architecture

Eight crates in the workspace:

```
                         +--------------+
                         |   miniboxd   |  binary -- daemon entrypoint
                         +------+-------+
                                |
              +-----------------+-----------------+
              |                 |                 |
        +-----+------+   +-----+------+   +------+-----+
        |  minibox   |   |   macbox   |   |   winbox   |
        |  Linux     |   | Colima/VZ/ |   |   (stub)   |
        | primitives |   |   krun     |   |            |
        +-----+------+   +------------+   +------------+
              |
        +-----+------+
        |minibox-core|  protocol, domain traits, OCI types, client
        +-----+------+
              |
        +-----+------+
        |minibox-     |  proc macros (as_any!, adapt!)
        |  macros     |
        +------------+

        +------------+         +------------+
        |    mbx     |  CLI    |  xtask     |  dev tooling
        +------------+         +------------+
```

**Hexagonal architecture.** Domain traits (`ImageRegistry`, `FilesystemProvider`,
`ResourceLimiter`, `ContainerRuntime`) live in `minibox-core`. Adapters implement
them. Tests use mock adapters -- no real HTTP or filesystem needed.

**Async/sync boundary.** Tokio handles socket I/O; container operations
(fork/clone/exec) run in `spawn_blocking`.

---

## Experimental and Planned

| Feature | Status | Notes |
| --- | --- | --- |
| Bridge networking | Experimental | `MINIBOX_NETWORK_MODE=bridge`; Linux native only |
| OCI push/commit/build | Experimental | Adapters exist; not wired into miniboxd |
| macOS Colima | Experimental | run/stop/ps work; exec/logs limited |
| macOS VZ.framework | Blocked | Apple bug on macOS 26 ARM64 ([GH #61](https://github.com/89jobrien/minibox/issues/61)) |
| Observability | Opt-in | OTLP (`feature = "otel"`), Prometheus (`feature = "metrics"`) |
| Windows | Planned | `winbox` stub compiles; WSL2 is the likely first backend |
| Port forwarding / DNS | Planned | Not started |
| Rootless | Planned | No user namespace remapping yet |
| Dockerfile parser | Planned | `MiniboxImageBuilder` exists but no DSL |
| MCP control surface | Planned | Agent-facing pull/run/ps/stop/rm |

Roadmap details in [`docs/ROADMAP.md`](docs/ROADMAP.md).

---

## Testing

```bash
cargo xtask test-unit              # ~760 unit/conformance/property tests (any platform)
just test-integration              # cgroup tests (Linux + root)
just test-e2e                      # daemon + CLI (Linux + root)
cargo xtask test-conformance       # OCI conformance matrix
just doctor                        # preflight capability check
```

See `CLAUDE.md` for the full testing strategy.

---

## Development

```bash
cargo build --release
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo xtask pre-commit             # fmt + clippy + release build gate
```

| Variable | Default | Purpose |
| --- | --- | --- |
| `MINIBOX_ADAPTER` | `native` | Adapter suite selection |
| `MINIBOX_DATA_DIR` | `/var/lib/minibox` | Image + container storage |
| `MINIBOX_RUN_DIR` | `/run/minibox` | Socket + runtime state |
| `MINIBOX_CGROUP_ROOT` | `/sys/fs/cgroup/minibox.slice/miniboxd.service` | Cgroup root |
| `RUST_LOG` | -- | Tracing log level |

Git workflow: `main` (develop) -> `next` (auto-promoted on green CI) ->
`stable` (manual, tagged releases). See
[`docs/superpowers/specs/2026-03-26-git-workflow-design.md`](docs/superpowers/specs/2026-03-26-git-workflow-design.md).

---

See `CLAUDE.md` for the full development guide, debugging tips, and architecture
details.

<sup>Previously named `linuxbox` and `mbx` during early development.</sup>
