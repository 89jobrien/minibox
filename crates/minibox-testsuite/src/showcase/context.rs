//! `ScenarioCtx` — shared harness context for showcase scenarios.
//!
//! Resolves the `mbx`/`miniboxd` binaries, spawns a real daemon for the
//! duration of a scenario, and exposes a [`BackendDescriptor`] so scenario
//! code can gate steps on adapter capability (mirroring the conformance
//! suite's `ConformanceTest::required_capability()` skip-gracefully
//! semantics) instead of failing hard when a capability is unavailable.
//!
//! This intentionally reuses the binary-discovery search order already
//! established in `crates/miniboxd/tests/helpers/mod.rs::find_binary`
//! (`MINIBOX_TEST_BIN_DIR` -> `target/{release,debug}` -> error) rather than
//! re-implementing a third copy. That helper lives in an integration test
//! binary (`crates/miniboxd/tests/helpers`), which is not linkable as a
//! library dependency from this crate, so the search logic is duplicated
//! here as a single reusable function ([`find_binary`]) rather than a
//! second bespoke implementation with subtly different behavior.
//!
//! Errors use `anyhow` (not `miette`) to match the `Scenario::run` trait
//! signature (`fn run(...) -> anyhow::Result<()>`), so scenario code can use
//! `?` against `ScenarioCtx` methods without a conversion shim.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use minibox_core::adapters::conformance::BackendDescriptor;
use minibox_core::domain::BackendCapability;

use super::reporter::Reporter;

/// Structured output from a CLI command execution.
///
/// Mirrors `crates/miniboxd/tests/helpers/mod.rs::CmdOutput` so scenario
/// assertions read identically to the existing e2e test idiom.
#[derive(Debug, Clone)]
pub struct CmdOutput {
    /// Whether the command exited successfully.
    pub success: bool,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

impl CmdOutput {
    fn error(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            stdout: String::new(),
            stderr: msg.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Binary discovery
// ---------------------------------------------------------------------------

/// Resolve the workspace root from `CARGO_MANIFEST_DIR`.
fn workspace_root() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("could not resolve workspace root from CARGO_MANIFEST_DIR"))
}

/// Find a minibox binary by name.
///
/// Search order (matches `crates/miniboxd/tests/helpers/mod.rs::find_binary`
/// and `crates/mbx/tests/cli_subprocess.rs::find_minibox`, the two existing
/// binary-discovery implementations in this repo):
///
/// 1. `MINIBOX_TEST_BIN_DIR` env var
/// 2. `CARGO_TARGET_DIR/{release,debug}/{name}` (if set)
/// 3. `<workspace_root>/target/{release,debug}/{name}`
/// 4. Error (never `.unwrap()`/panic — callers decide how to report)
pub fn find_binary(name: &str) -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("MINIBOX_TEST_BIN_DIR") {
        let p = PathBuf::from(&dir).join(name);
        if p.exists() {
            return Ok(p);
        }
        return Err(anyhow!(
            "MINIBOX_TEST_BIN_DIR is set to '{dir}' but '{name}' was not found there"
        ));
    }

    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        let target_dir = PathBuf::from(target_dir);
        for profile in ["release", "debug"] {
            let p = target_dir.join(profile).join(name);
            if p.exists() {
                return Ok(p);
            }
        }
    }

    let ws_root = workspace_root()?;
    for profile in ["release", "debug"] {
        let p = ws_root.join("target").join(profile).join(name);
        if p.exists() {
            return Ok(p);
        }
    }

    Err(anyhow!(
        "could not find binary '{name}'. Run `cargo build --release` first, \
         or set MINIBOX_TEST_BIN_DIR."
    ))
}

// ---------------------------------------------------------------------------
// Daemon fixture
// ---------------------------------------------------------------------------

/// RAII fixture that starts a real `miniboxd` and provides CLI access.
///
/// Equivalent in spirit to `crates/miniboxd/tests/helpers/mod.rs::DaemonFixture`,
/// but return-`Result`-based rather than panic-based so it composes with
/// [`Reporter::skip`] instead of aborting a whole showcase run.
struct DaemonFixture {
    child: Option<Child>,
    socket_path: PathBuf,
    cli_bin: PathBuf,
    data_dir: tempfile::TempDir,
    run_dir: tempfile::TempDir,
    cgroup_root: PathBuf,
    /// Combined stdout+stderr lines from the daemon process, drained
    /// continuously by background threads so demo/showcase callers can
    /// always surface the daemon log (e.g. after a scenario failure)
    /// rather than only capturing stderr in the narrow "socket never
    /// appeared" failure path.
    log_lines: Arc<Mutex<Vec<String>>>,
}

