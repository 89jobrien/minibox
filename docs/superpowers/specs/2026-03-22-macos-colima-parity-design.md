# macOS Colima Adapter Parity

**Date:** 2026-03-22
**Status:** Draft
**Scope:** Close the functional gaps in the Colima adapter suite so minibox can be dogfooded on macOS — streaming output, stop/remove lifecycle, io.max device detection, CI smoke tests, and Colima-specific benchmarks.

---

## Problem

The Colima adapter suite compiles and runs on macOS but is missing critical functionality:

- `minibox run` returns no stdout/stderr — `ColimaRuntime::spawn_process` backgrounds the container and returns `output_reader: None`
- `minibox stop` fails — the stored PID is an in-VM PID unreachable from the macOS host; `nix::kill()` on it is a no-op or hits the wrong process
- `io.max` silently fails — `ColimaLimiter` hardcodes device `8:0` but Colima VMs use virtio (`253:0`)
- No CI validation — GHA macOS job runs lint and unit tests only, never exercises the Colima path
- `handle_run_streaming` and `run_inner_capture` are gated `#[cfg(target_os = "linux")]` — ephemeral streaming is unreachable on macOS even if the adapter supported it

This blocks dogfooding minibox on macOS and using it as a CI execution environment.

---

## Goals

1. `minibox run alpine -- echo hello` prints `hello` on macOS
2. `minibox stop <id>` terminates a running container
3. `minibox rm <id>` cleans up overlay and cgroup state
4. `minibox ps` reflects correct status transitions (Running → Stopped)
5. io.max applies to the correct block device in the VM
6. GHA CI smoke tests the full macOS path
7. Benchmark suite quantifies Colima overhead

---

## Non-Goals

- Virtualization.framework support (Phase 2, separate spec)
- Docker Desktop / OrbStack adapter (future)
- Networking, exec, TTY (not yet on Linux either)
- Auth parity (`SO_PEERCRED` has no macOS equivalent; Colima delegates privilege to VM)
- Persistent state across daemon restarts
- Graceful SIGTERM to the in-VM container process (see Limitations)

---

## Key Insight: Host PID Tracking

The core design change: track the `limactl shell` process's **macOS host PID** instead of the in-VM container PID.

Currently `ColimaRuntime::spawn_process` backgrounds the container inside the VM (`&`) and returns the in-VM PID via `echo $!`. This PID is unreachable from the macOS host — you can't `kill()` or `waitpid()` on it.

Instead: run `limactl shell` as a foreground process with piped stdout. The limactl process is a regular macOS child process. Store its host PID in `DaemonState`. Now:

- **Streaming** works — limactl's stdout carries the container's output
- **Stop** works — `kill(limactl_host_pid, SIGTERM)` causes limactl to exit, which causes `unshare --kill-child` to SIGKILL the container process
- **Reaper** works — `waitpid(limactl_host_pid)` detects process exit on the macOS host
- **Remove** works — existing cleanup path already delegates through limactl

The existing `daemon_wait_for_exit` and `stop_inner` are gated `#[cfg(unix)]` which includes macOS — those work as-is. However, `handle_run_streaming` and `run_inner_capture` are gated `#[cfg(target_os = "linux")]` and must be widened to `#[cfg(unix)]`.

---

## Design

### 1. LimaSpawner Type

New executor type for streaming spawns, added to `ColimaRuntime` alongside the existing `LimaExecutor`:

```rust
/// Callable that starts a long-lived process inside the Lima VM,
/// returning the Child handle with piped stdout.
///
/// Default: `limactl shell <instance>` with Stdio::piped().
/// Tests: inject fake that returns a mock subprocess.
type LimaSpawner = Arc<dyn Fn(&[&str]) -> Result<std::process::Child> + Send + Sync>;
```

`LimaExecutor` (returns `Result<String>`) remains for fire-and-forget commands (cgroup writes, device probing, cleanup). `LimaSpawner` (returns `Result<Child>`) is used only by `spawn_process` for long-lived streaming containers.

Both are closure-based for testability (inject fakes in tests). A future cleanup could unify them into a `LimaBackend` trait with two methods, but closures are sufficient for now.

### 2. ColimaRuntime::spawn_process

Revised spawn script — no backgrounding, `exec` replaces shell:

```bash
CONFIG='<json>'
ROOTFS=$(echo "$CONFIG" | jq -r '.rootfs')
COMMAND=$(echo "$CONFIG" | jq -r '.command')
HOSTNAME=$(echo "$CONFIG" | jq -r '.hostname')
mapfile -t ARGS < <(echo "$CONFIG" | jq -r '.args[]')

exec unshare --pid --mount --uts --ipc --net \
    --fork --kill-child \
    chroot "$ROOTFS" "$COMMAND" "${ARGS[@]}"
```

