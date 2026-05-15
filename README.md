# minibox

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A container runtime written in Rust. Daemon/CLI split, OCI image pulling, Linux namespace
isolation, cgroups v2 resource limits, and overlay filesystem support. Hexagonal architecture
keeps adapter suites swappable at startup with no recompile.

**Status:** Active development — `v0.24.0`. Linux runs natively and is production-ready; macOS feels like native but requires `smolvm`
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

Linux is production-ready: full container lifecycle (pull, run, stop, rm, ps, pause/resume),
OCI image management with parallel layer pulls, bind mounts, privileged mode, log capture, and
container events. macOS runs containers via smolvm/krun (VM-backed) with exec/logs limited.
See [`docs/FEATURE_MATRIX.mbx.md`](docs/FEATURE_MATRIX.mbx.md) for the full per-adapter
capability matrix.

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

| Platform              | Status         | Adapter        | Notes                                     |
| --------------------- | -------------- | -------------- | ----------------------------------------- |
| Linux x86_64          | **Production** | `native`       | Full namespace/cgroup v2/overlay          |
| Linux aarch64         | **Production** | `native`       | Same as x86_64                            |
| Linux (GKE)           | **Production** | `gke`          | Unprivileged pods via proot + copy-FS     |
| macOS (Apple Silicon) | Experimental   | `smolvm`/`krun`| exec/logs limited; VZ blocked by Apple bug|
| macOS (Intel)         | Experimental   | `colima`       | exec/logs limited                         |
| Windows               | Planned        | `winbox` stub  | Returns error unconditionally             |

See [`docs/FEATURE_MATRIX.mbx.md`](docs/FEATURE_MATRIX.mbx.md) for the full per-adapter capability
breakdown.

---

## Architecture

10 crates, Rust 2024 edition:

```
minibox-macros          proc macros (as_any!, adapt!)
    ^
minibox-core            cross-platform types, domain traits, protocol, OCI ops
    ^
minibox                 Linux adapters, daemon handler/server/state, test infra
    ^         ^
macbox      winbox      macOS backends (colima/krun/smolvm/vz) | Windows stub
    ^          ^
miniboxd                daemon entry point, adapter dependency injection

mbx                     CLI client — connects via Unix socket
minibox-crux-plugin     crux agent bridge over JSON-RPC stdio
minibox-conformance     conformance test harness for adapter trait contracts
xtask                   CI gates, test runners, bench, VM image build
```

**Hexagonal ports.** Domain traits (`ImageRegistry`, `FilesystemProvider`, `ResourceLimiter`,
`ContainerRuntime`, `NetworkProvider`, …) live in `minibox-core`. Adapters implement them.
Tests use mock adapters — no real HTTP or filesystem required.

**Async/sync boundary.** Tokio handles socket I/O. Container operations (fork/clone/exec) run
in `spawn_blocking` to avoid blocking the runtime.

**Protocol.** JSON-over-newline on a Unix socket. 24 request variants, 22 response variants.
Canonical source: `minibox-core/src/protocol.rs`.

Full architecture reference: [`docs/ARCHITECTURE.mbx.md`](docs/ARCHITECTURE.mbx.md).

---

## Security Model

| Area           | Protection                                                          |
| -------------- | ------------------------------------------------------------------- |
| Socket auth    | `SO_PEERCRED` — UID 0 only, socket mode `0600`                      |
| Path traversal | `canonicalize()` + `..` rejection in overlay FS and tar extraction  |
| Tar extraction | Rejects `..`, absolute symlinks, device nodes; strips setuid/setgid |
| DoS limits     | 1 MB request, 10 MB manifest, 1 GB/layer, 5 GB total image          |
| Mount flags    | `MS_NOSUID`, `MS_NODEV`, `MS_NOEXEC` on proc/sys/tmpfs              |
| PID limit      | 1024 per container (default)                                        |

**Not yet implemented:** capability dropping, seccomp filters, user namespace remapping,
rootless support.

---

## Configuration

| Variable              | Default                                         | Purpose                   |
| --------------------- | ----------------------------------------------- | ------------------------- |
| `MINIBOX_ADAPTER`     | `native` (Linux) / `smolvm` (macOS)             | Adapter suite selection   |
| `MINIBOX_DATA_DIR`    | `/var/lib/minibox`                              | Image + container storage |
| `MINIBOX_RUN_DIR`     | `/run/minibox`                                  | Socket + runtime state    |
| `MINIBOX_CGROUP_ROOT` | `/sys/fs/cgroup/minibox.slice/miniboxd.service` | Cgroup root               |
| `RUST_LOG`            | —                                               | Tracing log level         |

---

## Testing

```bash
cargo xtask test-unit        # unit + conformance + property tests (any platform)
cargo xtask test-conformance # OCI adapter conformance matrix
just test-integration        # cgroup tests (Linux + root)
just test-e2e                # daemon + CLI end-to-end (Linux + root)
```

The conformance suite runs 28 backend-agnostic tests against every adapter. Unit tests run on
macOS without root. See [`docs/TEST_INFRASTRUCTURE.mbx.md`](docs/TEST_INFRASTRUCTURE.mbx.md).

---

## Developer Workflow

```bash
cargo xtask pre-commit       # fmt + clippy + release build (macOS-safe gate)
cargo xtask prepush          # nextest + coverage (Linux gate)
just --list                  # all available recipes
mbx doctor                   # preflight: show compiled adapters and capabilities
```

See [`DEVELOPMENT.md`](DEVELOPMENT.md) for the full workflow.

---

## Contributing

Issues and PRs are welcome. A few things to know before contributing:

- Run `cargo xtask pre-commit` before pushing — it's the same gate as CI.
- New adapters implement the domain traits in `minibox-core/src/domain.rs`.
- Protocol changes start in `minibox-core/src/protocol.rs`; update handlers, CLI paths, and
  snapshot tests together.
- Linux-only code must be gated with `#[cfg(target_os = "linux")]` so macOS `cargo check`
  still passes.
- No `.unwrap()` in production paths — use `.context("description")?`.

---

## Roadmap

Planned and in-progress work includes seccomp/capabilities, rootless support, port
forwarding/DNS, Windows WSL2, and an MCP control surface. OCI push/commit/build and bridge
networking are experimental. Full details: [`docs/ROADMAP.mbx.md`](docs/ROADMAP.mbx.md).
Feature status by adapter: [`docs/FEATURE_MATRIX.mbx.md`](docs/FEATURE_MATRIX.mbx.md).

---

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.

<sup>Previously named `linuxbox` and `mbx` during early development.</sup>
