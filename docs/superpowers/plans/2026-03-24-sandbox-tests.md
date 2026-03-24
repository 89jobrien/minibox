# Sandbox Tests Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 15 integration tests validating minibox's contract as an AI agent code-execution sandbox.

**Architecture:** Extract `DaemonFixture` from `e2e_tests.rs` into a shared `helpers/mod.rs` module, add a `SandboxClient` wrapper with agent-oriented API, write tests in a new `sandbox_tests.rs` file, and wire it into xtask/Justfile.

**Tech Stack:** Rust, tokio, tempfile, serde_json (for JSON output test), alpine + python:3.12-alpine images from Docker Hub

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/miniboxd/tests/helpers/mod.rs` | **Create** | `DaemonFixture` (moved), `SandboxClient`, `ExecResult`, `find_binary`, `extract_container_id` |
| `crates/miniboxd/tests/e2e_tests.rs` | **Modify** | Replace inline fixture with `mod helpers; use helpers::*;` |
| `crates/miniboxd/tests/sandbox_tests.rs` | **Create** | 15 sandbox scenario tests |
| `xtask/src/main.rs` | **Modify** | Add `test-sandbox` subcommand |
| `Justfile` | **Modify** | Add `test-sandbox` recipe |

---

### Task 1: Extract DaemonFixture to helpers/mod.rs

Move `DaemonFixture`, `find_binary()`, and `extract_container_id()` from `e2e_tests.rs` into a shared module. Add `run_cli_with_exit_code()` and make everything `pub`.

**Files:**
- Create: `crates/miniboxd/tests/helpers/mod.rs`
- Modify: `crates/miniboxd/tests/e2e_tests.rs`

- [ ] **Step 1: Create `helpers/mod.rs` with DaemonFixture**

Create `crates/miniboxd/tests/helpers/mod.rs`. Copy the following from `e2e_tests.rs`:
- `find_binary()` (lines 32-59) — make `pub`
- `DaemonFixture` struct (lines 66-73) — make `pub` struct with `pub` fields
- All `impl DaemonFixture` methods (lines 75-197) — make `pub`
- `impl Drop for DaemonFixture` (lines 200-254) — unchanged
- `extract_container_id()` (lines 678-694) — make `pub`

Add one new method to `impl DaemonFixture`:

```rust
    /// Run a CLI command and return (exit_code, stdout, stderr).
    /// Unlike `run_cli`, returns the raw exit code instead of a bool.
    pub fn run_cli_with_exit_code(&self, args: &[&str]) -> (i32, String, String) {
        let output = self
            .cli(args)
            .output()
            .unwrap_or_else(|e| panic!("failed to run minibox {:?}: {e}", args));

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.code().unwrap_or(-1), stdout, stderr)
    }
```

The file needs these imports at the top:

```rust
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;
```

**Note:** `DaemonFixture` also uses `libc` (for `kill`/`SIGTERM` in `Drop` and `sigterm()`) and `uuid` (for `Uuid::new_v4()` in `start()`). Both resolve as transitive deps of miniboxd today, but if compilation fails, add to miniboxd's `[dev-dependencies]`:
```toml
libc = { workspace = true }
uuid = { workspace = true }
```

- [ ] **Step 2: Update `e2e_tests.rs` to use helpers**

Replace the inline code with imports. At the top of `e2e_tests.rs`, after `#![cfg(target_os = "linux")]`:

```rust
mod helpers;
use helpers::{DaemonFixture, extract_container_id};
```

Delete from `e2e_tests.rs`:
- `find_binary()` (lines 32-59)
- `DaemonFixture` struct + all impls + Drop (lines 62-254)
- `extract_container_id()` (lines 675-694)

Keep the existing test functions unchanged — they use `DaemonFixture::start()` and `extract_container_id()` which are now imported.

- [ ] **Step 3: Verify existing e2e tests still compile**

