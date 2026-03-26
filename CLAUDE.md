# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Minibox is a Docker-like container runtime written in Rust featuring daemon/client architecture, OCI image pulling from Docker Hub, Linux namespace isolation, cgroups v2 resource limits, and overlay filesystem support.

## Rust Edition

This workspace uses **Rust 2024 edition**. Watch for: match ergonomics changes, `unsafe` required for `set_var`/`remove_var`, `FromRawFd` scope differences. Always run `cargo clippy` and `cargo test` after generating code.

## Build and Development Commands

### Python Scripts

All scripts in `scripts/` use `#!/usr/bin/env -S uv run` + PEP 723 inline deps. Run with `uv run scripts/foo.py` or directly if executable. Never use `python`/`python3` directly.

### AI Agent Scripts (Claude Agent SDK)

All `scripts/*.py` SDK scripts (`council.py`, `ai-review.py`, `meta-agent.py`, etc.) require an interactive terminal — they fail with "Command failed with exit code 1" when run via `run_in_background`. Always run them foreground/interactively.

### scripts/ conventions

- **Shared libraries** (`agent_log.py`, `bench_data.py`): no shebang, no PEP 723 block — imported by agent scripts via `sys.path.insert(0, os.path.dirname(__file__))`. All agent scripts must use `agent_log` for telemetry (never duplicate logging inline).
- **AI agent scripts** (`*-agent.py`, `council.py`, etc.): `#!/usr/bin/env -S uv run` + PEP 723 inline deps + `claude-agent-sdk` dependency. Always log via `agent_log.log_start`/`log_complete`.
- **TUI scripts** (`dashboard.py`): same uv pattern but use `rich` instead of `claude-agent-sdk`. Dashboard reads both `~/.mbx/agent-runs.jsonl` (agents) and `bench/results/` (benchmarks).

- `just meta-agent "task"` — design + spawn parallel agents for any task; fetches + caches SDK docs, discovers repo context
- `just council [base] [mode]` — multi-role branch analysis (core: 3 roles, extensive: 5 roles) + synthesis
- `just ai-review [base]` — security/correctness review of diff vs base branch
- `just gen-tests <TraitName>` — scaffold unit tests for a new domain trait adapter
- `just diagnose [--container <id>]` — diagnose container failure from logs + cgroup state
- `just bench-agent <subcmd>` — AI bench analysis: `report`, `compare [sha...]`, `regress`, `cleanup [--dry-run]`, `trigger [--vps]`
- `just sync-check` — fetch + rebase onto origin/main, auto-resolve obvious conflicts (wired into `just push`)
- `just commit-msg [--all]` — AI-generated conventional commit message from staged diff + commit history
- `mise run all:standup [-- N]` — time-block standup from git activity across ~/dev/ repos (N = hours, default 24)
- `mise run all:dashboard` — agent + bench dashboard (reads ~/.mbx/agent-runs.jsonl + bench/results/)

### mise.toml vs Justfile Convention

- **Justfile** — AI agent commands: build, lint, test gates, CI gates, AI agent scripts (`meta-agent`, `council`, `ai-review`, `gen-tests`, `diagnose`, `bench-agent`, `sync-check`, `commit-msg`), cleanup
- **mise.toml** — Human commands: interactive demos, ops tasks (`ssh-vps`, `fix-socket`, `smoke`), git ops (`commit`, `push`), human reports (`standup`, `dashboard`)

**mise.toml script gotcha:** Bash scripts with ANSI escape codes (e.g. `\033[36m`) **must** use `run = '''...'''` (TOML literal string) not `run = """..."""` — TOML interprets `\0` as an invalid escape in double-quoted strings.

### Justfile recipes

Recipes using bash-specific features (arrays, `local`, `declare`, functions) require a `#!/usr/bin/env bash` shebang as the first line — Just defaults to `sh` otherwise.

### Building

