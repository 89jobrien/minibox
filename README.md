# minibox

A Docker-like container runtime written in Rust featuring hexagonal architecture, comprehensive security hardening, and cross-platform support.

**Architecture:** Daemon/client with OCI image pulling, Linux namespace isolation, cgroups v2 resource limits, overlay filesystem, and GKE unprivileged deployment support.

**Status:** Development - Security hardened (12/15 vulnerabilities fixed, zero dependencies with CVEs), 57 tests (36 unit + 21 protocol), performance validated (<5ns trait overhead), architecture validated by production frameworks.

## Quick Start

```bash
# Build (Linux)
cargo build --release

# Start daemon (requires root)
sudo ./target/release/miniboxd

# Pull and run
sudo ./target/release/minibox pull alpine
sudo ./target/release/minibox run alpine -- /bin/echo "Hello from minibox!"
```

## Ops Runtime (systemd)

```bash
# Build
cargo build --release

# Install binary + systemd unit
sudo ./ops/install-systemd.sh

# Enable and start
sudo systemctl enable --now miniboxd

# Verify
sudo systemctl status miniboxd --no-pager
sudo /usr/local/bin/minibox ps
```

## Features

### Core Capabilities

- **Container Isolation** - Linux namespaces (PID, Mount, UTS, IPC, Network)
- **Resource Limits** - cgroups v2 (memory, CPU weight, PID limits, I/O throttling)
- **Image Management** - OCI image pulling from Docker Hub with manifest list resolution
- **Overlay Filesystem** - Copy-on-write layered rootfs
- **Security Hardened** - Path validation, tar extraction safety, socket authentication

### Architecture

**Hexagonal Architecture** (Ports & Adapters):

- Domain layer with zero infrastructure dependencies
- Swappable adapters for registry, filesystem, cgroups, runtime
- 100% unit test coverage with mock implementations
- Cross-platform foundation (Linux native, GKE unprivileged, Windows WSL2, macOS Docker Desktop/Colima)
- Architecture pattern validated by production frameworks (Zombienet-SDK)

**Performance:**

- Trait object overhead: 1-5 nanoseconds (validated by benchmarks)
- 0.000001% impact on real operations (image pulls, container spawns)
- Validated by production frameworks (Zombienet-SDK uses identical pattern)

**Testing:**

- 36 unit tests (platform-agnostic with mocks)
- 21 protocol serialization tests (JSON encoding/decoding)
- 11 integration tests (Linux with real infrastructure)
- Conformance tests (cross-platform behavioral parity)
- Benchmark suite for performance validation
- Security scanning (cargo-deny, cargo-audit, clippy)

**Security Monitoring:**