Key changes from current:
- Removed `&` (no backgrounding)
- Removed `echo $!` (PID comes from host-side `child.id()`)
- Added `exec` so the shell is replaced by unshare (no extra process layer)

Rust side:

```rust
async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
    let script = build_spawn_script(config)?;
    let mut child = self.lima_spawn(&["sh", "-c", &script])?;
    let pid = child.id();
    let stdout_fd = child.stdout.take().map(OwnedFd::from);

    // Drop the Child handle. On Unix, Child::drop does NOT kill the
    // process — it only closes remaining stdio handles (all None after
    // take). The process continues running.
    //
    // INVARIANT: The daemon process is the direct parent of this Child
    // (it called Command::spawn). This means waitpid(pid) in the reaper
    // task will succeed — waitpid only works on direct children.
    drop(child);

    Ok(SpawnResult {
        pid,
        output_reader: stdout_fd,
    })
}
```

**SOLID note (DIP):** `SpawnResult` is a domain type — it stays unchanged (pid + output_reader). No `process_handle: Option<Child>` field. The `Child` is an infrastructure detail owned by the adapter; it is dropped after extracting the PID and stdout pipe. The existing `nix::kill(pid)` and `waitpid(pid)` calls in the handler work on the host PID without needing the `Child` handle.

### 3. Handler cfg Gate Widening

Three gates in `daemonbox/src/handler.rs` must change from `#[cfg(target_os = "linux")]` to `#[cfg(unix)]`:

| Line | Current | New | Function |
|------|---------|-----|----------|
| ~91 | `#[cfg(target_os = "linux")]` | `#[cfg(unix)]` | Ephemeral streaming dispatch in `handle_run` |
| ~134 | `#[cfg(target_os = "linux")]` | `#[cfg(unix)]` | `handle_run_streaming` function |
| ~241 | `#[cfg(target_os = "linux")]` | `#[cfg(unix)]` | `run_inner_capture` function |

These functions use `OwnedFd`, `waitpid`, and pipe operations — all available on any Unix platform via the `nix` crate. The Linux-specific gate was overly restrictive.