```bash
# Build all crates in workspace
cargo build --release

# Build specific crate
cargo build -p linuxbox
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
# On macOS, miniboxd dispatches to macbox::start() — full workspace builds.
# Use cargo xtask test-unit for the cross-platform unit test suite.

# Run all tests (requires Linux)
cargo test --workspace

# Run tests for specific crate
cargo test -p linuxbox

# Run specific test module
cargo test -p linuxbox protocol::tests

# Run with output
cargo test -- --nocapture

# Task runner (preferred for integration/e2e)
just test-unit          # unit tests, any platform
just test-integration   # cgroup tests, Linux+root
just test-e2e           # daemon+CLI tests, Linux+root
just doctor             # preflight capability check

# xtask (used directly by CI and just targets)
cargo xtask pre-commit      # fmt-check + lint + release build (macOS-safe)
cargo xtask prepush         # nextest + llvm-cov coverage
cargo xtask test-unit       # all unit + conformance tests
cargo xtask test-property   # property-based tests (proptest, any platform)
cargo xtask test-e2e-suite  # daemon+CLI e2e tests (Linux, root)
cargo xtask nuke-test-state # kill orphans, unmount overlays, clean cgroups/tmp
cargo xtask clean-artifacts # remove non-critical build outputs
cargo xtask bench           # run benchmark binary locally, saves to bench/results/bench.jsonl + latest.json
cargo xtask bench-vps               # run bench on VPS, fetch results (no git side-effects)
cargo xtask bench-vps --commit      # ... and commit results locally
cargo xtask bench-vps --commit --push  # ... and push to remote

# Run microbenchmarks (no daemon, any platform)
./target/release/minibox-bench --suite codec    # protocol encode/decode (nanosecond, 36 cases)
./target/release/minibox-bench --suite adapter  # trait-object overhead (nanosecond, 10 cases)
cargo bench -p linuxbox          # criterion benches (local HTML reports only, not saved)

# Bench result pipeline: bench.jsonl is append-only history; latest.json is canonical current
# snapshot for devloop — both must stay in sync (see save_bench_results in xtask/src/main.rs)
```

**Test Status:**

- Unit + conformance: 155 lib tests + 11 cli tests + 22 handler + 16 conformance + 13 minibox-llm + 36 minibox-secrets passing (257 total via nextest, 4 skipped on macOS)
- Property-based: 8 daemonbox proptest properties + 25 linuxbox property tests (`cargo xtask test-property`)
- Cgroup integration: 16 tests (Linux+root, `just test-integration`)
- E2E daemon+CLI: 14 tests (Linux+root, `just test-e2e`)
- Existing integration: 8 tests (Linux+root)
- Specs/plans: `docs/superpowers/specs/`, `docs/superpowers/plans/` — check `git log` to see if a plan was already executed before treating it as pending

**macOS quality gates** (`miniboxd` compiles via `macbox::start()` dispatch — `cargo check --workspace` works):

```bash
cargo fmt --all --check
cargo clippy -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -p minibox-llm -p minibox-secrets -- -D warnings
cargo xtask test-unit
```

## Architecture Overview

### Workspace Structure

Platform crates follow the `{platform}box` naming convention: `linuxbox` (Linux namespaces/cgroups), `macbox` (macOS Colima), `winbox` (Windows stub). All are platform-conditional deps in `miniboxd`.

Eleven crates in cargo workspace:

1. **minibox-core** (library): Cross-platform shared types — protocol, domain traits, error types, image management (`ImageStore`, `RegistryClient`), preflight; re-exported by linuxbox for macro compatibility
2. **linuxbox** (library): Linux-specific container primitives and adapters (namespaces, cgroups, overlay, process). Re-exports `minibox-core` — **do not remove re-exports** — `as_any!`/`adapt!` macros expand to `crate::domain::AsAny` at call sites inside linuxbox
   (formerly `minibox-lib` — renamed 2026-03-23; git history before this date uses the old name)
3. **minibox-macros** (proc-macro): Derive macros used by linuxbox
4. **daemonbox** (library): Handler, state, Unix socket server — extracted from miniboxd
4. **miniboxd** (binary): Async daemon entry point; dispatches to `macbox::start()` on macOS, `winbox::start()` on Windows
6. **macbox** (library): macOS daemon implementation (Colima adapter suite)
7. **winbox** (library): Windows daemon implementation (stub)
8. **minibox-cli** (binary): CLI client sending commands to daemon
9. **minibox-llm** (library): Multi-provider LLM client with structured output and fallback chains
10. **minibox-bench** (binary): Benchmark harness
11. **minibox-secrets** (library): Typed credential store — `CredentialProvider` port + adapters for env, OS keyring, 1Password (`op` CLI), and Bitwarden (`bw` CLI); SHA-256 audit hashes; expiry-aware provider chain

(`xtask` is also a workspace member but is a dev-tool, not a shipped crate)

### Critical Design Patterns

**Hexagonal Architecture**: Domain traits (`ResourceLimiter`, `FilesystemProvider`, `ContainerRuntime`, `ImageRegistry`) in `minibox-core/src/domain.rs` (re-exported via `linuxbox`) are implemented by adapters in `linuxbox/src/adapters/`. Tests use mock adapters (`minibox_core::adapters::mocks`, behind `test-utils` feature). Integration tests exercise real adapters against live infrastructure.