- Zero dependency vulnerabilities (cargo-deny daily scans)
- All licenses compliant (MIT, Apache-2.0, BSD-3-Clause)
- Continuous security scanning via GitHub Actions
- Static analysis with security-focused lints

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                     Hexagonal Architecture                    │
├──────────────────────────────────────────────────────────────┤
│                                                               │
│  ┌─────────────┐         ┌──────────────────────────┐        │
│  │   CLI       │         │      Daemon              │        │
│  │  (minibox)  │  JSON   │   (miniboxd)             │        │
│  │             │ ─────▶  │                          │        │
│  └─────────────┘  Unix   │   ┌──────────────────┐   │        │
│                   Socket │   │    Handlers      │   │        │
│                          │   │  (Business Logic)│   │        │
│                          │   └────────┬─────────┘   │        │
│                          │            │             │        │
│                          │   ┌────────▼─────────┐   │        │
│                          │   │  Domain Traits   │   │        │
│                          │   │    (Ports)       │   │        │
│                          │   └────────┬─────────┘   │        │
│                          │            │             │        │
│                          │   ┌────────▼─────────┐   │        │
│                          │   │    Adapters      │   │        │
│                          │   │  (Infrastructure)│   │        │
│                          │   │                  │   │        │
│                          │   │ • DockerHub      │   │        │
│                          │   │ • OverlayFS      │   │        │
│                          │   │ • CgroupsV2      │   │        │
│                          │   │ • LinuxRuntime   │   │        │
│                          │   │ • ProotRuntime   │   │        │
│                          │   │ • CopyFilesystem │   │        │
│                          │   │ • NoopLimiter    │   │        │
│                          │   └──────────────────┘   │        │
│                          └──────────────────────────┘        │
│                                                               │
└──────────────────────────────────────────────────────────────┘
```

## Crate Structure

| Crate         | Type    | Description                                          |
| ------------- | ------- | ---------------------------------------------------- |
| `minibox-lib` | Library | Domain layer, adapters, infrastructure (2,491 lines) |
| `miniboxd`    | Binary  | Async daemon with handler logic                      |
| `minibox-cli` | Binary  | CLI client                                           |

**Key Modules:**

- `domain.rs` - Pure business logic traits (ImageRegistry, FilesystemProvider, ResourceLimiter, ContainerRuntime)
- `adapters/` - Infrastructure implementations (registry, filesystem, limiter, runtime, mocks, GKE, WSL, Docker Desktop, Colima)
- `handlers/` - Request handling with dependency injection
- `protocol.rs` - JSON-over-newline communication protocol

## Platform Support

### Linux (Native)

**Requirements:**

- Linux kernel 5.0+ (4.0+ minimum)
- cgroups v2 unified hierarchy
- Overlay filesystem support
- Root privileges

**Adapters:**

- `DockerHubRegistry` - Docker Hub v2 API
- `OverlayFilesystem` - Linux overlayfs
- `CgroupV2Limiter` - cgroups v2
- `LinuxNamespaceRuntime` - clone() syscall

### GKE (Unprivileged Pods)

**Requirements:**

- GKE Standard cluster (Autopilot not supported)
- Linux container image with `proot` binary
- No `CAP_SYS_ADMIN` needed

**Adapters:**

- `ProotRuntime` - ptrace-based fake chroot via proot (no namespaces or pivot_root needed)
- `CopyFilesystem` - Copy-based layer merging (no overlay FS needed)
- `NoopLimiter` - No-op resource limiter (no cgroup access)

**Configuration:**

```bash
# Select GKE adapter at daemon startup
MINIBOX_ADAPTER=gke miniboxd

# Or specify proot binary location
MINIBOX_PROOT_PATH=/usr/local/bin/proot MINIBOX_ADAPTER=gke miniboxd
```

**How it works:**
Standard GKE pods lack `CAP_SYS_ADMIN`, blocking `mount()`, `pivot_root()`, `clone()` with namespace flags,
overlay FS, and cgroup writes. The GKE adapter suite works within those constraints by using proot's ptrace-based
syscall interception for fake chroot, plain file copying instead of overlay mounts, and skipping cgroup resource
limits entirely. The same minibox binary runs in both native and GKE modes -- no recompilation needed.

### Adapter Wiring Status

| Adapter Suite | `MINIBOX_ADAPTER` value | Daemon wired? | Status |
|---------------|------------------------|---------------|--------|
| Native Linux  | `native` (default)     | ✅ Yes         | Production |
| GKE (proot)   | `gke`                  | ✅ Yes         | Production |
| macOS Colima  | `colima`               | ⚙️ In progress | Library only — not yet accepted by daemon |
| macOS Docker Desktop | `docker-desktop` | ❌ No       | Library only — not yet accepted by daemon |
| Windows WSL2  | `wsl`                  | ❌ No          | Library only — not yet accepted by daemon |

Passing an unwired value to `MINIBOX_ADAPTER` causes the daemon to exit at startup with an unrecognized adapter error.

### Windows (WSL2)

> **Status: Library only** — `WslRuntime`, `WslFilesystem`, `WslLimiter` are implemented in `minibox-lib` but not yet wired into `miniboxd`. `MINIBOX_ADAPTER=wsl` is not currently accepted.

**Requirements (when wired):**

- Windows 10/11 with WSL2
- Ubuntu 20.04+ distribution
- minibox-wsl-helper binary in WSL

**Adapters:**

- `WslRuntime` - Delegates to WSL Linux environment
- `WslFilesystem` - Overlay operations via WSL
- `WslLimiter` - cgroups via WSL

### macOS (Docker Desktop)

> **Status: Library only** — `DockerDesktopRuntime`, `DockerDesktopFilesystem`, `DockerDesktopLimiter` are implemented in `minibox-lib` but not yet wired into `miniboxd`. `MINIBOX_ADAPTER=docker-desktop` is not currently accepted.

**Requirements (when wired):**

- macOS 10.15+ (Catalina)
- Docker Desktop 4.0+
- minibox-docker-helper container

**Adapters:**

- `DockerDesktopRuntime` - Delegates to Docker VM
- `DockerDesktopFilesystem` - Operations in helper container
- `DockerDesktopLimiter` - cgroups in helper container

### macOS (Colima)

> **Status: In progress** — `ColimaRegistry`, `ColimaRuntime`, `ColimaFilesystem`, `ColimaLimiter` are implemented and tested in `minibox-lib`. Wiring into `miniboxd` is in progress; `MINIBOX_ADAPTER=colima` is not yet accepted.

**Requirements (when wired):**

- macOS 10.15+ (Catalina)
- Colima installed (`brew install colima`)
- Colima VM running (`colima start`)

**Adapters:**

- `ColimaRegistry` - Image operations via `nerdctl` in Lima VM; layers exported to `/tmp/minibox-layers/` (Lima-shared path accessible from macOS host)
- `ColimaRuntime` - Container spawn via `limactl shell` + chroot in VM; args passed correctly via `mapfile`
- `ColimaFilesystem` - Overlay operations via limactl
- `ColimaLimiter` - cgroups via limactl

**Advantages:**

- Fully open-source (no Docker Desktop licensing)
- Lightweight VM compared to Docker Desktop
- Native containerd/nerdctl integration

## Building

```bash
# Linux (full build)
cargo build --release

