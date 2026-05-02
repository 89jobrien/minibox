# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Minibox is a Docker-like container runtime written in Rust featuring daemon/client architecture, OCI image pulling from Docker Hub, Linux namespace isolation, cgroups v2 resource limits, and overlay filesystem support.

## Rust Edition

This workspace uses **Rust 2024 edition**. Watch for: match ergonomics changes, `unsafe` required for `set_var`/`remove_var`, `FromRawFd` scope differences. Always run `cargo clippy` and `cargo test` after generating code.

**Critical:** See [`rules/rust-patterns.md`](rules/rust-patterns.md) for Minibox-specific patterns (error handling, path validation, async/sync boundaries, tracing discipline). This is a must-read before writing container code.

## Build and Development Commands

### Python Scripts

All scripts in `scripts/` use `#!/usr/bin/env -S uv run` + PEP 723 inline deps. Run with `uv run scripts/foo.py` or directly if executable. Never use `python`/`python3` directly.

### AI Agent Scripts (Claude Agent SDK)

All `scripts/*.py` SDK scripts (`council.py`, `ai-review.py`, `meta-agent.py`, etc.) require an interactive terminal — they fail with "Command failed with exit code 1" when run via `run_in_background`. Always run them foreground/interactively.

- `just gen-tests <TraitName>` — scaffold unit tests for a new domain trait adapter
- `just diagnose [--container <id>]` — diagnose container failure from logs + cgroup state
- `just sync-check` — fetch + rebase onto origin/main, auto-resolve obvious conflicts (wired into `just push`)

### mise.toml vs Justfile Convention

- **Justfile** — build, lint, test gates, CI gates, cleanup
- **mise.toml** — interactive demos, ops tasks (`ssh-vps`, `fix-socket`, `smoke`), git ops (`commit`, `push`)

**mise.toml script gotcha:** Bash scripts with ANSI escape codes (e.g. `\033[36m`) **must** use `run = '''...'''` (TOML literal string) not `run = """..."""` — TOML interprets `\0` as an invalid escape in double-quoted strings.

### Justfile recipes

Recipes using bash-specific features (arrays, `local`, `declare`, functions) require a `#!/usr/bin/env bash` shebang as the first line — Just defaults to `sh` otherwise.

### Building

```bash
# Build all crates in workspace
cargo build --release

# Build specific crate
cargo build -p minibox
cargo build -p miniboxd
cargo build -p mbx

# Check all crates without building
cargo check --workspace

# Build macOS VM image (Alpine kernel + agent, macOS only — required before MINIBOX_ADAPTER=vz)
cargo xtask build-vm-image          # cached, skips if already built
cargo xtask build-vm-image --force  # re-download + recompile
just build-vm-image                 # shorthand
just build-vm-image force           # force rebuild
```

Binaries output to `target/release/miniboxd` and `target/release/mbx`.

### Running the Daemon and CLI

```bash
# Start daemon (requires root)
sudo ./target/release/miniboxd

# CLI commands (daemon must be running)
sudo ./target/release/mbx pull alpine
sudo ./target/release/mbx run alpine -- /bin/echo "Hello"
sudo ./target/release/mbx ps
sudo ./target/release/mbx stop <container_id>
sudo ./target/release/mbx rm <container_id>

# Image refresh (daemon must be running)
sudo ./target/release/mbx update alpine:latest    # re-pull specific image
sudo ./target/release/mbx update --all            # re-pull all cached images
sudo ./target/release/mbx update --containers     # re-pull images used by running containers

# Self-update (no daemon needed)
./target/release/mbx upgrade              # upgrade to latest GitHub release
./target/release/mbx upgrade --dry-run    # preview without replacing binary
./target/release/mbx upgrade --version v0.21.0  # pin to specific version
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
cargo test -p minibox

# Run specific test module
cargo test -p minibox-core protocol::tests

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

# Run criterion benches (minibox has inline [[bench]] targets)
cargo bench -p minibox          # trait_overhead + protocol_codec (local HTML reports only)
```

