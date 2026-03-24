# Sandbox Tests Design

**Date:** 2026-03-24
**Status:** Approved

## Overview

Add a dedicated `sandbox_tests.rs` integration test file that validates minibox's contract as an AI agent code-execution sandbox. Tests exercise real images (alpine, python:3.12-alpine) in real containers via the CLI, verifying output capture, exit codes, network isolation, filesystem containment, resource limits, and concurrency isolation.

A `SandboxClient` abstraction wraps `DaemonFixture` with an agent-oriented API, modelling how an AI agent would actually invoke minibox.

---

## 1. File Structure

### New files

**`crates/miniboxd/tests/helpers/mod.rs`** — shared test infrastructure extracted from `e2e_tests.rs`:
- `DaemonFixture` (moved from `e2e_tests.rs`, unchanged)
- `SandboxClient` — agent-oriented wrapper around `DaemonFixture`
- `ExecResult` — structured return type: stdout, stderr, exit_code, duration

**`crates/miniboxd/tests/sandbox_tests.rs`** — 15 sandbox scenario tests using `SandboxClient`.

### Modified files

**`crates/miniboxd/tests/e2e_tests.rs`** — replace inline `DaemonFixture` with `mod helpers; use helpers::DaemonFixture;`. No behavior change, just import path.

**`xtask/src/main.rs`** — add `test-sandbox` subcommand (same gating as `test-e2e-suite`: Linux, root, `--test-threads=1`).

**`Justfile`** — add `test-sandbox` recipe.

---

## 2. SandboxClient API

```rust
use std::collections::HashSet;
use std::time::Duration;

pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration: Duration,
}

pub struct SandboxClient {
    fixture: DaemonFixture,
    pulled_images: HashSet<String>,
}

impl SandboxClient {
    /// Start a fresh daemon and return a client ready for sandbox operations.
    pub fn start() -> Self {
        Self {
            fixture: DaemonFixture::start(),
            pulled_images: HashSet::new(),
        }
    }

    /// Pull an image if not already cached in this session.
    pub fn ensure_image(&mut self, image: &str) {
        if self.pulled_images.contains(image) {
            return;
        }
        let (ok, _stdout, stderr) = self.fixture.run_cli(&["pull", image]);
        assert!(ok, "failed to pull {image}: {stderr}");
        self.pulled_images.insert(image.to_string());
    }

    /// Execute a command in a fresh ephemeral container.
    /// Calls `minibox run <image> -- <cmd...>` via the CLI.
    pub fn execute(&mut self, image: &str, cmd: &[&str]) -> ExecResult {
        self.ensure_image(image);
        let start = std::time::Instant::now();
        let mut args = vec!["run", image, "--"];
        args.extend_from_slice(cmd);
        let (exit_code, stdout, stderr) = self.fixture.run_cli_with_exit_code(&args);
        ExecResult {
            stdout,
            stderr,
            exit_code,
            duration: start.elapsed(),
        }
    }

    /// Execute with resource limits (memory in bytes, cpu weight 1-10000).
    pub fn execute_with_limits(
        &mut self,
        image: &str,
        cmd: &[&str],
        memory_bytes: u64,
        cpu_weight: u64,
    ) -> ExecResult {
        self.ensure_image(image);
        let mem_str = memory_bytes.to_string();
        let cpu_str = cpu_weight.to_string();
        let start = std::time::Instant::now();
        let mut args = vec![
            "run", image,
            "--memory", &mem_str,
            "--cpu-weight", &cpu_str,
            "--",
        ];
        args.extend_from_slice(cmd);
        let (exit_code, stdout, stderr) = self.fixture.run_cli_with_exit_code(&args);
        ExecResult {
            stdout,
            stderr,
            exit_code,
            duration: start.elapsed(),
        }
    }
}
```

### DaemonFixture changes

`DaemonFixture` needs one new method: `run_cli_with_exit_code(&self, args) -> (i32, String, String)` that returns the raw exit code instead of a bool. The existing `run_cli` returns `(bool, String, String)` where the bool is `status.success()`. The new method returns `(status.code().unwrap_or(-1), stdout, stderr)`.

---

## 3. Test Scenarios

All tests are `#[test]`, `#[ignore]`, gated with `#[cfg(target_os = "linux")]`. They require root and network access (for image pulls). Run via `cargo xtask test-sandbox` or `just test-sandbox`.

Tests share a single `SandboxClient` per group via `lazy_static` or `OnceLock` to avoid re-starting the daemon and re-pulling images for every test. Image pulls happen once per daemon lifetime.

### Shell scenarios (alpine) — 10 tests

```rust
#[test] fn sandbox_stdout_captured()
```
`execute("alpine", &["echo", "hello"])` — assert `result.stdout.trim() == "hello"`, `result.exit_code == 0`.

```rust
#[test] fn sandbox_stderr_captured()
```
`execute("alpine", &["sh", "-c", "echo err >&2"])` — assert `result.stderr.contains("err")`.

```rust
#[test] fn sandbox_exit_code_zero()
```
`execute("alpine", &["true"])` — assert `result.exit_code == 0`.

