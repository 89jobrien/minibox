# minibox

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

> **Status**: Stabilization freeze active — see [CONTRIBUTING.md](CONTRIBUTING.md) and
> [docs/core/STABILITY_CHECKLIST.mbx.md](docs/core/STABILITY_CHECKLIST.mbx.md).

An agent-controllable container runtime written in Rust. Daemon/CLI split, OCI image pulling,
Linux namespace isolation, cgroups v2 resource limits, and overlay filesystem support. Hexagonal
architecture keeps adapter suites swappable at startup with no recompile. A built-in MCP stdio
server exposes policy-gated daemon operations directly to MCP clients so agents can drive container
lifecycle without shelling out to the CLI.

**Status:** Active development — `v0.32.0`. Linux runs natively and is production-ready; macOS feels like native but requires `smolvm`
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
- Image management — local `search`, `prune` / `rmi` with lease-based GC
- Bind mounts and privileged mode — `-v`/`--mount`, `--privileged`
- Log capture — `mbx logs <id>` for stored stdout/stderr
- Container events — `mbx events` streams lifecycle events

### Experimental-ish

- **Container exec** — `setns`-based exec with PTY support (`-it`); Linux (`native`) only
- **Bridge networking** — veth pairs, NAT via iptables DNAT (`MINIBOX_NETWORK_MODE=bridge`); Linux only
- **macOS adapters** — run/stop/ps via smolvm or krun (VM-backed); exec/logs not supported;
  Colima available as an alternative via Lima VM

---

## Quick Start

Requires Linux, root, kernel 5.0+, cgroups v2, overlay FS.

First-time contributors: run `just install-hooks` and `cargo xtask doctor` to verify your
toolchain and environment before building — see
[`docs/core/DEVELOPMENT.mbx.md`](docs/core/DEVELOPMENT.mbx.md). For per-environment usage
workflows (systemd, GKE, WSL2, Colima) see [`docs/core/USAGE.mbx.md`](docs/core/USAGE.mbx.md).

```bash
# Build
cargo build --release

# Start daemon
sudo ./target/release/miniboxd

# Pull and run
sudo ./target/release/mbx pull alpine
sudo ./target/release/mbx search alpine
sudo ./target/release/mbx run alpine -- /bin/echo "hello from minibox"

# Manage containers
sudo ./target/release/mbx ps
sudo ./target/release/mbx logs <id>
sudo ./target/release/mbx stop <id>
sudo ./target/release/mbx rm <id>

# Check compiled adapter info (no daemon needed)
./target/release/mbx doctor
```

### Commit and image volumes

`mbx commit <container> <image:tag>` captures the container writable layer. Paths declared by the source image with Dockerfile `VOLUME` (stored as `config.Volumes` in the image config) are excluded by default, matching container-runtime commit semantics. If an excluded path contains data, the command prints a warning naming the path. Use `--include-volumes` to deliberately capture that data:

```bash
sudo ./target/release/mbx commit <id> my-image:latest --include-volumes
```

Host bind-mount contents are never captured. `--include-volumes` is currently supported by the native overlay adapter; Colima rejects the option rather than silently ignoring it.

---

## Platform Support

| Platform              | Status         | Adapter         | Notes                                 |
| --------------------- | -------------- | --------------- | ------------------------------------- |
| Linux x86_64          | **Production** | `native`        | Full namespace/cgroup v2/overlay      |
| Linux aarch64         | **Production** | `native`        | Same as x86_64                        |
| Linux (GKE)           | **Production** | `gke`           | Unprivileged pods via proot + copy-FS |
| macOS (Apple Silicon) | Experimental   | `smolvm`/`krun` | exec/logs limited; VZ adapter removed |
| macOS (Intel)         | Experimental   | `colima`        | exec/logs limited                     |
| Windows               | Planned        | `winbox` stub   | Returns error unconditionally         |

See [`docs/core/FEATURE_MATRIX.mbx.md`](docs/core/FEATURE_MATRIX.mbx.md) for the full per-adapter capability
breakdown.

---

## Architecture

15 crates plus `xtask` (16 workspace members), Rust 2024 edition:

```
minibox-macros          proc macros (as_any!, adapt!)
    ^
minibox-core            cross-platform types, domain traits, protocol, OCI ops
    ^
minibox                 Linux adapters, daemon handler/server/state, test infra
    ^        ^        ^
macbox   smolbox   winbox  macOS Colima | macOS smolvm/krun | Windows stub
    ^        ^        ^
miniboxd                daemon entry point, adapter dependency injection

mbx                     CLI client — connects via Unix socket
minibox-tui             read-only TUI dashboard (ratatui) — live container table + event log
minibox-crux-plugin     crux agent bridge over JSON-RPC stdio
minibox-mcp             MCP stdio server for agent-controlled minibox tools
minibox-testsuite       conformance test harness for adapter trait contracts
minibox-bench           benchmark crate
minibox-cni             CNI plugin exec protocol and chain orchestration
ail                     placeholder crate
xtask                   CI gates, test runners, bench, VM image build
```