**Test Status:**

- Unit + conformance: ~770+ tests via nextest (run `cargo nextest list --workspace` for current count; 4 skipped on macOS)
- Property-based: 8 daemon proptest properties + 25 minibox property tests (`cargo xtask test-property`)
- Cgroup integration: 16 tests (Linux+root, `just test-integration`)
- E2E daemon+CLI: 14 tests (Linux+root, `just test-e2e`)
- Existing integration: 8 tests (Linux+root)
- Specs/plans: `docs/superpowers/specs/`, `docs/superpowers/plans/` — check `git log` to see if a plan was already executed before treating it as pending

### Before Committing

**One-command pre-commit gate** (macOS):

```bash
cargo xtask pre-commit
```

This runs: `cargo fmt --all --check` + clippy (all crates) + `cargo build --release` (macOS-safe).

**On Linux**, use `cargo xtask prepush` for full coverage including `cargo xtask test-unit` + llvm-cov report.

**macOS quality gates** (what `cargo xtask pre-commit` runs):

```bash
cargo fmt --all --check
cargo clippy -p minibox -p minibox-macros -p mbx -p macbox -p miniboxd -- -D warnings
cargo xtask test-unit
```

### Handler Testing Patterns

**Channel-based responses**: `handle_run` sends responses via channel, not direct return. Use `handle_run_once()` test helper (defined in handler_tests.rs) to recover single response.

**Mock adapter builders**: Configure mocks at creation time via builder methods (`.with_empty_layers()`, `.with_pull_failure()`, `.with_cached_image()`) — don't try to mutate after construction.

**Mock registry type casting**: When storing `Arc<MockRegistry>` in `Arc<dyn ImageRegistry>`, use explicit cast: `Arc::clone(&mock) as Arc<dyn minibox_core::domain::ImageRegistry>`.

**Test file organization**: Handler tests in `crates/minibox/tests/handler_tests.rs`. Test helpers (`create_test_deps_with_dir`, `create_test_state_with_dir`) are in that file.

**CLI command test helpers**: `mbx/src/commands/mod.rs` has `test_helpers` module with `setup()`
(single response) and `setup_multi()` (multi-response) — bind a mock Unix socket, accept one
connection, write canned `DaemonResponse` lines. Use for any new CLI command tests.

**Hexagonal ports in CLI**: `upgrade.rs` uses `ReleaseProvider` and `AssetDownloader` traits
with mock doubles for testing `run_upgrade()` without network access. Follow this pattern for
any new CLI command that talks to external services.

### Coverage Focus Areas

`handler.rs` (minibox::daemon) coverage improved with Wave 2 handler tests but remains the
biggest gap. Error path tests (image pull failure, empty image, registry unreachable) have good
ROI. Use `cargo xtask prepush` to generate llvm-cov coverage report.

## Architecture Overview

### Workspace Structure

Platform crates follow the `{platform}box` naming convention: `minibox` (Linux namespaces/cgroups),
`macbox` (macOS Colima/VZ/krun), `winbox` (Windows stub). All are platform-conditional deps in
`miniboxd`.

9 crates in cargo workspace:

1. **minibox-core** (library): Cross-platform shared types — protocol, domain traits, error types,
   image management (`ImageStore`, `RegistryClient`), preflight; re-exported by minibox for macro
   compatibility
2. **minibox** (library, dir: `crates/minibox`): Linux-specific container primitives, adapters
   (namespaces, cgroups, overlay, process), daemon handler/server/state, OCI image ops, and Unix
   socket client. Re-exports `minibox-core` — **do not remove re-exports** — `as_any!`/`adapt!`
   macros expand to `crate::domain::AsAny` at call sites inside minibox. Absorbed former crates:
   `daemonbox` (now `minibox::daemon`), `minibox-oci` (now `minibox::image`), `minibox-client`
   (now `minibox::client`), `minibox-testers` (now behind `test-utils` feature).
3. **minibox-macros** (proc-macro): Derive macros used by minibox
4. **miniboxd** (binary): Async daemon entry point; dispatches to `macbox::start()` on macOS,
   `winbox::start()` on Windows