**Core library split**: Cross-platform types live in `minibox-core`; linuxbox re-exports them. Prefer `use minibox_core::protocol::*` in new code outside linuxbox rather than going through the re-export.

**Adapter Suites**: `MINIBOX_ADAPTER` env var selects between `native` (Linux namespaces, overlay FS, cgroups v2, requires root), `gke` (proot, copy FS, no-op limiter, unprivileged), and `colima` (macOS via limactl/nerdctl). Wired in `miniboxd/src/main.rs`.

**Async/Sync Boundary**: Daemon uses Tokio async for socket I/O (`server.rs`) but spawns blocking tasks for container operations (fork/clone syscalls cannot be async). Container creation in `handler.rs` uses `tokio::task::spawn_blocking`.

**Protocol**: JSON-over-newline on Unix socket (`/run/minibox/miniboxd.sock`). Each message is single JSON object terminated by `\n`. Types defined in `linuxbox/src/protocol.rs` using serde with `#[serde(tag = "type")]` for tagged enums.

**State Management**: In-memory HashMap in `miniboxd/src/state.rs` tracks containers. Not persisted - daemon restart loses all records. Container state machine: Created → Running → Stopped.

**CLI streaming** — `minibox run` uses `ephemeral: true` and streams stdout/stderr back to the terminal in real time via `ContainerOutput`/`ContainerStopped` protocol messages. The CLI exits with the container's exit code. Non-ephemeral runs (daemon-direct) still return immediately with a container ID.

**Image Storage**: Layers stored as extracted directories in `/var/lib/minibox/images/{image}/{digest}/`. Overlay filesystem stacks layers (read-only lower dirs) + container-specific upper/work dirs.

### Container Lifecycle Flow

1. CLI sends `RunContainer` request to daemon
2. Daemon checks image cache, pulls from Docker Hub if missing (anonymous auth)
3. Creates overlay mount: `lowerdir=layer1:layer2:...`, `upperdir=container_rw`, `workdir=container_work`
4. Forks child with `clone(CLONE_NEWPID|CLONE_NEWNS|CLONE_NEWUTS|CLONE_NEWIPC|CLONE_NEWNET)` via nix crate
5. Child process (in `linuxbox/src/container/process.rs`):
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

