# minibox

> Terminal‑first tooling for sandboxed dev environments on macOS, Linux, and Windows.

> Disclaimer: I primarily use the CLI and try to keep parity for the TUI but no promises.  
> Designed to be a solid tool/command/skill target for AI agents.

`minibox` is a workspace of Rust crates that provide a unified daemon (`miniboxd`), platform shims, and a shared core library for building sandboxed development workflows.

## Features

- **Unified binary (`miniboxd`)** – Single entrypoint that selects platform‑specific backends behind compile‑time cfg gates.
- **Platform shims** – `macbox`, `winbox`, and `daemonbox` hide OS differences behind a stable interface.
- **Core library (`linuxbox`)** – Platform‑agnostic crate shared by the daemon, CLI, and benchmark tooling.
- **JSON CLI (`minibox-cli`)** – Thin, platform‑agnostic client that speaks JSON over pipes/sockets.
- **Bench tooling (`minibox-bench`)** – Focused crate for performance exploration and regression tracking.
- **Proc‑macros (`minibox-macros`)** – Ergonomic proc‑macros used by `linuxbox` for internal APIs.

<!--
[![CI](https://github.com/89jobrien/minibox/actions/workflows/ci.yml/badge.svg)](https://github.com/89jobrien/minibox/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/89jobrien/minibox/branch/main/graph/badge.svg)](https://codecov.io/gh/89jobrien/minibox)
[![dependency status](https://deps.rs/repo/github/89jobrien/minibox/status.svg)](https://deps.rs/repo/github/89jobrien/minibox)
-->

A Docker-like container runtime written in Rust. Daemon/client architecture with OCI image pulling, Linux namespace isolation, cgroups v2 resource limits, overlay filesystem, and hexagonal architecture for cross-platform adapter swapping.

**Status:** Development

## Supported

- Linux: native 
- Colima: mac/linux runtime, Docker alt
- GKE: unprivileged deployment

## On Deck

- WSL2: Everything is wired or at least stubbed. 
- MacOS: Virtualization framework for native macos

## Backburner

- Windows: WSL is best bet. I planned to do full native win

---

## Contents

- [Quick Start](#quick-start)
- [Crate Structure](#crate-structure)
- [Architecture](#architecture)
- [Platform Support](#platform-support)
- [CLI Reference](#cli-reference)
- [Testing](#testing)
- [Security](#security)
- [Current Limitations](#current-limitations)
- [Extending](#extending)
- [Development](#development)

---

## Quick Start

```bash
# Build (Linux required for daemon)
cargo build --release

# Start daemon (requires root)
sudo ./target/release/miniboxd

# Pull and run
sudo ./target/release/minibox pull alpine
sudo ./target/release/minibox run alpine -- /bin/echo "Hello from minibox!"
```

**Systemd deployment:**

```bash
sudo ./ops/install-systemd.sh
sudo systemctl enable --now miniboxd
sudo /usr/local/bin/minibox ps
```

---

## Crate Structure

| Crate            | Type    | Description                                            |
| ---------------- | ------- | ------------------------------------------------------ |
| `linuxbox`    | Library | Domain layer, adapters, image management, protocol     |
| `minibox-macros` | Library | `adapt!`, `as_any!`, `default_new!` boilerplate macros |
| `daemonbox`      | Library | Handler, state, Unix socket server (extracted from miniboxd) |
| `miniboxd`       | Binary  | Async daemon — Unix socket listener, platform dispatch |
| `macbox`         | Library | macOS daemon implementation (Colima adapter suite)     |
| `winbox`         | Library | Windows daemon implementation (stub)                   |
| `minibox-cli`    | Binary  | CLI client                                             |
| `minibox-llm`   | Library | Multi-provider LLM client with structured output       |
| `minibox-bench`  | Binary  | Benchmark harness                                      |

**Key modules in `linuxbox`:**

| Module         | Purpose                                                                                   |
| -------------- | ----------------------------------------------------------------------------------------- |
| `domain.rs`    | Port traits: `ImageRegistry`, `FilesystemProvider`, `ResourceLimiter`, `ContainerRuntime` |
| `adapters/`    | Concrete adapter implementations + mocks                                                  |
| `container/`   | Namespace setup, cgroups, overlay FS, process spawn                                       |
| `image/`       | Docker Hub v2 API client, OCI manifest parsing, tar extraction                            |
| `protocol.rs`  | JSON-over-newline request/response types                                                  |
| `preflight.rs` | Host capability probing (`just doctor`)                                                   |

---

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│                    Hexagonal Architecture                  │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  ┌─────────────┐   JSON/Unix    ┌──────────────────────┐   │
│  │   minibox   │ ─────────────▶ │      miniboxd        │   │
│  │   (CLI)     │                │                      │   │
│  └─────────────┘                │  ┌────────────────┐  │   │
│                                 │  │    Handlers    │  │   │
│                                 │  └───────┬────────┘  │   │
│                                 │          │           │   │
│                                 │  ┌───────▼────────┐  │   │
│                                 │  │  Domain Traits │  │   │
│                                 │  │   (Ports)      │  │   │
│                                 │  └───────┬────────┘  │   │
│                                 │          │           │   │
│                                 │  ┌───────▼────────┐  │   │
│                                 │  │   Adapters     │  │   │
│                                 │  │ DockerHub      │  │   │
│                                 │  │ OverlayFS      │  │   │
│                                 │  │ CgroupsV2      │  │   │
│                                 │  │ LinuxRuntime   │  │   │
│                                 │  │ ProotRuntime   │  │   │
│                                 │  └────────────────┘  │   │
│                                 └──────────────────────┘   │
└────────────────────────────────────────────────────────────┘
```

The domain layer has zero infrastructure dependencies. Adapters are swapped at daemon startup via `MINIBOX_ADAPTER`. Tests use `MockRegistry`, `MockFilesystem`, `MockLimiter`, `MockRuntime` from `adapters/mocks.rs`.

**Async/sync boundary:** Tokio handles socket I/O; container operations (fork/clone) run in `spawn_blocking`.

---

## Platform Adapter Selection

At startup, `miniboxd` detects the host platform and delegates to the appropriate
platform crate. Within each platform crate, `preflight()` checks which backends
are available and selects one — either via the `MINIBOX_ADAPTER` env var (explicit)
or by capability probing (auto). A fatal error is reported before the socket is
bound if no backend is available.

```
miniboxd starts
      │
      ├─── Linux ──────────────────────────────────────────────┐
      │      │                                                 │
      │    MINIBOX_ADAPTER?                                    │
      │      ├── native (default) → namespaces + cgroups v2    │
      │      ├── docker                                        │
      │      ├── gke              → proot + copy FS            │
      │      └── colima           → Colima/limactl delegate    │
      │                                                        │
      ├─── macOS ───────────────────────────────────────────── ┤
      │      │                                                 │
      │    macbox::preflight()                                 │
      │      ├── MINIBOX_ADAPTER=vf  OR  VF available  ───────►│ Virtualization.framework
      │      ├── MINIBOX_ADAPTER=colima  OR  Colima running ──►│ Colima delegate
      │      └── neither ──────────────────────────────────── ►│ FATAL: no backend
      │                                                        │
      └─── Windows ─────────────────────────────────────────── ┘
             │
           winbox::preflight()
             ├── MINIBOX_ADAPTER=hcs   OR  HCS available  ───► HCS (Windows Containers)
             ├── MINIBOX_ADAPTER=wsl2  OR  WSL2 available ───► WSL2 delegate
             └── neither ─────────────────────────────────── ► FATAL: no backend
```

## Platform Support

### Adapter Wiring Status

| Adapter Suite        | `MINIBOX_ADAPTER`  | Wired into daemon | Status       |
| -------------------- | ------------------ | ----------------- | ------------ |
| Native Linux         | `native` (default) | ✅ Yes            | Production   |
| GKE unprivileged     | `gke`              | ✅ Yes            | Production   |
| macOS Colima         | `colima`           | ✅ Yes            | Production   |
| macOS Docker Desktop | `docker-desktop`   | ❌ No             | Library only |
| Windows WSL2         | `wsl`              | ❌ No             | Library only |

Passing an unwired value causes the daemon to exit at startup with an error.

---

### Linux (Native)

**Requirements:** Linux 5.0+ (4.0+ minimum), cgroups v2, overlayfs, root.

| Adapter                 | Implementation                         |
| ----------------------- | -------------------------------------- |
| `DockerHubRegistry`     | Docker Hub v2 API with anonymous auth  |
| `OverlayFilesystem`     | Linux overlayfs via `mount()`          |
| `CgroupV2Limiter`       | cgroups v2 unified hierarchy           |
| `LinuxNamespaceRuntime` | `clone()` syscall with namespace flags |

---

### GKE (Unprivileged Pods)

Standard GKE pods lack `CAP_SYS_ADMIN`, which blocks `mount()`, `pivot_root()`, namespace-flagged `clone()`, and cgroup writes. The GKE adapter suite works within those constraints:

| Adapter          | Implementation                              |
| ---------------- | ------------------------------------------- |
| `ProotRuntime`   | ptrace-based fake chroot via `proot` binary |
| `CopyFilesystem` | Plain file copy instead of overlay mount    |
| `NoopLimiter`    | No-op (cgroup access unavailable)           |

```bash
MINIBOX_ADAPTER=gke miniboxd
MINIBOX_PROOT_PATH=/usr/local/bin/proot MINIBOX_ADAPTER=gke miniboxd
```

**Requirements:** GKE Standard cluster (not Autopilot), `proot` binary in container image.

---

### macOS (Colima) — Library only

`ColimaRegistry`, `ColimaRuntime`, `ColimaFilesystem`, `ColimaLimiter` are implemented, tested, and wired into the daemon.

**Requirements (when wired):** `brew install colima`, `colima start`.

- `ColimaRegistry` — image ops via `nerdctl`, layers exported to Lima-shared `/tmp/minibox-layers/`
- `ColimaRuntime` — container spawn via `limactl shell` + chroot
- `ColimaFilesystem` / `ColimaLimiter` — overlay and cgroups via limactl

---

### macOS (Docker Desktop) / Windows (WSL2) — Library only

Adapters are implemented in `linuxbox` but not yet wired into `miniboxd`. `MINIBOX_ADAPTER=docker-desktop` and `MINIBOX_ADAPTER=wsl` are not currently accepted by the daemon.

---

## CLI Reference

```bash
# Pull an image
sudo minibox pull alpine
sudo minibox pull ubuntu -t 22.04

# Run a container
sudo minibox run alpine -- /bin/echo "Hello!"
sudo minibox run alpine --memory 536870912 --cpu-weight 500 -- /bin/sh

# List running containers
sudo minibox ps

# Stop / remove
sudo minibox stop <container_id>
sudo minibox rm <container_id>
```

**Daemon flags:**

```bash
sudo miniboxd                          # default (native adapter)
RUST_LOG=debug sudo miniboxd           # verbose logging
MINIBOX_ADAPTER=gke miniboxd          # GKE adapter
```

**Resource limit flags:**

| Flag           | Type    | Default   | Notes                       |
| -------------- | ------- | --------- | --------------------------- |
| `--memory`     | bytes   | unlimited | e.g. `536870912` for 512 MB |
| `--cpu-weight` | 1–10000 | 100       | relative CPU share          |

---

## Testing

```bash
# Unit + protocol tests (any platform)
cargo test -p linuxbox

# All tests (Linux)
cargo test --workspace

# Integration tests — cgroup/namespace, requires Linux + root
just test-integration

# E2E daemon + CLI suite, requires Linux + root
just test-e2e

# Preflight check
just doctor

# Benchmarks (any platform, no daemon needed)
cargo xtask bench --suite codec    # 36 protocol encode/decode benchmarks
cargo xtask bench --suite adapter  # 10 trait-overhead benchmarks
cargo bench -p linuxbox         # Criterion HTML reports (local only)
```

**Current counts:** 221 unit + conformance + property (any platform), 16 cgroup integration (Linux+root), 14 E2E (Linux+root).

See `TESTING.md` for full strategy. See `CLAUDE.md` for macOS-specific compile guards.

---

## Security

### What's hardened

| Area           | Protection                                                             |
| -------------- | ---------------------------------------------------------------------- |
| Path traversal | `canonicalize()` + `..` rejection in overlay FS and tar extraction     |
| Tar extraction | Rejects `..`, absolute symlinks, device nodes, strips setuid/setgid    |
| Socket auth    | `SO_PEERCRED` — UID 0 only, socket mode `0600`                         |
| DoS limits     | 1 MB request max, 10 MB manifest max, 1 GB per layer, 5 GB total image |
| Mount flags    | `MS_NOSUID`, `MS_NODEV`, `MS_NOEXEC`                                   |
| PID limit      | 1024 per container (default)                                           |

### Remaining work

- Capability dropping (`CAP_SYS_ADMIN` etc.)
- Seccomp filters
- User namespace remapping
- Request rate limiting

See `SECURITY.md` for threat model, `SECURITY_FIXES.md` for full audit.

---

## Current Limitations

- **No networking** — containers get an isolated netns but no bridge/veth configuration
- **No exec** — cannot run commands in existing containers
- **No log capture** — container output not stored
- **No persistent state** — daemon restart loses all container records
- **Root required** — no rootless support
- **No Dockerfile** — image-only workflow

---

## Extending

Domain traits are already defined for upcoming features. Adding a capability means implementing the trait and wiring the adapter:

| Trait              | Adapter needed      | Notes                      |
| ------------------ | ------------------- | -------------------------- |
| `BridgeNetworking` | Linux bridge + veth |                            |
| `PseudoTerminal`   | `/dev/pts`          |                            |
| `ContainerExec`    | `setns` syscall     |                            |
| `LogStore`         | JSON-lines file     |                            |
| `StateStore`       | SQLite / sled       | replaces in-memory HashMap |

Trait definitions live in `crates/linuxbox/src/domain.rs`.

---

## Development

**Requirements:** Rust 1.85+, Linux 4.0+ (5.0+ recommended), cgroups v2, root.

```bash
# Verify kernel features
mount | grep cgroup2
ls /proc/self/ns/
lsmod | grep overlay

# Build
cargo build --release              # Linux full build
cargo build -p linuxbox         # macOS/Windows (lib only)
cargo check --workspace            # fast type check

# Lint
cargo clippy --workspace -- -D warnings
cargo deny check
```

**Environment variables:**

| Variable              | Default                                         | Purpose                          |
| --------------------- | ----------------------------------------------- | -------------------------------- |
| `MINIBOX_ADAPTER`     | `native`                                        | Adapter suite selection          |
| `MINIBOX_DATA_DIR`    | `/var/lib/minibox`                              | Image + container storage        |
| `MINIBOX_RUN_DIR`     | `/run/minibox`                                  | Socket + runtime state           |
| `MINIBOX_CGROUP_ROOT` | `/sys/fs/cgroup/minibox.slice/miniboxd.service` | Cgroup root                      |
| `RUST_LOG`            | —                                               | Tracing log level (e.g. `debug`) |

See `CLAUDE.md` for full development guide, debugging tips, and architecture details.

