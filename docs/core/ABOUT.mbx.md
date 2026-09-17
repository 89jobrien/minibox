---
source_sha: 583d795681594db0435e7fbc4d196e19edfef358
sources:
  - Cargo.toml
  - crates/minibox-domain
  - crates/minibox-core
  - crates/minibox
  - crates/miniboxd
  - crates/mbx
  - crates/macbox
  - crates/smolbox
  - crates/winbox
  - crates/minibox-crux-plugin
  - crates/mcp
  - crates/minibox-testsuite
  - crates/minibox-bench
  - crates/minibox-cni
  - crates/minibox-tui
  - crates/ail
  - xtask
generated: 2026-09-16
---

# About minibox

A container runtime written in Rust. Daemon/CLI split, OCI image pulling, Linux namespace
isolation, cgroups v2 resource limits, and overlay filesystem support. Hexagonal architecture
keeps adapter suites swappable at startup with no recompile.

**Status:** Active development — `v0.33.0`. Linux runs natively and is production-ready; macOS feels like native but requires `smolvm`
(VM-backed). See the [Platform Support](#platform-support) table.

---

## Why

Most container runtimes are large, opaque, and hard to embed or extend. Minibox is an
intentionally small Rust implementation where every layer — protocol, domain traits, adapters,
daemon — is readable and swappable. It exists as both a working runtime and a reference for
how I structure systems software in Rust: hexagonal architecture, async/sync boundaries,
structured tracing, property testing.

---

## What Works Today

### Linux (production)

- Container lifecycle — pull, run, stop, rm, ps, pause/resume
- OCI image pull — Docker Hub v2 + ghcr.io, anonymous auth, parallel layers
- Image management — `prune` / `rmi` with lease-based GC
- Bind mounts and privileged mode — `-v`/`--mount`, `--privileged`
- Log capture — `mbx logs <id>` for stored stdout/stderr
- Container events — `mbx events` streams lifecycle events

### Experimental-ish

- **Container exec** — `setns`-based exec with PTY support (`-it`); Linux (`native`) only
- **Bridge networking** — veth pairs, DNS configuration, and port-forwarding DNAT (`MINIBOX_NETWORK_MODE=bridge`); Linux only
- **macOS adapters** — run/stop/ps via smolvm or krun (VM-backed); exec/logs not supported;
  Colima available as an alternative via Lima VM; feature-gated VZ is restored but currently
  blocked by a boot failure

---

## Quick Start

Requires Linux, root, kernel 5.0+, cgroups v2, overlay FS.

```bash
# Build
cargo build --release

# Start daemon
sudo ./target/release/miniboxd

# Pull and run
sudo ./target/release/mbx pull alpine
sudo ./target/release/mbx run alpine -- /bin/echo "hello from minibox"

# Manage containers
sudo ./target/release/mbx ps
sudo ./target/release/mbx logs <id>
sudo ./target/release/mbx stop <id>
sudo ./target/release/mbx rm <id>

# Check compiled adapter info (no daemon needed)
./target/release/mbx doctor
```

---

## Platform Support

| Platform              | Status         | Adapter         | Notes                                      |
| --------------------- | -------------- | --------------- | ------------------------------------------ |
| Linux x86_64          | **Production** | `native`        | Full namespace/cgroup v2/overlay           |
| Linux aarch64         | **Production** | `native`        | Same as x86_64                             |
| Linux (GKE)           | **Production** | `gke`           | Unprivileged pods via proot + copy-FS      |
| macOS (Apple Silicon) | Experimental   | `smolvm`/`krun` | exec/logs limited; VZ opt-in but blocked   |
| macOS (Intel)         | Experimental   | `colima`        | exec/logs limited                          |
| Windows               | Planned        | `winbox` stub   | Returns error unconditionally              |

---

## Architecture

16 crates plus `xtask` (17 workspace members), Rust 2024 edition:

```
minibox-macros          proc macros (as_any!, adapt!)
    ^
minibox-domain          pure domain values, policies, events, and ports
    ^
minibox-core            protocol, client transport, OCI/image services, compatibility re-exports
    ^
minibox                 Linux adapters, daemon handler/server/state, test infra
    ^        ^        ^
macbox   smolbox   winbox  macOS Colima | macOS smolvm/krun | Windows stub
    ^        ^        ^
miniboxd                daemon entry point, adapter dependency injection

minibox-cli             package containing the `mbx` CLI binary
minibox-crux-plugin     crux agent bridge over JSON-RPC stdio
minibox-mcp             MCP stdio server for agent-controlled minibox tools
minibox-testsuite       conformance test harness for adapter trait contracts
minibox-bench           benchmark crate
minibox-cni             feature-gated CNI execution for native bridge networking
minibox-tui             read-only terminal dashboard, used by feature-gated `mbx tui`
ail                     placeholder crate
xtask                   CI gates, test runners, bench, VM image build
```

**Hexagonal ports.** Domain traits (`ImageRegistry`, `FilesystemProvider`, `ResourceLimiter`,
`ContainerRuntime`, `NetworkProvider`, …) live under `crates/minibox-domain/src/`. Adapters implement them.
Tests use mock adapters — no real HTTP or filesystem required.

**Async/sync boundary.** Tokio handles socket I/O. Container operations (fork/clone/exec) run
in `spawn_blocking` to avoid blocking the runtime.

**Protocol.** JSON-over-newline on a Unix socket. 29 request variants, 28 response variants.
Canonical source: `crates/minibox-core/src/protocol.rs`.

Full architecture reference: [`ARCHITECTURE`](ARCHITECTURE.mbx.md).

---

## Security Model

| Area           | Protection                                                          |
| -------------- | ------------------------------------------------------------------- |
| Socket auth    | `SO_PEERCRED` — UID 0 only, socket mode `0600`                      |
| Path traversal | `canonicalize()` + `..` rejection in overlay FS and tar extraction  |
| Tar extraction | Rejects `..`, absolute symlinks, device nodes; strips setuid/setgid |
| DoS limits     | 1 MB request, 10 MB manifest, 10 GiB/layer, 50 GiB total image      |
| Mount flags    | `MS_NOSUID`, `MS_NODEV`, `MS_NOEXEC` on proc/sys/tmpfs              |
| PID limit      | 1024 per container (default)                                        |

**Not yet implemented:** default capability dropping, general-purpose seccomp profiles, user
namespace remapping, and rootless support. A narrow seccomp filter does prevent widening mounts
with `mount(MS_REMOUNT)` after container initialization.

---

## Configuration

Configuration is layered: TOML config file → environment variables → defaults.

**Config files** (later overrides earlier):

1. `/etc/minibox/config.toml` (system)
2. `~/.config/minibox/config.toml` (user)

```toml
adapter = "smolvm"
log_level = "info"

[policy]
allow_privileged = false
allow_bind_mounts = false
max_image_size_mb = 2048
```

**Environment variables** (override config file values):

| Variable                    | Default                                         | Purpose                   |
| --------------------------- | ----------------------------------------------- | ------------------------- |
| `MINIBOX_ADAPTER`           | `native` (Linux) / `smolvm` (macOS)             | Adapter suite selection   |
| `MINIBOX_DATA_DIR`          | `/var/lib/minibox`                              | Image + container storage |
| `MINIBOX_RUN_DIR`           | `/run/minibox`                                  | Socket + runtime state    |
| `MINIBOX_CGROUP_ROOT`       | `/sys/fs/cgroup/minibox.slice/miniboxd.service` | Cgroup root               |
| `MINIBOX_ALLOW_BIND_MOUNTS` | `false`                                         | Permit `-v` bind mounts   |
| `MINIBOX_ALLOW_PRIVILEGED`  | `false`                                         | Permit `--privileged`     |
| `RUST_LOG`                  | —                                               | Tracing log level         |

---

## Testing

```bash
cargo xtask test unit        # workspace library tests (any platform)
cargo xtask test property    # property tests
cargo xtask test conformance # OCI adapter conformance matrix
just test-integration        # cgroup tests (Linux + root)
just test-e2e                # protocol end-to-end tests (any platform)
just test-system             # daemon + CLI full-stack tests (Linux + root)
```

The conformance suite runs 28 backend-agnostic tests against every adapter. Unit tests run on
macOS without root. See [`TEST_INFRASTRUCTURE`](TEST_INFRASTRUCTURE.mbx.md).

---

## Developer Workflow

```bash
cargo xtask pre-commit       # conditional fmt/restage + clippy/architecture + docs/date
cargo xtask prepush          # musl-check + release build/tests + conformance
just --list                  # all available recipes
mbx doctor                   # preflight: show compiled adapters and capabilities
```

See [`DEVELOPMENT.md`](../../DEVELOPMENT.md) for the full workflow.

---

## Contributing

Issues and PRs are welcome. A few things to know before contributing:

- Run `cargo xtask pre-commit` before committing and `cargo xtask prepush` before pushing.
- New adapters implement the domain traits under `crates/minibox-domain/src/`.
- Protocol changes start in `crates/minibox-core/src/protocol.rs`; update handlers, CLI paths, and
  snapshot tests together.
- Linux-only code must be gated with `#[cfg(target_os = "linux")]` so macOS `cargo check`
  still passes.
- No `.unwrap()` in production paths — use `.context("description")?`.

---

## Roadmap

| Feature                | Status                                |
| ---------------------- | ------------------------------------- |
| Bridge networking      | Experimental                          |
| OCI push/commit/build  | Experimental                          |
| macOS VZ.framework     | Restored behind `vz`; boot blocked    |
| Seccomp / capabilities | Planned                               |
| Rootless support       | Planned                               |
| Port forwarding / DNS  | Implemented for native bridge mode    |
| Windows (WSL2)         | Planned                               |
| MCP control surface    | MCP stdio server implemented          |

Full details: [`ROADMAP`](ROADMAP.mbx.md).

---

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.

<sup>Previously named `mbx` during early development.</sup>