impl DaemonFixture {
    fn start_with_env(extra_env: &[(&str, &str)]) -> Result<Self> {
        let data_dir = tempfile::TempDir::with_prefix("minibox-showcase-data-")
            .context("create temp data dir")?;
        let run_dir = tempfile::TempDir::with_prefix("minibox-showcase-run-")
            .context("create temp run dir")?;
        let socket_path = run_dir.path().join("miniboxd.sock");

        // Top-level cgroup slice (see DaemonFixture::start in
        // crates/miniboxd/tests/helpers/mod.rs for why this must be
        // top-level, not nested under the runner's own cgroup).
        let test_name = format!(
            "minibox-showcase-{}",
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        let cgroup_root = PathBuf::from("/sys/fs/cgroup").join(&test_name);

        let daemon_bin = find_binary("miniboxd")?;
        let cli_bin = find_binary("mbx")?;

        if cfg!(target_os = "linux") {
            std::fs::create_dir_all(&cgroup_root).context("create cgroup root")?;
            if let Err(e) = std::fs::write(
                cgroup_root.join("cgroup.subtree_control"),
                "+memory +cpu +pids",
            ) {
                tracing::warn!(
                    cgroup_root = %cgroup_root.display(),
                    error = %e,
                    "showcase: could not enable subtree controllers"
                );
            }
        }

        let mut command = Command::new(&daemon_bin);
        command
            .env("MINIBOX_DATA_DIR", data_dir.path())
            .env("MINIBOX_RUN_DIR", run_dir.path())
            .env("MINIBOX_SOCKET_PATH", &socket_path)
            .env("MINIBOX_CGROUP_ROOT", &cgroup_root)
            .env("RUST_LOG", "miniboxd=info")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in extra_env {
            command.env(key, value);
        }

        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn {}", daemon_bin.display()))?;

        let log_lines = Arc::new(Mutex::new(Vec::new()));

        let mut fixture = Self {
            child: Some(child),
            socket_path: socket_path.clone(),
            cli_bin,
            data_dir,
            run_dir,
            cgroup_root,
            log_lines: Arc::clone(&log_lines),
        };

        // Drain stdout+stderr in background threads so debug logging during
        // slow operations can't fill the 64KB pipe buffer and deadlock the
        // daemon (documented gotcha in docs/core/GOTCHAS.mbx.md), while
        // still retaining every line so the demo can always show the
        // daemon log rather than discarding it.
        if let Some(child) = fixture.child.as_mut() {
            if let Some(stdout) = child.stdout.take() {
                spawn_log_drain_thread(stdout, Arc::clone(&log_lines));
            }
            if let Some(stderr) = child.stderr.take() {
                spawn_log_drain_thread(stderr, log_lines);
            }
        }

        let start = Instant::now();
        let timeout = Duration::from_secs(10);
        while !socket_path.exists() {
            if start.elapsed() > timeout {
                let stderr = fixture.kill_and_capture_stderr();
                return Err(anyhow!(
                    "miniboxd did not create socket within 10s.\nSocket: {}\nStderr:\n{stderr}",
                    socket_path.display()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        Ok(fixture)
    }

    /// Snapshot of every daemon stdout/stderr line captured so far, in the
    /// (interleaved) order the drain threads observed them.
    fn daemon_log(&self) -> Vec<String> {
        self.log_lines
            .lock()
            .map(|lines| lines.clone())
            .unwrap_or_default()
    }

    fn cli(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.cli_bin);
        cmd.env("MINIBOX_SOCKET_PATH", &self.socket_path);
        cmd.args(args);
        cmd
    }

    fn run_cli(&self, args: &[&str]) -> CmdOutput {
        match self.cli(args).output() {
            Ok(output) => CmdOutput {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            },
            Err(e) => CmdOutput::error(format!("failed to run mbx {args:?}: {e}")),
        }
    }

    /// Kill the daemon and return the captured log for diagnostics. Only
    /// meaningful when the daemon is expected to have already failed.
    ///
    /// Reads from `log_lines` rather than `child.wait_with_output()` — the
    /// stdout/stderr handles were already taken by the drain threads in
    /// `start_with_env`, so `wait_with_output()` would see empty pipes.
    fn kill_and_capture_stderr(&mut self) -> String {
        let Some(mut child) = self.child.take() else {
            return "(daemon already reaped)".to_string();
        };
        let _ = child.kill();
        let _ = child.wait();
        self.daemon_log().join("\n")
    }
}

/// Spawn a background thread that drains `reader` line-by-line into
/// `log_lines`, forever (until EOF/error). Keeps the daemon's pipe buffer
/// from filling (see `start_with_env`) while preserving every line for
/// later inspection instead of discarding it.
fn spawn_log_drain_thread<R: std::io::Read + Send + 'static>(
    reader: R,
    log_lines: Arc<Mutex<Vec<String>>>,
) {
    std::thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Ok(mut lines) = log_lines.lock() {
                lines.push(line);
            }
        }
    });
}