**linuxbox/src/**:

- `preflight.rs`: Host capability probing (cgroups v2, overlay, systemd, kernel version). Used by `just doctor` and test `require_capability!` macro.
- `domain.rs`: Trait definitions (ports) for hexagonal architecture

**linuxbox/src/container/**:

- `namespace.rs`: Linux namespace setup using nix crate wrappers
- `cgroups.rs`: cgroups v2 manipulation (memory, CPU weight)
- `filesystem.rs`: overlay mount, pivot_root, path validation
- `process.rs`: Container init process, fork/clone, exec

**linuxbox/src/image/**:

- `reference.rs`: `ImageRef` — parse `[REGISTRY/]NAMESPACE/NAME[:TAG]`; routes to correct registry adapter
- `registry.rs`: Docker Hub v2 API client (token auth, manifest/blob fetch)
- `manifest.rs`: OCI manifest parsing
- `layer.rs`: Tar extraction with security validation

**linuxbox/src/adapters/**:

- `registry.rs`: `DockerHubRegistry` adapter
- `colima.rs`: `ColimaRegistry`, `ColimaRuntime`, `ColimaFilesystem`, `ColimaLimiter`
- `vf.rs`: `VfRegistry` (Virtualization.framework)
- `hcs.rs`: `HcsRegistry` (Windows HCS)
- `wsl2.rs`: WSL2 adapter

**daemonbox/src/** (handler/state/server extracted from miniboxd; macOS-safe):

- `server.rs`: Unix socket listener with SO_PEERCRED auth; channel-based streaming dispatch
- `handler.rs`: Request routing; `handle_run_streaming` for ephemeral containers (Linux)
- `state.rs`: In-memory container tracking
- `telemetry/mod.rs`: Metrics and tracing infrastructure adapters
- `telemetry/prometheus_adapter.rs`: `PrometheusMetricsRecorder` — `prometheus-client` crate
- `telemetry/noop.rs`: `NoOpMetricsRecorder` for tests and disabled metrics
- `telemetry/traces.rs`: OTEL trace exporter setup with optional OTLP bridge
- `telemetry/server.rs`: axum `/metrics` HTTP endpoint

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

### Worktrees

- `.worktrees/` — git worktrees for in-progress feature branches (managed by `superpowers:using-git-worktrees` skill); ignore when searching codebase

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
- **No exec command**: Cannot run commands in existing containers
- **No Dockerfile support**: Image-only workflow
- **Adapter wiring incomplete**: `docker_desktop`, `wsl`, `vf`, and `hcs` adapters exist in `linuxbox/src/adapters/` but are not wired into `miniboxd`. `MINIBOX_ADAPTER` accepts `native`, `gke`, or `colima`; the rest are library-only.

## Tracing Contract

All structured events follow these conventions. When adding new `warn!`/`info!`/`debug!`/`error!` calls, match the patterns below.

### Severity discipline

| Level | Usage |
|---|---|
| `error!` | Unrecoverable failures: container init crash, fatal exec errors |
| `warn!` | Security rejections, degraded behaviour, best-effort failures (unmount, cgroup cleanup, signal) |
| `info!` | Lifecycle milestones: container start/stop/remove, image pull phases, overlay mount, pivot_root |
| `debug!` | Implementation detail: syscall arguments, byte counts, internal state transitions |

### Event message convention

Messages use `"<subsystem>: <verb> <noun>"` lowercase prefix — e.g. `"tar: rejected device node"`, `"pivot_root: complete"`, `"container: process started"`.

### Canonical field names

| Field | Type | Used in |
|---|---|---|
| `pid` | u32 | Container process ID |
| `child_pid` | i32 | Cloned child PID (`namespace.rs`) |
| `clone_flags` | i32 | Raw `clone(2)` flags (`namespace.rs`) |
| `container_id` | &str | Container UUID |
| `entry` | &Path | Tar entry path (all `layer.rs` security events) → `ImageError::DeviceNodeRejected.entry` / `SymlinkTraversalRejected.entry` |
| `kind` | &EntryType | Tar entry type (device node rejection) |
| `target` / `original_target` | &Path | Symlink target before rewrite → `ImageError::SymlinkTraversalRejected.target` |
| `rewritten_target` | &Path | Symlink target after absolute→relative rewrite |
| `mode_before` / `mode_after` | u32 | Raw permission bits (octal) before/after strip |
| `new_root` | &Path | `pivot_root` destination path |
| `fds_closed` | usize | Extra FDs closed before exec (`process.rs`) |
| `command` | &str | Container entrypoint command |
| `rootfs` | &Path | Container rootfs path |

**Rule**: use `key = value` structured fields for queryable data — never embed structured values in the message string (e.g. use `pid = pid_value`, not `"PID={pid}"` in the message).

## Debugging

### Testing gotchas

- **`std::env::set_var`/`remove_var` are `unsafe` in Rust 2024** — wrap in `unsafe {}` and serialise with a `static Mutex<()>` guard to prevent parallel test races (see `commands/mod.rs` tests for the pattern).
- **Crate extraction + dev-deps**: When moving types to a new crate, check `tests/` files in the source crate — they may directly use crates that were previously transitive (e.g. `sha2`). Add them explicitly to `[dev-dependencies]` or `cargo nextest` (run by pre-push hook) will catch it at push time.
- **Subprocess tests**: Use `Command::from_std(std::process::Command::new(find_minibox()))` + `MINIBOX_TEST_BIN_DIR` env var — never `Command::cargo_bin()`, which triggers a full recompile. Gate subprocess test files with `#![cfg(all(unix, feature = "subprocess-tests"))]` and a named Cargo feature; run via `just test-cli-subprocess`.

### Proptest gotchas

- **`FileFailurePersistence` warning in integration tests** — proptest can't find `lib.rs` from the `tests/` context; suppress with `#![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]` inside each `proptest!` block.
- **Async methods in proptest** — use `tokio::runtime::Runtime::new().unwrap().block_on(...)` to drive `DaemonState` / handler calls synchronously inside proptest closures. Each closure must create its own `Runtime`.
- **`DaemonState` fixture** — requires `ImageStore::new(tmp.join("images"))` + a `data_dir` path; calls `save_to_disk` on every `add_container`/`remove_container` (256 proptest iterations = ~256 JSON writes to tmp). Use a named `TempDir` so it stays alive for the closure.
- **`CgroupManager::create()` runs `create_dir_all` before bounds checks** — proptest cgroup bound tests require a real cgroup2 mount and root to reach validation logic; a plain `TempDir` is insufficient. Gate with `#[cfg(target_os = "linux")]` and run under `just test-integration`.

### Macro and doctest gotchas

- **`as_any!` macro uses `crate::domain::AsAny`** — `crate` in `macro_rules!` resolves at the call site (linuxbox), not the defining crate (minibox-macros). This is intentional. Clippy warns with `crate_in_macro_def`; suppress with `#[allow(clippy::crate_in_macro_def)]`, do not change to `$crate`.
- **Private fn doctests** — mark with ```` ```ignore ```` (not `no_run`); private functions aren't accessible in doctest context and will fail to compile.

### Container init gotchas (relevant when modifying `filesystem.rs` or `process.rs`)

- **Pipe fds across `clone()`** — both parent and child get copies of any `OwnedFd` after clone. Use `std::mem::forget` on fds before the clone call, then manage raw fds manually. Child: `dup2` write end into stdout/stderr slots, then `close(write_fd_raw)` and `close(read_fd_raw)`. Parent: `drop(write_fd)` after clone returns, keep read end for output streaming.
- **`pivot_root` requires `MS_PRIVATE` first** — after `CLONE_NEWNS` the child inherits shared mount propagation from the parent; `pivot_root` fails EINVAL unless you call `mount("", "/", MS_REC|MS_PRIVATE)` inside the child before the bind-mount.
- **`close_extra_fds` uses `close_range(2)` fast path** — tries the syscall first (kernel 5.9+, QEMU-inspired). Falls back to `/proc/self/fd` iteration which must collect FD numbers into a `Vec` before closing (iterating and closing in the loop would close `ReadDir`'s own FD).
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
2. **Container primitives**: Add to `linuxbox/src/container/`, use nix crate for syscalls
3. **Image operations**: Extend `linuxbox/src/image/` modules
4. **State persistence**: Consider replacing HashMap in `state.rs` with serialized storage
5. **Networking**: Implement in new `network.rs` module, add bridge/veth setup in container init

### Platform-specific dependencies

Gate Linux-only deps with `[target.'cfg(target_os = "linux")'.dependencies]`, not unconditional. For Unix-wide code (e.g. `preflight.rs`), use `nix::unistd::geteuid().is_root()` instead of `libc::geteuid()`.

### Crate renames

`recipe.json` is a cargo-chef artifact that embeds crate names — update manually after any crate rename (or regenerate with `cargo chef prepare --recipe-path recipe.json`).

## Environment Variables

Override runtime paths (useful for testing and non-standard deployments):

- `MINIBOX_DATA_DIR` — image/container storage. UID-aware default: `~/.mbx/cache/` for non-root, `/var/lib/minibox` for root. Explicit env var overrides both.
- `MINIBOX_RUN_DIR` — socket/runtime dir (default: `/run/minibox`)
- `MINIBOX_SOCKET_PATH` — Unix socket path
- `MINIBOX_CGROUP_ROOT` — cgroup root for containers (default: `/sys/fs/cgroup/minibox.slice/miniboxd.service`)
- `MINIBOX_ADAPTER` — adapter suite: `native` (default), `gke`, or `colima`
- `MINIBOX_METRICS_ADDR` — Prometheus `/metrics` bind address (default: `127.0.0.1:9090`)
- `MINIBOX_OTLP_ENDPOINT` — OTLP collector endpoint for trace export (unset = disabled)

## GitHub Actions CI

`.github/workflows/ci.yml` — macOS job (`macos-latest`): `cargo fmt --all --check` + clippy (all crates) + `cargo xtask test-unit`. No Linux job yet (self-hosted runner planned).

## Gitea CI

CI runs on self-hosted Gitea Actions (jobrien-vm). Pipeline: `cargo deny check` + `cargo audit` only — no compilation (VPS has 2 CPUs, no swap; Rust builds saturate it).

- `deny.toml` — `licenses.private.ignore = false` by default; unpublished workspace crates (e.g. `xtask`) need `license = "MIT"` in Cargo.toml or they fail as unlicensed
- Gitea context vars are `gitea.repository`, `gitea.run_id`, `gitea.sha` — not `github.*`
- `GITEA_` prefix is reserved for secrets; use other names (e.g. `CI_AGENT_TOKEN`)
- Check CI status: `mise run all:ci`

## Skills Available

Global minibox skills available across all projects:

- `mbx:minibox-ci`: CI operations — self-hosted runner management, GHA diagnostics, xtask gates

Invoke with `/` prefix, e.g., `/mbx:minibox-ci`.