The `run_inner_capture` path calls `runtime.spawn_process()` which is trait-dispatched — on macOS it calls `ColimaRuntime::spawn_process` (returns limactl's piped stdout as the OwnedFd), on Linux it calls the native runtime (returns the clone pipe). The handler doesn't know or care which adapter produced the fd.

### 4. ColimaLimiter io.max Device Detection

Probe the VM's first block device at limiter construction:

```rust
impl ColimaLimiter {
    pub fn new(executor: LimaExecutor) -> Result<Self> {
        let block_device = executor(&["sh", "-c",
            "cat $(ls /sys/block/*/dev | head -1) 2>/dev/null"])
            .ok()
            .and_then(|s| {
                let trimmed = s.trim();
                if trimmed.contains(':') { Some(trimmed.to_string()) } else { None }
            });

        Ok(Self { executor, block_device })
    }
}
```

If no block device is found, io.max writes are skipped (best-effort, same as `GkeLimiter`). This mirrors the native Linux path which already has `find_first_block_device()`.

### 5. Composition Root Update (macbox/src/lib.rs)

`macbox::start()` is the composition root that wires Colima adapters. All four Colima adapters use the `with_executor()` builder pattern — we follow the same pattern for new fields:

- `ColimaLimiter::new().with_executor(executor.clone())` — executor enables io.max device probing at construction
- `ColimaRuntime::new().with_executor(executor.clone()).with_spawner(spawner)` — spawner enables streaming subprocess creation

This is consistent with the existing wiring:
```rust
// Current pattern (ColimaRegistry, ColimaRuntime use with_executor):
let registry = ColimaRegistry::new().with_executor(executor.clone());
// New: same pattern extended to limiter and runtime gets spawner too
let limiter = ColimaLimiter::new().with_executor(executor.clone());
let runtime = ColimaRuntime::new()
    .with_executor(executor.clone())
    .with_spawner(spawner);
```

### 6. CI Workflow

New job in `.github/workflows/ci.yml`:

```yaml
macos-colima:
  runs-on: macos-latest  # macOS 14+ required for --vm-type vz
  needs: [lint]  # run after existing lint job passes
  steps:
    - uses: actions/checkout@v4
    - name: Install Colima
      run: brew install colima lima
    - name: Start Colima
      run: colima start --vm-type vz --cpu 2 --memory 2
    - name: Build
      run: cargo build -p miniboxd -p minibox-cli --release
    - name: Smoke test
      run: |
        ./target/release/miniboxd &
        DAEMON_PID=$!
        sleep 2
        OUTPUT=$(./target/release/minibox run alpine -- echo hello)
        [ "$OUTPUT" = "hello" ] || exit 1
        kill $DAEMON_PID
```

**Note:** `--vm-type vz` requires macOS 13 (Ventura) or later with Virtualization.framework. GHA `macos-latest` is currently macOS 14 (Sonoma). If runners ever downgrade, this will need a version guard or fallback to `--vm-type qemu`.

### 7. Benchmark Suite

New `--suite colima` in `minibox-bench`:

| Measurement | What it captures |
|-------------|-----------------|
| `colima/limactl-roundtrip` | Baseline: `limactl shell minibox -- echo ok` latency |
| `colima/spawn-to-first-byte` | Time from `spawn_process()` call to first byte read from output pipe |
| `colima/stop-latency` | Time from host PID kill to `waitpid` return |

Gated behind macOS + Colima-running check. Results flow to `bench/results/bench.jsonl` and `bench/results/latest.json` via the existing pipeline.

---

## Changes Summary

| File | Change |
|------|--------|
| `minibox-lib/src/adapters/colima.rs` | Rewrite `spawn_process` for streaming; add `LimaSpawner`; fix io.max device detection in `ColimaLimiter::new` |
| `daemonbox/src/handler.rs` | Widen three `#[cfg(target_os = "linux")]` gates to `#[cfg(unix)]` |
| `macbox/src/lib.rs` | Update composition root: pass executor to `ColimaLimiter::new`, create and pass `LimaSpawner` to `ColimaRuntime` |
| `minibox-bench/src/main.rs` | Add `colima` suite with three measurements |
| `.github/workflows/ci.yml` | New `macos-colima` job |

**Not changed:** `domain.rs` (`SpawnResult` unchanged), `state.rs` (`DaemonState` unchanged), `server.rs`, `minibox-cli`, protocol types.

---

## Testing

1. **Unit tests** (any platform): mock `LimaSpawner` returning `Child` from simple subprocesses (`echo hello`, `sleep 10`). Verify PID extraction, output reading, stop/reaper behavior. Mock `LimaExecutor` for io.max device probe.
2. **Integration tests** (macOS + Colima): full `run` → streaming output, `stop`, `rm`, `ps` lifecycle, exit code propagation.
3. **CI smoke test** (GHA macOS): subset of integration tests in `macos-colima` job.

---

## Limitations

### Stop is SIGKILL, not SIGTERM

`unshare --kill-child` sends **SIGKILL** (not SIGTERM) to the container process when the unshare parent exits. The signal chain on stop:

1. Handler sends SIGTERM to limactl host PID
2. limactl exits (either forwards SIGHUP or drops connection)
3. In-VM unshare process loses its parent/connection and exits
4. `--kill-child` causes unshare to send SIGKILL to the forked container process

This means containers do not get a chance to handle SIGTERM gracefully (flush buffers, close connections, etc.).

**Future fix:** Add a `ContainerRuntime::signal()` trait method (proper DIP — adapter decides HOW to signal, handler decides WHEN). `ColimaRuntime::signal` would send `kill -TERM <in-vm-pid>` via a separate `limactl shell` call. This requires tracking the in-VM PID (e.g. writing it to `/tmp/minibox-pids/<id>` before exec in the spawn script). Deferred because: most dogfooding containers are short-lived ephemeral runs, and the SIGKILL fallback is safe.

### limactl stdout piping is unverified

The design assumes `limactl shell` faithfully pipes VM stdout to the host process's stdout when invoked with `Stdio::piped()`. This is the foundational assumption for Goal #1 (streaming output). `limactl shell` is a user-facing tool — its stdio behavior as a subprocess is not part of a documented stable API.

**Mitigation:** Validate empirically before implementation:
```bash
# Run on macOS with Colima running:
echo 'echo hello world' | limactl shell minibox
# Verify: stdout contains "hello world"
```

If limactl buffers or transforms output, we may need to use Lima's raw SSH transport instead (`ssh -p <port> localhost`).

---

## Risks

- **limactl signal propagation** — Unverified whether SIGTERM to limactl cleanly propagates to the VM process. `--kill-child` on unshare provides the safety net: even if limactl's signal handling is opaque, the container will be killed (forcefully) when limactl exits. Test the full chain in integration tests.
- **Colima startup time in CI** — Adds ~60s. Acceptable for a dedicated smoke job that runs after lint passes.
- **Lima mount latency** — `/tmp` shared mount may add latency to layer extraction. Benchmark suite will quantify.
- **macOS 13+ requirement** — `--vm-type vz` requires Ventura or later. GHA `macos-latest` is currently macOS 14 but this is not guaranteed. Add a version guard if runners change.