Run: `cd /Users/joe/dev/minibox && cargo test -p miniboxd --test e2e_tests --no-run 2>&1`
Expected: compiles successfully (tests won't actually run without Linux+root)

- [ ] **Step 4: Commit**

```bash
git add crates/miniboxd/tests/helpers/mod.rs crates/miniboxd/tests/e2e_tests.rs
git commit -m "refactor(e2e): extract DaemonFixture to shared helpers module"
```

---

### Task 2: Add SandboxClient to helpers

Add `ExecResult` and `SandboxClient` to the shared helpers module.

**Files:**
- Modify: `crates/miniboxd/tests/helpers/mod.rs`

- [ ] **Step 1: Add ExecResult and SandboxClient**

Append to `crates/miniboxd/tests/helpers/mod.rs`:

```rust
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// SandboxClient — agent-oriented wrapper around DaemonFixture
// ---------------------------------------------------------------------------

/// Structured result from a sandbox execution.
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration: Duration,
}

/// Agent-oriented test client wrapping DaemonFixture.
///
/// Models how an AI agent would invoke minibox: pull an image, execute a
/// command, get back stdout/stderr/exit_code.  Images are cached per session
/// so repeated `execute()` calls don't re-pull.
pub struct SandboxClient {
    pub fixture: DaemonFixture,
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

    /// Split "image:tag" into (image, tag). If no colon, tag defaults to "latest".
    fn parse_image_tag(image: &str) -> (&str, &str) {
        match image.split_once(':') {
            Some((img, tag)) => (img, tag),
            None => (image, "latest"),
        }
    }

    /// Pull an image if not already cached in this session.
    pub fn ensure_image(&mut self, image: &str) {
        if self.pulled_images.contains(image) {
            return;
        }
        let (img, tag) = Self::parse_image_tag(image);
        let (ok, _stdout, stderr) = self.fixture.run_cli(&["pull", img, "--tag", tag]);
        assert!(ok, "failed to pull {image}: {stderr}");
        self.pulled_images.insert(image.to_string());
    }

    /// Execute a command in a fresh ephemeral container.
    ///
    /// Calls `minibox run <image> --tag <tag> -- <cmd...>` via the CLI.
    /// Accepts "image:tag" syntax (e.g., "python:3.12-alpine").
    pub fn execute(&mut self, image: &str, cmd: &[&str]) -> ExecResult {
        self.ensure_image(image);
        let (img, tag) = Self::parse_image_tag(image);
        let start = Instant::now();
        let mut args = vec!["run", img, "--tag", tag, "--"];
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
        let (img, tag) = Self::parse_image_tag(image);
        let mem_str = memory_bytes.to_string();
        let cpu_str = cpu_weight.to_string();
        let start = Instant::now();
        let mut args = vec![
            "run", img, "--tag", tag,
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

    /// Spawn a container without blocking. Returns the child process handle.
    ///
    /// Used by the concurrency test to run two containers simultaneously.
    pub fn spawn_container(&mut self, image: &str, cmd: &[&str]) -> Child {
        self.ensure_image(image);
        let (img, tag) = Self::parse_image_tag(image);
        let mut args = vec!["run", img, "--tag", tag, "--"];
        args.extend_from_slice(cmd);
        self.fixture.cli(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn container")
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd /Users/joe/dev/minibox && cargo test -p miniboxd --test e2e_tests --no-run 2>&1`
Expected: compiles (SandboxClient isn't used yet, but should have no errors)

- [ ] **Step 3: Commit**

```bash
git add crates/miniboxd/tests/helpers/mod.rs
git commit -m "feat(test): add SandboxClient agent-oriented test wrapper"
```

---

### Task 3: Create sandbox_tests.rs with shell scenarios (tests 1-6)

First 6 shell tests: stdout, stderr, exit codes, large output, network isolation, filesystem.

**Files:**
- Create: `crates/miniboxd/tests/sandbox_tests.rs`

- [ ] **Step 1: Create sandbox_tests.rs with shared fixture and first 6 tests**

Create `crates/miniboxd/tests/sandbox_tests.rs`:

```rust
//! Sandbox contract tests: validates minibox as an AI agent code-execution sandbox.
//!
//! Tests exercise real images (alpine, python:3.12-alpine) in real containers,
//! verifying output capture, exit codes, network isolation, filesystem containment,
//! resource limits, and concurrency isolation.
//!
//! **Requirements:** Linux, root, cgroups v2, network access (Docker Hub)
//!
//! **Running:**
//! ```bash
//! just test-sandbox
//! ```

#![cfg(target_os = "linux")]

mod helpers;
use helpers::SandboxClient;

use linuxbox::preflight;
use linuxbox::require_capability;
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Shared fixture — one daemon + cached images for all tests
// ---------------------------------------------------------------------------

static SANDBOX: OnceLock<Mutex<SandboxClient>> = OnceLock::new();

fn sandbox() -> std::sync::MutexGuard<'static, SandboxClient> {
    SANDBOX
        .get_or_init(|| Mutex::new(SandboxClient::start()))
        .lock()
        .unwrap_or_else(|e| e.into_inner()) // recover from poison
}

/// Gate: skip all sandbox tests unless root + cgroups v2.
fn require_sandbox_caps() {
    let caps = preflight::probe();
    require_capability!(caps, is_root, "requires root");
    require_capability!(caps, cgroups_v2, "requires cgroups v2");
}

// ---------------------------------------------------------------------------
// Shell scenarios (alpine)
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn sandbox_stdout_captured() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["echo", "hello"]);
    assert_eq!(result.exit_code, 0, "echo should exit 0");
    assert_eq!(result.stdout.trim(), "hello", "stdout should capture echo output");
}

