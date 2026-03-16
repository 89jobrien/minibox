# E2E Test Infrastructure Design

## Overview

Automated test infrastructure for minibox covering three layers: preflight capability probing, handler-level integration tests (cgroup v2 focus), and daemon+CLI end-to-end tests. All tests exercise domain traits (hexagonal architecture), not concrete implementations directly. Runnable locally and on a self-hosted runner via justfile.

## Goals

1. Address the council's P0: automated e2e tests for cgroup v2 limits and delegation
2. Build a layered test pyramid: preflight → integration → e2e
3. Exercise the `ResourceLimiter`, `FilesystemProvider`, `ContainerRuntime`, and `ImageRegistry` traits against real infrastructure
4. Provide a justfile task runner for local dev and future CI
5. Handle cleanup of runtime state (cgroups, mounts, sockets, processes) and build artifacts (`target/`)

## Non-Goals

- GitHub Actions self-hosted runner setup (follow-up work)
- `minibox doctor` CLI subcommand (preflight module enables this later)
- Networking tests (no bridge/veth support yet)
- Rootless / user namespace tests (not implemented)

## Architecture

### Test Layers

```
┌─────────────────────────────────────────────┐
│  E2E Tests (e2e_tests.rs)                   │
│  Start real miniboxd + minibox CLI binaries  │
│  Test full stack through Unix socket         │
└─────────────────────────────────────────────┘
┌─────────────────────────────────────────────┐
│  Integration Tests (cgroup_tests.rs)         │
│  Test domain traits against real cgroupfs,   │
│  overlay FS, Docker Hub                      │
└─────────────────────────────────────────────┘
┌─────────────────────────────────────────────┐
│  Preflight (preflight.rs)                    │
│  Probe host capabilities, gate tests         │
└─────────────────────────────────────────────┘
┌─────────────────────────────────────────────┐
│  Unit Tests (existing, unchanged)            │
│  Mock-based, platform-agnostic               │
└─────────────────────────────────────────────┘
```

### Hexagonal Alignment

Integration tests are **adapter verification tests** — they exercise domain traits but verify outcomes against real infrastructure. This is distinct from the conformance tests (which verify trait contracts with mocks):

- Call `ResourceLimiter` trait methods → assert by reading cgroupfs
- Call `FilesystemProvider` trait methods → assert by checking mount table
- Call `ContainerRuntime` trait methods → assert by checking process state
- Call `ImageRegistry` trait methods → assert by checking extracted layers

The adapter under test is always the real adapter for the host. Cgroup tests require cgroups v2 and use `require_capability!` to skip when unavailable — there is no fallback to `NoopLimiter` in these tests. The `NoopLimiter` is covered by the existing conformance tests.

```rust
// Cgroup tests always use the real adapter, gated by capability check
fn real_limiter() -> Arc<dyn ResourceLimiter> {
    Arc::new(CgroupV2Limiter::new())
}
```

Test assertions read real infrastructure state (cgroupfs, procfs, mount table) to verify the adapter fulfilled the trait contract. This is intentional: the test calls through the trait boundary but verifies against the real system — an "anti-corruption layer test" pattern.

E2E tests are the outermost port — they test the assembled system through CLI binaries.

Existing `conformance_tests.rs` (trait-level assertions with mocks) serves as the template. The new integration tests are the "real adapter" counterpart of those same contracts.

## Components

### 1. Preflight Module

**File:** `crates/minibox-lib/src/preflight.rs`

Probes the host for capabilities needed by integration and e2e tests. Pure reads, no mutations. Infallible — missing data yields false/empty.

```rust
pub struct HostCapabilities {
    pub is_root: bool,
    pub kernel_version: (u32, u32, u32),
    pub cgroups_v2: bool,
    pub cgroup_controllers: Vec<String>,
    pub cgroup_subtree_delegatable: bool,
    pub overlay_fs: bool,
    pub systemd_available: bool,
    pub systemd_version: Option<u32>,
    pub minibox_slice_active: bool,
}

pub fn probe() -> HostCapabilities { /* reads procfs/sysfs/systemctl */ }
pub fn format_report(caps: &HostCapabilities) -> String { /* human-readable */ }
```

**Test helper macro:**

