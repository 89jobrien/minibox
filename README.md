# minibox

> Terminal-first tooling for sandboxed dev environments -- production on Linux,
> experimental on macOS, planned for Windows.

> Designed to be a solid tool/command/skill target for AI agents.

`minibox` is a workspace of Rust crates that provide a unified daemon (`miniboxd`), platform shims,
and a shared core library for building sandboxed development workflows.

The project is pushing toward agent-facing control surfaces, sandboxed code execution, and a
self-hosted CI flow that uses minibox to manage its own test environments. See `docs/ROADMAP.md`
for the active roadmap.

## Features

### Shipped (Linux native)

- **Unified binary (`miniboxd`)** – Single entrypoint; selects platform-specific backends behind
  compile-time cfg gates.
- **OCI image pull** – Docker Hub v2 API with anonymous auth; parallel layer pulls; ghcr.io support.
- **Run / stop / remove / list** – Full container lifecycle on Linux native and GKE adapter suites.
- **Named containers** – `--name` on `run`; name shown in `ps`; `exec` accepts names.
- **Container exec** – `minibox exec` / `-it` PTY — **Linux native only** (`setns`).
- **Log capture** – `minibox logs <id>` — **Linux native only**; stored stdout/stderr.
- **Image GC** – `minibox prune` / `minibox rmi`; lease-based GC; all adapter suites.
- **Bind mounts + privileged** – `-v` / `--mount`, `--privileged` — **Linux native only**.
- **Container events** – `minibox events` streams lifecycle events; all adapter suites.
- **Platform shims** – `macbox` (Colima + VZ.framework), `winbox` (stub).
- **Core library (`minibox`)** – Linux primitives, daemon handler/server/state; re-exports
  `minibox-core` for cross-platform use.
- **JSON CLI (`mbx`)** – Thin client over Unix socket.
- **Proc-macros (`minibox-macros`)** – `as_any!`, `adapt!`, `default_new!` for adapter boilerplate.

### Experimental (wired, limited coverage)

- **Bridge networking** – veth pairs, NAT via iptables DNAT; `MINIBOX_NETWORK_MODE=bridge`;
  Linux native only.
- **OCI push / commit / build** – `OciPushAdapter`, `overlay_commit_adapter`,
  `MiniboxImageBuilder`; Linux native only; no Dockerfile parser yet.
- **macOS Colima adapter** – `MINIBOX_ADAPTER=colima`; run/stop/ps work; exec/logs limited.
- **macOS VZ.framework adapter** – `MINIBOX_ADAPTER=vz`; requires `--features vz` and
  `cargo xtask build-vm-image`.
- **Observability** – OpenTelemetry OTLP (`feature = "otel"`); Prometheus `/metrics`
  (`feature = "metrics"`); compile-time opt-in.

### Tooling (not part of the runtime)

- **xtask** – Dev tool: pre-commit gate, test suites, conformance, build-vm-image.

### Not yet implemented

- Windows: `winbox` compiles but `start()` returns an error unconditionally.
- Port forwarding, in-container DNS.
- Rootless (user namespace remapping).
- Dockerfile parser.