**Agentic tooling.** Three surfaces exist specifically for agent-driven operation, layered on
top of the same daemon protocol the CLI uses:

- `minibox-mcp` (`mcp` binary) — local MCP stdio server. Read-only inspection tools are exposed
  by default; run/pull/stop/rm are gated behind explicit agent policy.
- `minibox-crux-plugin` — bridges minibox into [crux](https://github.com/89jobrien/crux) agent
  pipelines over JSON-RPC stdio.
- `minibox-tui` (`mbx tui`) — read-only live dashboard (container table + streaming lifecycle
  events) for watching what an agent is doing to the daemon in real time.

[`agentbox`](agentbox/) (a separate Go module in this repo) is the multi-role review/task-decomposition
agent runtime used to develop minibox itself — not a minibox subcommand, but worth knowing about
if you're exploring the repo.

**Hexagonal ports.** Domain traits (`ImageRegistry`, `FilesystemProvider`, `ResourceLimiter`,
`ContainerRuntime`, `NetworkProvider`, …) live in `minibox-core`. Adapters implement them.
Tests use mock adapters — no real HTTP or filesystem required.

**Async/sync boundary.** Tokio handles socket I/O. Container operations (fork/clone/exec) run
in `spawn_blocking` to avoid blocking the runtime.

**Protocol.** JSON-over-newline on a Unix socket. 29 request variants, 28 response variants.
Canonical source: `crates/minibox-core/src/protocol.rs`.

Full architecture reference: [`docs/core/ARCHITECTURE.mbx.md`](docs/core/ARCHITECTURE.mbx.md).

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

## Backup Setup

This project has a local rustic repository configured by the SOPS-backed just recipes:

- `RUSTIC_REPOSITORY=/Users/joe/.local/share/rustic/minibox`
- `RUSTIC_PASSWORD_FILE=~/.config/rustic/password`

The repository password is stored as an encrypted SOPS secret in
`secrets/rustic.sops.yaml`. Do not commit the materialized password file under
`~/.config/rustic/password`; regenerate it from SOPS when needed:

```bash
just rustic-password-materialize
```

Initialize and verify the local repository with:

```bash
just rustic-init
just rustic-verify
```

The recipes default to the paths above and respect `RUSTIC_REPOSITORY` /
`RUSTIC_PASSWORD_FILE` overrides. `just rustic-verify` runs `rustic snapshots` and should
report that the repository password is correct. A newly initialized repository reports
`0 snapshot(s)`.

---

## Testing

```bash
cargo xtask test unit        # unit + conformance + property tests (any platform)
cargo xtask test conformance # OCI adapter conformance matrix
just test-integration        # cgroup tests (Linux + root)
just test-e2e                # daemon + CLI end-to-end (Linux + root)
```

The conformance suite runs 28 backend-agnostic tests against every adapter. Unit tests run on
macOS without root. See [`docs/core/TESTING.mbx.md`](docs/core/TESTING.mbx.md) for the full test
strategy and [`docs/core/TEST_INFRASTRUCTURE.mbx.md`](docs/core/TEST_INFRASTRUCTURE.mbx.md) for how
the harness is built.

---

## Developer Workflow

```bash
cargo xtask pre-commit       # staged fmt/clippy + config/docs checks
cargo xtask prepush          # release build + release nextest + conformance
just --list                  # all available recipes
mbx doctor                   # preflight: show compiled adapters and capabilities
```

See [`docs/core/DEVELOPMENT.mbx.md`](docs/core/DEVELOPMENT.mbx.md) for the full workflow.

---

## Contributing

Issues and PRs are welcome. A few things to know before contributing:

- Run `cargo xtask pre-commit` before committing and `cargo xtask prepush` before pushing.
- New adapters implement ports from `crates/minibox-domain/src/`; the
  `minibox_core::domain` facade preserves compatibility paths.
- Protocol changes start in `crates/minibox-core/src/protocol.rs`; update handlers, CLI paths, and
  snapshot tests together.
- Linux-only code must be gated with `#[cfg(target_os = "linux")]` so macOS `cargo check`
  still passes.
- No `.unwrap()` in production paths — use `.context("description")?`.

---

## Roadmap

| Feature                | Status                               |
| ---------------------- | ------------------------------------ |
| Bridge networking      | Experimental                         |
| OCI push/commit/build  | Experimental                         |
| macOS VZ.framework     | Removed after Apple ARM64 bug        |
| Seccomp / capabilities | Planned                              |
| Rootless support       | Planned                              |
| Port forwarding / DNS  | Planned                              |
| Windows (WSL2)         | Planned                              |
| MCP control surface    | Initial MCP stdio server implemented |

Full details: [`docs/core/ROADMAP.mbx.md`](docs/core/ROADMAP.mbx.md).

---

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.

<sup>Previously named `mbx` during early development.</sup>