```rust
macro_rules! require_capability {
    ($caps:expr, $field:ident, $reason:expr) => {
        if !$caps.$field {
            eprintln!("SKIPPED: {}", $reason);
            return;
        }
    };
}
```

Replaces the current `require_root()` / `require_cgroups_v2()` panics with graceful skips in **new test files only**. Existing `integration_tests.rs` remains unchanged and keeps its `require_root()` / `require_cgroups_v2()` panic helpers. No dependencies beyond `std`.

### 2. Cgroup Integration Tests

**File:** `crates/miniboxd/tests/cgroup_tests.rs`

Tests the `ResourceLimiter` trait against real cgroupfs. Uses `MINIBOX_CGROUP_ROOT` env var for test isolation (already supported by `cgroups.rs`).

**Test isolation:** Each test sets `MINIBOX_CGROUP_ROOT` to a unique path. On systemd-managed hosts, tests cannot create top-level cgroups directly — the test harness uses `systemd-run --scope` to get a delegated subtree, then creates `minibox-test-{uuid}/` under that scope. A Drop guard cleans up the cgroup subtree.

```rust
struct CgroupTestGuard {
    root: PathBuf,        // e.g. /sys/fs/cgroup/system.slice/run-xxx.scope/minibox-test-abc123/
    _scope: Option<Child>, // systemd-run process, if used
}

impl CgroupTestGuard {
    fn new() -> Self {
        // Try delegated scope first (systemd hosts), fall back to direct creation (bare metal)
    }
}

impl Drop for CgroupTestGuard {
    fn drop(&mut self) {
        // Remove child cgroups, then the root dir
    }
}
```

**Env var override test:**
- `test_cgroup_root_env_override` — set `MINIBOX_CGROUP_ROOT` to test path, create via `ResourceLimiter` trait, verify cgroup appears under test path (not default `/sys/fs/cgroup/minibox`)

**Cgroup lifecycle tests:**
- `test_cgroup_create_and_verify_directory` — `limiter.create()`, verify dir exists
- `test_cgroup_memory_limit_written_and_readable` — set memory.max, read back, verify
- `test_cgroup_cpu_weight_written_and_readable` — same for cpu.weight
- `test_cgroup_pids_max_default` — verify default 1024 written when not specified
- `test_cgroup_pids_max_custom` — verify custom value
- `test_cgroup_io_max_written` — verify io.max line format
- `test_cgroup_add_process` — fork child, add PID, verify in cgroup.procs
- `test_cgroup_cleanup_removes_directory` — cleanup after exit, verify gone
- `test_cgroup_cleanup_idempotent` — cleanup already-removed cgroup is not an error

**Controller delegation tests:**
- `test_subtree_controllers_enabled` — verify parent's `cgroup.subtree_control` contains expected controllers
- `test_cgroup_in_delegated_subtree` — verify test cgroup root can create children and write limits

**Validation / error tests:**
- `test_cgroup_rejects_memory_below_minimum` — memory < 4096 returns error
- `test_cgroup_rejects_invalid_cpu_weight` — cpu_weight 0 or 10001 returns error
- `test_cgroup_add_process_invalid_pid` — adding invalid PID fails gracefully

**Controller availability tests:**
- `test_cgroup_io_controller_unavailable` — if `io` controller is not in `cgroup.controllers`, verify `limiter.create()` succeeds (non-fatal warning) but `io.max` is not written

### 3. Daemon+CLI E2E Tests

**File:** `crates/miniboxd/tests/e2e_tests.rs`

Starts real `miniboxd` and exercises `minibox` CLI as subprocesses.

**`DaemonFixture` helper:**
- Starts `miniboxd` with env overrides: temp `MINIBOX_DATA_DIR`, `MINIBOX_RUN_DIR`, `MINIBOX_SOCKET_PATH`, `MINIBOX_CGROUP_ROOT`
- Polls for socket file to appear (100ms intervals, 10s timeout). On timeout: panic with daemon stderr captured for debugging.
- `fn cli(&self, args: &[&str]) -> Command` — pre-configured Command with `--socket` pointing at temp socket
- `fn daemon_pid(&self) -> u32` — returns daemon PID for cgroup/process assertions
- `Drop` impl: SIGTERM → wait up to 5s → escalate to SIGKILL → cleanup cgroups/mounts/temps

