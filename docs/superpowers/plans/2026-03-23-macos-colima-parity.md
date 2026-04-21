# macOS Colima Adapter Parity — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the Colima adapter gaps so minibox can be dogfooded on macOS — streaming output, stop/remove, io.max fix, CI, benchmarks.

**Architecture:** Track the `limactl shell` host PID instead of the in-VM PID. Spawn limactl with `Stdio::piped()` to capture container stdout. Existing handler stop/reaper logic works on any Unix (macOS included) because it uses `nix::kill` and `waitpid` on PIDs. The `container` module is Linux-only, so the handler gets a local `wait_for_exit` helper for the streaming path.

**Tech Stack:** Rust, nix crate (Unix signals/waitpid), std::process::Command, tokio (async runtime), GitHub Actions CI

**Spec:** `docs/superpowers/specs/2026-03-22-macos-colima-parity-design.md`

---

## File Map

| File                                    | Action | Responsibility                                                                                                                 |
| --------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------------------------------ |
| `crates/minibox/src/adapters/colima.rs` | Modify | Add `LimaSpawner` type, `with_spawner()` builder, rewrite `spawn_process`, add `block_device` field + probe to `ColimaLimiter` |
| `crates/daemonbox/src/handler.rs`       | Modify | Widen 3 cfg gates, add local `wait_for_exit` helper, conditionally import `wait_for_exit`                                      |
| `crates/macbox/src/lib.rs`              | Modify | Wire `LimaSpawner` closure and executor into adapter constructors                                                              |
| `crates/minibox-bench/src/main.rs`      | Modify | Add `colima` benchmark suite                                                                                                   |
| `.github/workflows/ci.yml`              | Modify | Add `macos-colima` smoke test job                                                                                              |

---

## Task 1: Add `LimaSpawner` Type and Builder to `ColimaRuntime`

**Files:**

- Modify: `crates/minibox/src/adapters/colima.rs:38-45` (type alias area)
- Modify: `crates/minibox/src/adapters/colima.rs:544-570` (ColimaRuntime struct + impl)
- Test: `crates/minibox/src/adapters/colima.rs` (existing test module)

- [ ] **Step 1: Write failing test — `spawn_process_returns_piped_output`**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `colima.rs`:

```rust
/// spawn_process must return a readable output_reader when a LimaSpawner is set.
#[tokio::test]
async fn spawn_process_returns_piped_output() {
    use crate::domain::{ContainerHooks, ContainerSpawnConfig};
    use std::io::Read;

    let runtime = ColimaRuntime::new().with_spawner(Arc::new(|_args: &[&str]| {
        // Spawn a real subprocess that prints to stdout
        std::process::Command::new("echo")
            .arg("hello from container")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))
    }));

    let config = ContainerSpawnConfig {
        rootfs: PathBuf::from("/tmp/rootfs"),
        command: "/bin/echo".to_string(),
        args: vec!["hello".to_string()],
        env: vec![],
        hostname: "test".to_string(),
        cgroup_path: PathBuf::from("/sys/fs/cgroup/minibox/test"),
        capture_output: true,
        hooks: ContainerHooks::default(),
    };

    let result = runtime.spawn_process(&config).await.unwrap();
    assert!(result.pid > 0, "PID must be positive");
    assert!(result.output_reader.is_some(), "output_reader must be Some when spawner is set");

    // Read the output from the pipe
    let fd = result.output_reader.unwrap();
    let mut file = std::fs::File::from(fd);
    let mut output = String::new();
    file.read_to_string(&mut output).unwrap();
    assert!(output.contains("hello from container"), "output was: {output}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minibox spawn_process_returns_piped_output -- --nocapture`
Expected: FAIL — `ColimaRuntime` has no `with_spawner` method.

- [ ] **Step 3: Add `LimaSpawner` type alias and `with_spawner` builder**

In `crates/minibox/src/adapters/colima.rs`, after the existing `LimaExecutor` type alias (line ~45), add:

```rust
/// Callable that starts a long-lived process inside the Lima VM,
/// returning the [`Child`](std::process::Child) handle with piped stdout.
///
/// The default implementation invokes `limactl shell <instance> <args...>`
/// with [`Stdio::piped`](std::process::Stdio::piped) stdout.
/// Tests inject a fake closure via [`ColimaRuntime::with_spawner`] to
/// avoid real `limactl` calls.
pub type LimaSpawner = Arc<dyn Fn(&[&str]) -> Result<std::process::Child> + Send + Sync>;
```