#[test]
#[ignore]
fn sandbox_stderr_captured() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["sh", "-c", "echo err >&2"]);
    assert!(
        result.stderr.contains("err"),
        "stderr should contain 'err', got: {:?}",
        result.stderr
    );
}

#[test]
#[ignore]
fn sandbox_exit_code_zero() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["true"]);
    assert_eq!(result.exit_code, 0, "/bin/true should exit 0");
}

#[test]
#[ignore]
fn sandbox_nonzero_exit_code() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["sh", "-c", "exit 42"]);
    assert_eq!(result.exit_code, 42, "exit 42 should propagate as exit code 42");
}

#[test]
#[ignore]
fn sandbox_large_output() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["seq", "1", "10000"]);
    assert_eq!(result.exit_code, 0, "seq should exit 0");
    let line_count = result.stdout.lines().count();
    assert_eq!(
        line_count, 10000,
        "should capture all 10000 lines, got {line_count}"
    );
}

#[test]
#[ignore]
fn sandbox_network_isolated() {
    require_sandbox_caps();
    let mut sb = sandbox();
    // wget should fail — container has NetworkMode::None (isolated namespace, no interfaces)
    let result = sb.execute("alpine", &["sh", "-c", "wget -T 2 http://1.1.1.1/ 2>&1; exit 0"]);
    // Container exits 0 (we forced it), but wget output should show a network error
    assert!(
        result.stdout.contains("Network unreachable")
            || result.stdout.contains("Connection timed out")
            || result.stdout.contains("bad address")
            || result.stderr.contains("Network unreachable")
            || result.stderr.contains("bad address"),
        "wget should fail with a network error.\nstdout: {:?}\nstderr: {:?}",
        result.stdout,
        result.stderr
    );
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd /Users/joe/dev/minibox && cargo test -p miniboxd --test sandbox_tests --no-run 2>&1`
Expected: compiles successfully

- [ ] **Step 3: Commit**

```bash
git add crates/miniboxd/tests/sandbox_tests.rs
git commit -m "test(sandbox): add shell scenarios — stdout, stderr, exit codes, network isolation"
```

---

### Task 4: Add shell scenarios 7-10

Filesystem, sequential isolation, concurrent isolation, OOM kill.

**Files:**
- Modify: `crates/miniboxd/tests/sandbox_tests.rs`

- [ ] **Step 1: Append tests 7-10**

Append to `sandbox_tests.rs` (before the closing of the file):

```rust
#[test]
#[ignore]
fn sandbox_filesystem_write_read() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute("alpine", &["sh", "-c", "echo data > /tmp/t && cat /tmp/t"]);
    assert_eq!(result.exit_code, 0, "write+read should succeed");
    assert_eq!(result.stdout.trim(), "data", "should read back what was written");
}