# macOS/Windows (cross-platform code only)
cargo build -p minibox-lib

# Benchmarks
cargo bench -p minibox-lib --bench trait_overhead

# Tests
cargo test --workspace                          # Unit tests
sudo -E cargo test -- --ignored --test-threads=1  # Integration tests (Linux)
```

## Usage

### Daemon

```bash
# Start daemon (Linux)
sudo ./target/release/miniboxd

# With debug logging
sudo RUST_LOG=debug ./target/release/miniboxd
```

**Daemon listens on:** `/run/minibox/miniboxd.sock`

### CLI Commands

```bash
# Pull images
sudo ./target/release/minibox pull alpine
sudo ./target/release/minibox pull ubuntu -t 22.04

# Run containers
sudo ./target/release/minibox run alpine -- /bin/echo "Hello!"
sudo ./target/release/minibox run alpine --memory 512M --cpu-weight 500 -- /bin/sh

# List containers
sudo ./target/release/minibox ps

# Stop/remove
sudo ./target/release/minibox stop <container_id>
sudo ./target/release/minibox rm <container_id>
```

### Resource Limits

```bash
# Memory limit (bytes)
--memory 536870912  # 512MB

# CPU weight (1-10000, default 100)
--cpu-weight 500    # 50% of default CPU share
```

## Security

### Fixed Vulnerabilities (12/15)

**Critical (CVSS 7.5-9.8):**

- [FIXED] Path traversal in overlay filesystem (CVSS 9.8)
- [FIXED] Symlink attack in tar extraction (CVSS 9.6)
- [FIXED] No Unix socket authentication (CVSS 7.8)
- [FIXED] Unlimited image pull sizes (CVSS 7.5)

**High (CVSS 7.0-7.9):**

- [FIXED] Missing cgroup PID/IO limits (CVSS 7.5)
- [FIXED] Insecure mount flags (CVSS 7.8)
- [FIXED] ImageStore path validation (CVSS 7.6)
- [FIXED] HTTPS enforcement for registry (CVSS 7.4)
- [FIXED] Directory permission issues (CVSS 7.1)
- [FIXED] Concurrent spawn DoS (CVSS 7.5)

**Medium (CVSS 6.0-6.9):**

- [FIXED] Request size DoS (CVSS 6.2)
- [FIXED] Container ID collisions

### Security Features

**Input Validation:**

- Path canonicalization with `..` rejection
- Tar entry validation (no Zip Slip attacks)
- Request size limits (1MB max)
- Image size limits (10GB per layer)

**Authentication:**

- SO_PEERCRED Unix socket authentication
- Root-only daemon access (UID 0)
- Socket permissions: 0600

**Isolation:**

- Mount flags: MS_NOSUID, MS_NODEV, MS_NOEXEC
- Read-only /sys mount
- PID limit: 1024 (default, prevents fork bombs)
- I/O bandwidth throttling support

**Continuous Security:**

- Daily automated vulnerability scans (cargo-deny)
- GitHub Actions CI security pipeline
- Static analysis with security-focused lints (clippy)
- Dependency license compliance checks
- Zero known CVEs in dependencies

**Remaining Work:**

- Capability dropping (CAP_SYS_ADMIN, etc.)
- Seccomp filters
- User namespace support
- Request rate limiting

**Security Documentation:**

- `SECURITY.md` - Threat model and security architecture
- `SECURITY_FIXES.md` - Complete vulnerability audit
- `SECURITY_TESTING.md` - Security testing procedures and test cases
- `.github/workflows/security.yml` - Automated security scanning

## Testing

**Test Pyramid:**

```
              E2E Tests
         ┌─────────────────────────┐
         │   Conformance Tests     │  Cross-platform parity
         │  Integration Tests      │  Linux only, real infrastructure
         └─────────────────────────┘
    ┌──────────────────────────────────┐
    │ Unit Tests + Protocol Tests      │  Platform-agnostic, mocks
    └──────────────────────────────────┘