Also make `LimaExecutor` public:

```rust
pub type LimaExecutor = Arc<dyn Fn(&[&str]) -> Result<String> + Send + Sync>;
```

In `ColimaRuntime` struct (line ~544), add a field:

```rust
/// Optional injected spawner for streaming output.
spawner: Option<LimaSpawner>,
```

In `ColimaRuntime::new()` (line ~555), initialise: `spawner: None,`

Add a new builder method after `with_executor`:

```rust
/// Inject a custom spawner for streaming output.
///
/// The closure receives the argument slice that would be passed to
/// `limactl shell <instance>` and must return a [`Child`] with piped stdout.
pub fn with_spawner(mut self, spawner: LimaSpawner) -> Self {
    self.spawner = Some(spawner);
    self
}
```

Add a `lima_spawn` method alongside the existing `lima_exec`:

```rust
/// Start a long-lived process inside the Lima VM, returning the [`Child`] handle.
///
/// If an injected spawner is present it is used instead of a real
/// `limactl` subprocess — this is the test seam.
fn lima_spawn(&self, args: &[&str]) -> Result<std::process::Child> {
    if let Some(spawner) = &self.spawner {
        return spawner(args);
    }
    Command::new(&self.limactl_path)
        .arg("shell")
        .arg(&self.instance)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn limactl: {e}"))
}
```

- [ ] **Step 4: Rewrite `spawn_process` to use `lima_spawn` when spawner is available**

Replace `ColimaRuntime::spawn_process` (lines ~626-672) with:

```rust
async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
    let config_json = serde_json::to_string(&SpawnRequest {
        rootfs: config.rootfs.to_string_lossy().to_string(),
        command: config.command.clone(),
        args: config.args.clone(),
        env: config.env.clone(),
        hostname: config.hostname.clone(),
        cgroup_path: config.cgroup_path.to_string_lossy().to_string(),
    })
    .map_err(|e| anyhow!("Failed to serialize config: {e}"))?;

    // When a spawner is available, run limactl as a foreground process
    // with piped stdout — enabling streaming output to the client.
    if self.spawner.is_some() {
        let spawn_script = format!(
            r#"
            CONFIG='{config_json}'

            ROOTFS=$(echo "$CONFIG" | jq -r '.rootfs')
            COMMAND=$(echo "$CONFIG" | jq -r '.command')
            HOSTNAME=$(echo "$CONFIG" | jq -r '.hostname')

            # Build args array from JSON
            mapfile -t ARGS < <(echo "$CONFIG" | jq -r '.args[]')

            exec unshare --pid --mount --uts --ipc --net \
                --fork --kill-child \
                chroot "$ROOTFS" "$COMMAND" "${{ARGS[@]}}"
            "#
        );

        let mut child = self.lima_spawn(&["sh", "-c", &spawn_script])?;
        let pid = child.id();
        let stdout_fd = child.stdout.take().map(std::os::fd::OwnedFd::from);

        // INVARIANT: The daemon process is the direct parent of this Child
        // (it called Command::spawn). waitpid(pid) in the reaper will succeed.
        // On Unix, Child::drop does NOT kill the process — it only closes
        // remaining stdio handles (all None after take).
        drop(child);

        return Ok(SpawnResult {
            pid,
            output_reader: stdout_fd,
        });
    }

    // Fallback: background the process and return the in-VM PID.
    // This path is used when no spawner is injected (e.g. fire-and-forget runs).
    let spawn_script = format!(
        r#"
        CONFIG='{config_json}'

        ROOTFS=$(echo "$CONFIG" | jq -r '.rootfs')
        COMMAND=$(echo "$CONFIG" | jq -r '.command')
        HOSTNAME=$(echo "$CONFIG" | jq -r '.hostname')
        CGROUP=$(echo "$CONFIG" | jq -r '.cgroup_path')

        # Build args array from JSON
        mapfile -t ARGS < <(echo "$CONFIG" | jq -r '.args[]')

        unshare --pid --mount --uts --ipc --net \
            --fork --kill-child \
            chroot "$ROOTFS" "$COMMAND" "${{ARGS[@]}}" &

        echo $!
        "#
    );

    let output = self.lima_exec(&["sh", "-c", &spawn_script])?;
    let pid: u32 = output
        .trim()
        .parse()
        .map_err(|e| anyhow!("Invalid PID returned: {e}"))?;

    Ok(SpawnResult {
        pid,
        output_reader: None,
    })
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p minibox spawn_process_returns_piped_output -- --nocapture`
Expected: PASS