#[test]
#[ignore]
fn sandbox_sequential_runs_isolated() {
    require_sandbox_caps();
    let mut sb = sandbox();

    // Run 1: write a file
    let r1 = sb.execute("alpine", &["sh", "-c", "echo secret > /tmp/state && echo ok"]);
    assert_eq!(r1.exit_code, 0, "first run should succeed");
    assert_eq!(r1.stdout.trim(), "ok");

    // Run 2: try to read that file — should fail (fresh overlay)
    let r2 = sb.execute("alpine", &["sh", "-c", "cat /tmp/state"]);
    assert_ne!(
        r2.exit_code, 0,
        "second run should fail: /tmp/state should not exist in a fresh container"
    );
}

#[test]
#[ignore]
fn sandbox_concurrent_runs_isolated() {
    require_sandbox_caps();
    let mut sb = sandbox();

    // Spawn two containers that each write a unique value, sleep, then read it back
    let mut child_a = sb.spawn_container(
        "alpine",
        &["sh", "-c", "echo AAA > /tmp/id && sleep 1 && cat /tmp/id"],
    );
    let mut child_b = sb.spawn_container(
        "alpine",
        &["sh", "-c", "echo BBB > /tmp/id && sleep 1 && cat /tmp/id"],
    );

    // Wait for both and capture output
    let out_a = child_a.wait_with_output().expect("child_a wait failed");
    let out_b = child_b.wait_with_output().expect("child_b wait failed");

    let stdout_a = String::from_utf8_lossy(&out_a.stdout);
    let stdout_b = String::from_utf8_lossy(&out_b.stdout);

    // Each container should read its own value, not the other's
    assert!(
        stdout_a.contains("AAA"),
        "container A should read AAA, got: {stdout_a}"
    );
    assert!(
        stdout_b.contains("BBB"),
        "container B should read BBB, got: {stdout_b}"
    );
    assert!(
        !stdout_a.contains("BBB"),
        "container A should NOT see BBB"
    );
    assert!(
        !stdout_b.contains("AAA"),
        "container B should NOT see AAA"
    );
}