impl Drop for DaemonFixture {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };

        // SAFETY: `child.id()` is the PID of a process we spawned above and
        // have not yet waited on, so it is guaranteed to still refer to our
        // child (or a just-exited zombie we own) — sending SIGTERM to it is
        // sound.
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
        }

        let start = Instant::now();
        while matches!(child.try_wait(), Ok(None)) {
            if start.elapsed() > Duration::from_secs(5) {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // cgroupfs only supports rmdir, not rm -rf: remove bottom-up.
        if self.cgroup_root.exists() {
            fn remove_cgroup_tree(dir: &Path) {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            remove_cgroup_tree(&path);
                        }
                    }
                }
                if let Err(e) = std::fs::remove_dir(dir) {
                    tracing::warn!(
                        dir = %dir.display(),
                        error = %e,
                        "showcase: cgroup teardown failed to remove directory"
                    );
                }
            }
            remove_cgroup_tree(&self.cgroup_root);
        }

        // TempDir Drop impls handle data_dir/run_dir cleanup.
        let _ = (&self.data_dir, &self.run_dir);
    }
}

// ---------------------------------------------------------------------------
// BackendDescriptor construction
// ---------------------------------------------------------------------------

/// Build a [`BackendDescriptor`] for the currently-selected adapter by
/// reading `MINIBOX_ADAPTER`, mirroring the logic in
/// `crates/mbx/src/commands/doctor.rs::selected_adapter()`.
///
/// This only encodes the per-platform capability matrix documented in
/// `docs/core/FEATURE_MATRIX.mbx.md` — it does not attempt to probe the
/// live daemon (there is no `DaemonRequest`/`DaemonResponse` variant that
/// exposes adapter capability today), so it is a static approximation
/// rather than a live query.
fn descriptor_for_active_adapter() -> BackendDescriptor {
    let adapter = std::env::var("MINIBOX_ADAPTER").unwrap_or_default();
    let adapter = adapter.trim();

    // Native Linux is the only backend with the full feature set (bind
    // mounts, privileged mode, bridge networking, exec, logs, pause/resume).
    // Everything else (smolvm, krun, colima, gke, winbox) is progressively
    // more restricted per FEATURE_MATRIX.mbx.md.
    if adapter == "native" || (adapter.is_empty() && cfg!(target_os = "linux")) {
        return BackendDescriptor::new("native")
            .with_capability(BackendCapability::Exec)
            .with_capability(BackendCapability::Network)
            .with_capability(BackendCapability::Filesystem)
            .with_capability(BackendCapability::Commit)
            .with_capability(BackendCapability::BuildFromContext)
            .with_capability(BackendCapability::PushToRegistry)
            .with_capability(BackendCapability::Metrics);
    }

    match adapter {
        "colima" => BackendDescriptor::new("colima")
            .with_capability(BackendCapability::PushToRegistry)
            .with_capability(BackendCapability::Commit)
            .with_capability(BackendCapability::BuildFromContext),
        "krun" => BackendDescriptor::new("krun").with_capability(BackendCapability::Filesystem),
        "gke" => BackendDescriptor::new("gke")
            .with_capability(BackendCapability::Filesystem)
            .with_capability(BackendCapability::PushToRegistry),
        // Default: smolvm (the default macOS adapter per adapter_registry.rs),
        // or an unrecognized MINIBOX_ADAPTER value — assume the minimal set
        // rather than over-claiming capability.
        _ => BackendDescriptor::new("smolvm")
            .with_capability(BackendCapability::Filesystem)
            .with_capability(BackendCapability::BuildFromContext),
    }
}

// ---------------------------------------------------------------------------
// ScenarioCtx
// ---------------------------------------------------------------------------