```

**Run Tests:**

```bash
# Unit tests (any platform)
cargo test --workspace

# Integration tests (Linux, requires root)
just test-integration

# E2E lifecycle test (Linux, requires root + network)
just test-e2e

# E2E daemon+CLI suite (Linux, requires root)
just test-e2e-suite

# Conformance tests (Linux)
cargo test -p miniboxd --test conformance_tests

# Security scans
cargo deny check
cargo clippy --workspace -- -D warnings

# Benchmarks
cargo bench -p minibox-lib --bench trait_overhead
```

**Test Results:** See `TEST_RESULTS.md` for detailed validation report.

**Testing Strategy:** See `TESTING.md` for comprehensive testing approach.

## Performance

**Hexagonal Architecture Overhead:** 1-5 nanoseconds per trait call

**Benchmark Results:**

- Registry: +4.5ns (+7.3%)
- Filesystem: +0.2ns (+0.5%)
- Limiter: -2.0ns (-5.4%, faster!)
- Runtime: +0.7ns (+2.4%)
- Arc clone: 3.5ns
- Downcast: 0.75ns

**Impact:** 0.000001% of real container operations (ms/sec scale)

See `BENCHMARK_RESULTS.md` for detailed analysis.

## Protocol

JSON-over-newline on Unix socket (`/run/minibox/miniboxd.sock`).

**Request Examples:**

```json
{"type":"Run","image":"alpine","tag":"latest","command":["/bin/sh"],"memory_limit_bytes":null,"cpu_weight":null}
{"type":"Pull","image":"ubuntu","tag":"22.04"}
{"type":"List"}
{"type":"Stop","id":"a1b2c3d4e5f6"}
{"type":"Remove","id":"a1b2c3d4e5f6"}
```

**Response Examples:**

```json
{"type":"ContainerCreated","id":"a1b2c3d4e5f6"}
{"type":"Success","message":"image alpine:latest pulled"}
{"type":"ContainerList","containers":[...]}
{"type":"Error","message":"container not found"}
```

## Directory Layout

| Path                                | Purpose                            |
| ----------------------------------- | ---------------------------------- |
| `/run/minibox/miniboxd.sock`        | Daemon Unix socket                 |
| `/run/minibox/containers/{id}/`     | Runtime state (PID files)          |
| `/var/lib/minibox/images/`          | Image layers + manifests           |
| `/var/lib/minibox/containers/{id}/` | Overlay dirs (merged, upper, work) |
| `/sys/fs/cgroup/minibox/{id}/`      | Per-container cgroups              |

## Container Lifecycle

1. CLI sends `Run` request to daemon over Unix socket
2. Daemon checks image cache, pulls from Docker Hub if missing
3. Creates overlay mount: `lowerdir=layers, upperdir=rw, workdir=work`
4. Forks child with `CLONE_NEWPID|CLONE_NEWNS|CLONE_NEWUTS|CLONE_NEWIPC|CLONE_NEWNET`
5. Child: creates cgroup → sets hostname → pivot_root → closes FDs → exec command
6. Parent: tracks PID, spawns reaper task
7. On exit: reaper updates state to Stopped

## Current Limitations

**v0.1 - Development:**

- No networking (containers get isolated netns but no bridge/veth)
- No user namespace remapping (runs as root)
- No Dockerfile/build support
- No persistent state (daemon restart loses containers)
- No interactive TTY (no I/O piping to CLI)
- No exec command
- No logs capture

**Note:** GKE adapter is fully wired and production-ready. WSL2, Docker Desktop, and Colima adapter suites are implemented in `minibox-lib` but not yet wired into `miniboxd` — see the [Adapter Wiring Status](#adapter-wiring-status) table.

## Extending

**Domain traits defined for:**

- [READY] Networking - Bridge, veth pairs, port mappings
- [READY] TTY Support - Pseudo-terminals for interactive shells
- [READY] Exec - Run commands in live containers
- [READY] Logs - Output capture and streaming
- [READY] State Store - Persistent container records

**Implementation required:**

- `BridgeNetworking` adapter (Linux bridge + veth)
- `PseudoTerminal` adapter (/dev/pts)
- `NamespaceExec` adapter (setns syscall)
- `FileLogStore` adapter (JSON lines)
- `SqliteStateStore` adapter (rusqlite)

See trait definitions in `crates/minibox-lib/src/domain/`.

## Documentation

### Core Documentation

- **README.md** - Project overview and quick start
- **CLAUDE.md** - Development guide, architecture, debugging

### Testing & Validation

- **TESTING.md** - Testing strategy and methodology
- **TEST_RESULTS.md** - Comprehensive test validation report
- **BENCHMARK_RESULTS.md** - Performance analysis and benchmarks

### Security

- **SECURITY.md** - Threat model and security architecture
- **SECURITY_FIXES.md** - Vulnerability audit and remediation
- **SECURITY_TESTING.md** - Security testing procedures

### Architecture

- **ZOMBIENET_PATTERNS.md** - Architectural validation from production frameworks

## Development

**Requirements:**

- Rust 1.75+
- Linux kernel 4.0+ (5.0+ recommended)
- cgroups v2 enabled
- Root access

**Recommended:**

```bash
# Check kernel features
grep CONFIG_USER_NS /boot/config-$(uname -r)
grep CONFIG_CGROUPS /boot/config-$(uname -r)
grep CONFIG_OVERLAY_FS /boot/config-$(uname -r)