#[test]
#[ignore]
fn sandbox_oom_kill() {
    require_sandbox_caps();
    let mut sb = sandbox();

    // 16 MB memory limit, try to allocate 64 MB via /dev/zero
    let result = sb.execute_with_limits(
        "alpine",
        &["sh", "-c", "dd if=/dev/zero of=/dev/shm/fill bs=1M count=64 2>&1; echo done"],
        16 * 1024 * 1024,
        100,
    );

    // The process should either be OOM-killed (exit != 0) or dd should fail
    // We check both: non-zero exit AND that dd didn't fully succeed
    let succeeded_fully = result.stdout.contains("64+0 records out");
    assert!(
        result.exit_code != 0 || !succeeded_fully,
        "container should be OOM-killed or dd should fail with 16 MB limit.\n\
         exit_code: {}\nstdout: {:?}\nstderr: {:?}",
        result.exit_code, result.stdout, result.stderr
    );
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd /Users/joe/dev/minibox && cargo test -p miniboxd --test sandbox_tests --no-run 2>&1`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add crates/miniboxd/tests/sandbox_tests.rs
git commit -m "test(sandbox): add filesystem, isolation, concurrent, and OOM scenarios"
```

---

### Task 5: Add Python scenarios (tests 11-15)

Five Python-specific tests using `python:3.12-alpine`.

**Files:**
- Modify: `crates/miniboxd/tests/sandbox_tests.rs`

- [ ] **Step 1: Append Python tests**

Append to `sandbox_tests.rs`:

```rust
// ---------------------------------------------------------------------------
// Python scenarios (python:3.12-alpine)
// ---------------------------------------------------------------------------

const PYTHON_IMAGE: &str = "python:3.12-alpine";

#[test]
#[ignore]
fn python_sandbox_basic_script() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute(PYTHON_IMAGE, &["python3", "-c", "print(1+1)"]);
    assert_eq!(result.exit_code, 0, "python should exit 0");
    assert_eq!(result.stdout.trim(), "2", "print(1+1) should output '2'");
}

#[test]
#[ignore]
fn python_sandbox_exception_captured() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute(PYTHON_IMAGE, &["python3", "-c", "raise ValueError('oops')"]);
    assert_ne!(result.exit_code, 0, "exception should produce non-zero exit");
    assert!(
        result.stderr.contains("ValueError"),
        "stderr should contain 'ValueError', got: {:?}",
        result.stderr
    );
}

#[test]
#[ignore]
fn python_sandbox_json_output() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute(
        PYTHON_IMAGE,
        &["python3", "-c", "import json; print(json.dumps({'r': 42}))"],
    );
    assert_eq!(result.exit_code, 0, "json output should exit 0");

    let parsed: serde_json::Value =
        serde_json::from_str(result.stdout.trim()).unwrap_or_else(|e| {
            panic!(
                "stdout should be valid JSON: {e}\nstdout: {:?}",
                result.stdout
            )
        });
    assert_eq!(parsed["r"], 42, "JSON should contain r=42, got: {parsed}");
}

#[test]
#[ignore]
fn python_sandbox_multiline_output() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute(
        PYTHON_IMAGE,
        &["python3", "-c", "for i in range(5): print(i)"],
    );
    assert_eq!(result.exit_code, 0, "multiline script should exit 0");
    let lines: Vec<&str> = result.stdout.trim().lines().collect();
    assert_eq!(lines, vec!["0", "1", "2", "3", "4"], "should print 0-4");
}

#[test]
#[ignore]
fn python_sandbox_network_blocked() {
    require_sandbox_caps();
    let mut sb = sandbox();
    let result = sb.execute(
        PYTHON_IMAGE,
        &[
            "python3",
            "-c",
            "import urllib.request; urllib.request.urlopen('http://1.1.1.1', timeout=2)",
        ],
    );
    assert_ne!(
        result.exit_code, 0,
        "network access should fail in isolated container"
    );
    // Python should raise OSError or URLError
    assert!(
        result.stderr.contains("Error") || result.stderr.contains("error"),
        "stderr should mention an error.\nstderr: {:?}",
        result.stderr
    );
}
```

- [ ] **Step 2: Add serde_json dev-dependency**

Check if `serde_json` is already a dependency of miniboxd. If not, add it to `crates/miniboxd/Cargo.toml` under `[dev-dependencies]`:

```toml
[dev-dependencies]
serde_json = "1"
```

(It may already be present — check first before adding.)

- [ ] **Step 3: Verify it compiles**

Run: `cd /Users/joe/dev/minibox && cargo test -p miniboxd --test sandbox_tests --no-run 2>&1`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add crates/miniboxd/tests/sandbox_tests.rs crates/miniboxd/Cargo.toml
git commit -m "test(sandbox): add Python scenarios — script execution, JSON, exceptions, network"
```

---

### Task 6: Wire into xtask and Justfile

Add `test-sandbox` subcommand to xtask and a Just recipe.

**Files:**
- Modify: `xtask/src/main.rs`
- Modify: `Justfile`

- [ ] **Step 1: Add test-sandbox to xtask**

In `xtask/src/main.rs`, add `"test-sandbox"` to the match arm (near `"test-e2e-suite"`):

```rust
        Some("test-sandbox") => test_sandbox(&sh),
```

Then add the function (modeled exactly on `test_e2e_suite`):

```rust
/// Sandbox contract tests (Linux, root, Docker Hub required)
fn test_sandbox(sh: &Shell) -> Result<()> {
    cmd!(sh, "cargo build --release")
        .run()
        .context("build failed")?;

    cmd!(
        sh,
        "cargo test -p miniboxd --test sandbox_tests --release --no-run"
    )
    .run()
    .context("failed to build sandbox test binary")?;

    let binary = find_test_binary("target/release/deps", "sandbox_tests")
        .context("could not locate sandbox test binary in target/release/deps")?;

    let bin_dir = env::current_dir()?.join("target/release");
    cmd!(
        sh,
        "sudo -E env MINIBOX_TEST_BIN_DIR={bin_dir} {binary} --test-threads=1 --ignored --nocapture"
    )
    .run()
    .context("sandbox tests failed")?;
    Ok(())
}
```

- [ ] **Step 2: Add test-sandbox to Justfile**

Add after the `test-e2e-suite` recipe (around line 74):

```just
# Sandbox contract tests (Linux, root, Docker Hub)
test-sandbox:
    cargo xtask test-sandbox
```

- [ ] **Step 3: Verify xtask compiles**

Run: `cd /Users/joe/dev/minibox && cargo build -p xtask 2>&1`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add xtask/src/main.rs Justfile
git commit -m "ci(xtask): add test-sandbox subcommand and Just recipe"
```

---

### Task 7: Final verification

Run the full unit test suite and clippy to ensure nothing is broken.

**Files:** None (verification only)

- [ ] **Step 1: Run unit tests**

Run: `cd /Users/joe/dev/minibox && cargo xtask test-unit 2>&1`
Expected: all existing tests pass (sandbox tests won't run — they're `#[ignore]` and need Linux+root)

- [ ] **Step 2: Run clippy**

Run: `cd /Users/joe/dev/minibox && cargo clippy -p linuxbox -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -p minibox-llm -p minibox-secrets -- -D warnings 2>&1`
Expected: no warnings

- [ ] **Step 3: Verify sandbox_tests compiles**

Run: `cd /Users/joe/dev/minibox && cargo test -p miniboxd --test sandbox_tests --no-run 2>&1`
Expected: compiles (binary created but tests not executed)

---

## Execution Order

Tasks must be executed sequentially:
1. **Task 1** — extract DaemonFixture (prerequisite for everything)
2. **Task 2** — add SandboxClient (prerequisite for tests)
3. **Task 3** — shell tests 1-6
4. **Task 4** — shell tests 7-10
5. **Task 5** — Python tests 11-15
6. **Task 6** — xtask + Justfile wiring
7. **Task 7** — final verification

## Notes for implementers

- **Tests are `#[ignore]`** — they won't run in normal `cargo test`. They require `cargo xtask test-sandbox` with Linux+root+network.
- **`OnceLock` shared fixture** — all tests share one daemon instance. The `unwrap_or_else(|e| e.into_inner())` pattern on the mutex recovers from poison if a test panics.
- **`cli()` method already exists** on `DaemonFixture` at line 155 of `e2e_tests.rs`. It returns a pre-configured `Command`. No need to add it.
- **Image tag parsing** — `SandboxClient::parse_image_tag("python:3.12-alpine")` splits into `("python", "3.12-alpine")`. The CLI's `--tag` flag is used explicitly.
- **OOM test** — uses `/dev/shm` (tmpfs) write instead of raw memory allocation for reliability. The assertion checks that `dd` did NOT succeed fully, rather than asserting a specific exit code.
- **Concurrency test** — uses `spawn_container()` which returns `Child`. Both containers run simultaneously; `wait_with_output()` collects results after both are spawned.