[![CI](https://github.com/89jobrien/minibox/actions/workflows/ci.yml/badge.svg)](https://github.com/89jobrien/minibox/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/89jobrien/minibox/branch/main/graph/badge.svg)](https://codecov.io/gh/89jobrien/minibox)
[![dependency status](https://deps.rs/repo/github/89jobrien/minibox/status.svg)](https://deps.rs/repo/github/89jobrien/minibox)

A Docker-like container runtime written in Rust. Daemon/client architecture with OCI image pulling, Linux namespace isolation, cgroups v2 resource limits, overlay filesystem, and hexagonal architecture for cross-platform adapter swapping.

**Status:** Development (`v0.21.0`)

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

## Near-Term Roadmap

- MCP control surface: expose pull/run/ps/stop/rm cleanly enough for Claude-style agent workflows
- Docker parity: wire commit/build/push adapters end-to-end into `miniboxd` (conformance suite
  phases 1–3 shipped; adapter wiring is the remaining gap)
- Sandboxed AI execution: run generated scripts and tests inside disposable minibox containers
  instead of on the host
- CI dogfooding: let the CI agent provision, stream, and tear down its own minibox-managed test
  environment
- Windows: WSL2 remains the most practical path; native HCS is still secondary

---

## Contents

- [Quick Start](#quick-start)
- [Crate Structure](#crate-structure)
- [Architecture](#architecture)
- [Platform Support](#platform-support)
- [Platform Support (Detail)](#platform-support-detail)
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
sudo ./target/release/mbx pull alpine
sudo ./target/release/mbx run alpine -- /bin/echo "Hello from minibox!"
```

**Systemd deployment:**

```bash
sudo ./ops/install-systemd.sh
sudo systemctl enable --now miniboxd
sudo /usr/local/bin/mbx ps
```

**Current dogfood path:**

```bash
# Build the Linux test image used for macOS/Colima dogfooding
cargo xtask build-test-image

# Load a local OCI tarball into minibox
sudo ./target/release/mbx load ~/.minibox/test-image/minibox-tester.tar --name minibox-tester

# Run the Linux suite inside minibox
cargo xtask test-linux
```

---

## Crate Structure

| Crate             | Type    | Description                                                            |
| ----------------- | ------- | ---------------------------------------------------------------------- |
| `minibox-core`    | Library | Protocol, domain traits, OCI image types, client, error types          |
| `minibox`         | Library | Linux primitives, adapters, daemon handler/server/state                |
| `miniboxd`        | Binary  | Async daemon — Unix socket listener, platform dispatch                 |
| `mbx`             | Binary  | CLI client                                                             |
| `minibox-macros`  | Library | Proc macros (`as_any!`, `adapt!`, `default_new!`)                      |
| `minibox-testers` | Library | Test infrastructure — mocks, fixtures, conformance helpers             |
| `macbox`          | Library | macOS daemon (Colima adapter suite + VZ.framework + krun adapters)     |
| `winbox`          | Library | Windows daemon implementation (stub)                                   |

**Key modules in `minibox`:**

| Module         | Purpose                                                                                   |
| -------------- | ----------------------------------------------------------------------------------------- |
| `domain.rs`    | Port traits: `ImageRegistry`, `FilesystemProvider`, `ResourceLimiter`, `ContainerRuntime` |
| `adapters/`    | Concrete adapter implementations + mocks                                                  |
| `container/`   | Namespace setup, cgroups, overlay FS, process spawn                                       |
| `daemon/`      | Handler, state machine, Unix socket server, telemetry                                     |
| `preflight.rs` | Host capability probing (`just doctor`)                                                   |

---

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│                    Hexagonal Architecture                  │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  ┌─────────────┐   JSON/Unix    ┌──────────────────────┐   │
│  │     mbx     │ ─────────────▶ │      miniboxd        │   │
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
      │      ├── gke              → proot + copy FS            │
      │      └── colima           → Colima/limactl delegate    │
      │      (any other value causes daemon to exit at startup) │
      │                                                        │
      ├─── macOS ───────────────────────────────────────────── ┤
      │      │                                                 │
      │    macbox::preflight()                                 │
      │      ├── MINIBOX_ADAPTER=vz   OR  VZ available  ───────►│ Virtualization.framework (blocked)
      │      ├── MINIBOX_ADAPTER=colima  OR  Colima running ──►│ Colima delegate
      │      └── neither ──────────────────────────────────── ►│ FATAL: no backend
      │                                                        │
      └─── Windows (STUB — no runtime yet) ──────────────────── ┘
             │
           winbox::start() → returns error unconditionally
           (Future: HCS / WSL2 backends planned)
```

## Platform Support (Detail)

See the [Platform Support](#platform-support) table above for the status matrix.

Additional adapters (`docker_desktop`, `wsl2`, `vf`, `hcs`) exist as library code but are
**not wired** into the daemon. Passing an unrecognized `MINIBOX_ADAPTER` value causes the
daemon to exit at startup.

---

## CLI Reference

```bash
# Pull an image
sudo mbx pull alpine
sudo mbx pull ubuntu -t 22.04

# Run a container
sudo mbx run alpine -- /bin/echo "Hello!"
sudo mbx run alpine --memory 536870912 --cpu-weight 500 -- /bin/sh
sudo mbx run --name mybox alpine -- /bin/sh   # named container
sudo mbx run -it alpine -- /bin/sh            # interactive PTY
sudo mbx run -v /host/path:/container/path alpine -- /bin/sh  # bind mount

# List running containers
sudo mbx ps

# Exec into a running container
sudo mbx exec <container_id> -- /bin/sh
sudo mbx exec -it mybox -- /bin/bash          # interactive PTY

# Retrieve logs
sudo mbx logs <container_id>

# Stream lifecycle events
sudo mbx events

# Stop / remove
sudo mbx stop <container_id>
sudo mbx pause <container_id>
sudo mbx resume <container_id>
sudo mbx rm <container_id>

# Load local OCI tarball
sudo mbx load ./minibox-tester.tar --name minibox-tester

# Image management
sudo mbx prune          # GC unused images
sudo mbx rmi <image>    # remove specific image
```

**Daemon flags:**

```bash
sudo miniboxd                              # default (native adapter)
RUST_LOG=debug sudo miniboxd              # verbose logging
MINIBOX_ADAPTER=gke miniboxd             # GKE adapter
MINIBOX_ADAPTER=vz miniboxd              # macOS VZ.framework
MINIBOX_NETWORK_MODE=bridge miniboxd     # bridge networking
```

**Run flags:**

| Flag            | Type    | Default   | Notes                          |
| --------------- | ------- | --------- | ------------------------------ |
| `--memory`      | bytes   | unlimited | e.g. `536870912` for 512 MB    |
| `--cpu-weight`  | 1–10000 | 100       | relative CPU share             |
| `--name`        | string  | —         | assign a name to the container |
| `-it`           | flag    | false     | interactive PTY mode           |
| `-v`/`--volume` | string  | —         | bind mount (`host:container`)  |
| `--mount`       | string  | —         | mount spec (long form)         |
| `--privileged`  | flag    | false     | curated capability whitelist   |

---

## Testing

```bash
# Unit + protocol tests (any platform)
cargo test -p minibox

# All tests (Linux)
cargo test --workspace

# Integration tests — cgroup/namespace, requires Linux + root
just test-integration

# E2E daemon + CLI suite, requires Linux + root
just test-e2e

# VM suite — cross-compile aarch64-musl binaries + run inside QEMU Alpine VM (macOS)
just test-vm

# Conformance suite — backend-agnostic OCI commit/build/push matrix
cargo xtask test-conformance     # reports written to artifacts/conformance/

# Preflight check
just doctor

# Benchmarks (any platform, no daemon needed)
cargo xtask bench --suite codec    # 36 protocol encode/decode benchmarks
cargo xtask bench --suite adapter  # 10 trait-overhead benchmarks
cargo bench -p linuxbox        # Criterion HTML reports (local only)
```

**Current counts:** 1039 unit + conformance + property (any platform), 16 cgroup integration
(Linux+root), 14 E2E (Linux+root), 7 skipped (platform-gated).

**krun conformance tests** (macOS only) are opt-in: `MINIBOX_KRUN_TESTS=1 cargo nextest run -p macbox
--test krun_conformance_tests`.

**Fuzzing** (`fuzz/` harness, requires nightly):

```bash
cd fuzz
cargo +nightly fuzz run fuzz_decode_request    # arbitrary bytes → decode_request, never panics
cargo +nightly fuzz run fuzz_decode_response   # arbitrary bytes → decode_response, never panics
cargo +nightly fuzz run fuzz_extract_layer     # arbitrary bytes → extract_layer, escape-proof
cargo +nightly fuzz run fuzz_validate_layer_path  # arbitrary paths → validate_layer_path
```

See `CLAUDE.md` for full testing strategy and macOS-specific compile guards.

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
- Rootless support

See `CLAUDE.md` ("Security Considerations") for threat model details.

---

## Current Limitations

See `CLAUDE.md` ("Current Limitations") for the authoritative list. Key constraints:
root required, VZ.framework blocked upstream, push/commit/build not wired end-to-end,
no seccomp/capability dropping, no rootless support.

---

## Extending

Domain traits are defined as ports in `crates/minibox/src/domain.rs`. Adding a capability means
implementing the trait and wiring the adapter into `HandlerDependencies`.

| Trait                | Status  | Notes                                         |
| -------------------- | ------- | --------------------------------------------- |
| `BridgeNetworking`   | Shipped | veth + NAT, `MINIBOX_NETWORK_MODE=bridge`     |
| `ExecRuntime`        | Shipped | `setns`, PTY, stdin relay                     |
| `NetworkProvider`    | Shipped | None / Host / Bridge dispatch                 |
| `ImagePusher`        | Partial | OCI Distribution Spec — not wired in miniboxd |
| `ContainerCommitter` | Partial | Overlay upperdir snapshot — not wired         |
| `ImageBuilder`       | Partial | Basic Dockerfile subset — not wired           |
| `StateStore`         | Open    | SQLite / sled — replaces JSON-file persistence |

---

## Agent Direction

Minibox is increasingly shaped as infrastructure for agent workflows, not just a human CLI:

- the next layer is an MCP-friendly control surface so an agent can drive image pulls, container
  lifecycle, and log streaming directly
- the longer-term dogfood goal is to run agent-generated code and CI jobs inside minibox-managed
  containers by default

That work is tracked in `docs/ROADMAP.md`.

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
cargo build -p minibox         # macOS/Windows (lib only, Linux-only features excluded)
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

---

## Git Workflow (3-Tier Stability Pipeline)

Minibox uses a three-tier branching model designed for stability and Maestro integration:

```
feature/*  ──┐
hotfix/*   ──┤
chore/*    ──┴──► main (develop) ──auto──► next (validated) ──manual──► stable (release)
                                                                           │
                                                                       v* tag → GitHub Release
```

### Branch Purposes

| Branch                             | Purpose                                      | Deletion Policy     |
| ---------------------------------- | -------------------------------------------- | ------------------- |
| `main`                             | Active R&D. All feature work merges here     | Never deleted       |
| `next`                             | Auto-promoted from `main` when CI passes     | Never deleted       |
| `stable`                           | Maestro-consumable. Tagged releases cut here | Never deleted       |
| `feature/*`, `hotfix/*`, `chore/*` | Short-lived topic branches                   | Deleted after merge |

**Invariant:** Every commit on every branch must compile.

### CI Gates

**Local hooks (developer machine):**

| Hook       | Command                  | Enforces                            |
| ---------- | ------------------------ | ----------------------------------- |
| pre-commit | `cargo xtask pre-commit` | fmt-check + clippy + release build  |
| pre-push   | `cargo xtask prepush`    | nextest + llvm-cov + snapshot check |

**Remote CI (GitHub Actions):**

- **`main`**: `cargo check --workspace` + `cargo fmt --all --check` + clippy
- **`next`**: above + `cargo xtask test-unit` + audit/deny/machete + benchmarks
- **`stable`**: above + `cargo geiger` + manual review

### Auto-Promotion

- **main → next:** Automatic on green CI (fast-forward merge)
- **next → stable:** Manual `workflow_dispatch` (with full `stable`-tier gates)
- **Hotfixes:** Commits on `stable` with `[hotfix]` tag auto-backmerge to `next` and `main`

See `docs/superpowers/specs/2026-03-26-git-workflow-design.md` for full workflow specification.

---

See `CLAUDE.md` for full development guide, debugging tips, and architecture details.