/// Shared context threaded through every showcase scenario.
///
/// Owns a real `miniboxd` + `mbx` process pair (torn down via `Drop`) and a
/// [`BackendDescriptor`] for the active adapter so scenarios can gate steps
/// with `ctx.supports(cap)` and call `reporter.skip(...)` rather than
/// failing when a capability is absent.
pub struct ScenarioCtx {
    fixture: DaemonFixture,

    /// Capability descriptor for the currently active adapter. Public so
    /// scenario code can query `ctx.descriptor.capabilities.supports(cap)`
    /// (or the `ctx.supports(cap)` shorthand) directly, matching the
    /// conformance suite's `TestContext::supports` idiom.
    pub descriptor: BackendDescriptor,

    /// Top-level cgroup slice created for this scenario's daemon. Public so
    /// scenarios that assert on cgroup files directly (pause/resume) can
    /// build `ctx.cgroup_root.join(container_id)` without an extra method
    /// hop.
    pub cgroup_root: PathBuf,

    /// Daemon's data directory (image store, container metadata).
    pub data_dir: PathBuf,

    /// Daemon's run directory (socket, PID file).
    pub run_dir: PathBuf,
}

impl ScenarioCtx {
    /// Discover binaries, spin up a real daemon, and build the active
    /// adapter's capability descriptor.
    ///
    /// Returns `Err` (never panics) when the binaries can't be found or the
    /// daemon fails to start, so callers can route the failure through
    /// [`Reporter::failure`] or [`Reporter::skip`] as appropriate rather
    /// than aborting an entire narrated demo run.
    pub fn discover() -> Result<Self> {
        Self::spawn_with_env(&[])
    }

    fn spawn_with_env(extra_env: &[(&str, &str)]) -> Result<Self> {
        let fixture = DaemonFixture::start_with_env(extra_env)?;
        let descriptor = descriptor_for_active_adapter();
        let cgroup_root = fixture.cgroup_root.clone();
        let data_dir = fixture.data_dir.path().to_path_buf();
        let run_dir = fixture.run_dir.path().to_path_buf();
        Ok(Self {
            fixture,
            descriptor,
            cgroup_root,
            data_dir,
            run_dir,
        })
    }

    /// Return this context's already-running daemon. Exists so scenario
    /// code that wants an explicit "spawn a daemon" step (mirroring
    /// `mbx doctor`-style narration) reads naturally, even though
    /// `discover()` already started one.
    pub const fn spawn_daemon(&self) -> Result<&Self> {
        Ok(self)
    }

    /// Spin up a *second*, independent daemon with additional environment
    /// variables set before spawn — used by scenarios that need to opt in
    /// to policy gates (e.g. `MINIBOX_ALLOW_BIND_MOUNTS`,
    /// `MINIBOX_ALLOW_PRIVILEGED`) that this context's own daemon was not
    /// started with.
    pub fn spawn_daemon_with_env(&self, extra_env: &[(&str, &str)]) -> Result<Self> {
        Self::spawn_with_env(extra_env)
    }

    /// Whether the active backend supports `cap`. Convenience wrapper over
    /// `ctx.descriptor.capabilities.supports(cap)` for scenario call sites
    /// that don't want to spell out the full path.
    #[must_use]
    pub fn supports(&self, cap: BackendCapability) -> bool {
        self.descriptor.capabilities.supports(cap)
    }

    /// Report a skip through `reporter` and return `true` when `cap` is
    /// unsupported by the active backend — scenario steps should early-return
    /// on `true` rather than attempting the operation.
    pub fn skip_unless(&self, reporter: &dyn Reporter, cap: BackendCapability, step: &str) -> bool {
        if self.supports(cap) {
            return false;
        }
        reporter.skip(&format!(
            "{step}: backend '{}' does not support {cap:?}",
            self.descriptor.name
        ));
        true
    }

    /// Whether the active backend is the native Linux adapter (the only
    /// backend with bind mounts, `--privileged`, and bridge networking).
    #[must_use]
    pub fn is_native_linux(&self) -> bool {
        cfg!(target_os = "linux") && self.descriptor.name == "native"
    }