```rust
struct DaemonFixture {
    child: Child,
    socket_path: PathBuf,
    data_dir: TempDir,
    run_dir: TempDir,
    cgroup_root: PathBuf,
    daemon_bin: PathBuf,
    cli_bin: PathBuf,
}

impl Drop for DaemonFixture {
    fn drop(&mut self) {
        // 1. Send SIGTERM
        // 2. Wait up to 5s for clean exit
        // 3. If still running, SIGKILL
        // 4. Cleanup cgroups (recursive child removal)
        // 5. Unmount any overlay mounts under data_dir
        // 6. TempDir handles the rest
    }
}
```

**Binary resolution:** The justfile `test-e2e` recipe runs `cargo build --release` before tests. `DaemonFixture` resolves binaries via path-based lookup:

```rust
fn find_binary(name: &str) -> PathBuf {
    // 1. Check MINIBOX_TEST_BIN_DIR env var (set by justfile or CI)
    // 2. Fall back to target/release/{name}
    // 3. Fall back to target/debug/{name}
    // Panic with clear message if not found
}
```

This avoids the `CARGO_BIN_EXE_` cross-crate limitation (`minibox` CLI is in `minibox-cli` crate, but e2e tests are in `miniboxd` crate).

**Parallelism:** `--test-threads=1` for v1. Each test gets isolated socket/data/cgroup paths so parallel is possible later.

**Image operations:**
- `test_e2e_pull_alpine` — exit 0, stdout contains "pulled"
- `test_e2e_pull_nonexistent` — non-zero exit, stderr has error

**Container lifecycle:**
- `test_e2e_run_echo` — run echo command, verify container created
- `test_e2e_ps_shows_container` — run sleep container, ps shows Running
- `test_e2e_stop_container` — stop running container, verify Stopped
- `test_e2e_rm_container` — rm stopped container, verify removed from ps
- `test_e2e_rm_running_rejected` — rm running container returns error

**Resource limits:**
- `test_e2e_run_with_memory_limit` — verify memory.max in cgroup
- `test_e2e_run_with_cpu_weight` — verify cpu.weight in cgroup

**Cleanup verification:**
- `test_e2e_cgroup_cleaned_after_rm` — run → stop → rm, cgroup dir gone
- `test_e2e_overlay_cleaned_after_rm` — same, no overlay mount remains

**Socket/auth:**
- `test_e2e_nonroot_rejected` — non-root connection refused (skip if always root)

**Supervisor cgroup migration:**
- `test_e2e_daemon_migrates_to_supervisor` — after daemon starts, read `/proc/{daemon_pid}/cgroup`, verify path ends with `/supervisor`. This validates the `migrate_to_supervisor_cgroup()` fix from commit `8e97d3f`.

**Signal handling:**
- `test_e2e_sigterm_clean_shutdown` — SIGTERM daemon, socket removed, exit 0

### 4. Justfile

**File:** `justfile` at repo root