```rust
#[test] fn sandbox_nonzero_exit_code()
```
`execute("alpine", &["sh", "-c", "exit 42"])` — assert `result.exit_code == 42`.

```rust
#[test] fn sandbox_large_output()
```
`execute("alpine", &["seq", "1", "10000"])` — assert stdout has 10000 lines.

```rust
#[test] fn sandbox_network_isolated()
```
`execute("alpine", &["sh", "-c", "wget -T 2 http://1.1.1.1/ 2>&1; echo done"])` — assert wget fails (exit code from wget is non-zero, or stderr contains "Network unreachable" / "Connection timed out"). The `echo done` ensures the container exits 0 even if wget fails, so we check stderr content rather than exit code.

Alternative approach: `execute("alpine", &["sh", "-c", "wget -T 2 http://1.1.1.1/"])` and assert `result.exit_code != 0`.

```rust
#[test] fn sandbox_filesystem_write_read()
```
`execute("alpine", &["sh", "-c", "echo data > /tmp/t && cat /tmp/t"])` — assert `result.stdout.trim() == "data"`.

```rust
#[test] fn sandbox_sequential_runs_isolated()
```
Run 1: `execute("alpine", &["sh", "-c", "echo secret > /tmp/state"])` — assert exit 0.
Run 2: `execute("alpine", &["sh", "-c", "cat /tmp/state"])` — assert exit_code != 0 (file not found). Each ephemeral container gets a fresh overlay.

```rust
#[test] fn sandbox_concurrent_runs_isolated()
```
Spawn two `execute` calls on separate threads, each writing a unique value to `/tmp/id`, then reading it back. Assert each container reads its own value, not the other's.

```rust
#[test] fn sandbox_oom_kill()
```
`execute_with_limits("alpine", &["sh", "-c", "head -c 64M /dev/zero | tail -c 1"], 16 * 1024 * 1024, 100)` — assert `result.exit_code != 0` (OOM-killed). The exact exit code depends on the signal (137 = SIGKILL).

### Python scenarios (python:3.12-alpine) — 5 tests

```rust
#[test] fn python_sandbox_basic_script()
```
`execute("python:3.12-alpine", &["python3", "-c", "print(1+1)"])` — assert `result.stdout.trim() == "2"`.

```rust
#[test] fn python_sandbox_exception_captured()
```
`execute("python:3.12-alpine", &["python3", "-c", "raise ValueError('oops')"])` — assert `result.stderr.contains("ValueError")`, `result.exit_code != 0`.

```rust
#[test] fn python_sandbox_json_output()
```
`execute("python:3.12-alpine", &["python3", "-c", "import json; print(json.dumps({'r':42}))"])` — parse stdout as JSON, assert `obj["r"] == 42`.

```rust
#[test] fn python_sandbox_multiline_output()
```
`execute("python:3.12-alpine", &["python3", "-c", "for i in range(5): print(i)"])` — assert 5 lines, `"0"` through `"4"`.

```rust
#[test] fn python_sandbox_network_blocked()
```
`execute("python:3.12-alpine", &["python3", "-c", "import urllib.request; urllib.request.urlopen('http://1.1.1.1')"])` — assert `result.exit_code != 0`, stderr contains a connection error.

---

## 4. Gating and Execution

### Prerequisites

- Linux with kernel 5.0+
- Root
- Network access (for Docker Hub image pulls on first run)
- cgroups v2

### xtask integration

New `test-sandbox` subcommand in `xtask/src/main.rs`, following the same pattern as `test-e2e-suite`:
1. Build workspace in release mode
2. Find test binary in `target/release/deps/`
3. Run via `sudo -E` with `MINIBOX_TEST_BIN_DIR`, `--test-threads=1`, `--ignored`

### Justfile recipe

```just
test-sandbox:
    cargo xtask test-sandbox
```

### Run sequence

```bash
just test-sandbox          # full sandbox suite (Linux+root, ~60s)
cargo xtask test-sandbox   # same, via xtask directly
```

---

## 5. Shared Fixture Strategy

To avoid re-pulling images for each of the 15 tests, a shared `SandboxClient` is initialized once per test binary execution:

```rust
use std::sync::OnceLock;
static SANDBOX: OnceLock<Mutex<SandboxClient>> = OnceLock::new();

fn sandbox() -> MutexGuard<'static, SandboxClient> {
    SANDBOX
        .get_or_init(|| Mutex::new(SandboxClient::start()))
        .lock()
        .expect("sandbox mutex poisoned")
}
```

Tests acquire the lock, run their scenario, and release. `--test-threads=1` ensures sequential execution (required by cgroup and daemon constraints), so the mutex is uncontended in practice but provides safety.

The concurrency test (`sandbox_concurrent_runs_isolated`) is the exception — it needs two containers running simultaneously. This test uses `sandbox().execute()` for both runs within a single test function, spawning one container in a background thread.

---

## Non-Goals

- No seccomp profile testing (minibox doesn't implement seccomp yet)
- No user namespace remapping tests (not supported yet)
- No persistent/stateful sandbox tests (minibox containers are ephemeral by design)
- No stdin pipe to container (minibox CLI doesn't support stdin forwarding yet)
- No image building — tests use pre-built Docker Hub images only