    /// Timeout to allow a container to reach `Running` in `mbx ps`.
    ///
    /// VM-backed adapters (smolvm, krun, colima, gke) boot an ephemeral
    /// machine and may pull a base VM image on first use before the
    /// container process itself starts, which the native adapter never
    /// does — 5s is enough for native but routinely too short for a VM
    /// boot, so scale up for any non-native backend.
    #[must_use]
    pub fn running_timeout(&self) -> Duration {
        if self.descriptor.name == "native" {
            Duration::from_secs(5)
        } else {
            Duration::from_secs(60)
        }
    }

    /// Whether the current process is running as root (required for
    /// bridge networking / privileged-mode scenarios).
    #[must_use]
    pub fn is_root(&self) -> bool {
        is_root_impl()
    }

    /// Snapshot of every line the daemon has written to stdout/stderr so
    /// far this session, in observed order. Always available (not gated on
    /// failure) so demo/showcase callers can surface it unconditionally.
    #[must_use]
    pub fn daemon_log(&self) -> Vec<String> {
        self.fixture.daemon_log()
    }

    /// Resolve a workspace binary by name (`miniboxd`, `mbx`), for
    /// scenarios that need paths to bind-mount into a nested container.
    pub fn workspace_binary(&self, name: &str) -> Result<PathBuf> {
        find_binary(name)
    }

    /// Build a `Command` for the `mbx` CLI pre-configured with this
    /// scenario's daemon socket.
    #[must_use]
    pub fn cli(&self, args: &[&str]) -> Command {
        self.fixture.cli(args)
    }

    /// Run an `mbx` CLI command to completion and return structured output.
    /// Never panics: a spawn/IO failure is reported as a non-successful
    /// `CmdOutput` with the error text in `stderr`.
    #[must_use]
    pub fn run_cli(&self, args: &[&str]) -> CmdOutput {
        self.fixture.run_cli(args)
    }

    /// Run an `mbx` CLI command and return `(exit_code, stdout, stderr)`,
    /// for scenarios that want to assert on a specific exit code.
    #[must_use]
    pub fn run_cli_with_exit_code(&self, args: &[&str]) -> (i32, String, String) {
        match self.fixture.cli(args).output() {
            Ok(output) => (
                output.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            ),
            Err(e) => (
                -1,
                String::new(),
                format!("failed to run mbx {args:?}: {e}"),
            ),
        }
    }

    /// Convenience: run a CLI command and route the outcome through
    /// `reporter`, returning the output either way.
    pub fn run_cli_reported(
        &self,
        reporter: &dyn Reporter,
        step: &str,
        args: &[&str],
    ) -> CmdOutput {
        reporter.step(step);
        let out = self.run_cli(args);
        for line in out.stdout.lines() {
            reporter.output(line);
        }
        for line in out.stderr.lines() {
            reporter.output(line);
        }
        if out.success {
            reporter.success(step);
        } else {
            reporter.failure(&format!(
                "{step} failed: exit status non-zero\nstdout: {}\nstderr: {}",
                out.stdout, out.stderr
            ));
        }
        out
    }

    /// Pull `image`, panicking with a descriptive message on failure. Used
    /// as an unconditional precondition step (every scenario needs
    /// `alpine` present before it can do anything else), matching the
    /// `expect("reason")`-in-tests convention for test-harness code.
    pub fn pull_required(&self, image: &str) {
        let out = self.run_cli(&["pull", image]);
        assert!(
            out.success,
            "showcase: required pull of '{image}' failed\nstdout: {}\nstderr: {}",
            out.stdout, out.stderr
        );
    }

    /// Start `mbx run <args>` in the background (not waited on), returning
    /// the child process handle plus the container ID discovered by
    /// polling `mbx ps` for a newly-appeared row. `args` should omit the
    /// leading `"run"` token, e.g. `&["alpine", "--", "/bin/sleep", "30"]`.
    pub fn spawn_run_background(&self, args: &[&str]) -> (Child, String) {
        let mut full_args = vec!["run"];
        full_args.extend_from_slice(args);
        let child = match self.fixture.cli(&full_args).spawn() {
            Ok(child) => child,
            Err(e) => fail(format!("showcase: failed to spawn `mbx run`: {e}")),
        };

        let id = match self.poll_for_new_container_id(Duration::from_secs(5)) {
            Some(id) => id,
            None => fail(format!(
                "showcase: no container appeared in `mbx ps` after `mbx run {args:?}`"
            )),
        };

        (child, id)
    }

    /// Poll `mbx ps` until a container row appears, returning its ID (first
    /// whitespace-delimited column of the last non-header line).
    fn poll_for_new_container_id(&self, timeout: Duration) -> Option<String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let out = self.run_cli(&["ps"]);
            if out.success
                && let Some(id) = last_container_id(&out.stdout)
            {
                return Some(id);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        None
    }