- [ ] **Step 6: Run existing tests to verify no regressions**

Run: `cargo test -p minibox -- colima`
Expected: All existing colima tests pass (spawn_process_includes_args_in_script, etc.)

- [ ] **Step 7: Commit**

```bash
git add crates/minibox/src/adapters/colima.rs
git commit -m "feat(colima): add LimaSpawner for streaming output from containers"
```

---

## Task 2: Fix io.max Device Detection in `ColimaLimiter`

**Files:**

- Modify: `crates/minibox/src/adapters/colima.rs:409-533` (ColimaLimiter)
- Test: `crates/minibox/src/adapters/colima.rs` (test module)

- [ ] **Step 1: Write failing test — `limiter_detects_block_device`**

```rust
#[test]
fn limiter_detects_block_device() {
    let limiter = ColimaLimiter::new().with_executor(Arc::new(|args: &[&str]| {
        let joined = args.join(" ");
        if joined.contains("/sys/block") {
            Ok("253:0\n".to_string())
        } else {
            Ok(String::new())
        }
    }));
    assert_eq!(limiter.block_device.as_deref(), Some("253:0"));
}

#[test]
fn limiter_io_max_uses_detected_device() {
    let commands = Arc::new(std::sync::Mutex::new(Vec::new()));
    let cmds = commands.clone();
    let limiter = ColimaLimiter::new().with_executor(Arc::new(move |args: &[&str]| {
        cmds.lock().unwrap().push(args.join(" "));
        if args.join(" ").contains("/sys/block") {
            Ok("253:0\n".to_string())
        } else {
            Ok(String::new())
        }
    }));

    let config = minibox::domain::ResourceConfig {
        memory_limit_bytes: None,
        cpu_weight: None,
        pids_max: None,
        io_max_bytes_per_sec: Some(1048576),
    };
    limiter.create("test-container", &config).unwrap();

    let all = commands.lock().unwrap();
    let io_cmd = all.iter().find(|c| c.contains("io.max")).expect("should write io.max");
    assert!(io_cmd.contains("253:0"), "should use detected device, got: {io_cmd}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p minibox limiter_detects_block_device limiter_io_max_uses_detected_device -- --nocapture`
Expected: FAIL — `ColimaLimiter` has no `with_executor` method or `block_device` field.

- [ ] **Step 3: Add `block_device` field and `with_executor` builder to `ColimaLimiter`**

Add `block_device: Option<String>` field to `ColimaLimiter` struct. Update `new()` to init it as `None`.

Add `with_executor`:

```rust
/// Inject a custom executor and probe the VM's block device for io.max.
pub fn with_executor(mut self, executor: LimaExecutor) -> Self {
    // Probe block device — best-effort, io.max is optional.
    self.block_device = executor(&["sh", "-c",
        "cat $(ls /sys/block/*/dev | head -1) 2>/dev/null"])
        .ok()
        .and_then(|s| {
            let trimmed = s.trim().to_string();
            if trimmed.contains(':') { Some(trimmed) } else { None }
        });
    self.executor = Some(executor);
    self
}
```

- [ ] **Step 4: Update `create` to use `block_device` instead of hardcoded `8:0`**

Replace the io.max block in `ResourceLimiter::create`:

```rust
if let Some(io_max) = config.io_max_bytes_per_sec {
    if let Some(ref device) = self.block_device {
        let io_file = format!("{cgroup_path}/io.max");
        self.lima_exec(&[
            "sh",
            "-c",
            &format!("echo '{device} rbps={io_max} wbps={io_max}' > {io_file}"),
        ])?;
    }
    // If no block device detected, skip io.max (best-effort).
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p minibox limiter_detects_block_device limiter_io_max_uses_detected_device -- --nocapture`
Expected: PASS

- [ ] **Step 6: Run all colima tests for regressions**

Run: `cargo test -p minibox -- colima`
Expected: All pass

- [ ] **Step 7: Commit**

```bash
git add crates/minibox/src/adapters/colima.rs
git commit -m "fix(colima): detect VM block device for io.max instead of hardcoding 8:0"
```

---

## Task 3: Widen Handler cfg Gates for macOS Streaming

**Files:**

- Modify: `crates/daemonbox/src/handler.rs:91,134,241`
- Test: compile check on macOS

This is the most delicate task. Three issues to solve:

1. Three `#[cfg(target_os = "linux")]` gates must become `#[cfg(unix)]`
2. `use minibox::container::process::wait_for_exit` — the `container` module is gated `#[cfg(target_os = "linux")]` in `minibox/src/lib.rs:32`, so this import fails on macOS
3. Solution: add a local `handler_wait_for_exit` helper

- [ ] **Step 1: Add local `handler_wait_for_exit` helper**

Add this function in `handler.rs` just above `daemon_wait_for_exit` (around line 590):

```rust
/// Wait for a process to exit and return its exit code.
///
/// Thin wrapper around `waitpid` usable on any Unix platform.
/// The `minibox::container::process::wait_for_exit` variant is only
/// available on Linux (the `container` module is gated
/// `#[cfg(target_os = "linux")]`). This local version provides the same
/// functionality for the macOS streaming path.
#[cfg(unix)]
fn handler_wait_for_exit(pid: u32) -> Result<i32> {
    use nix::sys::wait::{WaitStatus, waitpid};
    use nix::unistd::Pid;
    let nix_pid = Pid::from_raw(pid as i32);
    match waitpid(nix_pid, None) {
        Ok(WaitStatus::Exited(_, code)) => Ok(code),
        Ok(WaitStatus::Signaled(_, sig, _)) => Ok(-(sig as i32)),
        Ok(other) => {
            info!(pid = pid, wait_status = ?other, "handler_wait_for_exit: unexpected status");
            Ok(-1)
        }
        Err(e) => {
            warn!(pid = pid, error = %e, "handler_wait_for_exit: waitpid error");
            Ok(-1)
        }
    }
}
```

- [ ] **Step 2: Widen the three cfg gates and update the wait_for_exit call**

Change line ~91 in `handle_run`:

```rust
// Before:  #[cfg(target_os = "linux")]
// After:   #[cfg(unix)]
```

Change line ~134 on `handle_run_streaming`:

```rust
// Before:  #[cfg(target_os = "linux")]
// After:   #[cfg(unix)]
```

Change line ~241 on `run_inner_capture` (update doc comment too):

```rust
// Before:
/// Only compiled on Linux because the output pipe requires Linux primitives.
#[cfg(target_os = "linux")]

// After:
/// Compiled on Unix (Linux and macOS). The output pipe uses `OwnedFd`
/// and `waitpid` — both available on any Unix via the `nix` crate.
#[cfg(unix)]
```

Inside `handle_run_streaming`, replace the `wait_for_exit` call (lines ~207-211):

```rust
// Before:
use minibox::container::process::wait_for_exit;
let exit_code = tokio::task::spawn_blocking(move || wait_for_exit(pid))
    .await
    .unwrap_or(Ok(-1))
    .unwrap_or(-1);

// After:
let exit_code = tokio::task::spawn_blocking(move || handler_wait_for_exit(pid))
    .await
    .unwrap_or(Ok(-1))
    .unwrap_or(-1);
```

- [ ] **Step 3: Verify compilation on macOS**

Run: `cargo check -p daemonbox`
Expected: Compiles without errors.

- [ ] **Step 4: Run existing handler tests**

Run: `cargo test -p daemonbox`
Expected: All existing tests pass.

- [ ] **Step 5: Verify full workspace compiles**

Run: `cargo check --workspace`
Expected: Clean compile.

- [ ] **Step 6: Commit**

```bash
git add crates/daemonbox/src/handler.rs
git commit -m "feat(daemonbox): widen streaming cfg gates from linux-only to unix