```just
default:
    @just --list

# Preflight capability check
doctor:
    cargo run -p minibox-lib --example doctor 2>/dev/null || cargo test -p minibox-lib preflight -- --nocapture

# Unit tests (mock-based, any platform)
test-unit:
    cargo test --workspace --lib
    cargo test -p miniboxd --test handler_tests
    cargo test -p miniboxd --test conformance_tests

# Cgroup integration tests (Linux, root)
test-integration:
    sudo -E cargo test -p miniboxd --test cgroup_tests -- --test-threads=1 --nocapture
    sudo -E cargo test -p miniboxd --test integration_tests -- --test-threads=1 --ignored --nocapture

# Daemon+CLI e2e tests (Linux, root)
# Build and compile test binary as current user to avoid root-owned target/ files.
# Run the compiled test binary directly under sudo.
test-e2e:
    cargo build --release
    cargo test -p miniboxd --test e2e_tests --release --no-run --message-format=json 2>/dev/null | jq -r 'select(.executable) | .executable' > /tmp/minibox-e2e-bin
    sudo -E MINIBOX_TEST_BIN_DIR={{justfile_directory()}}/target/release $(cat /tmp/minibox-e2e-bin) --test-threads=1 --nocapture

# Full pipeline: clean state → doctor → all tests → clean state
test-all: nuke-test-state doctor test-unit test-integration test-e2e nuke-test-state

# Build release binaries
build:
    cargo build --release

# Remove all build artifacts
clean:
    cargo clean

# Remove only test-related build artifacts
clean-test:
    find target/debug/deps -name '*_tests-*' -delete 2>/dev/null || true
    find target/debug/deps -name '*miniboxd-*' -delete 2>/dev/null || true

# Remove target/ artifacts older than N days (default 7)
clean-stale days="7":
    find target/ -type f -mtime +{{days}} -delete 2>/dev/null || true
    find target/ -type d -empty -delete 2>/dev/null || true

# Kill orphan processes, unmount overlays, remove test cgroups, clean temp dirs
nuke-test-state:
    #!/usr/bin/env bash
    set -euo pipefail
    # Kill orphan miniboxd processes (test instances only, skip system service)
    pkill -f 'miniboxd.*minibox-test' 2>/dev/null || true
    # Unmount test overlay mounts
    mount | grep 'minibox-test' | awk '{print $3}' | xargs -r umount 2>/dev/null || true
    # Stop any systemd scopes from test runs
    systemctl list-units --type=scope --no-legend 2>/dev/null | grep minibox-test | awk '{print $1}' | xargs -r systemctl stop 2>/dev/null || true
    # Remove test cgroup trees (top-level and under systemd slices)
    find /sys/fs/cgroup -name 'minibox-test-*' -type d -exec rmdir {} \; 2>/dev/null || true
    # Clean temp dirs
    rm -rf /tmp/minibox-test-* 2>/dev/null || true
    echo "test state cleaned"
```

### 5. Modified Files

**`crates/minibox-lib/src/lib.rs`** — add `pub mod preflight;`

**`TESTING.md`** — update test pyramid counts, add just recipes, document the three test layers.

## Cleanup Strategy

### Runtime cleanup (per-test)

Each test uses a `TestGuard` or Drop-based cleanup:
- Cgroup dirs: remove `/sys/fs/cgroup/minibox-test-{uuid}/`
- Overlay mounts: unmount before removing dirs
- Temp dirs: `tempfile::TempDir` handles this
- Socket files: removed by DaemonFixture Drop
- Orphan processes: killed by DaemonFixture Drop

### Build artifact cleanup (justfile)

- `just clean` — full `cargo clean`
- `just clean-test` — remove test binary artifacts only, keep compiled deps
- `just clean-stale` — remove `target/` files older than N days

### CI-level cleanup

- `just test-all` wraps everything in `nuke-test-state` pre and post
- `just nuke-test-state` for manual recovery after crashes

## File Layout

**New files:**
```
justfile
crates/minibox-lib/src/preflight.rs
crates/miniboxd/tests/cgroup_tests.rs
crates/miniboxd/tests/e2e_tests.rs
```

**Modified files:**
```
crates/minibox-lib/src/lib.rs
TESTING.md
```

**Unchanged:**
```
crates/miniboxd/tests/integration_tests.rs
crates/miniboxd/tests/handler_tests.rs
crates/miniboxd/tests/conformance_tests.rs
```

## Dependencies

No new external crates. Uses existing workspace deps: `uuid`, `tempfile`, `nix`, `libc`, `tokio`.

## Test Counts

| Layer | New Tests | Existing | Total |
|-------|-----------|----------|-------|
| Unit (mock) | 0 | ~37 | ~37 |
| Conformance | 0 | ~15 | ~15 |
| Integration (cgroup) | ~17 | 0 | ~17 |
| Integration (existing) | 0 | 11 | 11 |
| E2E (daemon+CLI) | ~14 | 0 | ~14 |
| **Total** | **~31** | **~63** | **~94** |

Note: Existing test counts are approximate and should be verified during implementation.

## Future Work

- GitHub Actions self-hosted runner workflow
- `minibox doctor` CLI subcommand (wraps `preflight::probe()`)
- Compatibility matrix CI (Ubuntu/Debian/Fedora, multiple kernels)
- Parallel e2e test execution
- Cgroup telemetry/metrics tests