5. **macbox** (library): macOS daemon implementation (Colima adapter suite + VZ + krun backends)
6. **winbox** (library): Windows daemon implementation (stub)
7. **mbx** (binary): CLI client sending commands to daemon (formerly `minibox-cli`)
8. **minibox-crux-plugin** (binary): Crux plugin host — exposes pull/run/ps/stop/rm over
   JSON-RPC stdio for agent pipelines; connects minibox to the crux DSL runtime
9. **xtask** (dev-tool): Pre-commit gate, test suites, conformance, build-vm-image; not shipped

### Critical Design Patterns

**Hexagonal Architecture**: Domain traits (`ResourceLimiter`, `FilesystemProvider`, `ContainerRuntime`, `ImageRegistry`) in `minibox-core/src/domain.rs` (re-exported via `minibox`) are implemented by adapters in `crates/minibox/src/adapters/`. Tests use mock adapters (`minibox::testing::mocks`, behind `test-utils` feature). Integration tests exercise real adapters against live infrastructure.

**Core library split**: Cross-platform types live in `minibox-core`; minibox re-exports them. Prefer `use minibox_core::protocol::*` in new code outside the minibox crate rather than going through the re-export.

**Adapter Suites**: `MINIBOX_ADAPTER` env var selects between `native` (Linux namespaces, overlay
FS, cgroups v2, requires root), `gke` (proot, copy FS, no-op limiter, unprivileged), and
`colima` (macOS via limactl/nerdctl — experimental). The `vz` adapter is wired in `macbox` but
blocked by a VZErrorInternal Apple bug on macOS 26 ARM64 (GH #61); `krun` is next (in progress).
Wired in `miniboxd/src/main.rs`.

**Async/Sync Boundary**: Daemon uses Tokio async for socket I/O (`daemon/server.rs`) but spawns blocking tasks for container operations (fork/clone syscalls cannot be async). Container creation in `daemon/handler.rs` uses `tokio::task::spawn_blocking`.

**Protocol**: JSON-over-newline on Unix socket (`/run/minibox/miniboxd.sock`). Each message is single JSON object terminated by `\n`. Types defined in `minibox-core/src/protocol.rs` (canonical source) using serde with `#[serde(tag = "type")]` for tagged enums. `minibox` re-exports via `pub use minibox_core::protocol`.

**State Management**: `DaemonState` in `minibox::daemon::state` tracks containers in a HashMap and persists to disk via `save_to_disk` on every add/remove (atomic rename, no fsync). State survives daemon restart; running processes are not reattached. See `docs/STATE_MODEL.md` for full persistence contract. Container state machine: Created → Running → Paused → Stopped.

**CLI streaming** — `mbx run` uses `ephemeral: true` and streams stdout/stderr back to the terminal in real time via `ContainerOutput`/`ContainerStopped` protocol messages. The CLI exits with the container's exit code. Non-ephemeral runs (daemon-direct) still return immediately with a container ID.

**Image Storage**: Layers stored as extracted directories in `/var/lib/minibox/images/{image}/{digest}/`. Overlay filesystem stacks layers (read-only lower dirs) + container-specific upper/work dirs.

### Container Lifecycle Flow

1. CLI sends `RunContainer` request to daemon
2. Daemon checks image cache, pulls from Docker Hub if missing (anonymous auth)
3. Creates overlay mount: `lowerdir=layer1:layer2:...`, `upperdir=container_rw`, `workdir=container_work`
4. Forks child with `clone(CLONE_NEWPID|CLONE_NEWNS|CLONE_NEWUTS|CLONE_NEWIPC|CLONE_NEWNET)` via nix crate
5. Child process (in `crates/minibox/src/container/process.rs`):
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

**crates/minibox/src/**:

- `preflight.rs`: Host capability probing (cgroups v2, overlay, systemd, kernel version). Used by `just doctor` and test `require_capability!` macro.
- `domain.rs`: Trait definitions (ports) for hexagonal architecture

**crates/minibox/src/container/**:

- `namespace.rs`: Linux namespace setup using nix crate wrappers
- `cgroups.rs`: cgroups v2 manipulation (memory, CPU weight)
- `filesystem.rs`: overlay mount, pivot_root, path validation
- `process.rs`: Container init process, fork/clone, exec

**crates/minibox/src/image/** (absorbed from former `minibox-oci`):

- `reference.rs`: `ImageRef` — parse `[REGISTRY/]NAMESPACE/NAME[:TAG]`; routes to correct registry adapter
- `registry.rs`: Docker Hub v2 API client (token auth, manifest/blob fetch)
- `manifest.rs`: OCI manifest parsing
- `layer.rs`: Tar extraction with security validation

**crates/minibox/src/adapters/**:

- `registry.rs`: `DockerHubRegistry` adapter
- `colima.rs`: `ColimaRegistry`, `ColimaRuntime`, `ColimaFilesystem`, `ColimaLimiter`
- `vf.rs`: `VfRegistry` (Virtualization.framework)
- `hcs.rs`: `HcsRegistry` (Windows HCS)
- `wsl2.rs`: WSL2 adapter

**crates/minibox/src/daemon/** (absorbed from former `daemonbox`; macOS-safe):

- `server.rs`: Unix socket listener with SO_PEERCRED auth; channel-based streaming dispatch
- `handler.rs`: Request routing; `handle_run_streaming` for ephemeral containers (Linux)
- `state.rs`: In-memory container tracking

**crates/minibox/src/client/** (absorbed from former `minibox-client`):

- `DaemonClient`, `DaemonResponseStream` — Unix socket client for mbx CLI

### DaemonResponse Protocol Notes

Current variants: `ContainerCreated`, `Success`, `Error`, `ContainerList`, `ContainerStopped`, `ContainerOutput`. **Terminal vs non-terminal**: Only `ContainerOutput` is non-terminal (can be sent multiple times during streaming). All other variants end streaming/request. If adding new variant, update `is_terminal_response()` in `daemon/server.rs` to include it in the match.

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

`daemon/server.rs` uses `SO_PEERCRED` to authenticate clients:

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

See `docs/FEATURE_MATRIX.md` for the full per-platform breakdown. Key constraints as of
2026-04-19:

- **Networking is opt-in and experimental**: Bridge networking (`MINIBOX_NETWORK_MODE=bridge`)
  is wired but has limited test coverage. Containers get an isolated network namespace by
  default with no external connectivity. Port forwarding and in-container DNS are not
  implemented.
- **No user namespace remapping**: Runs as root inside containers (no rootless support).
- **Container records persist across restarts, but running processes do not reattach**: State
  is saved to disk and loaded at startup. Containers that were running when the daemon stopped
  appear as records but are not reattached — their PIDs are gone.
- **Exec is Linux native only**: `minibox exec` / `handle_exec` uses `setns` and is wired only
  for the `native` adapter. GKE, Colima, and macOS adapters return an error.
- **No Dockerfile parser**: `MiniboxImageBuilder` exists but there is no Dockerfile DSL. Build
  support is experimental and native-only.
- **Push/commit are experimental and native-only**: `OciPushAdapter` and `overlay_commit_adapter`
  are wired in the native suite only and have limited test coverage.
- **Adapter wiring incomplete**: `docker_desktop`, `wsl2`, `vf`, and `hcs` adapters exist as
  library code but are not wired into `miniboxd`. Passing unrecognized values to
  `MINIBOX_ADAPTER` causes the daemon to exit at startup.
- **Windows is a stub**: `winbox::start()` returns an error unconditionally. Phase 2 work
  (Named Pipe server, HCS/WSL2 adapter wiring) has not started.
- **Protocol types**: `DaemonRequest`/`DaemonResponse` are defined in
  `minibox-core/src/protocol.rs` (single source of truth). `minibox` re-exports via
  `pub use minibox_core::protocol`.

## Tracing Contract

All structured events follow these conventions. When adding new `warn!`/`info!`/`debug!`/`error!` calls, match the patterns below.

### Severity discipline

| Level    | Usage                                                                                           |
| -------- | ----------------------------------------------------------------------------------------------- |
| `error!` | Unrecoverable failures: container init crash, fatal exec errors                                 |
| `warn!`  | Security rejections, degraded behaviour, best-effort failures (unmount, cgroup cleanup, signal) |
| `info!`  | Lifecycle milestones: container start/stop/remove, image pull phases, overlay mount, pivot_root |
| `debug!` | Implementation detail: syscall arguments, byte counts, internal state transitions               |

### Event message convention

Messages use `"<subsystem>: <verb> <noun>"` lowercase prefix — e.g. `"tar: rejected device node"`, `"pivot_root: complete"`, `"container: process started"`.

### Canonical field names

| Field                        | Type       | Used in                                                                                                                     |
| ---------------------------- | ---------- | --------------------------------------------------------------------------------------------------------------------------- |
| `pid`                        | u32        | Container process ID                                                                                                        |
| `child_pid`                  | i32        | Cloned child PID (`namespace.rs`)                                                                                           |
| `clone_flags`                | i32        | Raw `clone(2)` flags (`namespace.rs`)                                                                                       |
| `container_id`               | &str       | Container UUID                                                                                                              |
| `entry`                      | &Path      | Tar entry path (all `layer.rs` security events) → `ImageError::DeviceNodeRejected.entry` / `SymlinkTraversalRejected.entry` |
| `kind`                       | &EntryType | Tar entry type (device node rejection)                                                                                      |
| `target` / `original_target` | &Path      | Symlink target before rewrite → `ImageError::SymlinkTraversalRejected.target`                                               |
| `rewritten_target`           | &Path      | Symlink target after absolute→relative rewrite                                                                              |
| `mode_before` / `mode_after` | u32        | Raw permission bits (octal) before/after strip                                                                              |
| `new_root`                   | &Path      | `pivot_root` destination path                                                                                               |
| `fds_closed`                 | usize      | Extra FDs closed before exec (`process.rs`)                                                                                 |
| `command`                    | &str       | Container entrypoint command                                                                                                |
| `rootfs`                     | &Path      | Container rootfs path                                                                                                       |

**Rule**: use `key = value` structured fields for queryable data — never embed structured values in the message string (e.g. use `pid = pid_value`, not `"PID={pid}"` in the message).

## Crate Renaming Gotcha

- When renaming a crate directory, `Cargo.toml` workspace `members` paths must match the actual
  directory name. If they diverge, `cargo check` fails with "failed to read Cargo.toml / No such
  file or directory". Fix: `mv crates/old crates/new`.
- `cargo fix --lib -p <crate>` only fixes lib targets. Add `--tests` to also fix integration test
  files, or use `--allow-dirty` to fix everything in-tree.

## FilesystemProvider Supertrait

`FilesystemProvider` is a marker supertrait combining `RootfsSetup + ChildInit`. `setup_rootfs`
is defined on `RootfsSetup` — import `RootfsSetup` explicitly wherever `setup_rootfs` is called
(benches, tests). Importing `FilesystemProvider` alone is not sufficient.

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

- **`as_any!` macro uses `crate::domain::AsAny`** — `crate` in `macro_rules!` resolves at the call site (minibox), not the defining crate (minibox-macros). This is intentional. Clippy warns with `crate_in_macro_def`; suppress with `#[allow(clippy::crate_in_macro_def)]`, do not change to `$crate`.
- **`adapt!` requires `new() -> Self`** — `adapt!` calls `default_new!` which implements `Default` via `Self::new()`. Any adapter whose `new()` returns `Result<Self>` must use `as_any!` only — do NOT use `adapt!` for it.
- **Private fn doctests** — mark with ` ```ignore ` (not `no_run`); private functions aren't accessible in doctest context and will fail to compile.

### Protocol gotchas (relevant when modifying `protocol.rs` or `handler.rs`)

- **Single `DaemonRequest` definition** — canonical source is `crates/minibox-core/src/protocol.rs`. `minibox` re-exports it. Wire format snapshot tests in minibox-core pin serialization. When adding a field, update `minibox-core/src/protocol.rs` and add a snapshot test.
- **`HandlerDependencies` construction sites** — Adding fields to `HandlerDependencies` in `daemon/handler.rs` requires updating all three adapter suites in `miniboxd/src/main.rs` (native, gke, colima). These are Linux-only (`#[cfg(target_os = "linux")]`) and won't fail on macOS `cargo check`.
- **`handle_run` param chain** — Adding a parameter requires updating in order: `daemon/server.rs` dispatch pattern match → `handle_run` → `handle_run_streaming` → `run_inner_capture`; and separately `run_inner`. All five sites must change together.
- **`#[serde(default)]` for backward-compatible protocol additions** — New fields on `DaemonRequest` variants must use `#[serde(default)]` so existing JSON clients that omit the field continue to work.
- **Silent channel-send discards are a bug** — never use `let _ = tx.send(...).await` in handler
  code. Use `if tx.send(...).await.is_err() { warn!("handle_run: client disconnected before
<context> could be sent"); }` so dropped connections are observable in logs.
- **Stale rust-analyzer diagnostics** — During multi-file edits, rust-analyzer lags behind. Use `cargo check -p <crate>` as the source of truth, not the IDE error count.
- **Linux-only clippy lints** — Files under `#![cfg(target_os = "linux")]` (e.g. `bridge.rs`) are invisible to macOS clippy. Lints like `clone_on_copy` and `collapsible_if` only surface on Linux CI runners — always check CI after touching the adapter layer.
- **Linux-only cfg-gated imports** — `#[cfg(target_os = "linux")]` code is not checked by
  `cargo check` on macOS. When writing or reviewing Linux-only modules (cgroup_tests.rs,
  bridge.rs, etc.), manually verify all `use` statements are present — the compiler won't
  catch missing imports until Linux CI runs.

### macbox/vz gotchas (relevant when modifying `crates/macbox/src/vz/` or merging the vz branch)

- **`MINIBOX_ADAPTER=vz` requires compile-time feature** — must build with `--features vz` for `macbox`/`miniboxd`; the env var alone at runtime is not enough.
- **Duplicate `pub mod vz;` after merges** — `crates/macbox/src/lib.rs` should have exactly one `#[cfg(feature = "vz")] pub mod vz;`. Merges that touch this file can silently introduce a duplicate unconditional declaration; `cargo check` catches it immediately.
- **Crate name is `minibox`** — the lib crate was briefly named `linuxbox` (2026-04-21 to
  2026-04-26); any `linuxbox::` reference in code, notes, or plans is stale. Use `minibox::`
  (e.g. `minibox::adapters::NoopNetwork`). The CLI binary was renamed: `minibox-cli` → `mbx`.

### Container init gotchas (relevant when modifying `filesystem.rs` or `process.rs`)

- **Pipe fds across `clone()`** — both parent and child get copies of any `OwnedFd` after clone. Use `std::mem::forget` on fds before the clone call, then manage raw fds manually. Child: `dup2` write end into stdout/stderr slots, then `close(write_fd_raw)` and `close(read_fd_raw)`. Parent: `drop(write_fd)` after clone returns, keep read end for output streaming.
- **`pivot_root` requires `MS_PRIVATE` first** — after `CLONE_NEWNS` the child inherits shared mount propagation from the parent; `pivot_root` fails EINVAL unless you call `mount("", "/", MS_REC|MS_PRIVATE)` inside the child before the bind-mount.
- **`close_extra_fds` uses `close_range(2)` fast path** — tries the syscall first (kernel 5.9+, QEMU-inspired). Falls back to `/proc/self/fd` iteration which must collect FD numbers into a `Vec` before closing (iterating and closing in the loop would close `ReadDir`'s own FD).
- **Absolute symlink rewrite in `layer.rs`** — `strip_prefix("/")` gives a path relative to the container root, not the symlink's directory. Use `relative_path(entry_dir, abs_target)` (defined in `layer.rs`) to get the correct relative target; otherwise busybox applet symlinks resolve to non-existent paths (e.g. `/bin/bin/busybox`).
- **Tar root entries** — `"."` and `"./"` entries in OCI layers must be skipped before path validation; `Path::join("./")` normalizes the CurDir component away, causing a false path-escape error.
- **`child_init` uses `execve` not `execvp`** — `execvp` inherits the daemon's host environment into the container. `child_init` calls `execve` with an explicit `envp` built from `config.env`. Do not revert to `execvp`.

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

1. **Protocol changes**: Update `minibox-core/src/protocol.rs` types first, then implement in `handler.rs`
2. **Container primitives**: Add to `crates/minibox/src/container/`, use nix crate for syscalls
3. **Image operations**: Extend `crates/minibox/src/image/` modules
4. **State persistence**: Consider replacing HashMap in `state.rs` with serialized storage
5. **Networking**: Implement in new `network.rs` module, add bridge/veth setup in container init

### Platform-specific dependencies

Gate Linux-only deps with `[target.'cfg(target_os = "linux")'.dependencies]`, not unconditional. For Unix-wide code (e.g. `preflight.rs`), use `nix::unistd::geteuid().is_root()` instead of `libc::geteuid()`.

### Crate renames

`recipe.json` is a cargo-chef artifact that embeds crate names — update manually after any crate rename (or regenerate with `cargo chef prepare --recipe-path recipe.json`).

## Conformance Suite

Branch: `feat/conformance-suite` — Phases 1–3 complete as of 2026-04-11.

- Entry point: `cargo xtask test-conformance` — runs suite, prints artifact paths
- Reports written to `artifacts/conformance/` (gitignored; CI uploads as artifacts)
- `CONFORMANCE_ARTIFACT_DIR` — override report output directory
- `CONFORMANCE_PUSH_REGISTRY=localhost:5000` — enable Tier 2 push tests (requires live registry)
- `CONFORMANCE_COLIMA=1` — enable Colima backend tests
- See `docs/conformance.md` for capability matrix and full usage

## Environment Variables

Override runtime paths (useful for testing and non-standard deployments):

- `MINIBOX_DATA_DIR` — image/container storage. UID-aware default: `~/.minibox/cache/` for non-root, `/var/lib/minibox` for root. Explicit env var overrides both.
- `MINIBOX_RUN_DIR` — socket/runtime dir (default: `/run/minibox`)
- `MINIBOX_SOCKET_PATH` — Unix socket path
- `MINIBOX_CGROUP_ROOT` — cgroup root for containers (default: `/sys/fs/cgroup/minibox.slice/miniboxd.service`)
- `MINIBOX_ADAPTER` — adapter suite: `native` (default), `gke`, `colima`, or `vz`

### Committing `.ctx/HANDOFF.*.*.yaml`

Use `git add -f .ctx/HANDOFF.minibox.workspace.yaml` — the directory is gitignored and negation
exceptions don't auto-apply without `-f`. The global obfsck hook must have `:!.ctx/HANDOFF.*.*.yaml`
(bare path) alongside `:!**/.ctx/HANDOFF.*.*.yaml` — the `**/` form alone does not match top-level
dirs. If obfsck blocks the commit, add the bare exclusion to `~/.config/git/hooks/pre-commit`.

## Git Workflow (3-tier stability pipeline)

```
main (develop) ──auto──► next (validated) ──manual──► stable (release) ──► v* tag
```

- **`main`**: Active R&D. Must compile. Direct push or PR merge.
- **`next`**: Auto-promoted from `main` on green CI. Full test + audit gates.
- **`stable`**: Maestro-consumable. Tagged releases (`v*`) cut here.
- **`feature/*`, `hotfix/*`, `chore/*`**: Short-lived, target `main`. Auto-deleted on merge.

Every commit on every branch must compile. `origin` is GitHub (`git@github.com:89jobrien/minibox.git`).

**Guardrail**: Never promote `next → stable` without confirming `next` CI is green. After
merging to `next`, run `gh run list --limit 3` and verify all `next` jobs pass before
touching `stable`. The `stable` branch is a manual gate — do not batch the promotion.

**Shared target dir**: `CARGO_TARGET_DIR=~/.minibox/cache/target/` (set in `.envrc`). Worktrees share the same cache.

## GitHub Actions CI

Workflows in `.github/workflows/`:

- **`ci.yml`** — Quality gates, branch-conditional:
  - All branches: `cargo check --workspace` + `cargo fmt --all --check` + clippy
  - `next` + `stable`: above + `cargo xtask test-unit` + audit/deny/machete
  - `stable` only: above + `cargo geiger`
- **`phased-deployment.yml`** — Auto-promote `main→next` on green CI; manual promote `next→stable` via `workflow_dispatch`; hotfix backmerge `stable→next→main`
- **`release.yml`** — Triggered by `v*` tag on `stable`; cross-compile musl binaries; GitHub Release
- **`nightly.yml`** — Daily `cargo geiger` unsafe audit (informational)

`deny.toml` — `licenses.private.ignore = false` by default; unpublished workspace crates (e.g. `xtask`) need `license = "MIT"` in Cargo.toml or they fail as unlicensed

- Check CI status: `mise run all:ci`

## macOS Path Gotchas

- `dirs::config_dir()` on macOS resolves to `~/Library/Application Support/`, not `~/.config/`.
  Any crate using `dirs` for config paths must account for this in docs and setup instructions.

## Serde/Default Gotcha

- `#[derive(Default)]` ignores `#[serde(default = "fn")]` field annotations — derived `Default`
  uses `u16::default()` (0), not the serde default fn. When a struct has field-level serde
  defaults AND is used as `#[serde(default)]` on its parent, implement `Default` manually.

## Plan Status Verification

Plans in `docs/superpowers/plans/` with `status: done` may be incorrect --
agentic sessions sometimes mark plans done when protocol fields land, even if
primary deliverable crates were never created. Always verify with `cargo
metadata` and file existence before trusting plan status.

## Documentation Drift

`docs/CRATE_INVENTORY.md` and `docs/CRATE_TIERS.md` drift as the codebase
grows. Regenerate inventory counts with `cargo metadata --no-deps` + `wc -l`
on `crates/*/src/`. Test counts via `cargo nextest list --workspace | tail -1`.

Spec/plan files in `docs/superpowers/` use date prefixes (e.g.
`2026-04-26-winbox-wsl2-proxy-design.md`). When searching by name, always
glob with `*name*` pattern, not exact match.

## Adapter Default Logic

- Default adapter: `smolvm`; auto-falls back to `krun` when `smolvm` binary is absent from
  PATH. Implemented in `miniboxd/src/adapter_registry.rs::adapter_from_env()`.
- Explicit `MINIBOX_ADAPTER=<value>` bypasses the probe entirely — no fallback applied.
- `DEFAULT_ADAPTER_SUITE = "smolvm"`, `FALLBACK_ADAPTER_SUITE = "krun"` are the constants.

## Crux Integration

- `cruxx-plugin` and `cruxx-types` exist in `~/dev/crux/crates/`. The plugin protocol
  (JSON-RPC over stdio: `Declare`/`Invoke`/`Shutdown`) is fully implemented in
  `cruxx-plugin::host::PluginHost`.
- `minibox-agent` crate does NOT exist — archived specs and some doob todos reference it,
  but it was never created. The `minibox-crux-plugin` binary (`crates/minibox-crux-plugin/`)
  is the live replacement — it exposes minibox ops over JSON-RPC stdio for crux agent pipelines.

## Skills Available

Global minibox skills available across all projects:

- `minibox:minibox-ci`: CI operations — self-hosted runner management, GHA diagnostics, xtask gates

Invoke with `/` prefix, e.g., `/minibox:minibox-ci`.