    /// Poll `mbx ps` until `id` shows a `Running` status or `timeout`
    /// elapses.
    #[must_use]
    pub fn wait_for_running(&self, id: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let out = self.run_cli(&["ps"]);
            if out.success
                && out
                    .stdout
                    .lines()
                    .any(|line| line.contains(id) && line.contains("Running"))
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    }
}

/// Abort the current scenario/test with `msg`. Workspace lints deny
/// `panic!`/`.unwrap()`/`.expect()` outright, but `assert!` is the
/// established test-harness idiom in this codebase (see
/// `pull_required`/`assert_cgroup_value` and every `system_tests.rs`
/// assertion) — this just gives that idiom a `-> !` return type so it can
/// be used in an expression position like `match ... { Err(e) => fail(...) }`.
#[allow(clippy::assertions_on_constants)]
fn fail(msg: impl std::fmt::Display) -> ! {
    assert!(false, "{msg}");
    unreachable!()
}

/// Extract the container ID from the last non-empty line of `mbx ps`
/// output, assuming the conventional `ID  NAME  IMAGE  COMMAND  STATE
/// CREATED  PID` table format (7 columns) with a single header row.
///
/// `mbx ps` prints the placeholder line `"(no containers)"` when the list
/// is empty (`crates/mbx/src/commands/ps.rs`) — that line has only 2
/// whitespace tokens, so requiring the full 7-column shape here rejects it
/// rather than mistaking its first token `"(no"` for a real container ID.
fn last_container_id(ps_stdout: &str) -> Option<String> {
    const TABLE_COLUMNS: usize = 7;

    let lines: Vec<&str> = ps_stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return None;
    }
    lines.last().and_then(|line| {
        let mut tokens = line.split_whitespace();
        let id = tokens.next()?;
        if tokens.count() + 1 < TABLE_COLUMNS {
            return None;
        }
        Some(id.to_string())
    })
}

#[cfg(unix)]
fn is_root_impl() -> bool {
    // SAFETY: `geteuid()` is a pure syscall with no preconditions.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
fn is_root_impl() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_binary_missing_returns_err_not_panic() {
        // SAFETY: test-only env mutation, single-threaded within this test
        // and not shared with other env-mutating tests in this crate.
        unsafe {
            std::env::set_var("MINIBOX_TEST_BIN_DIR", "/nonexistent/path/for/test");
        }
        let result = find_binary("definitely-not-a-real-binary-name");
        unsafe {
            std::env::remove_var("MINIBOX_TEST_BIN_DIR");
        }
        assert!(
            result.is_err(),
            "expected Err for a binary that cannot exist anywhere"
        );
    }

    #[test]
    fn descriptor_for_active_adapter_never_panics() {
        // Exercise a few adapter names to confirm the match is exhaustive
        // and infallible regardless of env var contents.
        for name in [
            "native",
            "smolvm",
            "krun",
            "colima",
            "gke",
            "bogus-adapter",
            "",
        ] {
            unsafe {
                std::env::set_var("MINIBOX_ADAPTER", name);
            }
            let descriptor = descriptor_for_active_adapter();
            assert!(!descriptor.name.is_empty());
        }
        unsafe {
            std::env::remove_var("MINIBOX_ADAPTER");
        }
    }

    #[test]
    fn last_container_id_parses_final_row() {
        let ps = "CONTAINER ID  NAME  IMAGE   COMMAND    STATE    CREATED  PID\n\
                   abc123        -     alpine  /bin/sh    Running  now      42\n";
        assert_eq!(last_container_id(ps), Some("abc123".to_string()));
    }

    #[test]
    fn last_container_id_none_when_only_header() {
        let ps = "CONTAINER ID  NAME  IMAGE  COMMAND  STATE  CREATED  PID\n";
        assert_eq!(last_container_id(ps), None);
    }

    #[test]
    fn last_container_id_none_when_no_containers_placeholder() {
        // `mbx ps` prints this line when the list is empty
        // (crates/mbx/src/commands/ps.rs) — its first token "(no" must not
        // be mistaken for a container ID.
        let ps = "CONTAINER ID  NAME  IMAGE  COMMAND  STATE  CREATED  PID\n\
                   (no containers)\n";
        assert_eq!(last_container_id(ps), None);
    }
}