Enables handle_run_streaming and run_inner_capture on macOS.
Adds local handler_wait_for_exit to avoid dependency on the
Linux-gated container::process module."
```

---

## Task 4: Wire Spawner into macOS Composition Root

**Files:**

- Modify: `crates/macbox/src/lib.rs:109-117`
- Modify: `crates/minibox/src/adapters/colima.rs` (make types public, may already be done in Task 1)
- Test: compile check + `cargo test -p macbox`

- [ ] **Step 1: Verify LimaExecutor and LimaSpawner are pub**

Check `crates/minibox/src/adapters/colima.rs` — both type aliases should be `pub` (done in Task 1). Also check `crates/minibox/src/adapters/mod.rs` to verify they are re-exported. If `mod.rs` uses `pub use colima::*;` they are covered. If not, add:

```rust
pub use colima::{LimaExecutor, LimaSpawner};
```

- [ ] **Step 2: Update the dependency injection block in macbox**

Replace the adapter wiring at lines 109-117 in `macbox/src/lib.rs`:

```rust
// ── Dependency Injection — Colima adapter suite ──────────────────────
// Shared executor closure — runs commands inside the Lima VM.
let executor: minibox::adapters::LimaExecutor =
    Arc::new(|args: &[&str]| {
        let output = std::process::Command::new("limactl")
            .arg("shell")
            .arg("colima")
            .args(args)
            .output()
            .map_err(|e| anyhow::anyhow!("limactl exec failed: {e}"))?;
        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "limactl command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    });

// Spawner closure — starts a long-lived process with piped stdout.
let spawner: minibox::adapters::LimaSpawner =
    Arc::new(|args: &[&str]| {
        std::process::Command::new("limactl")
            .arg("shell")
            .arg("colima")
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow::anyhow!("limactl spawn failed: {e}"))
    });

let deps = Arc::new(HandlerDependencies {
    registry: Arc::new(ColimaRegistry::new().with_executor(executor.clone())),
    filesystem: Arc::new(ColimaFilesystem::new()),
    resource_limiter: Arc::new(ColimaLimiter::new().with_executor(executor.clone())),
    runtime: Arc::new(ColimaRuntime::new()
        .with_executor(executor)
        .with_spawner(spawner)),
    containers_base: containers_dir,
    run_containers_base: run_containers_dir,
});
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p macbox`
Expected: Clean compile.

- [ ] **Step 4: Run macbox tests**

Run: `cargo test -p macbox`
Expected: All tests pass.

- [ ] **Step 5: Run full workspace check**

Run: `cargo check --workspace`
Expected: Clean compile.

- [ ] **Step 6: Run the macOS quality gates**

Run: `cargo fmt --all --check && cargo clippy -p minibox -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -p minibox-llm -p minibox-secrets -- -D warnings && cargo xtask test-unit`
Expected: All pass.

- [ ] **Step 7: Commit**

```bash
git add crates/minibox/src/adapters/colima.rs crates/minibox/src/adapters/mod.rs crates/macbox/src/lib.rs
git commit -m "feat(macbox): wire LimaSpawner and executor into composition root

Enables streaming output and io.max device detection on macOS.
Makes LimaExecutor and LimaSpawner public for use from macbox."
```

---

## Task 5: Add Colima Benchmark Suite

**Depends on:** Tasks 1-4 (streaming must work for spawn-to-first-byte and stop-latency measurements)

**Files:**

- Modify: `crates/minibox-bench/src/main.rs`
- Test: `--suite colima --dry-run`

- [ ] **Step 1: Add `colima` to the suite list**

In `planned_suites` (line ~579), add `"colima"` to the array. In `suite_enabled` (line ~564), add `"colima"` to the explicit-only list alongside `"codec"` and `"adapter"`. Update `print_help` to include `colima`.

- [ ] **Step 2: Add Colima availability check**

```rust
/// Check if Colima is installed and running (macOS only).
fn colima_available() -> bool {
    cfg!(target_os = "macos")
        && std::process::Command::new("colima")
            .arg("status")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}
```

- [ ] **Step 3: Implement `bench_colima_suite`**

```rust
fn bench_colima_suite(cfg: &BenchConfig) -> SuiteResult {
    let iters = cfg.iters.max(5);
    let mut tests = Vec::new();

    if !colima_available() {
        eprintln!("warn: colima suite skipped — Colima not running");
        return SuiteResult { name: "colima".to_string(), tests: vec![] };
    }

    // 1. Limactl round-trip baseline
    tests.push({
        let mut durations = Vec::new();
        for _ in 0..iters {
            let start = std::time::Instant::now();
            let _ = std::process::Command::new("limactl")
                .args(["shell", "colima", "echo", "ok"])
                .output();
            durations.push(start.elapsed().as_micros() as u64);
        }
        let stats = if durations.is_empty() { None } else { Some(Stats::from_samples(&durations)) };
        TestResult {
            name: "limactl-roundtrip".to_string(),
            iterations: durations.len(),
            durations_micros: durations,
            durations_nanos: vec![],
            stats,
            unit: String::new(),
        }
    });

    // 2. Spawn-to-first-byte (requires running miniboxd)
    let minibox_bin = std::env::var("MINIBOX_BIN").unwrap_or_else(|_| "minibox".to_string());
    if std::process::Command::new(&minibox_bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        tests.push({
            let mut durations = Vec::new();
            for _ in 0..iters {
                let start = std::time::Instant::now();
                let result = run_cmd(&minibox_bin, &["run", "alpine", "--", "echo", "bench-marker"]);
                let elapsed = start.elapsed().as_micros() as u64;
                if let Ok(r) = result {
                    if r.success && r.stdout.contains("bench-marker") {
                        durations.push(elapsed);
                    }
                }
            }
            let stats = if durations.is_empty() { None } else { Some(Stats::from_samples(&durations)) };
            TestResult {
                name: "spawn-to-first-byte".to_string(),
                iterations: durations.len(),
                durations_micros: durations,
                durations_nanos: vec![],
                stats,
                unit: String::new(),
            }
        });
    }

    // 3. Stop latency (requires running miniboxd with a container to stop)
    if std::process::Command::new(&minibox_bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        tests.push({
            let mut durations = Vec::new();
            for _ in 0..iters {
                // Start a long-running container
                let run_result = run_cmd(&minibox_bin, &["run", "alpine", "--", "sleep", "300"]);
                if let Ok(r) = run_result {
                    if r.success {
                        let container_id = r.stdout.trim().to_string();
                        let start = std::time::Instant::now();
                        let stop_result = run_cmd(&minibox_bin, &["stop", &container_id]);
                        let elapsed = start.elapsed().as_micros() as u64;
                        if stop_result.is_ok() {
                            durations.push(elapsed);
                        }
                        // Clean up
                        let _ = run_cmd(&minibox_bin, &["rm", &container_id]);
                    }
                }
            }
            let stats = if durations.is_empty() { None } else { Some(Stats::from_samples(&durations)) };
            TestResult {
                name: "stop-latency".to_string(),
                iterations: durations.len(),
                durations_micros: durations,
                durations_nanos: vec![],
                stats,
                unit: String::new(),
            }
        });
    }

    SuiteResult { name: "colima".to_string(), tests }
}
```

- [ ] **Step 4: Wire into `run_suites`**

Add after the existing suite blocks:

```rust
if suite_enabled(cfg, "colima") {
    suites.push(bench_colima_suite(cfg));
}
```

- [ ] **Step 5: Run dry-run to verify suite structure**

Run: `cargo run -p minibox-bench -- --suite colima --dry-run`
Expected: Report shows empty colima suite.

- [ ] **Step 6: Run existing bench tests**

Run: `cargo test -p minibox-bench`
Expected: All existing tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/minibox-bench/src/main.rs
git commit -m "feat(bench): add colima suite — limactl-roundtrip, spawn-to-first-byte, stop-latency"
```

---

## Task 6: Add CI Colima Smoke Test Job

**Files:**

- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Add `macos-colima` job**

Append after the existing `coverage` job:

```yaml
macos-colima:
  name: macOS Colima smoke
  runs-on: macos-latest
  needs: [lint]
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - name: Install Colima + Lima
      run: brew install colima lima
    - name: Start Colima VM
      run: |
        colima start --vm-type vz --cpu 2 --memory 2
        colima status
    - name: Build daemon and CLI
      run: cargo build -p miniboxd -p minibox-cli --release
    - name: Smoke test
      run: |
        ./target/release/miniboxd &
        DAEMON_PID=$!
        sleep 3
        ./target/release/minibox pull alpine
        OUTPUT=$(./target/release/minibox run alpine -- /bin/echo "hello from minibox")
        echo "Output: $OUTPUT"
        echo "$OUTPUT" | grep -q "hello from minibox"
        kill $DAEMON_PID
        echo "Smoke test passed"
```

- [ ] **Step 2: Verify YAML syntax**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`
Expected: No errors.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add macOS Colima smoke test job

Installs Colima with VZ backend on macOS-latest, builds miniboxd+CLI,
and verifies streaming output from an ephemeral alpine container."
```

---

## Task 7: Final Integration Verification

- [ ] **Step 1: Run full macOS quality gates**

Run: `cargo xtask pre-commit`
Expected: fmt + clippy + build all pass.

- [ ] **Step 2: Run full unit test suite**

Run: `cargo xtask test-unit`
Expected: All tests pass (257+ tests, 4 skipped on macOS).

- [ ] **Step 3: Verify the complete streaming path compiles**

Run: `cargo build -p miniboxd --release`
Expected: Clean build. The streaming path is now active on macOS.

- [ ] **Step 4: Commit any fixups from steps 1-3**

If any issues were found, fix and commit individually.