# Verify cgroups v2
mount | grep cgroup2

# View daemon logs
RUST_LOG=debug sudo ./target/release/miniboxd
```

## Continuous Integration

**GitHub Actions Workflows:**

- **security.yml** - Daily security scanning
  - cargo-deny (dependency vulnerabilities)
  - cargo-audit (security advisories)
  - clippy (security-focused lints)
  - semgrep (static analysis)

**Automated Checks:**

- Pull request blocking on critical vulnerabilities
- License compliance verification
- Source validation (crates.io only)
- Multiple version detection

**Quality Gates:**

- All tests must pass
- Zero clippy warnings with security lints
- No known CVEs in dependencies
- All licenses approved

## Contributing

This is a learning/experimental project demonstrating:

- Hexagonal architecture in Rust
- Container runtime fundamentals
- Security-first development
- Comprehensive testing strategies

Pull requests welcome for:

- Feature implementations (networking, TTY, exec, logs)
- Security improvements
- Cross-platform support (GKE, WSL2/Docker Desktop helpers)
- Test coverage expansion

## License

MIT

## Acknowledgments

**Built with:**

- `tokio` - Async runtime
- `clap` - CLI parsing
- `serde` - Serialization
- `reqwest` - HTTP client
- `nix` - Unix syscalls
- `criterion` - Benchmarking
- `async-trait` - Async trait methods

**Inspired by:**

- Docker, Podman, and containerd - Container runtime design
- Zombienet-SDK
